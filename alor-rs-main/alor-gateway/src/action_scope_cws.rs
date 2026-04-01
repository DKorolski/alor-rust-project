use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde_json::{Map, Value, json};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::TokenProvider;
use crate::health::{CwsFrameTraceEntry, HealthState};

const ACTION_SCOPE_FRAME_RING_LIMIT: usize = 100;
const CWS_TIME_IN_FORCE: &str = "OneDay";
const CWS_ALLOW_MARGIN: bool = true;
const DIAG_CWS_SEND_SEQ: &str = "_diag_cws_send_seq";
const DIAG_CWS_SEND_TS_UTC: &str = "_diag_cws_send_ts_utc";
const DIAG_CWS_RECV_SEQ: &str = "_diag_cws_recv_seq";
const DIAG_CWS_RECV_TS_UTC: &str = "_diag_cws_recv_ts_utc";
const DIAG_CWS_MESSAGE_CLASS: &str = "_diag_cws_message_class";
const DIAG_CWS_CONNECTION_INSTANCE_ID: &str = "_diag_cws_connection_instance_id";
const DIAG_CWS_CONNECT_SEQ: &str = "_diag_cws_connect_seq";
const DIAG_CWS_RECONNECT_SEQ: &str = "_diag_cws_reconnect_seq";
const DIAG_CWS_ORDER_ID: &str = "_diag_cws_order_id";
const DIAG_CWS_REQUEST_ORDER_ID: &str = "_diag_cws_request_order_id";

#[derive(Debug, Clone)]
pub struct ActionScopeCwsConfig {
    pub open_timeout: Duration,
    pub authorize_timeout: Duration,
    pub followup_window: Duration,
    pub max_session_lifetime: Duration,
    pub close_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ActionScopeCwsManager {
    cws_url: String,
    instrument_group: String,
    token_provider: TokenProvider,
    health: Arc<RwLock<HealthState>>,
    config: ActionScopeCwsConfig,
}

struct ActionScopeSession {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    conn_instance_id: String,
    send_seq: u64,
    recv_seq: u64,
    opened_at: Instant,
    primary_opcode: String,
    followup_deadline: Option<Instant>,
}

#[derive(Debug, Clone)]
struct SendTrace {
    send_seq: u64,
    send_ts_utc: i64,
}

impl ActionScopeCwsManager {
    pub fn new(
        cws_url: String,
        instrument_group: String,
        token_provider: TokenProvider,
        health: Arc<RwLock<HealthState>>,
        config: ActionScopeCwsConfig,
    ) -> Self {
        Self {
            cws_url,
            instrument_group,
            token_provider,
            health,
            config,
        }
    }

    pub async fn create_limit(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        price: f64,
        qty: f64,
        side: &str,
        comment: Option<&str>,
        request_id: &str,
    ) -> Result<Value> {
        let mut session = self.open_session("create:limit").await?;
        let result = session
            .send_create_limit(
                self, portfolio, exchange, symbol, price, qty, side, comment, request_id,
            )
            .await;
        self.close_session(&mut session).await;
        result
    }

    pub async fn cancel(
        &self,
        portfolio: &str,
        exchange: &str,
        order_id: i64,
        request_id: &str,
    ) -> Result<Value> {
        let mut session = self.open_session("delete:limit").await?;
        let result = session
            .send_delete_limit(self, portfolio, exchange, order_id, request_id, false)
            .await;
        self.close_session(&mut session).await;
        result
    }

    #[allow(dead_code)]
    pub async fn create_then_cancel(
        &self,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        price: f64,
        qty: f64,
        side: &str,
        comment: Option<&str>,
        request_id: &str,
    ) -> Result<(Value, Value)> {
        let mut session = self.open_session("create:limit").await?;
        let result = async {
            let create = session
                .send_create_limit(
                    self, portfolio, exchange, symbol, price, qty, side, comment, request_id,
                )
                .await?;
            let order_id = extract_order_id(&create)
                .ok_or_else(|| anyhow!("create:limit response missing order_id for follow-up"))?;
            let cancel = session
                .send_delete_limit(self, portfolio, exchange, order_id, request_id, true)
                .await?;
            Ok((create, cancel))
        }
        .await;
        self.close_session(&mut session).await;
        result
    }

