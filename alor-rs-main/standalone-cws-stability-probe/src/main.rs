use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct ProbeConfig {
    cws_url: String,
    oauth_url: String,
    refresh_token: String,
    portfolio: String,
    exchange: String,
    instrument_group: String,
    symbols: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    cws_url: Option<String>,
    oauth_url: Option<String>,
    refresh_token: Option<String>,
    portfolio: Option<String>,
    exchange: Option<String>,
    instrument_group: Option<String>,
    symbols: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
}

#[derive(Debug)]
struct ArtifactBundle {
    dir: PathBuf,
    inbound: File,
    outbound: File,
    events: File,
}

#[derive(Debug, Serialize)]
struct ProbeResult {
    run_id: String,
    status: String,
    mode: String,
    symbol: String,
    side: String,
    qty: f64,
    price: f64,
    idle_seconds: u64,
    reconnect_before_final_send: bool,
    ping_keepalive_interval_seconds: Option<u64>,
    final_order_id: Option<String>,
    final_http_code: Option<i64>,
    final_request_guid: Option<String>,
    final_error: Option<String>,
    cancel_final_http_code: Option<i64>,
    close_error: Option<String>,
    connection_instances: Vec<String>,
}

#[derive(Debug)]
struct Session {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    conn_instance_id: String,
    send_seq: u64,
    recv_seq: u64,
    last_rx_ts_utc: Option<i64>,
    last_tx_ts_utc: Option<i64>,
    last_ping_ts_utc: Option<i64>,
    last_pong_ts_utc: Option<i64>,
    authorize_ts_utc: Option<i64>,
}

#[derive(Debug)]
struct SendOutcome {
    http_code: Option<i64>,
    request_guid: Option<String>,
    order_id: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let cfg = load_config(&args)?;

    let dry_run = has_flag(&args, "--dry-run");
    let live_confirm = has_flag(&args, "--live-confirm");
    if dry_run == live_confirm {
        bail!("specify exactly one of --dry-run or --live-confirm");
    }

    let symbol = arg_value(&args, "--symbol")
        .or_else(|| env::var("ALOR_SYMBOL").ok())
        .or_else(|| cfg.symbols.first().cloned())
        .context("symbol is required (set --symbol, ALOR_SYMBOL, or symbols in config)")?;
    let side = arg_value(&args, "--side").unwrap_or_else(|| "buy".to_string());
    if !matches!(side.as_str(), "buy" | "sell") {
        bail!("--side must be buy or sell");
    }
    let qty = parse_f64(&args, "--qty", 1.0)?;
    let price = parse_required_f64(&args, "--price")?;
    let idle_seconds = parse_u64(&args, "--idle-seconds", 60)?;
    let send_timeout_seconds = parse_u64(&args, "--send-timeout-seconds", 15)?;
    let ping_keepalive_interval_seconds = parse_u64_opt(&args, "--ping-interval-seconds")?;
    let reconnect_before_final_send = has_flag(&args, "--reconnect-before-final-send");
    let cancel_final_order = has_flag(&args, "--cancel-final-order");
    let instrument_group =
        arg_value(&args, "--instrument-group").unwrap_or_else(|| cfg.instrument_group.clone());
    let comment_prefix =
        arg_value(&args, "--comment-prefix").unwrap_or_else(|| "standalone_cws_probe".to_string());
    let artifact_dir = artifact_dir(&args)?;
    let run_id = run_id();
    let mut artifacts = ArtifactBundle::new(artifact_dir.join(&run_id))?;

    let mut result = ProbeResult {
        run_id: run_id.clone(),
        status: if dry_run {
            "dry_run".to_string()
        } else {
            "started".to_string()
        },
        mode: if dry_run {
            "dry-run".to_string()
        } else {
            "live-confirm".to_string()
        },
        symbol: symbol.clone(),
        side: side.clone(),
        qty,
        price,
        idle_seconds,
        reconnect_before_final_send,
        ping_keepalive_interval_seconds,
        final_order_id: None,
        final_http_code: None,
        final_request_guid: None,
        final_error: None,
        cancel_final_http_code: None,
        close_error: None,
        connection_instances: Vec::new(),
    };

