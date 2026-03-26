use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::{
    Error as WsError, Message, error::ProtocolError, protocol::CloseFrame,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use alor_protocol::StopLimitCondition;

use crate::auth::{AccessTokenSnapshot, TokenProvider};
use crate::config::AlorGatewayConfig;
use crate::gateway_events::{GatewayEvent, log_event};
use crate::health::{CwsFrameTraceEntry, HealthState, evaluate_control_path_stale};

const CWS_TIME_IN_FORCE: &str = "OneDay";
const CWS_ALLOW_MARGIN: bool = true;
const CWS_MARKET_TIME_IN_FORCE: &str = "oneday";
const CWS_MARKET_ALLOW_MARGIN: bool = true;
const DIAG_CWS_SEND_SEQ: &str = "_diag_cws_send_seq";
const DIAG_CWS_SEND_TS_UTC: &str = "_diag_cws_send_ts_utc";
const DIAG_CWS_RECV_SEQ: &str = "_diag_cws_recv_seq";
const DIAG_CWS_RECV_TS_UTC: &str = "_diag_cws_recv_ts_utc";
const DIAG_CWS_MESSAGE_CLASS: &str = "_diag_cws_message_class";
const DIAG_CWS_CONNECTION_INSTANCE_ID: &str = "_diag_cws_connection_instance_id";
const DIAG_CWS_CONNECT_SEQ: &str = "_diag_cws_connect_seq";
const DIAG_CWS_RECONNECT_SEQ: &str = "_diag_cws_reconnect_seq";
const DIAG_CWS_CORRELATION_GUID: &str = "_diag_cws_correlation_guid";
const DIAG_CWS_ORDER_ID: &str = "_diag_cws_order_id";
const DIAG_CWS_REQUEST_ORDER_ID: &str = "_diag_cws_request_order_id";
const DIAG_CWS_SYMBOL: &str = "_diag_cws_symbol";
const CWS_PENDING_GUID_SUMMARY_LIMIT: usize = 8;
const CWS_FRAME_RING_LIMIT: usize = 100;

#[derive(Debug, Clone)]
pub struct CwsHandle {
    cmd_tx: mpsc::Sender<SessionCommand>,
    instrument_group: String,
    health: Arc<RwLock<HealthState>>,
    recycle_guard: Arc<Mutex<()>>,
}

#[derive(Debug)]
struct CwsCommand {
    payload: Value,
    request_id: Option<String>,
    symbol: Option<String>,
    resp_tx: oneshot::Sender<anyhow::Result<Value>>,
}

#[derive(Debug)]
enum SessionCommand {
    Send(CwsCommand),
    Recycle(CwsRecycleCommand),
}

#[derive(Debug)]
struct CwsRecycleCommand {
    reason: String,
}

#[derive(Debug)]
struct PendingRequest {
    request_id: Option<String>,
    cws_guid: String,
    opcode: String,
    order_id: Option<String>,
    symbol: Option<String>,
    send_seq: u64,
    send_ts_utc: i64,
    send_at: Instant,
    pending_count_before_send: u64,
    resp_tx: oneshot::Sender<anyhow::Result<Value>>,
}

#[derive(Debug, Clone)]
struct CwsTelemetrySnapshot {
    stack_name: Option<String>,
    gateway_instance_id: Option<String>,
    auth_principal_fingerprint: Option<String>,
    cws_connection_instance_id: Option<String>,
    connect_seq: u64,
    reconnect_seq: u64,
    connected_ts_utc: Option<i64>,
    connection_age_ms: Option<u64>,
    time_since_last_reconnect_ms: Option<u64>,
    in_flight_pending_count: u64,
    last_successful_send_ts_utc: Option<i64>,
    last_successful_ack_ts_utc: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ControlPathRecycleOutcome {
    pub recycled: bool,
    pub stale_reason: Option<String>,
    pub stale_for_ms: Option<u64>,
    pub previous_connection_instance_id: Option<String>,
    pub current_connection_instance_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ControlPathRecycleFailure {
    code: &'static str,
    message: String,
}

impl ControlPathRecycleFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone)]
struct RequestedRecycle {
    reason: String,
    previous_connection_instance_id: Option<String>,
}

impl RequestedRecycle {
    fn new(reason: impl Into<String>, previous_connection_instance_id: Option<String>) -> Self {
        Self {
            reason: reason.into(),
            previous_connection_instance_id,
        }
    }
}

impl fmt::Display for RequestedRecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "control path recycle requested: {}",
            self.reason
        )
    }
}

