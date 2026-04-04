use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{debug, info, warn};

use alor_protocol::{CommandAck, CommandAction, IntentClass, OrderCommand, Side};
use alor_types::MarketState;

use crate::action_scope_cws::ActionScopeCwsManager;
use crate::config::ControlCwsMode;
use crate::health::{GatewayPhase, HealthState};
use crate::transport::{CommandEnvelope, CommandSink, CommandSource};

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
pub struct CommandConsumerConfig {
    pub pause_when_degraded: bool,
    pub control_path_pre_entry_recycle_enabled: bool,
    pub control_path_pre_exit_recycle_enabled: bool,
    pub control_path_recycle_timeout: Duration,
    pub control_path_recycle_timeout_exit: Duration,
    pub control_path_post_recycle_exit_send_window: Duration,
    pub control_path_hardening_log_only: bool,
    pub control_cws_mode: ControlCwsMode,
    pub action_scope_enable_create_limit: bool,
    pub action_scope_enable_delete_limit: bool,
    pub action_scope_enable_replace_limit: bool,
    pub action_scope_enable_exit: bool,
    pub idempotency_ttl: Duration,
    pub idempotency_max: usize,
    pub error_backoff_base: Duration,
    pub error_backoff_max: Duration,
    pub no_message_log_interval: Duration,
}