    artifacts.log_event(
        "probe_started",
        &json!({
            "run_id": run_id,
            "mode": result.mode,
            "symbol": symbol,
            "side": side,
            "qty": qty,
            "price": price,
            "idle_seconds": idle_seconds,
            "reconnect_before_final_send": reconnect_before_final_send,
            "ping_keepalive_interval_seconds": ping_keepalive_interval_seconds,
            "cancel_final_order": cancel_final_order,
        }),
    )?;

    if dry_run {
        artifacts.write_summary(&result)?;
        artifacts.write_result(&result)?;
        println!(
            "standalone cws probe dry-run: artifacts={}",
            artifacts.dir.display()
        );
        return Ok(());
    }

    let probe_outcome = run_probe(
        &cfg,
        &args,
        &symbol,
        &side,
        qty,
        price,
        idle_seconds,
        send_timeout_seconds,
        ping_keepalive_interval_seconds,
        reconnect_before_final_send,
        cancel_final_order,
        &instrument_group,
        &comment_prefix,
        &mut artifacts,
        &mut result,
    )
    .await;

    if let Err(error) = &probe_outcome {
        if result.final_error.is_none() {
            result.final_error = Some(error.to_string());
        }
        if result.status == "started" {
            result.status = "probe_error".to_string();
        }
        let _ = artifacts.log_event("probe_error", &json!({ "error": error.to_string() }));
    }

    artifacts.write_summary(&result)?;
    artifacts.write_result(&result)?;

    probe_outcome?;