impl std::error::Error for RequestedRecycle {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawCwsMessageClass {
    RequestResponse,
    DomainEvent,
    Transport,
    Unknown,
}

impl RawCwsMessageClass {
    fn as_str(&self) -> &'static str {
        match self {
            Self::RequestResponse => "response",
            Self::DomainEvent => "event",
            Self::Transport => "transport",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
struct InboundCwsMessageMeta {
    message_class: RawCwsMessageClass,
    guid: Option<String>,
    request_guid: Option<String>,
    correlation_guid: Option<String>,
    opcode: Option<String>,
    order_id: Option<String>,
    symbol: Option<String>,
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
    order_id: Option<String>,
    symbol: Option<String>,
    transport: CwsTransportFailure,
}

impl CwsRequestFailure {
    pub(crate) fn new(
        request_id: Option<String>,
        cws_guid: String,
        opcode: String,
        order_id: Option<String>,
        symbol: Option<String>,
        transport: CwsTransportFailure,
    ) -> Self {
        Self {
            request_id,
            cws_guid,
            opcode,
            order_id,
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

    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref()
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
        let session_health = Arc::clone(&health);
        {
            let mut guard = health.write();
            guard.control_path_stale_after_sec = cfg.control_path_stale_after_sec;
        }

        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(cfg.backoff_initial_ms);
            let mut connect_seq = 0_u64;
            let mut reconnect_seq = 0_u64;
            loop {
                connect_seq = connect_seq.saturating_add(1);
                match run_session(
                    &cfg,
                    &token_provider,
                    &mut cmd_rx,
                    &session_health,
                    connect_seq,
                    reconnect_seq,
                )
                .await
                {
                    Ok(()) => break,
                    Err(error) => {
                        let requested_recycle = error.downcast_ref::<RequestedRecycle>().cloned();
                        {
                            let mut guard = session_health.write();
                            guard.cws_authorized = false;
                            reconnect_seq = reconnect_seq.saturating_add(1);
                            guard.cws_reconnect_seq = reconnect_seq;
                            guard.cws_reconnect_total = guard.cws_reconnect_total.saturating_add(1);
                            guard.cws_reconnects_total =
                                guard.cws_reconnects_total.saturating_add(1);
                        }
                        if let Some(recycle) = requested_recycle {
                            info!(
                                reason = recycle.reason,
                                previous_cws_connection_instance_id = ?recycle.previous_connection_instance_id,
                                reconnect_seq,
                                "cws recycle requested; reconnecting"
                            );
                            backoff = Duration::from_millis(cfg.backoff_initial_ms);
                        } else {
                            warn!(?error, "cws session error; reconnecting");
                            tokio::time::sleep(jittered(backoff)).await;
                            backoff = next_backoff(backoff, &cfg);
                        }
                    }
                }
            }
        });

        CwsHandle {
            cmd_tx,
            instrument_group,
            health,
            recycle_guard: Arc::new(Mutex::new(())),
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
        let telemetry = {
            let mut guard = self.health.write();
            guard.cws_limit_send_total = guard.cws_limit_send_total.saturating_add(1);
            guard.cws_last_limit_send_ts_utc = Some(Utc::now().timestamp());
            snapshot_cws_telemetry(&guard)
        };
        info!(
            action = "cws_limit_send",
            opcode = "create:limit",
            request_id = ?request_id,
            cws_guid,
            stack_name = ?telemetry.stack_name,
            gateway_instance_id = ?telemetry.gateway_instance_id,
            auth_principal_fingerprint = ?telemetry.auth_principal_fingerprint,
            cws_connection_instance_id = ?telemetry.cws_connection_instance_id,
            connect_seq = telemetry.connect_seq,
            reconnect_seq = telemetry.reconnect_seq,
            connected_ts_utc = ?telemetry.connected_ts_utc,
            connection_age_ms = ?telemetry.connection_age_ms,
            time_since_last_reconnect_ms = ?telemetry.time_since_last_reconnect_ms,
            in_flight_pending_count = telemetry.in_flight_pending_count,
            last_successful_send_ts_utc = ?telemetry.last_successful_send_ts_utc,
            last_successful_ack_ts_utc = ?telemetry.last_successful_ack_ts_utc,
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
            .send(SessionCommand::Send(CwsCommand {
                payload,
                request_id: request_id.map(ToString::to_string),
                symbol: symbol.map(ToString::to_string),
                resp_tx,
            }))
            .await
            .map_err(|_| anyhow::anyhow!("cws command channel closed"))?;
        let response = tokio::time::timeout(Duration::from_secs(5), resp_rx)
            .await
            .map_err(|_| anyhow::anyhow!("cws response timeout"))?;
        let response = response.map_err(|_| anyhow::anyhow!("cws response channel closed"))??;
        Ok(response)
    }

    pub async fn ensure_fresh_control_path_for_live_limit_entry(
        &self,
        request_id: uuid::Uuid,
        recycle_timeout: Duration,
        log_only: bool,
    ) -> Result<ControlPathRecycleOutcome, ControlPathRecycleFailure> {
        let initial = {
            let guard = self.health.read();
            let stale = evaluate_control_path_stale(&guard);
            (
                stale,
                guard.cws_connection_instance_id.clone(),
                guard.gateway_instance_id.clone(),
                guard.control_path_stale_after_sec,
                guard.cws_connect_seq,
                guard.cws_reconnect_seq,
            )
        };
        if !initial.0.stale {
            return Ok(ControlPathRecycleOutcome {
                recycled: false,
                stale_reason: None,
                stale_for_ms: None,
                previous_connection_instance_id: initial.1.clone(),
                current_connection_instance_id: initial.1,
            });
        }

        {
            let mut guard = self.health.write();
            guard.control_path_stale_detected_total =
                guard.control_path_stale_detected_total.saturating_add(1);
        }
        warn!(
            request_id = %request_id,
            action = "control_path_stale_detected",
            cws_connection_instance_id = ?initial.1,
            gateway_instance_id = ?initial.2,
            control_path_stale_after_sec = initial.3,
            cws_connect_seq = initial.4,
            cws_reconnect_seq = initial.5,
            control_path_stale_reason = ?initial.0.reason,
            control_path_stale_for_ms = ?initial.0.stale_for_ms,
            cws_last_tx_age_ms = ?initial.0.last_tx_age_ms,
            cws_last_control_success_age_ms = ?initial.0.last_control_success_age_ms,
            cws_last_rx_age_ms = ?initial.0.last_rx_age_ms,
            "control_path_stale_detected"
        );
        if log_only {
            return Ok(ControlPathRecycleOutcome {
                recycled: false,
                stale_reason: initial.0.reason.map(str::to_string),
                stale_for_ms: initial.0.stale_for_ms,
                previous_connection_instance_id: initial.1.clone(),
                current_connection_instance_id: initial.1,
            });
        }

        let _recycle_guard = self.recycle_guard.lock().await;
        let current = {
            let guard = self.health.read();
            let stale = evaluate_control_path_stale(&guard);
            (
                stale,
                guard.cws_connection_instance_id.clone(),
                guard.gateway_instance_id.clone(),
                guard.control_path_stale_after_sec,
                guard.cws_connect_seq,
                guard.cws_reconnect_seq,
            )
        };
        if !current.0.stale {
            return Ok(ControlPathRecycleOutcome {
                recycled: false,
                stale_reason: None,
                stale_for_ms: None,
                previous_connection_instance_id: current.1.clone(),
                current_connection_instance_id: current.1,
            });
        }

        {
            let mut guard = self.health.write();
            guard.control_path_recycle_total = guard.control_path_recycle_total.saturating_add(1);
        }
        info!(
            request_id = %request_id,
            action = "control_path_recycle_start",
            cws_connection_instance_id = ?current.1,
            gateway_instance_id = ?current.2,
            control_path_stale_after_sec = current.3,
            cws_connect_seq = current.4,
            cws_reconnect_seq = current.5,
            control_path_stale_reason = ?current.0.reason,
            control_path_stale_for_ms = ?current.0.stale_for_ms,
            recycle_timeout_ms = recycle_timeout.as_millis() as u64,
            "control_path_recycle_start"
        );
        self.cmd_tx
            .send(SessionCommand::Recycle(CwsRecycleCommand {
                reason: current
                    .0
                    .reason
                    .unwrap_or("control_path_stale")
                    .to_string(),
            }))
            .await
            .map_err(|_| {
                let mut guard = self.health.write();
                guard.control_path_recycle_failed_total = guard
                    .control_path_recycle_failed_total
                    .saturating_add(1);
                guard.control_path_stale_blocked_send_total = guard
                    .control_path_stale_blocked_send_total
                    .saturating_add(1);
                ControlPathRecycleFailure::new(
                    "control_path_recycle_failed",
                    "cws recycle command channel closed",
                )
            })?;

        let started_at = Instant::now();
        let previous_connection_instance_id = current.1.clone();
        loop {
            let snapshot = {
                let guard = self.health.read();
                let stale = evaluate_control_path_stale(&guard);
                (
                    guard.readiness,
                    guard.cws_authorized,
                    guard.cws_connection_instance_id.clone(),
                    stale,
                    guard.cws_connect_seq,
                    guard.cws_reconnect_seq,
                )
            };
            let new_connection_ready = snapshot.0
                && snapshot.1
                && snapshot.2.is_some()
                && snapshot.2 != previous_connection_instance_id
                && !snapshot.3.stale;
            if new_connection_ready {
                let elapsed_ms = started_at.elapsed().as_millis() as u64;
                {
                    let mut guard = self.health.write();
                    guard.control_path_recycle_success_total = guard
                        .control_path_recycle_success_total
                        .saturating_add(1);
                }
                info!(
                    request_id = %request_id,
                    action = "control_path_recycle_success",
                    previous_cws_connection_instance_id = ?previous_connection_instance_id,
                    current_cws_connection_instance_id = ?snapshot.2,
                    cws_connect_seq = snapshot.4,
                    cws_reconnect_seq = snapshot.5,
                    elapsed_ms,
                    "control_path_recycle_success"
                );
                return Ok(ControlPathRecycleOutcome {
                    recycled: true,
                    stale_reason: current.0.reason.map(str::to_string),
                    stale_for_ms: current.0.stale_for_ms,
                    previous_connection_instance_id,
                    current_connection_instance_id: snapshot.2,
                });
            }

            if started_at.elapsed() >= recycle_timeout {
                let error = ControlPathRecycleFailure::new(
                    "control_path_recycle_failed",
                    "fresh cws session was not ready before recycle timeout",
                );
                {
                    let mut guard = self.health.write();
                    guard.control_path_recycle_failed_total = guard
                        .control_path_recycle_failed_total
                        .saturating_add(1);
                    guard.control_path_stale_blocked_send_total = guard
                        .control_path_stale_blocked_send_total
                        .saturating_add(1);
                }
                warn!(
                    request_id = %request_id,
                    action = "control_path_recycle_failed",
                    previous_cws_connection_instance_id = ?previous_connection_instance_id,
                    current_cws_connection_instance_id = ?snapshot.2,
                    cws_connect_seq = snapshot.4,
                    cws_reconnect_seq = snapshot.5,
                    recycle_timeout_ms = recycle_timeout.as_millis() as u64,
                    error = error.message(),
                    "control_path_recycle_failed"
                );
                return Err(error);
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(8);
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::Send(cmd) = cmd {
                    let _ = cmd.resp_tx.send(Ok(serde_json::json!({})));
                }
            }
        });
        CwsHandle {
            cmd_tx,
            instrument_group: "TEST".to_string(),
            health: Arc::new(RwLock::new(HealthState::default())),
            recycle_guard: Arc::new(Mutex::new(())),
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

fn elapsed_ms(since: Option<Instant>) -> Option<u64> {
    since.map(|instant| {
        let elapsed_ms = instant.elapsed().as_millis();
        u64::try_from(elapsed_ms).unwrap_or(u64::MAX)
    })
}

fn snapshot_cws_telemetry(guard: &HealthState) -> CwsTelemetrySnapshot {
    CwsTelemetrySnapshot {
        stack_name: guard.stack_name.clone(),
        gateway_instance_id: guard.gateway_instance_id.clone(),
        auth_principal_fingerprint: guard.auth_principal_fingerprint.clone(),
        cws_connection_instance_id: guard.cws_connection_instance_id.clone(),
        connect_seq: guard.cws_connect_seq,
        reconnect_seq: guard.cws_reconnect_seq,
        connected_ts_utc: guard.cws_connected_ts_utc,
        connection_age_ms: elapsed_ms(guard.cws_connected_at),
        time_since_last_reconnect_ms: elapsed_ms(guard.cws_last_reconnect_at),
        in_flight_pending_count: guard.cws_pending_count,
        last_successful_send_ts_utc: guard.cws_last_successful_send_ts_utc,
        last_successful_ack_ts_utc: guard.cws_last_successful_ack_ts_utc,
    }
}

fn record_access_token_snapshot(
    health: &Arc<RwLock<HealthState>>,
    consumer: &str,
    snapshot: &AccessTokenSnapshot,
) {
    let mut guard = health.write();
    guard.access_token_fingerprint = Some(snapshot.fingerprint().to_string());
    guard.access_token_last_source = Some(snapshot.source().as_str().to_string());
    guard.access_token_last_consumer = Some(consumer.to_string());
    guard.access_token_obtained_ts_utc = snapshot.obtained_ts_utc();
    guard.access_token_last_used_ts_utc = Some(Utc::now().timestamp());
    guard.access_token_age_ms = snapshot.age_ms();
    guard.access_token_ttl_remaining_ms = snapshot.ttl_remaining_ms();
    guard.token_refresh_count = snapshot.refresh_count();
}

fn control_opcode_key(opcode: &str) -> Option<&'static str> {
    match opcode {
        "create:limit" => Some("create:limit"),
        "delete:limit" => Some("delete:limit"),
        "replace:limit" => Some("replace:limit"),
        _ => None,
    }
}

fn update_control_send_counters(guard: &mut HealthState, opcode: &str, ts_utc: i64, at: Instant) {
    guard.cws_last_tx_ts_utc = Some(ts_utc);
    guard.cws_last_tx_at = Some(at);
    match control_opcode_key(opcode) {
        Some("create:limit") => {
            guard.cws_create_limit_send_total =
                guard.cws_create_limit_send_total.saturating_add(1);
        }
        Some("delete:limit") => {
            guard.cws_delete_limit_send_total =
                guard.cws_delete_limit_send_total.saturating_add(1);
        }
        Some("replace:limit") => {
            guard.cws_replace_limit_send_total =
                guard.cws_replace_limit_send_total.saturating_add(1);
        }
        _ => {}
    }
}

fn update_control_result_counters(
    guard: &mut HealthState,
    opcode: &str,
    success: bool,
    ts_utc: i64,
    at: Instant,
) {
    if success {
        guard.cws_last_control_success_ts_utc = Some(ts_utc);
        guard.cws_last_control_success_at = Some(at);
    } else {
        guard.cws_last_control_failure_ts_utc = Some(ts_utc);
        guard.cws_last_control_failure_at = Some(at);
    }
    match (control_opcode_key(opcode), success) {
        (Some("create:limit"), true) => {
            guard.cws_create_limit_success_total =
                guard.cws_create_limit_success_total.saturating_add(1);
        }
        (Some("create:limit"), false) => {
            guard.cws_create_limit_failure_total =
                guard.cws_create_limit_failure_total.saturating_add(1);
        }
        (Some("delete:limit"), true) => {
            guard.cws_delete_limit_success_total =
                guard.cws_delete_limit_success_total.saturating_add(1);
        }
        (Some("delete:limit"), false) => {
            guard.cws_delete_limit_failure_total =
                guard.cws_delete_limit_failure_total.saturating_add(1);
        }
        (Some("replace:limit"), true) => {
            guard.cws_replace_limit_success_total =
                guard.cws_replace_limit_success_total.saturating_add(1);
        }
        (Some("replace:limit"), false) => {
            guard.cws_replace_limit_failure_total =
                guard.cws_replace_limit_failure_total.saturating_add(1);
        }
        _ => {}
    }
}

fn record_frame(
    frames: &mut Vec<CwsFrameTraceEntry>,
    entry: CwsFrameTraceEntry,
) {
    frames.push(entry);
    if frames.len() > CWS_FRAME_RING_LIMIT {
        let remove_count = frames.len() - CWS_FRAME_RING_LIMIT;
        frames.drain(0..remove_count);
    }
}

fn record_outbound_frame(
    health: &Arc<RwLock<HealthState>>,
    connection_instance_id: &str,
    seq: u64,
    ts_utc: i64,
    opcode: &str,
    guid: &str,
    order_id: Option<&str>,
    symbol: Option<&str>,
) {
    let mut guard = health.write();
    record_frame(
        &mut guard.cws_recent_outbound_frames,
        CwsFrameTraceEntry {
            ts_utc,
            seq,
            direction: "outbound".to_string(),
            conn_instance_id: Some(connection_instance_id.to_string()),
            opcode: Some(opcode.to_string()),
            guid: Some(guid.to_string()),
            request_guid: None,
            correlation_guid: Some(guid.to_string()),
            order_id: order_id.map(ToString::to_string),
            symbol: symbol.map(ToString::to_string),
            message_class: "command".to_string(),
        },
    );
}

fn record_inbound_frame(
    health: &Arc<RwLock<HealthState>>,
    connection_instance_id: &str,
    seq: u64,
    ts_utc: i64,
    meta: &InboundCwsMessageMeta,
    at: Instant,
) {
    let mut guard = health.write();
    guard.cws_last_rx_ts_utc = Some(ts_utc);
    guard.cws_last_rx_at = Some(at);
    record_frame(
        &mut guard.cws_recent_inbound_frames,
        CwsFrameTraceEntry {
            ts_utc,
            seq,
            direction: "inbound".to_string(),
            conn_instance_id: Some(connection_instance_id.to_string()),
            opcode: meta.opcode.clone(),
            guid: meta.guid.clone(),
            request_guid: meta.request_guid.clone(),
            correlation_guid: meta.correlation_guid.clone(),
            order_id: meta.order_id.clone(),
            symbol: meta.symbol.clone(),
            message_class: meta.message_class.as_str().to_string(),
        },
    );
}

fn sync_pending_health(health: &Arc<RwLock<HealthState>>, pending: &HashMap<String, PendingRequest>) {
    let mut pending_items = pending.values().collect::<Vec<_>>();
    pending_items.sort_by_key(|request| request.send_seq);

    let pending_guids = pending_items
        .iter()
        .take(CWS_PENDING_GUID_SUMMARY_LIMIT)
        .map(|request| request.cws_guid.clone())
        .collect::<Vec<_>>();
    let oldest_pending_age_ms = pending_items
        .first()
        .map(|request| request.send_at.elapsed().as_millis() as u64);

    let mut guard = health.write();
    guard.cws_pending_count = pending.len() as u64;
    guard.cws_pending_guids = pending_guids;
    guard.cws_oldest_pending_age_ms = oldest_pending_age_ms;
}

async fn run_session(
    cfg: &AlorGatewayConfig,
    token_provider: &TokenProvider,
    cmd_rx: &mut mpsc::Receiver<SessionCommand>,
    health: &Arc<RwLock<HealthState>>,
    connect_seq: u64,
    reconnect_seq: u64,
) -> anyhow::Result<()> {
    let token_snapshot = token_provider
        .access_token_with_context("cws_run_session")
        .await?;
    record_access_token_snapshot(health, "cws_run_session", &token_snapshot);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&cfg.cws_url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();
    let connection_instance_id = new_guid();
    let connected_at = Instant::now();
    let connected_ts_utc = Utc::now().timestamp();

    {
        let mut guard = health.write();
        guard.cws_authorized = false;
        guard.cws_connection_instance_id = Some(connection_instance_id.clone());
        guard.cws_connect_seq = connect_seq;
        guard.cws_reconnect_seq = reconnect_seq;
        guard.cws_connected_ts_utc = Some(connected_ts_utc);
        guard.cws_last_connect_ts_utc = Some(connected_ts_utc);
        guard.cws_connected_at = Some(connected_at);
        guard.cws_last_reconnect_at = (reconnect_seq > 0).then_some(connected_at);
        guard.cws_pending_count = 0;
        guard.cws_pending_guids.clear();
        guard.cws_oldest_pending_age_ms = None;
        guard.cws_recent_inbound_frames.clear();
        guard.cws_recent_outbound_frames.clear();
        guard.cws_last_rx_ts_utc = None;
        guard.cws_last_tx_ts_utc = None;
        guard.cws_last_rx_at = None;
        guard.cws_last_tx_at = None;
        guard.cws_last_ping_ts_utc = None;
        guard.cws_last_pong_ts_utc = None;
        guard.cws_last_ping_at = None;
        guard.cws_last_pong_at = None;
        guard.cws_connect_total = guard.cws_connect_total.saturating_add(1);
    }
    authorize(
        &mut ws_sink,
        &mut ws_stream,
        token_provider,
        &token_snapshot,
        health,
    )
    .await?;

    let mut pending: HashMap<String, PendingRequest> = HashMap::new();
    let mut send_seq = 0_u64;
    let mut recv_seq = 0_u64;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SessionCommand::Send(cmd)) => {
                        let pending_count_before_send = pending.len() as u64;
                        send_seq = send_seq.saturating_add(1);
                        let send_at = Instant::now();
                        let send_ts_utc = Utc::now().timestamp();
                        let guid = cmd
                            .payload
                            .get("guid")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                            .unwrap_or_else(new_guid);
                        let mut payload = cmd.payload;
                        if let Some(map) = payload.as_object_mut() {
                            map.insert("guid".to_string(), Value::String(guid.clone()));
                            map.insert(
                                "token".to_string(),
                                Value::String(token_snapshot.token().to_string()),
                            );
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
                                order_id: order_id_of_value(&payload),
                                symbol: cmd.symbol.or_else(|| symbol_of_payload(&payload)),
                                send_seq,
                                send_ts_utc,
                                send_at,
                                pending_count_before_send,
                                resp_tx: cmd.resp_tx,
                            },
                        );
                        sync_pending_health(health, &pending);
                        {
                            let mut guard = health.write();
                            update_control_send_counters(&mut guard, &opcode, send_ts_utc, send_at);
                        }
                        record_outbound_frame(
                            health,
                            &connection_instance_id,
                            send_seq,
                            send_ts_utc,
                            &opcode,
                            &guid,
                            pending.get(&guid).and_then(|request| request.order_id.as_deref()),
                            pending.get(&guid).and_then(|request| request.symbol.as_deref()),
                        );
                        let connection_age_ms = connected_at.elapsed().as_millis() as u64;
                        info!(
                            handler = "socket_writer",
                            state_before = "ready_to_send",
                            state_after = "pending_registered",
                            opcode,
                            guid,
                            send_seq,
                            send_ts_utc,
                            request_id = pending.get(&guid).and_then(|request| request.request_id.as_deref()),
                            order_id = pending.get(&guid).and_then(|request| request.order_id.as_deref()),
                            cws_connection_instance_id = %connection_instance_id,
                            connect_seq,
                            reconnect_seq,
                            connection_age_ms,
                            pending_count_before_send,
                            "cws send"
                        );
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
                            let session_error = handle_transport_failure(&mut pending, failure, health);
                            return Err(session_error);
                        }
                        let now_ts_utc = Utc::now().timestamp();
                        let mut guard = health.write();
                        guard.cws_last_successful_send_ts_utc = Some(now_ts_utc);
                    }
                    Some(SessionCommand::Recycle(recycle)) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        info!(
                            handler = "socket_writer",
                            state_before = "session_ready",
                            state_after = "recycle_requested",
                            reason = recycle.reason,
                            pending_count = pending.len(),
                            cws_connection_instance_id = %connection_instance_id,
                            connect_seq,
                            reconnect_seq,
                            "cws recycle requested"
                        );
                        let _ = ws_sink.send(Message::Close(None)).await;
                        return Err(anyhow::Error::new(RequestedRecycle::new(
                            recycle.reason,
                            Some(connection_instance_id.clone()),
                        )));
                    }
                    None => return Ok(()),
                }
            }
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        recv_seq = recv_seq.saturating_add(1);
                        let recv_at = Instant::now();
                        let recv_ts_utc = Utc::now().timestamp();
                        if let Ok(value) = serde_json::from_str::<Value>(&txt) {
                            let meta = inspect_inbound_cws_message(&value);
                            record_inbound_frame(
                                health,
                                &connection_instance_id,
                                recv_seq,
                                recv_ts_utc,
                                &meta,
                                recv_at,
                            );
                            let opcode = meta.opcode.as_deref().unwrap_or("unknown");
                            debug!(
                                handler = "socket_reader",
                                state_before = "frame_received",
                                state_after = "message_classified",
                                recv_seq,
                                recv_ts_utc,
                                cws_connection_instance_id = %connection_instance_id,
                                connect_seq,
                                reconnect_seq,
                                raw_message_class = meta.message_class.as_str(),
                                opcode,
                                guid = ?meta.guid,
                                request_guid = ?meta.request_guid,
                                correlation_guid = ?meta.correlation_guid,
                                order_id = ?meta.order_id,
                                symbol = ?meta.symbol,
                                payload = %value,
                                "cws recv payload"
                            );
                            if matches!(meta.message_class, RawCwsMessageClass::RequestResponse) {
                                if let Some(guid) = meta.correlation_guid.clone() {
                                    if let Some(request) = pending.remove(&guid) {
                                        let pending_count_before = pending.len().saturating_add(1);
                                        let pending_count_after = pending.len();
                                        let mut value = value;
                                        attach_diag_trace_fields(
                                            &mut value,
                                            &meta,
                                            &request,
                                            recv_seq,
                                            recv_ts_utc,
                                            &connection_instance_id,
                                            connect_seq,
                                            reconnect_seq,
                                        );
                                        let response_success = response_is_success(&value);
                                        info!(
                                            handler = "pending_resolver",
                                            state_before = "pending_open",
                                            state_after = "pending_resolved_response",
                                            recv_seq,
                                            recv_ts_utc,
                                            send_seq = request.send_seq,
                                            send_ts_utc = request.send_ts_utc,
                                            request_id = request.request_id.as_deref(),
                                            cws_guid = request.cws_guid.as_str(),
                                            order_id = meta.order_id.as_deref().or(request.order_id.as_deref()),
                                            message_class = meta.message_class.as_str(),
                                            pending_count_before,
                                            pending_count_after,
                                            "cws response matched pending request"
                                        );
                                        {
                                            let mut guard = health.write();
                                            guard.cws_last_successful_ack_ts_utc =
                                                Some(Utc::now().timestamp());
                                            update_control_result_counters(
                                                &mut guard,
                                                &request.opcode,
                                                response_success,
                                                recv_ts_utc,
                                                recv_at,
                                            );
                                        }
                                        sync_pending_health(health, &pending);
                                        let _ = request.resp_tx.send(Ok(value));
                                    } else {
                                        warn!(
                                            handler = "pending_resolver",
                                            state_before = "pending_missing",
                                            state_after = "pending_missing",
                                            recv_seq,
                                            recv_ts_utc,
                                            opcode,
                                            guid,
                                            raw_message_class = meta.message_class.as_str(),
                                            "cws recv response without pending request"
                                        );
                                    }
                                } else {
                                    warn!(
                                        handler = "pending_resolver",
                                        state_before = "response_unmatched",
                                        state_after = "response_unmatched",
                                        recv_seq,
                                        recv_ts_utc,
                                        opcode,
                                        raw_message_class = meta.message_class.as_str(),
                                        "cws recv response without correlation guid"
                                    );
                                }
                            } else {
                                let matched_request = meta
                                    .correlation_guid
                                    .as_ref()
                                    .and_then(|guid| pending.get(guid));
                                if let Some(request) = matched_request {
                                    warn!(
                                        handler = "pending_resolver",
                                        state_before = "pending_open",
                                        state_after = "pending_open",
                                        recv_seq,
                                        recv_ts_utc,
                                        opcode,
                                        raw_message_class = meta.message_class.as_str(),
                                        correlation_guid = ?meta.correlation_guid,
                                        request_id = request.request_id.as_deref(),
                                        order_id = meta.order_id.as_deref().or(request.order_id.as_deref()),
                                        pending_count = pending.len(),
                                        "cws recv non-response frame matched pending guid; pending left open"
                                    );
                                }
                            }
                        } else {
                            warn!(
                                recv_seq,
                                recv_ts_utc,
                                cws_connection_instance_id = %connection_instance_id,
                                connect_seq,
                                reconnect_seq,
                                payload = %txt,
                                "cws recv non-json payload"
                            );
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        let now_at = Instant::now();
                        let now_ts_utc = Utc::now().timestamp();
                        let mut guard = health.write();
                        guard.cws_last_ping_ts_utc = Some(now_ts_utc);
                        guard.cws_last_ping_at = Some(now_at);
                    }
                    Some(Ok(Message::Pong(_))) => {
                        let now_at = Instant::now();
                        let now_ts_utc = Utc::now().timestamp();
                        let mut guard = health.write();
                        guard.cws_last_pong_ts_utc = Some(now_ts_utc);
                        guard.cws_last_pong_at = Some(now_at);
                    }
                    Some(Ok(Message::Close(frame))) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        let failure = transport_failure_from_close_frame(frame);
                        let session_error = handle_transport_failure(&mut pending, failure, health);
                        return Err(session_error);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        let failure = transport_failure_from_receive_error(&error);
                        let session_error = handle_transport_failure(&mut pending, failure, health);
                        return Err(session_error);
                    }
                    None => {
                        {
                            let mut guard = health.write();
                            guard.cws_authorized = false;
                        }
                        let failure = transport_failure_from_eof();
                        let session_error = handle_transport_failure(&mut pending, failure, health);
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
    token_provider: &TokenProvider,
    token_snapshot: &AccessTokenSnapshot,
    health: &Arc<RwLock<HealthState>>,
) -> anyhow::Result<()> {
    record_access_token_snapshot(health, "cws_authorize", token_snapshot);
    let guid = new_guid();
    let send_at = Instant::now();
    let send_ts_utc = Utc::now().timestamp();
    let payload = serde_json::json!({
        "opcode": "authorize",
        "guid": guid,
        "token": token_snapshot.token(),
    });
    {
        let mut guard = health.write();
        guard.cws_last_tx_ts_utc = Some(send_ts_utc);
        guard.cws_last_tx_at = Some(send_at);
        let connection_instance_id = guard.cws_connection_instance_id.clone();
        record_frame(
            &mut guard.cws_recent_outbound_frames,
            CwsFrameTraceEntry {
                ts_utc: send_ts_utc,
                seq: 0,
                direction: "outbound".to_string(),
                conn_instance_id: connection_instance_id,
                opcode: Some("authorize".to_string()),
                guid: Some(guid.clone()),
                request_guid: None,
                correlation_guid: Some(guid.clone()),
                order_id: None,
                symbol: None,
                message_class: "authorize".to_string(),
            },
        );
    }
    info!(
        guid,
        label = "authorize",
        auth_principal_fingerprint = token_provider.principal_fingerprint(),
        access_token_fingerprint = token_snapshot.fingerprint(),
        access_token_source = token_snapshot.source().as_str(),
        token_refresh_count = token_snapshot.refresh_count(),
        access_token_obtained_ts_utc = token_snapshot.obtained_ts_utc(),
        access_token_age_ms = token_snapshot.age_ms(),
        access_token_ttl_remaining_ms = token_snapshot.ttl_remaining_ms(),
        "ws subscribe send"
    );
    let redacted_payload = redact_token(&payload.to_string());
    debug!(payload = %redacted_payload, guid, label = "authorize", "ws subscribe payload");
    ws_sink
        .send(Message::Text(payload.to_string().into()))
        .await?;

    let response = read_until_guid(ws_stream, &guid, Duration::from_secs(5)).await?;
    let recv_at = Instant::now();
    let recv_ts_utc = Utc::now().timestamp();
    {
        let mut guard = health.write();
        guard.cws_last_rx_ts_utc = Some(recv_ts_utc);
        guard.cws_last_rx_at = Some(recv_at);
        let connection_instance_id = guard.cws_connection_instance_id.clone();
        record_frame(
            &mut guard.cws_recent_inbound_frames,
            CwsFrameTraceEntry {
                ts_utc: recv_ts_utc,
                seq: 0,
                direction: "inbound".to_string(),
                conn_instance_id: connection_instance_id,
                opcode: response
                    .get("opcode")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| Some("authorize".to_string())),
                guid: transport_guid_of(&response),
                request_guid: request_guid_of(&response),
                correlation_guid: correlation_guid_of(&response),
                order_id: order_id_of_value(&response),
                symbol: symbol_of_value(&response),
                message_class: classify_inbound_cws_message(&response).as_str().to_string(),
            },
        );
    }
    info!(
        payload = %response,
        guid,
        label = "authorize",
        auth_principal_fingerprint = token_provider.principal_fingerprint(),
        access_token_fingerprint = token_snapshot.fingerprint(),
        access_token_source = token_snapshot.source().as_str(),
        token_refresh_count = token_snapshot.refresh_count(),
        access_token_obtained_ts_utc = token_snapshot.obtained_ts_utc(),
        access_token_age_ms = token_snapshot.age_ms(),
        access_token_ttl_remaining_ms = token_snapshot.ttl_remaining_ms(),
        "ws subscribe ack"
    );
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
        if is_invalid_jwt_response(&response) {
            warn!(
                guid,
                label = "authorize",
                auth_principal_fingerprint = token_provider.principal_fingerprint(),
                access_token_fingerprint = token_snapshot.fingerprint(),
                access_token_source = token_snapshot.source().as_str(),
                token_refresh_count = token_snapshot.refresh_count(),
                access_token_obtained_ts_utc = token_snapshot.obtained_ts_utc(),
                access_token_age_ms = token_snapshot.age_ms(),
                access_token_ttl_remaining_ms = token_snapshot.ttl_remaining_ms(),
                "cws authorize rejected access token; invalidating cached token"
            );
            token_provider.invalidate("cws_authorize_401").await;
        }
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

