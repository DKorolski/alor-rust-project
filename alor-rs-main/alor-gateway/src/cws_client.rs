use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::{
    Error as WsError, Message, error::ProtocolError, protocol::CloseFrame,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use alor_protocol::StopLimitCondition;

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::gateway_events::{GatewayEvent, log_event};
use crate::health::HealthState;

const CWS_TIME_IN_FORCE: &str = "OneDay";
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
    request_id: Option<String>,
    symbol: Option<String>,
    resp_tx: oneshot::Sender<anyhow::Result<Value>>,
}

#[derive(Debug)]
struct PendingRequest {
    request_id: Option<String>,
    cws_guid: String,
    opcode: String,
    symbol: Option<String>,
    resp_tx: oneshot::Sender<anyhow::Result<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwsDisconnectKind {
    ProtocolResetWithoutCloseHandshake,
    CloseFrame,
    Eof,
    SendError,
    SocketError,
    ProtocolError,
}

impl CwsDisconnectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProtocolResetWithoutCloseHandshake => "protocol_reset_without_close_handshake",
            Self::CloseFrame => "close_frame",
            Self::Eof => "eof",
            Self::SendError => "send_error",
            Self::SocketError => "socket_error",
            Self::ProtocolError => "protocol_error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwsTransportFailure {
    disconnect_kind: CwsDisconnectKind,
    raw_error: String,
    close_code: Option<u16>,
    close_reason: Option<String>,
}

impl CwsTransportFailure {
    pub(crate) fn new(
        disconnect_kind: CwsDisconnectKind,
        raw_error: impl Into<String>,
        close_code: Option<u16>,
        close_reason: Option<String>,
    ) -> Self {
        Self {
            disconnect_kind,
            raw_error: raw_error.into(),
            close_code,
            close_reason,
        }
    }

    pub fn disconnect_kind(&self) -> &CwsDisconnectKind {
        &self.disconnect_kind
    }

    pub fn disconnect_kind_str(&self) -> &'static str {
        self.disconnect_kind.as_str()
    }

    pub fn raw_error(&self) -> &str {
        &self.raw_error
    }

    pub fn close_code(&self) -> Option<u16> {
        self.close_code
    }

    pub fn close_reason(&self) -> Option<&str> {
        self.close_reason.as_deref()
    }

    pub fn summary(&self) -> String {
        format!("cws disconnected: {}", self.disconnect_kind_str())
    }
}

impl fmt::Display for CwsTransportFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

impl std::error::Error for CwsTransportFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwsRequestFailure {
    request_id: Option<String>,
    cws_guid: String,
    opcode: String,
    symbol: Option<String>,
    transport: CwsTransportFailure,
}

impl CwsRequestFailure {
    pub(crate) fn new(
        request_id: Option<String>,
        cws_guid: String,
        opcode: String,
        symbol: Option<String>,
        transport: CwsTransportFailure,
    ) -> Self {
        Self {
            request_id,
            cws_guid,
            opcode,
            symbol,
            transport,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub fn cws_guid(&self) -> &str {
        &self.cws_guid
    }

    pub fn opcode(&self) -> &str {
        &self.opcode
    }

    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    pub fn transport(&self) -> &CwsTransportFailure {
        &self.transport
    }

    pub fn summary(&self) -> String {
        self.transport.summary()
    }
}

impl fmt::Display for CwsRequestFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

impl std::error::Error for CwsRequestFailure {}

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
        comment: Option<&str>,
        request_id: Option<&str>,
    ) -> anyhow::Result<Value> {
        let qty = qty.round() as i64;
        let check_duplicates = true;
        let payload = build_create_limit_payload(
            portfolio,
            exchange,
            symbol,
            &self.instrument_group,
            price,
            qty,
            side,
            comment,
            check_duplicates,
            request_id,
        );
        let cws_guid = payload
            .get("guid")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        info!(
            action = "cws_limit_send",
            opcode = "create:limit",
            request_id = ?request_id,
            cws_guid,
            symbol,
            exchange,
            instrument_group = %self.instrument_group,
            side,
            qty,
            price,
            time_in_force = CWS_TIME_IN_FORCE,
            allow_margin = CWS_ALLOW_MARGIN,
            check_duplicates,
            "cws limit request prepared"
        );
        self.send_command(payload, request_id, Some(symbol)).await
    }