    println!(
        "standalone cws probe complete: status={} artifacts={}",
        result.status,
        artifacts.dir.display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_probe(
    cfg: &ProbeConfig,
    args: &[String],
    symbol: &str,
    side: &str,
    qty: f64,
    price: f64,
    idle_seconds: u64,
    send_timeout_seconds: u64,
    ping_keepalive_interval_seconds: Option<u64>,
    reconnect_before_final_send: bool,
    cancel_final_order: bool,
    instrument_group: &str,
    comment_prefix: &str,
    artifacts: &mut ArtifactBundle,
    result: &mut ProbeResult,
) -> Result<()> {
    let mut session =
        Session::connect_and_authorize(cfg, artifacts, "initial_connect", send_timeout_seconds)
            .await?;
    result
        .connection_instances
        .push(session.conn_instance_id.clone());

    session
        .idle(
            idle_seconds,
            ping_keepalive_interval_seconds,
            artifacts,
            "idle_before_final_send",
        )
        .await?;

    if reconnect_before_final_send {
        if let Err(error) = session
            .close(artifacts, "reconnect_before_final_send")
            .await
        {
            result.close_error = Some(error.to_string());
            artifacts.log_event(
                "close_failed",
                &json!({
                    "phase": "reconnect_before_final_send",
                    "error": error.to_string(),
                }),
            )?;
        }
        session = Session::connect_and_authorize(
            cfg,
            artifacts,
            "reconnect_before_final_send",
            send_timeout_seconds,
        )
        .await?;
        result
            .connection_instances
            .push(session.conn_instance_id.clone());
    }

    let final_comment = format!("{comment_prefix}_final_{}", now_unix_ts());
    match session
        .send_create_limit(
            cfg,
            symbol,
            side,
            qty,
            price,
            instrument_group,
            &final_comment,
            send_timeout_seconds,
            artifacts,
            "final_create_limit",
        )
        .await
    {
        Ok(final_send) => {
            result.final_order_id = final_send.order_id.clone();
            result.final_http_code = final_send.http_code;
            result.final_request_guid = final_send.request_guid.clone();
            result.status = if final_send.http_code == Some(200) {
                "send_ok".to_string()
            } else {
                "send_response_non_200".to_string()
            };
            if cancel_final_order {
                let order_id = final_send
                    .order_id
                    .as_deref()
                    .context("final create response missing order id for cancel")?;
                let cancel = session
                    .send_delete_limit(
                        cfg,
                        order_id,
                        send_timeout_seconds,
                        artifacts,
                        "final_delete_limit",
                    )
                    .await?;
                result.cancel_final_http_code = cancel.http_code;
            }
        }
        Err(error) => {
            result.status = "send_failed".to_string();
            result.final_error = Some(error.to_string());
            artifacts.log_event("final_send_failed", &json!({ "error": error.to_string() }))?;
        }
    }

    if let Err(error) = session.close(artifacts, "probe_complete").await {
        result.close_error = Some(error.to_string());
        artifacts.log_event(
            "close_failed",
            &json!({
                "phase": "probe_complete",
                "error": error.to_string(),
            }),
        )?;
    }

    let _ = args;
    Ok(())
}

impl ArtifactBundle {
    fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create artifact dir {}", dir.display()))?;
        Ok(Self {
            inbound: OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("frames.inbound.log"))?,
            outbound: OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("frames.outbound.log"))?,
            events: OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("events.log"))?,
            dir,
        })
    }

    fn log_event(&mut self, event: &str, payload: &Value) -> Result<()> {
        let line = json!({
            "ts_utc": Utc::now().timestamp(),
            "event": event,
            "payload": payload,
        });
        writeln!(self.events, "{line}")?;
        Ok(())
    }

    fn log_outbound(
        &mut self,
        conn_instance_id: &str,
        seq: u64,
        frame_type: &str,
        payload: &Value,
    ) -> Result<()> {
        let line = json!({
            "ts_utc": Utc::now().timestamp(),
            "conn_instance_id": conn_instance_id,
            "send_seq": seq,
            "frame_type": frame_type,
            "payload": payload,
        });
        writeln!(self.outbound, "{line}")?;
        Ok(())
    }

    fn log_inbound(
        &mut self,
        conn_instance_id: &str,
        seq: u64,
        frame_type: &str,
        payload: &Value,
    ) -> Result<()> {
        let line = json!({
            "ts_utc": Utc::now().timestamp(),
            "conn_instance_id": conn_instance_id,
            "recv_seq": seq,
            "frame_type": frame_type,
            "payload": payload,
        });
        writeln!(self.inbound, "{line}")?;
        Ok(())
    }

    fn write_summary(&mut self, result: &ProbeResult) -> Result<()> {
        let mut summary = File::create(self.dir.join("summary.txt"))?;
        writeln!(summary, "run_id={}", result.run_id)?;
        writeln!(summary, "status={}", result.status)?;
        writeln!(summary, "mode={}", result.mode)?;
        writeln!(summary, "symbol={}", result.symbol)?;
        writeln!(summary, "side={}", result.side)?;
        writeln!(summary, "qty={}", result.qty)?;
        writeln!(summary, "price={}", result.price)?;
        writeln!(summary, "idle_seconds={}", result.idle_seconds)?;
        writeln!(
            summary,
            "reconnect_before_final_send={}",
            result.reconnect_before_final_send
        )?;
        writeln!(
            summary,
            "ping_keepalive_interval_seconds={}",
            result
                .ping_keepalive_interval_seconds
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        )?;
        writeln!(
            summary,
            "final_order_id={}",
            result
                .final_order_id
                .clone()
                .unwrap_or_else(|| "none".to_string())
        )?;
        writeln!(
            summary,
            "final_http_code={}",
            result
                .final_http_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        )?;
        writeln!(
            summary,
            "cancel_final_http_code={}",
            result
                .cancel_final_http_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        )?;
        writeln!(
            summary,
            "final_error={}",
            result
                .final_error
                .clone()
                .unwrap_or_else(|| "none".to_string())
        )?;
        writeln!(
            summary,
            "close_error={}",
            result
                .close_error
                .clone()
                .unwrap_or_else(|| "none".to_string())
        )?;
        Ok(())
    }

    fn write_result(&mut self, result: &ProbeResult) -> Result<()> {
        let result_json = serde_json::to_vec_pretty(result)?;
        fs::write(self.dir.join("result.json"), result_json)?;
        Ok(())
    }
}