fn is_invalid_jwt_response(response: &Value) -> bool {
    let status = response.get("status").and_then(Value::as_i64);
    let http_code = response.get("httpCode").and_then(Value::as_i64);
    if status == Some(401) || http_code == Some(401) {
        return true;
    }

    response
        .get("message")
        .and_then(Value::as_str)
        .map(|message| message.to_ascii_lowercase().contains("invalid jwt token"))
        .unwrap_or(false)
}

fn response_is_success(response: &Value) -> bool {
    let status = response.get("status").and_then(Value::as_i64);
    let http_code = response.get("httpCode").and_then(Value::as_i64);
    status == Some(200) || http_code == Some(200)
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
    health: &Arc<RwLock<HealthState>>,
) -> anyhow::Error {
    log_transport_failure(&failure, pending, health);
    fail_pending_with_transport(pending, failure.clone(), health);
    anyhow::Error::new(failure)
}

fn log_transport_failure(
    failure: &CwsTransportFailure,
    pending: &HashMap<String, PendingRequest>,
    health: &Arc<RwLock<HealthState>>,
) {
    let first = pending.values().next();
    let request_id = first.and_then(|request| request.request_id.as_deref());
    let cws_guid = first.map(|request| request.cws_guid.as_str());
    let opcode_in_flight = first.map(|request| request.opcode.as_str());
    let order_id_in_flight = first.and_then(|request| request.order_id.as_deref());
    let now_ts_utc = Utc::now().timestamp();
    let telemetry = {
        let mut guard = health.write();
        if matches!(
            failure.disconnect_kind(),
            CwsDisconnectKind::ProtocolResetWithoutCloseHandshake
        ) {
            guard.cws_protocol_reset_total = guard.cws_protocol_reset_total.saturating_add(1);
        }
        guard.cws_last_transport_failure_ts_utc = Some(now_ts_utc);
        snapshot_cws_telemetry(&guard)
    };
    warn!(
        handler = "socket_reader",
        state_before = "pending_open",
        state_after = "transport_failure_detected",
        action = "cws_transport_failure",
        disconnect_kind = failure.disconnect_kind_str(),
        opcode_in_flight = ?opcode_in_flight,
        request_id = ?request_id,
        cws_guid = ?cws_guid,
        order_id = ?order_id_in_flight,
        stack_name = ?telemetry.stack_name,
        gateway_instance_id = ?telemetry.gateway_instance_id,
        auth_principal_fingerprint = ?telemetry.auth_principal_fingerprint,
        cws_connection_instance_id = ?telemetry.cws_connection_instance_id,
        connect_seq = telemetry.connect_seq,
        reconnect_seq = telemetry.reconnect_seq,
        connected_ts_utc = ?telemetry.connected_ts_utc,
        connection_age_ms_at_failure = ?telemetry.connection_age_ms,
        time_since_last_reconnect_ms_at_failure = ?telemetry.time_since_last_reconnect_ms,
        pending_count = pending.len(),
        last_successful_send_ts_utc = ?telemetry.last_successful_send_ts_utc,
        last_successful_ack_ts_utc = ?telemetry.last_successful_ack_ts_utc,
        close_code = ?failure.close_code(),
        close_reason = ?failure.close_reason(),
        raw_error = %failure.raw_error(),
        "cws transport failure"
    );
}

