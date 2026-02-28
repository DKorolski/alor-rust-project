use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::gateway_events::{GatewayEvent, log_event};
use crate::health::HealthState;

const CWS_TIME_IN_FORCE: &str = "BookOrCancel";
const CWS_ALLOW_MARGIN: bool = true;
const CWS_MARKET_TIME_IN_FORCE: &str = "oneday";
const CWS_MARKET_ALLOW_MARGIN: bool = true;

#[derive(Debug, Clone)]
pub struct CwsHandle {
    cmd_tx: mpsc::Sender<CwsCommand>,
    instrument_group: String,
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
        let instrument_group = cfg.instrument_group.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(cfg.backoff_initial_ms);
            loop {
                match run_session(&cfg, &token_provider, &mut cmd_rx, &health).await {
                    Ok(()) => break,
                    Err(error) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                            guard.cws_reconnects_total =
                                guard.cws_reconnects_total.saturating_add(1);
                        }
                        warn!(?error, "cws session error; reconnecting");
                        tokio::time::sleep(jittered(backoff)).await;
                        backoff = next_backoff(backoff, &cfg);
                    }
                }
            }
        });

        CwsHandle {
            cmd_tx,
            instrument_group,
        }
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
        let qty = qty.round() as i64;
        let payload = serde_json::json!({
            "opcode": "create:limit",
            "guid": guid,
            "side": side,
            "quantity": qty,
            "price": price,
            "instrument": {"symbol": symbol, "exchange": exchange},
            "user": {"portfolio": portfolio},
            "timeInForce": CWS_TIME_IN_FORCE,
            "allowMargin": CWS_ALLOW_MARGIN,
        });
        self.send(payload).await
    }

    pub async fn create_market(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        qty: f64,
        side: &str,
    ) -> anyhow::Result<Value> {
        let qty = qty.round() as i64;
        let payload = build_create_market_payload(
            portfolio,
            exchange,
            symbol,
            &self.instrument_group,
            qty,
            side,
        );
        self.send(payload).await
    }

    pub async fn cancel(
        &self,
        portfolio: &str,
        exchange: &str,
        order_id: i64,
    ) -> anyhow::Result<Value> {
        let guid = new_guid();
        let payload = serde_json::json!({
            "opcode": "delete:limit",
            "guid": guid,
            "orderId": order_id,
            "exchange": exchange,
            "user": {"portfolio": portfolio},
            "checkDuplicates": true,
        });
        self.send(payload).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: Option<&str>,
        side: Option<&str>,
        order_id: i64,
        new_price: f64,
        new_qty: f64,
    ) -> anyhow::Result<Value> {
        let guid = new_guid();
        let new_qty = new_qty.round() as i64;
        let mut payload = Map::new();
        payload.insert(
            "opcode".to_string(),
            Value::String("update:limit".to_string()),
        );
        payload.insert("guid".to_string(), Value::String(guid));
        payload.insert("orderId".to_string(), Value::from(order_id));
        payload.insert("exchange".to_string(), Value::String(exchange.to_string()));
        payload.insert(
            "user".to_string(),
            serde_json::json!({"portfolio": portfolio}),
        );
        payload.insert("price".to_string(), Value::from(new_price));
        payload.insert("quantity".to_string(), Value::from(new_qty));
        payload.insert("allowMargin".to_string(), Value::from(CWS_ALLOW_MARGIN));
        payload.insert(
            "timeInForce".to_string(),
            Value::String(CWS_TIME_IN_FORCE.to_string()),
        );
        payload.insert("checkDuplicates".to_string(), Value::from(true));
        if let Some(symbol) = symbol {
            payload.insert(
                "instrument".to_string(),
                serde_json::json!({"symbol": symbol, "exchange": exchange}),
            );
        }
        if let Some(side) = side {
            payload.insert("side".to_string(), Value::String(side.to_string()));
        }
        let payload = Value::Object(payload);
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

#[cfg(test)]
impl CwsHandle {
    pub fn new_test() -> Self {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<CwsCommand>(8);
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let _ = cmd.resp_tx.send(Ok(serde_json::json!({})));
            }
        });
        CwsHandle {
            cmd_tx,
            instrument_group: "TEST".to_string(),
        }
    }
}