impl Session {
    async fn connect_and_authorize(
        cfg: &ProbeConfig,
        artifacts: &mut ArtifactBundle,
        phase: &str,
        send_timeout_seconds: u64,
    ) -> Result<Self> {
        let conn_instance_id = Uuid::new_v4().to_string();
        let connect_ts = Utc::now().timestamp();
        artifacts.log_event(
            "connect_start",
            &json!({
                "phase": phase,
                "conn_instance_id": conn_instance_id,
                "cws_url": cfg.cws_url,
                "connect_ts_utc": connect_ts,
            }),
        )?;
        let (ws, _) = connect_async(&cfg.cws_url)
            .await
            .with_context(|| format!("failed to connect cws_url={}", cfg.cws_url))?;
        let token = fetch_access_token(cfg).await?;
        let authorize_guid = Uuid::new_v4().to_string();
        let authorize_payload = json!({
            "opcode": "authorize",
            "guid": authorize_guid,
            "token": token,
        });

        let mut session = Self {
            ws,
            conn_instance_id,
            send_seq: 0,
            recv_seq: 0,
            last_rx_ts_utc: None,
            last_tx_ts_utc: None,
            last_ping_ts_utc: None,
            last_pong_ts_utc: None,
            authorize_ts_utc: None,
        };
        session
            .send_json(artifacts, "authorize", authorize_payload.clone())
            .await?;
        let authorize_response = session
            .read_until_guid(
                &authorize_guid,
                Duration::from_secs(send_timeout_seconds),
                artifacts,
                "authorize_wait",
            )
            .await?;
        let status_ok = authorize_response
            .get("status")
            .and_then(Value::as_i64)
            .or_else(|| authorize_response.get("httpCode").and_then(Value::as_i64))
            == Some(200);
        let cws_authorized = authorize_response
            .get("cws_authorized")
            .or_else(|| authorize_response.get("cwsAuthorized"))
            .and_then(Value::as_bool)
            .unwrap_or(status_ok);
        if !status_ok || !cws_authorized {
            bail!("authorize failed: {authorize_response}");
        }
        session.authorize_ts_utc = Some(Utc::now().timestamp());
        artifacts.log_event(
            "authorize_ok",
            &json!({
                "phase": phase,
                "conn_instance_id": session.conn_instance_id,
                "authorize_guid": authorize_guid,
                "authorize_ts_utc": session.authorize_ts_utc,
            }),
        )?;
        Ok(session)
    }