fn fail_pending_with_transport(
    pending: &mut HashMap<String, PendingRequest>,
    failure: CwsTransportFailure,
    health: &Arc<RwLock<HealthState>>,
) {
    let affected_count = pending.len() as u64;
    let affected = pending
        .values()
        .map(|request| {
            serde_json::json!({
                "request_id": request.request_id.as_deref(),
                "cws_guid": request.cws_guid.as_str(),
                "opcode": request.opcode.as_str(),
                "order_id": request.order_id.as_deref(),
                "symbol": request.symbol.as_deref(),
                "send_seq": request.send_seq,
                "send_ts_utc": request.send_ts_utc,
                "pending_count_before_send": request.pending_count_before_send,
            })
        })
        .collect::<Vec<_>>();
    if !affected.is_empty() {
        let affected_json = serde_json::Value::Array(affected);
        let has_limit_request = pending
            .values()
            .any(|request| request.opcode == "create:limit");
        let now_at = Instant::now();
        let now_ts_utc = Utc::now().timestamp();
        let telemetry = {
            let mut guard = health.write();
            guard.cws_pending_failed_total = guard
                .cws_pending_failed_total
                .saturating_add(affected_count);
            guard.cws_pending_count = 0;
            guard.cws_pending_guids.clear();
            guard.cws_oldest_pending_age_ms = None;
            if has_limit_request {
                guard.cws_last_limit_error_ts_utc = Some(now_ts_utc);
            }
            for request in pending.values() {
                update_control_result_counters(
                    &mut guard,
                    &request.opcode,
                    false,
                    now_ts_utc,
                    now_at,
                );
            }
            snapshot_cws_telemetry(&guard)
        };
        warn!(
            handler = "pending_resolver",
            state_before = "pending_open",
            state_after = "pending_failed_transport",
            action = "cws_fail_pending",
            disconnect_kind = failure.disconnect_kind_str(),
            stack_name = ?telemetry.stack_name,
            gateway_instance_id = ?telemetry.gateway_instance_id,
            auth_principal_fingerprint = ?telemetry.auth_principal_fingerprint,
            cws_connection_instance_id = ?telemetry.cws_connection_instance_id,
            connect_seq = telemetry.connect_seq,
            reconnect_seq = telemetry.reconnect_seq,
            connected_ts_utc = ?telemetry.connected_ts_utc,
            connection_age_ms_at_failure = ?telemetry.connection_age_ms,
            time_since_last_reconnect_ms_at_failure = ?telemetry.time_since_last_reconnect_ms,
            pending_count = affected_json.as_array().map_or(0, Vec::len),
            last_successful_send_ts_utc = ?telemetry.last_successful_send_ts_utc,
            last_successful_ack_ts_utc = ?telemetry.last_successful_ack_ts_utc,
            affected = %affected_json,
            "failing pending cws requests after transport failure"
        );
    }
    for (_, request) in pending.drain() {
        let error = CwsRequestFailure::new(
            request.request_id,
            request.cws_guid,
            request.opcode,
            request.order_id,
            request.symbol,
            failure.clone(),
        );
        let _ = request.resp_tx.send(Err(anyhow::Error::new(error)));
    }
    sync_pending_health(health, pending);
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

fn request_guid_of(value: &Value) -> Option<String> {
    value
        .get("requestGuid")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn transport_guid_of(value: &Value) -> Option<String> {
    value
        .get("guid")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn correlation_guid_of(value: &Value) -> Option<String> {
    request_guid_of(value).or_else(|| transport_guid_of(value))
}

fn order_id_of_value(value: &Value) -> Option<String> {
    value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(value_to_string)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(value_to_string)
            })
        })
}