    async fn open_session(&self, primary_opcode: &str) -> Result<ActionScopeSession> {
        let conn_instance_id = Uuid::new_v4().to_string();
        let open_ts_utc = Utc::now().timestamp();
        {
            let mut guard = self.health.write();
            guard.last_action_scope_mode = Some("action_scoped".to_string());
            guard.last_action_scope_open_ts_utc = Some(open_ts_utc);
            guard.last_action_scope_error = None;
            guard.last_action_scope_conn_instance_id = Some(conn_instance_id.clone());
            guard.last_action_scope_primary_opcode = Some(primary_opcode.to_string());
            guard.last_action_scope_followup_opcode = None;
            guard.action_scope_open_total = guard.action_scope_open_total.saturating_add(1);
        }
        info!(
            control_cws_mode = "action_scoped",
            action = "action_scope_session_open_start",
            conn_instance_id = %conn_instance_id,
            primary_opcode = primary_opcode,
            cws_url = %self.cws_url,
            "action scope session open start"
        );

        let ws = match timeout(self.config.open_timeout, connect_async(&self.cws_url)).await {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(error)) => {
                self.record_action_scope_open_error(&conn_instance_id, &error.to_string());
                return Err(error.into());
            }
            Err(_) => {
                self.record_action_scope_open_error(&conn_instance_id, "open timeout");
                bail!("action-scope cws open timeout");
            }
        };

        self.record_action_scope_open_success(&conn_instance_id);

        let mut session = ActionScopeSession {
            ws,
            conn_instance_id,
            send_seq: 0,
            recv_seq: 0,
            opened_at: Instant::now(),
            primary_opcode: primary_opcode.to_string(),
            followup_deadline: None,
        };

        if let Err(error) = session.authorize(self).await {
            let message = error.to_string();
            {
                let mut guard = self.health.write();
                guard.action_scope_authorize_failed_total =
                    guard.action_scope_authorize_failed_total.saturating_add(1);
                guard.last_action_scope_error = Some(message.clone());
            }
            warn!(
                control_cws_mode = "action_scoped",
                action = "action_scope_authorize_failed",
                conn_instance_id = %session.conn_instance_id,
                primary_opcode = %session.primary_opcode,
                error = %message,
                "action scope authorize failed"
            );
            let _ = timeout(self.config.close_timeout, session.ws.close(None)).await;
            return Err(error);
        }

        Ok(session)
    }

    async fn close_session(&self, session: &mut ActionScopeSession) {
        let close_ts_utc = Utc::now().timestamp();
        {
            let mut guard = self.health.write();
            guard.last_action_scope_close_ts_utc = Some(close_ts_utc);
            guard.action_scope_close_total = guard.action_scope_close_total.saturating_add(1);
        }
        info!(
            control_cws_mode = "action_scoped",
            action = "action_scope_close_start",
            conn_instance_id = %session.conn_instance_id,
            primary_opcode = %session.primary_opcode,
            "action scope session close start"
        );
        let result = timeout(self.config.close_timeout, session.ws.close(None)).await;
        match result {
            Ok(Ok(())) => {
                info!(
                    control_cws_mode = "action_scoped",
                    action = "action_scope_close_result",
                    conn_instance_id = %session.conn_instance_id,
                    primary_opcode = %session.primary_opcode,
                    outcome = "ok",
                    "action scope session close result"
                );
            }
            Ok(Err(error)) => {
                let mut guard = self.health.write();
                guard.action_scope_close_failed_total =
                    guard.action_scope_close_failed_total.saturating_add(1);
                guard.last_action_scope_error = Some(error.to_string());
                warn!(
                    control_cws_mode = "action_scoped",
                    action = "action_scope_close_result",
                    conn_instance_id = %session.conn_instance_id,
                    primary_opcode = %session.primary_opcode,
                    outcome = "error",
                    error = %error,
                    "action scope session close result"
                );
            }
            Err(_) => {
                let mut guard = self.health.write();
                guard.action_scope_close_failed_total =
                    guard.action_scope_close_failed_total.saturating_add(1);
                guard.last_action_scope_error = Some("close timeout".to_string());
                warn!(
                    control_cws_mode = "action_scoped",
                    action = "action_scope_close_result",
                    conn_instance_id = %session.conn_instance_id,
                    primary_opcode = %session.primary_opcode,
                    outcome = "timeout",
                    "action scope session close result"
                );
            }
        }
    }

