use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{
    Datelike, Duration as ChronoDuration, FixedOffset, NaiveTime, TimeZone, Utc, Weekday,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::{sleep, Instant};
use tracing::{debug, error, info, warn};

use alor_protocol::{Envelope, MessageType};
use alor_types::{MarketState, Scheduler};

use crate::health_server::{spawn_health_server, HealthCfg, RuntimeSharedState};
use crate::live_guard::{evaluate_live_guard, HealthEvent, LiveGuardDecision, LiveGuardState};
use crate::redis_transport::{RedisRuntimeTransport, RuntimeMessage};
use crate::risk_gate_store::{
    append_risk_gate_runtime_session, run_risk_gate_startup_store,
    startup_store_config_from_strategy_config,
};
use crate::state::{RuntimeState, StrategyState, StrategyStateEnvelopeCompat};
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
    RiskGateRuntimeState, RuntimeStateRestored, StopOrderEvent, Strategy, StrategyCtx, TradeEvent,
};
use crate::strategy_registry::{StrategyCapabilities, StrategyRegistry};
use crate::trade_ledger::{OrderRecord, TradeLedger, TradeRecord};
use crate::{
    BacktestConfig, PaperConfig, PaperExecutionMode, PaperOutput, RuntimeConfig,
    RuntimeHealthSnapshot, TradeMode,
};

const MAX_PENDING_LOOPS: usize = 10;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(2);
const BARS_STREAM_INFO_GRACE: Duration = Duration::from_secs(30);
const BARS_STREAM_WARN_GRACE: Duration = Duration::from_secs(120);
const SNAPSHOT_SCAN_COUNT: usize = 200;
const WARMUP_BAR_SCAN_COUNT: usize = 5000;
const TRADE_DEDUP_LIMIT: usize = 512;
const STOP_END_BUFFER_SEC_DEFAULT: i64 = 60;
const NON_WORKING_ORDER_STATUSES: [&str; 5] =
    ["filled", "canceled", "cancelled", "expired", "rejected"];
const NON_WORKING_STOP_ORDER_STATUSES: [&str; 9] = [
    "canceled",
    "cancelled",
    "rejected",
    "expired",
    "filled",
    "executed",
    "triggered",
    "done",
    "completed",
];

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeStateSnapshot {
    pub ts_utc: i64,
    pub last_processed_bar_ts: std::collections::HashMap<String, i64>,
    pub strategy_state: StrategyStateEnvelopeCompat,
    #[serde(default)]
    pub last_trade_ts: Option<i64>,
    #[serde(default)]
    pub last_trade_id: Option<String>,
    #[serde(default)]
    pub seen_trade_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OrdersSnapshot {
    pub orders: HashMap<i64, OrderEvent>,
}

#[derive(Debug, Deserialize)]
struct StopOrdersSnapshot {
    pub stop_orders: HashMap<String, StopOrderEvent>,
}

#[derive(Debug, Deserialize)]
struct PositionsSnapshot {
    pub positions: HashMap<String, PositionEvent>,
}

#[derive(Debug, Deserialize)]
struct ReplayCsvBarRow {
    time: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Deserialize)]
struct ReplayReferenceTradeRow {
    entry_time: String,
    exit_time: String,
    direction: String,
    size: i64,
    entry_price: f64,
    exit_price: f64,
    reason: String,
    pnl: f64,
}

#[derive(Debug, Serialize)]
struct ReplayParityReport {
    status: String,
    tolerance: f64,
    runtime_trades: usize,
    reference_trades: usize,
    matched_trades: usize,
    first_divergence: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct PositionDump {
    symbol: String,
    qty: f64,
    existing: bool,
    avg_price: f64,
    ts_utc: i64,
}

#[derive(Debug)]
#[allow(dead_code)]
struct OrderDump {
    order_id: i64,
    status: String,
    side: String,
    price: f64,
    qty: f64,
    filled: f64,
    existing: bool,
    request_id: Option<uuid::Uuid>,
    comment: Option<String>,
    ts_utc: i64,
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
    strategy_capabilities: StrategyCapabilities,
    our_request_ids: HashSet<uuid::Uuid>,
    our_order_ids: HashSet<i64>,
    bootstrap_snapshot: Option<BootstrapSnapshot>,
    pending_trades_by_order_id: HashMap<i64, Vec<TradeEvent>>,
    pending_exec: HashMap<i64, PendingExecution>,
    sim_orders: Vec<SimOrder>,
    next_sim_order_id: i64,
    strategy_now_ts_utc: i64,
    health_snapshot: RuntimeSharedState,
    health_server_handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct PendingExecution {
    order_id: i64,
    symbol: String,
    side: String,
    target_qty: f64,
    filled_qty: f64,
    order_price: f64,
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
    last_live_guard_log_ts_utc: i64,
    last_live_guard: Option<GuardSnapshot>,
    last_waiting_next_bar_active: bool,
    last_health_poll: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuardSnapshot {
    status: &'static str,
    reasons: Vec<String>,
}

#[derive(Debug, Clone)]
struct SimOrder {
    order_id: i64,
    symbol: String,
    side: String,
    intent_class: Option<alor_protocol::IntentClass>,
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

#[derive(Debug, Clone, Copy)]
enum StrategyLifecycleHook {
    BootstrapSnapshot,
    RuntimeStateRestore,
    HistoryWarmup,
    StopOrder,
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
            last_live_guard_log_ts_utc: 0,
            last_live_guard: None,
            last_waiting_next_bar_active: false,
            last_health_poll: None,
        }
    }
}

impl StrategyRuntime {
    fn lifecycle_hook_enabled(&self, hook: StrategyLifecycleHook) -> bool {
        match hook {
            StrategyLifecycleHook::BootstrapSnapshot => {
                self.strategy_capabilities.uses_bootstrap_snapshot
            }
            StrategyLifecycleHook::RuntimeStateRestore => {
                self.strategy_capabilities.uses_runtime_state_restore
            }
            StrategyLifecycleHook::HistoryWarmup => self.strategy_capabilities.uses_history_warmup,
            StrategyLifecycleHook::StopOrder => self.strategy_capabilities.uses_stop_orders,
        }
    }

    fn invoke_strategy_callback<F>(
        &mut self,
        ctx: &StrategyCtx,
        callback_name: &'static str,
        callback: F,
    ) -> (Vec<Intent>, StrategyState)
    where
        F: FnOnce(&mut (dyn Strategy + Send + Sync), &StrategyCtx) -> Vec<Intent>,
    {
        let previous_strategy_state = self.state.strategy_state.clone();
        let intents = callback(self.strategy.as_mut(), ctx);
        self.state.strategy_state = self.strategy.state().clone();
        self.audit_event(
            if intents.is_empty() {
                "signal_not_generated"
            } else {
                "signal_generated"
            },
            json!({
                "callback": callback_name,
                "event_ts_utc": ctx.event_ts_utc(),
                "intents_count": intents.len(),
            }),
        );
        (intents, previous_strategy_state)
    }

    async fn invoke_and_apply_strategy_callback<F>(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        callback_name: &'static str,
        callback: F,
    ) -> Result<usize>
    where
        F: FnOnce(&mut (dyn Strategy + Send + Sync), &StrategyCtx) -> Vec<Intent>,
    {
        let (intents, previous_strategy_state) =
            self.invoke_strategy_callback(ctx, callback_name, callback);
        let intents_count = intents.len();
        self.apply_intents(ctx, created_ts_utc, intents, previous_strategy_state)
            .await?;
        Ok(intents_count)
    }

    fn audit_event(&self, event: &'static str, details: serde_json::Value) {
        info!(
            target: "strategy_runtime::audit",
            strategy_id = %self.config.strategy.strategy_id,
            strategy_kind = ?self.config.strategy.strategy_kind,
            symbol = %self.config.strategy.symbol,
            event,
            details = %details,
            "strategy_audit"
        );
    }

    fn can_advance_paper_execution(&self, origin: DataOrigin) -> bool {
        if self.config.trade_mode != TradeMode::Paper {
            return true;
        }
        match self.config.paper.execution_mode {
            PaperExecutionMode::HistorySim => true,
            PaperExecutionMode::LiveOnly => origin == DataOrigin::Live,
        }
    }

    async fn record_non_live_intents(
        &mut self,
        created_ts_utc: i64,
        intents: &[Intent],
        mode: TradeMode,
    ) -> Result<()> {
        if intents.is_empty() {
            return Ok(());
        }
        self.health_snapshot.write().last_intent_ts_utc = Some(created_ts_utc);
        let config = self.config.clone();
        let paper = self.config.paper.clone();
        let backtest = self.config.backtest.clone();
        log_virtual_trades(
            created_ts_utc,
            &config,
            &paper,
            &backtest,
            intents.to_vec(),
            mode,
        )
        .await
    }

    fn test_delay_before_publish_ms() -> u64 {
        let requested = std::env::var("RUNTIME_TEST_DELAY_BEFORE_PUBLISH_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);
        if requested == 0 {
            return 0;
        }
        let hooks_enabled = std::env::var("RUNTIME_ENABLE_TEST_HOOKS")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if !hooks_enabled {
            warn!(
                requested_delay_ms = requested,
                "test_delay_before_publish_ignored_hooks_disabled"
            );
            return 0;
        }
        requested
    }

    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        let transport = RedisRuntimeTransport::new(config.clone())?;
        let registry = StrategyRegistry::builtin()?;
        let descriptor = registry
            .descriptor(config.strategy.strategy_kind)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no strategy descriptor registered for kind {:?}",
                    config.strategy.strategy_kind
                )
            })?;
        let strategy = registry.create(&config.strategy)?;
        let now = chrono::Utc::now().timestamp();
        let health_snapshot = Arc::new(parking_lot::RwLock::new(RuntimeHealthSnapshot {
            uptime_start: std::time::Instant::now(),
            runtime_phase: "SyncingHistory".to_string(),
            live_guard_status: "BLOCKED".to_string(),
            live_guard_reasons: vec!["bootstrap:not_ready".to_string()],
            live_guard_last_change_ts_utc: now,
            gateway_health_last_ts_utc: None,
            gateway_health_age_sec: None,
            gateway_ready: None,
            ws_connected: None,
            cws_authorized: None,
            gateway_scheduler_state: None,
            scheduler_state: "Unconfigured".to_string(),
            now_local: "unknown".to_string(),
            scheduler_note: Some("trading_periods missing".to_string()),
            timezone_offset_hours: config.strategy.timezone_offset_hours,
            last_bar_ts_utc: None,
            last_ack_ts_utc: None,
            last_intent_ts_utc: None,
            orders_mode: runtime_orders_mode(&config).to_string(),
            allow_live_orders: config.allow_live_orders,
            allow_paper_orders: config.allow_paper_orders,
            require_gateway_ready: config.require_gateway_ready,
            exit_recovery_active: false,
            close_only_degraded: false,
            operator_intervention_required: false,
            open_risk_position_unflattened: false,
            readiness: false,
        }));