    async fn idle(
        &mut self,
        seconds: u64,
        ping_interval_seconds: Option<u64>,
        artifacts: &mut ArtifactBundle,
        phase: &str,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let ping_interval = ping_interval_seconds.map(Duration::from_secs);
        let mut next_ping_at = ping_interval.map(|interval| Instant::now() + interval);
        artifacts.log_event(
            "idle_start",
            &json!({
                "phase": phase,
                "conn_instance_id": self.conn_instance_id,
                "idle_seconds": seconds,
                "ping_interval_seconds": ping_interval_seconds,
            }),
        )?;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if let (Some(interval), Some(next_ping)) = (ping_interval, next_ping_at)
                && now >= next_ping
            {
                self.send_ping(artifacts, phase).await?;
                next_ping_at = Some(now + interval);
                continue;
            }
            let mut wait_for = deadline.saturating_duration_since(now);
            if let Some(next_ping) = next_ping_at {
                wait_for = wait_for.min(next_ping.saturating_duration_since(now));
            }
            wait_for = wait_for.min(Duration::from_secs(1));
            match timeout(wait_for, self.ws.next()).await {
                Ok(Some(Ok(message))) => {
                    self.handle_inbound_message(message, artifacts, phase)
                        .await?;
                }
                Ok(Some(Err(error))) => return Err(error.into()),
                Ok(None) => bail!("cws stream closed during idle"),
                Err(_) => {}
            }
        }
        artifacts.log_event(
            "idle_complete",
            &json!({
                "phase": phase,
                "conn_instance_id": self.conn_instance_id,
                "last_rx_ts_utc": self.last_rx_ts_utc,
                "last_tx_ts_utc": self.last_tx_ts_utc,
                "last_ping_ts_utc": self.last_ping_ts_utc,
                "last_pong_ts_utc": self.last_pong_ts_utc,
            }),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_create_limit(
        &mut self,
        cfg: &ProbeConfig,
        symbol: &str,
        side: &str,
        qty: f64,
        price: f64,
        instrument_group: &str,
        comment: &str,
        send_timeout_seconds: u64,
        artifacts: &mut ArtifactBundle,
        phase: &str,
    ) -> Result<SendOutcome> {
        let guid = Uuid::new_v4().to_string();
        let payload = json!({
            "opcode": "create:limit",
            "guid": guid,
            "side": side,
            "quantity": qty.round() as i64,
            "price": price,
            "instrument": {
                "symbol": symbol,
                "exchange": cfg.exchange,
                "instrumentGroup": instrument_group
            },
            "user": {
                "portfolio": cfg.portfolio
            },
            "timeInForce": "OneDay",
            "allowMargin": true,
            "checkDuplicates": true,
            "comment": comment,
        });
        artifacts.log_event(
            "create_limit_prepare",
            &json!({
                "phase": phase,
                "conn_instance_id": self.conn_instance_id,
                "guid": guid,
                "symbol": symbol,
                "side": side,
                "qty": qty,
                "price": price,
                "comment": comment,
            }),
        )?;
        self.send_json(artifacts, "create:limit", payload).await?;
        self.read_until_send_outcome(&guid, send_timeout_seconds, artifacts, phase)
            .await
    }

    async fn send_delete_limit(
        &mut self,
        cfg: &ProbeConfig,
        order_id: &str,
        send_timeout_seconds: u64,
        artifacts: &mut ArtifactBundle,
        phase: &str,
    ) -> Result<SendOutcome> {
        let guid = Uuid::new_v4().to_string();
        let numeric_order_id: i64 = order_id
            .parse()
            .with_context(|| format!("delete:limit expects numeric order_id, got {order_id}"))?;
        let payload = json!({
            "opcode": "delete:limit",
            "guid": guid,
            "orderId": numeric_order_id,
            "exchange": cfg.exchange,
            "user": {
                "portfolio": cfg.portfolio
            },
            "checkDuplicates": true,
        });
        artifacts.log_event(
            "delete_limit_prepare",
            &json!({
                "phase": phase,
                "conn_instance_id": self.conn_instance_id,
                "guid": guid,
                "order_id": order_id,
            }),
        )?;
        self.send_json(artifacts, "delete:limit", payload).await?;
        self.read_until_send_outcome(&guid, send_timeout_seconds, artifacts, phase)
            .await
    }

    async fn close(&mut self, artifacts: &mut ArtifactBundle, phase: &str) -> Result<()> {
        let ts_utc = Utc::now().timestamp();
        artifacts.log_event(
            "close_start",
            &json!({
                "phase": phase,
                "conn_instance_id": self.conn_instance_id,
                "ts_utc": ts_utc,
            }),
        )?;
        self.send_seq = self.send_seq.saturating_add(1);
        artifacts.log_outbound(
            &self.conn_instance_id,
            self.send_seq,
            "close",
            &json!({ "opcode": "close" }),
        )?;
        self.ws.send(Message::Close(None)).await?;
        Ok(())
    }

    async fn send_json(
        &mut self,
        artifacts: &mut ArtifactBundle,
        opcode: &str,
        payload: Value,
    ) -> Result<()> {
        self.send_seq = self.send_seq.saturating_add(1);
        self.last_tx_ts_utc = Some(Utc::now().timestamp());
        artifacts.log_outbound(&self.conn_instance_id, self.send_seq, "text", &payload)?;
        artifacts.log_event(
            "send",
            &json!({
                "conn_instance_id": self.conn_instance_id,
                "send_seq": self.send_seq,
                "opcode": opcode,
                "guid": payload.get("guid").and_then(Value::as_str),
            }),
        )?;
        self.ws
            .send(Message::Text(payload.to_string().into()))
            .await?;
        Ok(())
    }

    async fn send_ping(&mut self, artifacts: &mut ArtifactBundle, phase: &str) -> Result<()> {
        self.send_seq = self.send_seq.saturating_add(1);
        self.last_ping_ts_utc = Some(Utc::now().timestamp());
        self.last_tx_ts_utc = self.last_ping_ts_utc;
        let payload = json!({
            "phase": phase,
            "payload_len": 0,
        });
        artifacts.log_outbound(&self.conn_instance_id, self.send_seq, "ping", &payload)?;
        artifacts.log_event(
            "ping_sent",
            &json!({
                "phase": phase,
                "conn_instance_id": self.conn_instance_id,
                "send_seq": self.send_seq,
                "last_ping_ts_utc": self.last_ping_ts_utc,
            }),
        )?;
        self.ws.send(Message::Ping(Vec::new().into())).await?;
        Ok(())
    }

    async fn read_until_send_outcome(
        &mut self,
        guid: &str,
        send_timeout_seconds: u64,
        artifacts: &mut ArtifactBundle,
        phase: &str,
    ) -> Result<SendOutcome> {
        let response = self
            .read_until_guid(
                guid,
                Duration::from_secs(send_timeout_seconds),
                artifacts,
                phase,
            )
            .await?;
        Ok(SendOutcome {
            http_code: response.get("httpCode").and_then(Value::as_i64),
            request_guid: response
                .get("requestGuid")
                .or_else(|| response.get("guid"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            order_id: order_id_of(&response),
        })
    }

    async fn read_until_guid(
        &mut self,
        guid: &str,
        timeout_duration: Duration,
        artifacts: &mut ArtifactBundle,
        phase: &str,
    ) -> Result<Value> {
        let deadline = Instant::now() + timeout_duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timeout waiting for guid={guid}");
            }
            let msg = timeout(remaining, self.ws.next())
                .await
                .context("cws read timeout while waiting for guid")?
                .context("cws stream closed while waiting for guid")??;
            let Some(value) = self.handle_inbound_message(msg, artifacts, phase).await? else {
                continue;
            };
            let request_guid = value
                .get("requestGuid")
                .or_else(|| value.get("guid"))
                .or_else(|| value.get("correlationGuid"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if request_guid == guid {
                return Ok(value);
            }
        }
    }

    async fn handle_inbound_message(
        &mut self,
        message: Message,
        artifacts: &mut ArtifactBundle,
        phase: &str,
    ) -> Result<Option<Value>> {
        self.recv_seq = self.recv_seq.saturating_add(1);
        self.last_rx_ts_utc = Some(Utc::now().timestamp());
        match message {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(&text)
                    .with_context(|| format!("failed to parse inbound text frame: {text}"))?;
                artifacts.log_inbound(&self.conn_instance_id, self.recv_seq, "text", &value)?;
                artifacts.log_event(
                    "recv_text",
                    &json!({
                        "phase": phase,
                        "conn_instance_id": self.conn_instance_id,
                        "recv_seq": self.recv_seq,
                        "request_guid": value.get("requestGuid"),
                        "guid": value.get("guid"),
                        "opcode": value.get("opcode"),
                    }),
                )?;
                Ok(Some(value))
            }
            Message::Ping(payload) => {
                artifacts.log_inbound(
                    &self.conn_instance_id,
                    self.recv_seq,
                    "ping",
                    &json!({ "payload_len": payload.len() }),
                )?;
                self.send_seq = self.send_seq.saturating_add(1);
                artifacts.log_outbound(
                    &self.conn_instance_id,
                    self.send_seq,
                    "pong",
                    &json!({ "payload_len": payload.len() }),
                )?;
                self.ws.send(Message::Pong(payload)).await?;
                Ok(None)
            }
            Message::Pong(payload) => {
                self.last_pong_ts_utc = Some(Utc::now().timestamp());
                artifacts.log_inbound(
                    &self.conn_instance_id,
                    self.recv_seq,
                    "pong",
                    &json!({ "payload_len": payload.len() }),
                )?;
                Ok(None)
            }
            Message::Binary(payload) => {
                artifacts.log_inbound(
                    &self.conn_instance_id,
                    self.recv_seq,
                    "binary",
                    &json!({ "payload_len": payload.len() }),
                )?;
                Ok(None)
            }
            Message::Close(frame) => {
                artifacts.log_inbound(
                    &self.conn_instance_id,
                    self.recv_seq,
                    "close",
                    &json!({
                        "code": frame.as_ref().map(|value| value.code.to_string()),
                        "reason": frame.as_ref().map(|value| value.reason.to_string()),
                    }),
                )?;
                bail!("cws closed during {phase}");
            }
            Message::Frame(_) => Ok(None),
        }
    }
}

async fn fetch_access_token(cfg: &ProbeConfig) -> Result<String> {
    let response = reqwest::Client::new()
        .post(&cfg.oauth_url)
        .query(&[("token", cfg.refresh_token.trim_matches('"').trim())])
        .send()
        .await?
        .error_for_status()?;
    let payload: TokenResponse = response.json().await?;
    Ok(payload.access_token)
}

fn load_config(args: &[String]) -> Result<ProbeConfig> {
    let file_cfg = if let Some(path) = detect_config_path(args) {
        let body = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file {path}"))?;
        toml::from_str::<FileConfig>(&body)
            .with_context(|| format!("failed to parse config file {path}"))?
    } else {
        FileConfig::default()
    };

    let cws_url = env::var("ALOR_CWS_URL")
        .ok()
        .or(file_cfg.cws_url)
        .unwrap_or_else(|| "wss://api.alor.ru/cws".to_string());
    let oauth_url = env::var("ALOR_OAUTH_URL")
        .ok()
        .or(file_cfg.oauth_url)
        .unwrap_or_else(|| "https://oauth.alor.ru/refresh".to_string());
    let refresh_token = env::var("ALOR_REFRESH_TOKEN")
        .ok()
        .or(file_cfg.refresh_token)
        .filter(|value| !value.trim().is_empty())
        .context(
            "missing required refresh_token (set ALOR_REFRESH_TOKEN or refresh_token in config)",
        )?;
    let portfolio = env::var("ALOR_PORTFOLIO")
        .ok()
        .or(file_cfg.portfolio)
        .filter(|value| !value.trim().is_empty())
        .context("missing required portfolio (set ALOR_PORTFOLIO or portfolio in config)")?;
    let exchange = env::var("ALOR_EXCHANGE")
        .ok()
        .or(file_cfg.exchange)
        .unwrap_or_else(|| "MOEX".to_string());
    let instrument_group = env::var("ALOR_INSTRUMENT_GROUP")
        .ok()
        .or(file_cfg.instrument_group)
        .unwrap_or_else(|| "RFUD".to_string());
    let symbols = file_cfg.symbols.unwrap_or_default();

    Ok(ProbeConfig {
        cws_url,
        oauth_url,
        refresh_token,
        portfolio,
        exchange,
        instrument_group,
        symbols,
    })
}

fn detect_config_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            return iter.next().cloned();
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(path.to_string());
        }
    }
    None
}