    pub async fn create_market(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        qty: f64,
        side: &str,
        comment: Option<&str>,
        request_id: Option<&str>,
    ) -> anyhow::Result<Value> {
        let qty = qty.round() as i64;
        let payload = build_create_market_payload(
            portfolio,
            exchange,
            symbol,
            &self.instrument_group,
            qty,
            side,
            comment,
        );
        self.send_command(payload, request_id, Some(symbol)).await
    }

    pub async fn cancel(
        &self,
        portfolio: &str,
        exchange: &str,
        order_id: i64,
        request_id: Option<&str>,
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
        self.send_command(payload, request_id, None).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_stop_limit(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        side: &str,
        qty: f64,
        trigger_price: f64,
        price: f64,
        condition: StopLimitCondition,
        stop_end_unix_time: i64,
        comment: Option<&str>,
        instrument_group: Option<&str>,
        check_duplicates: bool,
        request_id: Option<&str>,
    ) -> anyhow::Result<Value> {
        let qty = qty.round() as i64;
        let resolved_instrument_group =
            resolve_stop_limit_instrument_group(instrument_group, &self.instrument_group);
        let payload = build_create_stop_limit_payload(
            portfolio,
            exchange,
            symbol,
            side,
            qty,
            trigger_price,
            price,
            condition,
            stop_end_unix_time,
            comment,
            resolved_instrument_group,
            check_duplicates,
        );
        self.send_command(payload, request_id, Some(symbol)).await
    }

    pub async fn delete_stop_limit(
        &self,
        portfolio: &str,
        exchange: &str,
        stop_order_id: &str,
        side: Option<&str>,
        check_duplicates: bool,
        request_id: Option<&str>,
    ) -> anyhow::Result<Value> {
        let guid = new_guid();
        let mut payload = Map::new();
        payload.insert(
            "opcode".to_string(),
            Value::String("delete:stopLimit".to_string()),
        );
        payload.insert("guid".to_string(), Value::String(guid));
        payload.insert(
            "orderId".to_string(),
            Value::String(stop_order_id.to_string()),
        );
        payload.insert("exchange".to_string(), Value::String(exchange.to_string()));
        payload.insert(
            "user".to_string(),
            serde_json::json!({"portfolio": portfolio}),
        );
        payload.insert("checkDuplicates".to_string(), Value::from(check_duplicates));
        if let Some(side) = side {
            payload.insert("side".to_string(), Value::String(side.to_string()));
        }
        self.send_command(Value::Object(payload), request_id, None)
            .await
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
        request_id: Option<&str>,
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
        self.send_command(payload, request_id, symbol).await
    }

    async fn send_command(
        &self,
        payload: Value,
        request_id: Option<&str>,
        symbol: Option<&str>,
    ) -> anyhow::Result<Value> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.cmd_tx
            .send(CwsCommand {
                payload,
                request_id: request_id.map(ToString::to_string),
                symbol: symbol.map(ToString::to_string),
                resp_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("cws command channel closed"))?;
        let response = tokio::time::timeout(Duration::from_secs(5), resp_rx)
            .await
            .map_err(|_| anyhow::anyhow!("cws response timeout"))?;
        let response = response.map_err(|_| anyhow::anyhow!("cws response channel closed"))??;
        Ok(response)
    }
}

fn resolve_stop_limit_instrument_group<'a>(
    requested: Option<&'a str>,
    default_group: &'a str,
) -> Option<&'a str> {
    requested.or(Some(default_group))
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
    comment: Option<&str>,
) -> Value {
    let guid = new_guid();
    let mut payload = Map::new();
    payload.insert(
        "opcode".to_string(),
        Value::String("create:market".to_string()),
    );
    payload.insert("guid".to_string(), Value::String(guid));
    payload.insert("side".to_string(), Value::String(side.to_string()));
    payload.insert("quantity".to_string(), Value::from(qty));
    payload.insert(
        "instrument".to_string(),
        serde_json::json!({
            "symbol": symbol,
            "exchange": exchange,
            "instrumentGroup": instrument_group
        }),
    );
    payload.insert(
        "user".to_string(),
        serde_json::json!({"portfolio": portfolio}),
    );
    payload.insert(
        "timeInForce".to_string(),
        Value::String(CWS_MARKET_TIME_IN_FORCE.to_string()),
    );
    payload.insert(
        "allowMargin".to_string(),
        Value::from(CWS_MARKET_ALLOW_MARGIN),
    );
    payload.insert("checkDuplicates".to_string(), Value::from(true));
    if let Some(comment) = comment {
        payload.insert("comment".to_string(), Value::String(comment.to_string()));
    }
    Value::Object(payload)
}