    fn record_action_scope_open_error(&self, conn_instance_id: &str, error: &str) {
        let mut guard = self.health.write();
        guard.action_scope_open_failed_total =
            guard.action_scope_open_failed_total.saturating_add(1);
        guard.last_action_scope_error = Some(error.to_string());
        guard.last_action_scope_conn_instance_id = Some(conn_instance_id.to_string());
        warn!(
            control_cws_mode = "action_scoped",
            action = "action_scope_session_open_error",
            conn_instance_id = conn_instance_id,
            error = error,
            "action scope session open failed"
        );
    }

    fn record_action_scope_open_success(&self, conn_instance_id: &str) {
        info!(
            control_cws_mode = "action_scoped",
            action = "action_scope_session_open_success",
            conn_instance_id = conn_instance_id,
            "action scope session open success"
        );
    }
}

impl ActionScopeSession {
    async fn authorize(&mut self, manager: &ActionScopeCwsManager) -> Result<()> {
        let token_snapshot = manager
            .token_provider
            .access_token_with_context("action_scope_cws_authorize")
            .await?;
        let guid = Uuid::new_v4().to_string();
        let payload = json!({
            "opcode": "authorize",
            "guid": guid,
            "token": token_snapshot.token(),
        });
        self.send_json(manager, payload.clone(), "authorize")
            .await?;
        let response = self
            .read_until_guid(manager, &guid, manager.config.authorize_timeout)
            .await?;
        let status_ok = response
            .get("status")
            .and_then(Value::as_i64)
            .or_else(|| response.get("httpCode").and_then(Value::as_i64))
            == Some(200);
        let cws_authorized = response
            .get("cws_authorized")
            .or_else(|| response.get("cwsAuthorized"))
            .and_then(Value::as_bool)
            .unwrap_or(status_ok);
        if !status_ok || !cws_authorized {
            bail!("action-scope authorize failed: {response}");
        }
        let authorize_ts_utc = Utc::now().timestamp();
        {
            let mut guard = manager.health.write();
            guard.last_action_scope_authorize_ts_utc = Some(authorize_ts_utc);
        }
        info!(
            control_cws_mode = "action_scoped",
            action = "action_scope_authorize_ok",
            conn_instance_id = %self.conn_instance_id,
            primary_opcode = %self.primary_opcode,
            access_token_fingerprint = token_snapshot.fingerprint(),
            access_token_source = token_snapshot.source().as_str(),
            token_refresh_count = token_snapshot.refresh_count(),
            "action scope authorize ok"
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_create_limit(
        &mut self,
        manager: &ActionScopeCwsManager,
        portfolio: &str,
        exchange: &str,
        symbol: &str,
        price: f64,
        qty: f64,
        side: &str,
        comment: Option<&str>,
        request_id: &str,
    ) -> Result<Value> {
        let payload = build_create_limit_payload(
            portfolio,
            exchange,
            symbol,
            &manager.instrument_group,
            price,
            qty.round() as i64,
            side,
            comment,
            Some(request_id),
        );
        self.send_and_await_response(manager, payload, "create:limit", None, false)
            .await
    }

    async fn send_delete_limit(
        &mut self,
        manager: &ActionScopeCwsManager,
        portfolio: &str,
        exchange: &str,
        order_id: i64,
        request_id: &str,
        is_followup: bool,
    ) -> Result<Value> {
        if is_followup {
            let now = Instant::now();
            match self.followup_deadline {
                Some(deadline) if now <= deadline => {
                    let mut guard = manager.health.write();
                    guard.action_scope_followup_total =
                        guard.action_scope_followup_total.saturating_add(1);
                    guard.last_action_scope_followup_opcode = Some("delete:limit".to_string());
                }
                _ => {
                    let mut guard = manager.health.write();
                    guard.action_scope_followup_failed_total =
                        guard.action_scope_followup_failed_total.saturating_add(1);
                    guard.last_action_scope_error = Some("follow-up window expired".to_string());
                    bail!("action-scope follow-up window expired before delete:limit");
                }
            }
        }

        let payload = build_delete_limit_payload(portfolio, exchange, order_id, Some(request_id));
        self.send_and_await_response(
            manager,
            payload,
            "delete:limit",
            Some(order_id.to_string()),
            is_followup,
        )
        .await
    }

    async fn send_and_await_response(
        &mut self,
        manager: &ActionScopeCwsManager,
        payload: Value,
        opcode: &str,
        request_order_id: Option<String>,
        is_followup: bool,
    ) -> Result<Value> {
        self.ensure_lifetime(manager)?;
        let guid = payload
            .get("guid")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| anyhow!("action-scope payload missing guid"))?;
        let trace = match self.send_json(manager, payload.clone(), opcode).await {
            Ok(trace) => trace,
            Err(error) => {
                update_action_scope_outcome_counters(&manager.health, opcode, false, is_followup);
                let mut guard = manager.health.write();
                guard.last_action_scope_error = Some(error.to_string());
                return Err(error);
            }
        };
        let response = self
            .read_until_guid(manager, &guid, manager.config.max_session_lifetime)
            .await;
        match response {
            Ok(mut response) => {
                attach_diag_fields(
                    &mut response,
                    &self.conn_instance_id,
                    trace.send_seq,
                    trace.send_ts_utc,
                    self.recv_seq,
                    self.primary_opcode.as_str(),
                    opcode,
                    request_order_id.as_deref(),
                );
                let http_code = response.get("httpCode").and_then(Value::as_i64);
                let success = http_code == Some(200);
                {
                    let mut guard = manager.health.write();
                    if success {
                        guard.cws_last_control_success_ts_utc = Some(Utc::now().timestamp());
                    } else {
                        guard.cws_last_control_failure_ts_utc = Some(Utc::now().timestamp());
                    }
                }
                update_action_scope_outcome_counters(&manager.health, opcode, success, is_followup);
                if !is_followup {
                    self.followup_deadline = Some(Instant::now() + manager.config.followup_window);
                }
                info!(
                    control_cws_mode = "action_scoped",
                    action = "action_scope_send_result",
                    conn_instance_id = %self.conn_instance_id,
                    primary_opcode = %self.primary_opcode,
                    opcode = opcode,
                    is_followup,
                    http_code = ?http_code,
                    order_id = ?extract_order_id(&response),
                    "action scope send result"
                );
                Ok(response)
            }
            Err(error) => {
                update_action_scope_outcome_counters(&manager.health, opcode, false, is_followup);
                let mut guard = manager.health.write();
                guard.last_action_scope_error = Some(error.to_string());
                warn!(
                    control_cws_mode = "action_scoped",
                    action = "action_scope_send_result",
                    conn_instance_id = %self.conn_instance_id,
                    primary_opcode = %self.primary_opcode,
                    opcode = opcode,
                    is_followup,
                    error = %error,
                    "action scope send failed"
                );
                Err(error)
            }
        }
    }

    async fn send_json(
        &mut self,
        manager: &ActionScopeCwsManager,
        payload: Value,
        opcode: &str,
    ) -> Result<SendTrace> {
        self.send_seq = self.send_seq.saturating_add(1);
        let send_ts_utc = Utc::now().timestamp();
        {
            let mut guard = manager.health.write();
            guard.last_action_scope_send_ts_utc = Some(send_ts_utc);
            guard.action_scope_send_total = guard.action_scope_send_total.saturating_add(1);
            record_frame(
                &mut guard.cws_recent_outbound_frames,
                CwsFrameTraceEntry {
                    ts_utc: send_ts_utc,
                    seq: self.send_seq,
                    direction: "outbound".to_string(),
                    conn_instance_id: Some(self.conn_instance_id.clone()),
                    opcode: Some(opcode.to_string()),
                    guid: payload
                        .get("guid")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    request_guid: payload
                        .get("requestGuid")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    correlation_guid: payload
                        .get("guid")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    order_id: payload.get("orderId").and_then(value_to_string),
                    symbol: payload
                        .get("instrument")
                        .and_then(Value::as_object)
                        .and_then(|instrument| instrument.get("symbol"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    message_class: "action_scope_send".to_string(),
                },
            );
        }
        info!(
            control_cws_mode = "action_scoped",
            action = "action_scope_send_start",
            conn_instance_id = %self.conn_instance_id,
            primary_opcode = %self.primary_opcode,
            opcode = opcode,
            send_seq = self.send_seq,
            guid = payload.get("guid").and_then(|value| value.as_str()),
            "action scope send start"
        );
        self.ws
            .send(Message::Text(payload.to_string().into()))
            .await
            .with_context(|| format!("action-scope failed to send opcode={opcode}"))?;
        Ok(SendTrace {
            send_seq: self.send_seq,
            send_ts_utc,
        })
    }

    async fn read_until_guid(
        &mut self,
        manager: &ActionScopeCwsManager,
        guid: &str,
        timeout_duration: Duration,
    ) -> Result<Value> {
        let deadline = Instant::now() + timeout_duration;
        loop {
            self.ensure_lifetime(manager)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timeout waiting for guid={guid}");
            }
            let message = timeout(remaining, self.ws.next())
                .await
                .context("action-scope cws read timeout while waiting for guid")?
                .context("action-scope cws stream closed while waiting for guid")??;
            let Some(value) = self.handle_inbound_message(manager, message).await? else {
                continue;
            };
            let response_guid = value
                .get("requestGuid")
                .or_else(|| value.get("guid"))
                .or_else(|| value.get("correlationGuid"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if response_guid == guid {
                return Ok(value);
            }
        }
    }

    async fn handle_inbound_message(
        &mut self,
        manager: &ActionScopeCwsManager,
        message: Message,
    ) -> Result<Option<Value>> {
        self.recv_seq = self.recv_seq.saturating_add(1);
        let recv_ts_utc = Utc::now().timestamp();
        match message {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(&text).with_context(|| {
                    format!("failed to parse action-scope inbound text frame: {text}")
                })?;
                let (opcode, guid, request_guid, order_id, symbol, message_class) =
                    describe_inbound(&value);
                let mut guard = manager.health.write();
                record_frame(
                    &mut guard.cws_recent_inbound_frames,
                    CwsFrameTraceEntry {
                        ts_utc: recv_ts_utc,
                        seq: self.recv_seq,
                        direction: "inbound".to_string(),
                        conn_instance_id: Some(self.conn_instance_id.clone()),
                        opcode,
                        guid,
                        request_guid,
                        correlation_guid: None,
                        order_id,
                        symbol,
                        message_class,
                    },
                );
                Ok(Some(value))
            }
            Message::Ping(payload) => {
                self.ws.send(Message::Pong(payload)).await?;
                Ok(None)
            }
            Message::Pong(_) => Ok(None),
            Message::Binary(_) => Ok(None),
            Message::Close(frame) => {
                bail!(
                    "action-scope cws closed while waiting for response: code={:?} reason={:?}",
                    frame.as_ref().map(|value| value.code),
                    frame.as_ref().map(|value| value.reason.to_string())
                );
            }
            Message::Frame(_) => Ok(None),
        }
    }

    fn ensure_lifetime(&self, manager: &ActionScopeCwsManager) -> Result<()> {
        if self.opened_at.elapsed() > manager.config.max_session_lifetime {
            bail!("action-scope max session lifetime exceeded");
        }
        Ok(())
    }
}

fn build_create_limit_payload(
    portfolio: &str,
    exchange: &str,
    symbol: &str,
    instrument_group: &str,
    price: f64,
    qty: i64,
    side: &str,
    comment: Option<&str>,
    request_id: Option<&str>,
) -> Value {
    let guid = Uuid::new_v4().to_string();
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
        json!({
            "symbol": symbol,
            "exchange": exchange,
            "instrumentGroup": instrument_group,
        }),
    );
    payload.insert(
        "user".to_string(),
        json!({
            "portfolio": portfolio,
        }),
    );
    payload.insert(
        "timeInForce".to_string(),
        Value::String(CWS_TIME_IN_FORCE.to_string()),
    );
    payload.insert("allowMargin".to_string(), Value::from(CWS_ALLOW_MARGIN));
    payload.insert("checkDuplicates".to_string(), Value::from(true));
    if let Some(comment) = comment {
        payload.insert("comment".to_string(), Value::String(comment.to_string()));
    }
    if let Some(request_id) = request_id {
        payload.insert(
            "requestId".to_string(),
            Value::String(request_id.to_string()),
        );
    }
    Value::Object(payload)
}

fn build_delete_limit_payload(
    portfolio: &str,
    exchange: &str,
    order_id: i64,
    request_id: Option<&str>,
) -> Value {
    let guid = Uuid::new_v4().to_string();
    let mut payload = Map::new();
    payload.insert(
        "opcode".to_string(),
        Value::String("delete:limit".to_string()),
    );
    payload.insert("guid".to_string(), Value::String(guid));
    payload.insert("orderId".to_string(), Value::from(order_id));
    payload.insert("exchange".to_string(), Value::String(exchange.to_string()));
    payload.insert(
        "user".to_string(),
        json!({
            "portfolio": portfolio,
        }),
    );
    payload.insert("checkDuplicates".to_string(), Value::from(true));
    if let Some(request_id) = request_id {
        payload.insert(
            "requestId".to_string(),
            Value::String(request_id.to_string()),
        );
    }
    Value::Object(payload)
}

fn extract_order_id(value: &Value) -> Option<i64> {
    value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(Value::as_i64)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(Value::as_i64)
            })
        })
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
    None
}