fn artifact_dir(args: &[String]) -> Result<PathBuf> {
    if let Some(path) = arg_value(args, "--artifact-dir") {
        return Ok(PathBuf::from(path));
    }
    Ok(Path::new("artifacts").join("standalone-cws-stability-probe"))
}

fn order_id_of(value: &Value) -> Option<String> {
    value
        .get("orderNumber")
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .or_else(|| {
            value
                .get("orderNumber")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .or_else(|| {
            value
                .get("orderId")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
        })
        .or_else(|| {
            value
                .get("orderId")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| {
            if window[0] == flag {
                Some(window[1].clone())
            } else {
                None
            }
        })
        .or_else(|| {
            args.iter().find_map(|arg| {
                arg.strip_prefix(&format!("{flag}="))
                    .map(ToString::to_string)
            })
        })
}

fn parse_u64(args: &[String], flag: &str, default: u64) -> Result<u64> {
    match arg_value(args, flag) {
        Some(value) => value
            .parse::<u64>()
            .with_context(|| format!("failed to parse {flag}={value}")),
        None => Ok(default),
    }
}

fn parse_u64_opt(args: &[String], flag: &str) -> Result<Option<u64>> {
    match arg_value(args, flag) {
        Some(value) => {
            Ok(Some(value.parse::<u64>().with_context(|| {
                format!("failed to parse {flag}={value}")
            })?))
        }
        None => Ok(None),
    }
}

fn parse_f64(args: &[String], flag: &str, default: f64) -> Result<f64> {
    match arg_value(args, flag) {
        Some(value) => value
            .parse::<f64>()
            .with_context(|| format!("failed to parse {flag}={value}")),
        None => Ok(default),
    }
}

fn parse_required_f64(args: &[String], flag: &str) -> Result<f64> {
    let value = arg_value(args, flag).with_context(|| format!("{flag} is required"))?;
    value
        .parse::<f64>()
        .with_context(|| format!("failed to parse {flag}={value}"))
}

fn run_id() -> String {
    format!("standalone-cws-probe-{}", now_unix_ts())
}

fn now_unix_ts() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    i64::try_from(now.as_secs()).unwrap_or(i64::MAX)
}
