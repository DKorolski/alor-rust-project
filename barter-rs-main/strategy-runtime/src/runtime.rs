use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Instant};
use tracing::{debug, error, info, warn};

use alor_protocol::{Envelope, MessageType};

use crate::live_guard::{evaluate_live_guard, HealthEvent, LiveGuardDecision, LiveGuardState};
use crate::redis_transport::{RedisRuntimeTransport, RuntimeMessage};
use crate::state::{RuntimeState, StrategyState};
use crate::strategies::limit_cancel::LimitCancelStrategy;
use crate::strategies::market_buy_and_close::MarketBuyAndCloseStrategy;
use crate::trade_ledger::{OrderRecord, TradeLedger, TradeRecord};
use crate::{
    BacktestConfig, BarEvent, DataOrigin, Intent, OrderEvent, PaperConfig, PaperOutput,
    PositionEvent, RuntimeConfig, Strategy, StrategyCtx, StrategyKind, TradeMode,
};

const MAX_PENDING_LOOPS: usize = 10;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(2);
const BARS_STREAM_INFO_GRACE: Duration = Duration::from_secs(30);
const BARS_STREAM_WARN_GRACE: Duration = Duration::from_secs(120);
const SNAPSHOT_SCAN_COUNT: usize = 200;

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeStateSnapshot {
    pub ts_utc: i64,
    pub last_processed_bar_ts: std::collections::HashMap<String, i64>,
    pub strategy_state: StrategyState,
}

#[derive(Debug, Deserialize)]
struct OrdersSnapshot {
    pub orders: HashMap<i64, OrderEvent>,
}

#[derive(Debug, Deserialize)]
struct PositionsSnapshot {
    pub positions: HashMap<String, PositionEvent>,
}

#[derive(Debug, Default, Clone)]
struct BootstrapState {
    orders_snapshot_loaded: bool,
    positions_snapshot_loaded: bool,
    seen_live_bar: bool,
}

impl BootstrapState {
    fn ready(&self) -> bool {
        self.orders_snapshot_loaded && self.positions_snapshot_loaded && self.seen_live_bar
    }

    fn reasons(&self) -> Vec<String> {
        if self.ready() {
            return Vec::new();
        }
        let mut reasons = Vec::new();
        reasons.push("bootstrap:not_ready".to_string());
        if !self.orders_snapshot_loaded {
            reasons.push("bootstrap:missing_orders_snapshot".to_string());
        }
        if !self.positions_snapshot_loaded {
            reasons.push("bootstrap:missing_positions_snapshot".to_string());
        }
        if !self.seen_live_bar {
            reasons.push("bootstrap:missing_live_bar".to_string());
        }
        reasons
    }
}

pub struct StrategyRuntime {
    config: RuntimeConfig,
    transport: RedisRuntimeTransport,
    state: RuntimeState,
    strategy: Box<dyn Strategy + Send + Sync>,
    live_guard: LiveGuardState,
    bootstrap_state: BootstrapState,
    metrics: RuntimeMetrics,
    ledger: TradeLedger,
    sim_orders: Vec<SimOrder>,
    next_sim_order_id: i64,
}

#[derive(Debug)]
struct RuntimeMetrics {
    bars_read_total: u64,
    bars_decoded_ok_total: u64,
    bars_decode_failed_total: u64,
    bars_acked_total: u64,
    bars_last_seen_close_time_utc: Option<i64>,
    redis_empty_polls_total: u64,
    redis_read_errors_total: u64,
    commands_sent_total: u64,
    publish_failures_total: u64,
    start_time: Instant,
    waiting_for_first_bar_info_logged: bool,
    bars_stream_xlen_last: Option<u64>,
    last_log: Option<Instant>,
    last_guard_log: Option<Instant>,
    last_health_poll: Option<Instant>,
}

#[derive(Debug, Clone)]
struct SimOrder {
    order_id: i64,
    symbol: String,
    side: String,
    order_type: SimOrderType,
    qty: f64,
    price: Option<f64>,
    created_bar_ts: i64,
}

#[derive(Debug, Clone, Copy)]
enum SimOrderType {
    Market,
    Limit,
}

impl Default for RuntimeMetrics {
    fn default() -> Self {
        Self {
            bars_read_total: 0,
            bars_decoded_ok_total: 0,
            bars_decode_failed_total: 0,
            bars_acked_total: 0,
            bars_last_seen_close_time_utc: None,
            redis_empty_polls_total: 0,
            redis_read_errors_total: 0,
            commands_sent_total: 0,
            publish_failures_total: 0,
            start_time: Instant::now(),
            waiting_for_first_bar_info_logged: false,
            bars_stream_xlen_last: None,
            last_log: None,
            last_guard_log: None,
            last_health_poll: None,
        }
    }
}

