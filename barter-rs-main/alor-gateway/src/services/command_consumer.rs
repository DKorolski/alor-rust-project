use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use alor_protocol::{CommandAck, CommandAction, OrderCommand, Side};

use crate::health::{GatewayPhase, HealthState};
use crate::transport::{CommandEnvelope, CommandSink, CommandSource};

#[derive(Debug, Clone)]
pub struct CommandConsumerConfig {
    pub pause_when_degraded: bool,
    pub idempotency_ttl: Duration,
    pub idempotency_max: usize,
}

impl Default for CommandConsumerConfig {
    fn default() -> Self {
        Self {
            pause_when_degraded: true,
            idempotency_ttl: Duration::from_secs(300),
            idempotency_max: 10_000,
        }
    }
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

pub async fn run_command_consumer(
    mut source: Box<dyn CommandSource>,
    sink: Arc<dyn CommandSink>,
    idempotency: Arc<dyn IdempotencyStore>,
    cws: crate::cws_client::CwsHandle,
    price_step: f64,
    volume_step: f64,
    health: Arc<parking_lot::RwLock<HealthState>>,
    shutdown_rx: &mut watch::Receiver<bool>,
    config: CommandConsumerConfig,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            envelope = source.next_command() => {
                let Some(CommandEnvelope { command, message_id }) = envelope else {
                    break;
                };
                let request_id = command.request_id;
                if !idempotency.check_and_set(request_id).await? {
                    increment_counter(&health, |h| h.command_duplicate_total = h.command_duplicate_total.saturating_add(1));
                    sink.publish_ack(CommandAck::duplicate(request_id)).await?;
                    if let Some(message_id) = message_id.as_deref() {
                        source.ack(message_id).await?;
                    }
                    continue;
                }

                if let Some(error_code) = validate_command(&command, price_step, volume_step, &health, config.pause_when_degraded) {
                    increment_counter(&health, |h| h.command_validation_failed_total = h.command_validation_failed_total.saturating_add(1));
                    sink.publish_ack(CommandAck::error(request_id, error_code, "validation failed"))
                        .await?;
                    if let Some(message_id) = message_id.as_deref() {
                        source.ack(message_id).await?;
                    }
                    continue;
                }

                if is_command_expired(&command) {
                    increment_counter(&health, |h| h.command_expired_total = h.command_expired_total.saturating_add(1));
                    sink.publish_ack(CommandAck::error(request_id, "expired", "command expired"))
                        .await?;
                    if let Some(message_id) = message_id.as_deref() {
                        source.ack(message_id).await?;
                    }
                    continue;
                }

                match execute_command(&cws, &command, price_step, volume_step).await {
                    Ok(order_id) => {
                        increment_counter(&health, |h| h.command_processed_total = h.command_processed_total.saturating_add(1));
                        sink.publish_ack(CommandAck::success(request_id, order_id))
                            .await?;
                        if let Some(message_id) = message_id.as_deref() {
                            source.ack(message_id).await?;
                        }
                    }
                    Err(error) => {
                        sink.publish_ack(CommandAck::error(
                            request_id,
                            "command_failed",
                            format!("{error}"),
                        ))
                        .await?;
                        if let Some(message_id) = message_id.as_deref() {
                            source.ack(message_id).await?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn increment_counter(health: &Arc<parking_lot::RwLock<HealthState>>, update: impl FnOnce(&mut HealthState)) {
    let mut guard = health.write();
    update(&mut guard);
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
    drop(guard);
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
    let now_ms = chrono::Utc::now().timestamp_millis();
    let deadline_ms = command.created_ts_utc.saturating_mul(1_000) + ttl_ms as i64;
    now_ms > deadline_ms
}

async fn execute_command(
    cws: &crate::cws_client::CwsHandle,
    command: &OrderCommand,
    price_step: f64,
    volume_step: f64,
) -> anyhow::Result<Option<i64>> {
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
            Ok(response.get("orderId").and_then(|value| value.as_i64()))
        }
        CommandAction::Cancel(payload) => {
            let response = cws.cancel(payload.order_id).await?;
            Ok(response.get("orderId").and_then(|value| value.as_i64()))
        }
        CommandAction::Replace(payload) => {
            let new_price = normalize_step_round(payload.new_price, price_step);
            let new_qty = normalize_qty(payload.new_qty, volume_step);
            let response = cws
                .replace(payload.order_id, new_price, new_qty)
                .await?;
            Ok(response.get("orderId").and_then(|value| value.as_i64()))
        }
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