impl Default for CommandConsumerConfig {
    fn default() -> Self {
        Self {
            pause_when_degraded: true,
            control_path_pre_entry_recycle_enabled: true,
            control_path_pre_exit_recycle_enabled: true,
            control_path_recycle_timeout: Duration::from_secs(5),
            control_path_recycle_timeout_exit: Duration::from_secs(10),
            control_path_post_recycle_exit_send_window: Duration::from_secs(5),
            control_path_hardening_log_only: false,
            control_cws_mode: ControlCwsMode::LegacyLongLived,
            action_scope_enable_create_limit: false,
            action_scope_enable_delete_limit: false,
            action_scope_enable_replace_limit: false,
            action_scope_enable_exit: false,
            idempotency_ttl: Duration::from_secs(300),
            idempotency_max: 10_000,
            error_backoff_base: Duration::from_millis(50),
            error_backoff_max: Duration::from_secs(2),
            no_message_log_interval: Duration::from_secs(20),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlPathHardeningPolicy {
    Entry,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandExecutionPath {
    LegacyLongLived,
    ActionScoped,
}

#[derive(Debug, Clone)]
pub struct CommandConsumerInfo {
    pub consumer_group: String,
    pub consumer_name: String,
    pub stream: String,
    pub block_ms: usize,
}

#[async_trait::async_trait]
pub trait IdempotencyStore: Send + Sync {
    async fn check_and_set(&self, request_id: uuid::Uuid) -> anyhow::Result<bool>;
}

pub struct InMemoryIdempotency {
    entries: parking_lot::RwLock<HashMap<uuid::Uuid, Instant>>,
    order: parking_lot::RwLock<VecDeque<uuid::Uuid>>,
    ttl: Duration,
    max: usize,
}

impl InMemoryIdempotency {
    pub fn new(ttl: Duration, max: usize) -> Self {
        Self {
            entries: parking_lot::RwLock::new(HashMap::new()),
            order: parking_lot::RwLock::new(VecDeque::new()),
            ttl,
            max,
        }
    }

    fn evict_expired(&self) {
        let now = Instant::now();
        let mut order = self.order.write();
        let mut entries = self.entries.write();
        while let Some(front) = order.front().copied() {
            let Some(ts) = entries.get(&front) else {
                order.pop_front();
                continue;
            };
            if now.duration_since(*ts) > self.ttl {
                entries.remove(&front);
                order.pop_front();
            } else {
                break;
            }
        }
    }

    fn evict_overflow(&self) {
        let mut order = self.order.write();
        let mut entries = self.entries.write();
        while order.len() > self.max {
            if let Some(front) = order.pop_front() {
                entries.remove(&front);
            }
        }
    }
}

#[async_trait::async_trait]
impl IdempotencyStore for InMemoryIdempotency {
    async fn check_and_set(&self, request_id: uuid::Uuid) -> anyhow::Result<bool> {
        self.evict_expired();
        let mut entries = self.entries.write();
        if entries.contains_key(&request_id) {
            return Ok(false);
        }
        entries.insert(request_id, Instant::now());
        drop(entries);
        self.order.write().push_back(request_id);
        self.evict_overflow();
        Ok(true)
    }
}

pub struct RedisIdempotencyStore {
    client: redis::Client,
    key_prefix: String,
    ttl_secs: usize,
}

impl RedisIdempotencyStore {
    pub fn new(redis_url: String, key_prefix: String, ttl: Duration) -> anyhow::Result<Self> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self {
            client,
            key_prefix,
            ttl_secs: ttl.as_secs() as usize,
        })
    }

    fn key(&self, request_id: uuid::Uuid) -> String {
        format!("{}:{}", self.key_prefix, request_id)
    }
}

#[async_trait::async_trait]
impl IdempotencyStore for RedisIdempotencyStore {
    async fn check_and_set(&self, request_id: uuid::Uuid) -> anyhow::Result<bool> {
        let key = self.key(request_id);
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let result: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg("1")
            .arg("EX")
            .arg(self.ttl_secs)
            .arg("NX")
            .query_async(&mut conn)
            .await?;
        Ok(result.is_some())
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_command_consumer(
    mut source: Box<dyn CommandSource>,
    sink: Arc<dyn CommandSink>,
    idempotency: Arc<dyn IdempotencyStore>,
    request_map: Arc<parking_lot::RwLock<HashMap<i64, uuid::Uuid>>>,
    cws: crate::cws_client::CwsHandle,
    action_scope_cws: Arc<ActionScopeCwsManager>,
    price_step: f64,
    volume_step: f64,
    health: Arc<parking_lot::RwLock<HealthState>>,
    shutdown_rx: &mut watch::Receiver<bool>,
    info: CommandConsumerInfo,
    config: CommandConsumerConfig,
) -> anyhow::Result<()> {
    let mut error_backoff = config.error_backoff_base;
    let mut last_no_message_log = Instant::now().checked_sub(config.no_message_log_interval);
    let mut last_trading_window_warn = Instant::now().checked_sub(Duration::from_secs(30));
    {
        let mut guard = health.write();
        guard.command_consumer_alive = true;
        guard.command_consumer_last_poll_ts_utc = chrono::Utc::now().timestamp();
    }
    info!(
        consumer_group = %info.consumer_group,
        consumer_name = %info.consumer_name,
        stream = %info.stream,
        block_ms = info.block_ms,
        "command consumer started"
    );
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            result = source.next_command() => {
                let now_ts = chrono::Utc::now().timestamp();
                {
                    let mut guard = health.write();
                    guard.command_consumer_alive = true;
                    guard.command_consumer_last_poll_ts_utc = now_ts;
                }
                let envelope = match result {
                    Ok(Some(envelope)) => {
                        error_backoff = config.error_backoff_base;
                        envelope
                    }
                    Ok(None) => {
                        increment_counter(&health, |h| h.command_consumer_redis_timeouts_total = h.command_consumer_redis_timeouts_total.saturating_add(1));
                        if last_no_message_log
                            .map(|last| last.elapsed() >= config.no_message_log_interval)
                            .unwrap_or(true)
                        {
                            debug!(
                                consumer_group = %info.consumer_group,
                                consumer_name = %info.consumer_name,
                                stream = %info.stream,
                                "command consumer poll timeout (no messages)"
                            );
                            last_no_message_log = Some(Instant::now());
                        }
                        continue;
                    }
                    Err(error) => {
                        increment_counter(&health, |h| h.command_consumer_errors_total = h.command_consumer_errors_total.saturating_add(1));
                        warn!(?error, "command consumer poll failed; backing off");
                        tokio::time::sleep(error_backoff).await;
                        error_backoff = (error_backoff * 2).min(config.error_backoff_max);
                        continue;
                    }
                };
                let CommandEnvelope { command, message_id } = envelope;
                let request_id = command.request_id;
                increment_counter(&health, |h| {
                    h.commands_received_total = h.commands_received_total.saturating_add(1)
                });
                if let Some(message_id) = message_id.as_deref() {
                    let mut guard = health.write();
                    guard.command_consumer_last_message_id = Some(message_id.to_string());
                }
                info!(
                    handler = "command_consumer",
                    state_before = "command_received",
                    state_after = "command_validated",
                    request_id = %request_id,
                    strategy_id = %command.strategy_id,
                    symbol = %command.symbol,
                    action = %command_action_label(&command.action),
                    target_order_id = ?command_target_order_id(&command.action),
                    ttl_ms = ?command.ttl_ms,
                    stream_id = ?message_id,
                    "command received"
                );
                if !idempotency.check_and_set(request_id).await? {
                    increment_counter(&health, |h| h.command_duplicate_total = h.command_duplicate_total.saturating_add(1));
                    increment_counter(&health, |h| {
                        h.commands_duplicate_total = h.commands_duplicate_total.saturating_add(1)
                    });
                    let ack = CommandAck::duplicate(request_id);
                    sink.publish_ack(ack.clone()).await?;
                    info!(
                        request_id = %ack.request_id,
                        status = ?ack.status,
                        processed_ts_utc = ack.processed_ts_utc,
                        "command ack published"
                    );
                    if let Some(message_id) = message_id.as_deref() {
                        source.ack(message_id).await?;
                    }
                    continue;
                }

                if let Some(error_code) = validate_command(&command, price_step, volume_step, &health, config.pause_when_degraded) {
                    increment_counter(&health, |h| h.command_validation_failed_total = h.command_validation_failed_total.saturating_add(1));
                    increment_counter(&health, |h| {
                        h.commands_rejected_total = h.commands_rejected_total.saturating_add(1)
                    });
                    if error_code == "trading_window_closed"
                        && last_trading_window_warn
                            .map(|last| last.elapsed() >= Duration::from_secs(30))
                            .unwrap_or(true)
                    {
                        warn!(
                            request_id = %request_id,
                            strategy_id = %command.strategy_id,
                            symbol = %command.symbol,
                            action = %command_action_label(&command.action),
                            scheduler_state = ?health.read().scheduler_state,
                            "command rejected: trading window closed"
                        );
                        last_trading_window_warn = Some(Instant::now());
                    }
                    let ack = CommandAck::rejected(request_id, error_code, "validation failed");
                    sink.publish_ack(ack.clone()).await?;
                    info!(
                        request_id = %ack.request_id,
                        status = ?ack.status,
                        processed_ts_utc = ack.processed_ts_utc,
                        "command ack published"
                    );
                    if let Some(message_id) = message_id.as_deref() {
                        source.ack(message_id).await?;
                    }
                    continue;
                }

                if is_command_expired(&command) {
                    increment_counter(&health, |h| h.command_expired_total = h.command_expired_total.saturating_add(1));
                    increment_counter(&health, |h| {
                        h.commands_rejected_total = h.commands_rejected_total.saturating_add(1)
                    });
                    let ack = CommandAck::expired(request_id, "command expired");
                    sink.publish_ack(ack.clone()).await?;
                    info!(
                        request_id = %ack.request_id,
                        status = ?ack.status,
                        processed_ts_utc = ack.processed_ts_utc,
                        "command ack published"
                    );
                    if let Some(message_id) = message_id.as_deref() {
                        source.ack(message_id).await?;
                    }
                    continue;
                }

                if let CommandAction::Cancel(payload) = &command.action {
                    let (previous_request_id, request_map_size) = {
                        let mut map = request_map.write();
                        let previous_request_id = map.insert(payload.order_id, request_id);
                        (previous_request_id, map.len() as u64)
                    };
                    increment_counter(&health, |h| h.request_map_size = request_map_size);
                    info!(
                        handler = "command_consumer",
                        state_before = "request_map_before_cancel_preinsert",
                        state_after = "request_map_after_cancel_preinsert",
                        request_id = %request_id,
                        broker_order_id = payload.order_id,
                        previous_request_id = ?previous_request_id,
                        current_request_id = %request_id,
                        "request_map updated from cancel intent"
                    );
                }

                let mut sent_after_recycle = false;
                let mut post_recycle_exit_send_active = false;
                let mut post_recycle_exit_send_conn_id = None;
                let execution_path = execution_path_for_command(&command, &config);
                if execution_path == CommandExecutionPath::LegacyLongLived
                    && let Some(policy) = control_path_hardening_policy(&command, &config)
                {
                    let recycle_timeout = match policy {
                        ControlPathHardeningPolicy::Entry => config.control_path_recycle_timeout,
                        ControlPathHardeningPolicy::Exit => {
                            config.control_path_recycle_timeout_exit
                        }
                    };
                    match cws
                        .ensure_fresh_control_path_for_live_limit_command(
                            request_id,
                            control_path_admission_policy(policy),
                            recycle_timeout,
                            config.control_path_post_recycle_exit_send_window,
                            config.control_path_hardening_log_only,
                        )
                        .await
                    {
                        Ok(outcome) => {
                            if outcome.recycled {
                                sent_after_recycle = true;
                                post_recycle_exit_send_active = outcome.post_recycle_exit_send_armed;
                                post_recycle_exit_send_conn_id =
                                    outcome.current_connection_instance_id.clone();
                                info!(
                                    handler = "command_consumer",
                                    state_before = "control_path_recycled",
                                    state_after = "ready_to_send_after_recycle",
                                    request_id = %request_id,
                                    strategy_id = %command.strategy_id,
                                    symbol = %command.symbol,
                                    previous_cws_connection_instance_id = ?outcome.previous_connection_instance_id,
                                    current_cws_connection_instance_id = ?outcome.current_connection_instance_id,
                                    control_path_stale_reason = ?outcome.stale_reason,
                                    control_path_stale_for_ms = ?outcome.stale_for_ms,
                                    post_recycle_exit_send_armed = outcome.post_recycle_exit_send_armed,
                                    control_path_policy = ?policy,
                                    "control_path_send_after_recycle"
                                );
                            }
                        }
                        Err(error) => {
                            increment_counter(&health, |h| {
                                h.cws_errors_total = h.cws_errors_total.saturating_add(1)
                            });
                            let ack = build_control_path_error_ack(
                                request_id,
                                error.code(),
                                error.message(),
                            );
                            warn!(
                                handler = "command_consumer",
                                state_before = "control_path_stale_detected",
                                state_after = "send_blocked_due_to_stale",
                                request_id = %request_id,
                                strategy_id = %command.strategy_id,
                                symbol = %command.symbol,
                                error_code = error.code(),
                                error_msg = error.message(),
                                control_path_policy = ?policy,
                                "control_path_send_blocked_due_to_stale"
                            );
                            sink.publish_ack(ack.clone()).await?;
                            info!(
                                handler = "command_consumer",
                                state_before = "control_path_block_ack_ready",
                                state_after = "ack_published",
                                request_id = %ack.request_id,
                                status = ?ack.status,
                                processed_ts_utc = ack.processed_ts_utc,
                                error_code = ?ack.error_code,
                                "command ack published"
                            );
                            if let Some(message_id) = message_id.as_deref() {
                                source.ack(message_id).await?;
                            }
                            continue;
                        }
                    }
                }
                if post_recycle_exit_send_active
                    && let Err(error) = cws.begin_post_recycle_exit_send(
                        request_id,
                        post_recycle_exit_send_conn_id.as_deref(),
                    )
                {
                        increment_counter(&health, |h| {
                            h.cws_errors_total = h.cws_errors_total.saturating_add(1)
                        });
                        let ack = build_control_path_error_ack(
                            request_id,
                            error.code(),
                            error.message(),
                        );
                        warn!(
                            handler = "command_consumer",
                            state_before = "post_recycle_exit_send_armed",
                            state_after = "post_recycle_exit_send_blocked",
                            request_id = %request_id,
                            strategy_id = %command.strategy_id,
                            symbol = %command.symbol,
                            error_code = error.code(),
                            error_msg = error.message(),
                            "post_recycle_exit_send_blocked"
                        );
                        sink.publish_ack(ack.clone()).await?;
                        info!(
                            handler = "command_consumer",
                            state_before = "post_recycle_exit_send_block_ack_ready",
                            state_after = "ack_published",
                            request_id = %ack.request_id,
                            status = ?ack.status,
                            processed_ts_utc = ack.processed_ts_utc,
                            error_code = ?ack.error_code,
                            "command ack published"
                        );
                        if let Some(message_id) = message_id.as_deref() {
                            source.ack(message_id).await?;
                        }
                        continue;
                }

                match execute_command(
                    &cws,
                    &action_scope_cws,
                    execution_path,
                    &command,
                    request_id,
                    price_step,
                    volume_step,
                )
                .await
                {
                    Ok(response) => {
                        let info = parse_cws_response(&response);
                        let http_code = info.http_code.unwrap_or(0);
                        let cws_request_guid = info.request_guid.clone();
                        info!(
                            handler = "command_consumer",
                            state_before = "response_received",
                            state_after = "response_classified",
                            request_id = %request_id,
                            action = %command_action_label(&command.action),
                            target_order_id = ?command_target_order_id(&command.action),
                            cws_http_code = http_code,
                            cws_message = ?info.message,
                            cws_request_guid = ?info.request_guid,
                            broker_order_id = info.order_id,
                            cws_order_id = ?info.trace.order_id,
                            cws_request_order_id = ?info.trace.request_order_id,
                            cws_send_seq = info.trace.send_seq,
                            cws_send_ts_utc = info.trace.send_ts_utc,
                            cws_recv_seq = info.trace.recv_seq,
                            cws_recv_ts_utc = info.trace.recv_ts_utc,
                            cws_message_class = ?info.trace.message_class,
                            cws_connection_instance_id = ?info.trace.connection_instance_id,
                            cws_connect_seq = info.trace.connect_seq,
                            cws_reconnect_seq = info.trace.reconnect_seq,
                            sent_after_recycle,
                            "cws response"
                        );
                        if post_recycle_exit_send_active {
                            let detail = if http_code == 200 {
                                "accepted".to_string()
                            } else if http_code > 0 {
                                format!("http_code={http_code}")
                            } else {
                                "missing_http_code".to_string()
                            };
                            cws.finish_post_recycle_exit_send(request_id, http_code == 200, detail);
                        }
                        if matches!(command.action, CommandAction::Place(_)) {
                            let (status, error_code, error_msg): (&str, Option<String>, Option<String>) =
                                if http_code == 200 {
                                    ("accepted", None, None)
                                } else if http_code > 0 {
                                    (
                                        "rejected",
                                        Some(format!("cws_http_{http_code}")),
                                        info.message.clone(),
                                    )
                                } else {
                                    (
                                        "error",
                                        Some("cws_error".to_string()),
                                        Some("missing httpCode".to_string()),
                                    )
                                };
                            info!(
                                action = "cws_limit_ack",
                                opcode = "create:limit",
                                request_id = %request_id,
                                cws_guid = ?cws_request_guid,
                                status,
                                broker_order_id = info.order_id,
                                error_code = ?error_code,
                                error_msg = ?error_msg,
                                cws_send_seq = info.trace.send_seq,
                                cws_recv_seq = info.trace.recv_seq,
                                cws_message_class = ?info.trace.message_class,
                                "cws limit ack received"
                            );
                        }
                        let (status, error_code, error_msg) = if http_code == 200 {
                            (alor_protocol::AckStatus::Accepted, None, None)
                        } else if http_code > 0 {
                            let error_code = format!("cws_http_{http_code}");
                            (alor_protocol::AckStatus::Rejected, Some(error_code), info.message.clone())
                        } else {
                            (
                                alor_protocol::AckStatus::Error,
                                Some("cws_error".to_string()),
                                Some("missing httpCode".to_string()),
                            )
                        };
                        if status == alor_protocol::AckStatus::Accepted {
                            increment_counter(&health, |h| h.command_processed_total = h.command_processed_total.saturating_add(1));
                            increment_counter(&health, |h| h.commands_accepted_total = h.commands_accepted_total.saturating_add(1));
                            if let Some(order_id) = info.order_id {
                                let (previous_request_id, request_map_size) = {
                                    let mut map = request_map.write();
                                    let previous_request_id = map.insert(order_id, request_id);
                                    (previous_request_id, map.len() as u64)
                                };
                                increment_counter(&health, |h| h.request_map_size = request_map_size);
                                info!(
                                    handler = "command_consumer",
                                    state_before = "request_map_before_ack_insert",
                                    state_after = "request_map_after_ack_insert",
                                    request_id = %request_id,
                                    broker_order_id = order_id,
                                    previous_request_id = ?previous_request_id,
                                    current_request_id = %request_id,
                                    cws_request_guid = ?cws_request_guid,
                                    cws_send_seq = info.trace.send_seq,
                                    cws_recv_seq = info.trace.recv_seq,
                                    "request_map updated from cws ack"
                                );
                            }
                        } else if status == alor_protocol::AckStatus::Rejected {
                            increment_counter(&health, |h| h.commands_rejected_total = h.commands_rejected_total.saturating_add(1));
                            increment_http_code(&health, http_code);
                            if matches!(command.action, CommandAction::Place(_)) {
                                increment_counter(&health, |h| {
                                    h.cws_limit_error_total =
                                        h.cws_limit_error_total.saturating_add(1);
                                    h.cws_last_limit_error_ts_utc =
                                        Some(chrono::Utc::now().timestamp());
                                });
                            }
                        } else if status == alor_protocol::AckStatus::Error {
                            increment_counter(&health, |h| h.cws_errors_total = h.cws_errors_total.saturating_add(1));
                            if matches!(command.action, CommandAction::Place(_)) {
                                increment_counter(&health, |h| {
                                    h.cws_limit_error_total =
                                        h.cws_limit_error_total.saturating_add(1);
                                    h.cws_last_limit_error_ts_utc =
                                        Some(chrono::Utc::now().timestamp());
                                });
                            }
                        }
                        let ack_trace = info.trace.clone();
                        let ack = build_cws_ack(
                            request_id,
                            status,
                            info,
                            error_code,
                            error_msg,
                        );
                        sink.publish_ack(ack.clone()).await?;
                        info!(
                            handler = "command_consumer",
                            state_before = "ack_ready_to_publish",
                            state_after = "ack_published",
                            request_id = %ack.request_id,
                            status = ?ack.status,
                            processed_ts_utc = ack.processed_ts_utc,
                            broker_order_id = ack.broker_order_id,
                            target_order_id = ?command_target_order_id(&command.action),
                            error_code = ?ack.error_code,
                            cws_http_code = ?ack.cws_http_code,
                            cws_request_guid = ?ack.cws_request_guid,
                            cws_order_id = ?ack_trace.order_id,
                            cws_request_order_id = ?ack_trace.request_order_id,
                            cws_send_seq = ack_trace.send_seq,
                            cws_send_ts_utc = ack_trace.send_ts_utc,
                            cws_recv_seq = ack_trace.recv_seq,
                            cws_recv_ts_utc = ack_trace.recv_ts_utc,
                            cws_message_class = ?ack_trace.message_class,
                            cws_connection_instance_id = ?ack_trace.connection_instance_id,
                            cws_connect_seq = ack_trace.connect_seq,
                            cws_reconnect_seq = ack_trace.reconnect_seq,
                            sent_after_recycle,
                            "command ack published"
                        );
                        if let Some(message_id) = message_id.as_deref() {
                            source.ack(message_id).await?;
                        }
                    }
                    Err(error) => {
                        if post_recycle_exit_send_active {
                            cws.finish_post_recycle_exit_send(
                                request_id,
                                false,
                                format_transport_error_message(&error),
                            );
                        }
                        increment_counter(&health, |h| h.cws_errors_total = h.cws_errors_total.saturating_add(1));
                        if matches!(command.action, CommandAction::Place(_)) {
                            increment_counter(&health, |h| {
                                h.cws_limit_error_total =
                                    h.cws_limit_error_total.saturating_add(1);
                                h.cws_last_limit_error_ts_utc =
                                    Some(chrono::Utc::now().timestamp());
                            });
                        }
                        let transport_failure = error.downcast_ref::<crate::cws_client::CwsRequestFailure>();
                        let cws_guid = transport_failure.map(|failure| failure.cws_guid().to_string());
                        let disconnect_kind = transport_failure
                            .map(|failure| failure.transport().disconnect_kind_str().to_string());
                        let failure_order_id = transport_failure
                            .and_then(|failure| failure.order_id().map(ToString::to_string));
                        if matches!(command.action, CommandAction::Place(_)) {
                            info!(
                                action = "cws_limit_ack",
                                opcode = "create:limit",
                                request_id = %request_id,
                                cws_guid = ?cws_guid,
                                status = "error",
                                broker_order_id = Option::<i64>::None,
                                error_code = "cws_error",
                                error_msg = %format_transport_error_message(&error),
                                disconnect_kind = ?disconnect_kind,
                                "cws limit ack received"
                            );
                        }
                        let ack = build_transport_error_ack(request_id, &error);
                        sink.publish_ack(ack.clone()).await?;
                        info!(
                            handler = "command_consumer",
                            state_before = "transport_error_ready_to_publish",
                            state_after = "ack_published",
                            request_id = %ack.request_id,
                            status = ?ack.status,
                            processed_ts_utc = ack.processed_ts_utc,
                            broker_order_id = ack.broker_order_id,
                            target_order_id = ?command_target_order_id(&command.action),
                            failure_order_id = ?failure_order_id,
                            error_code = ?ack.error_code,
                            cws_http_code = ?ack.cws_http_code,
                            cws_request_guid = ?ack.cws_request_guid,
                            sent_after_recycle,
                            "command ack published"
                        );
                        if let Some(message_id) = message_id.as_deref() {
                            source.ack(message_id).await?;
                        }
                    }
                }
            }
        }
    }
    {
        let mut guard = health.write();
        guard.command_consumer_alive = false;
    }
    info!(
        consumer_group = %info.consumer_group,
        consumer_name = %info.consumer_name,
        stream = %info.stream,
        "command consumer stopped: shutdown"
    );
    Ok(())
}

fn command_action_label(action: &CommandAction) -> &'static str {
    match action {
        CommandAction::Place(_) => "place",
        CommandAction::Market(_) => "market",
        CommandAction::Cancel(_) => "cancel",
        CommandAction::Replace(_) => "replace",
        CommandAction::CreateStopLimit(_) => "create_stop_limit",
        CommandAction::DeleteStopLimit(_) => "delete_stop_limit",
    }
}

fn increment_counter(
    health: &Arc<parking_lot::RwLock<HealthState>>,
    update: impl FnOnce(&mut HealthState),
) {
    let mut guard = health.write();
    update(&mut guard);
}

fn increment_http_code(health: &Arc<parking_lot::RwLock<HealthState>>, http_code: i64) {
    if http_code <= 0 {
        return;
    }
    let mut guard = health.write();
    let entry = guard
        .commands_rejected_http_code_total
        .entry(http_code)
        .or_insert(0);
    *entry = entry.saturating_add(1);
}

fn control_path_hardening_policy(
    command: &OrderCommand,
    config: &CommandConsumerConfig,
) -> Option<ControlPathHardeningPolicy> {
    if !matches!(command.action, CommandAction::Place(_)) {
        return None;
    }
    match command.effective_intent_class() {
        IntentClass::Entry if config.control_path_pre_entry_recycle_enabled => {
            Some(ControlPathHardeningPolicy::Entry)
        }
        IntentClass::Exit if config.control_path_pre_exit_recycle_enabled => {
            Some(ControlPathHardeningPolicy::Exit)
        }
        _ => None,
    }
}

fn execution_path_for_command(
    command: &OrderCommand,
    config: &CommandConsumerConfig,
) -> CommandExecutionPath {
    if config.control_cws_mode != ControlCwsMode::ActionScoped {
        return CommandExecutionPath::LegacyLongLived;
    }

    match (&command.action, command.effective_intent_class()) {
        (&CommandAction::Place(_), IntentClass::Entry)
            if config.action_scope_enable_create_limit =>
        {
            CommandExecutionPath::ActionScoped
        }
        (&CommandAction::Place(_), IntentClass::Exit) if config.action_scope_enable_exit => {
            CommandExecutionPath::ActionScoped
        }
        (&CommandAction::Cancel(_), _) if config.action_scope_enable_delete_limit => {
            CommandExecutionPath::ActionScoped
        }
        _ => CommandExecutionPath::LegacyLongLived,
    }
}

fn control_path_admission_policy(
    policy: ControlPathHardeningPolicy,
) -> crate::cws_client::ControlPathAdmissionPolicy {
    match policy {
        ControlPathHardeningPolicy::Entry => crate::cws_client::ControlPathAdmissionPolicy::Entry,
        ControlPathHardeningPolicy::Exit => {
            crate::cws_client::ControlPathAdmissionPolicy::ExitRiskReducing
        }
    }
}

fn validate_command(
    command: &OrderCommand,
    price_step: f64,
    volume_step: f64,
    health: &Arc<parking_lot::RwLock<HealthState>>,
    pause_when_degraded: bool,
) -> Option<&'static str> {
    let guard = health.read();
    if guard.gateway_phase != GatewayPhase::LiveReady {
        return Some("gateway_not_ready");
    }
    if pause_when_degraded && guard.event_sink_degraded {
        return Some("gateway_degraded");
    }
    let scheduler_state = guard.scheduler_state;
    drop(guard);
    if !matches!(
        &command.action,
        CommandAction::Cancel(_) | CommandAction::DeleteStopLimit(_)
    ) && !matches!(scheduler_state, Some(MarketState::Open))
    {
        return Some("trading_window_closed");
    }
    match &command.action {
        CommandAction::Place(payload) => {
            if payload.price <= 0.0 || payload.qty <= 0.0 {
                return Some("validation_failed");
            }
            let price = normalize_price(payload.price, price_step, payload.side);
            let qty = normalize_qty(payload.qty, volume_step);
            if price <= 0.0 || qty <= 0.0 {
                return Some("validation_failed");
            }
        }
        CommandAction::Market(payload) => {
            if payload.qty <= 0.0 {
                return Some("validation_failed");
            }
            let qty = normalize_qty(payload.qty, volume_step);
            if qty <= 0.0 {
                return Some("validation_failed");
            }
        }
        CommandAction::Cancel(payload) => {
            if payload.order_id <= 0 {
                return Some("validation_failed");
            }
        }
        CommandAction::Replace(payload) => {
            if payload.order_id <= 0 || payload.new_price <= 0.0 || payload.new_qty <= 0.0 {
                return Some("validation_failed");
            }
            let price = normalize_step_round(payload.new_price, price_step);
            let qty = normalize_qty(payload.new_qty, volume_step);
            if price <= 0.0 || qty <= 0.0 {
                return Some("validation_failed");
            }
        }
        CommandAction::CreateStopLimit(payload) => {
            if payload.qty <= 0.0
                || payload.price <= 0.0
                || payload.trigger_price <= 0.0
                || payload.stop_end_unix_time <= 0
            {
                return Some("validation_failed");
            }
            let price = normalize_step_round(payload.price, price_step);
            let trigger = normalize_step_round(payload.trigger_price, price_step);
            let qty = normalize_qty(payload.qty, volume_step);
            if price <= 0.0 || trigger <= 0.0 || qty <= 0.0 {
                return Some("validation_failed");
            }
        }
        CommandAction::DeleteStopLimit(payload) => {
            if payload.order_id.trim().is_empty() {
                return Some("validation_failed");
            }
        }
    }
    None
}