impl StrategyRuntime {
    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        let transport = RedisRuntimeTransport::new(config.clone())?;
        let strategy: Box<dyn Strategy + Send + Sync> = match config.strategy.strategy_kind {
            StrategyKind::LimitCancel => Box::new(LimitCancelStrategy::new(
                config.strategy.to_limit_cancel_config(),
            )),
            StrategyKind::MarketBuyAndClose => Box::new(MarketBuyAndCloseStrategy::new(
                config.strategy.to_market_buy_and_close_config(),
            )),
        };
        Ok(Self {
            config,
            transport,
            state: RuntimeState::default(),
            strategy,
            live_guard: LiveGuardState::default(),
            bootstrap_state: BootstrapState::default(),
            metrics: RuntimeMetrics::default(),
            ledger: TradeLedger::default(),
            sim_orders: Vec::new(),
            next_sim_order_id: 1,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        self.bootstrap().await?;
        loop {
            self.poll_once().await?;
            sleep(Duration::from_millis(self.config.read.poll_interval_ms)).await;
        }
    }

    pub async fn flush_reports(&self) -> Result<()> {
        self.persist_ledger_reports().await
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
        let trim_acks = self.config.trim.acks;
        let trim_orders = self.config.trim.orders;
        let trim_positions = self.config.trim.positions;
        let trim_bars = self.config.trim.bars;

        self.recover_pending(&streams.acks, MessageType::CommandAck, trim_acks)
            .await?;
        self.recover_pending(&streams.orders, MessageType::Order, trim_orders)
            .await?;
        self.recover_pending(&streams.positions, MessageType::Position, trim_positions)
            .await?;
        self.recover_pending(&streams.bars, MessageType::Bar, trim_bars)
            .await?;

        self.refresh_health_if_due().await?;
        self.log_live_guard_status_if_due().await?;

        Ok(())
    }

    async fn load_snapshots(&mut self) -> Result<()> {
        let stream = match &self.config.streams.snapshots {
            Some(stream) => stream,
            None => {
                warn!("bootstrap: snapshots stream not configured");
                return Ok(());
            }
        };
        info!(
            stream,
            scan = SNAPSHOT_SCAN_COUNT,
            "bootstrap: loading snapshots"
        );
        let payloads = self
            .transport
            .xrevrange_last_n(stream, SNAPSHOT_SCAN_COUNT)
            .await?;
        let mut orders_snapshot: Option<OrdersSnapshot> = None;
        let mut positions_snapshot: Option<PositionsSnapshot> = None;
        for payload in payloads {
            let envelope = match serde_json::from_str::<Envelope<serde_json::Value>>(&payload) {
                Ok(envelope) => envelope,
                Err(error) => {
                    warn!(?error, "failed to parse snapshot envelope");
                    continue;
                }
            };
            match envelope.msg_type {
                MessageType::SnapshotOrders if orders_snapshot.is_none() => {
                    match serde_json::from_str::<Envelope<OrdersSnapshot>>(&payload) {
                        Ok(envelope) => {
                            orders_snapshot = Some(envelope.payload);
                        }
                        Err(error) => {
                            warn!(?error, "failed to parse orders snapshot");
                        }
                    }
                }
                MessageType::SnapshotPositions if positions_snapshot.is_none() => {
                    match serde_json::from_str::<Envelope<PositionsSnapshot>>(&payload) {
                        Ok(envelope) => {
                            positions_snapshot = Some(envelope.payload);
                        }
                        Err(error) => {
                            warn!(?error, "failed to parse positions snapshot");
                        }
                    }
                }
                _ => {}
            }
            if orders_snapshot.is_some() && positions_snapshot.is_some() {
                break;
            }
        }

        if let Some(snapshot) = orders_snapshot {
            self.state.orders = snapshot.orders;
            self.bootstrap_state.orders_snapshot_loaded = true;
        }
        if let Some(snapshot) = positions_snapshot {
            self.state.positions = snapshot.positions;
            self.bootstrap_state.positions_snapshot_loaded = true;
        }

        if !self.bootstrap_state.orders_snapshot_loaded
            || !self.bootstrap_state.positions_snapshot_loaded
        {
            if self.config.trade_mode == TradeMode::Live {
                anyhow::bail!(
                    "bootstrap: snapshots missing orders={} positions={}",
                    self.bootstrap_state.orders_snapshot_loaded,
                    self.bootstrap_state.positions_snapshot_loaded
                );
            }
            warn!(
                orders_loaded = self.bootstrap_state.orders_snapshot_loaded,
                positions_loaded = self.bootstrap_state.positions_snapshot_loaded,
                "bootstrap: snapshots missing"
            );
        } else {
            info!(
                orders = self.state.orders.len(),
                positions = self.state.positions.len(),
                "bootstrap: snapshots loaded"
            );
        }

        Ok(())
    }

    async fn load_runtime_state(&mut self) -> Result<()> {
        if self.config.reset_state_on_start {
            info!("reset_state_on_start enabled; skipping runtime state restore");
            return Ok(());
        }
        if let Some(payload) = self
            .transport
            .xrevrange_last(&self.config.streams.runtime_state)
            .await?
        {
            match serde_json::from_str::<RuntimeStateSnapshot>(&payload) {
                Ok(snapshot) => {
                    self.state.last_processed_bar_ts = snapshot.last_processed_bar_ts;
                    self.state.strategy_state = snapshot.strategy_state.clone();
                    self.strategy.set_state(snapshot.strategy_state);
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
                .claim_idle(stream, &start, self.config.read.claim_batch)
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
        let trim_acks = self.config.trim.acks;
        let trim_orders = self.config.trim.orders;
        let trim_positions = self.config.trim.positions;
        let trim_bars = self.config.trim.bars;

        self.drain_stream(&streams.acks, MessageType::CommandAck, trim_acks, 10)
            .await?;
        self.drain_stream(&streams.orders, MessageType::Order, trim_orders, 10)
            .await?;
        self.drain_stream(
            &streams.positions,
            MessageType::Position,
            trim_positions,
            10,
        )
        .await?;
        self.drain_stream(&streams.bars, MessageType::Bar, trim_bars, 100)
            .await?;
        self.refresh_health_if_due().await?;
        self.log_live_guard_status_if_due().await?;
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
                self.metrics.redis_read_errors_total =
                    self.metrics.redis_read_errors_total.saturating_add(1);
                return Ok(());
            }
        };
        let entries = self.transport.parse_read_group_entries(stream, reply);
        if entries.is_empty() {
            self.metrics.redis_empty_polls_total =
                self.metrics.redis_empty_polls_total.saturating_add(1);
            return Ok(());
        }
        if msg_type == MessageType::Bar {
            self.metrics.bars_read_total = self
                .metrics
                .bars_read_total
                .saturating_add(entries.len() as u64);
        }
        for entry in entries {
            if let Some(message) = self
                .transport
                .decode_entry::<serde_json::Value>(stream, msg_type, trim_maxlen, entry)
                .await
            {
                self.dispatch_message(message).await?;
                if msg_type == MessageType::Bar {
                    self.metrics.bars_decoded_ok_total =
                        self.metrics.bars_decoded_ok_total.saturating_add(1);
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
                self.handle_ack(message.stream, message.message_id, ack)
                    .await?;
            }
            stream if stream == self.config.streams.orders => {
                let order = serde_json::from_value(message.payload)?;
                self.handle_order(message.stream, message.message_id, order)
                    .await?;
            }
            stream if stream == self.config.streams.positions => {
                let position = serde_json::from_value(message.payload)?;
                self.handle_position(message.stream, message.message_id, position)
                    .await?;
            }
            stream if stream == self.config.streams.bars => {
                let bar = serde_json::from_value(message.payload)?;
                self.handle_bar(message.stream, message.message_id, bar)
                    .await?;
            }
            _ => {
                warn!(stream = message.stream, "unknown stream message");
                let _ = self
                    .transport
                    .xack(&message.stream, &message.message_id)
                    .await;
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
        match ack.status {
            alor_protocol::AckStatus::Rejected
            | alor_protocol::AckStatus::Expired
            | alor_protocol::AckStatus::Error => {
                warn!(
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    error_msg = ?ack.error_msg,
                    cws_http_code = ?ack.cws_http_code,
                    cws_request_guid = ?ack.cws_request_guid,
                    "command rejected"
                );
            }
            alor_protocol::AckStatus::Accepted | alor_protocol::AckStatus::Confirmed => {
                info!(
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    broker_order_id = ack.broker_order_id,
                    "command accepted"
                );
            }
            alor_protocol::AckStatus::Duplicate => {
                info!(
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    "command duplicate"
                );
            }
        }
        let ctx = self.strategy_ctx();
        let intents = self.strategy.on_ack(&ctx, &ack);
        self.state.strategy_state = self.strategy.state().clone();
        self.apply_intents(&ctx, ack.processed_ts_utc, intents)
            .await?;
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    async fn handle_order(
        &mut self,
        stream: String,
        message_id: String,
        order: OrderEvent,
    ) -> Result<()> {
        let ctx = self.strategy_ctx();
        let created_ts = ctx.last_bar_ts().unwrap_or(0);
        let intents = self.strategy.on_order(&ctx, &order);
        self.state.strategy_state = self.strategy.state().clone();
        self.apply_intents(&ctx, created_ts, intents).await?;
        self.update_ledger_from_order(&order)?;
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
        let ctx = self.strategy_ctx();
        let created_ts = ctx.last_bar_ts().unwrap_or(0);
        let intents = self.strategy.on_position(&ctx, &position);
        self.state.strategy_state = self.strategy.state().clone();
        self.apply_intents(&ctx, created_ts, intents).await?;
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
        if self.state.is_duplicate_bar(&bar.symbol, bar.close_time_utc) {
            self.transport.xack(&stream, &message_id).await?;
            self.metrics.bars_acked_total = self.metrics.bars_acked_total.saturating_add(1);
            return Ok(());
        }
        self.state
            .update_last_bar_ts(&bar.symbol, bar.close_time_utc);
        if bar.origin == DataOrigin::Live {
            self.bootstrap_state.seen_live_bar = true;
        }
        let ctx = self.strategy_ctx();
        let intents = self.strategy.on_bar(&ctx, &bar);
        self.state.strategy_state = self.strategy.state().clone();
        self.metrics.bars_last_seen_close_time_utc = Some(bar.close_time_utc);
        if self.config.trade_mode != TradeMode::Live {
            self.simulate_fills(&bar).await?;
            self.simulate_intents(&bar, intents).await?;
            self.persist_state(None).await?;
        } else {
            self.apply_intents(&ctx, bar.close_time_utc, intents)
                .await?;
        }
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
            self.metrics.commands_sent_total = self.metrics.commands_sent_total.saturating_add(1);
        } else {
            if let Err(error) = self
                .transport
                .xadd_state(
                    &self.config.streams.runtime_state,
                    &payload,
                    self.config.trim.runtime_state,
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

    fn update_ledger_from_order(&mut self, order: &OrderEvent) -> Result<()> {
        if order.order_id <= 0 {
            return Ok(());
        }
        let status = order.status.to_lowercase();
        let record = OrderRecord {
            order_id: order.order_id,
            symbol: order.symbol.clone(),
            side: order.side.to_lowercase(),
            qty: order.qty,
            filled: order.filled,
            price: order.price,
            status: status.clone(),
            ts_utc: order.ts_utc,
        };
        self.ledger.record_order(record);
        if status == "filled" {
            let fill_qty = if order.filled > 0.0 { order.filled } else { order.qty };
            if fill_qty > 0.0 {
                let trade = TradeRecord {
                    ts_utc: order.ts_utc,
                    order_id: order.order_id,
                    symbol: order.symbol.clone(),
                    side: order.side.to_lowercase(),
                    qty: fill_qty,
                    price: order.price,
                    commission: 0.0,
                };
                self.ledger.record_fill(trade);
                info!(
                    order_id = order.order_id,
                    symbol = order.symbol,
                    qty = fill_qty,
                    price = order.price,
                    "order filled"
                );
            }
        }
        Ok(())
    }

    async fn simulate_intents(&mut self, bar: &BarEvent, intents: Vec<Intent>) -> Result<()> {
        for intent in intents {
            match intent {
                Intent::Place { price, qty, side } => {
                    let order_id = self.next_sim_order_id;
                    self.next_sim_order_id += 1;
                    let side = format!("{side:?}").to_lowercase();
                    self.sim_orders.push(SimOrder {
                        order_id,
                        symbol: bar.symbol.clone(),
                        side: side.clone(),
                        order_type: SimOrderType::Limit,
                        qty,
                        price: Some(price),
                        created_bar_ts: bar.close_time_utc,
                    });
                    self.ledger.record_order(OrderRecord {
                        order_id,
                        symbol: bar.symbol.clone(),
                        side,
                        qty,
                        filled: 0.0,
                        price,
                        status: "working".to_string(),
                        ts_utc: bar.close_time_utc,
                    });
                }
                Intent::Market { qty, side, .. } => {
                    let order_id = self.next_sim_order_id;
                    self.next_sim_order_id += 1;
                    let side = format!("{side:?}").to_lowercase();
                    self.sim_orders.push(SimOrder {
                        order_id,
                        symbol: bar.symbol.clone(),
                        side: side.clone(),
                        order_type: SimOrderType::Market,
                        qty,
                        price: None,
                        created_bar_ts: bar.close_time_utc,
                    });
                    self.ledger.record_order(OrderRecord {
                        order_id,
                        symbol: bar.symbol.clone(),
                        side,
                        qty,
                        filled: 0.0,
                        price: 0.0,
                        status: "working".to_string(),
                        ts_utc: bar.close_time_utc,
                    });
                }
                Intent::Cancel { order_id } => {
                    if let Some(pos) = self
                        .sim_orders
                        .iter()
                        .position(|order| order.order_id == order_id)
                    {
                        let order = self.sim_orders.remove(pos);
                        self.ledger.record_order(OrderRecord {
                            order_id: order.order_id,
                            symbol: order.symbol.clone(),
                            side: order.side,
                            qty: order.qty,
                            filled: 0.0,
                            price: order.price.unwrap_or(0.0),
                            status: "canceled".to_string(),
                            ts_utc: bar.close_time_utc,
                        });
                    }
                }
                Intent::Replace {
                    order_id,
                    new_price,
                    new_qty,
                } => {
                    if let Some(order) = self
                        .sim_orders
                        .iter_mut()
                        .find(|order| order.order_id == order_id)
                    {
                        order.price = Some(new_price);
                        order.qty = new_qty;
                        self.ledger.record_order(OrderRecord {
                            order_id,
                            symbol: order.symbol.clone(),
                            side: order.side.clone(),
                            qty: order.qty,
                            filled: 0.0,
                            price: new_price,
                            status: "working".to_string(),
                            ts_utc: bar.close_time_utc,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    async fn simulate_fills(&mut self, bar: &BarEvent) -> Result<()> {
        let mut filled = Vec::new();
        for order in &self.sim_orders {
            if bar.close_time_utc <= order.created_bar_ts {
                continue;
            }
            let fill_price = match order.order_type {
                SimOrderType::Market => Some(bar.o),
                SimOrderType::Limit => {
                    let price = order.price.unwrap_or(0.0);
                    if order.side == "buy" && bar.l <= price {
                        Some(price)
                    } else if order.side == "sell" && bar.h >= price {
                        Some(price)
                    } else {
                        None
                    }
                }
            };
            if let Some(price) = fill_price {
                filled.push((order.order_id, price));
            }
        }
        for (order_id, price) in filled {
            if let Some(index) = self
                .sim_orders
                .iter()
                .position(|order| order.order_id == order_id)
            {
                let order = self.sim_orders.remove(index);
                let trade = TradeRecord {
                    ts_utc: bar.close_time_utc,
                    order_id: order.order_id,
                    symbol: order.symbol.clone(),
                    side: order.side.clone(),
                    qty: order.qty,
                    price,
                    commission: 0.0,
                };
                self.ledger.record_fill(trade);
                self.ledger.record_order(OrderRecord {
                    order_id: order.order_id,
                    symbol: order.symbol.clone(),
                    side: order.side,
                    qty: order.qty,
                    filled: order.qty,
                    price,
                    status: "filled".to_string(),
                    ts_utc: bar.close_time_utc,
                });
                self.persist_ledger_reports().await?;
            }
        }
        Ok(())
    }

    async fn persist_ledger_reports(&self) -> Result<()> {
        match self.config.trade_mode {
            TradeMode::Paper => self
                .ledger
                .persist_reports(
                    &self.config.strategy.strategy_id,
                    &self.config.strategy.symbol,
                    &self.config.paper.trades_csv,
                    &self.config.paper.summary_json,
                ),
            TradeMode::Backtest => self.ledger.persist_reports(
                &self.config.strategy.strategy_id,
                &self.config.strategy.symbol,
                &self.config.backtest.trades_csv,
                &self.config.backtest.summary_json,
            ),
            TradeMode::Live => Ok(()),
        }
    }

    fn strategy_ctx(&self) -> StrategyCtx {
        let gateway_phase = self
            .live_guard
            .health
            .as_ref()
            .map(|health| health.gateway_phase)
            .unwrap_or_default();
        let position_qty = self
            .state
            .positions
            .get(&self.config.strategy.symbol)
            .map(|pos| pos.qty);
        StrategyCtx {
            strategy_id: self.config.strategy.strategy_id.clone(),
            portfolio: self.config.portfolio.clone(),
            exchange: self.config.exchange.clone(),
            symbol: self.config.strategy.symbol.clone(),
            tick_size: self.config.strategy.tick_size,
            trade_mode: self.config.trade_mode,
            allow_live_orders: self.config.allow_live_orders,
            gateway_phase,
            position_qty,
            last_bar_ts: self
                .state
                .last_processed_bar_ts
                .get(&self.config.strategy.symbol)
                .copied(),
        }
    }

    async fn apply_intents(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        intents: Vec<Intent>,
    ) -> Result<()> {
        if intents.is_empty() {
            self.persist_state(None).await?;
            return Ok(());
        }
        match self.config.trade_mode {
            TradeMode::Live => {
                let decision = self.evaluate_guard_decision();
                if !decision.allowed {
                    self.log_guard_decision_if_due(&decision)?;
                    self.persist_state(None).await?;
                    return Ok(());
                }
                for intent in intents {
                    let command = self.intent_to_command(ctx, created_ts_utc, intent);
                    self.persist_state(Some(&command)).await?;
                }
            }
            TradeMode::Paper => {
                let config = self.config.clone();
                let paper = self.config.paper.clone();
                let backtest = self.config.backtest.clone();
                log_virtual_trades(
                    created_ts_utc,
                    &config,
                    &paper,
                    &backtest,
                    intents,
                    TradeMode::Paper,
                )
                .await?;
                self.persist_state(None).await?;
            }
            TradeMode::Backtest => {
                let config = self.config.clone();
                let paper = self.config.paper.clone();
                let backtest = self.config.backtest.clone();
                log_virtual_trades(
                    created_ts_utc,
                    &config,
                    &paper,
                    &backtest,
                    intents,
                    TradeMode::Backtest,
                )
                .await?;
                self.persist_state(None).await?;
            }
        }
        Ok(())
    }

    fn intent_to_command(
        &self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        intent: Intent,
    ) -> alor_protocol::OrderCommand {
        let (action, seq, action_name) = match intent {
            Intent::Place { price, qty, side } => (
                alor_protocol::CommandAction::Place(alor_protocol::PlaceOrder { price, qty, side }),
                0,
                "place",
            ),
            Intent::Market {
                qty,
                side,
                fill_price: _,
            } => (
                alor_protocol::CommandAction::Market(alor_protocol::MarketOrder { qty, side }),
                if side == alor_protocol::Side::Buy { 3 } else { 4 },
                "market",
            ),
            Intent::Cancel { order_id } => (
                alor_protocol::CommandAction::Cancel(alor_protocol::CancelOrder { order_id }),
                1,
                "cancel",
            ),
            Intent::Replace {
                order_id,
                new_price,
                new_qty,
            } => (
                alor_protocol::CommandAction::Replace(alor_protocol::ReplaceOrder {
                    order_id,
                    new_price,
                    new_qty,
                }),
                2,
                "replace",
            ),
        };
        let request_id = crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ctx.symbol,
            action_name,
            created_ts_utc,
            seq,
        );
        alor_protocol::OrderCommand {
            request_id,
            created_ts_utc,
            strategy_id: ctx.strategy_id.clone(),
            portfolio: ctx.portfolio.clone(),
            exchange: ctx.exchange.clone(),
            symbol: ctx.symbol.clone(),
            action,
            ttl_ms: None,
        }
    }

    async fn log_metrics_if_due(&mut self) -> Result<()> {
        let now = Instant::now();
        let log_due = match self.metrics.last_log {
            Some(last) => now.duration_since(last) >= Duration::from_secs(60),
            None => true,
        };
        if !log_due {
            return Ok(());
        }
        self.metrics.last_log = Some(now);
        debug!(
            bars_read_total = self.metrics.bars_read_total,
            bars_decoded_ok_total = self.metrics.bars_decoded_ok_total,
            bars_decode_failed_total = self.metrics.bars_decode_failed_total,
            bars_acked_total = self.metrics.bars_acked_total,
            bars_last_seen_close_time_utc = self.metrics.bars_last_seen_close_time_utc,
            redis_empty_polls_total = self.metrics.redis_empty_polls_total,
            redis_read_errors_total = self.metrics.redis_read_errors_total,
            commands_sent_total = self.metrics.commands_sent_total,
            publish_failures_total = self.metrics.publish_failures_total,
            "runtime bars metrics"
        );
        if self.metrics.bars_read_total == 0 {
            let xlen = self
                .transport
                .xlen(&self.config.streams.bars)
                .await
                .unwrap_or(0);
            self.metrics.bars_stream_xlen_last = Some(xlen.max(0) as u64);
            let elapsed = now.duration_since(self.metrics.start_time);
            match bars_stream_diagnostic(elapsed, xlen) {
                BarsStreamDiagnostic::Empty => {
                    tracing::debug!(
                        bars_stream = self.config.streams.bars,
                        "bars stream is empty"
                    );
                }
                BarsStreamDiagnostic::WaitingInfo => {
                    if !self.metrics.waiting_for_first_bar_info_logged {
                        info!(
                            bars_stream = self.config.streams.bars,
                            consumer_group = self.config.consumer_group,
                            consumer_name = self.config.consumer_name,
                            xlen,
                            "waiting_for_next_bar_after_restart: bars stream has data; runtime reads only new entries (\">\")"
                        );
                        self.metrics.waiting_for_first_bar_info_logged = true;
                    } else {
                        tracing::debug!(
                            bars_stream = self.config.streams.bars,
                            "still waiting for next bar"
                        );
                    }
                }
                BarsStreamDiagnostic::WaitingDebug => {
                    tracing::debug!(
                        bars_stream = self.config.streams.bars,
                        xlen,
                        "bars stream has data but no bars read yet; grace period active"
                    );
                }
                BarsStreamDiagnostic::StalledWarn => {
                    warn!(
                        bars_stream = self.config.streams.bars,
                        consumer_group = self.config.consumer_group,
                        consumer_name = self.config.consumer_name,
                        xlen,
                        "bars stream has data but runtime reads none; check group/consumer and start id"
                    );
                }
            }
        }
        Ok(())
    }

    async fn refresh_health_if_due(&mut self) -> Result<()> {
        let stream = match &self.config.streams.health {
            Some(stream) => stream,
            None => return Ok(()),
        };
        let now = Instant::now();
        if let Some(last) = self.metrics.last_health_poll {
            if now.duration_since(last) < HEALTH_POLL_INTERVAL {
                return Ok(());
            }
        }
        self.metrics.last_health_poll = Some(now);
        if let Some(payload) = self.transport.xrevrange_last(stream).await? {
            match serde_json::from_str::<Envelope<HealthEvent>>(&payload) {
                Ok(envelope) => {
                    self.live_guard.update_health(envelope.payload);
                }
                Err(error) => {
                    warn!(?error, stream, "failed to decode health event");
                }
            }
        }
        Ok(())
    }

    async fn log_live_guard_status_if_due(&mut self) -> Result<()> {
        let decision = self.evaluate_guard_decision();
        self.log_guard_decision_if_due(&decision)
    }

    fn evaluate_guard_decision(&self) -> LiveGuardDecision {
        let mut decision = evaluate_live_guard(
            self.config.trade_mode,
            self.config.allow_live_orders,
            &self.live_guard,
            self.metrics.bars_read_total > 0,
            self.metrics.bars_stream_xlen_last.unwrap_or(0) > 0,
        );
        decision.reasons.extend(self.bootstrap_state.reasons());
        decision.allowed = decision.reasons.is_empty();
        decision
    }

    fn log_guard_decision_if_due(&mut self, decision: &LiveGuardDecision) -> Result<()> {
        let now = Instant::now();
        let interval = Duration::from_millis(self.config.guard_log_interval_ms);
        let log_due = match self.metrics.last_guard_log {
            Some(last) => now.duration_since(last) >= interval,
            None => true,
        };
        if !log_due {
            return Ok(());
        }
        self.metrics.last_guard_log = Some(now);
        if decision.allowed {
            info!("live_guard=ALLOWED");
        } else {
            info!(reasons = ?decision.reasons, "live_guard=BLOCKED");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarsStreamDiagnostic {
    Empty,
    WaitingInfo,
    WaitingDebug,
    StalledWarn,
}

fn bars_stream_diagnostic(elapsed: Duration, xlen: i64) -> BarsStreamDiagnostic {
    if xlen <= 0 {
        return BarsStreamDiagnostic::Empty;
    }
    if elapsed < BARS_STREAM_INFO_GRACE {
        return BarsStreamDiagnostic::WaitingInfo;
    }
    if elapsed < BARS_STREAM_WARN_GRACE {
        return BarsStreamDiagnostic::WaitingDebug;
    }
    BarsStreamDiagnostic::StalledWarn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grace_period_diagnostics() {
        assert_eq!(
            bars_stream_diagnostic(Duration::from_secs(5), 10),
            BarsStreamDiagnostic::WaitingInfo
        );
        assert_eq!(
            bars_stream_diagnostic(Duration::from_secs(40), 10),
            BarsStreamDiagnostic::WaitingDebug
        );
        assert_eq!(
            bars_stream_diagnostic(Duration::from_secs(200), 10),
            BarsStreamDiagnostic::StalledWarn
        );
        assert_eq!(
            bars_stream_diagnostic(Duration::from_secs(5), 0),
            BarsStreamDiagnostic::Empty
        );
    }

    #[test]
    fn bootstrap_ready_requires_snapshots_and_live_bar() {
        let state = BootstrapState::default();
        assert!(!state.ready());
        assert!(state
            .reasons()
            .iter()
            .any(|reason| reason == "bootstrap:not_ready"));
    }

    #[test]
    fn bootstrap_ready_without_live_bar_is_false() {
        let state = BootstrapState {
            orders_snapshot_loaded: true,
            positions_snapshot_loaded: true,
            seen_live_bar: false,
        };
        assert!(!state.ready());
        assert!(state
            .reasons()
            .iter()
            .any(|reason| reason == "bootstrap:missing_live_bar"));
    }

    #[test]
    fn bootstrap_ready_with_snapshots_and_live_bar_is_true() {
        let state = BootstrapState {
            orders_snapshot_loaded: true,
            positions_snapshot_loaded: true,
            seen_live_bar: true,
        };
        assert!(state.ready());
        assert!(state.reasons().is_empty());
    }
}

#[derive(Debug, Serialize)]
struct VirtualTradeLog {
    ts_utc: i64,
    strategy_id: String,
    portfolio: String,
    symbol: String,
    action: String,
    qty: Option<f64>,
    price: Option<f64>,
    side: Option<String>,
    order_id: Option<i64>,
    new_price: Option<f64>,
    new_qty: Option<f64>,
}

impl VirtualTradeLog {
    fn from_intent(created_ts_utc: i64, config: &RuntimeConfig, intent: Intent) -> Self {
        match intent {
            Intent::Place { price, qty, side } => Self {
                ts_utc: created_ts_utc,
                strategy_id: config.strategy.strategy_id.clone(),
                portfolio: config.portfolio.clone(),
                symbol: config.strategy.symbol.clone(),
                action: "place".to_string(),
                qty: Some(qty),
                price: Some(price),
                side: Some(format!("{side:?}")),
                order_id: None,
                new_price: None,
                new_qty: None,
            },
            Intent::Market {
                qty,
                side,
                fill_price,
            } => Self {
                ts_utc: created_ts_utc,
                strategy_id: config.strategy.strategy_id.clone(),
                portfolio: config.portfolio.clone(),
                symbol: config.strategy.symbol.clone(),
                action: "market".to_string(),
                qty: Some(qty),
                price: fill_price,
                side: Some(format!("{side:?}")),
                order_id: None,
                new_price: None,
                new_qty: None,
            },
            Intent::Cancel { order_id } => Self {
                ts_utc: created_ts_utc,
                strategy_id: config.strategy.strategy_id.clone(),
                portfolio: config.portfolio.clone(),
                symbol: config.strategy.symbol.clone(),
                action: "cancel".to_string(),
                qty: None,
                price: None,
                side: None,
                order_id: Some(order_id),
                new_price: None,
                new_qty: None,
            },
            Intent::Replace {
                order_id,
                new_price,
                new_qty,
            } => Self {
                ts_utc: created_ts_utc,
                strategy_id: config.strategy.strategy_id.clone(),
                portfolio: config.portfolio.clone(),
                symbol: config.strategy.symbol.clone(),
                action: "replace".to_string(),
                qty: None,
                price: None,
                side: None,
                order_id: Some(order_id),
                new_price: Some(new_price),
                new_qty: Some(new_qty),
            },
        }
    }
}

async fn log_virtual_trades(
    created_ts_utc: i64,
    config: &RuntimeConfig,
    paper: &PaperConfig,
    backtest: &BacktestConfig,
    intents: Vec<Intent>,
    trade_mode: TradeMode,
) -> Result<()> {
    if trade_mode == TradeMode::Paper && !paper.enabled {
        return Ok(());
    }
    if trade_mode == TradeMode::Backtest && !backtest.enabled {
        return Ok(());
    }
    for intent in intents {
        let entry = VirtualTradeLog::from_intent(created_ts_utc, config, intent);
        match trade_mode {
            TradeMode::Paper => match paper.output {
                PaperOutput::Stdout => {
                    info!(trade = ?entry, "paper_trade");
                }
                PaperOutput::File => {
                    append_json_line(&paper.file_path, &entry).await?;
                }
            },
            TradeMode::Backtest => {
                append_log_line(&backtest.trade_log, &entry).await?;
            }
            TradeMode::Live => {}
        }
    }
    Ok(())
}

async fn append_json_line(path: &str, entry: &VirtualTradeLog) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let payload = serde_json::to_string(entry)?;
    file.write_all(payload.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

async fn append_log_line(path: &str, entry: &VirtualTradeLog) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = format!(
        "{} {} {} action={} qty={:?} price={:?} side={:?} order_id={:?} new_price={:?} new_qty={:?}",
        entry.ts_utc,
        entry.strategy_id,
        entry.symbol,
        entry.action,
        entry.qty,
        entry.price,
        entry.side,
        entry.order_id,
        entry.new_price,
        entry.new_qty
    );
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}