fn symbol_of_value(value: &Value) -> Option<String> {
    value
        .get("symbol")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("symbol")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
        })
        .or_else(|| symbol_of_payload(value))
}

fn classify_inbound_cws_message(value: &Value) -> RawCwsMessageClass {
    let has_http_code = value.get("httpCode").is_some();
    let has_request_guid = value.get("requestGuid").is_some();
    let has_numeric_status = value
        .get("status")
        .map(|status| status.is_i64() || status.is_u64() || status.is_f64())
        .unwrap_or(false);
    let has_authorize_flag =
        value.get("cwsAuthorized").is_some() || value.get("cws_authorized").is_some();
    if has_http_code || has_request_guid || has_numeric_status || has_authorize_flag {
        return RawCwsMessageClass::RequestResponse;
    }

    let has_order_marker = value.get("orderId").is_some()
        || value.get("orderNumber").is_some()
        || value
            .get("data")
            .and_then(Value::as_object)
            .map(|data| {
                data.contains_key("orderId")
                    || data.contains_key("orderNumber")
                    || data.contains_key("symbol")
                    || data.contains_key("existing")
                    || data.contains_key("status")
            })
            .unwrap_or(false);
    if has_order_marker {
        return RawCwsMessageClass::DomainEvent;
    }

    let has_transport_marker =
        value.get("event").is_some() || value.get("type").is_some() || value.get("code").is_some();
    if has_transport_marker {
        return RawCwsMessageClass::Transport;
    }

    RawCwsMessageClass::Unknown
}

