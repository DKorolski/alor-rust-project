use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Instant};
use tracing::{info, warn, error};

use alor_protocol::MessageType;

use crate::redis_transport::{RedisRuntimeTransport, RuntimeMessage};
use crate::state::{RuntimeState, StrategyState};
use crate::strategy_limit_cancel::{LimitCancelStateMachine, LimitCancelConfig};
use crate::{OrderEvent, PositionEvent, RuntimeConfig};

const MAX_PENDING_LOOPS: usize = 10;

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeStateSnapshot {
    pub ts_utc: i64,
    pub last_processed_bar_ts: std::collections::HashMap<String, i64>,
    pub strategy_state: StrategyState,
}

pub struct StrategyRuntime {
    config: RuntimeConfig,
    transport: RedisRuntimeTransport,
    state: RuntimeState,
    machine: LimitCancelStateMachine,
    metrics: RuntimeMetrics,
}

#[derive(Debug, Default)]
struct RuntimeMetrics {
    bars_read_total: u64,
    bars_decoded_ok_total: u64,
    bars_decode_failed_total: u64,
    bars_acked_total: u64,
    bars_last_seen_close_time_utc: Option<i64>,
    redis_read_timeouts_total: u64,
    commands_sent_total: u64,
    publish_failures_total: u64,
    last_log: Option<Instant>,
}

