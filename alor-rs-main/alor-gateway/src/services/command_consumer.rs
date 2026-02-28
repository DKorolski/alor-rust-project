use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{debug, info, warn};

use alor_protocol::{CommandAck, CommandAction, OrderCommand, Side};
use alor_types::MarketState;

use crate::health::{GatewayPhase, HealthState};
use crate::transport::{CommandEnvelope, CommandSink, CommandSource};

#[derive(Debug, Clone)]
pub struct CommandConsumerConfig {
    pub pause_when_degraded: bool,
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
            idempotency_ttl: Duration::from_secs(300),
            idempotency_max: 10_000,
            error_backoff_base: Duration::from_millis(50),
            error_backoff_max: Duration::from_secs(2),
            no_message_log_interval: Duration::from_secs(20),
        }
    }
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
                    request_id = %request_id,
                    strategy_id = %command.strategy_id,
                    symbol = %command.symbol,
                    action = %command_action_label(&command.action),
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
                    request_map.write().insert(payload.order_id, request_id);
                }

                match execute_command(&cws, &command, price_step, volume_step).await {
                    Ok(response) => {
                        let info = parse_cws_response(&response);
                        let http_code = info.http_code.unwrap_or(0);
                        info!(
                            request_id = %request_id,
                            action = %command_action_label(&command.action),
                            cws_http_code = http_code,
                            cws_message = ?info.message,
                            cws_request_guid = ?info.request_guid,
                            broker_order_id = info.order_id,
                            "cws response"
                        );
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
                                request_map.write().insert(order_id, request_id);
                            }
                        } else if status == alor_protocol::AckStatus::Rejected {
                            increment_counter(&health, |h| h.commands_rejected_total = h.commands_rejected_total.saturating_add(1));
                            increment_http_code(&health, http_code);
                        } else if status == alor_protocol::AckStatus::Error {
                            increment_counter(&health, |h| h.cws_errors_total = h.cws_errors_total.saturating_add(1));
                        }
                        let ack = build_cws_ack(
                            request_id,
                            status,
                            info,
                            error_code,
                            error_msg,
                        );
                        sink.publish_ack(ack.clone()).await?;
                        info!(
                            request_id = %ack.request_id,
                            status = ?ack.status,
                            processed_ts_utc = ack.processed_ts_utc,
                            broker_order_id = ack.broker_order_id,
                            error_code = ?ack.error_code,
                            cws_http_code = ?ack.cws_http_code,
                            cws_request_guid = ?ack.cws_request_guid,
                            "command ack published"
                        );
                        if let Some(message_id) = message_id.as_deref() {
                            source.ack(message_id).await?;
                        }
                    }
                    Err(error) => {
                        increment_counter(&health, |h| h.cws_errors_total = h.cws_errors_total.saturating_add(1));
                        let ack = CommandAck::error(request_id, "cws_error", format!("{error}"));
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
    if !matches!(&command.action, CommandAction::Cancel(_))
        && !matches!(scheduler_state, Some(MarketState::Open))
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

async fn execute_command(
    cws: &crate::cws_client::CwsHandle,
    command: &OrderCommand,
    price_step: f64,
    volume_step: f64,
) -> anyhow::Result<serde_json::Value> {
    match &command.action {
        CommandAction::Place(payload) => {
            let price = normalize_price(payload.price, price_step, payload.side);
            let qty = normalize_qty(payload.qty, volume_step);
            let response = cws
                .create_limit(
                    &command.portfolio,
                    &command.exchange,
                    &command.symbol,
                    price,
                    qty,
                    side_str(payload.side),
                )
                .await?;
            Ok(response)
        }
        CommandAction::Market(payload) => {
            let qty = normalize_qty(payload.qty, volume_step);
            let response = cws
                .create_market(
                    &command.portfolio,
                    &command.exchange,
                    &command.symbol,
                    qty,
                    side_str(payload.side),
                )
                .await?;
            Ok(response)
        }
        CommandAction::Cancel(payload) => {
            let response = cws
                .cancel(&command.portfolio, &command.exchange, payload.order_id)
                .await?;
            Ok(response)
        }
        CommandAction::Replace(payload) => {
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
    CwsResponseInfo {
        http_code,
        message,
        request_guid,
        order_id,
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

fn to_i64(value: &serde_json::Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return Some(v as i64);
    }
    value.as_str().and_then(|value| value.parse::<i64>().ok())
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
        error_code,
        error_msg,
        cws_http_code: info.http_code,
        cws_message: info.message,
        cws_request_guid: info.request_guid,
        processed_ts_utc: chrono::Utc::now().timestamp(),
    }
}

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
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
            }),
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
}
