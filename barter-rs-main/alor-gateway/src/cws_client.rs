use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use std::sync::Arc;
use parking_lot::RwLock;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::gateway_events::{GatewayEvent, log_event};
use crate::health::HealthState;

#[derive(Debug, Clone)]
pub struct CwsHandle {
    cmd_tx: mpsc::Sender<CwsCommand>,
}

#[derive(Debug)]
struct CwsCommand {
    payload: Value,
    resp_tx: oneshot::Sender<anyhow::Result<Value>>,
}

pub struct CwsClient;

impl CwsClient {
    pub fn start(
        cfg: AlorGatewayConfig,
        token_provider: TokenProvider,
        health: Arc<RwLock<HealthState>>,
    ) -> CwsHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(256);

        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(cfg.backoff_initial_ms);
            loop {
                match run_session(&cfg, &token_provider, &mut cmd_rx, &health).await {
                    Ok(()) => break,
                    Err(error) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        warn!(?error, "cws session error; reconnecting");
                        tokio::time::sleep(jittered(backoff)).await;
                        backoff = next_backoff(backoff, &cfg);
                    }
                }
            }
        });

        CwsHandle { cmd_tx }
    }
}

impl CwsHandle {
    pub async fn create_limit(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        price: f64,
        qty: f64,
        side: &str,
    ) -> anyhow::Result<Value> {
        let guid = new_guid();
        let payload = serde_json::json!({
            "opcode": "CreateLimitOrder",
            "guid": guid,
            "portfolio": portfolio,
            "exchange": exchange,
            "symbol": symbol,
            "price": price,
            "quantity": qty,
            "side": side,
        });
        self.send(payload).await
    }

    pub async fn cancel(&self, order_id: i64) -> anyhow::Result<Value> {
        let guid = new_guid();
        let payload = serde_json::json!({
            "opcode": "CancelOrder",
            "guid": guid,
            "orderId": order_id,
        });
        self.send(payload).await
    }

    pub async fn replace(&self, order_id: i64, new_price: f64, new_qty: f64) -> anyhow::Result<Value> {
        let guid = new_guid();
        let payload = serde_json::json!({
            "opcode": "ReplaceOrder",
            "guid": guid,
            "orderId": order_id,
            "price": new_price,
            "quantity": new_qty,
        });
        self.send(payload).await
    }

    async fn send(&self, payload: Value) -> anyhow::Result<Value> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.cmd_tx
            .send(CwsCommand { payload, resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("cws command channel closed"))?;
        let response = tokio::time::timeout(Duration::from_secs(5), resp_rx)
            .await
            .map_err(|_| anyhow::anyhow!("cws response timeout"))?;
        let response = response.map_err(|_| anyhow::anyhow!("cws response channel closed"))??;
        Ok(response)
    }
}