fn build_create_market_payload(
    portfolio: &str,
    exchange: &str,
    symbol: &str,
    instrument_group: &str,
    qty: i64,
    side: &str,
) -> Value {
    let guid = new_guid();
    serde_json::json!({
        "opcode": "create:market",
        "guid": guid,
        "side": side,
        "quantity": qty,
        "instrument": {
            "symbol": symbol,
            "exchange": exchange,
            "instrumentGroup": instrument_group
        },
        "user": {"portfolio": portfolio},
        "timeInForce": CWS_MARKET_TIME_IN_FORCE,
        "allowMargin": CWS_MARKET_ALLOW_MARGIN,
        "checkDuplicates": true,
    })
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
                        if let Some(map) = payload.as_object_mut() {
                            map.insert("guid".to_string(), Value::String(guid.clone()));
                            map.insert("token".to_string(), Value::String(token.clone()));
                        }
                        let opcode = payload.get("opcode").and_then(Value::as_str).unwrap_or("unknown");
                        info!(opcode, guid, "cws send");
                        let redacted_payload = redact_token(&payload.to_string());
                        debug!(opcode, guid, payload = %redacted_payload, "cws send payload");
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
                            let guid = guid_of(&value);
                            let opcode = value.get("opcode").and_then(Value::as_str).unwrap_or("unknown");
                            debug!(opcode, guid = ?guid, payload = %value, "cws recv payload");
                            if let Some(guid) = guid {
                                if let Some(tx) = pending.remove(&guid) {
                                    let _ = tx.send(Ok(value));
                                } else {
                                    warn!(opcode, guid, "cws recv without pending request");
                                }
                            } else {
                                warn!(opcode, "cws recv without guid");
                            }
                        } else {
                            warn!(payload = %txt, "cws recv non-json payload");
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
    ws_sink: &mut (
             impl futures_util::sink::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
             + Unpin
         ),
    ws_stream: &mut (
             impl futures_util::stream::Stream<
        Item = Result<Message, tokio_tungstenite::tungstenite::Error>,
    > + Unpin
         ),
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
    ws_sink
        .send(Message::Text(payload.to_string().into()))
        .await?;

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
    stream: &mut (
             impl futures_util::stream::Stream<
        Item = Result<Message, tokio_tungstenite::tungstenite::Error>,
    > + Unpin
         ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_create_market_payload_includes_required_fields() {
        let payload = build_create_market_payload("D39004", "MOEX", "SBER", "TQBR", 300, "buy");
        let obj = payload.as_object().expect("payload object");
        assert_eq!(
            obj.get("opcode").and_then(Value::as_str),
            Some("create:market")
        );
        assert_eq!(obj.get("side").and_then(Value::as_str), Some("buy"));
        assert_eq!(obj.get("quantity").and_then(Value::as_i64), Some(300));
        assert_eq!(
            obj.get("timeInForce").and_then(Value::as_str),
            Some(CWS_MARKET_TIME_IN_FORCE)
        );
        assert_eq!(
            obj.get("allowMargin").and_then(Value::as_bool),
            Some(CWS_MARKET_ALLOW_MARGIN)
        );
        assert_eq!(
            obj.get("checkDuplicates").and_then(Value::as_bool),
            Some(true)
        );
        let instrument = obj
            .get("instrument")
            .and_then(Value::as_object)
            .expect("instrument");
        assert_eq!(
            instrument.get("symbol").and_then(Value::as_str),
            Some("SBER")
        );
        assert_eq!(
            instrument.get("exchange").and_then(Value::as_str),
            Some("MOEX")
        );
        assert_eq!(
            instrument.get("instrumentGroup").and_then(Value::as_str),
            Some("TQBR")
        );
    }
}