#[allow(clippy::too_many_arguments)]
fn build_create_limit_payload(
    portfolio: &str,
    exchange: &str,
    symbol: &str,
    instrument_group: &str,
    price: f64,
    qty: i64,
    side: &str,
    comment: Option<&str>,
    check_duplicates: bool,
    request_id: Option<&str>,
) -> Value {
    let guid = request_id.map(ToString::to_string).unwrap_or_else(new_guid);
    let mut payload = Map::new();
    payload.insert(
        "opcode".to_string(),
        Value::String("create:limit".to_string()),
    );
    payload.insert("guid".to_string(), Value::String(guid));
    payload.insert("side".to_string(), Value::String(side.to_string()));
    payload.insert("quantity".to_string(), Value::from(qty));
    payload.insert("price".to_string(), Value::from(price));
    payload.insert(
        "instrument".to_string(),
        serde_json::json!({
            "symbol": symbol,
            "exchange": exchange,
            "instrumentGroup": instrument_group
        }),
    );
    payload.insert(
        "user".to_string(),
        serde_json::json!({"portfolio": portfolio}),
    );
    payload.insert(
        "timeInForce".to_string(),
        Value::String(CWS_TIME_IN_FORCE.to_string()),
    );
    payload.insert("allowMargin".to_string(), Value::from(CWS_ALLOW_MARGIN));
    payload.insert("checkDuplicates".to_string(), Value::from(check_duplicates));
    if let Some(comment) = comment {
        payload.insert("comment".to_string(), Value::String(comment.to_string()));
    }
    Value::Object(payload)
}