        Ok(Self {
            config,
            transport,
            state: RuntimeState::default(),
            strategy,
            live_guard: LiveGuardState::default(),
            bootstrap_state: BootstrapState::default(),
            metrics: RuntimeMetrics::default(),
            ledger: TradeLedger::default(),
            strategy_capabilities: descriptor.capabilities,
            our_request_ids: HashSet::new(),
            our_order_ids: HashSet::new(),
            bootstrap_snapshot: None,
            pending_trades_by_order_id: HashMap::new(),
            pending_exec: HashMap::new(),
            sim_orders: Vec::new(),
            next_sim_order_id: 1,
            strategy_now_ts_utc: 0,
            health_snapshot,
            health_server_handle: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        self.run_until_shutdown(shutdown_rx).await
    }

    pub async fn run_until_shutdown(
        &mut self,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> Result<()> {
        if self.config.replay.enabled {
            return self.run_replay().await;
        }

        self.bootstrap().await?;
        self.start_health_server();
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        self.mark_shutting_down();
                        break;
                    }
                }
                result = self.poll_once() => {
                    result?;
                    self.refresh_health_snapshot();
                }
            }

            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        self.mark_shutting_down();
                        break;
                    }
                }
                _ = sleep(Duration::from_millis(self.config.read.poll_interval_ms)) => {}
            }
        }

        Ok(())
    }

    fn mark_shutting_down(&self) {
        let mut guard = self.health_snapshot.write();
        guard.readiness = false;
        guard.runtime_phase = "ShuttingDown".to_string();
    }

    fn start_health_server(&mut self) {
        let cfg = HealthCfg {
            enabled: self.config.health.enabled,
            listen_addr: self.config.health.listen_addr.clone(),
            expose_metrics: self.config.health.expose_metrics,
        };
        if !cfg.enabled {
            tracing::info!("health server disabled");
            return;
        }
        let shared = Arc::clone(&self.health_snapshot);
        self.health_server_handle = Some(tokio::spawn(async move {
            if let Err(error) = spawn_health_server(shared, cfg).await {
                tracing::error!(?error, "health server terminated with error");
            }
        }));
    }

    fn refresh_health_snapshot(&self) {
        let decision = self.evaluate_guard_decision();
        let now_ts = chrono::Utc::now().timestamp();
        let mut runtime_phase = self
            .live_guard
            .health
            .as_ref()
            .map(|h| format!("{:?}", h.gateway_phase))
            .unwrap_or_else(|| "SyncingHistory".to_string());
        let mut scheduler_state = "Unconfigured".to_string();
        let mut now_local = "unknown".to_string();
        let mut scheduler_note = Some("trading_periods missing".to_string());
        if let Some(periods) = &self.config.strategy.trading_periods {
            scheduler_note = None;
            let scheduler = Scheduler::new_with_fallback_offset_hours(
                periods.clone(),
                self.config.strategy.timezone_offset_hours,
            );
            if let Some(local) = scheduler.local_datetime_utc(now_ts) {
                now_local = local.to_rfc3339();
            }
            scheduler_state = format!("{:?}", scheduler.market_state_utc(now_ts));
        }

        let gateway_health_last_ts_utc = self.live_guard.health.as_ref().map(|h| h.last_event_ts);
        let gateway_health_age_sec = gateway_health_last_ts_utc.map(|ts| now_ts.saturating_sub(ts));
        let has_open_position = self
            .state
            .positions
            .get(&self.config.strategy.symbol)
            .map(|pos| pos.qty.abs() > f64::EPSILON)
            .unwrap_or(false);
        let strategy_risk = self.strategy.exit_risk_status(has_open_position);
        if let Some(override_phase) = strategy_risk.phase_override.clone() {
            runtime_phase = override_phase;
        }
        let close_only_degraded = runtime_phase == "CloseOnlyDegraded";
        let mut live_guard_reasons = decision.reasons;
        if strategy_risk.exit_recovery_active {
            live_guard_reasons.push("strategy:exit_recovery_active".to_string());
        }
        if strategy_risk.operator_intervention_required {
            live_guard_reasons.push("strategy:operator_intervention_required".to_string());
        }
        if strategy_risk.open_risk_position_unflattened {
            live_guard_reasons.push("strategy:open_risk_position_unflattened".to_string());
        }
        let readiness = decision.allowed
            && !strategy_risk.exit_recovery_active
            && !strategy_risk.operator_intervention_required;

        let mut guard = self.health_snapshot.write();
        guard.runtime_phase = runtime_phase;
        guard.live_guard_status = if decision.allowed {
            "ALLOWED"
        } else {
            "BLOCKED"
        }
        .to_string();
        guard.live_guard_reasons = live_guard_reasons;
        guard.readiness = readiness;
        guard.live_guard_last_change_ts_utc = now_ts;
        guard.gateway_health_last_ts_utc = gateway_health_last_ts_utc;
        guard.gateway_health_age_sec = gateway_health_age_sec;
        guard.gateway_ready = self.live_guard.health.as_ref().map(|h| h.readiness);
        guard.ws_connected = self.live_guard.health.as_ref().map(|h| h.ws_connected);
        guard.cws_authorized = self.live_guard.health.as_ref().map(|h| h.cws_authorized);
        guard.gateway_scheduler_state = self
            .live_guard
            .health
            .as_ref()
            .and_then(|h| h.scheduler_state.clone());
        guard.scheduler_state = scheduler_state;
        guard.now_local = now_local;
        guard.scheduler_note = scheduler_note;
        guard.timezone_offset_hours = self.config.strategy.timezone_offset_hours;
        guard.last_bar_ts_utc = self.metrics.bars_last_seen_close_time_utc;
        guard.exit_recovery_active = strategy_risk.exit_recovery_active;
        guard.close_only_degraded = close_only_degraded;
        guard.operator_intervention_required = strategy_risk.operator_intervention_required;
        guard.open_risk_position_unflattened = strategy_risk.open_risk_position_unflattened;
    }

    pub async fn flush_reports(&self) -> Result<()> {
        self.persist_ledger_reports().await
    }

    async fn run_replay(&mut self) -> Result<()> {
        self.prepare_artifacts()?;
        let bars_csv_path = self
            .config
            .replay
            .bars_csv_path
            .as_deref()
            .context("replay.enabled=true requires replay.bars_csv_path")?;

        let bars = Self::load_replay_bars(
            bars_csv_path,
            &self.config.strategy.symbol,
            self.config.replay.strict_dedup,
        )?;

        for bar in bars {
            self.handle_replay_bar(bar).await?;
        }

        self.persist_ledger_reports().await?;
        self.persist_replay_parity_report()?;
        Ok(())
    }

    fn load_replay_bars(path: &str, symbol: &str, strict_dedup: bool) -> Result<Vec<BarEvent>> {
        let mut rdr = csv::Reader::from_path(path)
            .with_context(|| format!("failed to open replay bars csv at {path}"))?;
        let mut bars = Vec::new();
        let mut previous_ts: Option<i64> = None;

        for (idx, row) in rdr.deserialize::<ReplayCsvBarRow>().enumerate() {
            let row_number = idx + 2;
            let raw = row.with_context(|| format!("invalid replay csv row #{row_number}"))?;
            let ts_utc = chrono::DateTime::parse_from_rfc3339(&raw.time)
                .with_context(|| {
                    format!(
                        "invalid replay RFC3339 time at row #{row_number}: {}",
                        raw.time
                    )
                })?
                .timestamp();

            if let Some(prev) = previous_ts {
                if ts_utc < prev {
                    anyhow::bail!(
                        "replay bars are not ascending at row #{row_number}: {ts_utc} < {prev}"
                    );
                }
                if ts_utc == prev {
                    if strict_dedup {
                        anyhow::bail!(
                            "replay bars contain duplicate timestamp at row #{row_number}: {ts_utc}"
                        );
                    }
                    continue;
                }
            }
            previous_ts = Some(ts_utc);

            bars.push(BarEvent {
                symbol: symbol.to_string(),
                close_time_utc: ts_utc,
                close: raw.close,
                o: raw.open,
                h: raw.high,
                l: raw.low,
                v: 0.0,
                origin: DataOrigin::Replay,
            });
        }

        Ok(bars)
    }

    async fn handle_replay_bar(&mut self, bar: BarEvent) -> Result<()> {
        if self.state.is_duplicate_bar(&bar.symbol, bar.close_time_utc) {
            return Ok(());
        }

        let prev_bar_ts = self.state.last_processed_bar_ts.get(&bar.symbol).copied();
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(prev_bar_ts, bar.close_time_utc);
        let (intents, previous_strategy_state) =
            self.invoke_strategy_callback(&ctx, "on_bar_replay", |strategy, strategy_ctx| {
                strategy.on_bar(strategy_ctx, &bar)
            });
        self.metrics.bars_last_seen_close_time_utc = Some(bar.close_time_utc);

        if self.config.trade_mode != TradeMode::Live {
            self.record_non_live_intents(bar.close_time_utc, &intents, self.config.trade_mode)
                .await?;
            if self.can_advance_paper_execution(bar.origin.clone()) {
                self.simulate_fills(&bar).await?;
                self.simulate_intents(&bar, intents).await?;
            }
        } else {
            self.apply_intents(&ctx, bar.close_time_utc, intents, previous_strategy_state)
                .await?;
        }
        self.state
            .update_last_bar_ts(&bar.symbol, bar.close_time_utc);

        self.metrics.bars_read_total = self.metrics.bars_read_total.saturating_add(1);
        self.metrics.bars_decoded_ok_total = self.metrics.bars_decoded_ok_total.saturating_add(1);
        self.metrics.bars_acked_total = self.metrics.bars_acked_total.saturating_add(1);

        Ok(())
    }

    async fn bootstrap(&mut self) -> Result<()> {
        self.prepare_artifacts()?;
        self.transport
            .ensure_groups(&[
                &self.config.streams.bars,
                &self.config.streams.orders,
                &self.config.streams.trades,
                &self.config.streams.positions,
                &self.config.streams.acks,
            ])
            .await?;

        self.load_snapshots().await?;
        self.load_runtime_state().await?;
        self.bootstrap_risk_gate_startup_store().await?;
        self.flush_risk_gate_session_finalizations().await?;
        self.notify_bootstrap_snapshot().await?;
        self.notify_runtime_state_restored().await?;
        self.warmup_strategy_indicators_from_history().await?;

        let streams = self.config.streams.clone();
        let trim_acks = self.config.trim.acks;
        let trim_orders = self.config.trim.orders;
        let trim_trades = self.config.trim.trades;
        let trim_positions = self.config.trim.positions;
        let trim_bars = self.config.trim.bars;

        self.audit_event(
            "pending_recovery_started",
            json!({"stream": streams.acks.clone(), "message_type": "CommandAck"}),
        );
        self.recover_pending(&streams.acks, MessageType::CommandAck, trim_acks)
            .await?;
        self.audit_event(
            "pending_recovery_finished",
            json!({"stream": streams.acks.clone(), "message_type": "CommandAck"}),
        );

        self.audit_event(
            "pending_recovery_started",
            json!({"stream": streams.orders.clone(), "message_type": "Order/StopOrder"}),
        );
        self.recover_pending_orders_stream(&streams.orders, trim_orders)
            .await?;
        self.audit_event(
            "pending_recovery_finished",
            json!({"stream": streams.orders.clone(), "message_type": "Order/StopOrder"}),
        );

        self.audit_event(
            "pending_recovery_started",
            json!({"stream": streams.trades.clone(), "message_type": "Trade"}),
        );
        self.recover_pending(&streams.trades, MessageType::Trade, trim_trades)
            .await?;
        self.audit_event(
            "pending_recovery_finished",
            json!({"stream": streams.trades.clone(), "message_type": "Trade"}),
        );

        self.audit_event(
            "pending_recovery_started",
            json!({"stream": streams.positions.clone(), "message_type": "Position"}),
        );
        self.recover_pending(&streams.positions, MessageType::Position, trim_positions)
            .await?;
        self.audit_event(
            "pending_recovery_finished",
            json!({"stream": streams.positions.clone(), "message_type": "Position"}),
        );

        self.audit_event(
            "pending_recovery_started",
            json!({"stream": streams.bars.clone(), "message_type": "Bar"}),
        );
        self.recover_pending(&streams.bars, MessageType::Bar, trim_bars)
            .await?;
        self.audit_event(
            "pending_recovery_finished",
            json!({"stream": streams.bars.clone(), "message_type": "Bar"}),
        );

        self.refresh_health_if_due().await?;
        self.log_live_guard_status_if_due().await?;

        Ok(())
    }

    async fn bootstrap_risk_gate_startup_store(&mut self) -> Result<()> {
        let finalized_at_utc = Utc::now().timestamp();
        let Some(config) =
            startup_store_config_from_strategy_config(&self.config.strategy, finalized_at_utc)?
        else {
            return Ok(());
        };

        info!(
            strategy = %self.config.strategy.strategy_id,
            profile = %config.identity.profile_id,
            mode = ?config.mode,
            "risk gate startup store bootstrap started"
        );
        let result = run_risk_gate_startup_store(&self.transport, &config).await?;
        self.apply_risk_gate_materialized_state(
            config.identity.profile_id.clone(),
            &result.artifacts.materialized_state,
        );
        self.audit_event(
            "risk_gate_startup_bootstrap",
            json!({
                "profile_id": config.identity.profile_id,
                "mode": format!("{:?}", config.mode),
                "decision": format!("{:?}", result.artifacts.decision),
                "existing_records_loaded": result.existing_records_loaded,
                "records_attempted": result.write_summary.attempted_records,
                "records_inserted": result.write_summary.inserted_records,
                "records_duplicate": result.write_summary.duplicate_records,
                "state_refreshed": result.write_summary.state_refreshed,
                "ledger_rows_count": result.artifacts.materialized_state.ledger_rows_count,
                "seed_loaded": result.artifacts.materialized_state.seed_loaded,
            }),
        );
        info!(
            strategy = %self.config.strategy.strategy_id,
            profile = %config.identity.profile_id,
            mode = ?config.mode,
            decision = ?result.artifacts.decision,
            existing_records_loaded = result.existing_records_loaded,
            records_attempted = result.write_summary.attempted_records,
            records_inserted = result.write_summary.inserted_records,
            records_duplicate = result.write_summary.duplicate_records,
            state_refreshed = result.write_summary.state_refreshed,
            "risk gate startup store bootstrap finished"
        );
        Ok(())
    }

    fn apply_risk_gate_materialized_state(
        &mut self,
        profile_id: String,
        state: &crate::strategies::hybrid_intraday::RiskGateMaterializedState,
    ) {
        let runtime_state = RiskGateRuntimeState {
            profile_id,
            last_finalized_session_date: state.last_finalized_session_date,
            rolling_sum_lb120: state.rolling_sum_lb120,
            mr_enabled_current_session: state.mr_enabled_current_session,
            mr_enabled_next_session: state.mr_enabled_next_session,
            ledger_rows_count: state.ledger_rows_count,
        };
        self.strategy.on_risk_gate_state(&runtime_state);
        self.state.strategy_state = self.strategy.state().clone();
    }

    async fn flush_risk_gate_session_finalizations(&mut self) -> Result<()> {
        let finalizations = self.strategy.risk_gate_session_finalizations();
        if finalizations.is_empty() {
            return Ok(());
        }

        if self.config.trade_mode != TradeMode::Live {
            let session_dates = finalizations
                .iter()
                .map(|finalization| finalization.session_date)
                .collect::<Vec<_>>();
            self.strategy
                .acknowledge_risk_gate_session_finalizations(&session_dates);
            self.state.strategy_state = self.strategy.state().clone();
            return Ok(());
        }

        let finalized_at_utc = Utc::now().timestamp();
        let Some(config) =
            startup_store_config_from_strategy_config(&self.config.strategy, finalized_at_utc)?
        else {
            return Ok(());
        };

        let mut acknowledged_dates = Vec::new();
        for finalization in &finalizations {
            let append_result = append_risk_gate_runtime_session(
                &self.transport,
                &config.identity,
                finalization,
                finalized_at_utc,
                config.ledger_scan_count,
            )
            .await?;
            if let Some(materialized_state) = append_result.materialized_state.as_ref() {
                self.apply_risk_gate_materialized_state(
                    config.identity.profile_id.clone(),
                    materialized_state,
                );
            }
            let summary = append_result.write_summary;
            self.audit_event(
                "risk_gate_runtime_session_finalized",
                json!({
                    "profile_id": config.identity.profile_id,
                    "session_date": finalization.session_date.format("%Y-%m-%d").to_string(),
                    "shadow_pnl_points": finalization.shadow_pnl_points,
                    "shadow_trade_count": finalization.shadow_trade_count,
                    "records_attempted": summary.attempted_records,
                    "records_inserted": summary.inserted_records,
                    "records_duplicate": summary.duplicate_records,
                    "state_refreshed": summary.state_refreshed,
                }),
            );
            info!(
                strategy = %self.config.strategy.strategy_id,
                profile = %config.identity.profile_id,
                session_date = %finalization.session_date,
                shadow_pnl_points = finalization.shadow_pnl_points,
                shadow_trade_count = finalization.shadow_trade_count,
                inserted_records = summary.inserted_records,
                duplicate_records = summary.duplicate_records,
                state_refreshed = summary.state_refreshed,
                "risk gate runtime session finalized"
            );
            acknowledged_dates.push(finalization.session_date);
        }

        self.strategy
            .acknowledge_risk_gate_session_finalizations(&acknowledged_dates);
        self.state.strategy_state = self.strategy.state().clone();
        self.persist_state(None).await?;
        Ok(())
    }

    async fn warmup_strategy_indicators_from_history(&mut self) -> Result<()> {
        if !self.lifecycle_hook_enabled(StrategyLifecycleHook::HistoryWarmup) {
            return Ok(());
        }
        if self.config.reset_state_on_start {
            info!(
                strategy = self.config.strategy.strategy_id,
                symbol = self.config.strategy.symbol,
                stream = self.config.streams.bars,
                scan = WARMUP_BAR_SCAN_COUNT,
                "bootstrap: warmup from history bars (reset_state_on_start=true)"
            );
        }
        let payloads = self
            .transport
            .xrevrange_last_n(&self.config.streams.bars, WARMUP_BAR_SCAN_COUNT)
            .await?;
        if payloads.is_empty() {
            return Ok(());
        }

        let mut bars = Vec::new();
        for payload in payloads {
            let envelope = match serde_json::from_str::<Envelope<serde_json::Value>>(&payload) {
                Ok(envelope) => envelope,
                Err(error) => {
                    warn!(?error, "warmup: failed to parse bar envelope");
                    continue;
                }
            };
            if envelope.msg_type != MessageType::Bar {
                continue;
            }
            match serde_json::from_value::<BarEvent>(envelope.payload) {
                Ok(bar) if bar.symbol == self.config.strategy.symbol => bars.push(bar),
                Ok(_) => {}
                Err(error) => {
                    warn!(?error, "warmup: failed to decode bar payload");
                }
            }
        }

        if bars.is_empty() {
            return Ok(());
        }
        bars.sort_by_key(|bar| bar.close_time_utc);
        bars.dedup_by_key(|bar| bar.close_time_utc);

        let mut ctx = self.strategy_ctx();
        ctx.allow_live_orders = false;
        let processed = self.strategy.warmup_from_history(&ctx, &bars);
        if processed > 0 {
            if let Some(last_bar) = bars.last() {
                self.state
                    .update_last_bar_ts(&last_bar.symbol, last_bar.close_time_utc);
            }
        }
        self.state.strategy_state = self.strategy.state().clone();
        if processed > 0 {
            info!(
                strategy = self.config.strategy.strategy_id,
                symbol = self.config.strategy.symbol,
                bars_processed = processed,
                scan = WARMUP_BAR_SCAN_COUNT,
                "bootstrap: strategy warmup from history bars completed"
            );
        }
        Ok(())
    }

    fn prepare_artifacts(&self) -> Result<()> {
        match self.config.trade_mode {
            TradeMode::Paper if self.config.paper.enabled && !self.config.paper.append => {
                self.truncate_file(&self.config.paper.file_path)?;
                self.truncate_file(&self.config.paper.trades_csv)?;
                self.truncate_file(&self.config.paper.summary_json)?;
            }
            TradeMode::Backtest if self.config.backtest.enabled && !self.config.backtest.append => {
                self.truncate_file(&self.config.backtest.trade_log)?;
                self.truncate_file(&self.config.backtest.trades_csv)?;
                self.truncate_file(&self.config.backtest.summary_json)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn truncate_file(&self, path: &str) -> Result<()> {
        ensure_parent_dir(path)?;
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
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
        let mut stop_orders_snapshot: Option<StopOrdersSnapshot> = None;
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
                MessageType::SnapshotStopOrders if stop_orders_snapshot.is_none() => {
                    match serde_json::from_str::<Envelope<StopOrdersSnapshot>>(&payload) {
                        Ok(envelope) => {
                            stop_orders_snapshot = Some(envelope.payload);
                        }
                        Err(error) => {
                            warn!(?error, "failed to parse stop-orders snapshot");
                        }
                    }
                }
                _ => {}
            }
            if orders_snapshot.is_some()
                && positions_snapshot.is_some()
                && stop_orders_snapshot.is_some()
            {
                break;
            }
        }

        let strategy_symbol = self.config.strategy.symbol.clone();
        let mut positions_strategy = HashMap::new();
        let mut working_orders_strategy = HashMap::new();
        let mut working_stop_orders_strategy = HashMap::new();
        let mut snapshot_ts_utc = None;
        let mut positions_total_all = 0usize;
        let mut positions_open_all = 0usize;
        let mut positions_total_strategy = 0usize;
        let mut positions_open_strategy = 0usize;
        let mut orders_total_all = 0usize;
        let mut orders_open_all = 0usize;
        let mut orders_total_strategy = 0usize;
        let mut orders_open_strategy = 0usize;
        let mut stop_orders_total_all = 0usize;
        let mut stop_orders_open_all = 0usize;
        let mut stop_orders_total_strategy = 0usize;
        let mut stop_orders_open_strategy = 0usize;

        if let Some(snapshot) = orders_snapshot {
            let mut strategy_orders = HashMap::new();
            for (order_id, order) in snapshot.orders {
                orders_total_all += 1;
                if self.is_working_order(&order) {
                    orders_open_all += 1;
                }
                if order.symbol == strategy_symbol {
                    if self.is_working_order(&order) {
                        orders_open_strategy += 1;
                        working_orders_strategy.insert(order_id, order.clone());
                        self.our_order_ids.insert(order_id);
                    }
                    orders_total_strategy += 1;
                    if order.ts_utc > 0 {
                        snapshot_ts_utc = Some(
                            snapshot_ts_utc.map_or(order.ts_utc, |ts: i64| ts.max(order.ts_utc)),
                        );
                    }
                    strategy_orders.insert(order_id, order);
                }
            }
            self.state.orders = strategy_orders;
            self.bootstrap_state.orders_snapshot_loaded = true;
        }
        if let Some(snapshot) = stop_orders_snapshot {
            let mut strategy_stop_orders = HashMap::new();
            for (stop_order_id, stop_order) in snapshot.stop_orders {
                stop_orders_total_all += 1;
                if self.is_working_stop_order(&stop_order) {
                    stop_orders_open_all += 1;
                }
                if stop_order.symbol == strategy_symbol {
                    if self.is_working_stop_order(&stop_order) {
                        stop_orders_open_strategy += 1;
                        working_stop_orders_strategy
                            .insert(stop_order_id.clone(), stop_order.clone());
                    }
                    stop_orders_total_strategy += 1;
                    if stop_order.ts_utc > 0 {
                        snapshot_ts_utc = Some(
                            snapshot_ts_utc
                                .map_or(stop_order.ts_utc, |ts: i64| ts.max(stop_order.ts_utc)),
                        );
                    }
                    strategy_stop_orders.insert(stop_order_id, stop_order);
                }
            }
            self.state.stop_orders = strategy_stop_orders;
        }
        if let Some(snapshot) = positions_snapshot {
            let mut strategy_positions = HashMap::new();
            for (symbol, position) in snapshot.positions {
                positions_total_all += 1;
                if self.is_open_position(&position) {
                    positions_open_all += 1;
                }
                if symbol == strategy_symbol {
                    positions_total_strategy += 1;
                    if self.is_open_position(&position) {
                        positions_open_strategy += 1;
                    }
                    if position.ts_utc > 0 {
                        snapshot_ts_utc = Some(
                            snapshot_ts_utc
                                .map_or(position.ts_utc, |ts: i64| ts.max(position.ts_utc)),
                        );
                    }
                    positions_strategy.insert(symbol.clone(), position.clone());
                    strategy_positions.insert(symbol, position);
                }
            }
            self.state.positions = strategy_positions;
            self.bootstrap_state.positions_snapshot_loaded = true;
        }
        if self.bootstrap_state.orders_snapshot_loaded
            || self.bootstrap_state.positions_snapshot_loaded
        {
            info!(
                positions_total_all,
                positions_open_all,
                positions_total_strategy,
                positions_open_strategy,
                orders_total_all,
                orders_open_all,
                orders_total_strategy,
                orders_open_strategy,
                stop_orders_total_all,
                stop_orders_open_all,
                stop_orders_total_strategy,
                stop_orders_open_strategy,
                "bootstrap: snapshots filtered"
            );
            self.log_bootstrap_dump(
                &positions_strategy,
                &self.state.orders,
                positions_open_strategy,
                orders_open_strategy,
                snapshot_ts_utc,
            );
            self.bootstrap_snapshot = Some(BootstrapSnapshot {
                positions_strategy,
                working_orders_strategy,
                working_stop_orders_strategy,
                snapshot_ts_utc,
            });
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
                    if let Some(saved_kind) = snapshot.strategy_state.strategy_kind() {
                        if saved_kind != self.config.strategy.strategy_kind {
                            warn!(
                                saved_strategy_kind = ?saved_kind,
                                configured_strategy_kind = ?self.config.strategy.strategy_kind,
                                "runtime state envelope strategy kind differs from current config"
                            );
                        }
                    }
                    let strategy_state = snapshot.strategy_state.into_payload();
                    self.state.last_processed_bar_ts = snapshot.last_processed_bar_ts;
                    self.state.strategy_state = strategy_state.clone();
                    self.state.last_trade_ts = snapshot.last_trade_ts;
                    self.state.last_trade_id = snapshot.last_trade_id;
                    self.state.seen_trade_ids = snapshot.seen_trade_ids;
                    self.strategy.set_state(strategy_state);
                    self.restore_pending_requests();
                    info!("runtime state restored");
                    self.audit_event(
                        "runtime_state_snapshot_loaded",
                        json!({
                            "last_trade_ts": self.state.last_trade_ts,
                            "seen_trade_ids_count": self.state.seen_trade_ids.len(),
                            "pending_requests_count": self.our_request_ids.len(),
                        }),
                    );
                    self.log_runtime_state_dump();
                }
                Err(error) => {
                    warn!(?error, "failed to parse runtime state snapshot");
                }
            }
        }
        Ok(())
    }

    fn restore_pending_requests(&mut self) {
        self.our_request_ids
            .extend(self.strategy.pending_request_ids());
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

    async fn recover_pending_orders_stream(
        &mut self,
        stream: &str,
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
                    warn!(?error, stream, "orders pending autoclaim failed");
                    break;
                }
            };
            let (next_start, entries) = self.transport.parse_autoclaim_entries(stream, reply);
            if entries.is_empty() {
                break;
            }
            start = next_start;
            for entry in entries {
                self.decode_and_dispatch_orders_entry(stream, trim_maxlen, entry)
                    .await?;
            }
        }
        Ok(())
    }

    async fn poll_once(&mut self) -> Result<()> {
        let streams = self.config.streams.clone();
        let trim_acks = self.config.trim.acks;
        let trim_orders = self.config.trim.orders;
        let trim_trades = self.config.trim.trades;
        let trim_positions = self.config.trim.positions;
        let trim_bars = self.config.trim.bars;

        self.drain_stream(&streams.acks, MessageType::CommandAck, trim_acks, 10)
            .await?;
        self.drain_orders_stream(&streams.orders, trim_orders, 10)
            .await?;
        self.drain_stream(&streams.trades, MessageType::Trade, trim_trades, 10)
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

    async fn drain_orders_stream(
        &mut self,
        stream: &str,
        trim_maxlen: usize,
        count: usize,
    ) -> Result<()> {
        let reply = match self.transport.read_group(stream, count).await {
            Ok(reply) => reply,
            Err(error) => {
                warn!(?error, stream, "orders xreadgroup failed");
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
        for entry in entries {
            self.decode_and_dispatch_orders_entry(stream, trim_maxlen, entry)
                .await?;
        }
        Ok(())
    }

    async fn decode_and_dispatch_orders_entry(
        &mut self,
        stream: &str,
        trim_maxlen: usize,
        entry: crate::redis_transport::RedisStreamMessage,
    ) -> Result<()> {
        let message_id = entry.id;
        let payload = entry.payload;
        if payload.is_empty() {
            self.transport
                .write_dlq(
                    stream,
                    &message_id,
                    &payload,
                    "missing_payload",
                    trim_maxlen,
                )
                .await?;
            self.transport.xack(stream, &message_id).await?;
            return Ok(());
        }

        let envelope: Envelope<serde_json::Value> = match serde_json::from_str(&payload) {
            Ok(envelope) => envelope,
            Err(error) => {
                let reason = format!("parse_error: {error}");
                self.transport
                    .write_dlq(stream, &message_id, &payload, &reason, trim_maxlen)
                    .await?;
                self.transport.xack(stream, &message_id).await?;
                return Ok(());
            }
        };
        if envelope.schema_version > alor_protocol::SCHEMA_VERSION {
            self.transport
                .write_dlq(
                    stream,
                    &message_id,
                    &payload,
                    "unsupported_schema",
                    trim_maxlen,
                )
                .await?;
            self.transport.xack(stream, &message_id).await?;
            return Ok(());
        }

        let message = RuntimeMessage {
            stream: stream.to_string(),
            message_id,
            payload: envelope.payload,
        };

        match envelope.msg_type {
            MessageType::Order => self.dispatch_message(message).await?,
            MessageType::StopOrder => {
                let stop_order: StopOrderEvent = serde_json::from_value(message.payload)?;
                self.handle_stop_order(message.stream, message.message_id, stop_order)
                    .await?;
            }
            _ => {
                self.transport
                    .write_dlq(
                        stream,
                        &message.message_id,
                        &payload,
                        "unexpected_msg_type",
                        trim_maxlen,
                    )
                    .await?;
                self.transport.xack(stream, &message.message_id).await?;
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
            stream if stream == self.config.streams.trades => {
                let trade = serde_json::from_value(message.payload)?;
                self.handle_trade(message.stream, message.message_id, trade)
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
        if self.our_request_ids.remove(&ack.request_id) {
            if let Some(order_id) = ack.broker_order_id {
                self.our_order_ids.insert(order_id);
                if let Some(trades) = self.pending_trades_by_order_id.remove(&order_id) {
                    for trade in trades {
                        self.apply_trade_execution(trade);
                    }
                }
            }
        }
        match ack.status {
            alor_protocol::AckStatus::Rejected
            | alor_protocol::AckStatus::Expired
            | alor_protocol::AckStatus::Error => {
                warn!(
                    action = "command_acknowledged",
                    outcome = "rejected",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    error_msg = ?ack.error_msg,
                    cws_http_code = ?ack.cws_http_code,
                    cws_message = ?ack.cws_message,
                    cws_request_guid = ?ack.cws_request_guid,
                    "command rejected"
                );
            }
            alor_protocol::AckStatus::Accepted | alor_protocol::AckStatus::Confirmed => {
                info!(
                    action = "command_acknowledged",
                    outcome = "accepted",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    broker_order_id = ack.broker_order_id,
                    "command acknowledged"
                );
            }
            alor_protocol::AckStatus::Duplicate => {
                info!(
                    action = "command_acknowledged",
                    outcome = "duplicate",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    "command duplicate ack"
                );
            }
        }
        let event_ts = self.normalize_event_ts(ack.processed_ts_utc);
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, ack.processed_ts_utc);
        let intents_count = self
            .invoke_and_apply_strategy_callback(
                &ctx,
                event_ts,
                "on_ack",
                |strategy, strategy_ctx| strategy.on_ack(strategy_ctx, &ack),
            )
            .await?;
        self.audit_event(
            "order_acknowledged_by_strategy",
            json!({
                "callback": "on_ack",
                "event_ts_utc": event_ts,
                "request_id": ack.request_id.to_string(),
                "intents_count": intents_count,
            }),
        );
        self.transport.xack(&stream, &message_id).await?;
        self.health_snapshot.write().last_ack_ts_utc = Some(event_ts);
        Ok(())
    }

    async fn handle_order(
        &mut self,
        stream: String,
        message_id: String,
        order: OrderEvent,
    ) -> Result<()> {
        if self.config.trade_mode != TradeMode::Live {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        if order.symbol != self.config.strategy.symbol {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        let event_ts = self.normalize_event_ts(order.ts_utc);
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, order.ts_utc);
        let intents_count = self
            .invoke_and_apply_strategy_callback(
                &ctx,
                event_ts,
                "on_order",
                |strategy, strategy_ctx| strategy.on_order(strategy_ctx, &order),
            )
            .await?;
        self.audit_event(
            "order_acknowledged_by_strategy",
            json!({
                "callback": "on_order",
                "event_ts_utc": event_ts,
                "order_id": order.order_id,
                "status": order.status.clone(),
                "intents_count": intents_count,
            }),
        );
        self.update_ledger_from_order(&order)?;
        self.state.orders.insert(order.order_id, order);
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    async fn handle_stop_order(
        &mut self,
        stream: String,
        message_id: String,
        stop_order: StopOrderEvent,
    ) -> Result<()> {
        if self.config.trade_mode != TradeMode::Live {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        if stop_order.symbol != self.config.strategy.symbol {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        if !self.lifecycle_hook_enabled(StrategyLifecycleHook::StopOrder) {
            self.state
                .stop_orders
                .insert(stop_order.stop_order_id.clone(), stop_order);
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        let event_ts = self.normalize_event_ts(stop_order.ts_utc);
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, stop_order.ts_utc);
        let intents_count = self
            .invoke_and_apply_strategy_callback(
                &ctx,
                event_ts,
                "on_stop_order",
                |strategy, strategy_ctx| strategy.on_stop_order(strategy_ctx, &stop_order),
            )
            .await?;
        self.audit_event(
            "stop_order_acknowledged_by_strategy",
            json!({
                "event_ts_utc": event_ts,
                "stop_order_id": stop_order.stop_order_id.clone(),
                "status": stop_order.status.clone(),
                "intents_count": intents_count,
            }),
        );
        self.state
            .stop_orders
            .insert(stop_order.stop_order_id.clone(), stop_order);
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    async fn handle_trade(
        &mut self,
        stream: String,
        message_id: String,
        trade: TradeEvent,
    ) -> Result<()> {
        if self.config.trade_mode != TradeMode::Live {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        if trade.symbol != self.config.strategy.symbol {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        if !self.should_process_trade(&trade) {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        if trade.order_id <= 0 {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        let owned = self.our_order_ids.contains(&trade.order_id);
        if !owned {
            warn!(
                trade_id = trade.trade_id,
                order_id = trade.order_id,
                symbol = trade.symbol,
                side = trade.side,
                qty = trade.qty,
                price = trade.price,
                "orphan_trade"
            );
            self.our_order_ids.insert(trade.order_id);
            self.record_orphan_trade(&trade);
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        debug!(
            trade_id = trade.trade_id,
            order_id = trade.order_id,
            symbol = trade.symbol,
            side = trade.side,
            qty = trade.qty,
            price = trade.price,
            commission = trade.commission,
            existing = trade.existing,
            ts_utc = trade.ts_utc,
            "trade event accepted"
        );
        self.apply_trade_execution(trade);
        self.transport.xack(&stream, &message_id).await?;
        Ok(())
    }

    fn apply_trade_execution(&mut self, trade: TradeEvent) {
        if !self.pending_exec.contains_key(&trade.order_id) {
            if let Some(order) = self.state.orders.get(&trade.order_id) {
                let fill_target = if order.filled > 0.0 {
                    order.filled
                } else {
                    order.qty
                };
                self.pending_exec.insert(
                    trade.order_id,
                    PendingExecution {
                        order_id: trade.order_id,
                        symbol: order.symbol.clone(),
                        side: order.side.to_lowercase(),
                        target_qty: fill_target.max(trade.qty),
                        filled_qty: 0.0,
                        order_price: order.price,
                    },
                );
            }
        }
        if let Some(pending) = self.pending_exec.get_mut(&trade.order_id) {
            let exec_qty = trade.qty;
            if exec_qty > 0.0 {
                let trade_record = TradeRecord {
                    ts_utc: trade.ts_utc,
                    order_id: trade.order_id,
                    symbol: trade.symbol.clone(),
                    side: trade.side.to_lowercase(),
                    qty: exec_qty,
                    price: trade.price,
                    commission: trade.commission,
                    owned: true,
                };
                self.ledger.record_fill(trade_record);
                pending.filled_qty += exec_qty;
                let position_qty_runtime_snapshot =
                    self.state.positions.get(&pending.symbol).map(|p| p.qty);
                info!(
                    action = "execution_confirmed",
                    order_id = pending.order_id,
                    symbol = pending.symbol,
                    side = pending.side,
                    qty = exec_qty,
                    reference_price_from_order_record = pending.order_price,
                    exec_price = trade.price,
                    commission = trade.commission,
                    position_qty_runtime_snapshot = ?position_qty_runtime_snapshot,
                    "execution confirmed (exec_price is fill; reference_price_from_order_record is not execution price)"
                );
                if pending.filled_qty + f64::EPSILON >= pending.target_qty {
                    self.pending_exec.remove(&trade.order_id);
                }
            }
        } else {
            debug!(
                order_id = trade.order_id,
                trade_id = trade.trade_id,
                "trade ignored: no pending execution"
            );
        }
    }

    async fn handle_position(
        &mut self,
        stream: String,
        message_id: String,
        position: PositionEvent,
    ) -> Result<()> {
        if self.config.trade_mode != TradeMode::Live {
            // In paper/backtest, position lifecycle is driven by synthetic fills.
            // Ignore broker positions stream to avoid external-state contamination.
            if let Err(error) = self.transport.xack(&stream, &message_id).await {
                warn!(
                    stream = %stream,
                    message_id = %message_id,
                    ?error,
                    "paper mode: failed to ack ignored external position event"
                );
            }
            return Ok(());
        }
        if position.symbol != self.config.strategy.symbol {
            self.transport.xack(&stream, &message_id).await?;
            return Ok(());
        }
        let event_ts = self.normalize_event_ts(position.ts_utc);
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, position.ts_utc);
        let intents_count = self
            .invoke_and_apply_strategy_callback(
                &ctx,
                event_ts,
                "on_position",
                |strategy, strategy_ctx| strategy.on_position(strategy_ctx, &position),
            )
            .await?;
        self.audit_event(
            "position_acknowledged_by_strategy",
            json!({
                "event_ts_utc": event_ts,
                "symbol": position.symbol.clone(),
                "qty": position.qty,
                "intents_count": intents_count,
            }),
        );
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
        let prev_bar_ts = self.state.last_processed_bar_ts.get(&bar.symbol).copied();
        let event_ts = self.normalize_event_ts(bar.close_time_utc);
        if bar.origin == DataOrigin::Live {
            self.bootstrap_state.seen_live_bar = true;
        }
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(prev_bar_ts, event_ts);
        let (intents, previous_strategy_state) =
            self.invoke_strategy_callback(&ctx, "on_bar", |strategy, strategy_ctx| {
                strategy.on_bar(strategy_ctx, &bar)
            });
        self.metrics.bars_last_seen_close_time_utc = Some(bar.close_time_utc);
        if self.config.trade_mode != TradeMode::Live {
            self.record_non_live_intents(event_ts, &intents, self.config.trade_mode)
                .await?;
            if self.can_advance_paper_execution(bar.origin.clone()) {
                self.simulate_fills(&bar).await?;
                self.simulate_intents(&bar, intents).await?;
            }
            self.persist_state(None).await?;
        } else {
            self.apply_intents(&ctx, event_ts, intents, previous_strategy_state)
                .await?;
        }
        self.flush_risk_gate_session_finalizations().await?;
        self.state
            .update_last_bar_ts(&bar.symbol, bar.close_time_utc);
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
            strategy_state: StrategyStateEnvelopeCompat::from_strategy_state(
                self.config.strategy.strategy_kind,
                self.state.strategy_state.clone(),
            ),
            last_trade_ts: self.state.last_trade_ts,
            last_trade_id: self.state.last_trade_id.clone(),
            seen_trade_ids: self.state.seen_trade_ids.clone(),
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
        } else if let Err(error) = self
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
        Ok(())
    }

    fn update_ledger_from_order(&mut self, order: &OrderEvent) -> Result<()> {
        if order.order_id <= 0 {
            return Ok(());
        }
        if order.symbol != self.config.strategy.symbol {
            return Ok(());
        }
        let owned = self.our_order_ids.contains(&order.order_id);
        let status = order.status.to_lowercase();
        debug!(
            order_id = order.order_id,
            order_price = order.price,
            status = order.status,
            existing = order.existing,
            "order event price observed"
        );
        let record = OrderRecord {
            order_id: order.order_id,
            symbol: order.symbol.clone(),
            side: order.side.to_lowercase(),
            qty: order.qty,
            filled: order.filled,
            price: order.price,
            status: status.clone(),
            ts_utc: order.ts_utc,
            owned,
        };
        self.ledger.record_order(record);
        if self.config.trade_mode == TradeMode::Live {
            if owned {
                let prev_filled = self
                    .state
                    .orders
                    .get(&order.order_id)
                    .map(|prev| prev.status.eq_ignore_ascii_case("filled"))
                    .unwrap_or(false);
                if status == "filled" {
                    let fill_qty = if order.filled > 0.0 {
                        order.filled
                    } else {
                        order.qty
                    };
                    if fill_qty > 0.0 && !(prev_filled && order.existing) {
                        let entry = self.pending_exec.entry(order.order_id).or_insert_with(|| {
                            PendingExecution {
                                order_id: order.order_id,
                                symbol: order.symbol.clone(),
                                side: order.side.to_lowercase(),
                                target_qty: fill_qty,
                                filled_qty: 0.0,
                                order_price: order.price,
                            }
                        });
                        entry.target_qty = fill_qty;
                        entry.order_price = order.price;
                        info!(
                            order_id = order.order_id,
                            symbol = order.symbol,
                            side = order.side,
                            qty = fill_qty,
                            order_price = order.price,
                            exec_price = "UNKNOWN",
                            "order filled awaiting execution"
                        );
                    }
                }
            }
            return Ok(());
        }

        if status == "filled" {
            let fill_qty = if order.filled > 0.0 {
                order.filled
            } else {
                order.qty
            };
            if fill_qty > 0.0 {
                let trade = TradeRecord {
                    ts_utc: order.ts_utc,
                    order_id: order.order_id,
                    symbol: order.symbol.clone(),
                    side: order.side.to_lowercase(),
                    qty: fill_qty,
                    price: order.price,
                    commission: 0.0,
                    owned: true,
                };
                self.ledger.record_fill(trade);
                info!(
                    order_id = order.order_id,
                    symbol = order.symbol,
                    qty = fill_qty,
                    exec_price = order.price,
                    "order filled"
                );
            }
        }
        Ok(())
    }

    fn should_process_trade(&mut self, trade: &TradeEvent) -> bool {
        let trade_key = self.trade_dedupe_key(trade);
        if self.state.seen_trade_ids.iter().any(|id| id == &trade_key) {
            return false;
        }
        if let Some(last_ts) = self.state.last_trade_ts {
            if trade.ts_utc < last_ts {
                return false;
            }
        }
        self.remember_trade(&trade_key, trade.ts_utc);
        true
    }

    fn trade_dedupe_key(&self, trade: &TradeEvent) -> String {
        if trade.trade_id.trim().is_empty() {
            format!(
                "{}:{}:{}:{}:{}",
                trade.order_id, trade.ts_utc, trade.side, trade.qty, trade.price
            )
        } else {
            trade.trade_id.clone()
        }
    }

    fn remember_trade(&mut self, trade_key: &str, ts_utc: i64) {
        if self.state.seen_trade_ids.iter().any(|id| id == trade_key) {
            return;
        }
        self.state.seen_trade_ids.push(trade_key.to_string());
        if self.state.seen_trade_ids.len() > TRADE_DEDUP_LIMIT {
            let overflow = self.state.seen_trade_ids.len() - TRADE_DEDUP_LIMIT;
            self.state.seen_trade_ids.drain(0..overflow);
        }
        let last_ts = self.state.last_trade_ts.unwrap_or(i64::MIN);
        if ts_utc >= last_ts {
            self.state.last_trade_ts = Some(ts_utc);
            self.state.last_trade_id = Some(trade_key.to_string());
        }
    }

    async fn simulate_intents(&mut self, bar: &BarEvent, intents: Vec<Intent>) -> Result<()> {
        for intent in intents {
            let mut intent = intent;
            let mut intent_class = intent.explicit_class();
            while let Intent::Classified {
                intent: inner,
                intent_class: class,
            } = intent
            {
                intent_class = Some(class);
                intent = *inner;
            }
            match intent {
                Intent::Place {
                    price, qty, side, ..
                } => {
                    let order_id = self.next_sim_order_id;
                    self.next_sim_order_id += 1;
                    let side = format!("{side:?}").to_lowercase();
                    self.sim_orders.push(SimOrder {
                        order_id,
                        symbol: bar.symbol.clone(),
                        side: side.clone(),
                        intent_class,
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
                        owned: true,
                    });
                }
                Intent::Market {
                    qty,
                    side,
                    fill_price,
                    ..
                } => {
                    let symbol = bar.symbol.clone();
                    let current_pos_qty = self
                        .state
                        .positions
                        .get(&symbol)
                        .map(|p| p.qty)
                        .unwrap_or(0.0);
                    let mut effective_qty = qty;
                    let is_exit = matches!(intent_class, Some(alor_protocol::IntentClass::Exit));
                    if is_exit {
                        let pos_abs = current_pos_qty.abs();
                        if pos_abs <= f64::EPSILON {
                            info!(
                                target: "strategy_runtime::runtime",
                                strategy_id = %self.config.strategy.strategy_id,
                                symbol = %symbol,
                                intent_class = "EXIT",
                                side = ?side,
                                intent_qty = qty,
                                current_position_qty = current_pos_qty,
                                drop_reason = "flat_position",
                                "paper_exit_dropped"
                            );
                            self.audit_event(
                                "intent_blocked",
                                json!({
                                    "mode": "paper",
                                    "reason": "flat_position",
                                    "action": "market",
                                    "class": "Exit",
                                    "symbol": symbol.clone(),
                                    "requested_qty": qty,
                                    "current_position_qty": current_pos_qty,
                                }),
                            );
                            continue;
                        }
                        let wrong_side = (side == alor_protocol::Side::Sell
                            && current_pos_qty < 0.0)
                            || (side == alor_protocol::Side::Buy && current_pos_qty > 0.0);
                        if wrong_side {
                            info!(
                                target: "strategy_runtime::runtime",
                                strategy_id = %self.config.strategy.strategy_id,
                                symbol = %symbol,
                                intent_class = "EXIT",
                                side = ?side,
                                intent_qty = qty,
                                current_position_qty = current_pos_qty,
                                drop_reason = "wrong_side_for_exit",
                                "paper_exit_dropped"
                            );
                            self.audit_event(
                                "intent_blocked",
                                json!({
                                    "mode": "paper",
                                    "reason": "wrong_side_for_exit",
                                    "action": "market",
                                    "class": "Exit",
                                    "symbol": symbol.clone(),
                                    "requested_qty": qty,
                                    "current_position_qty": current_pos_qty,
                                }),
                            );
                            continue;
                        }
                        effective_qty = effective_qty.min(pos_abs);
                        if effective_qty <= f64::EPSILON {
                            info!(
                                target: "strategy_runtime::runtime",
                                strategy_id = %self.config.strategy.strategy_id,
                                symbol = %symbol,
                                intent_class = "EXIT",
                                side = ?side,
                                intent_qty = qty,
                                current_position_qty = current_pos_qty,
                                drop_reason = "effective_qty_zero",
                                "paper_exit_dropped"
                            );
                            self.audit_event(
                                "intent_blocked",
                                json!({
                                    "mode": "paper",
                                    "reason": "effective_qty_zero",
                                    "action": "market",
                                    "class": "Exit",
                                    "symbol": symbol.clone(),
                                    "requested_qty": qty,
                                    "current_position_qty": current_pos_qty,
                                }),
                            );
                            continue;
                        }
                    } else if side == alor_protocol::Side::Buy && current_pos_qty < 0.0 {
                        // Non-exit market intents are still prevented from flipping an opposite position in paper mode.
                        effective_qty = effective_qty.min(current_pos_qty.abs());
                    } else if side == alor_protocol::Side::Sell && current_pos_qty > 0.0 {
                        effective_qty = effective_qty.min(current_pos_qty.abs());
                    }
                    if effective_qty <= 0.0 {
                        continue;
                    }
                    let order_id = self.next_sim_order_id;
                    self.next_sim_order_id += 1;
                    let side = format!("{side:?}").to_lowercase();

                    if let Some(price) = fill_price {
                        self.ledger.record_fill(TradeRecord {
                            ts_utc: bar.close_time_utc,
                            order_id,
                            symbol: symbol.clone(),
                            side: side.clone(),
                            qty: effective_qty,
                            price,
                            commission: 0.0,
                            owned: true,
                        });
                        self.ledger.record_order(OrderRecord {
                            order_id,
                            symbol: symbol.clone(),
                            side,
                            qty: effective_qty,
                            filled: effective_qty,
                            price,
                            status: "filled".to_string(),
                            ts_utc: bar.close_time_utc,
                            owned: true,
                        });
                        self.persist_ledger_reports().await?;
                    } else {
                        self.sim_orders.push(SimOrder {
                            order_id,
                            symbol: symbol.clone(),
                            side: side.clone(),
                            intent_class,
                            order_type: SimOrderType::Market,
                            qty: effective_qty,
                            price: None,
                            created_bar_ts: bar.close_time_utc,
                        });
                        self.ledger.record_order(OrderRecord {
                            order_id,
                            symbol,
                            side,
                            qty: effective_qty,
                            filled: 0.0,
                            price: 0.0,
                            status: "working".to_string(),
                            ts_utc: bar.close_time_utc,
                            owned: true,
                        });
                    }
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
                            owned: true,
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
                            owned: true,
                        });
                    }
                }
                Intent::CreateStopLimit { .. } | Intent::DeleteStopLimit { .. } => {
                    // Backtest/paper simulator does not emulate broker-side stop-order lifecycle.
                }
                Intent::Classified { .. } => unreachable!("classified intents are flattened above"),
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
                    if (order.side == "buy" && bar.l <= price)
                        || (order.side == "sell" && bar.h >= price)
                    {
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
                let mut effective_qty = order.qty;
                if matches!(order.intent_class, Some(alor_protocol::IntentClass::Exit)) {
                    let current_pos_qty = self
                        .state
                        .positions
                        .get(&order.symbol)
                        .map(|p| p.qty)
                        .unwrap_or(0.0);
                    let pos_abs = current_pos_qty.abs();
                    if pos_abs <= f64::EPSILON {
                        info!(
                            target: "strategy_runtime::runtime",
                            strategy_id = %self.config.strategy.strategy_id,
                            symbol = %order.symbol,
                            intent_class = "EXIT",
                            requested_qty = order.qty,
                            current_position_qty_before_fill = current_pos_qty,
                            effective_qty_after_recalc = 0.0,
                            fill_drop_reason = "flat_position",
                            "paper_exit_fill_dropped"
                        );
                        self.audit_event(
                            "intent_blocked",
                            json!({
                                "mode": "paper",
                                "reason": "flat_position",
                                "action": "simulated_fill",
                                "class": "Exit",
                                "symbol": order.symbol.clone(),
                                "requested_qty": order.qty,
                                "current_position_qty_before_fill": current_pos_qty,
                            }),
                        );
                        self.ledger.record_order(OrderRecord {
                            order_id: order.order_id,
                            symbol: order.symbol.clone(),
                            side: order.side.clone(),
                            qty: order.qty,
                            filled: 0.0,
                            price: order.price.unwrap_or(price),
                            status: "dropped".to_string(),
                            ts_utc: bar.close_time_utc,
                            owned: true,
                        });
                        continue;
                    }
                    let is_sell = order.side.eq_ignore_ascii_case("sell");
                    let wrong_side =
                        (is_sell && current_pos_qty < 0.0) || (!is_sell && current_pos_qty > 0.0);
                    if wrong_side {
                        info!(
                            target: "strategy_runtime::runtime",
                            strategy_id = %self.config.strategy.strategy_id,
                            symbol = %order.symbol,
                            intent_class = "EXIT",
                            requested_qty = order.qty,
                            current_position_qty_before_fill = current_pos_qty,
                            effective_qty_after_recalc = 0.0,
                            fill_drop_reason = "wrong_side_for_exit",
                            "paper_exit_fill_dropped"
                        );
                        self.audit_event(
                            "intent_blocked",
                            json!({
                                "mode": "paper",
                                "reason": "wrong_side_for_exit",
                                "action": "simulated_fill",
                                "class": "Exit",
                                "symbol": order.symbol.clone(),
                                "requested_qty": order.qty,
                                "current_position_qty_before_fill": current_pos_qty,
                            }),
                        );
                        self.ledger.record_order(OrderRecord {
                            order_id: order.order_id,
                            symbol: order.symbol.clone(),
                            side: order.side.clone(),
                            qty: order.qty,
                            filled: 0.0,
                            price: order.price.unwrap_or(price),
                            status: "dropped".to_string(),
                            ts_utc: bar.close_time_utc,
                            owned: true,
                        });
                        continue;
                    }
                    effective_qty = effective_qty.min(pos_abs);
                    if effective_qty <= f64::EPSILON {
                        info!(
                            target: "strategy_runtime::runtime",
                            strategy_id = %self.config.strategy.strategy_id,
                            symbol = %order.symbol,
                            intent_class = "EXIT",
                            requested_qty = order.qty,
                            current_position_qty_before_fill = current_pos_qty,
                            effective_qty_after_recalc = effective_qty,
                            fill_drop_reason = "effective_qty_zero",
                            "paper_exit_fill_dropped"
                        );
                        self.audit_event(
                            "intent_blocked",
                            json!({
                                "mode": "paper",
                                "reason": "effective_qty_zero",
                                "action": "simulated_fill",
                                "class": "Exit",
                                "symbol": order.symbol.clone(),
                                "requested_qty": order.qty,
                                "current_position_qty_before_fill": current_pos_qty,
                            }),
                        );
                        self.ledger.record_order(OrderRecord {
                            order_id: order.order_id,
                            symbol: order.symbol.clone(),
                            side: order.side.clone(),
                            qty: order.qty,
                            filled: 0.0,
                            price: order.price.unwrap_or(price),
                            status: "dropped".to_string(),
                            ts_utc: bar.close_time_utc,
                            owned: true,
                        });
                        continue;
                    }
                }
                let trade = TradeRecord {
                    ts_utc: bar.close_time_utc,
                    order_id: order.order_id,
                    symbol: order.symbol.clone(),
                    side: order.side.clone(),
                    qty: effective_qty,
                    price,
                    commission: 0.0,
                    owned: true,
                };
                self.ledger.record_fill(trade);
                self.ledger.record_order(OrderRecord {
                    order_id: order.order_id,
                    symbol: order.symbol.clone(),
                    side: order.side.clone(),
                    qty: effective_qty,
                    filled: effective_qty,
                    price,
                    status: "filled".to_string(),
                    ts_utc: bar.close_time_utc,
                    owned: true,
                });
                // Synthetic paper feedback: propagate position delta back into strategy.
                let delta_qty = if order.side.eq_ignore_ascii_case("buy") {
                    effective_qty
                } else {
                    -effective_qty
                };
                let prev_qty = self
                    .state
                    .positions
                    .get(&order.symbol)
                    .map(|p| p.qty)
                    .unwrap_or(0.0);
                let next_qty = prev_qty + delta_qty;
                let pos_event = PositionEvent {
                    symbol: order.symbol.clone(),
                    qty: next_qty,
                    existing: false,
                    avg_price: price,
                    ts_utc: bar.close_time_utc,
                };
                self.state
                    .positions
                    .insert(order.symbol.clone(), pos_event.clone());
                let ctx = self.strategy_ctx();
                let intents = self.strategy.on_position(&ctx, &pos_event);
                self.state.strategy_state = self.strategy.state().clone();
                if !intents.is_empty() {
                    self.record_non_live_intents(
                        bar.close_time_utc,
                        &intents,
                        self.config.trade_mode,
                    )
                    .await?;
                    self.simulate_intents(bar, intents).await?;
                }
                self.persist_ledger_reports().await?;
            }
        }
        Ok(())
    }

    async fn persist_ledger_reports(&self) -> Result<()> {
        match self.config.trade_mode {
            TradeMode::Paper => self.ledger.persist_reports(
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

    fn persist_replay_parity_report(&self) -> Result<()> {
        if !self.config.replay.enabled {
            return Ok(());
        }

        let Some(reference_path) = self.config.replay.reference_trades_csv_path.as_deref() else {
            return Ok(());
        };

        let reference = Self::load_reference_trades(reference_path)?;
        let report = Self::build_replay_parity_report(
            self.ledger.closed_trades(),
            &reference,
            self.config.replay.price_tolerance,
        );

        let output_path = format!("{}/parity_report.json", self.config.replay.output_dir);
        if let Some(parent) = std::path::Path::new(&output_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create replay output directory {}",
                        parent.display()
                    )
                })?;
            }
        }

        let file = std::fs::File::create(&output_path)
            .with_context(|| format!("failed to create replay parity report at {output_path}"))?;
        serde_json::to_writer_pretty(file, &report)?;

        info!(
            output_path = %output_path,
            status = %report.status,
            runtime_trades = report.runtime_trades,
            reference_trades = report.reference_trades,
            matched_trades = report.matched_trades,
            "replay parity report written"
        );

        Ok(())
    }

    fn load_reference_trades(path: &str) -> Result<Vec<ReplayReferenceTradeRow>> {
        if !std::path::Path::new(path).exists() {
            return Ok(Vec::new());
        }
        let mut rdr = csv::Reader::from_path(path)
            .with_context(|| format!("failed to open reference trades csv at {path}"))?;
        let mut rows = Vec::new();
        for row in rdr.deserialize::<ReplayReferenceTradeRow>() {
            rows.push(
                row.with_context(|| {
                    format!("failed to decode reference trades csv row in {path}")
                })?,
            );
        }
        Ok(rows)
    }

    fn normalize_reference_side(direction: &str) -> Option<&'static str> {
        if direction.eq_ignore_ascii_case("long") || direction.eq_ignore_ascii_case("buy") {
            Some("buy")
        } else if direction.eq_ignore_ascii_case("short") || direction.eq_ignore_ascii_case("sell")
        {
            Some("sell")
        } else {
            None
        }
    }

    fn build_replay_parity_report(
        runtime: &[crate::trade_ledger::ClosedTradeRecord],
        reference: &[ReplayReferenceTradeRow],
        tolerance: f64,
    ) -> ReplayParityReport {
        let mut first_divergence = None;
        let mut matched = 0usize;
        let common = runtime.len().min(reference.len());

        for index in 0..common {
            let rt = &runtime[index];
            let rf = &reference[index];

            let entry_match = chrono::DateTime::parse_from_rfc3339(&rf.entry_time)
                .ok()
                .map(|d| d.timestamp())
                == Some(rt.entry_ts_utc);
            let exit_match = chrono::DateTime::parse_from_rfc3339(&rf.exit_time)
                .ok()
                .map(|d| d.timestamp())
                == Some(rt.exit_ts_utc);
            let normalized_ref_side = Self::normalize_reference_side(&rf.direction);
            let side_match = normalized_ref_side
                .map(|side| side.eq_ignore_ascii_case(&rt.side))
                .unwrap_or_else(|| rf.direction.eq_ignore_ascii_case(&rt.side));
            let qty_match = (rf.size as f64 - rt.qty).abs() <= tolerance;
            let entry_price_match = (rf.entry_price - rt.entry_price).abs() <= tolerance;
            let exit_price_match = (rf.exit_price - rt.exit_price).abs() <= tolerance;
            let pnl_match = (rf.pnl - rt.pnl_net).abs() <= tolerance;

            if entry_match
                && exit_match
                && side_match
                && qty_match
                && entry_price_match
                && exit_price_match
                && pnl_match
            {
                matched += 1;
                continue;
            }

            first_divergence = Some(format!(
                "index={index} entry_match={entry_match} exit_match={exit_match} side_match={side_match} qty_match={qty_match} entry_price_match={entry_price_match} exit_price_match={exit_price_match} pnl_match={pnl_match} reason_ref={} expected_side={} runtime={} ",
                rf.reason,
                normalized_ref_side.unwrap_or(rf.direction.as_str()),
                json!({
                    "entry_ts_utc": rt.entry_ts_utc,
                    "exit_ts_utc": rt.exit_ts_utc,
                    "side": rt.side,
                    "qty": rt.qty,
                    "entry_price": rt.entry_price,
                    "exit_price": rt.exit_price,
                    "pnl_net": rt.pnl_net,
                })
            ));
            break;
        }

        if first_divergence.is_none() && runtime.len() != reference.len() {
            first_divergence = Some(format!(
                "trade count mismatch runtime={} reference={}",
                runtime.len(),
                reference.len()
            ));
        }

        ReplayParityReport {
            status: if first_divergence.is_none() {
                "pass".to_string()
            } else {
                "fail".to_string()
            },
            tolerance,
            runtime_trades: runtime.len(),
            reference_trades: reference.len(),
            matched_trades: matched,
            first_divergence,
        }
    }

    fn strategy_ctx_with_last_bar_and_event_ts(
        &self,
        last_bar_ts: Option<i64>,
        event_ts_utc: i64,
    ) -> StrategyCtx {
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
            paper_execution_mode: self.config.paper.execution_mode,
            allow_live_orders: self.config.allow_live_orders,
            gateway_phase,
            position_qty,
            event_ts_utc,
            now_ts_utc: Utc::now().timestamp(),
            last_bar_ts,
        }
    }

    fn strategy_ctx_with_last_bar(&self, last_bar_ts: Option<i64>) -> StrategyCtx {
        self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, self.strategy_now_ts_utc)
    }

    fn strategy_ctx(&self) -> StrategyCtx {
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        self.strategy_ctx_with_last_bar(last_bar_ts)
    }

    fn compute_intraday_stop_end_utc(&self, created_ts_utc: i64) -> Option<i64> {
        if created_ts_utc <= 0 {
            return None;
        }
        let offset_hours = self
            .config
            .strategy
            .trading_periods
            .as_ref()
            .map(|p| {
                if p.timezone_offset_hours == 0 {
                    self.config.strategy.timezone_offset_hours
                } else {
                    p.timezone_offset_hours
                }
            })
            .unwrap_or(self.config.strategy.timezone_offset_hours);
        let offset = FixedOffset::east_opt(offset_hours.saturating_mul(3600))?;
        let local_dt = Utc
            .timestamp_opt(created_ts_utc, 0)
            .single()?
            .with_timezone(&offset);
        let session_end = self
            .config
            .strategy
            .trading_periods
            .as_ref()
            .map(|p| p.session_end)
            .unwrap_or_else(|| {
                NaiveTime::from_hms_opt(
                    self.config.strategy.session_close_hour.min(23),
                    self.config.strategy.session_close_minute.min(59),
                    0,
                )
                .unwrap_or_else(|| NaiveTime::from_hms_opt(23, 50, 0).expect("valid time"))
            });
        let weekends_off = self
            .config
            .strategy
            .trading_periods
            .as_ref()
            .map(|p| p.weekends_off)
            .unwrap_or(false);
        let mut day = local_dt.date_naive();
        for _ in 0..8 {
            if weekends_off && matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
                day += ChronoDuration::days(1);
                continue;
            }
            let local_close = day.and_time(session_end);
            if let Some(with_offset) = offset.from_local_datetime(&local_close).single() {
                let stop_end = with_offset
                    .timestamp()
                    .saturating_add(STOP_END_BUFFER_SEC_DEFAULT);
                if stop_end > created_ts_utc {
                    return Some(stop_end);
                }
            }
            day += ChronoDuration::days(1);
        }
        None
    }

    async fn notify_bootstrap_snapshot(&mut self) -> Result<()> {
        if !self.lifecycle_hook_enabled(StrategyLifecycleHook::BootstrapSnapshot) {
            return Ok(());
        }
        let snapshot = match &self.bootstrap_snapshot {
            Some(snapshot) => snapshot.clone(),
            None => return Ok(()),
        };
        let created_ts = self.normalize_event_ts(snapshot.snapshot_ts_utc.unwrap_or(0));
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, created_ts);
        let intents_count = self
            .invoke_and_apply_strategy_callback(
                &ctx,
                created_ts,
                "on_bootstrap_snapshot",
                |strategy, strategy_ctx| strategy.on_bootstrap_snapshot(strategy_ctx, &snapshot),
            )
            .await?;
        self.audit_event(
            "bootstrap_processed",
            json!({
                "created_ts_utc": created_ts,
                "intents_count": intents_count,
            }),
        );
        Ok(())
    }

    async fn notify_runtime_state_restored(&mut self) -> Result<()> {
        if !self.lifecycle_hook_enabled(StrategyLifecycleHook::RuntimeStateRestore) {
            return Ok(());
        }
        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let restored = RuntimeStateRestored {
            known_order_ids: self.our_order_ids.iter().copied().collect(),
            pending_requests: self.our_request_ids.iter().copied().collect(),
        };
        let created_ts = self.normalize_event_ts(last_bar_ts.unwrap_or(0));
        let ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, created_ts);
        let intents_count = self
            .invoke_and_apply_strategy_callback(
                &ctx,
                created_ts,
                "on_runtime_state_restored",
                |strategy, strategy_ctx| {
                    strategy.on_runtime_state_restored(strategy_ctx, &restored)
                },
            )
            .await?;
        self.audit_event(
            "runtime_state_restored",
            json!({
                "created_ts_utc": created_ts,
                "known_order_ids_count": restored.known_order_ids.len(),
                "pending_requests_count": restored.pending_requests.len(),
                "intents_count": intents_count,
            }),
        );
        Ok(())
    }

    fn record_orphan_trade(&mut self, trade: &TradeEvent) {
        let record = TradeRecord {
            ts_utc: trade.ts_utc,
            order_id: trade.order_id,
            symbol: trade.symbol.clone(),
            side: trade.side.to_lowercase(),
            qty: trade.qty,
            price: trade.price,
            commission: trade.commission,
            owned: false,
        };
        self.ledger.record_fill(record);
    }

    fn is_open_position(&self, position: &PositionEvent) -> bool {
        position.qty.abs() > f64::EPSILON
    }

    fn is_working_order(&self, order: &OrderEvent) -> bool {
        if order.order_id <= 0 {
            return false;
        }
        let status = order.status.to_lowercase();
        !NON_WORKING_ORDER_STATUSES.contains(&status.as_str())
    }

    fn is_working_stop_order(&self, order: &StopOrderEvent) -> bool {
        if order.stop_order_id.trim().is_empty() {
            return false;
        }
        let status = order.status.to_lowercase();
        !NON_WORKING_STOP_ORDER_STATUSES.contains(&status.as_str())
    }

    fn log_bootstrap_dump(
        &self,
        positions_strategy: &HashMap<String, PositionEvent>,
        strategy_orders: &HashMap<i64, OrderEvent>,
        positions_open_strategy: usize,
        orders_open_strategy: usize,
        snapshot_ts_utc: Option<i64>,
    ) {
        if !self.config.bootstrap_dump {
            return;
        }
        let mut positions: Vec<PositionDump> = positions_strategy
            .iter()
            .map(|(symbol, position)| PositionDump {
                symbol: symbol.clone(),
                qty: position.qty,
                existing: position.existing,
                avg_price: position.avg_price,
                ts_utc: position.ts_utc,
            })
            .collect();
        positions.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        let mut orders: Vec<OrderDump> = strategy_orders
            .iter()
            .map(|(order_id, order)| OrderDump {
                order_id: *order_id,
                status: order.status.clone(),
                side: order.side.clone(),
                price: order.price,
                qty: order.qty,
                filled: order.filled,
                existing: order.existing,
                request_id: order.request_id,
                comment: order.comment.clone(),
                ts_utc: order.ts_utc,
            })
            .collect();
        orders.sort_by(|a, b| a.order_id.cmp(&b.order_id));
        info!(
            source = "snapshot",
            snapshot_ts_utc,
            positions = ?positions,
            orders = ?orders,
            positions_open_strategy,
            orders_open_strategy,
            open_order_excluded_statuses = ?NON_WORKING_ORDER_STATUSES,
            open_stop_order_excluded_statuses = ?NON_WORKING_STOP_ORDER_STATUSES,
            "bootstrap_dump"
        );
    }

    fn log_runtime_state_dump(&self) {
        if !self.config.bootstrap_dump {
            return;
        }
        let pending_request_ids = self.strategy.pending_request_ids();
        let strategy_state_order_ids = self.strategy.tracked_order_ids();
        let mut our_request_ids: Vec<_> = self.our_request_ids.iter().copied().collect();
        our_request_ids.sort();
        let mut known_order_ids = self.our_order_ids.clone();
        known_order_ids.extend(strategy_state_order_ids.iter().copied());
        let mut active_known_orders: Vec<(i64, String)> = known_order_ids
            .iter()
            .map(|order_id| {
                let status = self
                    .state
                    .orders
                    .get(order_id)
                    .map(|order| order.status.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                (*order_id, status)
            })
            .collect();
        active_known_orders.sort_by(|a, b| a.0.cmp(&b.0));
        info!(
            source = "runtime_state",
            active_known_orders = ?active_known_orders,
            strategy_state_order_ids = ?strategy_state_order_ids,
            strategy_state = ?self.state.strategy_state,
            pending_request_ids = ?pending_request_ids,
            pending_request_ids_len = pending_request_ids.len(),
            our_request_ids = ?our_request_ids,
            our_request_ids_len = our_request_ids.len(),
            "state_dump"
        );
    }

    fn trading_window_allows_order(
        &self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        intent_class: alor_protocol::IntentClass,
    ) -> bool {
        if !matches!(intent_class, alor_protocol::IntentClass::Entry) {
            return true;
        }
        let Some(periods) = &self.config.strategy.trading_periods else {
            return true;
        };

        let scheduler = Scheduler::new_with_fallback_offset_hours(
            periods.clone(),
            self.config.strategy.timezone_offset_hours,
        );
        let Some(now_utc) = chrono::DateTime::from_timestamp(created_ts_utc, 0) else {
            return true;
        };
        let now_local = scheduler
            .local_datetime_utc(created_ts_utc)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "unknown".to_string());
        let market_state = scheduler.market_state_utc(created_ts_utc);

        if market_state != MarketState::Open {
            info!(
                state = ?market_state,
                created_ts_utc,
                now_local,
                "intent_dropped_market_closed"
            );
            return false;
        }

        let max_silence = self.config.strategy.max_silence_bars_sec as i64;
        if max_silence == 0 {
            return true;
        }

        let Some(last_bar_ts) = ctx.last_bar_ts() else {
            info!("intent_dropped_waiting_for_first_bar");
            return false;
        };

        let Some(last_bar_dt) =
            chrono::DateTime::from_timestamp(last_bar_ts, 0).map(|dt| dt.naive_utc())
        else {
            return true;
        };

        if !scheduler.check_silence_period_at(now_utc.naive_utc(), last_bar_dt, max_silence) {
            warn!(
                last_bar_ts_utc = last_bar_ts,
                max_silence_bars_sec = max_silence,
                state = ?market_state,
                created_ts_utc,
                now_local,
                "intent_dropped_bar_silence"
            );
            return false;
        }

        true
    }

    fn current_market_state(&self, created_ts_utc: i64) -> Option<(MarketState, String)> {
        let periods = self.config.strategy.trading_periods.clone()?;
        let scheduler = Scheduler::new_with_fallback_offset_hours(
            periods,
            self.config.strategy.timezone_offset_hours,
        );
        let now_local = scheduler
            .local_datetime_utc(created_ts_utc)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "unknown".to_string());
        Some((scheduler.market_state_utc(created_ts_utc), now_local))
    }

    async fn maybe_defer_exit_before_emit(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        intent: Intent,
        intent_class: alor_protocol::IntentClass,
    ) -> Result<bool> {
        if intent_class != alor_protocol::IntentClass::Exit {
            return Ok(false);
        }
        let Some((market_state, now_local)) = self.current_market_state(created_ts_utc) else {
            return Ok(false);
        };
        if market_state == MarketState::Open {
            return Ok(false);
        }

        let action = self.intent_action_name(&intent);
        let command = self.intent_to_command(ctx, created_ts_utc, intent, intent_class);
        info!(
            action,
            class = ?intent_class,
            state = ?market_state,
            created_ts_utc,
            now_local,
            request_id = %command.request_id,
            "intent_deferred_by_trading_window_pre_emit"
        );
        self.audit_event(
            "intent_deferred",
            json!({
                "reason": "trading_window_pre_emit",
                "action": action,
                "class": format!("{intent_class:?}"),
                "state": format!("{market_state:?}"),
                "request_id": command.request_id.to_string(),
                "created_ts_utc": created_ts_utc,
                "synthetic_ack": true,
            }),
        );

        let mut ack = alor_protocol::CommandAck::rejected(
            command.request_id,
            "trading_window_closed",
            "validation failed",
        );
        ack.processed_ts_utc = created_ts_utc;

        let last_bar_ts = self
            .state
            .last_processed_bar_ts
            .get(&self.config.strategy.symbol)
            .copied();
        let ack_ctx = self.strategy_ctx_with_last_bar_and_event_ts(last_bar_ts, created_ts_utc);
        let (follow_up_intents, _previous_strategy_state) = self.invoke_strategy_callback(
            &ack_ctx,
            "on_ack_pre_emit_window_closed",
            |strategy, strategy_ctx| strategy.on_ack(strategy_ctx, &ack),
        );
        let intents_count = follow_up_intents.len();
        if !follow_up_intents.is_empty() {
            warn!(
                request_id = %ack.request_id,
                intents_count,
                "synthetic_pre_emit_ack_generated_unexpected_follow_up_intents"
            );
        }
        self.audit_event(
            "order_acknowledged_by_strategy",
            json!({
                "callback": "on_ack_pre_emit_window_closed",
                "event_ts_utc": created_ts_utc,
                "request_id": ack.request_id.to_string(),
                "intents_count": intents_count,
                "synthetic_ack": true,
            }),
        );
        self.persist_state(None).await?;
        Ok(true)
    }

    fn restore_strategy_state_after_dropped_intents(
        &mut self,
        previous_state: StrategyState,
        reason: &'static str,
    ) {
        let current_state = self.state.strategy_state.clone();
        info!(
            from = ?current_state,
            to = ?previous_state,
            reason,
            "strategy_state_transition_reverted"
        );
        self.audit_event(
            "intent_blocked",
            json!({
                "reason": reason,
                "action": "batch",
            }),
        );
        self.strategy.set_state(previous_state.clone());
        self.state.strategy_state = previous_state;
    }

    async fn apply_intents(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        intents: Vec<Intent>,
        previous_strategy_state: StrategyState,
    ) -> Result<()> {
        if intents.is_empty() {
            self.persist_state(None).await?;
            return Ok(());
        }
        self.health_snapshot.write().last_intent_ts_utc = Some(created_ts_utc);
        match self.config.trade_mode {
            TradeMode::Live => {
                let mut accepted = Vec::new();
                for intent in intents {
                    let intent_class = Self::resolve_intent_class(ctx, &intent);
                    if !self.trading_window_allows_order(ctx, created_ts_utc, intent_class) {
                        let action = self.intent_action_name(&intent);
                        info!(
                            action,
                            class = ?intent_class,
                            "intent_dropped_by_trading_window"
                        );
                        self.audit_event(
                            "intent_blocked",
                            json!({
                                "reason": "trading_window",
                                "action": action,
                                "class": format!("{intent_class:?}"),
                                "created_ts_utc": created_ts_utc,
                            }),
                        );
                        continue;
                    }
                    accepted.push((intent, intent_class));
                }
                if accepted.is_empty() {
                    self.restore_strategy_state_after_dropped_intents(
                        previous_strategy_state,
                        "intent_dropped_before_emit",
                    );
                    self.persist_state(None).await?;
                    return Ok(());
                }
                let decision = self.evaluate_guard_decision();
                if !decision.allowed {
                    self.log_guard_decision_if_due(&decision)?;
                    let has_open_position = ctx.position_qty.unwrap_or(0.0).abs() > 0.0;
                    let mut passthrough = Vec::new();
                    for (intent, intent_class) in accepted {
                        if self.guard_allows_intent_when_blocked(intent_class, has_open_position) {
                            passthrough.push((intent, intent_class));
                        } else {
                            let action = self.intent_action_name(&intent);
                            info!(
                                action,
                                class = ?intent_class,
                                reasons = ?decision.reasons,
                                "intent_dropped_by_guard"
                            );
                            self.audit_event(
                                "intent_blocked",
                                json!({
                                    "reason": "live_guard",
                                    "action": action,
                                    "class": format!("{intent_class:?}"),
                                    "guard_reasons": decision.reasons.clone(),
                                    "created_ts_utc": created_ts_utc,
                                }),
                            );
                        }
                    }
                    if passthrough.is_empty() {
                        self.restore_strategy_state_after_dropped_intents(
                            previous_strategy_state,
                            "intent_dropped_by_guard",
                        );
                        self.persist_state(None).await?;
                        return Ok(());
                    }
                    for (intent, intent_class) in passthrough {
                        let action = self.intent_action_name(&intent);
                        if self
                            .maybe_defer_exit_before_emit(
                                ctx,
                                created_ts_utc,
                                intent.clone(),
                                intent_class,
                            )
                            .await?
                        {
                            continue;
                        }
                        let command =
                            self.intent_to_command(ctx, created_ts_utc, intent, intent_class);
                        info!(
                            action,
                            class = ?intent_class,
                            request_id = %command.request_id,
                            reasons = ?decision.reasons,
                            "intent_emitted_guard_close_only_path"
                        );
                        self.audit_event(
                            "intent_emitted",
                            json!({
                                "path": "guard_close_only",
                                "action": action,
                                "class": format!("{intent_class:?}"),
                                "request_id": command.request_id.to_string(),
                                "created_ts_utc": created_ts_utc,
                            }),
                        );
                        self.persist_state(Some(&command)).await?;
                        self.our_request_ids.insert(command.request_id);
                    }
                    return Ok(());
                }
                for (intent, intent_class) in accepted {
                    let action = self.intent_action_name(&intent);
                    if self
                        .maybe_defer_exit_before_emit(
                            ctx,
                            created_ts_utc,
                            intent.clone(),
                            intent_class,
                        )
                        .await?
                    {
                        continue;
                    }
                    let command = self.intent_to_command(ctx, created_ts_utc, intent, intent_class);
                    info!(
                        action,
                        request_id = %command.request_id,
                        "intent_emitted"
                    );
                    self.audit_event(
                        "intent_emitted",
                        json!({
                            "path": "normal",
                            "action": action,
                            "class": format!("{intent_class:?}"),
                            "request_id": command.request_id.to_string(),
                            "created_ts_utc": created_ts_utc,
                        }),
                    );
                    let test_delay_ms = Self::test_delay_before_publish_ms();
                    if test_delay_ms > 0 {
                        warn!(
                            request_id = %command.request_id,
                            delay_ms = test_delay_ms,
                            "test_delay_before_publish"
                        );
                        sleep(Duration::from_millis(test_delay_ms)).await;
                    }
                    self.persist_state(Some(&command)).await?;
                    self.our_request_ids.insert(command.request_id);
                }
            }
            TradeMode::Paper => {
                self.audit_event(
                    "intent_emitted",
                    json!({
                        "path": "virtual",
                        "mode": "paper",
                        "intents_count": intents.len(),
                        "created_ts_utc": created_ts_utc,
                    }),
                );
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
                self.audit_event(
                    "intent_emitted",
                    json!({
                        "path": "virtual",
                        "mode": "backtest",
                        "intents_count": intents.len(),
                        "created_ts_utc": created_ts_utc,
                    }),
                );
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
        intent_class: alor_protocol::IntentClass,
    ) -> alor_protocol::OrderCommand {
        let fallback_comment = self
            .strategy
            .intent_comment_tag(ctx, created_ts_utc, intent_class);
        let (action, seq, action_name) = match intent {
            Intent::Classified { intent, .. } => {
                return self.intent_to_command(ctx, created_ts_utc, *intent, intent_class);
            }
            Intent::Place {
                price,
                qty,
                side,
                comment,
            } => {
                let comment = Self::sanitize_comment(comment.or_else(|| fallback_comment.clone()));
                (
                    alor_protocol::CommandAction::Place(alor_protocol::PlaceOrder {
                        price,
                        qty,
                        side,
                        comment,
                    }),
                    0,
                    "place",
                )
            }
            Intent::Market {
                qty,
                side,
                fill_price: _,
                comment,
            } => {
                let comment = Self::sanitize_comment(comment.or_else(|| fallback_comment.clone()));
                (
                    alor_protocol::CommandAction::Market(alor_protocol::MarketOrder {
                        qty,
                        side,
                        comment,
                    }),
                    if side == alor_protocol::Side::Buy {
                        3
                    } else {
                        4
                    },
                    "market",
                )
            }
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
            Intent::CreateStopLimit {
                side,
                qty,
                trigger_price,
                price,
                condition,
                stop_end_unix_time,
                comment,
                instrument_group,
                check_duplicates,
            } => {
                let resolved_stop_end = self
                    .compute_intraday_stop_end_utc(created_ts_utc)
                    .unwrap_or(stop_end_unix_time);
                (
                    alor_protocol::CommandAction::CreateStopLimit(
                        alor_protocol::CreateStopLimitOrder {
                            side,
                            qty,
                            trigger_price,
                            price,
                            condition,
                            stop_end_unix_time: resolved_stop_end,
                            comment: Self::sanitize_comment(
                                comment.or_else(|| fallback_comment.clone()),
                            ),
                            instrument_group,
                            check_duplicates,
                        },
                    ),
                    5,
                    "create_stop_limit",
                )
            }
            Intent::DeleteStopLimit {
                order_id,
                side,
                check_duplicates,
            } => (
                alor_protocol::CommandAction::DeleteStopLimit(
                    alor_protocol::DeleteStopLimitOrder {
                        order_id,
                        side,
                        check_duplicates,
                    },
                ),
                6,
                "delete_stop_limit",
            ),
        };
        let request_id = if action_name == "market" {
            let side = match &action {
                alor_protocol::CommandAction::Market(market) => market.side,
                _ => unreachable!("market action_name must map to market command action"),
            };
            crate::deterministic_market_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                created_ts_utc,
                side,
            )
        } else {
            crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                action_name,
                created_ts_utc,
                seq,
            )
        };
        alor_protocol::OrderCommand {
            request_id,
            created_ts_utc,
            strategy_id: ctx.strategy_id.clone(),
            portfolio: ctx.portfolio.clone(),
            exchange: ctx.exchange.clone(),
            symbol: ctx.symbol.clone(),
            action,
            intent_class: Some(intent_class),
            ttl_ms: None,
        }
    }

    fn sanitize_comment(raw: Option<String>) -> Option<String> {
        let comment = raw?
            .chars()
            .filter(|c| c.is_ascii() && *c != '\n' && *c != '\r')
            .take(100)
            .collect::<String>();
        if comment.trim().is_empty() {
            return None;
        }
        Some(comment)
    }

    fn resolve_intent_class(ctx: &StrategyCtx, intent: &Intent) -> alor_protocol::IntentClass {
        if let Some(explicit) = intent.explicit_class() {
            return explicit;
        }
        match intent.base_intent() {
            Intent::Cancel { .. } => alor_protocol::IntentClass::CancelCleanup,
            Intent::Replace { .. } => alor_protocol::IntentClass::Entry,
            Intent::Place { .. } => alor_protocol::IntentClass::Entry,
            Intent::CreateStopLimit { .. } => alor_protocol::IntentClass::ProtectiveRepair,
            Intent::DeleteStopLimit { .. } => alor_protocol::IntentClass::CancelCleanup,
            Intent::Market { side, .. } => {
                let qty = ctx.position_qty.unwrap_or(0.0);
                if (qty > 0.0 && *side == alor_protocol::Side::Sell)
                    || (qty < 0.0 && *side == alor_protocol::Side::Buy)
                {
                    alor_protocol::IntentClass::Exit
                } else {
                    alor_protocol::IntentClass::Entry
                }
            }
            Intent::Classified { intent, .. } => Self::resolve_intent_class(ctx, intent),
        }
    }

    fn guard_allows_intent_when_blocked(
        &self,
        intent_class: alor_protocol::IntentClass,
        has_open_position: bool,
    ) -> bool {
        if !has_open_position {
            return false;
        }
        matches!(
            intent_class,
            alor_protocol::IntentClass::Exit
                | alor_protocol::IntentClass::CancelCleanup
                | alor_protocol::IntentClass::ProtectiveRepair
        )
    }

    fn normalize_event_ts(&mut self, event_ts_utc: i64) -> i64 {
        let candidate = if event_ts_utc > 0 {
            event_ts_utc
        } else {
            self.strategy_now_ts_utc
        };
        self.strategy_now_ts_utc = self.strategy_now_ts_utc.max(candidate);
        self.strategy_now_ts_utc
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
                    self.refresh_health_snapshot();
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
        self.log_guard_transition_if_needed(&decision)?;
        self.log_guard_decision_if_due(&decision)
    }

    fn evaluate_guard_decision(&self) -> LiveGuardDecision {
        let mut decision = evaluate_live_guard(
            self.config.trade_mode,
            self.config.allow_live_orders,
            &self.live_guard,
            self.metrics.bars_read_total > 0,
            self.metrics.bars_stream_xlen_last.unwrap_or(0) > 0,
            chrono::Utc::now().timestamp(),
            self.config.gateway_health_stale_sec,
            self.config.require_gateway_ready,
        );
        decision.reasons.extend(self.bootstrap_state.reasons());
        decision.allowed = decision.reasons.is_empty();
        decision
    }

    fn log_guard_decision_if_due(&mut self, decision: &LiveGuardDecision) -> Result<()> {
        let now_ts_utc = chrono::Utc::now().timestamp();
        if decision.allowed {
            return Ok(());
        }
        let Some(last) = &self.metrics.last_live_guard else {
            return Ok(());
        };
        let reasons_changed = last.reasons != decision.reasons;
        if reasons_changed {
            return Ok(());
        }
        let elapsed = now_ts_utc.saturating_sub(self.metrics.last_live_guard_log_ts_utc);
        let period = i64::try_from(self.config.still_blocked_log_period_sec).unwrap_or(i64::MAX);
        if elapsed < period {
            return Ok(());
        }
        tracing::debug!(
            reasons = ?decision.reasons,
            still_blocked_for_sec = elapsed,
            "live_guard_still_blocked"
        );
        self.metrics.last_live_guard_log_ts_utc = now_ts_utc;
        Ok(())
    }

    fn log_guard_transition_if_needed(&mut self, decision: &LiveGuardDecision) -> Result<()> {
        let phase = self
            .live_guard
            .health
            .as_ref()
            .map(|health| health.gateway_phase)
            .unwrap_or_default();
        let waiting_next_bar = decision
            .reasons
            .iter()
            .any(|reason| reason == "waiting_for_next_bar_after_restart");
        if waiting_next_bar && !self.metrics.last_waiting_next_bar_active {
            let tf_sec = bars_tf_seconds(&self.config.streams.bars).unwrap_or(60);
            info!(
                tf_sec,
                "waiting_for_next_bar_after_restart: will allow trading after next live bar (this may take up to one bar interval)"
            );
        }
        self.metrics.last_waiting_next_bar_active = waiting_next_bar;

        let mut reasons = decision.reasons.clone();
        reasons.sort();
        let status = if decision.allowed {
            "ALLOWED"
        } else {
            "BLOCKED"
        };
        let snapshot = GuardSnapshot { status, reasons };
        let now_ts_utc = chrono::Utc::now().timestamp();

        if let Some(prev) = &self.metrics.last_live_guard {
            if prev != &snapshot {
                info!(
                    from = prev.status,
                    to = snapshot.status,
                    reasons_before = ?prev.reasons,
                    reasons_after = ?snapshot.reasons,
                    phase = ?phase,
                    "live_guard_changed"
                );
                self.metrics.last_live_guard_log_ts_utc = now_ts_utc;
            }
        } else {
            let to = snapshot.status;
            if to == "BLOCKED" {
                info!(to, reasons = ?snapshot.reasons, phase = ?phase, "live_guard_changed");
                self.metrics.last_live_guard_log_ts_utc = now_ts_utc;
            }
        }

        self.metrics.last_live_guard = Some(snapshot);
        Ok(())
    }

    fn intent_action_name(&self, intent: &Intent) -> &'static str {
        match intent.base_intent() {
            Intent::Place { .. } => "place",
            Intent::Market { .. } => "market",
            Intent::Cancel { .. } => "cancel",
            Intent::Replace { .. } => "replace",
            Intent::CreateStopLimit { .. } => "create_stop_limit",
            Intent::DeleteStopLimit { .. } => "delete_stop_limit",
            Intent::Classified { .. } => unreachable!("base_intent flattens classified variant"),
        }
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

fn bars_tf_seconds(stream: &str) -> Option<u64> {
    let tf = stream.split('.').next_back()?;
    let (value, unit) = tf.split_at(tf.len().saturating_sub(1));
    let amount: u64 = value.parse().ok()?;
    match unit {
        "s" => Some(amount),
        "m" => Some(amount.saturating_mul(60)),
        "h" => Some(amount.saturating_mul(60 * 60)),
        "d" => Some(amount.saturating_mul(60 * 60 * 24)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReadConfig, ReplayConfig, StrategyConfig, StrategyKind, StreamNames, TrimConfig};
    use alor_types::TradingPeriods;

    fn test_runtime(trade_mode: TradeMode) -> StrategyRuntime {
        let mut strategy = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        strategy.strategy_id = "limit_cancel".to_string();
        strategy.symbol = "SBER".to_string();
        if let Some(settings) = strategy.limit_cancel_mut() {
            settings.max_wait_bars_for_ack = 1;
        }

        let config = RuntimeConfig {
            redis_url: "redis://127.0.0.1/".to_string(),
            source: "test".to_string(),
            portfolio: "demo".to_string(),
            exchange: "alor".to_string(),
            streams: StreamNames {
                bars: "bars".to_string(),
                orders: "orders".to_string(),
                trades: "trades".to_string(),
                positions: "positions".to_string(),
                commands: "commands".to_string(),
                acks: "acks".to_string(),
                snapshots: None,
                health: None,
                dlq_prefix: "dlq".to_string(),
                runtime_state: "runtime-state".to_string(),
            },
            consumer_group: "group".to_string(),
            consumer_name: "consumer".to_string(),
            trade_mode,
            allow_live_orders: true,
            allow_paper_orders: true,
            guard_log_interval_ms: 1_000,
            still_blocked_log_period_sec: 60,
            gateway_health_stale_sec: 20,
            require_gateway_ready: true,
            bootstrap_dump: false,
            health: crate::HealthServerConfig {
                enabled: false,
                listen_addr: "127.0.0.1:0".to_string(),
                expose_metrics: false,
            },
            read: ReadConfig {
                block_ms: 100,
                claim_idle_ms: 100,
                claim_batch: 1,
                poll_interval_ms: 10,
            },
            trim: TrimConfig {
                bars: 10,
                orders: 10,
                trades: 10,
                positions: 10,
                commands: 10,
                acks: 10,
                health: 10,
                runtime_state: 10,
            },
            strategy,
            paper: PaperConfig {
                enabled: false,
                output: PaperOutput::Stdout,
                execution_mode: PaperExecutionMode::LiveOnly,
                file_path: "paper.jsonl".to_string(),
                trades_csv: "trades.csv".to_string(),
                summary_json: "summary.json".to_string(),
                append: false,
            },
            backtest: BacktestConfig {
                enabled: false,
                trade_log: "backtest.log".to_string(),
                trades_csv: "trades.csv".to_string(),
                summary_json: "summary.json".to_string(),
                append: false,
            },
            replay: ReplayConfig {
                enabled: false,
                bars_csv_path: None,
                reference_trades_csv_path: None,
                output_dir: "replay_out".to_string(),
                price_tolerance: 1e-8,
                strict_dedup: true,
            },
            reset_state_on_start: false,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(StrategyRuntime::new(config))
            .unwrap()
    }

    #[derive(Default)]
    struct CallbackSpyStrategy {
        state: StrategyState,
    }

    impl Strategy for CallbackSpyStrategy {
        fn on_bar(&mut self, _ctx: &StrategyCtx, _bar: &BarEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &alor_protocol::CommandAck) -> Vec<Intent> {
            Vec::new()
        }

        fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_stop_order(&mut self, _ctx: &StrategyCtx, ord: &StopOrderEvent) -> Vec<Intent> {
            self.state = StrategyState::Blocked {
                reason: format!("spy_stop_order:{}", ord.stop_order_id),
                last_bar_ts: ord.ts_utc,
            };
            Vec::new()
        }

        fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_bootstrap_snapshot(
            &mut self,
            _ctx: &StrategyCtx,
            snapshot: &BootstrapSnapshot,
        ) -> Vec<Intent> {
            let ts = snapshot.snapshot_ts_utc.unwrap_or_default();
            self.state = StrategyState::Blocked {
                reason: "spy_bootstrap_snapshot".to_string(),
                last_bar_ts: ts,
            };
            Vec::new()
        }

        fn on_runtime_state_restored(
            &mut self,
            ctx: &StrategyCtx,
            state: &RuntimeStateRestored,
        ) -> Vec<Intent> {
            self.state = StrategyState::Blocked {
                reason: format!(
                    "spy_runtime_state_restored:{}:{}",
                    state.known_order_ids.len(),
                    state.pending_requests.len()
                ),
                last_bar_ts: ctx.last_bar_ts().unwrap_or_default(),
            };
            Vec::new()
        }

        fn state(&self) -> &StrategyState {
            &self.state
        }

        fn set_state(&mut self, state: StrategyState) {
            self.state = state;
        }
    }

    #[derive(Default)]
    struct HookSpyStrategy {
        state: StrategyState,
        pending_requests: Vec<uuid::Uuid>,
        tag: Option<String>,
        risk: crate::strategy_host::StrategyExitRiskStatus,
    }

    impl Strategy for HookSpyStrategy {
        fn on_bar(&mut self, _ctx: &StrategyCtx, _bar: &BarEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &alor_protocol::CommandAck) -> Vec<Intent> {
            Vec::new()
        }

        fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn pending_request_ids(&self) -> Vec<uuid::Uuid> {
            self.pending_requests.clone()
        }

        fn intent_comment_tag(
            &self,
            _ctx: &StrategyCtx,
            _created_ts_utc: i64,
            _intent_class: alor_protocol::IntentClass,
        ) -> Option<String> {
            self.tag.clone()
        }

        fn exit_risk_status(
            &self,
            _has_open_position: bool,
        ) -> crate::strategy_host::StrategyExitRiskStatus {
            self.risk.clone()
        }

        fn state(&self) -> &StrategyState {
            &self.state
        }

        fn set_state(&mut self, state: StrategyState) {
            self.state = state;
        }
    }

    #[derive(Default)]
    struct WindowClosedExitSpyStrategy {
        state: StrategyState,
    }

    impl Strategy for WindowClosedExitSpyStrategy {
        fn on_bar(&mut self, _ctx: &StrategyCtx, _bar: &BarEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &alor_protocol::CommandAck) -> Vec<Intent> {
            self.state = StrategyState::Blocked {
                reason: format!(
                    "ack:{}:{}",
                    ack.request_id,
                    ack.error_code.as_deref().unwrap_or("none")
                ),
                last_bar_ts: ack.processed_ts_utc,
            };
            Vec::new()
        }

        fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
            Vec::new()
        }

        fn state(&self) -> &StrategyState {
            &self.state
        }

        fn set_state(&mut self, state: StrategyState) {
            self.state = state;
        }
    }

    #[test]
    fn restore_pending_requests_uses_strategy_hook() {
        let mut runtime = test_runtime(TradeMode::Live);
        let request_id = uuid::Uuid::new_v4();
        runtime.strategy = Box::new(HookSpyStrategy {
            pending_requests: vec![request_id],
            ..HookSpyStrategy::default()
        });
        runtime.our_request_ids.clear();

        runtime.restore_pending_requests();

        assert!(runtime.our_request_ids.contains(&request_id));
    }

    #[test]
    fn intent_to_command_uses_strategy_comment_tag_hook() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.strategy = Box::new(HookSpyStrategy {
            tag: Some("hook-tag".to_string()),
            ..HookSpyStrategy::default()
        });
        let ctx = runtime.strategy_ctx();
        let command = runtime.intent_to_command(
            &ctx,
            1_700_000_000,
            Intent::Market {
                qty: 1.0,
                side: alor_protocol::Side::Buy,
                fill_price: None,
                comment: None,
            },
            alor_protocol::IntentClass::Entry,
        );

        match command.action {
            alor_protocol::CommandAction::Market(order) => {
                assert_eq!(order.comment.as_deref(), Some("hook-tag"));
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn refresh_health_snapshot_uses_strategy_exit_risk_hook() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.strategy = Box::new(HookSpyStrategy {
            risk: crate::strategy_host::StrategyExitRiskStatus {
                phase_override: Some("RiskOverride".to_string()),
                exit_recovery_active: true,
                operator_intervention_required: true,
                open_risk_position_unflattened: true,
            },
            ..HookSpyStrategy::default()
        });
        runtime.state.positions.insert(
            runtime.config.strategy.symbol.clone(),
            PositionEvent {
                symbol: runtime.config.strategy.symbol.clone(),
                qty: 1.0,
                existing: true,
                avg_price: 100.0,
                ts_utc: 1_700_000_000,
            },
        );

        runtime.refresh_health_snapshot();
        let snapshot = runtime.health_snapshot.read();

        assert_eq!(snapshot.runtime_phase, "RiskOverride");
        assert!(snapshot.exit_recovery_active);
        assert!(snapshot.operator_intervention_required);
        assert!(snapshot.open_risk_position_unflattened);
        assert!(!snapshot.readiness);
    }

    #[test]
    fn runtime_loads_minimal_strategy_capabilities_from_registry() {
        let limit_runtime = test_runtime(TradeMode::Live);
        assert_eq!(
            limit_runtime.strategy_capabilities,
            StrategyCapabilities::default()
        );

        let mut hybrid_config = limit_runtime.config.clone();
        hybrid_config
            .strategy
            .set_kind(StrategyKind::HybridIntraday);
        hybrid_config.strategy.strategy_id = "hybrid_intraday".to_string();

        let hybrid_runtime = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(StrategyRuntime::new(hybrid_config))
            .unwrap();

        assert_eq!(
            hybrid_runtime.strategy_capabilities,
            StrategyCapabilities {
                uses_bootstrap_snapshot: true,
                uses_runtime_state_restore: true,
                uses_history_warmup: true,
                uses_stop_orders: true,
            }
        );

        let mut alor_config = limit_runtime.config.clone();
        alor_config
            .strategy
            .set_kind(StrategyKind::AlorUsdrubfHybrid);
        alor_config.strategy.strategy_id = "alor_usdrubf_hybrid_v1".to_string();

        let alor_runtime = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(StrategyRuntime::new(alor_config))
            .unwrap();

        assert_eq!(
            alor_runtime.strategy_capabilities,
            StrategyCapabilities {
                uses_bootstrap_snapshot: true,
                uses_runtime_state_restore: true,
                uses_history_warmup: true,
                uses_stop_orders: false,
            }
        );
    }

    #[test]
    fn alor_skeleton_lifecycle_callbacks_are_wired_in_runtime() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime
            .config
            .strategy
            .set_kind(StrategyKind::AlorUsdrubfHybrid);
        runtime.config.strategy.strategy_id = "alor_usdrubf_hybrid_v1".to_string();
        let strategy_config =
            crate::strategy_adapters::AlorUsdrubfHybridAdapter::from_strategy_config(
                &runtime.config.strategy,
            )
            .expect("alor skeleton config");
        runtime.strategy = Box::new(
            crate::strategies::alor_usdrubf_hybrid::AlorUsdrubfHybridStrategy::new(strategy_config),
        );
        runtime.strategy_capabilities = StrategyCapabilities {
            uses_bootstrap_snapshot: true,
            uses_runtime_state_restore: true,
            uses_history_warmup: true,
            uses_stop_orders: false,
        };

        let snapshot_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        runtime.bootstrap_snapshot = Some(BootstrapSnapshot {
            positions_strategy: HashMap::new(),
            working_orders_strategy: HashMap::new(),
            working_stop_orders_strategy: HashMap::new(),
            snapshot_ts_utc: Some(snapshot_ts),
        });
        runtime
            .state
            .last_processed_bar_ts
            .insert(runtime.config.strategy.symbol.clone(), snapshot_ts);

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.notify_bootstrap_snapshot())
            .unwrap();
        assert!(matches!(
            &runtime.state.strategy_state,
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                bootstrap_seen,
                ..
            } if lifecycle_stage == "bootstrapped" && *bootstrap_seen
        ));

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.notify_runtime_state_restored())
            .unwrap();
        assert!(matches!(
            &runtime.state.strategy_state,
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                runtime_state_restored,
                ..
            } if lifecycle_stage == "runtime_state_restored" && *runtime_state_restored
        ));

        let stop_order = StopOrderEvent {
            stop_order_id: "alor-stop-1".to_string(),
            exchange_order_id: Some(1),
            symbol: runtime.config.strategy.symbol.clone(),
            status: "working".to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            filled: 0.0,
            stop_price: 100.0,
            price: 99.9,
            existing: false,
            comment: None,
            end_time: None,
            ts_utc: snapshot_ts + 1,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.handle_stop_order(
                "orders".to_string(),
                "1-0".to_string(),
                stop_order,
            ))
            .unwrap();
        assert!(matches!(
            &runtime.state.strategy_state,
            StrategyState::AlorUsdrubfHybrid { lifecycle_stage, .. }
                if lifecycle_stage == "runtime_state_restored"
        ));
    }

    #[test]
    fn bootstrap_snapshot_callback_is_skipped_without_capability() {
        let mut runtime = test_runtime(TradeMode::Live);
        let symbol = runtime.config.strategy.symbol.clone();
        let snapshot_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        runtime.strategy = Box::new(CallbackSpyStrategy::default());
        runtime.state.strategy_state = StrategyState::Idle;
        runtime
            .state
            .last_processed_bar_ts
            .insert(symbol, snapshot_ts - 60);
        runtime.bootstrap_snapshot = Some(BootstrapSnapshot {
            positions_strategy: HashMap::new(),
            working_orders_strategy: HashMap::new(),
            working_stop_orders_strategy: HashMap::new(),
            snapshot_ts_utc: Some(snapshot_ts),
        });
        runtime.strategy_capabilities = StrategyCapabilities::default();

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.notify_bootstrap_snapshot())
            .unwrap();
        assert!(matches!(runtime.state.strategy_state, StrategyState::Idle));

        runtime.strategy_capabilities.uses_bootstrap_snapshot = true;
        let _ = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.notify_bootstrap_snapshot());
        assert!(matches!(
            &runtime.state.strategy_state,
            StrategyState::Blocked { reason, last_bar_ts }
                if reason == "spy_bootstrap_snapshot" && *last_bar_ts == snapshot_ts
        ));
    }

    #[test]
    fn runtime_state_restore_callback_is_skipped_without_capability() {
        let mut runtime = test_runtime(TradeMode::Live);
        let symbol = runtime.config.strategy.symbol.clone();
        let last_bar_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 1, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        runtime.strategy = Box::new(CallbackSpyStrategy::default());
        runtime.state.strategy_state = StrategyState::Idle;
        runtime
            .state
            .last_processed_bar_ts
            .insert(symbol, last_bar_ts);
        runtime.our_order_ids.insert(42);
        runtime.our_request_ids.insert(uuid::Uuid::nil());
        runtime.strategy_capabilities = StrategyCapabilities::default();

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.notify_runtime_state_restored())
            .unwrap();
        assert!(matches!(runtime.state.strategy_state, StrategyState::Idle));

        runtime.strategy_capabilities.uses_runtime_state_restore = true;
        let _ = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.notify_runtime_state_restored());
        assert!(matches!(
            &runtime.state.strategy_state,
            StrategyState::Blocked { reason, last_bar_ts: state_last_bar_ts }
                if reason == "spy_runtime_state_restored:1:1"
                    && *state_last_bar_ts == last_bar_ts
        ));
    }

    #[test]
    fn stop_order_is_persisted_without_stop_order_callback_capability() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.strategy = Box::new(CallbackSpyStrategy::default());
        runtime.state.strategy_state = StrategyState::Idle;
        runtime.strategy_capabilities = StrategyCapabilities::default();

        let stop_order = StopOrderEvent {
            stop_order_id: "spy-stop-1".to_string(),
            exchange_order_id: Some(10),
            symbol: runtime.config.strategy.symbol.clone(),
            status: "working".to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            filled: 0.0,
            stop_price: 123.45,
            price: 123.40,
            existing: false,
            comment: Some("test".to_string()),
            end_time: None,
            ts_utc: chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
                .unwrap()
                .and_hms_opt(10, 2, 0)
                .unwrap()
                .and_utc()
                .timestamp(),
        };

        let _ = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.handle_stop_order(
                "stop-orders".to_string(),
                "1-0".to_string(),
                stop_order.clone(),
            ));

        assert_eq!(
            runtime.state.stop_orders.get(&stop_order.stop_order_id),
            Some(&stop_order)
        );
        assert!(matches!(runtime.state.strategy_state, StrategyState::Idle));
    }

    #[test]
    fn intent_gating_uses_created_ts_not_last_bar_ts() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.config.strategy.max_silence_bars_sec = 0;
        runtime.config.strategy.timezone_offset_hours = 3;
        runtime.config.strategy.trading_periods = Some(TradingPeriods {
            session_start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            session_end: chrono::NaiveTime::from_hms_opt(23, 49, 0).unwrap(),
            break_start_1: chrono::NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            break_end_1: chrono::NaiveTime::from_hms_opt(14, 5, 0).unwrap(),
            break_start_2: chrono::NaiveTime::from_hms_opt(18, 50, 0).unwrap(),
            break_end_2: chrono::NaiveTime::from_hms_opt(19, 5, 0).unwrap(),
            weekends_off: true,
            timezone_offset_hours: 0,
        });

        // Last bar in open local window: 13:00 local == 10:00 UTC.
        let last_bar_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        runtime
            .state
            .last_processed_bar_ts
            .insert(runtime.config.strategy.symbol.clone(), last_bar_ts);

        // Created ts in Break2 local window: 18:55 local == 15:55 UTC.
        let created_ts_utc = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(15, 55, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        let ctx = runtime.strategy_ctx();
        assert!(!runtime.trading_window_allows_order(
            &ctx,
            created_ts_utc,
            alor_protocol::IntentClass::Entry
        ));
        assert!(runtime.trading_window_allows_order(
            &ctx,
            created_ts_utc,
            alor_protocol::IntentClass::Exit
        ));
    }

    #[test]
    fn closed_window_exit_is_deferred_before_emit() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.strategy = Box::new(WindowClosedExitSpyStrategy::default());
        runtime.config.strategy.max_silence_bars_sec = 0;
        runtime.config.strategy.timezone_offset_hours = 3;
        runtime.config.strategy.trading_periods = Some(TradingPeriods {
            session_start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            session_end: chrono::NaiveTime::from_hms_opt(23, 49, 0).unwrap(),
            break_start_1: chrono::NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            break_end_1: chrono::NaiveTime::from_hms_opt(14, 5, 0).unwrap(),
            break_start_2: chrono::NaiveTime::from_hms_opt(18, 50, 0).unwrap(),
            break_end_2: chrono::NaiveTime::from_hms_opt(19, 5, 0).unwrap(),
            weekends_off: true,
            timezone_offset_hours: 0,
        });
        let last_bar_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        runtime
            .state
            .last_processed_bar_ts
            .insert(runtime.config.strategy.symbol.clone(), last_bar_ts);
        runtime.state.positions.insert(
            runtime.config.strategy.symbol.clone(),
            PositionEvent {
                symbol: runtime.config.strategy.symbol.clone(),
                qty: 1.0,
                existing: true,
                avg_price: 100.0,
                ts_utc: last_bar_ts,
            },
        );
        runtime.live_guard.health = Some(HealthEvent {
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            readiness: true,
            ws_connected: true,
            cws_authorized: true,
            scheduler_state: Some("Break2".to_string()),
            last_event_ts: Utc::now().timestamp(),
        });

        let created_ts_utc = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(15, 55, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let ctx =
            runtime.strategy_ctx_with_last_bar_and_event_ts(Some(last_bar_ts), created_ts_utc);
        let intent = Intent::Place {
            price: 100.5,
            qty: 1.0,
            side: alor_protocol::Side::Sell,
            comment: None,
        }
        .with_class(alor_protocol::IntentClass::Exit);
        let expected_request_id = runtime
            .intent_to_command(
                &ctx,
                created_ts_utc,
                intent.clone(),
                alor_protocol::IntentClass::Exit,
            )
            .request_id;

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.apply_intents(
                &ctx,
                created_ts_utc,
                vec![intent],
                runtime.state.strategy_state.clone(),
            ))
            .unwrap();

        assert!(!runtime.our_request_ids.contains(&expected_request_id));
        assert!(matches!(
            &runtime.state.strategy_state,
            StrategyState::Blocked { reason, last_bar_ts: state_last_bar_ts }
                if reason == &format!("ack:{expected_request_id}:trading_window_closed")
                    && *state_last_bar_ts == created_ts_utc
        ));
    }

    #[test]
    fn create_stop_limit_uses_session_close_plus_buffer_for_stop_end() {
        let runtime = test_runtime(TradeMode::Live);
        let created_ts_utc = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let expected = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(20, 50, 0)
            .unwrap()
            .and_utc()
            .timestamp()
            + STOP_END_BUFFER_SEC_DEFAULT;
        let ctx = runtime.strategy_ctx();
        let cmd = runtime.intent_to_command(
            &ctx,
            created_ts_utc,
            Intent::CreateStopLimit {
                side: alor_protocol::Side::Sell,
                qty: 1.0,
                trigger_price: 100.0,
                price: 99.5,
                condition: alor_protocol::StopLimitCondition::LessOrEqual,
                stop_end_unix_time: created_ts_utc.saturating_add(86_400),
                comment: None,
                instrument_group: None,
                check_duplicates: Some(true),
            },
            alor_protocol::IntentClass::ProtectiveRepair,
        );
        match cmd.action {
            alor_protocol::CommandAction::CreateStopLimit(payload) => {
                assert_eq!(payload.stop_end_unix_time, expected);
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn silence_gap_blocks_entry_on_first_gap_bar() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.config.strategy.max_silence_bars_sec = 60;
        runtime.config.strategy.timezone_offset_hours = 3;
        runtime.config.strategy.trading_periods = Some(TradingPeriods {
            session_start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            session_end: chrono::NaiveTime::from_hms_opt(23, 49, 0).unwrap(),
            break_start_1: chrono::NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            break_end_1: chrono::NaiveTime::from_hms_opt(14, 5, 0).unwrap(),
            break_start_2: chrono::NaiveTime::from_hms_opt(18, 50, 0).unwrap(),
            break_end_2: chrono::NaiveTime::from_hms_opt(19, 5, 0).unwrap(),
            weekends_off: true,
            timezone_offset_hours: 0,
        });
        let prev_bar = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let first_gap_bar = chrono::NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 10, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        let ctx_prev = runtime.strategy_ctx_with_last_bar(Some(prev_bar));
        assert!(!runtime.trading_window_allows_order(
            &ctx_prev,
            first_gap_bar,
            alor_protocol::IntentClass::Entry
        ));
        assert!(runtime.trading_window_allows_order(
            &ctx_prev,
            first_gap_bar,
            alor_protocol::IntentClass::Exit
        ));
    }

    #[test]
    fn market_intent_classified_as_exit_against_open_position() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.state.positions.insert(
            runtime.config.strategy.symbol.clone(),
            PositionEvent {
                symbol: runtime.config.strategy.symbol.clone(),
                qty: 1.0,
                existing: true,
                avg_price: 100.0,
                ts_utc: 1,
            },
        );
        let ctx = runtime.strategy_ctx();
        let intent = Intent::Market {
            qty: 1.0,
            side: alor_protocol::Side::Sell,
            fill_price: None,
            comment: None,
        };
        assert_eq!(
            StrategyRuntime::resolve_intent_class(&ctx, &intent),
            alor_protocol::IntentClass::Exit
        );
    }

    #[test]
    fn replace_requires_explicit_class_for_protective_repair() {
        let runtime = test_runtime(TradeMode::Live);
        let ctx = runtime.strategy_ctx();

        let legacy_replace = Intent::Replace {
            order_id: 1,
            new_price: 100.0,
            new_qty: 1.0,
        };
        assert_eq!(
            StrategyRuntime::resolve_intent_class(&ctx, &legacy_replace),
            alor_protocol::IntentClass::Entry
        );

        let protective_replace =
            legacy_replace.with_class(alor_protocol::IntentClass::ProtectiveRepair);
        assert_eq!(
            StrategyRuntime::resolve_intent_class(&ctx, &protective_replace),
            alor_protocol::IntentClass::ProtectiveRepair
        );
    }

    #[test]
    fn guard_close_only_path_allows_exit_cancel_repair_only_with_open_position() {
        let runtime = test_runtime(TradeMode::Live);
        assert!(!runtime.guard_allows_intent_when_blocked(alor_protocol::IntentClass::Exit, false));
        assert!(runtime.guard_allows_intent_when_blocked(alor_protocol::IntentClass::Exit, true));
        assert!(runtime
            .guard_allows_intent_when_blocked(alor_protocol::IntentClass::CancelCleanup, true));
        assert!(runtime
            .guard_allows_intent_when_blocked(alor_protocol::IntentClass::ProtectiveRepair, true));
        assert!(!runtime.guard_allows_intent_when_blocked(alor_protocol::IntentClass::Entry, true));
    }

    #[test]
    fn normalize_event_ts_is_monotonic_and_bootstrap_safe() {
        let mut runtime = test_runtime(TradeMode::Live);
        assert_eq!(runtime.normalize_event_ts(0), 0);
        assert_eq!(runtime.normalize_event_ts(-1), 0);
        assert_eq!(runtime.normalize_event_ts(100), 100);
        assert_eq!(runtime.normalize_event_ts(90), 100);
        assert_eq!(runtime.normalize_event_ts(0), 100);
    }
    #[test]
    fn runtime_scheduler_snapshot_is_unconfigured_when_periods_missing() {
        let runtime = test_runtime(TradeMode::Paper);
        runtime.refresh_health_snapshot();
        let snapshot = runtime.health_snapshot.read().clone();

        assert_eq!(snapshot.scheduler_state, "Unconfigured");
        assert_eq!(snapshot.now_local, "unknown");
        assert_eq!(
            snapshot.scheduler_note.as_deref(),
            Some("trading_periods missing")
        );
    }

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

    #[test]
    fn dedup_trade_cursor_blocks_duplicates() {
        let mut runtime = test_runtime(TradeMode::Live);
        runtime.state.last_trade_ts = Some(100);
        runtime.state.seen_trade_ids = vec!["trade-1".to_string()];

        let trade = TradeEvent {
            trade_id: "trade-1".to_string(),
            order_id: 1,
            symbol: "SBER".to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            price: 100.0,
            commission: 0.1,
            existing: true,
            ts_utc: 100,
        };
        assert!(!runtime.should_process_trade(&trade));

        let trade_new = TradeEvent {
            trade_id: "trade-2".to_string(),
            ts_utc: 101,
            ..trade
        };
        assert!(runtime.should_process_trade(&trade_new));
    }

    #[test]
    fn order_vs_exec_price_uses_trade_record() {
        let mut runtime = test_runtime(TradeMode::Live);
        let order = OrderEvent {
            order_id: 1,
            request_id: None,
            symbol: "SBER".to_string(),
            status: "filled".to_string(),
            side: "buy".to_string(),
            order_type: "market".to_string(),
            qty: 1.0,
            filled: 1.0,
            price: 100.0,
            existing: false,
            comment: None,
            ts_utc: 10,
        };
        runtime.update_ledger_from_order(&order).unwrap();
        assert!(runtime.ledger.trades().is_empty());

        runtime.ledger.record_fill(TradeRecord {
            ts_utc: 11,
            order_id: 1,
            symbol: "SBER".to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            price: 110.0,
            commission: 0.0,
            owned: true,
        });
        assert_eq!(runtime.ledger.trades()[0].price, 110.0);
    }

    #[test]
    fn parity_side_mapping_accepts_long_short_vs_buy_sell() {
        let runtime = test_runtime(TradeMode::Paper);
        let runtime_rows = vec![crate::trade_ledger::ClosedTradeRecord {
            entry_ts_utc: 100,
            exit_ts_utc: 160,
            symbol: "SBER".to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            entry_price: 10.0,
            exit_price: 11.0,
            commission_total: 0.0,
            pnl_gross: 1.0,
            pnl_net: 1.0,
        }];
        let reference_rows = vec![ReplayReferenceTradeRow {
            entry_time: chrono::DateTime::from_timestamp(100, 0)
                .unwrap()
                .to_rfc3339(),
            exit_time: chrono::DateTime::from_timestamp(160, 0)
                .unwrap()
                .to_rfc3339(),
            direction: "Long".to_string(),
            size: 1,
            entry_price: 10.0,
            exit_price: 11.0,
            reason: "tp".to_string(),
            pnl: 1.0,
        }];

        let report =
            StrategyRuntime::build_replay_parity_report(&runtime_rows, &reference_rows, 1e-8);
        assert_eq!(report.status, "pass");
        assert_eq!(report.matched_trades, 1);
        drop(runtime);
    }

    #[test]
    fn simulated_market_with_fill_price_fills_immediately_on_current_bar() {
        let mut runtime = test_runtime(TradeMode::Paper);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 100,
            close: 10.5,
            o: 10.0,
            h: 10.6,
            l: 9.9,
            v: 0.0,
            origin: DataOrigin::Replay,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime
                .simulate_intents(
                    &bar,
                    vec![Intent::Market {
                        qty: 1.0,
                        side: alor_protocol::Side::Buy,
                        fill_price: Some(123.45),
                        comment: None,
                    }],
                )
                .await
                .unwrap();

            assert!(runtime.sim_orders.is_empty());
            assert_eq!(runtime.ledger.trades().len(), 1);
            assert_eq!(runtime.ledger.trades()[0].ts_utc, 100);
            assert_eq!(runtime.ledger.trades()[0].price, 123.45);
            assert_eq!(runtime.ledger.orders_total(), 1);
            assert_eq!(runtime.ledger.order(1).unwrap().status, "filled");
            assert_eq!(runtime.ledger.order(1).unwrap().price, 123.45);
        });
    }

    #[test]
    fn simulated_market_without_fill_price_fills_on_next_bar_open() {
        let mut runtime = test_runtime(TradeMode::Paper);
        let bar1 = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 100,
            close: 10.5,
            o: 10.0,
            h: 10.6,
            l: 9.9,
            v: 0.0,
            origin: DataOrigin::Replay,
        };
        let bar2 = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 160,
            close: 11.2,
            o: 11.0,
            h: 11.3,
            l: 10.8,
            v: 0.0,
            origin: DataOrigin::Replay,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime
                .simulate_intents(
                    &bar1,
                    vec![Intent::Market {
                        qty: 1.0,
                        side: alor_protocol::Side::Buy,
                        fill_price: None,
                        comment: None,
                    }],
                )
                .await
                .unwrap();

            assert_eq!(runtime.sim_orders.len(), 1);
            assert_eq!(runtime.ledger.orders_total(), 1);
            assert_eq!(runtime.ledger.order(1).unwrap().status, "working");
            assert_eq!(runtime.ledger.order(1).unwrap().price, 0.0);

            runtime.simulate_fills(&bar1).await.unwrap();
            assert!(runtime.ledger.trades().is_empty());

            runtime.simulate_fills(&bar2).await.unwrap();
            assert_eq!(runtime.ledger.trades().len(), 1);
            assert_eq!(runtime.ledger.trades()[0].ts_utc, 160);
            assert_eq!(runtime.ledger.trades()[0].price, 11.0);
            assert_eq!(runtime.ledger.order(1).unwrap().status, "filled");
            assert_eq!(runtime.ledger.order(1).unwrap().price, 11.0);
        });
    }

    #[test]
    fn paper_exit_from_flat_is_dropped() {
        let mut runtime = test_runtime(TradeMode::Paper);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 100,
            close: 10.0,
            o: 10.0,
            h: 10.0,
            l: 10.0,
            v: 0.0,
            origin: DataOrigin::Live,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime
                .simulate_intents(
                    &bar,
                    vec![Intent::Market {
                        qty: 1.0,
                        side: alor_protocol::Side::Sell,
                        fill_price: None,
                        comment: None,
                    }
                    .with_class(alor_protocol::IntentClass::Exit)],
                )
                .await
                .unwrap();
        });
        assert!(runtime.sim_orders.is_empty());
    }

    #[test]
    fn paper_exit_wrong_side_is_dropped() {
        let mut runtime = test_runtime(TradeMode::Paper);
        runtime.state.positions.insert(
            "SBER".to_string(),
            PositionEvent {
                symbol: "SBER".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1,
            },
        );
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 100,
            close: 10.0,
            o: 10.0,
            h: 10.0,
            l: 10.0,
            v: 0.0,
            origin: DataOrigin::Live,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime
                .simulate_intents(
                    &bar,
                    vec![Intent::Market {
                        qty: 1.0,
                        side: alor_protocol::Side::Sell,
                        fill_price: None,
                        comment: None,
                    }
                    .with_class(alor_protocol::IntentClass::Exit)],
                )
                .await
                .unwrap();
        });
        assert!(runtime.sim_orders.is_empty());
    }

    #[test]
    fn paper_exit_qty_is_clamped_to_position_abs_on_create() {
        let mut runtime = test_runtime(TradeMode::Paper);
        runtime.state.positions.insert(
            "SBER".to_string(),
            PositionEvent {
                symbol: "SBER".to_string(),
                qty: 1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1,
            },
        );
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 100,
            close: 10.0,
            o: 10.0,
            h: 10.0,
            l: 10.0,
            v: 0.0,
            origin: DataOrigin::Live,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime
                .simulate_intents(
                    &bar,
                    vec![Intent::Market {
                        qty: 5.0,
                        side: alor_protocol::Side::Sell,
                        fill_price: None,
                        comment: None,
                    }
                    .with_class(alor_protocol::IntentClass::Exit)],
                )
                .await
                .unwrap();
        });
        assert_eq!(runtime.sim_orders.len(), 1);
        assert!((runtime.sim_orders[0].qty - 1.0).abs() <= f64::EPSILON);
        assert_eq!(
            runtime.sim_orders[0].intent_class,
            Some(alor_protocol::IntentClass::Exit)
        );
    }

    #[test]
    fn queued_duplicate_exit_sells_cannot_flip_long_position() {
        let mut runtime = test_runtime(TradeMode::Paper);
        runtime.state.positions.insert(
            "SBER".to_string(),
            PositionEvent {
                symbol: "SBER".to_string(),
                qty: 1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1,
            },
        );
        runtime.sim_orders.push(SimOrder {
            order_id: 1,
            symbol: "SBER".to_string(),
            side: "sell".to_string(),
            intent_class: Some(alor_protocol::IntentClass::Exit),
            order_type: SimOrderType::Market,
            qty: 1.0,
            price: None,
            created_bar_ts: 100,
        });
        runtime.sim_orders.push(SimOrder {
            order_id: 2,
            symbol: "SBER".to_string(),
            side: "sell".to_string(),
            intent_class: Some(alor_protocol::IntentClass::Exit),
            order_type: SimOrderType::Market,
            qty: 1.0,
            price: None,
            created_bar_ts: 100,
        });
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 160,
            close: 11.2,
            o: 11.0,
            h: 11.3,
            l: 10.8,
            v: 0.0,
            origin: DataOrigin::Replay,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime.simulate_fills(&bar).await.unwrap();
        });

        let pos_qty = runtime
            .state
            .positions
            .get("SBER")
            .map(|p| p.qty)
            .unwrap_or(0.0);
        assert!(pos_qty >= 0.0, "position must not flip short");
        assert!(pos_qty.abs() <= f64::EPSILON, "position must close to flat");
        assert_eq!(runtime.ledger.order(1).unwrap().status, "filled");
        assert_eq!(runtime.ledger.order(2).unwrap().status, "dropped");
        assert!((runtime.ledger.order(2).unwrap().filled - 0.0).abs() <= f64::EPSILON);
    }

    #[test]
    fn queued_duplicate_exit_buys_cannot_flip_short_position() {
        let mut runtime = test_runtime(TradeMode::Paper);
        runtime.state.positions.insert(
            "SBER".to_string(),
            PositionEvent {
                symbol: "SBER".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1,
            },
        );
        runtime.sim_orders.push(SimOrder {
            order_id: 1,
            symbol: "SBER".to_string(),
            side: "buy".to_string(),
            intent_class: Some(alor_protocol::IntentClass::Exit),
            order_type: SimOrderType::Market,
            qty: 1.0,
            price: None,
            created_bar_ts: 100,
        });
        runtime.sim_orders.push(SimOrder {
            order_id: 2,
            symbol: "SBER".to_string(),
            side: "buy".to_string(),
            intent_class: Some(alor_protocol::IntentClass::Exit),
            order_type: SimOrderType::Market,
            qty: 1.0,
            price: None,
            created_bar_ts: 100,
        });
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 160,
            close: 11.2,
            o: 11.0,
            h: 11.3,
            l: 10.8,
            v: 0.0,
            origin: DataOrigin::Replay,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime.simulate_fills(&bar).await.unwrap();
        });

        let pos_qty = runtime
            .state
            .positions
            .get("SBER")
            .map(|p| p.qty)
            .unwrap_or(0.0);
        assert!(pos_qty <= 0.0, "position must not flip long");
        assert!(pos_qty.abs() <= f64::EPSILON, "position must close to flat");
    }

    #[test]
    fn paper_execution_mode_controls_history_advance() {
        let mut runtime = test_runtime(TradeMode::Paper);
        runtime.config.paper.execution_mode = PaperExecutionMode::LiveOnly;
        assert!(!runtime.can_advance_paper_execution(DataOrigin::History));
        assert!(runtime.can_advance_paper_execution(DataOrigin::Live));

        runtime.config.paper.execution_mode = PaperExecutionMode::HistorySim;
        assert!(runtime.can_advance_paper_execution(DataOrigin::History));
        assert!(runtime.can_advance_paper_execution(DataOrigin::Live));
    }

    #[test]
    fn paper_ignores_external_position_stream_events() {
        let mut runtime = test_runtime(TradeMode::Paper);
        let position = PositionEvent {
            symbol: runtime.config.strategy.symbol.clone(),
            qty: 3.0,
            existing: true,
            avg_price: 100.0,
            ts_utc: 1_700_000_000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            runtime
                .handle_position(
                    runtime.config.streams.positions.clone(),
                    "1-0".to_string(),
                    position,
                )
                .await
                .unwrap();
        });
        assert!(runtime.state.positions.is_empty());
    }

    #[test]
    fn stop_order_working_status_table() {
        let runtime = test_runtime(TradeMode::Live);
        let mk = |status: &str| StopOrderEvent {
            stop_order_id: "s1".to_string(),
            exchange_order_id: None,
            symbol: "SBER".to_string(),
            status: status.to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            filled: 0.0,
            stop_price: 100.0,
            price: 101.0,
            existing: false,
            comment: None,
            end_time: None,
            ts_utc: 1,
        };

        assert!(runtime.is_working_stop_order(&mk("working")));
        assert!(runtime.is_working_stop_order(&mk("new")));
        assert!(!runtime.is_working_stop_order(&mk("canceled")));
        assert!(!runtime.is_working_stop_order(&mk("rejected")));
        assert!(!runtime.is_working_stop_order(&mk("expired")));
        assert!(!runtime.is_working_stop_order(&mk("executed")));
        assert!(!runtime.is_working_stop_order(&mk("triggered")));
        assert!(!runtime.is_working_stop_order(&mk("done")));
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
        let intent = match intent {
            Intent::Classified { intent, .. } => *intent,
            other => other,
        };
        match intent {
            Intent::Place {
                price, qty, side, ..
            } => Self {
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
                ..
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
            Intent::CreateStopLimit { .. } => Self {
                ts_utc: created_ts_utc,
                strategy_id: config.strategy.strategy_id.clone(),
                portfolio: config.portfolio.clone(),
                symbol: config.strategy.symbol.clone(),
                action: "create_stop_limit".to_string(),
                qty: None,
                price: None,
                side: None,
                order_id: None,
                new_price: None,
                new_qty: None,
            },
            Intent::DeleteStopLimit { .. } => Self {
                ts_utc: created_ts_utc,
                strategy_id: config.strategy.strategy_id.clone(),
                portfolio: config.portfolio.clone(),
                symbol: config.strategy.symbol.clone(),
                action: "delete_stop_limit".to_string(),
                qty: None,
                price: None,
                side: None,
                order_id: None,
                new_price: None,
                new_qty: None,
            },
            Intent::Classified { .. } => unreachable!("classified intents are flattened above"),
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
    ensure_parent_dir(path)?;
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
    ensure_parent_dir(path)?;
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

fn ensure_parent_dir(path: &str) -> Result<()> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn runtime_orders_mode(config: &RuntimeConfig) -> &'static str {
    if config.replay.enabled {
        "replay"
    } else {
        match config.trade_mode {
            TradeMode::Live => "live",
            TradeMode::Paper => "paper",
            TradeMode::Backtest => "replay",
        }
    }
}