fn describe_inbound(
    value: &Value,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
) {
    let opcode = value
        .get("opcode")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let guid = value
        .get("guid")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let request_guid = value
        .get("requestGuid")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let order_id = value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(value_to_string)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(value_to_string)
            })
        });
    let symbol = value
        .get("symbol")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("symbol")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
        });
    let message_class = if value.get("httpCode").is_some()
        || value.get("requestGuid").is_some()
        || value.get("status").is_some()
    {
        "response".to_string()
    } else {
        "event".to_string()
    };
    (opcode, guid, request_guid, order_id, symbol, message_class)
}

fn attach_diag_fields(
    value: &mut Value,
    conn_instance_id: &str,
    send_seq: u64,
    send_ts_utc: i64,
    recv_seq: u64,
    primary_opcode: &str,
    opcode: &str,
    request_order_id: Option<&str>,
) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.insert(DIAG_CWS_SEND_SEQ.to_string(), Value::from(send_seq));
    obj.insert(DIAG_CWS_SEND_TS_UTC.to_string(), Value::from(send_ts_utc));
    obj.insert(DIAG_CWS_RECV_SEQ.to_string(), Value::from(recv_seq));
    obj.insert(
        DIAG_CWS_RECV_TS_UTC.to_string(),
        Value::from(Utc::now().timestamp()),
    );
    obj.insert(
        DIAG_CWS_MESSAGE_CLASS.to_string(),
        Value::String("response".to_string()),
    );
    obj.insert(
        DIAG_CWS_CONNECTION_INSTANCE_ID.to_string(),
        Value::String(conn_instance_id.to_string()),
    );
    obj.insert(DIAG_CWS_CONNECT_SEQ.to_string(), Value::from(0));
    obj.insert(DIAG_CWS_RECONNECT_SEQ.to_string(), Value::from(0));
    if let Some(order_id) = request_order_id {
        obj.insert(
            DIAG_CWS_REQUEST_ORDER_ID.to_string(),
            Value::String(order_id.to_string()),
        );
    }
    if let Some(order_id) = obj
        .get("orderNumber")
        .or_else(|| obj.get("orderId"))
        .and_then(value_to_string)
    {
        obj.insert(DIAG_CWS_ORDER_ID.to_string(), Value::String(order_id));
    }
    obj.insert(
        "actionScopePrimaryOpcode".to_string(),
        Value::String(primary_opcode.to_string()),
    );
    obj.insert(
        "actionScopeOpcode".to_string(),
        Value::String(opcode.to_string()),
    );
}