fn is_command_expired(command: &OrderCommand) -> bool {
    let Some(ttl_ms) = command.ttl_ms else {
        return false;
    };
    if ttl_ms == 0 || command.created_ts_utc <= 0 {
        return false;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let deadline_ms = command.created_ts_utc.saturating_mul(1_000) + ttl_ms as i64;
    now_ms > deadline_ms
}

fn build_control_path_error_ack(
    request_id: uuid::Uuid,
    error_code: &str,
    error_msg: &str,
) -> CommandAck {
    CommandAck {
        request_id,
        status: alor_protocol::AckStatus::Error,
        broker_order_id: None,
        broker_order_id_str: None,
        error_code: Some(error_code.to_string()),
        error_msg: Some(error_msg.to_string()),
        cws_http_code: None,
        cws_message: None,
        cws_request_guid: None,
        processed_ts_utc: chrono::Utc::now().timestamp(),
    }
}

async fn execute_command(
    cws: &crate::cws_client::CwsHandle,
    action_scope_cws: &ActionScopeCwsManager,
    execution_path: CommandExecutionPath,
    command: &OrderCommand,
    request_id: uuid::Uuid,
    price_step: f64,
    volume_step: f64,
) -> anyhow::Result<serde_json::Value> {
    match &command.action {
        CommandAction::Place(payload) => {
            let request_id_str = request_id.to_string();
            let price = normalize_price(payload.price, price_step, payload.side);
            let qty = normalize_qty(payload.qty, volume_step);
            let response = match execution_path {
                CommandExecutionPath::LegacyLongLived => {
                    cws.create_limit(
                        &command.portfolio,
                        &command.exchange,
                        &command.symbol,
                        price,
                        qty,
                        side_str(payload.side),
                        payload.comment.as_deref(),
                        Some(request_id_str.as_str()),
                    )
                    .await?
                }
                CommandExecutionPath::ActionScoped => {
                    action_scope_cws
                        .create_limit(
                            &command.portfolio,
                            &command.exchange,
                            &command.symbol,
                            price,
                            qty,
                            side_str(payload.side),
                            payload.comment.as_deref(),
                            request_id_str.as_str(),
                        )
                        .await?
                }
            };
            Ok(response)
        }
        CommandAction::Market(payload) => {
            let request_id_str = request_id.to_string();
            let qty = normalize_qty(payload.qty, volume_step);
            let response = cws
                .create_market(
                    &command.portfolio,
                    &command.exchange,
                    &command.symbol,
                    qty,
                    side_str(payload.side),
                    payload.comment.as_deref(),
                    Some(request_id_str.as_str()),
                )
                .await?;
            Ok(response)
        }
        CommandAction::Cancel(payload) => {
            let request_id_str = request_id.to_string();
            let response = match execution_path {
                CommandExecutionPath::LegacyLongLived => {
                    cws.cancel(
                        &command.portfolio,
                        &command.exchange,
                        payload.order_id,
                        Some(request_id_str.as_str()),
                    )
                    .await?
                }
                CommandExecutionPath::ActionScoped => {
                    action_scope_cws
                        .cancel(
                            &command.portfolio,
                            &command.exchange,
                            payload.order_id,
                            request_id_str.as_str(),
                        )
                        .await?
                }
            };
            Ok(response)
        }
        CommandAction::Replace(payload) => {
            let request_id_str = request_id.to_string();
            let new_price = normalize_step_round(payload.new_price, price_step);
            let new_qty = normalize_qty(payload.new_qty, volume_step);
            let response = cws
                .replace(
                    &command.portfolio,
                    &command.exchange,
                    Some(&command.symbol),
                    None,
                    payload.order_id,
                    new_price,
                    new_qty,
                    Some(request_id_str.as_str()),
                )
                .await?;
            Ok(response)
        }
        CommandAction::CreateStopLimit(payload) => {
            let request_id_str = request_id.to_string();
            let qty = normalize_qty(payload.qty, volume_step);
            let trigger_price = normalize_step_round(payload.trigger_price, price_step);
            let price = normalize_step_round(payload.price, price_step);
            let response = cws
                .create_stop_limit(
                    &command.portfolio,
                    &command.exchange,
                    &command.symbol,
                    side_str(payload.side),
                    qty,
                    trigger_price,
                    price,
                    payload.condition,
                    payload.stop_end_unix_time,
                    payload.comment.as_deref(),
                    payload.instrument_group.as_deref(),
                    payload.check_duplicates.unwrap_or(true),
                    Some(request_id_str.as_str()),
                )
                .await?;
            Ok(response)
        }
        CommandAction::DeleteStopLimit(payload) => {
            let request_id_str = request_id.to_string();
            let response = cws
                .delete_stop_limit(
                    &command.portfolio,
                    &command.exchange,
                    &payload.order_id,
                    payload.side.map(side_str),
                    payload.check_duplicates.unwrap_or(true),
                    Some(request_id_str.as_str()),
                )
                .await?;
            Ok(response)
        }
    }
}

#[derive(Debug, Clone)]
struct CwsResponseInfo {
    http_code: Option<i64>,
    message: Option<String>,
    request_guid: Option<String>,
    order_id: Option<i64>,
    order_id_str: Option<String>,
    trace: CwsTraceInfo,
}

#[derive(Debug, Clone, Default)]
struct CwsTraceInfo {
    send_seq: Option<u64>,
    send_ts_utc: Option<i64>,
    recv_seq: Option<u64>,
    recv_ts_utc: Option<i64>,
    message_class: Option<String>,
    connection_instance_id: Option<String>,
    connect_seq: Option<u64>,
    reconnect_seq: Option<u64>,
    order_id: Option<String>,
    request_order_id: Option<String>,
}

fn parse_cws_response(value: &serde_json::Value) -> CwsResponseInfo {
    let http_code = value.get("httpCode").and_then(to_i64);
    let message = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    let request_guid = value
        .get("requestGuid")
        .or_else(|| value.get("guid"))
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    let order_id = extract_order_id(value);
    let order_id_str = extract_order_id_str(value);
    let trace = parse_cws_trace(value);
    CwsResponseInfo {
        http_code,
        message,
        request_guid,
        order_id,
        order_id_str,
        trace,
    }
}

fn parse_cws_trace(value: &serde_json::Value) -> CwsTraceInfo {
    CwsTraceInfo {
        send_seq: value.get(DIAG_CWS_SEND_SEQ).and_then(to_u64),
        send_ts_utc: value.get(DIAG_CWS_SEND_TS_UTC).and_then(to_i64),
        recv_seq: value.get(DIAG_CWS_RECV_SEQ).and_then(to_u64),
        recv_ts_utc: value.get(DIAG_CWS_RECV_TS_UTC).and_then(to_i64),
        message_class: value
            .get(DIAG_CWS_MESSAGE_CLASS)
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string()),
        connection_instance_id: value
            .get(DIAG_CWS_CONNECTION_INSTANCE_ID)
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string()),
        connect_seq: value.get(DIAG_CWS_CONNECT_SEQ).and_then(to_u64),
        reconnect_seq: value.get(DIAG_CWS_RECONNECT_SEQ).and_then(to_u64),
        order_id: value
            .get(DIAG_CWS_ORDER_ID)
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string()),
        request_order_id: value
            .get(DIAG_CWS_REQUEST_ORDER_ID)
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string()),
    }
}