async fn run_session(
    cfg: &AlorGatewayConfig,
    token_provider: &TokenProvider,
    cmd_rx: &mut mpsc::Receiver<CwsCommand>,
    health: &Arc<RwLock<HealthState>>,
) -> anyhow::Result<()> {
    let token = token_provider.access_token().await?;
    let (ws_stream, _) = tokio_tungstenite::connect_async(&cfg.cws_url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    {
        let mut guard = health.write();
        guard.cws_authorized = false;
    }
    authorize(&mut ws_sink, &mut ws_stream, &token, health).await?;

    let mut pending: HashMap<String, oneshot::Sender<anyhow::Result<Value>>> = HashMap::new();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let guid = cmd
                            .payload
                            .get("guid")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                            .unwrap_or_else(new_guid);
                        pending.insert(guid.clone(), cmd.resp_tx);
                        let mut payload = cmd.payload;
                        payload.as_object_mut().map(|map| {
                            map.insert("guid".to_string(), Value::String(guid.clone()));
                            map.insert("token".to_string(), Value::String(token.clone()));
                        });
                        ws_sink
                            .send(Message::Text(payload.to_string().into()))
                            .await?;
                    }
                    None => return Ok(()),
                }
            }
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&txt) {
                            if let Some(guid) = guid_of(&value) {
                                if let Some(tx) = pending.remove(&guid) {
                                    let _ = tx.send(Ok(value));
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        info!(?frame, "cws close received");
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        fail_pending(pending);
                        return Ok(());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        fail_pending(pending);
                        return Err(error.into());
                    }
                    None => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        fail_pending(pending);
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn authorize(
    ws_sink: &mut (impl futures_util::sink::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ws_stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    token: &str,
    health: &Arc<RwLock<HealthState>>,
) -> anyhow::Result<()> {
    let guid = new_guid();
    let payload = serde_json::json!({
        "opcode": "authorize",
        "guid": guid,
        "token": token,
    });
    info!(guid, label = "authorize", "ws subscribe send");
    let redacted_payload = redact_token(&payload.to_string());
    debug!(payload = %redacted_payload, guid, label = "authorize", "ws subscribe payload");
    ws_sink.send(Message::Text(payload.to_string().into())).await?;

    let response = read_until_guid(ws_stream, &guid, Duration::from_secs(5)).await?;
    info!(payload = %response, guid, label = "authorize", "ws subscribe ack");
    let status = response.get("status").and_then(Value::as_i64);
    let http_code = response.get("httpCode").and_then(Value::as_i64);
    let cws_authorized = response
        .get("cws_authorized")
        .or_else(|| response.get("cwsAuthorized"))
        .and_then(Value::as_bool);
    let message = response
        .get("message")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let status_ok = status == Some(200) || http_code == Some(200);
    let authorized = cws_authorized.unwrap_or(status_ok);
    if status_ok && authorized {
        {
            let mut guard = health.write();
            guard.cws_authorized = true;
        }
        log_event(GatewayEvent::CwsAuthorization {
            success: true,
            status: status.or(http_code),
            message,
        });
        Ok(())
    } else {
        {
            let mut guard = health.write();
            guard.cws_authorized = false;
        }
        log_event(GatewayEvent::CwsAuthorization {
            success: false,
            status: status.or(http_code),
            message,
        });
        Err(anyhow::anyhow!("cws authorization failed"))
    }
}

async fn read_until_guid(
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    guid: &str,
    timeout_dur: Duration,
) -> anyhow::Result<Value> {
    let fut = async move {
        while let Some(msg) = stream.next().await {
            let msg = msg?;
            if let Message::Text(txt) = msg {
                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                    if guid_of(&val).as_deref() == Some(guid) {
                        return Ok(val);
                    }
                }
            }
        }
        Err(anyhow::anyhow!("cws stream ended before response"))
    };

    match tokio::time::timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => Err(anyhow::anyhow!("cws authorize timeout")),
    }
}

fn fail_pending(mut pending: HashMap<String, oneshot::Sender<anyhow::Result<Value>>>) {
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(anyhow::anyhow!("cws disconnected")));
    }
}

fn new_guid() -> String {
    Uuid::new_v4().to_string()
}

fn guid_of(value: &Value) -> Option<String> {
    value
        .get("guid")
        .and_then(Value::as_str)
        .or_else(|| value.get("requestGuid").and_then(Value::as_str))
        .map(|value| value.to_string())
}

fn redact_token(payload: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return "<unparseable payload>".to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("token") {
            obj.insert("token".to_string(), Value::String("***".to_string()));
        }
    }
    value.to_string()
}

fn next_backoff(current: Duration, cfg: &AlorGatewayConfig) -> Duration {
    (current * cfg.backoff_multiplier as u32).min(Duration::from_millis(cfg.backoff_max_ms))
}

fn jittered(duration: Duration) -> Duration {
    let jitter_pct = 0.2;
    let millis = duration.as_millis() as f64;
    let jitter = rand::random::<f64>() * jitter_pct;
    let offset = millis * jitter;
    let lower = millis - offset;
    let upper = millis + offset;
    let jittered = lower + (rand::random::<f64>() * (upper - lower));
    Duration::from_millis(jittered.max(0.0) as u64)
}