fn update_action_scope_outcome_counters(
    health: &Arc<RwLock<HealthState>>,
    opcode: &str,
    success: bool,
    is_followup: bool,
) {
    let mut guard = health.write();
    if !success {
        guard.action_scope_send_failed_total =
            guard.action_scope_send_failed_total.saturating_add(1);
        if is_followup {
            guard.action_scope_followup_failed_total =
                guard.action_scope_followup_failed_total.saturating_add(1);
        }
    }
    match opcode {
        "create:limit" => {
            guard.cws_limit_send_total = guard.cws_limit_send_total.saturating_add(1);
            guard.cws_create_limit_send_total = guard.cws_create_limit_send_total.saturating_add(1);
            if success {
                guard.cws_create_limit_success_total =
                    guard.cws_create_limit_success_total.saturating_add(1);
            } else {
                guard.cws_limit_error_total = guard.cws_limit_error_total.saturating_add(1);
                guard.cws_create_limit_failure_total =
                    guard.cws_create_limit_failure_total.saturating_add(1);
            }
        }
        "delete:limit" => {
            guard.cws_delete_limit_send_total = guard.cws_delete_limit_send_total.saturating_add(1);
            if success {
                guard.cws_delete_limit_success_total =
                    guard.cws_delete_limit_success_total.saturating_add(1);
            } else {
                guard.cws_delete_limit_failure_total =
                    guard.cws_delete_limit_failure_total.saturating_add(1);
            }
        }
        "update:limit" => {
            guard.cws_replace_limit_send_total =
                guard.cws_replace_limit_send_total.saturating_add(1);
            if success {
                guard.cws_replace_limit_success_total =
                    guard.cws_replace_limit_success_total.saturating_add(1);
            } else {
                guard.cws_replace_limit_failure_total =
                    guard.cws_replace_limit_failure_total.saturating_add(1);
            }
        }
        _ => {}
    }
}

fn record_frame(frames: &mut Vec<CwsFrameTraceEntry>, entry: CwsFrameTraceEntry) {
    frames.push(entry);
    if frames.len() > ACTION_SCOPE_FRAME_RING_LIMIT {
        let overflow = frames.len() - ACTION_SCOPE_FRAME_RING_LIMIT;
        frames.drain(0..overflow);
    }
}