fn extract_order_id(value: &serde_json::Value) -> Option<i64> {
    value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(to_i64)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(to_i64)
            })
        })
}

fn extract_order_id_str(value: &serde_json::Value) -> Option<String> {
    value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(serde_json::Value::as_str)
        .map(|v| v.to_string())
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(serde_json::Value::as_str)
                    .map(|v| v.to_string())
            })
        })
}

fn to_i64(value: &serde_json::Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return Some(v as i64);
    }
    value.as_str().and_then(|value| value.parse::<i64>().ok())
}

fn to_u64(value: &serde_json::Value) -> Option<u64> {
    if let Some(v) = value.as_u64() {
        return Some(v);
    }
    if let Some(v) = value.as_i64() {
        return (v >= 0).then_some(v as u64);
    }
    value.as_str().and_then(|value| value.parse::<u64>().ok())
}

fn build_cws_ack(
    request_id: uuid::Uuid,
    status: alor_protocol::AckStatus,
    info: CwsResponseInfo,
    error_code: Option<String>,
    error_msg: Option<String>,
) -> CommandAck {
    CommandAck {
        request_id,
        status,
        broker_order_id: info.order_id,
        broker_order_id_str: info
            .order_id_str
            .or_else(|| info.order_id.map(|v| v.to_string())),
        error_code,
        error_msg,
        cws_http_code: info.http_code,
        cws_message: info.message,
        cws_request_guid: info.request_guid,
        processed_ts_utc: chrono::Utc::now().timestamp(),
    }
}