#[allow(clippy::too_many_arguments)]
fn build_create_stop_limit_payload(
    portfolio: &str,
    exchange: &str,
    symbol: &str,
    side: &str,
    qty: i64,
    trigger_price: f64,
    price: f64,
    condition: StopLimitCondition,
    stop_end_unix_time: i64,
    comment: Option<&str>,
    instrument_group: Option<&str>,
    check_duplicates: bool,
) -> Value {
    let guid = new_guid();
    let mut instrument = Map::new();
    instrument.insert("symbol".to_string(), Value::String(symbol.to_string()));
    instrument.insert("exchange".to_string(), Value::String(exchange.to_string()));
    if let Some(group) = instrument_group {
        instrument.insert(
            "instrumentGroup".to_string(),
            Value::String(group.to_string()),
        );
    }

    let mut payload = Map::new();
    payload.insert(
        "opcode".to_string(),
        Value::String("create:stopLimit".to_string()),
    );
    payload.insert("guid".to_string(), Value::String(guid));
    payload.insert("side".to_string(), Value::String(side.to_string()));
    payload.insert("quantity".to_string(), Value::from(qty));
    payload.insert("triggerPrice".to_string(), Value::from(trigger_price));
    payload.insert("price".to_string(), Value::from(price));
    payload.insert(
        "condition".to_string(),
        Value::String(condition.as_canonical_str().to_string()),
    );
    payload.insert(
        "stopEndUnixTime".to_string(),
        Value::from(stop_end_unix_time),
    );
    payload.insert("instrument".to_string(), Value::Object(instrument));
    payload.insert(
        "user".to_string(),
        serde_json::json!({"portfolio": portfolio}),
    );
    payload.insert("allowMargin".to_string(), Value::from(CWS_ALLOW_MARGIN));
    payload.insert("checkDuplicates".to_string(), Value::from(check_duplicates));
    if let Some(comment) = comment {
        payload.insert("comment".to_string(), Value::String(comment.to_string()));
    }
    Value::Object(payload)
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

    let mut pending: HashMap<String, PendingRequest> = HashMap::new();

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
                        let mut payload = cmd.payload;
                        if let Some(map) = payload.as_object_mut() {
                            map.insert("guid".to_string(), Value::String(guid.clone()));
                            map.insert("token".to_string(), Value::String(token.clone()));
                        }
                        let opcode = payload
                            .get("opcode")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        pending.insert(
                            guid.clone(),
                            PendingRequest {
                                request_id: cmd.request_id,
                                cws_guid: guid.clone(),
                                opcode: opcode.clone(),
                                symbol: cmd.symbol.or_else(|| symbol_of_payload(&payload)),
                                resp_tx: cmd.resp_tx,
                            },
                        );
                        info!(opcode, guid, "cws send");
                        let redacted_payload = redact_token(&payload.to_string());
                        debug!(opcode, guid, payload = %redacted_payload, "cws send payload");
                        if let Err(error) = ws_sink
                            .send(Message::Text(payload.to_string().into()))
                            .await
                        {
                            {
                                let mut guard = health.write();
                                guard.cws_authorized = false;
                            }
                            let failure = transport_failure_from_send_error(&error);
                            let session_error = handle_transport_failure(&mut pending, failure);
                            return Err(session_error);
                        }
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
                                if let Some(request) = pending.remove(&guid) {
                                    let _ = request.resp_tx.send(Ok(value));
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
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        let failure = transport_failure_from_close_frame(frame);
                        let session_error = handle_transport_failure(&mut pending, failure);
                        return Err(session_error);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        let failure = transport_failure_from_receive_error(&error);
                        let session_error = handle_transport_failure(&mut pending, failure);
                        return Err(session_error);
                    }
                    None => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        let failure = transport_failure_from_eof();
                        let session_error = handle_transport_failure(&mut pending, failure);
                        return Err(session_error);
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
                let Ok(val) = serde_json::from_str::<Value>(&txt) else {
                    continue;
                };
                if guid_of(&val).as_deref() == Some(guid) {
                    return Ok(val);
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

fn handle_transport_failure(
    pending: &mut HashMap<String, PendingRequest>,
    failure: CwsTransportFailure,
) -> anyhow::Error {
    log_transport_failure(&failure, pending);
    fail_pending_with_transport(pending, failure.clone());
    anyhow::Error::new(failure)
}

fn log_transport_failure(failure: &CwsTransportFailure, pending: &HashMap<String, PendingRequest>) {
    let first = pending.values().next();
    let request_id = first.and_then(|request| request.request_id.as_deref());
    let cws_guid = first.map(|request| request.cws_guid.as_str());
    let opcode_in_flight = first.map(|request| request.opcode.as_str());
    warn!(
        action = "cws_transport_failure",
        disconnect_kind = failure.disconnect_kind_str(),
        opcode_in_flight = ?opcode_in_flight,
        request_id = ?request_id,
        cws_guid = ?cws_guid,
        pending_count = pending.len(),
        close_code = ?failure.close_code(),
        close_reason = ?failure.close_reason(),
        raw_error = %failure.raw_error(),
        "cws transport failure"
    );
}

fn fail_pending_with_transport(
    pending: &mut HashMap<String, PendingRequest>,
    failure: CwsTransportFailure,
) {
    let affected = pending
        .values()
        .map(|request| {
            serde_json::json!({
                "request_id": request.request_id.as_deref(),
                "cws_guid": request.cws_guid.as_str(),
                "opcode": request.opcode.as_str(),
                "symbol": request.symbol.as_deref(),
            })
        })
        .collect::<Vec<_>>();
    if !affected.is_empty() {
        let affected_json = serde_json::Value::Array(affected);
        warn!(
            action = "cws_fail_pending",
            disconnect_kind = failure.disconnect_kind_str(),
            pending_count = affected_json.as_array().map_or(0, Vec::len),
            affected = %affected_json,
            "failing pending cws requests after transport failure"
        );
    }
    for (_, request) in pending.drain() {
        let error = CwsRequestFailure::new(
            request.request_id,
            request.cws_guid,
            request.opcode,
            request.symbol,
            failure.clone(),
        );
        let _ = request.resp_tx.send(Err(anyhow::Error::new(error)));
    }
}

fn transport_failure_from_send_error(error: &WsError) -> CwsTransportFailure {
    classify_transport_error(error, true)
}

fn transport_failure_from_receive_error(error: &WsError) -> CwsTransportFailure {
    classify_transport_error(error, false)
}

fn transport_failure_from_close_frame(frame: Option<CloseFrame>) -> CwsTransportFailure {
    let close_code = frame.as_ref().map(|frame| u16::from(frame.code));
    let close_reason = frame.as_ref().and_then(|frame| {
        let reason = frame.reason.to_string();
        (!reason.is_empty()).then_some(reason)
    });
    CwsTransportFailure::new(
        CwsDisconnectKind::CloseFrame,
        "close frame received",
        close_code,
        close_reason,
    )
}

fn transport_failure_from_eof() -> CwsTransportFailure {
    CwsTransportFailure::new(CwsDisconnectKind::Eof, "websocket stream ended", None, None)
}

fn classify_transport_error(error: &WsError, during_send: bool) -> CwsTransportFailure {
    let disconnect_kind = match error {
        WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            CwsDisconnectKind::ProtocolResetWithoutCloseHandshake
        }
        WsError::ConnectionClosed | WsError::AlreadyClosed => CwsDisconnectKind::CloseFrame,
        WsError::Io(io_error) if io_error.kind() == std::io::ErrorKind::UnexpectedEof => {
            CwsDisconnectKind::Eof
        }
        WsError::Protocol(_) => CwsDisconnectKind::ProtocolError,
        WsError::Io(_) if during_send => CwsDisconnectKind::SendError,
        WsError::Io(_) => CwsDisconnectKind::SocketError,
        _ if during_send => CwsDisconnectKind::SendError,
        _ => CwsDisconnectKind::SocketError,
    };
    CwsTransportFailure::new(disconnect_kind, error.to_string(), None, None)
}

fn new_guid() -> String {
    Uuid::new_v4().to_string()
}

fn symbol_of_payload(payload: &Value) -> Option<String> {
    payload
        .get("instrument")
        .and_then(Value::as_object)
        .and_then(|instrument| instrument.get("symbol"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
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
    if let Some(obj) = value
        .as_object_mut()
        .filter(|obj| obj.contains_key("token"))
    {
        obj.insert("token".to_string(), Value::String("***".to_string()));
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
    use tokio::sync::oneshot;
    use tokio_tungstenite::tungstenite::{
        Error as WsError, error::ProtocolError, protocol::frame::CloseFrame,
        protocol::frame::coding::CloseCode,
    };

    #[test]
    fn build_create_limit_payload_includes_required_fields() {
        let payload = build_create_limit_payload(
            "D39004",
            "MOEX",
            "SBER",
            "TQBR",
            123.45,
            7,
            "buy",
            None,
            true,
            Some("req-1"),
        );
        let obj = payload.as_object().expect("payload object");
        assert_eq!(
            obj.get("opcode").and_then(Value::as_str),
            Some("create:limit")
        );
        assert_eq!(obj.get("guid").and_then(Value::as_str), Some("req-1"));
        assert_eq!(obj.get("side").and_then(Value::as_str), Some("buy"));
        assert_eq!(obj.get("quantity").and_then(Value::as_i64), Some(7));
        assert_eq!(obj.get("price").and_then(Value::as_f64), Some(123.45));
        assert_eq!(
            obj.get("timeInForce").and_then(Value::as_str),
            Some(CWS_TIME_IN_FORCE)
        );
        assert_eq!(
            obj.get("allowMargin").and_then(Value::as_bool),
            Some(CWS_ALLOW_MARGIN)
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

    #[test]
    fn build_create_limit_payload_nests_instrument_group() {
        let payload = build_create_limit_payload(
            "D39004", "MOEX", "SBER", "TQBR", 123.45, 7, "buy", None, true, None,
        );
        let obj = payload.as_object().expect("payload object");
        assert!(
            !obj.contains_key("instrumentGroup"),
            "instrumentGroup must be nested under instrument"
        );
        let instrument = obj
            .get("instrument")
            .and_then(Value::as_object)
            .expect("instrument");
        assert_eq!(
            instrument.get("instrumentGroup").and_then(Value::as_str),
            Some("TQBR")
        );
    }

    #[test]
    fn build_create_limit_payload_preserves_optional_comment() {
        let payload = build_create_limit_payload(
            "D39004",
            "MOEX",
            "SBER",
            "TQBR",
            123.45,
            7,
            "buy",
            Some("hello"),
            true,
            None,
        );
        let obj = payload.as_object().expect("payload object");
        assert_eq!(obj.get("comment").and_then(Value::as_str), Some("hello"));
        assert_eq!(
            obj.get("timeInForce").and_then(Value::as_str),
            Some(CWS_TIME_IN_FORCE)
        );
        assert_eq!(
            obj.get("checkDuplicates").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn build_create_limit_payload_always_sets_check_duplicates() {
        let payload = build_create_limit_payload(
            "D39004", "MOEX", "SBER", "TQBR", 123.45, 7, "buy", None, true, None,
        );
        let obj = payload.as_object().expect("payload object");
        assert_eq!(
            obj.get("checkDuplicates").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn build_create_limit_payload_time_in_force_is_oneday() {
        let payload = build_create_limit_payload(
            "D39004", "MOEX", "SBER", "TQBR", 123.45, 7, "buy", None, true, None,
        );
        let obj = payload.as_object().expect("payload object");
        assert_eq!(
            obj.get("timeInForce").and_then(Value::as_str),
            Some("OneDay")
        );
    }

    #[test]
    fn limit_market_payload_parity_for_common_transport_fields() {
        let market = build_create_market_payload("D39004", "MOEX", "SBER", "TQBR", 7, "buy", None);
        let limit = build_create_limit_payload(
            "D39004", "MOEX", "SBER", "TQBR", 123.45, 7, "buy", None, true, None,
        );
        let market_obj = market.as_object().expect("market object");
        let limit_obj = limit.as_object().expect("limit object");
        for field in [
            "opcode",
            "guid",
            "side",
            "quantity",
            "instrument",
            "user",
            "timeInForce",
            "allowMargin",
            "checkDuplicates",
        ] {
            assert!(market_obj.contains_key(field), "market missing {field}");
            assert!(limit_obj.contains_key(field), "limit missing {field}");
        }
    }

    #[test]
    fn build_create_market_payload_includes_required_fields() {
        let payload =
            build_create_market_payload("D39004", "MOEX", "SBER", "TQBR", 300, "buy", None);
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

    #[test]
    fn build_create_stop_limit_payload_nests_instrument_group() {
        let payload = build_create_stop_limit_payload(
            "D39004",
            "MOEX",
            "IMOEXF",
            "buy",
            1,
            100.0,
            101.0,
            StopLimitCondition::LessOrEqual,
            1_700_000_000,
            Some("smoke"),
            Some("RFUD"),
            true,
        );
        let obj = payload.as_object().expect("payload object");
        assert_eq!(
            obj.get("condition").and_then(Value::as_str),
            Some("lessorequal")
        );
        assert!(
            !obj.contains_key("instrumentGroup"),
            "instrumentGroup must be nested under instrument"
        );
        let instrument = obj
            .get("instrument")
            .and_then(Value::as_object)
            .expect("instrument");
        assert_eq!(
            instrument.get("instrumentGroup").and_then(Value::as_str),
            Some("RFUD")
        );
    }

    #[test]
    fn stop_limit_instrument_group_falls_back_to_default() {
        assert_eq!(
            resolve_stop_limit_instrument_group(None, "RFUD"),
            Some("RFUD")
        );
        assert_eq!(
            resolve_stop_limit_instrument_group(Some("SPBFUT"), "RFUD"),
            Some("SPBFUT")
        );
    }

    #[test]
    fn classify_transport_error_protocol_reset_is_stable() {
        let failure = classify_transport_error(
            &WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake),
            false,
        );
        assert_eq!(
            failure.disconnect_kind(),
            &CwsDisconnectKind::ProtocolResetWithoutCloseHandshake
        );
        assert_eq!(
            failure.summary(),
            "cws disconnected: protocol_reset_without_close_handshake"
        );
    }

    #[test]
    fn classify_transport_error_send_error_is_stable() {
        let failure =
            classify_transport_error(&WsError::Io(std::io::Error::other("broken pipe")), true);
        assert_eq!(failure.disconnect_kind(), &CwsDisconnectKind::SendError);
        assert_eq!(failure.summary(), "cws disconnected: send_error");
    }

    #[test]
    fn close_frame_and_eof_are_classified_stably() {
        let close = transport_failure_from_close_frame(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "bye".into(),
        }));
        assert_eq!(close.disconnect_kind(), &CwsDisconnectKind::CloseFrame);
        assert_eq!(close.close_code(), Some(1000));
        assert_eq!(close.close_reason(), Some("bye"));

        let eof = transport_failure_from_eof();
        assert_eq!(eof.disconnect_kind(), &CwsDisconnectKind::Eof);
        assert_eq!(eof.summary(), "cws disconnected: eof");
    }

    #[tokio::test]
    async fn transport_failure_preserves_guid_for_pending_requests() {
        let (tx_one, rx_one) = oneshot::channel();
        let (tx_two, rx_two) = oneshot::channel();
        let mut pending = HashMap::new();
        pending.insert(
            "guid-1".to_string(),
            PendingRequest {
                request_id: Some("req-1".to_string()),
                cws_guid: "guid-1".to_string(),
                opcode: "create:limit".to_string(),
                symbol: Some("USDRUBF".to_string()),
                resp_tx: tx_one,
            },
        );
        pending.insert(
            "guid-2".to_string(),
            PendingRequest {
                request_id: Some("req-2".to_string()),
                cws_guid: "guid-2".to_string(),
                opcode: "delete:limit".to_string(),
                symbol: None,
                resp_tx: tx_two,
            },
        );

        let reconnect_error = handle_transport_failure(
            &mut pending,
            CwsTransportFailure::new(
                CwsDisconnectKind::ProtocolResetWithoutCloseHandshake,
                "Connection reset without closing handshake",
                None,
                None,
            ),
        );

        let session_failure = reconnect_error
            .downcast::<CwsTransportFailure>()
            .expect("transport failure");
        assert_eq!(
            session_failure.disconnect_kind(),
            &CwsDisconnectKind::ProtocolResetWithoutCloseHandshake
        );
        assert!(pending.is_empty(), "all pending requests must be drained");

        let err_one = rx_one
            .await
            .expect("response one")
            .expect_err("transport failure expected");
        let err_one = err_one
            .downcast::<CwsRequestFailure>()
            .expect("typed request failure");
        assert_eq!(err_one.request_id(), Some("req-1"));
        assert_eq!(err_one.cws_guid(), "guid-1");
        assert_eq!(err_one.opcode(), "create:limit");
        assert_eq!(err_one.symbol(), Some("USDRUBF"));
        assert_eq!(
            err_one.transport().disconnect_kind(),
            &CwsDisconnectKind::ProtocolResetWithoutCloseHandshake
        );

        let err_two = rx_two
            .await
            .expect("response two")
            .expect_err("transport failure expected");
        let err_two = err_two
            .downcast::<CwsRequestFailure>()
            .expect("typed request failure");
        assert_eq!(err_two.request_id(), Some("req-2"));
        assert_eq!(err_two.cws_guid(), "guid-2");
        assert_eq!(err_two.opcode(), "delete:limit");
        assert_eq!(err_two.symbol(), None);
    }
}