fn inspect_inbound_cws_message(value: &Value) -> InboundCwsMessageMeta {
    InboundCwsMessageMeta {
        message_class: classify_inbound_cws_message(value),
        guid: transport_guid_of(value),
        request_guid: request_guid_of(value),
        correlation_guid: correlation_guid_of(value),
        opcode: value
            .get("opcode")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        order_id: order_id_of_value(value),
        symbol: symbol_of_value(value),
    }
}

fn attach_diag_trace_fields(
    value: &mut Value,
    meta: &InboundCwsMessageMeta,
    request: &PendingRequest,
    recv_seq: u64,
    recv_ts_utc: i64,
    connection_instance_id: &str,
    connect_seq: u64,
    reconnect_seq: u64,
) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.insert(DIAG_CWS_SEND_SEQ.to_string(), Value::from(request.send_seq));
    obj.insert(
        DIAG_CWS_SEND_TS_UTC.to_string(),
        Value::from(request.send_ts_utc),
    );
    obj.insert(DIAG_CWS_RECV_SEQ.to_string(), Value::from(recv_seq));
    obj.insert(DIAG_CWS_RECV_TS_UTC.to_string(), Value::from(recv_ts_utc));
    obj.insert(
        DIAG_CWS_MESSAGE_CLASS.to_string(),
        Value::String(meta.message_class.as_str().to_string()),
    );
    obj.insert(
        DIAG_CWS_CONNECTION_INSTANCE_ID.to_string(),
        Value::String(connection_instance_id.to_string()),
    );
    obj.insert(DIAG_CWS_CONNECT_SEQ.to_string(), Value::from(connect_seq));
    obj.insert(
        DIAG_CWS_RECONNECT_SEQ.to_string(),
        Value::from(reconnect_seq),
    );
    if let Some(correlation_guid) = meta.correlation_guid.as_deref() {
        obj.insert(
            DIAG_CWS_CORRELATION_GUID.to_string(),
            Value::String(correlation_guid.to_string()),
        );
    }
    if let Some(order_id) = meta.order_id.as_deref() {
        obj.insert(
            DIAG_CWS_ORDER_ID.to_string(),
            Value::String(order_id.to_string()),
        );
    } else if let Some(order_id) = request.order_id.as_deref() {
        obj.insert(
            DIAG_CWS_ORDER_ID.to_string(),
            Value::String(order_id.to_string()),
        );
    }
    if let Some(order_id) = request.order_id.as_deref() {
        obj.insert(
            DIAG_CWS_REQUEST_ORDER_ID.to_string(),
            Value::String(order_id.to_string()),
        );
    }
    if let Some(symbol) = meta.symbol.as_deref() {
        obj.insert(
            DIAG_CWS_SYMBOL.to_string(),
            Value::String(symbol.to_string()),
        );
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_string());
    }
    if let Some(value) = value.as_i64() {
        return Some(value.to_string());
    }
    if let Some(value) = value.as_u64() {
        return Some(value.to_string());
    }
    if let Some(value) = value.as_f64() {
        return Some(value.to_string());
    }
    None
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
    fn classify_inbound_cws_message_distinguishes_response_and_event() {
        let response = serde_json::json!({
            "requestGuid": "guid-1",
            "httpCode": 200,
            "message": "ok"
        });
        let event = serde_json::json!({
            "guid": "guid-1",
            "data": {
                "orderNumber": 12345,
                "symbol": "USDRUBF",
                "status": "working"
            }
        });
        assert_eq!(
            classify_inbound_cws_message(&response),
            RawCwsMessageClass::RequestResponse
        );
        assert_eq!(
            classify_inbound_cws_message(&event),
            RawCwsMessageClass::DomainEvent
        );
    }

    #[test]
    fn attach_diag_trace_fields_preserves_send_and_recv_correlation() {
        let mut response = serde_json::json!({
            "requestGuid": "guid-1",
            "httpCode": 200
        });
        let meta = inspect_inbound_cws_message(&response);
        let (resp_tx, _resp_rx) = oneshot::channel();
        let request = PendingRequest {
            request_id: Some("req-1".to_string()),
            cws_guid: "guid-1".to_string(),
            opcode: "create:limit".to_string(),
            order_id: Some("2023555931497048623".to_string()),
            symbol: Some("USDRUBF".to_string()),
            send_seq: 7,
            send_ts_utc: 1774285000,
            send_at: Instant::now(),
            pending_count_before_send: 0,
            resp_tx,
        };
        attach_diag_trace_fields(
            &mut response,
            &meta,
            &request,
            8,
            1774285001,
            "conn-1",
            3,
            2,
        );
        assert_eq!(
            response.get(DIAG_CWS_SEND_SEQ).and_then(Value::as_u64),
            Some(7)
        );
        assert_eq!(
            response.get(DIAG_CWS_RECV_SEQ).and_then(Value::as_u64),
            Some(8)
        );
        assert_eq!(
            response.get(DIAG_CWS_MESSAGE_CLASS).and_then(Value::as_str),
            Some("response")
        );
        assert_eq!(
            response
                .get(DIAG_CWS_CONNECTION_INSTANCE_ID)
                .and_then(Value::as_str),
            Some("conn-1")
        );
        assert_eq!(
            response
                .get(DIAG_CWS_REQUEST_ORDER_ID)
                .and_then(Value::as_str),
            Some("2023555931497048623")
        );
    }

    #[test]
    fn invalid_jwt_response_detected_by_http_code_or_message() {
        let http_code = serde_json::json!({
            "httpCode": 401,
            "message": "Invalid JWT token!"
        });
        let message_only = serde_json::json!({
            "message": "Invalid JWT token!"
        });
        let ok = serde_json::json!({
            "httpCode": 200,
            "message": "ok"
        });

        assert!(is_invalid_jwt_response(&http_code));
        assert!(is_invalid_jwt_response(&message_only));
        assert!(!is_invalid_jwt_response(&ok));
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
        let health = Arc::new(RwLock::new(HealthState::default()));
        pending.insert(
            "guid-1".to_string(),
            PendingRequest {
                request_id: Some("req-1".to_string()),
                cws_guid: "guid-1".to_string(),
                opcode: "create:limit".to_string(),
                order_id: Some("123".to_string()),
                symbol: Some("USDRUBF".to_string()),
                send_seq: 1,
                send_ts_utc: 1774285000,
                send_at: Instant::now(),
                pending_count_before_send: 0,
                resp_tx: tx_one,
            },
        );
        pending.insert(
            "guid-2".to_string(),
            PendingRequest {
                request_id: Some("req-2".to_string()),
                cws_guid: "guid-2".to_string(),
                opcode: "delete:limit".to_string(),
                order_id: Some("456".to_string()),
                symbol: None,
                send_seq: 2,
                send_ts_utc: 1774285001,
                send_at: Instant::now(),
                pending_count_before_send: 1,
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
            &health,
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

    fn test_handle_with_health(
        health: Arc<RwLock<HealthState>>,
    ) -> (CwsHandle, mpsc::Receiver<SessionCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCommand>(8);
        (
            CwsHandle {
                cmd_tx,
                instrument_group: "TEST".to_string(),
                health,
                recycle_guard: Arc::new(Mutex::new(())),
            },
            cmd_rx,
        )
    }

    #[tokio::test]
    async fn fresh_control_path_does_not_trigger_recycle() {
        let health = Arc::new(RwLock::new(HealthState {
            control_path_stale_after_sec: 900,
            cws_connection_instance_id: Some("conn-fresh".to_string()),
            readiness: true,
            cws_authorized: true,
            cws_last_tx_at: Some(Instant::now()),
            ..HealthState::default()
        }));
        let (handle, mut rx) = test_handle_with_health(health.clone());

        let outcome = handle
            .ensure_fresh_control_path_for_live_limit_entry(
                Uuid::new_v4(),
                Duration::from_millis(150),
                false,
            )
            .await
            .expect("fresh path must pass");

        assert!(!outcome.recycled);
        assert!(rx.try_recv().is_err(), "no recycle command expected");
        let guard = health.read();
        assert_eq!(guard.control_path_stale_detected_total, 0);
        assert_eq!(guard.control_path_recycle_total, 0);
    }

    #[tokio::test]
    async fn stale_control_path_recycles_and_waits_for_new_connection() {
        let health = Arc::new(RwLock::new(HealthState {
            control_path_stale_after_sec: 900,
            cws_connection_instance_id: Some("conn-old".to_string()),
            readiness: true,
            cws_authorized: true,
            cws_last_tx_at: Some(Instant::now() - Duration::from_secs(901)),
            ..HealthState::default()
        }));
        let (handle, mut rx) = test_handle_with_health(health.clone());
        let health_for_task = health.clone();
        tokio::spawn(async move {
            if let Some(SessionCommand::Recycle(_)) = rx.recv().await {
                tokio::time::sleep(Duration::from_millis(40)).await;
                let mut guard = health_for_task.write();
                guard.readiness = true;
                guard.cws_authorized = true;
                guard.cws_connection_instance_id = Some("conn-new".to_string());
                guard.cws_last_tx_at = Some(Instant::now());
                guard.cws_last_tx_ts_utc = Some(Utc::now().timestamp());
            }
        });

        let outcome = handle
            .ensure_fresh_control_path_for_live_limit_entry(
                Uuid::new_v4(),
                Duration::from_millis(300),
                false,
            )
            .await
            .expect("recycle should succeed");

        assert!(outcome.recycled);
        assert_eq!(
            outcome.previous_connection_instance_id.as_deref(),
            Some("conn-old")
        );
        assert_eq!(
            outcome.current_connection_instance_id.as_deref(),
            Some("conn-new")
        );
        let guard = health.read();
        assert_eq!(guard.control_path_stale_detected_total, 1);
        assert_eq!(guard.control_path_recycle_total, 1);
        assert_eq!(guard.control_path_recycle_success_total, 1);
        assert_eq!(guard.control_path_recycle_failed_total, 0);
    }

    #[tokio::test]
    async fn stale_control_path_times_out_when_recycle_never_completes() {
        let health = Arc::new(RwLock::new(HealthState {
            control_path_stale_after_sec: 900,
            cws_connection_instance_id: Some("conn-old".to_string()),
            readiness: true,
            cws_authorized: true,
            cws_last_tx_at: Some(Instant::now() - Duration::from_secs(901)),
            ..HealthState::default()
        }));
        let (handle, _rx) = test_handle_with_health(health.clone());

        let error = handle
            .ensure_fresh_control_path_for_live_limit_entry(
                Uuid::new_v4(),
                Duration::from_millis(120),
                false,
            )
            .await
            .expect_err("recycle should time out");

        assert_eq!(error.code(), "control_path_recycle_failed");
        let guard = health.read();
        assert_eq!(guard.control_path_stale_detected_total, 1);
        assert_eq!(guard.control_path_recycle_total, 1);
        assert_eq!(guard.control_path_recycle_success_total, 0);
        assert_eq!(guard.control_path_recycle_failed_total, 1);
        assert_eq!(guard.control_path_stale_blocked_send_total, 1);
    }
}