fn format_transport_error_message(error: &anyhow::Error) -> String {
    error
        .downcast_ref::<crate::cws_client::CwsRequestFailure>()
        .map(|failure| failure.summary())
        .unwrap_or_else(|| error.to_string())
}

fn build_transport_error_ack(request_id: uuid::Uuid, error: &anyhow::Error) -> CommandAck {
    if let Some(failure) = error.downcast_ref::<crate::cws_client::CwsRequestFailure>() {
        return CommandAck {
            request_id,
            status: alor_protocol::AckStatus::Error,
            broker_order_id: None,
            broker_order_id_str: None,
            error_code: Some("cws_error".to_string()),
            error_msg: Some(failure.summary()),
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: Some(failure.cws_guid().to_string()),
            processed_ts_utc: chrono::Utc::now().timestamp(),
        };
    }
    CommandAck::error(request_id, "cws_error", error.to_string())
}

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn command_target_order_id(action: &CommandAction) -> Option<String> {
    match action {
        CommandAction::Cancel(payload) => Some(payload.order_id.to_string()),
        CommandAction::Replace(payload) => Some(payload.order_id.to_string()),
        CommandAction::DeleteStopLimit(payload) => Some(payload.order_id.clone()),
        _ => None,
    }
}

fn normalize_price(price: f64, step: f64, side: Side) -> f64 {
    if step <= 0.0 {
        return price;
    }
    let scaled = price / step;
    let adjusted = match side {
        Side::Buy => scaled.floor(),
        Side::Sell => scaled.ceil(),
    };
    adjusted * step
}