impl StrategyRuntime {
    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        let transport = RedisRuntimeTransport::new(config.clone())?;
        let machine = LimitCancelStateMachine::new(
            config.strategy_id.clone(),
            config.portfolio.clone(),
            config.exchange.clone(),
            config.limit_cancel.clone(),
        );
        Ok(Self {
            config,
            transport,
            state: RuntimeState::default(),
            machine,
            metrics: RuntimeMetrics::default(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        self.bootstrap().await?;
        loop {
            self.poll_once().await?;
            sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
        }
    }

    async fn bootstrap(&mut self) -> Result<()> {
        self.transport
            .ensure_groups(&[
                &self.config.streams.bars,
                &self.config.streams.orders,
                &self.config.streams.positions,
                &self.config.streams.acks,
            ])
            .await?;

        self.load_snapshots().await?;
        self.load_runtime_state().await?;

        let streams = self.config.streams.clone();
        let trim_acks = self.config.trim_maxlen_acks;
        let trim_orders = self.config.trim_maxlen_orders;
        let trim_positions = self.config.trim_maxlen_positions;
        let trim_bars = self.config.trim_maxlen_bars;

        self.recover_pending(&streams.acks, MessageType::CommandAck, trim_acks)
            .await?;
        self.recover_pending(&streams.orders, MessageType::Order, trim_orders)
            .await?;
        self.recover_pending(&streams.positions, MessageType::Position, trim_positions)
            .await?;
        self.recover_pending(&streams.bars, MessageType::Bar, trim_bars)
            .await?;

        Ok(())
    }

    async fn load_snapshots(&mut self) -> Result<()> {
        let orders_key = format!("snapshots.orders.{}", self.config.portfolio);
        let positions_key = format!("snapshots.positions.{}", self.config.portfolio);
        let orders = self.transport.hgetall(&orders_key).await?;
        for (_key, value) in orders {
            match serde_json::from_str::<OrderEvent>(&value) {
                Ok(order) => {
                    self.state.orders.insert(order.order_id, order);
                }
                Err(error) => {
                    warn!(?error, "failed to parse order snapshot");
                }
            }
        }
        let positions = self.transport.hgetall(&positions_key).await?;
        for (_key, value) in positions {
            match serde_json::from_str::<PositionEvent>(&value) {
                Ok(position) => {
                    self.state
                        .positions
                        .insert(position.symbol.clone(), position);
                }
                Err(error) => {
                    warn!(?error, "failed to parse position snapshot");
                }
            }
        }
        info!(
            orders = self.state.orders.len(),
            positions = self.state.positions.len(),
            "snapshots loaded"
        );
        Ok(())
    }

    async fn load_runtime_state(&mut self) -> Result<()> {
        if self.config.reset_state_on_start {
            info!("reset_state_on_start enabled; skipping runtime state restore");
            return Ok(());
        }
        if let Some(payload) = self
            .transport
            .xrevrange_last(&self.config.runtime_state_stream)
            .await?
        {
            match serde_json::from_str::<RuntimeStateSnapshot>(&payload) {
                Ok(snapshot) => {
                    self.state.last_processed_bar_ts = snapshot.last_processed_bar_ts;
                    self.state.strategy_state = snapshot.strategy_state.clone();
                    self.machine.state = snapshot.strategy_state;
                    self.machine.last_processed_bar_ts = self
                        .state
                        .last_processed_bar_ts
                        .get(&self.config.limit_cancel.symbol)
                        .copied();
                    info!("runtime state restored");
                }
                Err(error) => {
                    warn!(?error, "failed to parse runtime state snapshot");
                }
            }
        }
        Ok(())
    }

    async fn recover_pending(
        &mut self,
        stream: &str,
        msg_type: MessageType,
        trim_maxlen: usize,
    ) -> Result<()> {
        let mut start = "0-0".to_string();
        for _ in 0..MAX_PENDING_LOOPS {
            let reply = match self
                .transport
                .claim_idle(stream, &start, self.config.claim_batch)
                .await
            {
                Ok(reply) => reply,
                Err(error) => {
                    warn!(?error, stream, "pending autoclaim failed");
                    break;
                }
            };
            let (next_start, entries) = self.transport.parse_autoclaim_entries(stream, reply);
            if entries.is_empty() {
                break;
            }
            start = next_start;
            for entry in entries {
                if let Some(message) = self
                    .transport
                    .decode_entry::<serde_json::Value>(stream, msg_type, trim_maxlen, entry)
                    .await
                {
                    self.dispatch_message(message).await?;
                }
            }
        }
        Ok(())
    }

    async fn poll_once(&mut self) -> Result<()> {
        let streams = self.config.streams.clone();
        let trim_acks = self.config.trim_maxlen_acks;
        let trim_orders = self.config.trim_maxlen_orders;
        let trim_positions = self.config.trim_maxlen_positions;
        let trim_bars = self.config.trim_maxlen_bars;

        self.drain_stream(&streams.acks, MessageType::CommandAck, trim_acks, 10)
            .await?;
        self.drain_stream(&streams.orders, MessageType::Order, trim_orders, 10)
            .await?;
        self.drain_stream(&streams.positions, MessageType::Position, trim_positions, 10)
            .await?;
        self.drain_stream(&streams.bars, MessageType::Bar, trim_bars, 100)
            .await?;
        self.log_metrics_if_due().await?;
        Ok(())
    }

    async fn drain_stream(
        &mut self,
        stream: &str,
        msg_type: MessageType,
        trim_maxlen: usize,
        count: usize,
    ) -> Result<()> {
        let reply = match self.transport.read_group(stream, count).await {
            Ok(reply) => reply,
            Err(error) => {
                warn!(?error, stream, "xreadgroup failed");
                return Ok(());
            }
        };
        let entries = self.transport.parse_read_group_entries(stream, reply);
        if entries.is_empty() {
            self.metrics.redis_read_timeouts_total = self.metrics.redis_read_timeouts_total.saturating_add(1);
            return Ok(());
        }
        if msg_type == MessageType::Bar {
            self.metrics.bars_read_total = self.metrics.bars_read_total.saturating_add(entries.len() as u64);
        }
        for entry in entries {
            if let Some(message) = self
                .transport
                .decode_entry::<serde_json::Value>(stream, msg_type, trim_maxlen, entry)
                .await
            {
                self.dispatch_message(message).await?;
                if msg_type == MessageType::Bar {
                    self.metrics.bars_decoded_ok_total = self.metrics.bars_decoded_ok_total.saturating_add(1);
                }
            } else if msg_type == MessageType::Bar {
                self.metrics.bars_decode_failed_total =
                    self.metrics.bars_decode_failed_total.saturating_add(1);
            }
        }
        Ok(())
    }

    async fn dispatch_message(&mut self, message: RuntimeMessage<serde_json::Value>) -> Result<()> {
        match message.stream.as_str() {
            stream if stream == self.config.streams.acks => {
                let ack = serde_json::from_value(message.payload)?;
                self.handle_ack(message.stream, message.message_id, ack).await?;
            }
            stream if stream == self.config.streams.orders => {
                let order = serde_json::from_value(message.payload)?;
                self.handle_order(message.stream, message.message_id, order).await?;
            }
            stream if stream == self.config.streams.positions => {
                let position = serde_json::from_value(message.payload)?;
                self.handle_position(message.stream, message.message_id, position)
                    .await?;
            }
            stream if stream == self.config.streams.bars => {
                let bar = serde_json::from_value(message.payload)?;
                self.handle_bar(message.stream, message.message_id, bar).await?;
            }
            _ => {
                warn!(stream = message.stream, "unknown stream message");
                let _ = self.transport.xack(&message.stream, &message.message_id).await;
            }
        }
        Ok(())
    }

    async fn handle_ack(
        &mut self,
        stream: String,
        message_id: String,
        ack: alor_protocol::CommandAck,
    ) -> Result<()> {
        let maybe_cmd = self.machine.on_ack(&ack);
        self.state.strategy_state = self.machine.state.clone();
        self.persist_state(maybe_cmd.as_ref()).await?;
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    async fn handle_order(
        &mut self,
        stream: String,
        message_id: String,
        order: OrderEvent,
    ) -> Result<()> {
        let maybe_cmd = self.machine.on_order_event(&order);
        self.state.strategy_state = self.machine.state.clone();
        self.persist_state(maybe_cmd.as_ref()).await?;
        self.state.orders.insert(order.order_id, order);
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    async fn handle_position(
        &mut self,
        stream: String,
        message_id: String,
        position: PositionEvent,
    ) -> Result<()> {
        self.state
            .positions
            .insert(position.symbol.clone(), position);
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    async fn handle_bar(
        &mut self,
        stream: String,
        message_id: String,
        bar: crate::BarEvent,
    ) -> Result<()> {
        if bar.origin != crate::DataOrigin::Live {
            self.state.update_last_bar_ts(&bar.symbol, bar.close_time_utc);
            self.metrics.bars_last_seen_close_time_utc = Some(bar.close_time_utc);
            self.persist_state(None).await?;
            self.transport.xack(&stream, &message_id).await?;
            self.metrics.bars_acked_total = self.metrics.bars_acked_total.saturating_add(1);
            return Ok(());
        }
        if self.state.is_duplicate_bar(&bar.symbol, bar.close_time_utc) {
            self.transport.xack(&stream, &message_id).await?;
            self.metrics.bars_acked_total = self.metrics.bars_acked_total.saturating_add(1);
            return Ok(());
        }
        let maybe_cmd = self.machine.on_bar(&bar);
        self.state.strategy_state = self.machine.state.clone();
        self.state.update_last_bar_ts(&bar.symbol, bar.close_time_utc);
        self.metrics.bars_last_seen_close_time_utc = Some(bar.close_time_utc);
        self.persist_state(maybe_cmd.as_ref()).await?;
        self.transport.xack(&stream, &message_id).await?;
        self.metrics.bars_acked_total = self.metrics.bars_acked_total.saturating_add(1);
        Ok(())
    }

    async fn persist_state(
        &mut self,
        maybe_cmd: Option<&alor_protocol::OrderCommand>,
    ) -> Result<()> {
        let snapshot = RuntimeStateSnapshot {
            ts_utc: Utc::now().timestamp(),
            last_processed_bar_ts: self.state.last_processed_bar_ts.clone(),
            strategy_state: self.state.strategy_state.clone(),
        };
        let payload = serde_json::to_string(&snapshot)?;
        if let Some(command) = maybe_cmd {
            if let Err(error) = self
                .transport
                .publish_command_and_state(command, &payload)
                .await
            {
                self.metrics.publish_failures_total =
                    self.metrics.publish_failures_total.saturating_add(1);
                error!(
                    ?error,
                    request_id = %command.request_id,
                    "failed to publish command and state"
                );
                return Err(error);
            }
            self.metrics.commands_sent_total =
                self.metrics.commands_sent_total.saturating_add(1);
        } else {
            if let Err(error) = self
                .transport
                .xadd_state(
                    &self.config.runtime_state_stream,
                    &payload,
                    self.config.trim_maxlen_runtime_state,
                )
                .await
            {
                self.metrics.publish_failures_total =
                    self.metrics.publish_failures_total.saturating_add(1);
                error!(?error, "failed to persist runtime state");
                return Err(error);
            }
        }
        Ok(())
    }

    async fn log_metrics_if_due(&mut self) -> Result<()> {
        let now = Instant::now();
        let log_due = match self.metrics.last_log {
            Some(last) => now.duration_since(last) >= Duration::from_secs(5),
            None => true,
        };
        if !log_due {
            return Ok(());
        }
        self.metrics.last_log = Some(now);
        info!(
            bars_read_total = self.metrics.bars_read_total,
            bars_decoded_ok_total = self.metrics.bars_decoded_ok_total,
            bars_decode_failed_total = self.metrics.bars_decode_failed_total,
            bars_acked_total = self.metrics.bars_acked_total,
            bars_last_seen_close_time_utc = self.metrics.bars_last_seen_close_time_utc,
            redis_read_timeouts_total = self.metrics.redis_read_timeouts_total,
            commands_sent_total = self.metrics.commands_sent_total,
            publish_failures_total = self.metrics.publish_failures_total,
            "runtime bars metrics"
        );
        if self.metrics.bars_read_total == 0 {
            let xlen = self.transport.xlen(&self.config.streams.bars).await.unwrap_or(0);
            if xlen > 0 {
                error!(
                    bars_stream = self.config.streams.bars,
                    consumer_group = self.config.consumer_group,
                    consumer_name = self.config.consumer_name,
                    xlen,
                    "bars stream has data but runtime reads none — check group/consumer and start id"
                );
            }
        }
        Ok(())
    }
}

pub fn default_limit_cancel(symbol: String) -> LimitCancelConfig {
    LimitCancelConfig {
        symbol,
        tick_size: 0.01,
        offset_ticks: 1,
        qty: 1.0,
        side: alor_protocol::Side::Buy,
        max_wait_bars_for_ack: 3,
    }
}