fn normalize_step_round(value: f64, step: f64) -> f64 {
    if step <= 0.0 {
        value
    } else {
        (value / step).round() * step
    }
}

fn normalize_qty(value: f64, step: f64) -> f64 {
    if step <= 0.0 {
        value
    } else {
        (value / step).floor() * step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_cws_response_success_with_order_number() {
        let value = json!({
            "httpCode": 200,
            "message": "ok",
            "requestGuid": "guid-123",
            "orderNumber": 12345
        });
        let info = parse_cws_response(&value);
        assert_eq!(info.http_code, Some(200));
        assert_eq!(info.message.as_deref(), Some("ok"));
        assert_eq!(info.request_guid.as_deref(), Some("guid-123"));
        assert_eq!(info.order_id, Some(12345));
        assert_eq!(info.trace.recv_seq, None);
    }

    #[test]
    fn parse_cws_response_reject_with_nested_order_id() {
        let value = json!({
            "httpCode": 400,
            "message": "price out of limits",
            "requestGuid": "guid-456",
            "data": { "orderId": 987 }
        });
        let info = parse_cws_response(&value);
        assert_eq!(info.http_code, Some(400));
        assert_eq!(info.message.as_deref(), Some("price out of limits"));
        assert_eq!(info.request_guid.as_deref(), Some("guid-456"));
        assert_eq!(info.order_id, Some(987));
    }

    #[test]
    fn parse_cws_response_handles_string_order_number() {
        let value = json!({
            "httpCode": 200,
            "message": "ok",
            "requestGuid": "guid-789",
            "orderNumber": "A-12345"
        });
        let info = parse_cws_response(&value);
        assert_eq!(info.http_code, Some(200));
        assert_eq!(info.order_id, None);
        assert_eq!(info.order_id_str.as_deref(), Some("A-12345"));
    }

    #[test]
    fn parse_cws_response_extracts_diag_trace_fields() {
        let value = json!({
            "httpCode": 200,
            "requestGuid": "guid-trace",
            "_diag_cws_send_seq": 11,
            "_diag_cws_send_ts_utc": 1774285000,
            "_diag_cws_recv_seq": 12,
            "_diag_cws_recv_ts_utc": 1774285001,
            "_diag_cws_message_class": "response",
            "_diag_cws_connection_instance_id": "conn-1",
            "_diag_cws_connect_seq": 3,
            "_diag_cws_reconnect_seq": 2,
            "_diag_cws_order_id": "2023555931497048623",
            "_diag_cws_request_order_id": "2023555931497048623"
        });
        let info = parse_cws_response(&value);
        assert_eq!(info.trace.send_seq, Some(11));
        assert_eq!(info.trace.send_ts_utc, Some(1774285000));
        assert_eq!(info.trace.recv_seq, Some(12));
        assert_eq!(info.trace.recv_ts_utc, Some(1774285001));
        assert_eq!(info.trace.message_class.as_deref(), Some("response"));
        assert_eq!(info.trace.connection_instance_id.as_deref(), Some("conn-1"));
        assert_eq!(info.trace.connect_seq, Some(3));
        assert_eq!(info.trace.reconnect_seq, Some(2));
        assert_eq!(info.trace.order_id.as_deref(), Some("2023555931497048623"));
        assert_eq!(
            info.trace.request_order_id.as_deref(),
            Some("2023555931497048623")
        );
    }

    #[test]
    fn build_cws_ack_carries_string_order_id() {
        let ack = build_cws_ack(
            uuid::Uuid::new_v4(),
            alor_protocol::AckStatus::Accepted,
            CwsResponseInfo {
                http_code: Some(200),
                message: Some("ok".to_string()),
                request_guid: Some("guid-1".to_string()),
                order_id: None,
                order_id_str: Some("A-12345".to_string()),
                trace: CwsTraceInfo::default(),
            },
            None,
            None,
        );
        assert_eq!(ack.broker_order_id, None);
        assert_eq!(ack.broker_order_id_str.as_deref(), Some("A-12345"));
    }

    #[test]
    fn build_transport_error_ack_preserves_cws_guid() {
        let request_id = uuid::Uuid::new_v4();
        let error = anyhow::Error::new(crate::cws_client::CwsRequestFailure::new(
            Some(request_id.to_string()),
            "guid-1".to_string(),
            "create:limit".to_string(),
            Some("2023555931497048623".to_string()),
            Some("USDRUBF".to_string()),
            crate::cws_client::CwsTransportFailure::new(
                crate::cws_client::CwsDisconnectKind::ProtocolResetWithoutCloseHandshake,
                "Connection reset without closing handshake",
                None,
                None,
            ),
        ));

        let ack = build_transport_error_ack(request_id, &error);
        assert_eq!(ack.status, alor_protocol::AckStatus::Error);
        assert_eq!(ack.error_code.as_deref(), Some("cws_error"));
        assert_eq!(
            ack.error_msg.as_deref(),
            Some("cws disconnected: protocol_reset_without_close_handshake")
        );
        assert_eq!(ack.cws_request_guid.as_deref(), Some("guid-1"));
    }

    #[test]
    fn format_transport_error_message_falls_back_to_plain_error() {
        let error = anyhow::anyhow!("plain error");
        assert_eq!(format_transport_error_message(&error), "plain error");
    }

    fn sample_command(created_ts_utc: i64, ttl_ms: Option<u64>) -> OrderCommand {
        OrderCommand {
            request_id: uuid::Uuid::new_v4(),
            created_ts_utc,
            strategy_id: "s".to_string(),
            portfolio: "p".to_string(),
            exchange: "MOEX".to_string(),
            symbol: "USDRUBF".to_string(),
            action: CommandAction::Market(alor_protocol::MarketOrder {
                side: Side::Buy,
                qty: 1.0,
                comment: None,
            }),
            intent_class: None,
            ttl_ms,
        }
    }

    #[test]
    fn command_not_expired_when_ttl_is_zero() {
        let command = sample_command(chrono::Utc::now().timestamp() - 10, Some(0));
        assert!(!is_command_expired(&command));
    }

    #[test]
    fn command_not_expired_when_ttl_is_set_but_created_ts_missing() {
        let command = sample_command(0, Some(1_000));
        assert!(!is_command_expired(&command));
    }

    #[test]
    fn control_path_hardening_distinguishes_entry_and_exit_place() {
        let mut place_entry = sample_command(chrono::Utc::now().timestamp(), None);
        place_entry.action = CommandAction::Place(alor_protocol::PlaceOrder {
            side: Side::Buy,
            qty: 1.0,
            price: 79.0,
            comment: None,
        });
        place_entry.intent_class = Some(IntentClass::Entry);
        assert_eq!(
            control_path_hardening_policy(&place_entry, &CommandConsumerConfig::default()),
            Some(ControlPathHardeningPolicy::Entry)
        );

        let mut place_exit = place_entry.clone();
        place_exit.intent_class = Some(IntentClass::Exit);
        assert_eq!(
            control_path_hardening_policy(&place_exit, &CommandConsumerConfig::default()),
            Some(ControlPathHardeningPolicy::Exit)
        );

        let market_entry = sample_command(chrono::Utc::now().timestamp(), None);
        assert_eq!(
            control_path_hardening_policy(&market_entry, &CommandConsumerConfig::default()),
            None
        );
    }

    #[test]
    fn execution_path_respects_phase_flags_for_entry_exit_and_cancel() {
        let mut config = CommandConsumerConfig {
            control_cws_mode: ControlCwsMode::ActionScoped,
            action_scope_enable_create_limit: true,
            action_scope_enable_delete_limit: true,
            ..CommandConsumerConfig::default()
        };

        let mut place = sample_command(chrono::Utc::now().timestamp(), None);
        place.action = CommandAction::Place(alor_protocol::PlaceOrder {
            side: Side::Buy,
            qty: 1.0,
            price: 79.0,
            comment: None,
        });
        assert_eq!(
            execution_path_for_command(&place, &config),
            CommandExecutionPath::ActionScoped
        );

        let mut exit_place = place.clone();
        exit_place.intent_class = Some(IntentClass::Exit);
        assert_eq!(
            execution_path_for_command(&exit_place, &config),
            CommandExecutionPath::LegacyLongLived
        );

        config.action_scope_enable_exit = true;
        assert_eq!(
            execution_path_for_command(&exit_place, &config),
            CommandExecutionPath::ActionScoped
        );

        let mut cancel = sample_command(chrono::Utc::now().timestamp(), None);
        cancel.action = CommandAction::Cancel(alor_protocol::CancelOrder { order_id: 42 });
        assert_eq!(
            execution_path_for_command(&cancel, &config),
            CommandExecutionPath::ActionScoped
        );

        config.action_scope_enable_delete_limit = false;
        assert_eq!(
            execution_path_for_command(&cancel, &config),
            CommandExecutionPath::LegacyLongLived
        );
    }

    #[test]
    fn command_expired_when_older_than_ttl() {
        let ttl_ms = 1_000;
        let created_ts_utc = (chrono::Utc::now().timestamp_millis() - 5_000) / 1_000;
        let command = sample_command(created_ts_utc, Some(ttl_ms));
        assert!(is_command_expired(&command));
    }

    #[test]
    fn command_not_expired_when_within_ttl() {
        let ttl_ms = 120_000;
        let created_ts_utc = (chrono::Utc::now().timestamp_millis() - 1_000) / 1_000;
        let command = sample_command(created_ts_utc, Some(ttl_ms));
        assert!(!is_command_expired(&command));
    }
    #[test]
    fn validate_command_rejects_trading_actions_when_market_not_open() {
        let command = sample_command(chrono::Utc::now().timestamp(), None);
        let health = Arc::new(parking_lot::RwLock::new(HealthState {
            gateway_phase: GatewayPhase::LiveReady,
            scheduler_state: Some(MarketState::Break1),
            ..HealthState::default()
        }));

        let error = validate_command(&command, 0.01, 1.0, &health, true);
        assert_eq!(error, Some("trading_window_closed"));
    }

    #[test]
    fn validate_command_allows_cancel_when_market_not_open() {
        let mut command = sample_command(chrono::Utc::now().timestamp(), None);
        command.action = CommandAction::Cancel(alor_protocol::CancelOrder { order_id: 42 });
        let health = Arc::new(parking_lot::RwLock::new(HealthState {
            gateway_phase: GatewayPhase::LiveReady,
            scheduler_state: Some(MarketState::Break2),
            ..HealthState::default()
        }));

        let error = validate_command(&command, 0.01, 1.0, &health, true);
        assert_eq!(error, None);
    }

    #[test]
    fn validate_command_allows_delete_stop_limit_when_market_not_open() {
        let mut command = sample_command(chrono::Utc::now().timestamp(), None);
        command.action = CommandAction::DeleteStopLimit(alor_protocol::DeleteStopLimitOrder {
            order_id: "12345".to_string(),
            side: None,
            check_duplicates: None,
        });
        let health = Arc::new(parking_lot::RwLock::new(HealthState {
            gateway_phase: GatewayPhase::LiveReady,
            scheduler_state: Some(MarketState::Break2),
            ..HealthState::default()
        }));

        let error = validate_command(&command, 0.01, 1.0, &health, true);
        assert_eq!(error, None);
    }
}
