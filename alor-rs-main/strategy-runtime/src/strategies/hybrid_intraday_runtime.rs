use std::collections::HashSet;

use alor_protocol::{AckStatus, CommandAck, IntentClass, Side as OrderSide, StopLimitCondition};
use chrono::{
    Datelike, Duration as ChronoDuration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Utc, Weekday,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::state::StrategyState;
use crate::strategies::hybrid_intraday::{
    Action, BreakoutEodMode, EntrySignal, EntryStyle, High180MrConfig, High180MrEngine,
    High180Open, HybridOrchestrator, HybridOrchestratorConfig, IntradayBreakoutConfig,
    IntradayBreakoutEngine, MeanReversionConfig, MeanReversionEngine, Owner, ReasonCode, Side,
};
use crate::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle;
use crate::strategy_host::{
    BarEvent, DataOrigin, Intent, OrderEvent, PositionEvent, RiskGateRuntimeState,
    RiskGateSessionFinalization, StopOrderEvent, Strategy, StrategyCtx,
};

const RISK_GATE_MAKER_COST_POINTS: f64 = 0.1;

#[derive(Debug, Clone)]
pub struct HybridIntradayRuntimeConfig {
    pub symbol: String,
    pub profile: HybridIntradayProfile,
    pub mr_variant: MeanReversionVariant,
    pub mr_gate_policy: MrGatePolicy,
    pub risk_gate_mode: RiskGateMode,
    pub risk_gate_seed_file: Option<String>,
    pub risk_gate_ledger_key: Option<String>,
    pub model_session_start_time: Option<NaiveTime>,
    pub model_session_end_time: Option<NaiveTime>,
    pub qty: f64,
    pub live_order_style: MarketBuyAndCloseLiveOrderStyle,
    pub tick_size: f64,
    pub marketable_limit_offset_ticks: i64,
    pub timezone_offset_hours: i32,
    pub session_close_hour: u32,
    pub session_close_minute: u32,
    pub weekends_off: bool,
    pub stop_end_buffer_sec: u64,
    pub repair_deadline_sec: u64,
    pub sl_escalate_timeout_sec: u64,
    pub max_repair_retries: u32,
    pub repair_backoff_base_sec: u64,
    pub repair_backoff_max_sec: u64,
    pub pending_timeout_sec: u64,
    pub partial_entry_fill_timeout_ms: u64,
    pub mr_config: MeanReversionConfig,
    pub breakout_config: IntradayBreakoutConfig,
    pub orchestrator_config: HybridOrchestratorConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HybridIntradayProfile {
    BaselineRuntimeHybrid,
    ImoexfPrimaryRiskgateHigh180Lb120,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeanReversionVariant {
    ClassicPrevDayRange,
    High180,
    Author41BoundaryShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrGatePolicy {
    Disabled,
    ShadowPnlLb120Positive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskGateMode {
    Disabled,
    BootstrapFromSeed,
    NormalAppend,
    RebuildFromHistory,
    ShadowOnly,
    Enforced,
}

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    owner: Owner,
    side: Side,
    cycle_id: [u8; 10],
    reason: crate::strategies::hybrid_intraday::ReasonCode,
    entry_style: EntryStyle,
    stop_price: Option<f64>,
    take_price: Option<f64>,
    target_qty: f64,
    partial_started_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct PendingExit {
    owner: Owner,
    reason: crate::strategies::hybrid_intraday::ReasonCode,
}

#[derive(Debug, Clone, Copy)]
struct ShadowHigh180Position {
    side: Side,
    entry_ts_utc: i64,
    entry_price: f64,
}

#[derive(Debug, Clone)]
struct DeferredEntry {
    signal: crate::strategies::hybrid_intraday::EntrySignal,
    cycle_id: [u8; 10],
    deferred_ts_utc: i64,
    original_request_id: Uuid,
}

#[derive(Debug, Clone, Copy)]
struct DeferredExit {
    owner: Owner,
    reason: crate::strategies::hybrid_intraday::ReasonCode,
    cycle_id: [u8; 10],
    deferred_ts_utc: i64,
    original_request_id: Uuid,
}

#[derive(Debug, Clone)]
struct HybridTag {
    sid: String,
    cycle: String,
    owner: Option<Owner>,
    role: Option<TagRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagRole {
    Entry,
    Tp,
    Sl,
    Exit,
    Cancel,
}

#[derive(Debug)]
pub struct HybridIntradayRuntimeStrategy {
    config: HybridIntradayRuntimeConfig,
    orchestrator: HybridOrchestrator,
    high180_mr: High180MrEngine,
    risk_gate_shadow_mr: High180MrEngine,
    state: StrategyState,
    last_processed_bar_ts: Option<i64>,
    last_position_qty: f64,
    current_owner: Option<Owner>,
    current_side: Option<Side>,
    pending_entry: Option<PendingEntry>,
    pending_entry_request_id: Option<Uuid>,
    pending_entry_created_ts_utc: Option<i64>,
    deferred_entry: Option<DeferredEntry>,
    pending_exit: Option<PendingExit>,
    pending_exit_request_id: Option<Uuid>,
    pending_exit_created_ts_utc: Option<i64>,
    deferred_exit: Option<DeferredExit>,
    pending_tp_request_id: Option<Uuid>,
    pending_tp_created_ts_utc: Option<i64>,
    pending_sl_request_id: Option<Uuid>,
    pending_sl_created_ts_utc: Option<i64>,
    tp_order_id: Option<i64>,
    sl_stop_order_id: Option<String>,
    sl_exchange_order_id: Option<i64>,
    sl_triggered_ts: Option<i64>,
    mr_take_price: Option<f64>,
    mr_stop_price: Option<f64>,
    repair_deadline_ts: Option<i64>,
    next_repair_at_ts: Option<i64>,
    repair_backoff_level: u32,
    repair_attempts: u32,
    bracket_terminal_reconcile_started_ms: Option<i64>,
    active_cycle_id: Option<[u8; 10]>,
    safe_mode_close_only: bool,
    safe_mode_reason: Option<String>,
    next_cycle_seq: u32,
    last_bar_close: Option<f64>,
    prev_day_close: Option<f64>,
    last_day_local: Option<NaiveDate>,
    current_day_high: Option<f64>,
    current_day_low: Option<f64>,
    prev_day_range: Option<f64>,
    entry_ready: bool,
    last_warmup_log: Option<bool>,
    working_orders: HashSet<i64>,
    working_stop_orders: HashSet<String>,
    cleanup_stop_retry_attempts: u32,
    startup_live_replay_boundary_ts_utc: Option<i64>,
    startup_replay_suppressed_bars: u32,
    risk_gate_shadow_session_date: Option<NaiveDate>,
    risk_gate_shadow_pnl_points: f64,
    risk_gate_shadow_trade_count: u32,
    risk_gate_shadow_position: Option<ShadowHigh180Position>,
    risk_gate_shadow_open: Option<High180Open>,
    pending_risk_gate_finalizations: Vec<RiskGateSessionFinalization>,
    risk_gate_mr_enabled_current_session: Option<bool>,
    risk_gate_rolling_sum_lb120: Option<f64>,
    risk_gate_last_finalized_session_date: Option<NaiveDate>,
    risk_gate_ledger_rows_count: usize,
}

impl HybridIntradayRuntimeStrategy {
    const MIN_MR_TAKE_DISTANCE_TICKS: f64 = 2.0;
    const MAX_CLEANUP_STOP_RETRIES: u32 = 3;
    const BRACKET_TERMINAL_RECONCILE_GRACE_MS: i64 = 3_000;
    const AUTHOR41_SHORT_K: f64 = 0.16;
    const AUTHOR41_SHORT_TAKE_K: f64 = 0.02;
    const AUTHOR41_SHORT_STOP_K: f64 = 0.58;
    const AUTHOR41_MIN_REL_RANGE: f64 = 0.005;
    const AUTHOR41_MAX_REL_RANGE: f64 = 0.075;
    const AUTHOR41_ENTRY_END_HOUR: u32 = 12;
    const AUTHOR41_ENTRY_END_MINUTE: u32 = 0;
    const AUTHOR41_TIME_EXIT_HOUR: u32 = 20;
    const AUTHOR41_TIME_EXIT_MINUTE: u32 = 0;

    fn action_debug_label(action: &Action) -> String {
        match action {
            Action::SubmitEntry(entry) => format!(
                "submit_entry owner={:?} side={:?} style={:?} reason={:?} stop={:?} take={:?}",
                entry.owner,
                entry.side,
                entry.entry_style,
                entry.reason,
                entry.stop_price,
                entry.take_price
            ),
            Action::SubmitExit { owner, reason } => {
                format!("submit_exit owner={owner:?} reason={reason:?}")
            }
            Action::ArmOvernightExit {
                owner,
                reason,
                armed_date,
                exit_time,
            } => format!(
                "arm_overnight_exit owner={owner:?} reason={reason:?} armed_date={armed_date} exit_time={exit_time}"
            ),
        }
    }

    pub fn new(config: HybridIntradayRuntimeConfig) -> Self {
        let mr = MeanReversionEngine::new(config.mr_config);
        let br = IntradayBreakoutEngine::new(config.breakout_config);
        let orchestrator = HybridOrchestrator::new(mr, br, config.orchestrator_config);
        let high180_mr = High180MrEngine::new(High180MrConfig::default());
        let risk_gate_shadow_mr = High180MrEngine::new(High180MrConfig::default());
        Self {
            config,
            orchestrator,
            high180_mr,
            risk_gate_shadow_mr,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
            last_position_qty: 0.0,
            current_owner: None,
            current_side: None,
            pending_entry: None,
            pending_entry_request_id: None,
            pending_entry_created_ts_utc: None,
            deferred_entry: None,
            pending_exit: None,
            pending_exit_request_id: None,
            pending_exit_created_ts_utc: None,
            deferred_exit: None,
            pending_tp_request_id: None,
            pending_tp_created_ts_utc: None,
            pending_sl_request_id: None,
            pending_sl_created_ts_utc: None,
            tp_order_id: None,
            sl_stop_order_id: None,
            sl_exchange_order_id: None,
            sl_triggered_ts: None,
            mr_take_price: None,
            mr_stop_price: None,
            repair_deadline_ts: None,
            next_repair_at_ts: None,
            repair_backoff_level: 0,
            repair_attempts: 0,
            bracket_terminal_reconcile_started_ms: None,
            active_cycle_id: None,
            safe_mode_close_only: false,
            safe_mode_reason: None,
            next_cycle_seq: 0,
            last_bar_close: None,
            prev_day_close: None,
            last_day_local: None,
            current_day_high: None,
            current_day_low: None,
            prev_day_range: None,
            entry_ready: false,
            last_warmup_log: None,
            working_orders: HashSet::new(),
            working_stop_orders: HashSet::new(),
            cleanup_stop_retry_attempts: 0,
            startup_live_replay_boundary_ts_utc: None,
            startup_replay_suppressed_bars: 0,
            risk_gate_shadow_session_date: None,
            risk_gate_shadow_pnl_points: 0.0,
            risk_gate_shadow_trade_count: 0,
            risk_gate_shadow_position: None,
            risk_gate_shadow_open: None,
            pending_risk_gate_finalizations: Vec::new(),
            risk_gate_mr_enabled_current_session: None,
            risk_gate_rolling_sum_lb120: None,
            risk_gate_last_finalized_session_date: None,
            risk_gate_ledger_rows_count: 0,
        }
    }

    fn utc_to_local_naive(&self, ts_utc: i64) -> Option<NaiveDateTime> {
        let offset = FixedOffset::east_opt(self.config.timezone_offset_hours.saturating_mul(3600))?;
        chrono::DateTime::from_timestamp(ts_utc, 0)
            .map(|dt| dt.with_timezone(&offset).naive_local())
    }

    fn has_live_orders(&self) -> bool {
        self.pending_entry.is_some()
            || self.pending_entry_request_id.is_some()
            || self.pending_exit_request_id.is_some()
            || self.pending_tp_request_id.is_some()
            || self.pending_sl_request_id.is_some()
            || !self.working_orders.is_empty()
            || !self.working_stop_orders.is_empty()
    }

    fn can_execute_now(&self, ctx: &StrategyCtx, is_live_bar: bool) -> bool {
        match ctx.trade_mode {
            crate::TradeMode::Live => {
                ctx.allow_live_orders
                    && ctx.gateway_phase == crate::live_guard::GatewayPhase::LiveReady
                    && is_live_bar
            }
            crate::TradeMode::Paper => match ctx.paper_execution_mode {
                crate::PaperExecutionMode::HistorySim => true,
                crate::PaperExecutionMode::LiveOnly => is_live_bar,
            },
            crate::TradeMode::Backtest => true,
        }
    }

    fn can_emit_now(&self, ctx: &StrategyCtx, is_live_bar: bool) -> bool {
        match ctx.trade_mode {
            crate::TradeMode::Live => self.can_execute_now(ctx, is_live_bar),
            crate::TradeMode::Paper | crate::TradeMode::Backtest => true,
        }
    }

    fn suppress_weekend_signal_generation(&self, dt_local: NaiveDateTime) -> bool {
        self.config.weekends_off && matches!(dt_local.weekday(), Weekday::Sat | Weekday::Sun)
    }

    fn suppress_non_model_session_bar(&self, dt_local: NaiveDateTime) -> Option<&'static str> {
        if let Some(start) = self.config.model_session_start_time {
            if dt_local.time() < start {
                return Some("before_model_session_start");
            }
        }
        if let Some(end) = self.config.model_session_end_time {
            if dt_local.time() > end {
                return Some("after_model_session_end");
            }
        }
        None
    }

    fn uses_high180_mr(&self) -> bool {
        self.config.mr_variant == MeanReversionVariant::High180
    }

    fn uses_author41_boundary_short_mr(&self) -> bool {
        self.config.mr_variant == MeanReversionVariant::Author41BoundaryShort
    }

    fn uses_mr_override(&self) -> bool {
        self.uses_high180_mr() || self.uses_author41_boundary_short_mr()
    }

    fn round_to_tick(price: f64, tick_size: f64) -> f64 {
        if tick_size <= 0.0 || !price.is_finite() {
            return price;
        }
        (price / tick_size).round() * tick_size
    }

    fn mr_take_distance_ticks_after_rounding(
        &self,
        reference_price: f64,
        signal: &EntrySignal,
    ) -> Option<f64> {
        if signal.owner != Owner::MeanReversion || signal.entry_style != EntryStyle::Bracket {
            return None;
        }
        let take_price = signal.take_price?;
        let tick_size = self.config.tick_size;
        if tick_size <= 0.0 || !reference_price.is_finite() || !take_price.is_finite() {
            return None;
        }
        let entry = Self::round_to_tick(reference_price, tick_size);
        let take = Self::round_to_tick(take_price, tick_size);
        let distance_points = match signal.side {
            Side::Long => take - entry,
            Side::Short => entry - take,
        };
        Some(distance_points / tick_size)
    }

    fn filter_near_zero_mr_bracket_entry(
        &self,
        dt_local: NaiveDateTime,
        reference_price: f64,
        signal: Option<EntrySignal>,
    ) -> Option<EntrySignal> {
        let signal = signal?;
        let Some(distance_ticks) =
            self.mr_take_distance_ticks_after_rounding(reference_price, &signal)
        else {
            return Some(signal);
        };
        if distance_ticks >= Self::MIN_MR_TAKE_DISTANCE_TICKS {
            return Some(signal);
        }
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "mr_entry_suppressed",
            reason = "take_too_close_after_rounding",
            owner = ?signal.owner,
            side = ?signal.side,
            dt_local = %dt_local,
            reference_price,
            take_price = signal.take_price,
            rounded_reference_price = Self::round_to_tick(reference_price, self.config.tick_size),
            rounded_take_price = signal
                .take_price
                .map(|price| Self::round_to_tick(price, self.config.tick_size)),
            distance_ticks,
            min_distance_ticks = Self::MIN_MR_TAKE_DISTANCE_TICKS,
        );
        None
    }

    fn mr_gate_allows_current_session(&self) -> bool {
        match self.config.mr_gate_policy {
            MrGatePolicy::Disabled => true,
            MrGatePolicy::ShadowPnlLb120Positive => match self.config.risk_gate_mode {
                RiskGateMode::Enforced => {
                    self.risk_gate_mr_enabled_current_session.unwrap_or(false)
                }
                _ => true,
            },
        }
    }

    fn risk_gate_shadow_enabled(&self) -> bool {
        self.uses_high180_mr() && self.config.mr_gate_policy == MrGatePolicy::ShadowPnlLb120Positive
    }

    fn finalize_risk_gate_shadow_session(&mut self, session_date: NaiveDate) {
        if matches!(session_date.weekday(), Weekday::Sat | Weekday::Sun) {
            return;
        }
        if self
            .pending_risk_gate_finalizations
            .iter()
            .any(|finalization| finalization.session_date == session_date)
        {
            return;
        }
        self.pending_risk_gate_finalizations
            .push(RiskGateSessionFinalization {
                session_date,
                shadow_pnl_points: self.risk_gate_shadow_pnl_points,
                shadow_trade_count: self.risk_gate_shadow_trade_count,
            });
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "risk_gate_shadow_session_finalized",
            session_date = %Self::format_local_day(session_date),
            shadow_pnl_points = self.risk_gate_shadow_pnl_points,
            shadow_trade_count = self.risk_gate_shadow_trade_count,
        );
    }

    fn update_risk_gate_shadow(
        &mut self,
        dt_local: NaiveDateTime,
        ts_utc: i64,
        high: f64,
        low: f64,
        close: f64,
        close_prev: f64,
        day_range_prev: f64,
    ) {
        if !self.risk_gate_shadow_enabled() || close == 0.0 {
            return;
        }

        let session_date = dt_local.date();
        match self.risk_gate_shadow_session_date {
            None => {
                self.risk_gate_shadow_session_date = Some(session_date);
            }
            Some(prev_date) if prev_date != session_date => {
                self.finalize_risk_gate_shadow_session(prev_date);
                self.risk_gate_shadow_session_date = Some(session_date);
                self.risk_gate_shadow_pnl_points = 0.0;
                self.risk_gate_shadow_trade_count = 0;
                self.risk_gate_shadow_position = None;
                self.risk_gate_shadow_open = None;
            }
            Some(_) => {}
        }

        self.risk_gate_shadow_mr.on_bar(dt_local, high, low);
        let mut closed_this_bar = false;
        if let (Some(position), Some(open)) =
            (self.risk_gate_shadow_position, self.risk_gate_shadow_open)
        {
            let entry_dt = self
                .utc_to_local_naive(position.entry_ts_utc)
                .unwrap_or(dt_local);
            if let Some((_reason, exit_price)) = self.risk_gate_shadow_mr.evaluate_exit(
                &open,
                entry_dt,
                position.side,
                dt_local,
                close,
            ) {
                let pnl = match position.side {
                    Side::Long => exit_price - position.entry_price,
                    Side::Short => position.entry_price - exit_price,
                } - RISK_GATE_MAKER_COST_POINTS;
                self.risk_gate_shadow_pnl_points += pnl;
                self.risk_gate_shadow_trade_count =
                    self.risk_gate_shadow_trade_count.saturating_add(1);
                self.risk_gate_shadow_position = None;
                self.risk_gate_shadow_open = None;
                closed_this_bar = true;
                debug!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    action = "risk_gate_shadow_mr_exit",
                    dt_local = %dt_local,
                    side = ?position.side,
                    entry_price = position.entry_price,
                    exit_price,
                    pnl_points = pnl,
                    session_pnl_points = self.risk_gate_shadow_pnl_points,
                );
            }
        }

        if self.risk_gate_shadow_position.is_none() && !closed_this_bar {
            if let Some(signal) =
                self.risk_gate_shadow_mr
                    .evaluate_entry(dt_local, close, close_prev, day_range_prev)
            {
                self.risk_gate_shadow_position = Some(ShadowHigh180Position {
                    side: signal.side,
                    entry_ts_utc: ts_utc,
                    entry_price: close,
                });
                self.risk_gate_shadow_open = Some(High180Open::from_signal(
                    &signal,
                    High180MrConfig::default(),
                ));
                debug!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    action = "risk_gate_shadow_mr_entry",
                    dt_local = %dt_local,
                    side = ?signal.side,
                    entry_price = close,
                    target_price = signal.target_price,
                    stop_price = signal.stop_price,
                );
            }
        }
    }

    fn high180_entry_signal(
        &self,
        dt_local: NaiveDateTime,
        close: f64,
        close_prev: f64,
        day_range_prev: f64,
    ) -> Option<EntrySignal> {
        self.high180_mr
            .evaluate_entry(dt_local, close, close_prev, day_range_prev)
            .map(|signal| EntrySignal {
                owner: Owner::MeanReversion,
                side: signal.side,
                entry_style: EntryStyle::Bracket,
                reason: signal.reason,
                stop_price: Some(signal.stop_price),
                take_price: Some(signal.target_price),
            })
    }

    fn high180_exit_reason(&self, dt_local: NaiveDateTime) -> Option<ReasonCode> {
        if self.current_owner != Some(Owner::MeanReversion) {
            return None;
        }
        let entry_ts = self
            .active_cycle_id
            .and_then(Self::cycle_ts_utc)
            .and_then(|ts| self.utc_to_local_naive(ts))?;
        let max_hold = self.high180_mr.config().max_hold;
        if dt_local - entry_ts >= max_hold {
            return Some(ReasonCode::MeanRevTimeCutoff);
        }
        None
    }

    fn author41_boundary_short_entry_signal(
        &self,
        dt_local: NaiveDateTime,
        close: f64,
        close_prev: f64,
        day_range_prev: f64,
    ) -> Option<EntrySignal> {
        let entry_end = NaiveTime::from_hms_opt(
            Self::AUTHOR41_ENTRY_END_HOUR,
            Self::AUTHOR41_ENTRY_END_MINUTE,
            0,
        )
        .unwrap_or(NaiveTime::MIN);
        if dt_local.time() > entry_end {
            return None;
        }
        if close_prev <= 0.0 || day_range_prev <= 0.0 || !close.is_finite() {
            return None;
        }

        // We keep a close-based normalization to stay deterministic in live runtime
        // where only previous close/range are guaranteed at this stage.
        let rel_range = day_range_prev / close_prev;
        if !(Self::AUTHOR41_MIN_REL_RANGE < rel_range && rel_range < Self::AUTHOR41_MAX_REL_RANGE) {
            return None;
        }

        let upper = close_prev + Self::AUTHOR41_SHORT_K * day_range_prev;
        let short_signal = close > close_prev && close < upper;
        if !short_signal {
            return None;
        }

        Some(EntrySignal {
            owner: Owner::MeanReversion,
            side: Side::Short,
            entry_style: EntryStyle::Bracket,
            reason: ReasonCode::MorningMeanReversionShort,
            stop_price: Some(close_prev + Self::AUTHOR41_SHORT_STOP_K * day_range_prev),
            take_price: Some(close_prev - Self::AUTHOR41_SHORT_TAKE_K * day_range_prev),
        })
    }

    fn author41_boundary_short_exit_reason(&self, dt_local: NaiveDateTime) -> Option<ReasonCode> {
        if self.current_owner != Some(Owner::MeanReversion) {
            return None;
        }
        let time_exit = NaiveTime::from_hms_opt(
            Self::AUTHOR41_TIME_EXIT_HOUR,
            Self::AUTHOR41_TIME_EXIT_MINUTE,
            0,
        )
        .unwrap_or(NaiveTime::MIN);
        (dt_local.time() >= time_exit).then_some(ReasonCode::MeanRevTimeCutoff)
    }

    fn breakout_eod_time(&self) -> NaiveTime {
        NaiveTime::from_hms_opt(23, 30, 0).unwrap_or(NaiveTime::MIN)
    }

    fn active_cycle_local_day(&self) -> Option<NaiveDate> {
        let cycle_id = self.active_cycle_id?;
        let cycle_ts = Self::cycle_ts_utc(cycle_id)?;
        self.utc_to_local_naive(cycle_ts).map(|dt| dt.date())
    }

    fn has_breakout_exit_action(actions: &[Action]) -> bool {
        actions.iter().any(|action| {
            matches!(
                action,
                Action::SubmitExit {
                    owner: Owner::IntradayBreakout,
                    ..
                }
            )
        })
    }

    fn append_breakout_no_overnight_guard(
        &mut self,
        actions: &mut Vec<Action>,
        dt_local: NaiveDateTime,
        previous_day_local: Option<NaiveDate>,
        has_open_position: bool,
    ) {
        if !has_open_position
            || self.current_owner != Some(Owner::IntradayBreakout)
            || !matches!(
                self.config.orchestrator_config.breakout_eod_mode,
                BreakoutEodMode::SameDay
            )
            || self.pending_exit_request_id.is_some()
            || self.deferred_exit.is_some()
            || Self::has_breakout_exit_action(actions)
        {
            return;
        }

        let current_day = dt_local.date();
        let active_cycle_day = self
            .active_cycle_local_day()
            .filter(|cycle_day| *cycle_day <= current_day);
        let reference_day =
            active_cycle_day.or_else(|| previous_day_local.filter(|day| *day <= current_day));
        let crossed_regular_day = reference_day.is_some_and(|day| current_day > day);
        let after_same_day_eod = dt_local.time() >= self.breakout_eod_time();
        if !crossed_regular_day && !after_same_day_eod {
            return;
        }

        let guard_reason = if crossed_regular_day {
            "overnight_carry_rescue"
        } else {
            "late_same_day_eod_guard"
        };
        warn!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "breakout_no_overnight_guard_exit",
            reason = guard_reason,
            dt_local = %dt_local,
            reference_day = reference_day.map(Self::format_local_day),
            active_cycle_day = active_cycle_day.map(Self::format_local_day),
            previous_day_local = previous_day_local.map(Self::format_local_day),
            eod_time = %self.breakout_eod_time(),
            cycle_id = self.active_cycle_id.map(|v| Self::format_cycle_id(&v)),
            "BO position is still open after the same-day EOD contour; forcing exit"
        );
        actions.push(Action::SubmitExit {
            owner: Owner::IntradayBreakout,
            reason: ReasonCode::BreakoutEodExit,
        });
    }

    fn startup_live_replay_tolerance_sec(&self) -> i64 {
        self.pending_timeout_sec().max(60)
    }

    fn clear_startup_replay_guard(&mut self) {
        self.startup_live_replay_boundary_ts_utc = None;
        self.startup_replay_suppressed_bars = 0;
    }

    fn arm_startup_replay_guard(&mut self, ctx: &StrategyCtx) {
        if ctx.trade_mode != crate::TradeMode::Live || !ctx.allow_live_orders {
            self.clear_startup_replay_guard();
            return;
        }
        let boundary = ctx
            .now_ts_utc()
            .saturating_sub(self.startup_live_replay_tolerance_sec());
        self.startup_live_replay_boundary_ts_utc = Some(boundary);
        self.startup_replay_suppressed_bars = 0;
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            boundary_ts_utc = boundary,
            tolerance_sec = self.startup_live_replay_tolerance_sec(),
            "hybrid_startup_replay_guard_armed"
        );
    }

    fn bar_input(
        &self,
        dt_local: NaiveDateTime,
        bar: &BarEvent,
        close_prev: f64,
        day_range_prev: f64,
        has_open_position: bool,
    ) -> crate::strategies::hybrid_intraday::orchestrator::BarInput {
        crate::strategies::hybrid_intraday::orchestrator::BarInput {
            dt: dt_local,
            open: bar.o,
            high: bar.h,
            low: bar.l,
            close: bar.close,
            close_prev,
            day_range_prev,
            has_open_position,
            has_live_orders: self.has_live_orders(),
        }
    }

    fn use_live_marketable_limit(&self, ctx: &StrategyCtx) -> bool {
        ctx.trade_mode == crate::TradeMode::Live
            && matches!(
                self.config.live_order_style,
                MarketBuyAndCloseLiveOrderStyle::MarketableLimit
            )
    }

    fn live_marketable_price_from_reference(&self, side: OrderSide, reference_price: f64) -> f64 {
        let reference_price = if reference_price > 0.0 {
            reference_price
        } else if self.config.tick_size > 0.0 {
            self.config.tick_size
        } else {
            1.0
        };
        if self.config.tick_size <= 0.0 {
            return reference_price;
        }
        // Keep one extra tick of aggressiveness so gateway normalization does not
        // turn a marketable limit back into a passive one.
        let aggressive_ticks = self.config.marketable_limit_offset_ticks.max(0) + 1;
        let shift = aggressive_ticks as f64 * self.config.tick_size;
        match side {
            OrderSide::Buy => reference_price + shift,
            OrderSide::Sell => reference_price - shift,
        }
    }

    fn live_reference_price(&self) -> f64 {
        self.last_bar_close
            .filter(|price| *price > 0.0)
            .or_else(|| self.mr_take_price.filter(|price| *price > 0.0))
            .or_else(|| self.mr_stop_price.filter(|price| *price > 0.0))
            .unwrap_or({
                if self.config.tick_size > 0.0 {
                    self.config.tick_size
                } else {
                    1.0
                }
            })
    }

    fn live_request_id(&self, ctx: &StrategyCtx, created_ts_utc: i64, side: OrderSide) -> Uuid {
        if self.use_live_marketable_limit(ctx) {
            crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                "place",
                created_ts_utc,
                0,
            )
        } else {
            crate::deterministic_market_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                created_ts_utc,
                side,
            )
        }
    }

    fn effective_created_ts_utc(&self, ctx: &StrategyCtx, fallback_ts_utc: i64) -> i64 {
        ctx.event_ts_utc().max(fallback_ts_utc)
    }

    fn log_request_id_skew_if_pending(
        &self,
        kind: &'static str,
        expected_request_id: Option<Uuid>,
        pending_created_ts_utc: Option<i64>,
        ack: &CommandAck,
    ) {
        let Some(expected_request_id) = expected_request_id else {
            return;
        };
        if expected_request_id == ack.request_id {
            return;
        }
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "pending_request_id_skew_detected",
            kind,
            strategy_pending_request_id = %expected_request_id,
            ack_request_id = %ack.request_id,
            pending_created_ts_utc,
            ack_processed_ts_utc = ack.processed_ts_utc,
            error_code = ?ack.error_code,
            error_msg = ?ack.error_msg,
        );
    }

    fn build_live_entry_exit_intent(
        &self,
        ctx: &StrategyCtx,
        side: OrderSide,
        qty: f64,
        reference_price: f64,
        comment: Option<String>,
    ) -> Intent {
        if self.use_live_marketable_limit(ctx) {
            Intent::Place {
                price: self.live_marketable_price_from_reference(side, reference_price),
                qty,
                side,
                comment,
            }
        } else {
            Intent::Market {
                qty,
                side,
                fill_price: None,
                comment,
            }
        }
    }

    fn suppress_startup_replay_bar(
        &mut self,
        ctx: &StrategyCtx,
        bar: &BarEvent,
        dt_local: NaiveDateTime,
        close_prev: f64,
        day_range_prev: f64,
        has_open_position: bool,
    ) -> bool {
        if ctx.trade_mode != crate::TradeMode::Live {
            self.clear_startup_replay_guard();
            return false;
        }
        let Some(boundary_ts_utc) = self.startup_live_replay_boundary_ts_utc else {
            return false;
        };
        if bar.origin == DataOrigin::Live && bar.close_time_utc >= boundary_ts_utc {
            info!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                boundary_ts_utc,
                released_ts_utc = bar.close_time_utc,
                suppressed_bars = self.startup_replay_suppressed_bars,
                "hybrid_startup_replay_guard_released"
            );
            self.clear_startup_replay_guard();
            return false;
        }
        self.orchestrator.warm_bar(self.bar_input(
            dt_local,
            bar,
            close_prev,
            day_range_prev,
            has_open_position,
        ));
        self.last_processed_bar_ts = Some(bar.close_time_utc);
        self.last_bar_close = Some(bar.close);
        self.clear_stale_pending_tail(bar.close_time_utc, ctx.position_qty.unwrap_or(0.0));
        self.startup_replay_suppressed_bars = self.startup_replay_suppressed_bars.saturating_add(1);
        let should_log_info =
            bar.origin == DataOrigin::Live || self.startup_replay_suppressed_bars == 1;
        if should_log_info {
            info!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                ts_utc = bar.close_time_utc,
                dt_local = %dt_local,
                origin = ?bar.origin,
                boundary_ts_utc,
                suppressed_bars = self.startup_replay_suppressed_bars,
                "hybrid_startup_replay_bar_suppressed"
            );
        } else {
            debug!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                ts_utc = bar.close_time_utc,
                dt_local = %dt_local,
                origin = ?bar.origin,
                boundary_ts_utc,
                suppressed_bars = self.startup_replay_suppressed_bars,
                "hybrid_startup_replay_bar_suppressed"
            );
        }
        self.sync_state();
        true
    }

    fn cycle_ts_utc(cycle_id: [u8; 10]) -> Option<i64> {
        let raw = std::str::from_utf8(&cycle_id[..8]).ok()?;
        i64::from_str_radix(raw, 16).ok()
    }

    fn owner_code(owner: Owner) -> &'static str {
        match owner {
            Owner::MeanReversion => "MR",
            Owner::IntradayBreakout => "BO",
        }
    }

    fn role_code(role: TagRole) -> &'static str {
        match role {
            TagRole::Entry => "ENTRY",
            TagRole::Tp => "TP",
            TagRole::Sl => "SL",
            TagRole::Exit => "EXIT",
            TagRole::Cancel => "CANCEL",
        }
    }

    fn format_cycle_id(cycle: &[u8; 10]) -> String {
        let mut out = String::with_capacity(10);
        for b in cycle {
            out.push(*b as char);
        }
        out
    }

    fn parse_cycle_id(raw: &str) -> Option<[u8; 10]> {
        if raw.len() != 10 || !raw.is_ascii() {
            return None;
        }
        let mut out = [b'0'; 10];
        for (i, b) in raw.as_bytes().iter().enumerate() {
            if !(*b as char).is_ascii_hexdigit() {
                return None;
            }
            out[i] = *b;
        }
        Some(out)
    }

    fn format_local_day(day: NaiveDate) -> String {
        day.format("%Y-%m-%d").to_string()
    }

    fn parse_local_day(raw: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()
    }

    fn format_local_dt(dt: NaiveDateTime) -> String {
        dt.format("%Y-%m-%dT%H:%M:%S").to_string()
    }

    fn parse_local_dt(raw: &str) -> Option<NaiveDateTime> {
        NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S").ok()
    }

    fn breakout_snapshot(
        &self,
    ) -> crate::strategies::hybrid_intraday::intraday_breakout::IntradayBreakoutSnapshot {
        self.orchestrator.intraday_breakout.snapshot()
    }

    fn prev_day_close(&self) -> Option<f64> {
        self.prev_day_close
    }

    fn signals_warmed(&self) -> bool {
        self.prev_day_range.is_some() && self.prev_day_close().is_some()
    }

    fn next_cycle_id(&mut self, ts_utc: i64) -> [u8; 10] {
        let ts = (ts_utc.max(0) as u64) & 0xffff_ffff;
        let seq = self.next_cycle_seq & 0xff;
        self.next_cycle_seq = self.next_cycle_seq.wrapping_add(1);
        let value = format!("{ts:08x}{seq:02x}");
        let mut out = [b'0'; 10];
        out.copy_from_slice(value.as_bytes());
        out
    }

    fn parse_hybrid_tag(comment: Option<&str>) -> Option<HybridTag> {
        let comment = comment?;
        if !comment.is_ascii() || !comment.starts_with("HYB|") {
            return None;
        }
        let mut sid = None;
        let mut cycle = None;
        let mut owner = None;
        let mut role = None;
        for part in comment.split('|').skip(1) {
            let (key, value) = part.split_once('=')?;
            match key {
                "sid" => sid = Some(value.to_string()),
                "c" => cycle = Some(value.to_string()),
                "o" => {
                    owner = match value {
                        "MR" => Some(Owner::MeanReversion),
                        "BO" => Some(Owner::IntradayBreakout),
                        _ => None,
                    };
                }
                "r" => {
                    role = match value {
                        "ENTRY" => Some(TagRole::Entry),
                        "TP" => Some(TagRole::Tp),
                        "SL" => Some(TagRole::Sl),
                        "EXIT" => Some(TagRole::Exit),
                        "CANCEL" => Some(TagRole::Cancel),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        Some(HybridTag {
            sid: sid?,
            cycle: cycle?,
            owner,
            role,
        })
    }

    fn is_our_tag(&self, ctx: &StrategyCtx, comment: Option<&str>) -> bool {
        let Some(tag) = Self::parse_hybrid_tag(comment) else {
            return false;
        };
        tag.sid == ctx.strategy_id
    }

    fn ensure_active_cycle_from_comment(&mut self, comment: Option<&str>) {
        if self.active_cycle_id.is_some() {
            return;
        }
        let Some(tag) = Self::parse_hybrid_tag(comment) else {
            return;
        };
        if let Some(cycle) = Self::parse_cycle_id(&tag.cycle) {
            self.active_cycle_id = Some(cycle);
        }
    }

    fn is_terminal_order_status(status: &str) -> bool {
        matches!(
            status,
            "filled" | "canceled" | "cancelled" | "expired" | "rejected"
        )
    }

    fn is_terminal_stop_order_status(status: &str) -> bool {
        matches!(
            status,
            "filled"
                | "canceled"
                | "cancelled"
                | "expired"
                | "rejected"
                | "executed"
                | "triggered"
                | "done"
                | "completed"
        )
    }

    fn build_comment(
        &self,
        ctx: &StrategyCtx,
        cycle_id: &[u8; 10],
        owner: Owner,
        role: TagRole,
    ) -> String {
        let cycle = Self::format_cycle_id(cycle_id);
        let owner = Self::owner_code(owner);
        let role = Self::role_code(role);
        // HYB|sid=<sid>|c=<cycle>|o=<MR|BO>|r=<ENTRY|EXIT>
        format!("HYB|sid={}|c={cycle}|o={owner}|r={role}", ctx.strategy_id)
    }

    fn stop_side_for_entry_side(side: Side) -> OrderSide {
        match side {
            Side::Long => OrderSide::Sell,
            Side::Short => OrderSide::Buy,
        }
    }

    fn stop_condition_for_entry_side(side: Side) -> StopLimitCondition {
        match side {
            Side::Long => StopLimitCondition::LessOrEqual,
            Side::Short => StopLimitCondition::MoreOrEqual,
        }
    }

    fn stop_limit_price(stop_side: OrderSide, trigger_price: f64, tick_size: f64) -> f64 {
        let offset = tick_size.max(0.000_000_1);
        match stop_side {
            OrderSide::Buy => trigger_price + offset,
            OrderSide::Sell => trigger_price - offset,
        }
    }

    fn resolve_stop_end_unix_time(&self, created_ts_utc: i64) -> i64 {
        if created_ts_utc <= 0 {
            return self.stop_end_buffer_or_default(0);
        }
        let offset =
            match FixedOffset::east_opt(self.config.timezone_offset_hours.saturating_mul(3600)) {
                Some(v) => v,
                None => return self.stop_end_buffer_or_default(created_ts_utc),
            };
        let local_dt = match Utc.timestamp_opt(created_ts_utc, 0).single() {
            Some(v) => v.with_timezone(&offset),
            None => return self.stop_end_buffer_or_default(created_ts_utc),
        };
        let close_time = NaiveTime::from_hms_opt(
            self.config.session_close_hour.min(23),
            self.config.session_close_minute.min(59),
            0,
        )
        .unwrap_or_else(|| NaiveTime::from_hms_opt(23, 49, 0).expect("valid close time"));
        let mut day = local_dt.date_naive();
        for _ in 0..8 {
            if self.config.weekends_off && matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
                day += ChronoDuration::days(1);
                continue;
            }
            let local_close = day.and_time(close_time);
            if let Some(with_offset) = offset.from_local_datetime(&local_close).single() {
                let stop_end = with_offset
                    .timestamp()
                    .saturating_add(self.config.stop_end_buffer_sec as i64);
                if stop_end > created_ts_utc {
                    return stop_end;
                }
            }
            day += ChronoDuration::days(1);
        }
        self.stop_end_buffer_or_default(created_ts_utc)
    }

    fn stop_end_buffer_or_default(&self, created_ts_utc: i64) -> i64 {
        created_ts_utc
            .max(0)
            .saturating_add(self.config.stop_end_buffer_sec as i64)
    }

    fn emit_mr_bracket_intents(
        &mut self,
        ctx: &StrategyCtx,
        pos: &PositionEvent,
        entry: PendingEntry,
    ) -> Vec<Intent> {
        if entry.owner != Owner::MeanReversion || entry.entry_style != EntryStyle::Bracket {
            return Vec::new();
        }
        let qty = pos.qty.abs();
        if qty <= f64::EPSILON {
            return Vec::new();
        }
        let mut intents = Vec::new();
        if let Some(take_price) = entry.take_price {
            let tp_side = Self::stop_side_for_entry_side(entry.side);
            let comment = self.build_comment(ctx, &entry.cycle_id, entry.owner, TagRole::Tp);
            let request_id = crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                "place",
                pos.ts_utc,
                0,
            );
            self.pending_tp_request_id = Some(request_id);
            self.pending_tp_created_ts_utc = Some(pos.ts_utc);
            intents.push(
                Intent::Place {
                    price: take_price,
                    qty,
                    side: tp_side,
                    comment: Some(comment),
                }
                .with_class(IntentClass::ProtectiveRepair),
            );
        }
        if let Some(stop_price) = entry.stop_price {
            let stop_side = Self::stop_side_for_entry_side(entry.side);
            let condition = Self::stop_condition_for_entry_side(entry.side);
            let limit_price = Self::stop_limit_price(stop_side, stop_price, ctx.tick_size);
            let comment = self.build_comment(ctx, &entry.cycle_id, entry.owner, TagRole::Sl);
            let request_id = crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                "create_stop_limit",
                pos.ts_utc,
                5,
            );
            self.pending_sl_request_id = Some(request_id);
            self.pending_sl_created_ts_utc = Some(pos.ts_utc);
            intents.push(
                Intent::CreateStopLimit {
                    side: stop_side,
                    qty,
                    trigger_price: stop_price,
                    price: limit_price,
                    condition,
                    stop_end_unix_time: self.resolve_stop_end_unix_time(pos.ts_utc),
                    comment: Some(comment),
                    instrument_group: None,
                    check_duplicates: Some(true),
                }
                .with_class(IntentClass::ProtectiveRepair),
            );
        }
        intents
    }

    fn emit_cancel_all_protection(&mut self, side: Option<Side>) -> Vec<Intent> {
        let mut intents = Vec::new();
        if let Some(tp_order_id) = self.tp_order_id.take() {
            intents.push(
                Intent::Cancel {
                    order_id: tp_order_id,
                }
                .with_class(IntentClass::CancelCleanup),
            );
        }
        if let Some(stop_order_id) = self.sl_stop_order_id.take() {
            let stop_side = side.map(Self::stop_side_for_entry_side);
            intents.push(
                Intent::DeleteStopLimit {
                    order_id: stop_order_id,
                    side: stop_side,
                    check_duplicates: Some(true),
                }
                .with_class(IntentClass::CancelCleanup),
            );
        }
        if let Some(exchange_order_id) = self.sl_exchange_order_id.take() {
            intents.push(
                Intent::Cancel {
                    order_id: exchange_order_id,
                }
                .with_class(IntentClass::CancelCleanup),
            );
        }
        intents
    }

    fn emit_broker_residual_emergency_exit(
        &mut self,
        ctx: &StrategyCtx,
        pos: &PositionEvent,
        reason: &'static str,
    ) -> Vec<Intent> {
        if self.pending_exit_request_id.is_some() || pos.qty.abs() <= f64::EPSILON {
            return Vec::new();
        }
        let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
        let side = if pos.qty > 0.0 {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        };
        let created_ts_utc = self.effective_created_ts_utc(ctx, pos.ts_utc);
        let cycle_id = self
            .active_cycle_id
            .unwrap_or_else(|| self.next_cycle_id(created_ts_utc));
        let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Exit);
        let mut intents = self.emit_cancel_all_protection(self.current_side);
        intents.push(
            self.build_live_entry_exit_intent(
                ctx,
                side,
                pos.qty.abs(),
                self.live_reference_price(),
                Some(comment),
            )
            .with_class(IntentClass::Exit),
        );
        self.active_cycle_id = Some(cycle_id);
        self.pending_exit = Some(PendingExit {
            owner,
            reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
        });
        self.pending_exit_request_id = Some(self.live_request_id(ctx, created_ts_utc, side));
        self.pending_exit_created_ts_utc = Some(created_ts_utc);
        self.enter_safe_mode(reason);
        warn!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "broker_residual_emergency_exit",
            reason,
            previous_qty = self.last_position_qty,
            broker_qty = pos.qty,
            exit_side = ?side,
            exit_qty = pos.qty.abs(),
            "broker position changed while bracket was active; cancel protection and flatten residual"
        );
        intents
    }

    fn mark_bracket_terminal_reconcile(&mut self) {
        self.bracket_terminal_reconcile_started_ms = Some(Utc::now().timestamp_millis());
    }

    fn bracket_terminal_reconcile_active(&self, now_ms: i64) -> bool {
        self.bracket_terminal_reconcile_started_ms
            .is_some_and(|started| {
                now_ms.saturating_sub(started) < Self::BRACKET_TERMINAL_RECONCILE_GRACE_MS
            })
    }

    fn clear_bracket_terminal_reconcile(&mut self) {
        self.bracket_terminal_reconcile_started_ms = None;
    }

    fn lifecycle_bracket_terminal_reconcile_log(&self, prev: f64, cur: f64) {
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "bracket_terminal_reconcile_wait",
            previous_qty = prev,
            broker_qty = cur,
            "bracket terminal fill is settling; suppress residual emergency exit until reconcile grace expires"
        );
    }

    fn emit_bracket_reconcile_timeout_exit(
        &mut self,
        ctx: &StrategyCtx,
        now_ts_utc_ms: i64,
    ) -> Vec<Intent> {
        let Some(started) = self.bracket_terminal_reconcile_started_ms else {
            return Vec::new();
        };
        if now_ts_utc_ms.saturating_sub(started) < Self::BRACKET_TERMINAL_RECONCILE_GRACE_MS {
            return Vec::new();
        }
        let qty = ctx.position_qty.unwrap_or(self.last_position_qty);
        if qty.abs() <= f64::EPSILON || self.pending_exit_request_id.is_some() {
            self.clear_bracket_terminal_reconcile();
            return Vec::new();
        }
        let pos = PositionEvent {
            symbol: self.config.symbol.clone(),
            qty,
            existing: false,
            avg_price: self.live_reference_price(),
            ts_utc: now_ts_utc_ms.div_euclid(1_000),
        };
        self.clear_bracket_terminal_reconcile();
        self.emit_broker_residual_emergency_exit(ctx, &pos, "bracket_terminal_reconcile_timeout")
    }

    fn expected_entry_qty(entry: PendingEntry) -> f64 {
        match entry.side {
            Side::Long => entry.target_qty,
            Side::Short => -entry.target_qty,
        }
    }

    fn is_mr_bracket_entry(entry: PendingEntry) -> bool {
        entry.owner == Owner::MeanReversion && entry.entry_style == EntryStyle::Bracket
    }

    fn waits_for_partial_entry_target(entry: PendingEntry) -> bool {
        Self::is_mr_bracket_entry(entry) && entry.target_qty > 1.0
    }

    fn partial_entry_progress_is_valid(entry: PendingEntry, prev: f64, cur: f64) -> bool {
        let target = Self::expected_entry_qty(entry);
        cur.abs() > f64::EPSILON
            && cur.signum() == target.signum()
            && cur.abs() <= target.abs() + f64::EPSILON
            && cur.abs() + f64::EPSILON >= prev.abs()
    }

    fn emit_partial_entry_timeout_exit(
        &mut self,
        ctx: &StrategyCtx,
        now_ts_utc_ms: i64,
    ) -> Vec<Intent> {
        let Some(entry) = self.pending_entry else {
            return Vec::new();
        };
        if !Self::waits_for_partial_entry_target(entry) {
            return Vec::new();
        }
        let Some(started_at_ms) = entry.partial_started_at_ms else {
            return Vec::new();
        };
        if self.last_position_qty.abs() <= f64::EPSILON
            || now_ts_utc_ms.saturating_sub(started_at_ms)
                < self.config.partial_entry_fill_timeout_ms as i64
        {
            return Vec::new();
        }

        let created_ts_utc = now_ts_utc_ms.div_euclid(1_000);
        let mut intents = self
            .working_orders
            .iter()
            .copied()
            .map(|order_id| Intent::Cancel { order_id }.with_class(IntentClass::CancelCleanup))
            .collect::<Vec<_>>();
        let side = if self.last_position_qty > 0.0 {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        };
        let comment = self.build_comment(ctx, &entry.cycle_id, entry.owner, TagRole::Exit);
        intents.push(
            self.build_live_entry_exit_intent(
                ctx,
                side,
                self.last_position_qty.abs(),
                self.live_reference_price(),
                Some(comment),
            )
            .with_class(IntentClass::Exit),
        );
        self.current_owner = Some(entry.owner);
        self.current_side = Some(entry.side);
        self.pending_entry = None;
        self.pending_entry_request_id = None;
        self.pending_entry_created_ts_utc = None;
        self.pending_exit = Some(PendingExit {
            owner: entry.owner,
            reason: ReasonCode::MeanRevTimeCutoff,
        });
        self.pending_exit_request_id = Some(self.live_request_id(ctx, created_ts_utc, side));
        self.pending_exit_created_ts_utc = Some(created_ts_utc);
        self.enter_safe_mode("partial_entry_fill_timeout");
        warn!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "partial_entry_timeout_emergency_exit",
            owner = ?entry.owner,
            side = ?entry.side,
            broker_qty = self.last_position_qty,
            target_qty = entry.target_qty,
            elapsed_ms = now_ts_utc_ms.saturating_sub(started_at_ms),
            timeout_ms = self.config.partial_entry_fill_timeout_ms,
            working_entry_orders = self.working_orders.len(),
            "partial MR entry did not reach target; cancel remainder and flatten partial position"
        );
        self.sync_state();
        intents
    }

    fn sync_state(&mut self) {
        let breakout = self.breakout_snapshot();
        let orchestrator = self.orchestrator.snapshot();
        let pending_risk_gate_finalization = self.pending_risk_gate_finalizations.first();
        self.state = StrategyState::HybridIntradayRuntime {
            active_cycle_id: self.active_cycle_id.map(|v| Self::format_cycle_id(&v)),
            next_cycle_seq: self.next_cycle_seq,
            last_position_qty: self.last_position_qty,
            current_owner: self.current_owner,
            current_side: self.current_side,
            pending_entry_owner: self.pending_entry.map(|v| v.owner),
            pending_entry_side: self.pending_entry.map(|v| v.side),
            pending_entry_cycle_id: self
                .pending_entry
                .as_ref()
                .map(|v| Self::format_cycle_id(&v.cycle_id)),
            pending_entry_request_id: self.pending_entry_request_id,
            pending_entry_created_ts_utc: self.pending_entry_created_ts_utc,
            deferred_entry_owner: self.deferred_entry.as_ref().map(|v| v.signal.owner),
            deferred_entry_side: self.deferred_entry.as_ref().map(|v| v.signal.side),
            deferred_entry_cycle_id: self
                .deferred_entry
                .as_ref()
                .map(|v| Self::format_cycle_id(&v.cycle_id)),
            deferred_entry_entry_style: self.deferred_entry.as_ref().map(|v| v.signal.entry_style),
            deferred_entry_reason: self.deferred_entry.as_ref().map(|v| v.signal.reason),
            deferred_entry_stop_price: self
                .deferred_entry
                .as_ref()
                .and_then(|v| v.signal.stop_price),
            deferred_entry_take_price: self
                .deferred_entry
                .as_ref()
                .and_then(|v| v.signal.take_price),
            deferred_entry_ts_utc: self.deferred_entry.as_ref().map(|v| v.deferred_ts_utc),
            deferred_entry_request_id: self.deferred_entry.as_ref().map(|v| v.original_request_id),
            pending_exit_request_id: self.pending_exit_request_id,
            pending_exit_created_ts_utc: self.pending_exit_created_ts_utc,
            deferred_exit_owner: self.deferred_exit.map(|v| v.owner),
            deferred_exit_reason: self.deferred_exit.map(|v| v.reason),
            deferred_exit_cycle_id: self
                .deferred_exit
                .map(|v| Self::format_cycle_id(&v.cycle_id)),
            deferred_exit_ts_utc: self.deferred_exit.map(|v| v.deferred_ts_utc),
            deferred_exit_request_id: self.deferred_exit.map(|v| v.original_request_id),
            pending_tp_request_id: self.pending_tp_request_id,
            pending_tp_created_ts_utc: self.pending_tp_created_ts_utc,
            pending_sl_request_id: self.pending_sl_request_id,
            pending_sl_created_ts_utc: self.pending_sl_created_ts_utc,
            tp_order_id: self.tp_order_id,
            sl_stop_order_id: self.sl_stop_order_id.clone(),
            sl_exchange_order_id: self.sl_exchange_order_id,
            sl_triggered_ts: self.sl_triggered_ts,
            mr_take_price: self.mr_take_price,
            mr_stop_price: self.mr_stop_price,
            repair_deadline_ts: self.repair_deadline_ts,
            next_repair_at_ts: self.next_repair_at_ts,
            repair_backoff_level: self.repair_backoff_level,
            repair_attempts: self.repair_attempts,
            safe_mode_close_only: self.safe_mode_close_only,
            safe_mode_reason: self.safe_mode_reason.clone(),
            entry_ready: self.entry_ready,
            last_bar_close: self.last_bar_close,
            prev_day_close: self.prev_day_close,
            last_day_local: self.last_day_local.map(Self::format_local_day),
            current_day_high: self.current_day_high,
            current_day_low: self.current_day_low,
            prev_day_range: self.prev_day_range,
            current_day_close: breakout.cur_day_close,
            prev_day_return: breakout.yesterday_return,
            day_before_close: breakout.day_before_close,
            today_start_local: breakout.today_start.map(Self::format_local_dt),
            was_long_today: breakout.was_long_today,
            was_short_today: breakout.was_short_today,
            overnight_exit_armed_date: orchestrator
                .overnight_exit_armed_date
                .map(Self::format_local_day),
            risk_gate_shadow_session_date: self
                .risk_gate_shadow_session_date
                .map(Self::format_local_day),
            risk_gate_shadow_pnl_points: self.risk_gate_shadow_pnl_points,
            risk_gate_shadow_trade_count: self.risk_gate_shadow_trade_count,
            risk_gate_shadow_entry_ts_utc: self
                .risk_gate_shadow_position
                .map(|position| position.entry_ts_utc),
            risk_gate_shadow_entry_price: self
                .risk_gate_shadow_position
                .map(|position| position.entry_price),
            risk_gate_shadow_side: self.risk_gate_shadow_position.map(|position| position.side),
            risk_gate_shadow_target_price: self.risk_gate_shadow_open.map(|open| open.target_price),
            risk_gate_shadow_stop_price: self.risk_gate_shadow_open.map(|open| open.stop_price),
            risk_gate_pending_session_date: pending_risk_gate_finalization
                .map(|finalization| Self::format_local_day(finalization.session_date)),
            risk_gate_pending_shadow_pnl_points: pending_risk_gate_finalization
                .map(|finalization| finalization.shadow_pnl_points)
                .unwrap_or(0.0),
            risk_gate_pending_shadow_trade_count: pending_risk_gate_finalization
                .map(|finalization| finalization.shadow_trade_count)
                .unwrap_or(0),
            risk_gate_mr_enabled_current_session: self.risk_gate_mr_enabled_current_session,
            risk_gate_rolling_sum_lb120: self.risk_gate_rolling_sum_lb120,
            risk_gate_last_finalized_session_date: self
                .risk_gate_last_finalized_session_date
                .map(Self::format_local_day),
            risk_gate_ledger_rows_count: self.risk_gate_ledger_rows_count,
        };
    }

    fn is_window_closed_recoverable_reject(ack: &CommandAck) -> bool {
        ack.error_code.as_deref() == Some("trading_window_closed") && ack.broker_order_id.is_none()
    }

    fn clear_deferred_entry(&mut self) {
        self.deferred_entry = None;
    }

    fn clear_deferred_exit(&mut self) {
        self.deferred_exit = None;
    }

    fn deferred_entry_expired(&self, deferred: &DeferredEntry, dt_local: NaiveDateTime) -> bool {
        self.utc_to_local_naive(deferred.deferred_ts_utc)
            .map(|deferred_local| dt_local.date() > deferred_local.date())
            .unwrap_or(false)
    }

    fn maybe_reissue_deferred_entry(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        dt_local: NaiveDateTime,
        can_emit: bool,
        can_execute: bool,
        reference_price: f64,
    ) -> Option<Vec<Intent>> {
        let deferred = self.deferred_entry.clone()?;
        if ctx.position_qty.unwrap_or(0.0).abs() > f64::EPSILON {
            self.clear_deferred_entry();
            return None;
        }
        if self.deferred_entry_expired(&deferred, dt_local) {
            info!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                action = "deferred_entry_expired",
                deferred_ts_utc = deferred.deferred_ts_utc,
                owner = ?deferred.signal.owner,
                side = ?deferred.signal.side,
                cycle_id = %Self::format_cycle_id(&deferred.cycle_id),
                original_request_id = %deferred.original_request_id,
            );
            self.clear_deferred_entry();
            return None;
        }
        if !self.entry_ready
            || self.safe_mode_close_only
            || !can_emit
            || !can_execute
            || self.pending_entry_request_id.is_some()
        {
            return Some(Vec::new());
        }
        self.active_cycle_id = Some(deferred.cycle_id);
        let intents = self.map_action_to_intents_with_reference(
            ctx,
            created_ts_utc,
            can_emit,
            can_execute,
            reference_price,
            Action::SubmitEntry(deferred.signal.clone()),
        );
        if self.pending_entry_request_id.is_some() {
            info!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                action = "deferred_entry_reissued",
                deferred_ts_utc = deferred.deferred_ts_utc,
                owner = ?deferred.signal.owner,
                side = ?deferred.signal.side,
                cycle_id = %Self::format_cycle_id(&deferred.cycle_id),
                original_request_id = %deferred.original_request_id,
                new_request_id = ?self.pending_entry_request_id,
            );
            self.clear_deferred_entry();
        }
        Some(intents)
    }

    fn maybe_reissue_deferred_exit(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        can_emit: bool,
        can_execute: bool,
        reference_price: f64,
    ) -> Option<Vec<Intent>> {
        let deferred = self.deferred_exit?;
        if ctx.position_qty.unwrap_or(0.0).abs() <= f64::EPSILON {
            self.clear_deferred_exit();
            return None;
        }
        if !can_emit || !can_execute || self.pending_exit_request_id.is_some() {
            return Some(Vec::new());
        }
        self.active_cycle_id = Some(deferred.cycle_id);
        self.current_owner = Some(deferred.owner);
        let intents = self.map_action_to_intents_with_reference(
            ctx,
            created_ts_utc,
            can_emit,
            can_execute,
            reference_price,
            Action::SubmitExit {
                owner: deferred.owner,
                reason: deferred.reason,
            },
        );
        if self.pending_exit_request_id.is_some() {
            info!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                action = "deferred_exit_reissued",
                deferred_ts_utc = deferred.deferred_ts_utc,
                owner = ?deferred.owner,
                reason = ?deferred.reason,
                cycle_id = %Self::format_cycle_id(&deferred.cycle_id),
                original_request_id = %deferred.original_request_id,
                new_request_id = ?self.pending_exit_request_id,
            );
            self.clear_deferred_exit();
        }
        Some(intents)
    }

    fn enter_safe_mode(&mut self, reason: impl Into<String>) {
        self.safe_mode_close_only = true;
        self.safe_mode_reason = Some(reason.into());
    }

    fn reset_repair_tracking(&mut self) {
        self.pending_tp_request_id = None;
        self.pending_tp_created_ts_utc = None;
        self.pending_sl_request_id = None;
        self.pending_sl_created_ts_utc = None;
        self.tp_order_id = None;
        self.sl_stop_order_id = None;
        self.sl_exchange_order_id = None;
        self.sl_triggered_ts = None;
        self.mr_take_price = None;
        self.mr_stop_price = None;
        self.repair_deadline_ts = None;
        self.next_repair_at_ts = None;
        self.repair_backoff_level = 0;
        self.repair_attempts = 0;
    }

    fn schedule_next_repair(&mut self, now_ts: i64) {
        let power = self.repair_backoff_level.min(16);
        let step = self
            .config
            .repair_backoff_base_sec
            .saturating_mul(1u64 << power)
            .min(self.config.repair_backoff_max_sec);
        self.next_repair_at_ts = Some(now_ts.saturating_add(step as i64));
        self.repair_backoff_level = self.repair_backoff_level.saturating_add(1);
    }

    fn side_to_order_side(side: Side) -> OrderSide {
        match side {
            Side::Long => OrderSide::Buy,
            Side::Short => OrderSide::Sell,
        }
    }

    fn update_day_aggregates(&mut self, dt_local: NaiveDateTime, high: f64, low: f64) {
        let day = dt_local.date();
        match self.last_day_local {
            None => {
                self.last_day_local = Some(day);
                self.current_day_high = Some(high);
                self.current_day_low = Some(low);
            }
            Some(prev_day) if prev_day == day => {
                self.current_day_high = Some(self.current_day_high.unwrap_or(high).max(high));
                self.current_day_low = Some(self.current_day_low.unwrap_or(low).min(low));
            }
            Some(prev_day) => {
                let mut computed_prev_day_range = self.prev_day_range;
                if let (Some(h), Some(l)) = (self.current_day_high, self.current_day_low) {
                    computed_prev_day_range = Some((h - l).max(0.0));
                    self.prev_day_range = computed_prev_day_range;
                }
                self.prev_day_close = self.last_bar_close;
                info!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    prev_day = %Self::format_local_day(prev_day),
                    next_day = %Self::format_local_day(day),
                    prev_day_close = self.prev_day_close,
                    prev_day_high = self.current_day_high,
                    prev_day_low = self.current_day_low,
                    prev_day_range = computed_prev_day_range,
                    prev_day_return = self.breakout_snapshot().yesterday_return,
                    "hybrid day rollover: recalculated day features"
                );
                self.last_day_local = Some(day);
                self.current_day_high = Some(high);
                self.current_day_low = Some(low);
            }
        }
        self.entry_ready = self.signals_warmed();
    }

    fn log_signal_warmup_status_if_changed(&mut self, dt_local: NaiveDateTime, last_bar_ts: i64) {
        let breakout = self.breakout_snapshot();
        let warmed = self.signals_warmed();
        if self.last_warmup_log == Some(warmed) {
            return;
        }
        self.last_warmup_log = Some(warmed);
        if warmed {
            info!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                strategy = "hybrid_intraday_runtime",
                symbol = self.config.symbol,
                local_day = %Self::format_local_day(dt_local.date()),
                prev_day_close = breakout.yesterday_close,
                prev_day_range = self.prev_day_range,
                prev_day_return = breakout.yesterday_return,
                current_day_high = self.current_day_high,
                current_day_low = self.current_day_low,
                current_day_close = breakout.cur_day_close,
                today_start_local = breakout.today_start.map(Self::format_local_dt),
                was_long_today = breakout.was_long_today,
                was_short_today = breakout.was_short_today,
                entry_ready = true,
                "signal warmup complete"
            );
        } else {
            warn!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                strategy = "hybrid_intraday_runtime",
                symbol = self.config.symbol,
                local_day = %Self::format_local_day(dt_local.date()),
                last_bar_ts,
                prev_day_close = breakout.yesterday_close,
                prev_day_range = self.prev_day_range,
                prev_day_return = breakout.yesterday_return,
                current_day_high = self.current_day_high,
                current_day_low = self.current_day_low,
                current_day_close = breakout.cur_day_close,
                today_start_local = breakout.today_start.map(Self::format_local_dt),
                was_long_today = breakout.was_long_today,
                was_short_today = breakout.was_short_today,
                entry_ready = false,
                action = "signal_warmup_incomplete",
                reason = "prev_day_features_missing",
                "signal warmup incomplete"
            );
        }
    }

    fn map_action_to_intents(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        can_emit: bool,
        can_execute: bool,
        action: Action,
    ) -> Vec<Intent> {
        self.map_action_to_intents_with_reference(
            ctx,
            created_ts_utc,
            can_emit,
            can_execute,
            self.live_reference_price(),
            action,
        )
    }

    fn map_action_to_intents_with_reference(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        can_emit: bool,
        can_execute: bool,
        reference_price: f64,
        action: Action,
    ) -> Vec<Intent> {
        match action {
            Action::SubmitEntry(entry) => {
                if !self.entry_ready || self.safe_mode_close_only || !can_emit {
                    // Entry action was dropped by wrapper-level guards; release orchestrator pending state.
                    self.orchestrator.on_order_rejected("entry");
                    return Vec::new();
                }
                let cycle_id = self
                    .active_cycle_id
                    .unwrap_or_else(|| self.next_cycle_id(created_ts_utc));
                if can_execute {
                    self.pending_entry = Some(PendingEntry {
                        owner: entry.owner,
                        side: entry.side,
                        cycle_id,
                        reason: entry.reason,
                        entry_style: entry.entry_style,
                        stop_price: entry.stop_price,
                        take_price: entry.take_price,
                        target_qty: self.config.qty.max(1.0),
                        partial_started_at_ms: None,
                    });
                    self.pending_entry_request_id = Some(self.live_request_id(
                        ctx,
                        created_ts_utc,
                        Self::side_to_order_side(entry.side),
                    ));
                    self.pending_entry_created_ts_utc = Some(created_ts_utc);
                    self.active_cycle_id = Some(cycle_id);
                }
                let comment = self.build_comment(ctx, &cycle_id, entry.owner, TagRole::Entry);
                if can_execute {
                    self.sync_state();
                }
                vec![self
                    .build_live_entry_exit_intent(
                        ctx,
                        Self::side_to_order_side(entry.side),
                        self.config.qty.max(1.0),
                        reference_price,
                        Some(comment),
                    )
                    .with_class(IntentClass::Entry)]
            }
            Action::SubmitExit { owner, reason } => {
                if !can_emit {
                    return Vec::new();
                }
                if let Some(pending_req_id) = self.pending_exit_request_id {
                    info!(
                        target: "strategy_runtime::hybrid_intraday_runtime",
                        action = "exit_suppressed",
                        reason = "pending_exit_active",
                        owner = ?owner,
                        cycle_id = self.active_cycle_id.map(|v| Self::format_cycle_id(&v)),
                        pending_exit_request_id = %pending_req_id,
                    );
                    return Vec::new();
                }
                let Some(pos_qty) = ctx.position_qty else {
                    return Vec::new();
                };
                let qty = pos_qty.abs();
                if qty <= f64::EPSILON {
                    return Vec::new();
                }
                let side = if pos_qty >= 0.0 {
                    OrderSide::Sell
                } else {
                    OrderSide::Buy
                };
                let cycle_id = self
                    .active_cycle_id
                    .unwrap_or_else(|| self.next_cycle_id(created_ts_utc));
                if can_execute {
                    self.active_cycle_id = Some(cycle_id);
                    self.current_owner = Some(owner);
                    self.pending_exit = Some(PendingExit { owner, reason });
                    self.pending_exit_request_id =
                        Some(self.live_request_id(ctx, created_ts_utc, side));
                    self.pending_exit_created_ts_utc = Some(created_ts_utc);
                }
                let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Exit);
                if can_execute {
                    self.sync_state();
                }
                vec![self
                    .build_live_entry_exit_intent(ctx, side, qty, reference_price, Some(comment))
                    .with_class(IntentClass::Exit)]
            }
            Action::ArmOvernightExit { .. } => Vec::new(),
        }
    }

    fn maybe_emit_repair_intents(&mut self, ctx: &StrategyCtx, now_ts: i64) -> Vec<Intent> {
        let pos_qty = ctx.position_qty.unwrap_or(0.0);
        if pos_qty.abs() <= f64::EPSILON || self.current_owner != Some(Owner::MeanReversion) {
            self.sl_triggered_ts = None;
            return Vec::new();
        }
        if let Some(triggered_ts) = self.sl_triggered_ts {
            let escalate_at =
                triggered_ts.saturating_add(self.config.sl_escalate_timeout_sec as i64);
            if now_ts < escalate_at {
                return Vec::new();
            }
            let side = if pos_qty >= 0.0 {
                OrderSide::Sell
            } else {
                OrderSide::Buy
            };
            let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
            let cycle_id = self
                .active_cycle_id
                .unwrap_or_else(|| self.next_cycle_id(now_ts.max(0)));
            let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Exit);
            let reference_price = self.live_reference_price();
            let mut intents = Vec::new();
            if let Some(tp_order_id) = self.tp_order_id.take() {
                intents.push(
                    Intent::Cancel {
                        order_id: tp_order_id,
                    }
                    .with_class(IntentClass::CancelCleanup),
                );
            }
            if let Some(exchange_order_id) = self.sl_exchange_order_id.take() {
                intents.push(
                    Intent::Cancel {
                        order_id: exchange_order_id,
                    }
                    .with_class(IntentClass::CancelCleanup),
                );
            }
            intents.push(
                self.build_live_entry_exit_intent(
                    ctx,
                    side,
                    pos_qty.abs(),
                    reference_price,
                    Some(comment),
                )
                .with_class(IntentClass::Exit),
            );
            self.sl_triggered_ts = None;
            self.enter_safe_mode("sl_triggered_escalation");
            return intents;
        }
        let has_tp = self.tp_order_id.is_some() || self.pending_tp_request_id.is_some();
        let has_sl = self.sl_stop_order_id.is_some() || self.pending_sl_request_id.is_some();
        if has_tp && has_sl {
            return Vec::new();
        }
        let Some(cycle_id) = self.active_cycle_id else {
            self.enter_safe_mode("repair_missing_cycle_id");
            return Vec::new();
        };
        if self.mr_take_price.is_none() || self.mr_stop_price.is_none() {
            self.enter_safe_mode("repair_missing_bracket_levels");
            return Vec::new();
        }
        if self
            .repair_deadline_ts
            .is_some_and(|deadline| now_ts >= deadline)
        {
            let side = if pos_qty >= 0.0 {
                OrderSide::Sell
            } else {
                OrderSide::Buy
            };
            let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
            let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Exit);
            let reference_price = self.live_reference_price();
            self.enter_safe_mode("repair_deadline_force_flatten");
            return vec![self
                .build_live_entry_exit_intent(
                    ctx,
                    side,
                    pos_qty.abs(),
                    reference_price,
                    Some(comment),
                )
                .with_class(IntentClass::Exit)];
        }
        if self
            .next_repair_at_ts
            .is_some_and(|next_ts| now_ts < next_ts)
        {
            return Vec::new();
        }
        if !self.can_execute_now(ctx, true) {
            self.schedule_next_repair(now_ts);
            return Vec::new();
        }
        if ctx.gateway_phase != crate::live_guard::GatewayPhase::LiveReady {
            self.schedule_next_repair(now_ts);
            return Vec::new();
        }
        if self.repair_attempts >= self.config.max_repair_retries {
            self.enter_safe_mode("repair_retries_exhausted");
            return Vec::new();
        }

        let mut intents = Vec::new();
        let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
        let side = self.current_side.unwrap_or(if pos_qty >= 0.0 {
            Side::Long
        } else {
            Side::Short
        });
        if !has_tp && self.pending_tp_request_id.is_none() {
            let tp_side = Self::stop_side_for_entry_side(side);
            let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Tp);
            let request_id = crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                "place",
                now_ts,
                0,
            );
            self.pending_tp_request_id = Some(request_id);
            self.pending_tp_created_ts_utc = Some(now_ts);
            intents.push(
                Intent::Place {
                    price: self.mr_take_price.unwrap_or_default(),
                    qty: pos_qty.abs(),
                    side: tp_side,
                    comment: Some(comment),
                }
                .with_class(IntentClass::ProtectiveRepair),
            );
        }
        if !has_sl && self.pending_sl_request_id.is_none() {
            let stop_side = Self::stop_side_for_entry_side(side);
            let stop_price = self.mr_stop_price.unwrap_or_default();
            let limit_price = Self::stop_limit_price(stop_side, stop_price, ctx.tick_size);
            let condition = Self::stop_condition_for_entry_side(side);
            let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Sl);
            let request_id = crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                &ctx.symbol,
                "create_stop_limit",
                now_ts,
                5,
            );
            self.pending_sl_request_id = Some(request_id);
            self.pending_sl_created_ts_utc = Some(now_ts);
            intents.push(
                Intent::CreateStopLimit {
                    side: stop_side,
                    qty: pos_qty.abs(),
                    trigger_price: stop_price,
                    price: limit_price,
                    condition,
                    stop_end_unix_time: self.resolve_stop_end_unix_time(now_ts),
                    comment: Some(comment),
                    instrument_group: None,
                    check_duplicates: Some(true),
                }
                .with_class(IntentClass::ProtectiveRepair),
            );
        }
        if !intents.is_empty() {
            self.repair_attempts = self.repair_attempts.saturating_add(1);
            self.schedule_next_repair(now_ts);
        }
        intents
    }

    fn pending_timeout_sec(&self) -> i64 {
        self.config.pending_timeout_sec.max(1) as i64
    }

    fn clear_stale_pending_tail(&mut self, now_ts: i64, position_qty: f64) {
        if position_qty.abs() > f64::EPSILON {
            return;
        }
        if !self.working_orders.is_empty() || !self.working_stop_orders.is_empty() {
            return;
        }
        let timeout = self.pending_timeout_sec();

        if let Some(req_id) = self.pending_entry_request_id {
            let created = self
                .pending_entry_created_ts_utc
                .or_else(|| {
                    self.pending_entry
                        .and_then(|p| Self::cycle_ts_utc(p.cycle_id))
                })
                .unwrap_or(now_ts);
            if now_ts.saturating_sub(created) > timeout {
                self.orchestrator.on_order_rejected("entry");
                self.pending_entry = None;
                self.pending_entry_request_id = None;
                self.pending_entry_created_ts_utc = None;
                self.clear_deferred_entry();
                if self.last_position_qty.abs() <= f64::EPSILON {
                    self.active_cycle_id = None;
                }
                tracing::info!(
                    request_id = %req_id,
                    age_sec = now_ts.saturating_sub(created),
                    "hybrid_pending_gc_entry"
                );
            }
        }

        if let Some(req_id) = self.pending_exit_request_id {
            let created = self.pending_exit_created_ts_utc.unwrap_or(now_ts);
            if now_ts.saturating_sub(created) > timeout {
                self.pending_exit = None;
                self.pending_exit_request_id = None;
                self.pending_exit_created_ts_utc = None;
                tracing::info!(
                    request_id = %req_id,
                    age_sec = now_ts.saturating_sub(created),
                    "hybrid_pending_gc_exit"
                );
            }
        }

        if let Some(req_id) = self.pending_tp_request_id {
            let created = self.pending_tp_created_ts_utc.unwrap_or(now_ts);
            if now_ts.saturating_sub(created) > timeout {
                self.pending_tp_request_id = None;
                self.pending_tp_created_ts_utc = None;
                tracing::info!(
                    request_id = %req_id,
                    age_sec = now_ts.saturating_sub(created),
                    "hybrid_pending_gc_tp"
                );
            }
        }

        if let Some(req_id) = self.pending_sl_request_id {
            let created = self.pending_sl_created_ts_utc.unwrap_or(now_ts);
            if now_ts.saturating_sub(created) > timeout {
                self.pending_sl_request_id = None;
                self.pending_sl_created_ts_utc = None;
                tracing::info!(
                    request_id = %req_id,
                    age_sec = now_ts.saturating_sub(created),
                    "hybrid_pending_gc_sl"
                );
            }
        }
    }

    fn clear_boot_stale_pending_tail(&mut self, restored: &crate::RuntimeStateRestored) {
        if self.last_position_qty.abs() > f64::EPSILON {
            return;
        }
        if !self.working_orders.is_empty() || !self.working_stop_orders.is_empty() {
            return;
        }
        let had_stale_tail = self.pending_entry.is_some()
            || self.pending_entry_request_id.is_some()
            || self.pending_exit_request_id.is_some()
            || self.pending_tp_request_id.is_some()
            || self.pending_sl_request_id.is_some()
            || self.current_owner.is_some()
            || self.current_side.is_some()
            || self.active_cycle_id.is_some();
        if !had_stale_tail {
            return;
        }

        self.pending_entry = None;
        self.pending_entry_request_id = None;
        self.pending_entry_created_ts_utc = None;
        self.clear_deferred_entry();
        self.pending_exit = None;
        self.pending_exit_request_id = None;
        self.pending_exit_created_ts_utc = None;
        self.clear_deferred_exit();
        self.pending_tp_request_id = None;
        self.pending_tp_created_ts_utc = None;
        self.pending_sl_request_id = None;
        self.pending_sl_created_ts_utc = None;
        self.current_owner = None;
        self.current_side = None;
        self.active_cycle_id = None;
        self.reset_repair_tracking();
        self.orchestrator.reset();
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            known_order_ids = restored.known_order_ids.len(),
            pending_requests = restored.pending_requests.len(),
            "hybrid_boot_cleared_stale_pending_tail"
        );
        self.sync_state();
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::{StrategyCtx, TradeMode};

    fn test_config() -> HybridIntradayRuntimeConfig {
        HybridIntradayRuntimeConfig {
            symbol: "IMOEXF".to_string(),
            profile: HybridIntradayProfile::BaselineRuntimeHybrid,
            mr_variant: MeanReversionVariant::ClassicPrevDayRange,
            mr_gate_policy: MrGatePolicy::Disabled,
            risk_gate_mode: RiskGateMode::Disabled,
            risk_gate_seed_file: None,
            risk_gate_ledger_key: None,
            model_session_start_time: None,
            model_session_end_time: None,
            qty: 1.0,
            live_order_style: MarketBuyAndCloseLiveOrderStyle::Market,
            tick_size: 0.5,
            marketable_limit_offset_ticks: 0,
            timezone_offset_hours: 3,
            session_close_hour: 23,
            session_close_minute: 49,
            weekends_off: true,
            stop_end_buffer_sec: 60,
            repair_deadline_sec: 180,
            sl_escalate_timeout_sec: 30,
            max_repair_retries: 3,
            repair_backoff_base_sec: 5,
            repair_backoff_max_sec: 60,
            pending_timeout_sec: 30,
            partial_entry_fill_timeout_ms: 3_000,
            mr_config: MeanReversionConfig::default(),
            breakout_config: IntradayBreakoutConfig::default(),
            orchestrator_config: HybridOrchestratorConfig::default(),
        }
    }

    fn risk_gate_test_config() -> HybridIntradayRuntimeConfig {
        let mut cfg = test_config();
        cfg.profile = HybridIntradayProfile::ImoexfPrimaryRiskgateHigh180Lb120;
        cfg.mr_variant = MeanReversionVariant::High180;
        cfg.mr_gate_policy = MrGatePolicy::ShadowPnlLb120Positive;
        cfg.risk_gate_mode = RiskGateMode::NormalAppend;
        cfg
    }

    fn enabled_risk_gate_state(enabled: bool) -> RiskGateRuntimeState {
        RiskGateRuntimeState {
            profile_id: "imoexf_primary_high180_lb120".to_string(),
            last_finalized_session_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 5),
            rolling_sum_lb120: Some(if enabled { 1.0 } else { -1.0 }),
            mr_enabled_current_session: Some(enabled),
            mr_enabled_next_session: Some(enabled),
            ledger_rows_count: 120,
        }
    }

    fn test_ctx(position_qty: Option<f64>) -> StrategyCtx {
        StrategyCtx {
            strategy_id: "hyb-test".to_string(),
            portfolio: "demo".to_string(),
            exchange: "MOEX".to_string(),
            symbol: "IMOEXF".to_string(),
            tick_size: 0.5,
            trade_mode: TradeMode::Live,
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders: true,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty,
            event_ts_utc: 0,
            now_ts_utc: 0,
            last_bar_ts: Some(1),
        }
    }

    fn test_ctx_with_phase(
        position_qty: Option<f64>,
        phase: crate::live_guard::GatewayPhase,
    ) -> StrategyCtx {
        let mut ctx = test_ctx(position_qty);
        ctx.gateway_phase = phase;
        ctx
    }

    fn tag(owner: &str, cycle: &str, role: &str) -> String {
        format!("HYB|sid=hyb-test|c={cycle}|o={owner}|r={role}")
    }

    fn ts_local(y: i32, mo: u32, d: u32, h: u32, m: u32, s: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap_or(chrono::NaiveDate::MIN)
            .and_hms_opt(h, m, s)
            .unwrap_or(chrono::NaiveDateTime::MIN)
            .and_utc()
            .timestamp()
            - 3 * 3600
    }

    fn test_bar(ts_utc: i64, close: f64, origin: DataOrigin) -> BarEvent {
        test_bar_ohlc(ts_utc, close, close, close, close, origin)
    }

    fn test_bar_ohlc(
        ts_utc: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        origin: DataOrigin,
    ) -> BarEvent {
        BarEvent {
            symbol: "IMOEXF".to_string(),
            close_time_utc: ts_utc,
            close,
            o: open,
            h: high,
            l: low,
            v: 1.0,
            origin,
        }
    }

    #[test]
    fn author41_boundary_short_variant_emits_short_bracket_entry() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::Author41BoundaryShort;
        let strategy = HybridIntradayRuntimeStrategy::new(cfg);

        let dt_local = chrono::NaiveDate::from_ymd_opt(2026, 5, 28)
            .expect("date")
            .and_hms_opt(10, 0, 0)
            .expect("time");
        let signal = strategy
            .author41_boundary_short_entry_signal(dt_local, 100.2, 100.0, 2.0)
            .expect("author41 short entry");

        assert_eq!(signal.owner, Owner::MeanReversion);
        assert_eq!(signal.side, Side::Short);
        assert_eq!(signal.entry_style, EntryStyle::Bracket);
        assert_eq!(signal.reason, ReasonCode::MorningMeanReversionShort);
        assert_eq!(signal.take_price, Some(99.96));
        assert_eq!(signal.stop_price, Some(101.16));
    }

    #[test]
    fn author41_near_zero_take_after_rounding_suppresses_live_entry() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::Author41BoundaryShort;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(2.0);
        strategy.entry_ready = true;
        strategy.last_day_local =
            Some(chrono::NaiveDate::from_ymd_opt(2026, 5, 28).unwrap_or(chrono::NaiveDate::MIN));

        let intents = strategy.on_bar(
            &test_ctx(Some(0.0)),
            &test_bar_ohlc(
                ts_local(2026, 5, 28, 10, 0, 0),
                100.2,
                100.3,
                99.9,
                100.2,
                DataOrigin::Live,
            ),
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
    }

    #[test]
    fn author41_entry_with_sufficient_rounded_take_distance_is_allowed() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::Author41BoundaryShort;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(2600.0);
        strategy.prev_day_range = Some(60.0);
        strategy.entry_ready = true;
        strategy.last_day_local =
            Some(chrono::NaiveDate::from_ymd_opt(2026, 5, 28).unwrap_or(chrono::NaiveDate::MIN));

        let intents = strategy.on_bar(
            &test_ctx(Some(0.0)),
            &test_bar_ohlc(
                ts_local(2026, 5, 28, 10, 0, 0),
                2601.0,
                2602.0,
                2599.0,
                2601.0,
                DataOrigin::Live,
            ),
        );

        assert_eq!(intents.len(), 1);
        assert!(strategy.pending_entry.is_some());
    }

    #[test]
    fn author41_boundary_short_variant_time_cutoff_exit_after_20_00() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::Author41BoundaryShort;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.current_owner = Some(Owner::MeanReversion);

        let dt_local = chrono::NaiveDate::from_ymd_opt(2026, 5, 28)
            .expect("date")
            .and_hms_opt(20, 0, 0)
            .expect("time");
        assert_eq!(
            strategy.author41_boundary_short_exit_reason(dt_local),
            Some(ReasonCode::MeanRevTimeCutoff)
        );
    }

    #[test]
    fn high180_live_variant_emits_midpoint_bracket_entry() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::High180;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(4.0);
        strategy.entry_ready = true;
        strategy.last_day_local =
            Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 6).unwrap_or(chrono::NaiveDate::MIN));

        let ctx = test_ctx(Some(0.0));
        let intents = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 10, 0),
                99.7,
                102.0,
                99.7,
                99.7,
                DataOrigin::Live,
            ),
        );

        assert_eq!(intents.len(), 1);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Entry
            }] if matches!(intent.as_ref(), Intent::Market { side: OrderSide::Buy, .. })
        ));
        let pending = strategy.pending_entry.expect("pending high180 entry");
        assert_eq!(pending.owner, Owner::MeanReversion);
        assert_eq!(pending.entry_style, EntryStyle::Bracket);
        assert_eq!(pending.take_price, Some(100.85));
        assert!((pending.stop_price.unwrap_or_default() - 91.65).abs() <= 1e-9);
    }

    #[test]
    fn high180_near_zero_take_after_rounding_suppresses_live_entry() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::High180;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(4.0);
        strategy.entry_ready = true;
        strategy.last_day_local =
            Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 6).unwrap_or(chrono::NaiveDate::MIN));

        let intents = strategy.on_bar(
            &test_ctx(Some(0.0)),
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 10, 0),
                99.8,
                101.0,
                99.0,
                99.8,
                DataOrigin::Live,
            ),
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
    }

    #[test]
    fn high180_live_variant_forces_exit_after_max_hold() {
        let mut cfg = test_config();
        cfg.mr_variant = MeanReversionVariant::High180;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        let entry_ts = ts_local(2026, 1, 6, 9, 0, 0);
        let cycle_id = strategy.next_cycle_id(entry_ts);
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(cycle_id);
        strategy.last_day_local =
            Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 6).unwrap_or(chrono::NaiveDate::MIN));
        strategy.orchestrator.restore(
            crate::strategies::hybrid_intraday::orchestrator::HybridSnapshot {
                state: crate::strategies::hybrid_intraday::HybridState::Open,
                current_owner: Some(Owner::MeanReversion),
                current_side: Some(Side::Long),
                has_pending_entry: false,
                overnight_exit_armed_date: None,
            },
        );

        let ctx = test_ctx(Some(1.0));
        let intents = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 12, 0, 0),
                100.0,
                100.5,
                99.5,
                100.0,
                DataOrigin::Live,
            ),
        );

        assert_eq!(intents.len(), 1);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Exit
            }] if matches!(intent.as_ref(), Intent::Market { side: OrderSide::Sell, .. })
        ));
        assert_eq!(
            strategy.pending_exit.expect("pending exit").reason,
            ReasonCode::MeanRevTimeCutoff
        );
    }

    #[test]
    fn risk_gate_shadow_finalizes_previous_regular_session_on_rollover() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(risk_gate_test_config());
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(4.0);
        strategy.entry_ready = true;
        let ctx = test_ctx(Some(0.0));

        let _ = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 0, 0),
                100.2,
                101.0,
                99.0,
                100.2,
                DataOrigin::Live,
            ),
        );
        let _ = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 10, 0),
                99.9,
                100.1,
                99.8,
                99.9,
                DataOrigin::Live,
            ),
        );
        let _ = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 7, 9, 0, 0),
                100.0,
                100.5,
                99.5,
                100.0,
                DataOrigin::Live,
            ),
        );

        let finalizations = strategy.risk_gate_session_finalizations();
        assert_eq!(finalizations.len(), 1);
        assert_eq!(
            finalizations[0].session_date,
            chrono::NaiveDate::from_ymd_opt(2026, 1, 6).unwrap_or(chrono::NaiveDate::MIN)
        );
        assert_eq!(finalizations[0].shadow_trade_count, 1);
        assert!((finalizations[0].shadow_pnl_points - 0.2).abs() < 1e-9);
    }

    #[test]
    fn risk_gate_shadow_ack_clears_pending_finalization_and_state() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(risk_gate_test_config());
        let session_date =
            chrono::NaiveDate::from_ymd_opt(2026, 1, 6).unwrap_or(chrono::NaiveDate::MIN);
        strategy
            .pending_risk_gate_finalizations
            .push(RiskGateSessionFinalization {
                session_date,
                shadow_pnl_points: 1.0,
                shadow_trade_count: 1,
            });
        strategy.sync_state();

        strategy.acknowledge_risk_gate_session_finalizations(&[session_date]);

        assert!(strategy.risk_gate_session_finalizations().is_empty());
        match strategy.state() {
            StrategyState::HybridIntradayRuntime {
                risk_gate_pending_session_date,
                ..
            } => assert_eq!(risk_gate_pending_session_date, &None),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn enforced_risk_gate_blocks_mr_without_state() {
        let mut cfg = risk_gate_test_config();
        cfg.risk_gate_mode = RiskGateMode::Enforced;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(4.0);
        strategy.entry_ready = true;
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 0, 0),
                99.7,
                102.0,
                99.7,
                99.7,
                DataOrigin::Live,
            ),
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
    }

    #[test]
    fn enforced_risk_gate_allows_mr_when_state_enabled() {
        let mut cfg = risk_gate_test_config();
        cfg.risk_gate_mode = RiskGateMode::Enforced;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(4.0);
        strategy.entry_ready = true;
        strategy.on_risk_gate_state(&enabled_risk_gate_state(true));
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 0, 0),
                99.7,
                102.0,
                99.7,
                99.7,
                DataOrigin::Live,
            ),
        );

        assert_eq!(intents.len(), 1);
        assert!(strategy.pending_entry.is_some());
    }

    #[test]
    fn enforced_risk_gate_blocks_mr_when_state_disabled() {
        let mut cfg = risk_gate_test_config();
        cfg.risk_gate_mode = RiskGateMode::Enforced;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(4.0);
        strategy.entry_ready = true;
        strategy.on_risk_gate_state(&enabled_risk_gate_state(false));
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.on_bar(
            &ctx,
            &test_bar_ohlc(
                ts_local(2026, 1, 6, 9, 0, 0),
                100.2,
                101.0,
                99.0,
                99.9,
                DataOrigin::Live,
            ),
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
    }

    #[test]
    fn submit_exit_uses_current_position_qty_without_flip() {
        let mut cfg = test_config();
        cfg.qty = 10.0;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        let ctx = test_ctx(Some(1.0));
        let intents = strategy.map_action_to_intents(
            &ctx,
            1,
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            Intent::Classified {
                intent,
                intent_class,
            } => {
                assert_eq!(*intent_class, IntentClass::Exit);
                match intent.as_ref() {
                    Intent::Market {
                        qty, side, comment, ..
                    } => {
                        assert!((*qty - 1.0).abs() <= f64::EPSILON);
                        assert_eq!(*side, OrderSide::Sell);
                        let comment = comment.clone().unwrap_or_default();
                        assert!(comment.contains("HYB|sid=hyb-test|"));
                        assert!(comment.contains("|o=MR|"));
                        assert!(comment.contains("|r=EXIT"));
                    }
                    other => panic!("unexpected base intent: {other:?}"),
                }
            }
            other => panic!("expected classified intent, got {other:?}"),
        }
    }

    #[test]
    fn marketable_limit_live_entry_uses_place_intent_and_place_request_id() {
        let mut cfg = test_config();
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.entry_ready = true;
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.map_action_to_intents_with_reference(
            &ctx,
            100,
            true,
            true,
            100.0,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );

        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Entry
            }] if matches!(intent.as_ref(), Intent::Place { price, qty, side: OrderSide::Buy, .. } if (*qty - 1.0).abs() <= f64::EPSILON && (*price - 100.5).abs() <= 1e-9)
        ));
        let expected_request_id = crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ctx.symbol,
            "place",
            100,
            0,
        );
        assert_eq!(strategy.pending_entry_request_id, Some(expected_request_id));
    }

    #[test]
    fn marketable_limit_live_exit_uses_place_intent_and_place_request_id() {
        let mut cfg = test_config();
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        let ctx = test_ctx(Some(1.0));

        let intents = strategy.map_action_to_intents_with_reference(
            &ctx,
            200,
            true,
            true,
            101.0,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );

        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Exit
            }] if matches!(intent.as_ref(), Intent::Place { price, qty, side: OrderSide::Sell, .. } if (*qty - 1.0).abs() <= f64::EPSILON && (*price - 100.5).abs() <= 1e-9)
        ));
        let expected_request_id = crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ctx.symbol,
            "place",
            200,
            0,
        );
        assert_eq!(strategy.pending_exit_request_id, Some(expected_request_id));
    }

    #[test]
    fn warmup_blocks_entry_until_prev_day_range_ready() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        let entry_action = Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
            owner: Owner::MeanReversion,
            side: Side::Long,
            entry_style: crate::strategies::hybrid_intraday::EntryStyle::Market,
            reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
            stop_price: None,
            take_price: None,
        });

        let intents_blocked =
            strategy.map_action_to_intents(&ctx, 1, false, false, entry_action.clone());
        assert!(intents_blocked.is_empty());

        strategy.entry_ready = true;
        let intents_ready = strategy.map_action_to_intents(&ctx, 2, true, true, entry_action);
        assert_eq!(intents_ready.len(), 1);
        match &intents_ready[0] {
            Intent::Classified { intent, .. } => match intent.as_ref() {
                Intent::Market { comment, .. } => {
                    let comment = comment.clone().unwrap_or_default();
                    assert!(comment.contains("HYB|sid=hyb-test|"));
                    assert!(comment.contains("|o=MR|"));
                    assert!(comment.contains("|r=ENTRY"));
                }
                other => panic!("unexpected base intent: {other:?}"),
            },
            other => panic!("expected classified intent, got {other:?}"),
        }
    }

    #[test]
    fn warmup_status_flag_flips_when_prev_day_range_becomes_available() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));

        let _ = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 2, 10, 0, 0), 100.0, DataOrigin::History),
        );
        assert_eq!(strategy.last_warmup_log, Some(false));
        assert!(!strategy.entry_ready);

        let _ = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 3, 10, 0, 0), 101.0, DataOrigin::History),
        );
        assert_eq!(strategy.last_warmup_log, Some(true));
        assert!(strategy.entry_ready);
        assert_eq!(strategy.prev_day_range, Some(0.0));
    }

    #[test]
    fn weekend_bars_do_not_become_monday_anchor_when_weekends_off() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));

        let friday = test_bar_ohlc(
            ts_local(2026, 4, 17, 23, 40, 0),
            100.0,
            110.0,
            90.0,
            100.0,
            DataOrigin::History,
        );
        let saturday = test_bar_ohlc(
            ts_local(2026, 4, 18, 10, 0, 0),
            200.0,
            220.0,
            180.0,
            200.0,
            DataOrigin::History,
        );
        let sunday = test_bar_ohlc(
            ts_local(2026, 4, 19, 10, 0, 0),
            300.0,
            330.0,
            270.0,
            300.0,
            DataOrigin::History,
        );
        let monday = test_bar_ohlc(
            ts_local(2026, 4, 20, 9, 0, 0),
            101.0,
            102.0,
            100.0,
            101.0,
            DataOrigin::History,
        );

        let _ = strategy.on_bar(&ctx, &friday);
        assert_eq!(strategy.last_bar_close, Some(100.0));

        let _ = strategy.on_bar(&ctx, &saturday);
        let _ = strategy.on_bar(&ctx, &sunday);
        assert_eq!(strategy.last_bar_close, Some(100.0));
        assert_eq!(
            strategy.last_processed_bar_ts,
            Some(ts_local(2026, 4, 19, 10, 0, 0))
        );

        let _ = strategy.on_bar(&ctx, &monday);
        assert_eq!(strategy.prev_day_close, Some(100.0));
        assert_eq!(strategy.prev_day_range, Some(20.0));
        assert_eq!(
            strategy.last_day_local,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 20).unwrap())
        );
    }

    #[test]
    fn breakout_eod_guard_exits_on_late_same_day_bar_without_changing_signal_time() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::IntradayBreakout);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(ts_local(2026, 4, 17, 12, 0, 0)));
        strategy.last_day_local = Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 17).unwrap());
        strategy.current_day_high = Some(101.0);
        strategy.current_day_low = Some(99.0);
        strategy.last_bar_close = Some(100.0);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(2.0);

        let intents = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 17, 23, 40, 0), 100.5, DataOrigin::Live),
        );

        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Exit
            }] if matches!(intent.as_ref(), Intent::Market { qty, side: OrderSide::Sell, .. } if (*qty - 1.0).abs() <= f64::EPSILON)
        ));
        assert_eq!(
            strategy.pending_exit.map(|pending| pending.owner),
            Some(Owner::IntradayBreakout)
        );
        assert_eq!(
            strategy.pending_exit.map(|pending| pending.reason),
            Some(ReasonCode::BreakoutEodExit)
        );
    }

    #[test]
    fn breakout_eod_guard_does_not_move_baseline_exit_before_2330() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::IntradayBreakout);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(ts_local(2026, 4, 17, 12, 0, 0)));
        strategy.last_day_local = Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 17).unwrap());
        strategy.current_day_high = Some(101.0);
        strategy.current_day_low = Some(99.0);
        strategy.last_bar_close = Some(100.0);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(2.0);

        let intents = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 17, 23, 20, 0), 100.5, DataOrigin::Live),
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_exit_request_id.is_none());
    }

    #[test]
    fn breakout_eod_guard_rescues_carried_position_on_next_regular_day() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(-1.0));
        strategy.current_owner = Some(Owner::IntradayBreakout);
        strategy.current_side = Some(Side::Short);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(ts_local(2026, 4, 17, 18, 0, 0)));
        strategy.last_day_local = Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 17).unwrap());
        strategy.current_day_high = Some(101.0);
        strategy.current_day_low = Some(99.0);
        strategy.last_bar_close = Some(100.0);
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(2.0);

        let intents = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 20, 9, 10, 0), 98.5, DataOrigin::Live),
        );

        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Exit
            }] if matches!(intent.as_ref(), Intent::Market { qty, side: OrderSide::Buy, .. } if (*qty - 1.0).abs() <= f64::EPSILON)
        ));
        assert_eq!(
            strategy.pending_exit.map(|pending| pending.reason),
            Some(ReasonCode::BreakoutEodExit)
        );
    }

    #[test]
    fn terminal_historical_order_does_not_seed_stale_cycle_for_new_bo_entry() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));

        let _ = strategy.on_order(
            &ctx,
            &OrderEvent {
                order_id: 2033126196669077802,
                request_id: None,
                symbol: "IMOEXF".to_string(),
                status: "filled".to_string(),
                side: "sell".to_string(),
                order_type: "limit".to_string(),
                qty: 1.0,
                filled: 1.0,
                price: 2650.5,
                existing: true,
                comment: Some(tag("BO", "69f04ce000", "ENTRY")),
                ts_utc: ts_local(2026, 4, 28, 9, 0, 0),
            },
        );
        assert!(strategy.active_cycle_id.is_none());

        strategy.entry_ready = true;
        let entry_ts = ts_local(2026, 5, 1, 12, 50, 0);
        let intents = strategy.map_action_to_intents(
            &ctx,
            entry_ts,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::IntradayBreakout,
                side: Side::Short,
                entry_style: EntryStyle::Market,
                reason: ReasonCode::BreakoutShort,
                stop_price: None,
                take_price: None,
            }),
        );
        assert_eq!(intents.len(), 1);
        assert_eq!(
            strategy.active_cycle_local_day(),
            Some(chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap())
        );

        let _ = strategy.on_position(
            &ctx,
            &PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 2650.5,
                ts_utc: entry_ts + 1,
            },
        );

        let mut actions = Vec::new();
        strategy.append_breakout_no_overnight_guard(
            &mut actions,
            chrono::NaiveDate::from_ymd_opt(2026, 5, 1)
                .unwrap()
                .and_hms_opt(13, 0, 0)
                .unwrap(),
            Some(chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()),
            true,
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn working_tagged_order_can_restore_active_cycle() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));

        let _ = strategy.on_order(
            &ctx,
            &OrderEvent {
                order_id: 111,
                request_id: None,
                symbol: "IMOEXF".to_string(),
                status: "working".to_string(),
                side: "sell".to_string(),
                order_type: "limit".to_string(),
                qty: 1.0,
                filled: 0.0,
                price: 101.0,
                existing: true,
                comment: Some(tag("MR", "abc1230001", "TP")),
                ts_utc: 1_700_000_301,
            },
        );

        assert_eq!(
            strategy
                .active_cycle_id
                .map(|id| HybridIntradayRuntimeStrategy::format_cycle_id(&id)),
            Some("abc1230001".to_string())
        );
        assert!(strategy.working_orders.contains(&111));
    }

    #[test]
    fn publish_gate_blocks_pending_mutation_when_not_publishable() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx_with_phase(Some(0.0), crate::live_guard::GatewayPhase::SyncingHistory);
        strategy.entry_ready = true;
        let intents = strategy.map_action_to_intents(
            &ctx,
            100,
            false,
            false,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );
        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
        assert!(strategy.pending_entry_request_id.is_none());
        assert!(strategy.pending_entry_created_ts_utc.is_none());
    }

    #[test]
    fn paper_mode_emits_entry_intent_without_pending_mutation() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.entry_ready = true;
        let mut ctx = test_ctx(Some(0.0));
        ctx.trade_mode = TradeMode::Paper;
        ctx.gateway_phase = crate::live_guard::GatewayPhase::SyncingHistory;

        let intents = strategy.map_action_to_intents(
            &ctx,
            200,
            true,
            false,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );

        assert_eq!(intents.len(), 1);
        assert!(strategy.pending_entry.is_none());
        assert!(strategy.pending_entry_request_id.is_none());
        assert!(strategy.pending_entry_created_ts_utc.is_none());
    }

    #[test]
    fn dropped_entry_releases_orchestrator_pending_state() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.orchestrator.restore(
            crate::strategies::hybrid_intraday::orchestrator::HybridSnapshot {
                state: crate::strategies::hybrid_intraday::orchestrator::HybridState::Pending,
                current_owner: Some(Owner::MeanReversion),
                current_side: Some(Side::Long),
                has_pending_entry: true,
                overnight_exit_armed_date: None,
            },
        );
        strategy.entry_ready = false;
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.map_action_to_intents(
            &ctx,
            300,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );

        assert!(intents.is_empty());
        let snapshot = strategy.orchestrator.snapshot();
        assert_eq!(
            snapshot.state,
            crate::strategies::hybrid_intraday::orchestrator::HybridState::Flat
        );
        assert!(!snapshot.has_pending_entry);
    }

    #[test]
    fn pending_entry_gc_clears_stale_tail_when_flat_without_orders() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        strategy.orchestrator.restore(
            crate::strategies::hybrid_intraday::orchestrator::HybridSnapshot {
                state: crate::strategies::hybrid_intraday::orchestrator::HybridState::Pending,
                current_owner: Some(Owner::MeanReversion),
                current_side: Some(Side::Long),
                has_pending_entry: true,
                overnight_exit_armed_date: None,
            },
        );
        let _ = strategy.map_action_to_intents(
            &ctx,
            10,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.pending_entry_request_id.is_some());
        strategy.clear_stale_pending_tail(10 + test_config().pending_timeout_sec as i64 + 1, 0.0);
        assert!(strategy.pending_entry.is_none());
        assert!(strategy.pending_entry_request_id.is_none());
        assert!(strategy.pending_entry_created_ts_utc.is_none());
        let snapshot = strategy.orchestrator.snapshot();
        assert_eq!(
            snapshot.state,
            crate::strategies::hybrid_intraday::orchestrator::HybridState::Flat
        );
    }

    #[test]
    fn pending_entry_gc_keeps_edge_timeout_for_next_bar_fill() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        let created_ts = 10;
        let _ = strategy.map_action_to_intents(
            &ctx,
            created_ts,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.pending_entry_request_id.is_some());
        strategy
            .clear_stale_pending_tail(created_ts + test_config().pending_timeout_sec as i64, 0.0);
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.pending_entry_request_id.is_some());
    }

    #[test]
    fn runtime_state_restore_clears_flat_stale_pending_tail() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let mut ctx = test_ctx(Some(0.0));
        ctx.now_ts_utc = ts_local(2026, 1, 5, 10, 2, 0);
        strategy.entry_ready = true;
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.orchestrator.restore(
            crate::strategies::hybrid_intraday::orchestrator::HybridSnapshot {
                state: crate::strategies::hybrid_intraday::orchestrator::HybridState::Pending,
                current_owner: Some(Owner::MeanReversion),
                current_side: Some(Side::Long),
                has_pending_entry: true,
                overnight_exit_armed_date: None,
            },
        );
        let _ = strategy.map_action_to_intents(
            &ctx,
            ts_local(2026, 1, 5, 10, 0, 0),
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );

        let _ = strategy.on_runtime_state_restored(
            &ctx,
            &crate::RuntimeStateRestored {
                known_order_ids: Vec::new(),
                pending_requests: Vec::new(),
            },
        );

        assert!(strategy.pending_entry.is_none());
        assert!(strategy.pending_entry_request_id.is_none());
        assert!(strategy.pending_exit_request_id.is_none());
        assert!(strategy.pending_tp_request_id.is_none());
        assert!(strategy.pending_sl_request_id.is_none());
        assert!(strategy.current_owner.is_none());
        assert!(strategy.current_side.is_none());
        assert!(strategy.active_cycle_id.is_none());
        let snapshot = strategy.orchestrator.snapshot();
        assert_eq!(
            snapshot.state,
            crate::strategies::hybrid_intraday::orchestrator::HybridState::Flat
        );
    }

    #[test]
    fn startup_replay_guard_warms_stale_live_bar_without_emitting_intents() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let mut ctx = test_ctx(Some(0.0));
        ctx.now_ts_utc = ts_local(2026, 1, 5, 10, 2, 0);
        ctx.last_bar_ts = Some(ts_local(2026, 1, 5, 9, 59, 0));
        strategy.entry_ready = true;
        strategy.prev_day_close = Some(101.95);
        strategy.prev_day_range = Some(2.0);
        strategy.last_bar_close = Some(102.0);
        strategy.last_day_local = chrono::NaiveDate::from_ymd_opt(2026, 1, 5);

        let _ = strategy.on_runtime_state_restored(
            &ctx,
            &crate::RuntimeStateRestored {
                known_order_ids: Vec::new(),
                pending_requests: Vec::new(),
            },
        );

        let stale_bar = test_bar(ts_local(2026, 1, 5, 10, 0, 0), 101.95, DataOrigin::Live);
        let stale_intents = strategy.on_bar(&ctx, &stale_bar);
        assert!(stale_intents.is_empty());
        assert_eq!(
            strategy.startup_live_replay_boundary_ts_utc,
            Some(ts_local(2026, 1, 5, 10, 1, 0))
        );
        assert_eq!(strategy.startup_replay_suppressed_bars, 1);
        assert_eq!(
            strategy.last_processed_bar_ts,
            Some(stale_bar.close_time_utc)
        );

        let fresh_bar = test_bar(ts_local(2026, 1, 5, 10, 1, 0), 101.90, DataOrigin::Live);
        let fresh_intents = strategy.on_bar(&ctx, &fresh_bar);
        assert_eq!(fresh_intents.len(), 1);
        assert!(strategy.startup_live_replay_boundary_ts_utc.is_none());
        assert_eq!(strategy.startup_replay_suppressed_bars, 0);
    }

    #[test]
    fn restore_disables_entry_ready_when_prev_day_range_missing() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.entry_ready = true;
        strategy.prev_day_range = None;
        strategy.orchestrator.intraday_breakout.restore_snapshot(
            crate::strategies::hybrid_intraday::intraday_breakout::IntradayBreakoutSnapshot {
                cur_day_date: None,
                cur_day_high: None,
                cur_day_low: None,
                cur_day_close: None,
                yesterday_close: None,
                yesterday_range: None,
                yesterday_return: None,
                day_before_close: None,
                was_long_today: false,
                was_short_today: false,
                today_start: None,
            },
        );
        strategy.sync_state();
        let mut restored = strategy.state().clone();
        if let StrategyState::HybridIntradayRuntime {
            entry_ready,
            prev_day_range,
            prev_day_close,
            ..
        } = &mut restored
        {
            *entry_ready = true;
            *prev_day_range = None;
            *prev_day_close = None;
        } else {
            panic!("unexpected state variant");
        }
        strategy.set_state(restored);
        assert!(!strategy.entry_ready);
        match strategy.state() {
            StrategyState::HybridIntradayRuntime { entry_ready, .. } => assert!(!entry_ready),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn restore_recovers_day_features_from_state() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let mut restored = StrategyState::HybridIntradayRuntime {
            active_cycle_id: None,
            next_cycle_seq: 0,
            last_position_qty: 0.0,
            current_owner: None,
            current_side: None,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_entry_cycle_id: None,
            pending_entry_request_id: None,
            pending_entry_created_ts_utc: None,
            deferred_entry_owner: None,
            deferred_entry_side: None,
            deferred_entry_cycle_id: None,
            deferred_entry_entry_style: None,
            deferred_entry_reason: None,
            deferred_entry_stop_price: None,
            deferred_entry_take_price: None,
            deferred_entry_ts_utc: None,
            deferred_entry_request_id: None,
            pending_exit_request_id: None,
            pending_exit_created_ts_utc: None,
            deferred_exit_owner: None,
            deferred_exit_reason: None,
            deferred_exit_cycle_id: None,
            deferred_exit_ts_utc: None,
            deferred_exit_request_id: None,
            pending_tp_request_id: None,
            pending_tp_created_ts_utc: None,
            pending_sl_request_id: None,
            pending_sl_created_ts_utc: None,
            tp_order_id: None,
            sl_stop_order_id: None,
            sl_exchange_order_id: None,
            sl_triggered_ts: None,
            mr_take_price: None,
            mr_stop_price: None,
            repair_deadline_ts: None,
            next_repair_at_ts: None,
            repair_backoff_level: 0,
            repair_attempts: 0,
            safe_mode_close_only: false,
            safe_mode_reason: None,
            entry_ready: true,
            last_bar_close: Some(101.5),
            prev_day_close: Some(100.0),
            last_day_local: Some("2026-03-08".to_string()),
            current_day_high: Some(102.0),
            current_day_low: Some(100.5),
            current_day_close: Some(101.5),
            prev_day_range: Some(12.5),
            prev_day_return: Some(0.02),
            day_before_close: Some(98.0),
            today_start_local: Some("2026-03-08T09:00:00".to_string()),
            was_long_today: true,
            was_short_today: false,
            overnight_exit_armed_date: Some("2026-03-08".to_string()),
            risk_gate_shadow_session_date: None,
            risk_gate_shadow_pnl_points: 0.0,
            risk_gate_shadow_trade_count: 0,
            risk_gate_shadow_entry_ts_utc: None,
            risk_gate_shadow_entry_price: None,
            risk_gate_shadow_side: None,
            risk_gate_shadow_target_price: None,
            risk_gate_shadow_stop_price: None,
            risk_gate_pending_session_date: None,
            risk_gate_pending_shadow_pnl_points: 0.0,
            risk_gate_pending_shadow_trade_count: 0,
            risk_gate_mr_enabled_current_session: None,
            risk_gate_rolling_sum_lb120: None,
            risk_gate_last_finalized_session_date: None,
            risk_gate_ledger_rows_count: 0,
        };
        strategy.set_state(restored.clone());
        assert!(strategy.entry_ready);
        assert_eq!(strategy.last_bar_close, Some(101.5));
        assert_eq!(strategy.prev_day_close(), Some(100.0));
        assert_eq!(strategy.current_day_high, Some(102.0));
        assert_eq!(strategy.current_day_low, Some(100.5));
        assert_eq!(strategy.prev_day_range, Some(12.5));
        assert_eq!(
            strategy.last_day_local,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap())
        );
        let breakout = strategy.breakout_snapshot();
        assert_eq!(breakout.cur_day_close, Some(101.5));
        assert_eq!(breakout.yesterday_close, Some(100.0));
        assert_eq!(breakout.yesterday_return, Some(0.02));
        assert_eq!(breakout.day_before_close, Some(98.0));
        assert_eq!(
            breakout.today_start,
            Some(
                chrono::NaiveDate::from_ymd_opt(2026, 3, 8)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap()
            )
        );
        assert!(breakout.was_long_today);
        assert!(!breakout.was_short_today);
        assert_eq!(
            strategy.orchestrator.snapshot().overnight_exit_armed_date,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap())
        );

        if let StrategyState::HybridIntradayRuntime {
            last_day_local,
            prev_day_range,
            today_start_local,
            ..
        } = &mut restored
        {
            *last_day_local = Some("bad-date".to_string());
            *prev_day_range = Some(11.0);
            *today_start_local = Some("bad-datetime".to_string());
        }
        strategy.set_state(restored);
        assert_eq!(strategy.last_day_local, None);
        assert_eq!(strategy.prev_day_range, Some(11.0));
        assert_eq!(strategy.breakout_snapshot().today_start, None);
    }

    #[test]
    fn mean_reversion_uses_previous_day_close_not_last_bar_close() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.set_state(StrategyState::HybridIntradayRuntime {
            active_cycle_id: None,
            next_cycle_seq: 0,
            last_position_qty: 0.0,
            current_owner: None,
            current_side: None,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_entry_cycle_id: None,
            pending_entry_request_id: None,
            pending_entry_created_ts_utc: None,
            deferred_entry_owner: None,
            deferred_entry_side: None,
            deferred_entry_cycle_id: None,
            deferred_entry_entry_style: None,
            deferred_entry_reason: None,
            deferred_entry_stop_price: None,
            deferred_entry_take_price: None,
            deferred_entry_ts_utc: None,
            deferred_entry_request_id: None,
            pending_exit_request_id: None,
            pending_exit_created_ts_utc: None,
            deferred_exit_owner: None,
            deferred_exit_reason: None,
            deferred_exit_cycle_id: None,
            deferred_exit_ts_utc: None,
            deferred_exit_request_id: None,
            pending_tp_request_id: None,
            pending_tp_created_ts_utc: None,
            pending_sl_request_id: None,
            pending_sl_created_ts_utc: None,
            tp_order_id: None,
            sl_stop_order_id: None,
            sl_exchange_order_id: None,
            sl_triggered_ts: None,
            mr_take_price: None,
            mr_stop_price: None,
            repair_deadline_ts: None,
            next_repair_at_ts: None,
            repair_backoff_level: 0,
            repair_attempts: 0,
            safe_mode_close_only: false,
            safe_mode_reason: None,
            entry_ready: true,
            last_bar_close: Some(99.90),
            prev_day_close: Some(100.0),
            last_day_local: Some("2026-03-09".to_string()),
            current_day_high: Some(100.0),
            current_day_low: Some(99.5),
            current_day_close: Some(99.90),
            prev_day_range: Some(2.0),
            prev_day_return: Some(0.01),
            day_before_close: Some(99.0),
            today_start_local: Some("2026-03-09T09:00:00".to_string()),
            was_long_today: false,
            was_short_today: false,
            overnight_exit_armed_date: None,
            risk_gate_shadow_session_date: None,
            risk_gate_shadow_pnl_points: 0.0,
            risk_gate_shadow_trade_count: 0,
            risk_gate_shadow_entry_ts_utc: None,
            risk_gate_shadow_entry_price: None,
            risk_gate_shadow_side: None,
            risk_gate_shadow_target_price: None,
            risk_gate_shadow_stop_price: None,
            risk_gate_pending_session_date: None,
            risk_gate_pending_shadow_pnl_points: 0.0,
            risk_gate_pending_shadow_trade_count: 0,
            risk_gate_mr_enabled_current_session: None,
            risk_gate_rolling_sum_lb120: None,
            risk_gate_last_finalized_session_date: None,
            risk_gate_ledger_rows_count: 0,
        });
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 3, 9, 10, 0, 0), 99.95, DataOrigin::Live),
        );

        assert_eq!(intents.len(), 1);
        assert!(strategy.pending_entry.is_some());
        assert_eq!(strategy.prev_day_close(), Some(100.0));
    }

    #[test]
    fn weekend_bar_does_not_update_weekday_state_or_emit_intents() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.set_state(StrategyState::HybridIntradayRuntime {
            active_cycle_id: None,
            next_cycle_seq: 0,
            last_position_qty: 0.0,
            current_owner: None,
            current_side: None,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_entry_cycle_id: None,
            pending_entry_request_id: None,
            pending_entry_created_ts_utc: None,
            deferred_entry_owner: None,
            deferred_entry_side: None,
            deferred_entry_cycle_id: None,
            deferred_entry_entry_style: None,
            deferred_entry_reason: None,
            deferred_entry_stop_price: None,
            deferred_entry_take_price: None,
            deferred_entry_ts_utc: None,
            deferred_entry_request_id: None,
            pending_exit_request_id: None,
            pending_exit_created_ts_utc: None,
            deferred_exit_owner: None,
            deferred_exit_reason: None,
            deferred_exit_cycle_id: None,
            deferred_exit_ts_utc: None,
            deferred_exit_request_id: None,
            pending_tp_request_id: None,
            pending_tp_created_ts_utc: None,
            pending_sl_request_id: None,
            pending_sl_created_ts_utc: None,
            tp_order_id: None,
            sl_stop_order_id: None,
            sl_exchange_order_id: None,
            sl_triggered_ts: None,
            mr_take_price: None,
            mr_stop_price: None,
            repair_deadline_ts: None,
            next_repair_at_ts: None,
            repair_backoff_level: 0,
            repair_attempts: 0,
            safe_mode_close_only: false,
            safe_mode_reason: None,
            entry_ready: true,
            last_bar_close: Some(99.90),
            prev_day_close: Some(100.0),
            last_day_local: Some("2026-03-06".to_string()),
            current_day_high: Some(100.0),
            current_day_low: Some(99.5),
            current_day_close: Some(99.90),
            prev_day_range: Some(2.0),
            prev_day_return: Some(0.01),
            day_before_close: Some(99.0),
            today_start_local: Some("2026-03-06T09:00:00".to_string()),
            was_long_today: false,
            was_short_today: false,
            overnight_exit_armed_date: None,
            risk_gate_shadow_session_date: None,
            risk_gate_shadow_pnl_points: 0.0,
            risk_gate_shadow_trade_count: 0,
            risk_gate_shadow_entry_ts_utc: None,
            risk_gate_shadow_entry_price: None,
            risk_gate_shadow_side: None,
            risk_gate_shadow_target_price: None,
            risk_gate_shadow_stop_price: None,
            risk_gate_pending_session_date: None,
            risk_gate_pending_shadow_pnl_points: 0.0,
            risk_gate_pending_shadow_trade_count: 0,
            risk_gate_mr_enabled_current_session: None,
            risk_gate_rolling_sum_lb120: None,
            risk_gate_last_finalized_session_date: None,
            risk_gate_ledger_rows_count: 0,
        });
        let ctx = test_ctx(Some(0.0));

        let intents = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 3, 7, 10, 0, 0), 99.95, DataOrigin::Live),
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
        assert_eq!(strategy.last_bar_close, Some(99.90));
        assert_eq!(
            strategy.last_day_local,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap())
        );
        assert_eq!(strategy.current_day_high, Some(100.0));
        assert_eq!(strategy.current_day_low, Some(99.5));
        assert_eq!(
            strategy.last_processed_bar_ts,
            Some(ts_local(2026, 3, 7, 10, 0, 0))
        );
    }

    #[test]
    fn stop_end_uses_same_day_session_close_plus_buffer() {
        let strategy = HybridIntradayRuntimeStrategy::new(test_config());
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
            .timestamp();
        assert_eq!(
            strategy.resolve_stop_end_unix_time(created_ts_utc),
            expected
        );
    }

    #[test]
    fn stop_end_weekend_rolls_to_next_trading_day_when_weekends_off() {
        let strategy = HybridIntradayRuntimeStrategy::new(test_config());
        // 2025-01-05 10:00:00 UTC = Sunday 13:00:00 MSK
        let created_ts_utc = chrono::NaiveDate::from_ymd_opt(2025, 1, 5)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        // Monday 2025-01-06 23:49 MSK => 20:49 UTC + 60 sec buffer
        let expected = chrono::NaiveDate::from_ymd_opt(2025, 1, 6)
            .unwrap()
            .and_hms_opt(20, 50, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(
            strategy.resolve_stop_end_unix_time(created_ts_utc),
            expected
        );
    }

    #[test]
    fn ignores_foreign_working_orders_for_live_order_gate() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        let _ = strategy.on_order(
            &ctx,
            &OrderEvent {
                order_id: 42,
                request_id: None,
                symbol: "IMOEXF".to_string(),
                status: "working".to_string(),
                side: "buy".to_string(),
                order_type: "limit".to_string(),
                qty: 1.0,
                filled: 0.0,
                price: 100.0,
                existing: false,
                comment: None,
                ts_utc: 1,
            },
        );
        assert!(!strategy.has_live_orders());
    }

    #[test]
    fn pending_entry_is_counted_as_live_order() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        let _ = strategy.map_action_to_intents(
            &ctx,
            10,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: crate::strategies::hybrid_intraday::EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );
        assert!(strategy.has_live_orders());
    }

    #[test]
    fn pending_exit_is_counted_as_live_order() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        let _ = strategy.map_action_to_intents(
            &ctx,
            10,
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );
        assert!(strategy.pending_exit_request_id.is_some());
        assert!(strategy.has_live_orders());
    }

    #[test]
    fn submit_exit_is_suppressed_while_pending_exit_is_active() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        let first = strategy.map_action_to_intents(
            &ctx,
            10,
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );
        assert_eq!(first.len(), 1);
        let pending = strategy
            .pending_exit_request_id
            .expect("pending exit request");

        let second = strategy.map_action_to_intents(
            &ctx,
            11,
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );
        assert!(second.is_empty());
        assert_eq!(strategy.pending_exit_request_id, Some(pending));
    }

    #[test]
    fn ack_negative_clears_pending_exit_lifecycle() {
        let mk_strategy = || {
            let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
            let ctx = test_ctx(Some(1.0));
            let _ = strategy.map_action_to_intents(
                &ctx,
                10,
                true,
                true,
                Action::SubmitExit {
                    owner: Owner::MeanReversion,
                    reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
                },
            );
            (strategy, ctx)
        };

        {
            let (mut strategy, ctx) = mk_strategy();
            let req = strategy.pending_exit_request_id.expect("pending req");
            let _ = strategy.on_ack(&ctx, &CommandAck::rejected(req, "x", "y"));
            assert!(strategy.pending_exit_request_id.is_none());
            assert!(strategy.pending_exit_created_ts_utc.is_none());
        }
        {
            let (mut strategy, ctx) = mk_strategy();
            let req = strategy.pending_exit_request_id.expect("pending req");
            let _ = strategy.on_ack(&ctx, &CommandAck::expired(req, "x"));
            assert!(strategy.pending_exit_request_id.is_none());
            assert!(strategy.pending_exit_created_ts_utc.is_none());
        }
        {
            let (mut strategy, ctx) = mk_strategy();
            let req = strategy.pending_exit_request_id.expect("pending req");
            let _ = strategy.on_ack(&ctx, &CommandAck::error(req, "x", "y"));
            assert!(strategy.pending_exit_request_id.is_none());
            assert!(strategy.pending_exit_created_ts_utc.is_none());
        }
    }

    #[test]
    fn ack_reject_clears_only_matching_pending_entry() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        let _ = strategy.map_action_to_intents(
            &ctx,
            100,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: crate::strategies::hybrid_intraday::EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );
        let matching = strategy
            .pending_entry_request_id
            .expect("entry request id must exist");
        let stale = uuid::Uuid::new_v4();

        let _ = strategy.on_ack(&ctx, &CommandAck::rejected(stale, "x", "y"));
        assert!(strategy.pending_entry.is_some());

        let _ = strategy.on_ack(&ctx, &CommandAck::rejected(matching, "x", "y"));
        assert!(strategy.pending_entry.is_none());
        assert!(strategy.pending_entry_request_id.is_none());
    }

    #[test]
    fn trading_window_closed_entry_reject_enters_deferred_state() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        strategy.prev_day_range = Some(10.0);
        let _ = strategy.map_action_to_intents(
            &ctx,
            ts_local(2026, 4, 2, 14, 4, 0),
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Bracket,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: Some(99.0),
                take_price: Some(101.0),
            }),
        );
        let req = strategy.pending_entry_request_id.expect("entry request id");

        let _ = strategy.on_ack(
            &ctx,
            &CommandAck::rejected(req, "trading_window_closed", "validation failed"),
        );

        assert!(strategy.pending_entry_request_id.is_none());
        assert!(strategy.pending_entry.is_none());
        assert!(!strategy.safe_mode_close_only);
        let deferred = strategy
            .deferred_entry
            .as_ref()
            .expect("deferred entry must be present");
        assert_eq!(deferred.signal.owner, Owner::MeanReversion);
        assert_eq!(deferred.signal.side, Side::Long);
        assert_eq!(
            deferred.signal.reason,
            crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong
        );
        assert_eq!(deferred.original_request_id, req);
    }

    #[test]
    fn deferred_entry_reissues_only_after_live_ready_returns() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        strategy.prev_day_close = Some(100.0);
        strategy.prev_day_range = Some(10.0);
        let _ = strategy.map_action_to_intents(
            &ctx,
            ts_local(2026, 4, 2, 14, 4, 0),
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: None,
                take_price: None,
            }),
        );
        let req = strategy.pending_entry_request_id.expect("entry request id");
        let _ = strategy.on_ack(
            &ctx,
            &CommandAck::rejected(req, "trading_window_closed", "validation failed"),
        );

        let blocked_ctx =
            test_ctx_with_phase(Some(0.0), crate::live_guard::GatewayPhase::SyncingHistory);
        let blocked = strategy.on_bar(
            &blocked_ctx,
            &test_bar(ts_local(2026, 4, 2, 14, 5, 0), 100.0, DataOrigin::Live),
        );
        assert!(blocked.is_empty());
        assert!(strategy.deferred_entry.is_some());
        assert!(strategy.pending_entry_request_id.is_none());

        let resumed = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 2, 14, 6, 0), 100.5, DataOrigin::Live),
        );
        assert_eq!(resumed.len(), 1);
        assert!(strategy.deferred_entry.is_none());
        assert!(strategy.pending_entry_request_id.is_some());
    }

    #[test]
    fn trading_window_closed_exit_reject_enters_deferred_state() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        let _ = strategy.map_action_to_intents(
            &ctx,
            ts_local(2026, 4, 2, 14, 4, 0),
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );
        let req = strategy.pending_exit_request_id.expect("exit request id");

        let _ = strategy.on_ack(
            &ctx,
            &CommandAck::rejected(req, "trading_window_closed", "validation failed"),
        );

        assert!(strategy.pending_exit_request_id.is_none());
        let deferred = strategy
            .deferred_exit
            .expect("deferred exit must be present");
        assert_eq!(deferred.owner, Owner::MeanReversion);
        assert_eq!(
            deferred.reason,
            crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff
        );
        assert_eq!(deferred.original_request_id, req);
    }

    #[test]
    fn pending_exit_request_id_uses_effective_created_ts() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let mut ctx = test_ctx(Some(1.0));
        ctx.event_ts_utc = ts_local(2026, 4, 2, 14, 4, 55);
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");

        let created_ts_utc =
            strategy.effective_created_ts_utc(&ctx, ts_local(2026, 4, 2, 14, 4, 0));
        let _ = strategy.map_action_to_intents(
            &ctx,
            created_ts_utc,
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );

        let expected_request_id = strategy.live_request_id(&ctx, ctx.event_ts_utc, OrderSide::Sell);
        assert_eq!(strategy.pending_exit_request_id, Some(expected_request_id));
        assert_eq!(strategy.pending_exit_created_ts_utc, Some(ctx.event_ts_utc));
    }

    #[test]
    fn deferred_exit_reissues_after_live_ready_returns_until_flat() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        let _ = strategy.map_action_to_intents(
            &ctx,
            ts_local(2026, 4, 2, 14, 4, 0),
            true,
            true,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );
        let req = strategy.pending_exit_request_id.expect("exit request id");
        let _ = strategy.on_ack(
            &ctx,
            &CommandAck::rejected(req, "trading_window_closed", "validation failed"),
        );

        let blocked_ctx =
            test_ctx_with_phase(Some(1.0), crate::live_guard::GatewayPhase::SyncingHistory);
        let blocked = strategy.on_bar(
            &blocked_ctx,
            &test_bar(ts_local(2026, 4, 2, 14, 5, 0), 100.0, DataOrigin::Live),
        );
        assert!(blocked.is_empty());
        assert!(strategy.deferred_exit.is_some());
        assert!(strategy.pending_exit_request_id.is_none());

        let resumed = strategy.on_bar(
            &ctx,
            &test_bar(ts_local(2026, 4, 2, 14, 6, 0), 99.5, DataOrigin::Live),
        );
        assert_eq!(resumed.len(), 1);
        assert!(strategy.deferred_exit.is_none());
        assert!(strategy.pending_exit_request_id.is_some());
    }

    #[test]
    fn mr_entry_fill_emits_tp_and_sl_protective_intents() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.entry_ready = true;
        let _ = strategy.map_action_to_intents(
            &ctx,
            1_700_000_000,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Bracket,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: Some(99.0),
                take_price: Some(101.0),
            }),
        );
        let pos = PositionEvent {
            symbol: "IMOEXF".to_string(),
            qty: 1.0,
            existing: false,
            avg_price: 100.0,
            ts_utc: 1_700_000_060,
        };
        let intents = strategy.on_position(&ctx, &pos);
        assert_eq!(intents.len(), 2);
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::ProtectiveRepair
                } if matches!(intent.as_ref(), Intent::Place { .. })
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::ProtectiveRepair
                } if matches!(intent.as_ref(), Intent::CreateStopLimit { .. })
            )
        }));
        assert!(strategy.pending_tp_request_id.is_some());
        assert!(strategy.pending_tp_created_ts_utc.is_some());
        assert!(strategy.pending_sl_request_id.is_some());
        assert!(strategy.pending_sl_created_ts_utc.is_some());
    }

    #[test]
    fn flat_transition_emits_cancel_all_protection() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.last_position_qty = 1.0;
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.tp_order_id = Some(111);
        strategy.sl_stop_order_id = Some("abc".to_string());
        strategy.sl_exchange_order_id = Some(222);
        let ctx = test_ctx(Some(1.0));
        let pos = PositionEvent {
            symbol: "IMOEXF".to_string(),
            qty: 0.0,
            existing: false,
            avg_price: 0.0,
            ts_utc: 1_700_000_120,
        };
        let intents = strategy.on_position(&ctx, &pos);
        assert_eq!(intents.len(), 3);
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::CancelCleanup
                } if matches!(intent.as_ref(), Intent::Cancel { order_id: 111 })
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::CancelCleanup
                } if matches!(intent.as_ref(), Intent::DeleteStopLimit { order_id, .. } if order_id == "abc")
            )
        }));
    }

    #[test]
    fn tp_fill_waits_for_broker_flat_before_canceling_stop() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Short);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        let ctx = test_ctx(Some(-2.0));
        let order = OrderEvent {
            order_id: 111,
            symbol: "IMOEXF".to_string(),
            status: "filled".to_string(),
            qty: 1.0,
            filled: 1.0,
            comment: Some(tag("MR", "abc1230001", "TP")),
            ..OrderEvent::default()
        };

        let intents = strategy.on_order(&ctx, &order);

        assert!(intents.is_empty());
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-1"));
    }

    #[test]
    fn failed_flat_stop_cleanup_retries_are_bounded_and_reset_after_cancel() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(0.0));
        strategy.last_position_qty = 0.0;
        strategy.working_stop_orders.insert("sl-1".to_string());

        for expected_attempt in 1..=HybridIntradayRuntimeStrategy::MAX_CLEANUP_STOP_RETRIES {
            let intents = strategy.on_ack(
                &ctx,
                &CommandAck::error(uuid::Uuid::new_v4(), "cws_error", "transport_error"),
            );
            assert_eq!(strategy.cleanup_stop_retry_attempts, expected_attempt);
            assert_eq!(intents.len(), 1);
            assert!(matches!(
                intents[0].base_intent(),
                Intent::DeleteStopLimit {
                    order_id,
                    side: None,
                    check_duplicates: Some(true),
                } if order_id == "sl-1"
            ));
        }

        let exhausted = strategy.on_ack(
            &ctx,
            &CommandAck::error(uuid::Uuid::new_v4(), "cws_error", "transport_error"),
        );
        assert!(exhausted.is_empty());
        assert_eq!(
            strategy.cleanup_stop_retry_attempts,
            HybridIntradayRuntimeStrategy::MAX_CLEANUP_STOP_RETRIES
        );

        let canceled = strategy.on_stop_order(
            &ctx,
            &StopOrderEvent {
                stop_order_id: "sl-1".to_string(),
                exchange_order_id: None,
                symbol: "IMOEXF".to_string(),
                status: "canceled".to_string(),
                side: "sell".to_string(),
                qty: 2.0,
                filled: 0.0,
                stop_price: 99.0,
                price: 98.5,
                existing: false,
                comment: Some(tag("MR", "abc1230001", "SL")),
                end_time: None,
                ts_utc: 1_700_000_130,
            },
        );
        assert!(canceled.is_empty());
        assert!(strategy.working_stop_orders.is_empty());
        assert_eq!(strategy.cleanup_stop_retry_attempts, 0);
    }

    #[test]
    fn partial_protective_fill_waits_for_terminal_reconcile() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.last_position_qty = -2.0;
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Short);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        strategy.mark_bracket_terminal_reconcile();
        let ctx = test_ctx(Some(-1.0));
        let pos = PositionEvent {
            symbol: "IMOEXF".to_string(),
            qty: -1.0,
            existing: false,
            avg_price: 100.0,
            ts_utc: 1_700_000_120,
        };

        let intents = strategy.on_position(&ctx, &pos);

        assert!(intents.is_empty());
        assert!(!strategy.safe_mode_close_only);
        assert_eq!(strategy.last_position_qty, -1.0);
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-1"));
    }

    #[test]
    fn terminal_reconcile_timeout_emits_single_residual_flatten() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.last_position_qty = -1.0;
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Short);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        strategy.bracket_terminal_reconcile_started_ms = Some(10_000);

        let intents = strategy.on_timer(&test_ctx(Some(-1.0)), 13_001);

        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::DeleteStopLimit { order_id, .. } if order_id == "sl-1"
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::Market { qty, side: OrderSide::Buy, .. } if (*qty - 1.0).abs() <= f64::EPSILON
            )
        }));
        assert!(strategy.safe_mode_close_only);
        assert_eq!(
            strategy.safe_mode_reason.as_deref(),
            Some("bracket_terminal_reconcile_timeout")
        );

        let repeated = strategy.on_timer(&test_ctx(Some(-1.0)), 16_500);
        assert!(repeated.is_empty());
    }

    #[test]
    fn partial_mr_entry_waits_for_full_target_before_creating_bracket() {
        let mut cfg = test_config();
        cfg.qty = 3.0;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::MeanReversion,
            side: Side::Short,
            cycle_id: *b"abc1230001",
            reason: ReasonCode::MorningMeanReversionShort,
            entry_style: EntryStyle::Bracket,
            stop_price: Some(105.0),
            take_price: Some(95.0),
            target_qty: 3.0,
            partial_started_at_ms: None,
        });
        strategy.pending_entry_request_id = Some(uuid::Uuid::new_v4());

        let partial = strategy.on_position(
            &test_ctx(Some(-1.0)),
            &PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1_700_000_100,
            },
        );
        assert!(partial.is_empty());
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.tp_order_id.is_none());
        assert!(strategy.sl_stop_order_id.is_none());

        let complete = strategy.on_position(
            &test_ctx(Some(-3.0)),
            &PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: -3.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1_700_000_101,
            },
        );
        assert_eq!(complete.len(), 2);
        assert!(strategy.pending_entry.is_none());
        assert!(complete.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::Place { qty, side: OrderSide::Buy, .. }
                    if (*qty - 3.0).abs() <= f64::EPSILON
            )
        }));
        assert!(complete.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::CreateStopLimit { qty, side: OrderSide::Buy, .. }
                    if (*qty - 3.0).abs() <= f64::EPSILON
            )
        }));
    }

    #[test]
    fn bo_market_entry_partial_fill_completes_without_mr_wait_or_brackets() {
        let mut cfg = test_config();
        cfg.qty = 3.0;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::IntradayBreakout,
            side: Side::Short,
            cycle_id: *b"bo12340001",
            reason: ReasonCode::BreakoutShort,
            entry_style: EntryStyle::Market,
            stop_price: Some(105.0),
            take_price: Some(95.0),
            target_qty: 3.0,
            partial_started_at_ms: None,
        });
        strategy.pending_entry_request_id = Some(uuid::Uuid::new_v4());

        let intents = strategy.on_position(
            &test_ctx(Some(-1.0)),
            &PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1_700_000_100,
            },
        );

        assert!(intents.is_empty());
        assert!(strategy.pending_entry.is_none());
        assert_eq!(strategy.current_owner, Some(Owner::IntradayBreakout));
        assert_eq!(strategy.current_side, Some(Side::Short));
        assert_eq!(strategy.last_position_qty, -1.0);
        assert!(strategy.tp_order_id.is_none());
        assert!(strategy.sl_stop_order_id.is_none());
        assert!(strategy.pending_tp_request_id.is_none());
        assert!(strategy.pending_sl_request_id.is_none());
        assert!(!strategy.safe_mode_close_only);
    }

    #[test]
    fn bo_market_entry_partial_marker_does_not_trigger_mr_timeout_flatten() {
        let mut cfg = test_config();
        cfg.qty = 3.0;
        cfg.partial_entry_fill_timeout_ms = 3_000;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.last_position_qty = -1.0;
        strategy.working_orders.insert(111);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::IntradayBreakout,
            side: Side::Short,
            cycle_id: *b"bo12340001",
            reason: ReasonCode::BreakoutShort,
            entry_style: EntryStyle::Market,
            stop_price: Some(105.0),
            take_price: Some(95.0),
            target_qty: 3.0,
            partial_started_at_ms: Some(10_000),
        });

        let timeout = strategy.on_timer(&test_ctx(Some(-1.0)), 13_001);

        assert!(timeout.is_empty());
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.working_orders.contains(&111));
        assert!(!strategy.safe_mode_close_only);

        let completed = strategy.on_position(
            &test_ctx(Some(-1.0)),
            &PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100.0,
                ts_utc: 1_700_000_101,
            },
        );

        assert!(completed.is_empty());
        assert!(strategy.pending_entry.is_none());
        assert_eq!(strategy.current_owner, Some(Owner::IntradayBreakout));
        assert_eq!(strategy.current_side, Some(Side::Short));
        assert!(!strategy.safe_mode_close_only);
    }

    #[test]
    fn partial_mr_entry_timeout_cancels_remainder_and_flattens_partial() {
        let mut cfg = test_config();
        cfg.qty = 3.0;
        cfg.partial_entry_fill_timeout_ms = 3_000;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.last_position_qty = -1.0;
        strategy.working_orders.insert(111);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::MeanReversion,
            side: Side::Short,
            cycle_id: *b"abc1230001",
            reason: ReasonCode::MorningMeanReversionShort,
            entry_style: EntryStyle::Bracket,
            stop_price: Some(105.0),
            take_price: Some(95.0),
            target_qty: 3.0,
            partial_started_at_ms: Some(10_000),
        });

        let intents = strategy.on_timer(&test_ctx(Some(-1.0)), 13_001);

        assert!(intents
            .iter()
            .any(|intent| { matches!(intent.base_intent(), Intent::Cancel { order_id: 111 }) }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::Market { qty, side: OrderSide::Buy, .. }
                    if (*qty - 1.0).abs() <= f64::EPSILON
            )
        }));
        assert!(strategy.pending_entry.is_none());
        assert!(strategy.safe_mode_close_only);
        assert_eq!(
            strategy.safe_mode_reason.as_deref(),
            Some("partial_entry_fill_timeout")
        );
    }

    #[test]
    fn recovered_position_without_owner_enters_safe_mode_and_blocks_entry() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.entry_ready = true;
        let ctx = test_ctx(Some(0.0));
        let pos = PositionEvent {
            symbol: "IMOEXF".to_string(),
            qty: 1.0,
            existing: true,
            avg_price: 100.0,
            ts_utc: 1_700_000_200,
        };
        let _ = strategy.on_position(&ctx, &pos);
        assert!(strategy.safe_mode_close_only);

        let intents = strategy.map_action_to_intents(
            &ctx,
            1_700_000_260,
            true,
            true,
            Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Bracket,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                stop_price: Some(99.0),
                take_price: Some(101.0),
            }),
        );
        assert!(intents.is_empty());
    }

    #[test]
    fn bootstrap_adopts_working_mr_bracket_and_skips_repair() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        let mut snapshot = crate::BootstrapSnapshot {
            positions_strategy: std::collections::HashMap::new(),
            working_orders_strategy: std::collections::HashMap::new(),
            working_stop_orders_strategy: std::collections::HashMap::new(),
            snapshot_ts_utc: Some(1_700_000_300),
        };
        snapshot.positions_strategy.insert(
            "IMOEXF".to_string(),
            PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: 1.0,
                existing: true,
                avg_price: 100.0,
                ts_utc: 1_700_000_300,
            },
        );
        snapshot.working_orders_strategy.insert(
            111,
            OrderEvent {
                order_id: 111,
                request_id: None,
                symbol: "IMOEXF".to_string(),
                status: "working".to_string(),
                side: "sell".to_string(),
                order_type: "limit".to_string(),
                qty: 1.0,
                filled: 0.0,
                price: 101.0,
                existing: true,
                comment: Some(tag("MR", "abc1230001", "TP")),
                ts_utc: 1_700_000_301,
            },
        );
        snapshot.working_stop_orders_strategy.insert(
            "sl-1".to_string(),
            StopOrderEvent {
                stop_order_id: "sl-1".to_string(),
                exchange_order_id: Some(222),
                symbol: "IMOEXF".to_string(),
                status: "working".to_string(),
                side: "sell".to_string(),
                qty: 1.0,
                filled: 0.0,
                stop_price: 99.0,
                price: 98.5,
                existing: true,
                comment: Some(tag("MR", "abc1230001", "SL")),
                end_time: Some(1_700_086_400),
                ts_utc: 1_700_000_301,
            },
        );

        let _ = strategy.on_bootstrap_snapshot(&ctx, &snapshot);
        assert!(!strategy.safe_mode_close_only);
        assert_eq!(strategy.current_owner, Some(Owner::MeanReversion));
        assert_eq!(strategy.current_side, Some(Side::Long));
        assert_eq!(strategy.tp_order_id, Some(111));
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-1"));
        assert_eq!(strategy.sl_exchange_order_id, Some(222));

        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_305);
        assert!(intents.is_empty());
    }

    #[test]
    fn stop_order_lag_event_adopts_sl_state() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.pending_sl_request_id = Some(uuid::Uuid::new_v4());

        let intents = strategy.on_stop_order(
            &ctx,
            &StopOrderEvent {
                stop_order_id: "sl-lag".to_string(),
                exchange_order_id: Some(333),
                symbol: "IMOEXF".to_string(),
                status: "working".to_string(),
                side: "sell".to_string(),
                qty: 1.0,
                filled: 0.0,
                stop_price: 99.0,
                price: 98.5,
                existing: false,
                comment: Some(tag("MR", "abc1230001", "SL")),
                end_time: None,
                ts_utc: 1_700_000_320,
            },
        );
        assert!(intents.is_empty());
        assert!(strategy.pending_sl_request_id.is_none());
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-lag"));
        assert_eq!(strategy.sl_exchange_order_id, Some(333));
        assert!(strategy.working_stop_orders.contains("sl-lag"));
    }

    #[test]
    fn stop_order_triggered_keeps_exchange_order_for_escalation() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.tp_order_id = Some(111);

        let intents = strategy.on_stop_order(
            &ctx,
            &StopOrderEvent {
                stop_order_id: "sl-2".to_string(),
                exchange_order_id: Some(222),
                symbol: "IMOEXF".to_string(),
                status: "triggered".to_string(),
                side: "sell".to_string(),
                qty: 1.0,
                filled: 0.0,
                stop_price: 99.0,
                price: 98.5,
                existing: false,
                comment: Some(tag("MR", "abc1230001", "SL")),
                end_time: None,
                ts_utc: 1_700_000_120,
            },
        );
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::CancelCleanup
                } if matches!(intent.as_ref(), Intent::Cancel { order_id: 111 })
            )
        }));
        assert!(strategy.sl_stop_order_id.is_none());
        assert_eq!(strategy.sl_exchange_order_id, Some(222));
        assert_eq!(strategy.sl_triggered_ts, Some(1_700_000_120));
    }

    #[test]
    fn sl_triggered_escalation_waits_for_timeout() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.sl_triggered_ts = Some(1_700_000_100);
        strategy.tp_order_id = Some(111);
        strategy.sl_exchange_order_id = Some(222);

        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_120);
        assert!(intents.is_empty());
        assert_eq!(strategy.sl_triggered_ts, Some(1_700_000_100));
        assert_eq!(strategy.tp_order_id, Some(111));
        assert_eq!(strategy.sl_exchange_order_id, Some(222));
    }

    #[test]
    fn sl_triggered_escalation_forces_market_exit_after_timeout() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.sl_triggered_ts = Some(1_700_000_100);
        strategy.tp_order_id = Some(111);
        strategy.sl_exchange_order_id = Some(222);

        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_131);
        assert_eq!(intents.len(), 3);
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::CancelCleanup
                } if matches!(intent.as_ref(), Intent::Cancel { order_id: 111 })
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::CancelCleanup
                } if matches!(intent.as_ref(), Intent::Cancel { order_id: 222 })
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::Exit
                } if matches!(intent.as_ref(), Intent::Market { qty, side: OrderSide::Sell, .. } if (*qty - 1.0).abs() <= f64::EPSILON)
            )
        }));
        assert!(strategy.safe_mode_close_only);
        assert_eq!(
            strategy.safe_mode_reason.as_deref(),
            Some("sl_triggered_escalation")
        );
        assert!(strategy.sl_triggered_ts.is_none());
    }

    #[test]
    fn marketable_limit_sl_triggered_escalation_uses_place_exit() {
        let mut cfg = test_config();
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        let ctx = test_ctx(Some(1.0));
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(*b"abc1230001");
        strategy.last_bar_close = Some(101.0);
        strategy.sl_triggered_ts = Some(1_700_000_100);
        strategy.tp_order_id = Some(111);
        strategy.sl_exchange_order_id = Some(222);

        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_131);
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified {
                    intent,
                    intent_class: IntentClass::Exit
                } if matches!(intent.as_ref(), Intent::Place { price, qty, side: OrderSide::Sell, .. } if (*qty - 1.0).abs() <= f64::EPSILON && (*price - 100.5).abs() <= 1e-9)
            )
        }));
    }

    #[test]
    fn bootstrap_open_position_without_owner_enters_safe_mode_even_with_cycle() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.active_cycle_id = Some(*b"abc1230001");
        let ctx = test_ctx(Some(1.0));
        let mut snapshot = crate::BootstrapSnapshot {
            positions_strategy: std::collections::HashMap::new(),
            working_orders_strategy: std::collections::HashMap::new(),
            working_stop_orders_strategy: std::collections::HashMap::new(),
            snapshot_ts_utc: Some(1_700_000_400),
        };
        snapshot.positions_strategy.insert(
            "IMOEXF".to_string(),
            PositionEvent {
                symbol: "IMOEXF".to_string(),
                qty: 1.0,
                existing: true,
                avg_price: 100.0,
                ts_utc: 1_700_000_400,
            },
        );

        let _ = strategy.on_bootstrap_snapshot(&ctx, &snapshot);
        assert!(strategy.safe_mode_close_only);
        assert_eq!(
            strategy.safe_mode_reason.as_deref(),
            Some("bootstrap_position_owner_unknown")
        );
    }

    #[test]
    fn repair_is_deferred_with_backoff_when_gateway_not_live_ready() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(1_700_000_000));
        strategy.mr_take_price = Some(101.0);
        strategy.mr_stop_price = Some(99.0);
        strategy.repair_deadline_ts = Some(1_700_000_360);
        let ctx = test_ctx_with_phase(Some(1.0), crate::live_guard::GatewayPhase::SyncingHistory);

        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_100);
        assert!(intents.is_empty());
        assert!(strategy
            .next_repair_at_ts
            .is_some_and(|next| next > 1_700_000_100));
        let next = strategy.next_repair_at_ts.unwrap_or_default();
        let intents_again = strategy.maybe_emit_repair_intents(&ctx, next.saturating_sub(1));
        assert!(intents_again.is_empty());
    }

    #[test]
    fn repair_deadline_forces_market_exit_and_safe_mode() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(test_config());
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(1_700_000_000));
        strategy.mr_take_price = Some(101.0);
        strategy.mr_stop_price = Some(99.0);
        strategy.repair_deadline_ts = Some(1_700_000_100);
        let ctx = test_ctx(Some(2.0));
        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_101);
        assert_eq!(intents.len(), 1);
        assert!(strategy.safe_mode_close_only);
        assert_eq!(
            strategy.safe_mode_reason.as_deref(),
            Some("repair_deadline_force_flatten")
        );
        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Exit
            }] if matches!(intent.as_ref(), Intent::Market { qty, side: OrderSide::Sell, .. } if (*qty - 2.0).abs() <= f64::EPSILON)
        ));
    }

    #[test]
    fn marketable_limit_repair_deadline_uses_place_exit() {
        let mut cfg = test_config();
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(1_700_000_000));
        strategy.mr_take_price = Some(101.0);
        strategy.mr_stop_price = Some(99.0);
        strategy.last_bar_close = Some(101.0);
        strategy.repair_deadline_ts = Some(1_700_000_100);
        let ctx = test_ctx(Some(2.0));
        let intents = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_101);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Classified {
                intent,
                intent_class: IntentClass::Exit
            }] if matches!(intent.as_ref(), Intent::Place { price, qty, side: OrderSide::Sell, .. } if (*qty - 2.0).abs() <= f64::EPSILON && (*price - 100.5).abs() <= 1e-9)
        ));
    }

    #[test]
    fn repair_retries_exhausted_enters_safe_mode() {
        let mut cfg = test_config();
        cfg.max_repair_retries = 1;
        cfg.repair_backoff_base_sec = 1;
        cfg.repair_backoff_max_sec = 1;
        let mut strategy = HybridIntradayRuntimeStrategy::new(cfg);
        strategy.current_owner = Some(Owner::MeanReversion);
        strategy.current_side = Some(Side::Long);
        strategy.active_cycle_id = Some(strategy.next_cycle_id(1_700_000_000));
        strategy.mr_take_price = Some(101.0);
        strategy.mr_stop_price = Some(99.0);
        strategy.repair_deadline_ts = Some(1_700_010_000);
        let ctx = test_ctx(Some(1.0));
        let first = strategy.maybe_emit_repair_intents(&ctx, 1_700_000_100);
        assert!(!first.is_empty());
        let tp_req = strategy.pending_tp_request_id.expect("tp req");
        let sl_req = strategy.pending_sl_request_id.expect("sl req");
        let _ = strategy.on_ack(&ctx, &CommandAck::rejected(tp_req, "x", "y"));
        let _ = strategy.on_ack(&ctx, &CommandAck::rejected(sl_req, "x", "y"));
        strategy.repair_deadline_ts = None;
        let next_repair_at = strategy.next_repair_at_ts.unwrap_or(i64::MAX - 1);
        let second = strategy.maybe_emit_repair_intents(&ctx, next_repair_at);
        assert!(second.is_empty());
        assert!(strategy.safe_mode_close_only);
        assert_eq!(
            strategy.safe_mode_reason.as_deref(),
            Some("repair_retries_exhausted")
        );
    }
}

impl Strategy for HybridIntradayRuntimeStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            return Vec::new();
        }
        if self
            .last_processed_bar_ts
            .is_some_and(|last_ts| bar.close_time_utc <= last_ts)
        {
            return Vec::new();
        }
        let Some(dt_local) = self.utc_to_local_naive(bar.close_time_utc) else {
            return Vec::new();
        };
        let previous_day_local = self.last_day_local;
        if self.suppress_weekend_signal_generation(dt_local) {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            debug!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                ts_utc = bar.close_time_utc,
                dt_local = %dt_local,
                origin = ?bar.origin,
                "hybrid_weekend_bar_suppressed"
            );
            return Vec::new();
        }
        if let Some(reason) = self.suppress_non_model_session_bar(dt_local) {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            debug!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                ts_utc = bar.close_time_utc,
                dt_local = %dt_local,
                origin = ?bar.origin,
                reason,
                profile = ?self.config.profile,
                "hybrid_model_session_bar_suppressed"
            );
            return Vec::new();
        }
        self.update_day_aggregates(dt_local, bar.h, bar.l);
        if self.uses_high180_mr() {
            self.high180_mr.on_bar(dt_local, bar.h, bar.l);
        }
        self.log_signal_warmup_status_if_changed(dt_local, bar.close_time_utc);

        let close_prev = self.prev_day_close().unwrap_or(bar.close);
        let day_range_prev = self.prev_day_range.unwrap_or(0.0);
        self.update_risk_gate_shadow(
            dt_local,
            bar.close_time_utc,
            bar.h,
            bar.l,
            bar.close,
            close_prev,
            day_range_prev,
        );
        let has_open_position = ctx.position_qty.unwrap_or(0.0).abs() > f64::EPSILON;
        if self.suppress_startup_replay_bar(
            ctx,
            bar,
            dt_local,
            close_prev,
            day_range_prev,
            has_open_position,
        ) {
            return Vec::new();
        }
        self.last_processed_bar_ts = Some(bar.close_time_utc);
        self.last_bar_close = Some(bar.close);
        self.clear_stale_pending_tail(bar.close_time_utc, ctx.position_qty.unwrap_or(0.0));
        let can_emit = self.can_emit_now(ctx, bar.origin == DataOrigin::Live);
        let can_execute = self.can_execute_now(ctx, bar.origin == DataOrigin::Live);
        let created_ts_utc = self.effective_created_ts_utc(ctx, bar.close_time_utc);
        let reference_price = self.live_reference_price();
        let bar_input =
            self.bar_input(dt_local, bar, close_prev, day_range_prev, has_open_position);
        if self.deferred_exit.is_some() {
            self.orchestrator.warm_bar(bar_input);
            if let Some(intents) = self.maybe_reissue_deferred_exit(
                ctx,
                created_ts_utc,
                can_emit,
                can_execute,
                reference_price,
            ) {
                self.sync_state();
                return intents;
            }
        }
        if self.deferred_entry.is_some() {
            self.orchestrator.warm_bar(bar_input);
            if let Some(intents) = self.maybe_reissue_deferred_entry(
                ctx,
                created_ts_utc,
                dt_local,
                can_emit,
                can_execute,
                reference_price,
            ) {
                self.sync_state();
                return intents;
            }
        }
        let mut actions = if self.uses_mr_override() {
            let mr_entry_signal = if self.uses_high180_mr() {
                if self.mr_gate_allows_current_session() {
                    self.high180_entry_signal(dt_local, bar.close, close_prev, day_range_prev)
                } else {
                    None
                }
            } else {
                self.author41_boundary_short_entry_signal(
                    dt_local,
                    bar.close,
                    close_prev,
                    day_range_prev,
                )
            };
            let mr_entry_signal =
                self.filter_near_zero_mr_bracket_entry(dt_local, bar.close, mr_entry_signal);
            let mr_exit_reason = if self.uses_high180_mr() {
                self.high180_exit_reason(dt_local)
            } else {
                self.author41_boundary_short_exit_reason(dt_local)
            };
            self.orchestrator
                .on_bar_with_mr_override(bar_input, mr_entry_signal, mr_exit_reason)
        } else {
            self.orchestrator.on_bar(bar_input)
        };
        self.append_breakout_no_overnight_guard(
            &mut actions,
            dt_local,
            previous_day_local,
            has_open_position,
        );
        if !actions.is_empty() {
            let action_labels = actions
                .iter()
                .map(Self::action_debug_label)
                .collect::<Vec<_>>();
            let should_log_info = bar.origin == DataOrigin::Live || can_emit || can_execute;
            if should_log_info {
                info!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    ts_utc = bar.close_time_utc,
                    dt_local = %dt_local,
                    origin = ?bar.origin,
                    close = bar.close,
                    close_prev,
                    day_range_prev,
                    entry_ready = self.entry_ready,
                    has_open_position = has_open_position,
                    has_live_orders = self.has_live_orders(),
                    can_emit,
                    can_execute,
                    actions_count = action_labels.len(),
                    actions = ?action_labels,
                    "hybrid actions generated"
                );
            } else {
                debug!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    ts_utc = bar.close_time_utc,
                    dt_local = %dt_local,
                    origin = ?bar.origin,
                    close = bar.close,
                    close_prev,
                    day_range_prev,
                    entry_ready = self.entry_ready,
                    has_open_position = has_open_position,
                    has_live_orders = self.has_live_orders(),
                    can_emit,
                    can_execute,
                    actions_count = action_labels.len(),
                    actions = ?action_labels,
                    "hybrid actions generated"
                );
            }
        }
        let mut intents = self.maybe_emit_repair_intents(ctx, created_ts_utc);
        intents.extend(actions.into_iter().flat_map(|action| {
            self.map_action_to_intents(ctx, created_ts_utc, can_emit, can_execute, action)
        }));
        self.sync_state();
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        self.log_request_id_skew_if_pending(
            "entry",
            self.pending_entry_request_id,
            self.pending_entry_created_ts_utc,
            ack,
        );
        self.log_request_id_skew_if_pending(
            "exit",
            self.pending_exit_request_id,
            self.pending_exit_created_ts_utc,
            ack,
        );
        if Some(ack.request_id) == self.pending_entry_request_id {
            if matches!(
                ack.status,
                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
            ) {
                self.orchestrator.on_order_rejected("entry");
                if Self::is_window_closed_recoverable_reject(ack) {
                    if let Some(entry) = self.pending_entry.take() {
                        self.deferred_entry = Some(DeferredEntry {
                            signal: crate::strategies::hybrid_intraday::EntrySignal {
                                owner: entry.owner,
                                side: entry.side,
                                entry_style: entry.entry_style,
                                reason: entry.reason,
                                stop_price: entry.stop_price,
                                take_price: entry.take_price,
                            },
                            cycle_id: entry.cycle_id,
                            deferred_ts_utc: ack.processed_ts_utc,
                            original_request_id: ack.request_id,
                        });
                        info!(
                            target: "strategy_runtime::hybrid_intraday_runtime",
                            action = "entry_deferred_trading_window_closed",
                            owner = ?entry.owner,
                            side = ?entry.side,
                            cycle_id = %Self::format_cycle_id(&entry.cycle_id),
                            request_id = %ack.request_id,
                            error_code = ?ack.error_code,
                            error_msg = ?ack.error_msg,
                        );
                    }
                } else {
                    self.pending_entry = None;
                }
                self.pending_entry_request_id = None;
                self.pending_entry_created_ts_utc = None;
                self.active_cycle_id = None;
                self.tp_order_id = None;
                self.sl_stop_order_id = None;
                self.sl_exchange_order_id = None;
                if !Self::is_window_closed_recoverable_reject(ack) {
                    self.enter_safe_mode("entry_rejected");
                }
            } else if matches!(
                ack.status,
                AckStatus::Accepted | AckStatus::Confirmed | AckStatus::Duplicate
            ) {
                // entry still pending until PositionEvent confirms qty transition.
            }
            self.sync_state();
            return Vec::new();
        }
        if Some(ack.request_id) == self.pending_exit_request_id {
            if matches!(
                ack.status,
                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
            ) {
                self.orchestrator.on_order_rejected("exit");
                if Self::is_window_closed_recoverable_reject(ack) {
                    let cycle_id = self
                        .active_cycle_id
                        .unwrap_or_else(|| self.next_cycle_id(ack.processed_ts_utc));
                    self.deferred_exit = self.pending_exit.map(|pending| DeferredExit {
                        owner: pending.owner,
                        reason: pending.reason,
                        cycle_id,
                        deferred_ts_utc: ack.processed_ts_utc,
                        original_request_id: ack.request_id,
                    });
                    if let Some(deferred) = self.deferred_exit {
                        info!(
                            target: "strategy_runtime::hybrid_intraday_runtime",
                            action = "exit_deferred_trading_window_closed",
                            owner = ?deferred.owner,
                            reason = ?deferred.reason,
                            cycle_id = %Self::format_cycle_id(&deferred.cycle_id),
                            request_id = %ack.request_id,
                            error_code = ?ack.error_code,
                            error_msg = ?ack.error_msg,
                        );
                    }
                }
                self.pending_exit = None;
                self.pending_exit_request_id = None;
                self.pending_exit_created_ts_utc = None;
            }
            self.sync_state();
            return Vec::new();
        }
        if Some(ack.request_id) == self.pending_tp_request_id {
            if matches!(
                ack.status,
                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
            ) {
                self.pending_tp_request_id = None;
                self.pending_tp_created_ts_utc = None;
                self.schedule_next_repair(ack.processed_ts_utc);
            }
            self.sync_state();
            return Vec::new();
        }
        if Some(ack.request_id) == self.pending_sl_request_id {
            if matches!(
                ack.status,
                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
            ) {
                self.pending_sl_request_id = None;
                self.pending_sl_created_ts_utc = None;
                self.schedule_next_repair(ack.processed_ts_utc);
            }
            self.sync_state();
            return Vec::new();
        }
        if matches!(
            ack.status,
            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
        ) && self.last_position_qty.abs() <= f64::EPSILON
            && !self.working_stop_orders.is_empty()
        {
            warn!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                action = "cleanup_ack_error_with_active_stop_while_flat",
                request_id = %ack.request_id,
                error_code = ?ack.error_code,
                error_msg = ?ack.error_msg,
                working_stop_orders_count = self.working_stop_orders.len(),
                sl_stop_order_id = ?self.sl_stop_order_id,
                "cleanup command failed while strategy is flat and stop orders are still active"
            );
            if self.cleanup_stop_retry_attempts < Self::MAX_CLEANUP_STOP_RETRIES {
                self.cleanup_stop_retry_attempts =
                    self.cleanup_stop_retry_attempts.saturating_add(1);
                let mut stop_order_ids =
                    self.working_stop_orders.iter().cloned().collect::<Vec<_>>();
                stop_order_ids.sort();
                info!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    action = "sibling_cleanup_retry",
                    retry_attempt = self.cleanup_stop_retry_attempts,
                    max_retries = Self::MAX_CLEANUP_STOP_RETRIES,
                    stop_order_ids = ?stop_order_ids,
                    "retrying active sibling stop cleanup while broker position is flat"
                );
                self.sync_state();
                return stop_order_ids
                    .into_iter()
                    .map(|order_id| {
                        Intent::DeleteStopLimit {
                            order_id,
                            side: None,
                            check_duplicates: Some(true),
                        }
                        .with_class(IntentClass::CancelCleanup)
                    })
                    .collect();
            }
            warn!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                action = "sibling_cleanup_retry_exhausted",
                retry_attempts = self.cleanup_stop_retry_attempts,
                working_stop_orders_count = self.working_stop_orders.len(),
                "active sibling stop cleanup retries exhausted while broker position is flat"
            );
        }
        // Stale/foreign ack: ignore.
        Vec::new()
    }

    fn on_order(&mut self, ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent> {
        if ord.symbol != self.config.symbol {
            return Vec::new();
        }
        let is_ours = self.is_our_tag(ctx, ord.comment.as_deref());
        if !is_ours {
            return Vec::new();
        }
        let intents = Vec::new();
        let status = ord.status.to_ascii_lowercase();
        if !Self::is_terminal_order_status(&status) {
            self.ensure_active_cycle_from_comment(ord.comment.as_deref());
        }
        let tag = Self::parse_hybrid_tag(ord.comment.as_deref());
        if tag.as_ref().and_then(|v| v.role) == Some(TagRole::Tp) {
            self.pending_tp_request_id = None;
            self.pending_tp_created_ts_utc = None;
            if ord.order_id > 0 {
                self.tp_order_id = Some(ord.order_id);
            }
            if matches!(
                status.as_str(),
                "filled" | "canceled" | "cancelled" | "expired" | "rejected"
            ) {
                if status == "filled" {
                    self.mark_bracket_terminal_reconcile();
                }
                self.tp_order_id = None;
            }
        }
        if Self::is_terminal_order_status(&status) {
            self.working_orders.remove(&ord.order_id);
        } else if ord.order_id > 0 {
            self.working_orders.insert(ord.order_id);
        }
        self.sync_state();
        intents
    }

    fn on_stop_order(&mut self, ctx: &StrategyCtx, ord: &StopOrderEvent) -> Vec<Intent> {
        if ord.symbol != self.config.symbol {
            return Vec::new();
        }
        let is_ours = self.is_our_tag(ctx, ord.comment.as_deref());
        if !is_ours {
            return Vec::new();
        }
        let mut intents = Vec::new();
        let status = ord.status.to_ascii_lowercase();
        if !Self::is_terminal_stop_order_status(&status) {
            self.ensure_active_cycle_from_comment(ord.comment.as_deref());
        }
        let tag = Self::parse_hybrid_tag(ord.comment.as_deref());
        if tag.as_ref().and_then(|v| v.role) == Some(TagRole::Sl) {
            self.pending_sl_request_id = None;
            self.pending_sl_created_ts_utc = None;
            if !ord.stop_order_id.trim().is_empty() {
                self.sl_stop_order_id = Some(ord.stop_order_id.clone());
            }
            if let Some(exchange_order_id) = ord.exchange_order_id {
                self.sl_exchange_order_id = Some(exchange_order_id);
            }
            if matches!(
                status.as_str(),
                "filled" | "executed" | "triggered" | "done" | "completed"
            ) {
                self.mark_bracket_terminal_reconcile();
                self.sl_triggered_ts = Some(ord.ts_utc.max(0));
                if let Some(tp_order_id) = self.tp_order_id.take() {
                    intents.push(
                        Intent::Cancel {
                            order_id: tp_order_id,
                        }
                        .with_class(IntentClass::CancelCleanup),
                    );
                }
            }
            if matches!(
                status.as_str(),
                "filled"
                    | "canceled"
                    | "cancelled"
                    | "expired"
                    | "rejected"
                    | "executed"
                    | "triggered"
                    | "done"
                    | "completed"
            ) {
                self.sl_stop_order_id = None;
            }
            if matches!(
                status.as_str(),
                "canceled" | "cancelled" | "expired" | "rejected"
            ) {
                self.sl_exchange_order_id = None;
            }
        }
        if Self::is_terminal_stop_order_status(&status) {
            self.working_stop_orders.remove(&ord.stop_order_id);
            if self.working_stop_orders.is_empty() && self.cleanup_stop_retry_attempts > 0 {
                info!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    action = "sibling_cleanup_confirmed",
                    stop_order_id = %ord.stop_order_id,
                    status = %status,
                    retry_attempts = self.cleanup_stop_retry_attempts,
                    "all strategy-owned sibling stop orders are terminal"
                );
                self.cleanup_stop_retry_attempts = 0;
            }
        } else if !ord.stop_order_id.trim().is_empty() {
            self.working_stop_orders.insert(ord.stop_order_id.clone());
        }
        if self.last_position_qty.abs() <= f64::EPSILON && !self.working_stop_orders.is_empty() {
            warn!(
                target: "strategy_runtime::hybrid_intraday_runtime",
                action = "stop_order_active_while_flat",
                stop_order_id = %ord.stop_order_id,
                status = %status,
                working_stop_orders_count = self.working_stop_orders.len(),
                sl_stop_order_id = ?self.sl_stop_order_id,
                "stop order remains active while strategy position is flat"
            );
        }
        self.sync_state();
        intents
    }

    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }
        let mut intents = Vec::new();
        let prev = self.last_position_qty;
        let cur = pos.qty;
        let mut completes_pending_entry = false;
        if let Some(mut entry) = self.pending_entry {
            if cur.abs() > f64::EPSILON {
                if !Self::partial_entry_progress_is_valid(entry, prev, cur) {
                    let reason = if cur.signum() != Self::expected_entry_qty(entry).signum() {
                        "partial_entry_sign_mismatch"
                    } else if cur.abs() > entry.target_qty + f64::EPSILON {
                        "partial_entry_overfill"
                    } else {
                        "partial_entry_position_reduced"
                    };
                    intents.extend(self.emit_broker_residual_emergency_exit(ctx, pos, reason));
                    self.pending_entry = None;
                    self.last_position_qty = cur;
                    self.sync_state();
                    return intents;
                }
                if Self::waits_for_partial_entry_target(entry)
                    && cur.abs() + f64::EPSILON < entry.target_qty
                {
                    if entry.partial_started_at_ms.is_none() {
                        entry.partial_started_at_ms = Some(Utc::now().timestamp_millis());
                    }
                    self.pending_entry = Some(entry);
                    self.last_position_qty = cur;
                    info!(
                        target: "strategy_runtime::hybrid_intraday_runtime",
                        action = "partial_entry_progress",
                        owner = ?entry.owner,
                        side = ?entry.side,
                        broker_qty = cur,
                        target_qty = entry.target_qty,
                        "MR entry partially filled; waiting for target before creating bracket"
                    );
                    self.sync_state();
                    return intents;
                }
                completes_pending_entry = true;
            }
        }
        if !completes_pending_entry
            && prev.abs() > f64::EPSILON
            && cur.abs() > f64::EPSILON
            && (prev - cur).abs() > f64::EPSILON
        {
            if self.bracket_terminal_reconcile_active(Utc::now().timestamp_millis()) {
                self.last_position_qty = cur;
                self.lifecycle_bracket_terminal_reconcile_log(prev, cur);
                self.sync_state();
                return intents;
            }
            let reason = if prev.signum() != cur.signum() {
                "broker_position_sign_flip"
            } else {
                "broker_position_size_changed"
            };
            intents.extend(self.emit_broker_residual_emergency_exit(ctx, pos, reason));
            self.last_position_qty = cur;
            self.sync_state();
            return intents;
        }
        if (prev.abs() <= f64::EPSILON || completes_pending_entry) && cur.abs() > f64::EPSILON {
            self.cleanup_stop_retry_attempts = 0;
            let filled = self.pending_entry.take();
            self.clear_deferred_entry();
            if let Some(entry) = filled {
                self.current_owner = Some(entry.owner);
                self.current_side = Some(entry.side);
                self.active_cycle_id = Some(entry.cycle_id);
                self.pending_entry_request_id = None;
                self.pending_entry_created_ts_utc = None;
                self.repair_attempts = 0;
                self.repair_backoff_level = 0;
                self.next_repair_at_ts = Some(pos.ts_utc);
                self.repair_deadline_ts = Some(
                    pos.ts_utc
                        .saturating_add(self.config.repair_deadline_sec as i64),
                );
                self.mr_take_price = entry.take_price;
                self.mr_stop_price = entry.stop_price;
                self.orchestrator
                    .on_order_filled("entry", entry.owner, Some(entry.side));
                intents.extend(self.emit_mr_bracket_intents(ctx, pos, entry));
            } else {
                self.current_owner = None;
                self.current_side = None;
                if self.active_cycle_id.is_none() {
                    self.active_cycle_id = Some(self.next_cycle_id(pos.ts_utc));
                }
                self.enter_safe_mode("recovered_position_owner_unknown");
                if !pos.existing {
                    intents.extend(self.emit_broker_residual_emergency_exit(
                        ctx,
                        pos,
                        "unexpected_broker_residual",
                    ));
                }
            }
        } else if prev.abs() > f64::EPSILON && cur.abs() <= f64::EPSILON {
            self.cleanup_stop_retry_attempts = 0;
            let closing_side = self.current_side;
            let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
            self.orchestrator.on_order_filled("exit", owner, None);
            self.current_owner = None;
            self.current_side = None;
            self.pending_entry = None;
            self.pending_entry_request_id = None;
            self.pending_entry_created_ts_utc = None;
            self.clear_deferred_entry();
            self.pending_exit = None;
            self.pending_exit_request_id = None;
            self.pending_exit_created_ts_utc = None;
            self.clear_deferred_exit();
            self.clear_bracket_terminal_reconcile();
            self.active_cycle_id = None;
            self.safe_mode_close_only = false;
            self.safe_mode_reason = None;
            self.sl_triggered_ts = None;
            intents.extend(self.emit_cancel_all_protection(closing_side));
            self.reset_repair_tracking();
        }
        self.last_position_qty = cur;
        self.sync_state();
        intents
    }

    fn on_timer(&mut self, ctx: &StrategyCtx, now_ts_utc_ms: i64) -> Vec<Intent> {
        let mut intents = self.emit_bracket_reconcile_timeout_exit(ctx, now_ts_utc_ms);
        intents.extend(self.emit_partial_entry_timeout_exit(ctx, now_ts_utc_ms));
        intents
    }

    fn on_bootstrap_snapshot(
        &mut self,
        ctx: &StrategyCtx,
        snapshot: &crate::BootstrapSnapshot,
    ) -> Vec<Intent> {
        self.working_orders.clear();
        self.working_stop_orders.clear();
        self.tp_order_id = None;
        self.sl_stop_order_id = None;
        self.sl_exchange_order_id = None;
        let mut owner_from_tags: Option<Owner> = None;

        for (order_id, order) in &snapshot.working_orders_strategy {
            if order.symbol != self.config.symbol {
                continue;
            }
            if self.is_our_tag(ctx, order.comment.as_deref()) {
                self.working_orders.insert(*order_id);
                self.ensure_active_cycle_from_comment(order.comment.as_deref());
                if let Some(tag) = Self::parse_hybrid_tag(order.comment.as_deref()) {
                    if owner_from_tags.is_none() {
                        owner_from_tags = tag.owner;
                    } else if tag.owner.is_some() && tag.owner != owner_from_tags {
                        self.enter_safe_mode("bootstrap_conflicting_owner_tags");
                    }
                    if tag.role == Some(TagRole::Tp) {
                        self.tp_order_id = Some(*order_id);
                    }
                }
            }
        }
        for (stop_order_id, stop_order) in &snapshot.working_stop_orders_strategy {
            if stop_order.symbol != self.config.symbol {
                continue;
            }
            if self.is_our_tag(ctx, stop_order.comment.as_deref()) {
                self.working_stop_orders.insert(stop_order_id.clone());
                self.ensure_active_cycle_from_comment(stop_order.comment.as_deref());
                if let Some(tag) = Self::parse_hybrid_tag(stop_order.comment.as_deref()) {
                    if owner_from_tags.is_none() {
                        owner_from_tags = tag.owner;
                    } else if tag.owner.is_some() && tag.owner != owner_from_tags {
                        self.enter_safe_mode("bootstrap_conflicting_owner_tags");
                    }
                    if tag.role == Some(TagRole::Sl) {
                        self.sl_stop_order_id = Some(stop_order_id.clone());
                        self.sl_exchange_order_id = stop_order.exchange_order_id;
                    }
                }
            }
        }
        if self.current_owner.is_none() {
            self.current_owner = owner_from_tags;
        }
        if let Some(position) = snapshot.positions_strategy.get(&self.config.symbol) {
            self.last_position_qty = position.qty;
            if self.current_side.is_none() && position.qty.abs() > f64::EPSILON {
                self.current_side = Some(if position.qty >= 0.0 {
                    Side::Long
                } else {
                    Side::Short
                });
            }
            if position.qty.abs() > f64::EPSILON
                && self.pending_entry.is_none()
                && self.current_owner.is_none()
            {
                self.enter_safe_mode("bootstrap_position_owner_unknown");
            }
        }
        self.sync_state();
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        ctx: &StrategyCtx,
        state: &crate::RuntimeStateRestored,
    ) -> Vec<Intent> {
        self.clear_boot_stale_pending_tail(state);
        self.arm_startup_replay_guard(ctx);
        self.sync_state();
        Vec::new()
    }

    fn risk_gate_session_finalizations(&self) -> Vec<RiskGateSessionFinalization> {
        self.pending_risk_gate_finalizations.clone()
    }

    fn acknowledge_risk_gate_session_finalizations(&mut self, session_dates: &[NaiveDate]) {
        if session_dates.is_empty() {
            return;
        }
        self.pending_risk_gate_finalizations
            .retain(|finalization| !session_dates.contains(&finalization.session_date));
        self.sync_state();
    }

    fn on_risk_gate_state(&mut self, state: &RiskGateRuntimeState) {
        if !self.risk_gate_shadow_enabled() {
            return;
        }
        self.risk_gate_mr_enabled_current_session = state.mr_enabled_current_session;
        self.risk_gate_rolling_sum_lb120 = state.rolling_sum_lb120;
        self.risk_gate_last_finalized_session_date = state.last_finalized_session_date;
        self.risk_gate_ledger_rows_count = state.ledger_rows_count;
        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            action = "risk_gate_state_applied",
            profile_id = %state.profile_id,
            mr_enabled_current_session = state.mr_enabled_current_session,
            mr_enabled_next_session = state.mr_enabled_next_session,
            rolling_sum_lb120 = state.rolling_sum_lb120,
            last_finalized_session_date = state
                .last_finalized_session_date
                .map(Self::format_local_day),
            ledger_rows_count = state.ledger_rows_count,
        );
        self.sync_state();
    }

    fn warmup_from_history(&mut self, _ctx: &StrategyCtx, bars: &[BarEvent]) -> usize {
        let mut warmup = HybridIntradayRuntimeStrategy::new(self.config.clone());
        let mut processed = 0usize;

        for bar in bars {
            if bar.symbol != self.config.symbol {
                continue;
            }
            let Some(dt_local) = warmup.utc_to_local_naive(bar.close_time_utc) else {
                continue;
            };
            if warmup.suppress_weekend_signal_generation(dt_local) {
                warmup.last_processed_bar_ts = Some(bar.close_time_utc);
                continue;
            }
            if warmup.suppress_non_model_session_bar(dt_local).is_some() {
                warmup.last_processed_bar_ts = Some(bar.close_time_utc);
                continue;
            }
            warmup.update_day_aggregates(dt_local, bar.h, bar.l);
            if warmup.uses_high180_mr() {
                warmup.high180_mr.on_bar(dt_local, bar.h, bar.l);
                warmup.risk_gate_shadow_mr.on_bar(dt_local, bar.h, bar.l);
            }
            warmup
                .orchestrator
                .intraday_breakout
                .on_bar(dt_local, bar.o, bar.h, bar.l, bar.close);
            warmup.last_bar_close = Some(bar.close);
            warmup.last_processed_bar_ts = Some(bar.close_time_utc);
            processed += 1;
        }

        if processed == 0 {
            return 0;
        }

        let breakout = warmup.breakout_snapshot();
        self.last_bar_close = warmup.last_bar_close;
        self.prev_day_close = warmup.prev_day_close;
        self.last_day_local = warmup.last_day_local;
        self.current_day_high = warmup.current_day_high;
        self.current_day_low = warmup.current_day_low;
        self.prev_day_range = warmup.prev_day_range;
        self.high180_mr = warmup.high180_mr;
        self.risk_gate_shadow_mr = warmup.risk_gate_shadow_mr;
        self.orchestrator
            .intraday_breakout
            .restore_snapshot(breakout.clone());
        self.entry_ready = self.signals_warmed();
        self.last_warmup_log = None;
        self.sync_state();

        info!(
            target: "strategy_runtime::hybrid_intraday_runtime",
            strategy = "hybrid_intraday_runtime",
            symbol = self.config.symbol,
            processed,
            prev_day_close = self.prev_day_close,
            prev_day_range = self.prev_day_range,
            prev_day_return = breakout.yesterday_return,
            current_day_high = self.current_day_high,
            current_day_low = self.current_day_low,
            current_day_close = breakout.cur_day_close,
            today_start_local = breakout.today_start.map(Self::format_local_dt),
            was_long_today = breakout.was_long_today,
            was_short_today = breakout.was_short_today,
            last_bar_close = self.last_bar_close,
            "hybrid history warmup applied"
        );

        processed
    }

    fn intent_comment_tag(
        &self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        intent_class: IntentClass,
    ) -> Option<String> {
        let cycle = format!("{:08x}", created_ts_utc.max(0));
        let role = match intent_class {
            IntentClass::Entry => "ENTRY",
            IntentClass::Exit => "EXIT",
            IntentClass::CancelCleanup => "CANCEL",
            IntentClass::ProtectiveRepair => "REPAIR",
        };
        let comment = format!("HYB|sid={}|c={cycle}|r={role}", ctx.strategy_id);
        Some(comment.chars().filter(|c| c.is_ascii()).take(100).collect())
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        let mut needs_resync = false;
        if let StrategyState::HybridIntradayRuntime {
            active_cycle_id,
            next_cycle_seq,
            last_position_qty,
            current_owner,
            current_side,
            pending_entry_owner,
            pending_entry_side,
            pending_entry_cycle_id,
            pending_entry_request_id,
            pending_entry_created_ts_utc,
            deferred_entry_owner,
            deferred_entry_side,
            deferred_entry_cycle_id,
            deferred_entry_entry_style,
            deferred_entry_reason,
            deferred_entry_stop_price,
            deferred_entry_take_price,
            deferred_entry_ts_utc,
            deferred_entry_request_id,
            pending_exit_request_id,
            pending_exit_created_ts_utc,
            deferred_exit_owner,
            deferred_exit_reason,
            deferred_exit_cycle_id,
            deferred_exit_ts_utc,
            deferred_exit_request_id,
            pending_tp_request_id,
            pending_tp_created_ts_utc,
            pending_sl_request_id,
            pending_sl_created_ts_utc,
            tp_order_id,
            sl_stop_order_id,
            sl_exchange_order_id,
            sl_triggered_ts,
            mr_take_price,
            mr_stop_price,
            repair_deadline_ts,
            next_repair_at_ts,
            repair_backoff_level,
            repair_attempts,
            safe_mode_close_only,
            safe_mode_reason,
            entry_ready,
            last_bar_close,
            prev_day_close,
            last_day_local,
            current_day_high,
            current_day_low,
            prev_day_range,
            current_day_close,
            prev_day_return,
            day_before_close,
            today_start_local,
            was_long_today,
            was_short_today,
            overnight_exit_armed_date,
            risk_gate_shadow_session_date,
            risk_gate_shadow_pnl_points,
            risk_gate_shadow_trade_count,
            risk_gate_shadow_entry_ts_utc,
            risk_gate_shadow_entry_price,
            risk_gate_shadow_side,
            risk_gate_shadow_target_price,
            risk_gate_shadow_stop_price,
            risk_gate_pending_session_date,
            risk_gate_pending_shadow_pnl_points,
            risk_gate_pending_shadow_trade_count,
            risk_gate_mr_enabled_current_session,
            risk_gate_rolling_sum_lb120,
            risk_gate_last_finalized_session_date,
            risk_gate_ledger_rows_count,
        } = &state
        {
            self.active_cycle_id = active_cycle_id.as_deref().and_then(Self::parse_cycle_id);
            self.next_cycle_seq = *next_cycle_seq;
            self.last_position_qty = *last_position_qty;
            self.current_owner = *current_owner;
            self.current_side = *current_side;
            self.pending_entry = match (
                *pending_entry_owner,
                *pending_entry_side,
                pending_entry_cycle_id
                    .as_deref()
                    .and_then(Self::parse_cycle_id),
            ) {
                (Some(owner), Some(side), Some(cycle_id)) => Some(PendingEntry {
                    owner,
                    side,
                    cycle_id,
                    reason:
                        crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
                    entry_style: EntryStyle::Market,
                    stop_price: None,
                    take_price: None,
                    target_qty: self.config.qty.max(1.0),
                    partial_started_at_ms: None,
                }),
                _ => None,
            };
            self.pending_entry_request_id = *pending_entry_request_id;
            self.pending_entry_created_ts_utc = *pending_entry_created_ts_utc;
            self.deferred_entry = match (
                *deferred_entry_owner,
                *deferred_entry_side,
                deferred_entry_cycle_id
                    .as_deref()
                    .and_then(Self::parse_cycle_id),
                *deferred_entry_entry_style,
                *deferred_entry_reason,
                *deferred_entry_ts_utc,
                *deferred_entry_request_id,
            ) {
                (
                    Some(owner),
                    Some(side),
                    Some(cycle_id),
                    Some(entry_style),
                    Some(reason),
                    Some(deferred_ts_utc),
                    Some(original_request_id),
                ) => Some(DeferredEntry {
                    signal: crate::strategies::hybrid_intraday::EntrySignal {
                        owner,
                        side,
                        entry_style,
                        reason,
                        stop_price: *deferred_entry_stop_price,
                        take_price: *deferred_entry_take_price,
                    },
                    cycle_id,
                    deferred_ts_utc,
                    original_request_id,
                }),
                _ => None,
            };
            self.pending_exit = None;
            self.pending_exit_request_id = *pending_exit_request_id;
            self.pending_exit_created_ts_utc = *pending_exit_created_ts_utc;
            self.deferred_exit = match (
                *deferred_exit_owner,
                *deferred_exit_reason,
                deferred_exit_cycle_id
                    .as_deref()
                    .and_then(Self::parse_cycle_id),
                *deferred_exit_ts_utc,
                *deferred_exit_request_id,
            ) {
                (
                    Some(owner),
                    Some(reason),
                    Some(cycle_id),
                    Some(deferred_ts_utc),
                    Some(original_request_id),
                ) => Some(DeferredExit {
                    owner,
                    reason,
                    cycle_id,
                    deferred_ts_utc,
                    original_request_id,
                }),
                _ => None,
            };
            self.pending_tp_request_id = *pending_tp_request_id;
            self.pending_tp_created_ts_utc = *pending_tp_created_ts_utc;
            self.pending_sl_request_id = *pending_sl_request_id;
            self.pending_sl_created_ts_utc = *pending_sl_created_ts_utc;
            self.tp_order_id = *tp_order_id;
            self.sl_stop_order_id = sl_stop_order_id.clone();
            self.sl_exchange_order_id = *sl_exchange_order_id;
            self.sl_triggered_ts = *sl_triggered_ts;
            self.mr_take_price = *mr_take_price;
            self.mr_stop_price = *mr_stop_price;
            self.repair_deadline_ts = *repair_deadline_ts;
            self.next_repair_at_ts = *next_repair_at_ts;
            self.repair_backoff_level = *repair_backoff_level;
            self.repair_attempts = *repair_attempts;
            self.safe_mode_close_only = *safe_mode_close_only;
            self.safe_mode_reason = safe_mode_reason.clone();
            self.last_bar_close = *last_bar_close;
            self.prev_day_close = *prev_day_close;
            self.last_day_local = last_day_local.as_deref().and_then(Self::parse_local_day);
            self.current_day_high = *current_day_high;
            self.current_day_low = *current_day_low;
            self.prev_day_range = *prev_day_range;
            self.orchestrator.restore(
                crate::strategies::hybrid_intraday::orchestrator::HybridSnapshot {
                    state: if self.pending_entry.is_some()
                        || self.pending_entry_request_id.is_some()
                    {
                        crate::strategies::hybrid_intraday::HybridState::Pending
                    } else if self.last_position_qty.abs() > f64::EPSILON
                        && self.current_owner.is_some()
                        && self.current_side.is_some()
                    {
                        crate::strategies::hybrid_intraday::HybridState::Open
                    } else {
                        crate::strategies::hybrid_intraday::HybridState::Flat
                    },
                    current_owner: self.current_owner,
                    current_side: self.current_side,
                    has_pending_entry: self.pending_entry.is_some(),
                    overnight_exit_armed_date: overnight_exit_armed_date
                        .as_deref()
                        .and_then(Self::parse_local_day),
                },
            );
            self.orchestrator.intraday_breakout.restore_snapshot(
                crate::strategies::hybrid_intraday::intraday_breakout::IntradayBreakoutSnapshot {
                    cur_day_date: self.last_day_local,
                    cur_day_high: self.current_day_high,
                    cur_day_low: self.current_day_low,
                    cur_day_close: *current_day_close,
                    yesterday_close: *prev_day_close,
                    yesterday_range: *prev_day_range,
                    yesterday_return: *prev_day_return,
                    day_before_close: *day_before_close,
                    was_long_today: *was_long_today,
                    was_short_today: *was_short_today,
                    today_start: today_start_local.as_deref().and_then(Self::parse_local_dt),
                },
            );
            self.entry_ready = *entry_ready && self.signals_warmed();
            self.last_warmup_log = Some(self.entry_ready);
            self.risk_gate_shadow_session_date = risk_gate_shadow_session_date
                .as_deref()
                .and_then(Self::parse_local_day);
            self.risk_gate_shadow_pnl_points = *risk_gate_shadow_pnl_points;
            self.risk_gate_shadow_trade_count = *risk_gate_shadow_trade_count;
            self.risk_gate_shadow_position = match (
                *risk_gate_shadow_entry_ts_utc,
                *risk_gate_shadow_entry_price,
                *risk_gate_shadow_side,
            ) {
                (Some(entry_ts_utc), Some(entry_price), Some(side)) => {
                    Some(ShadowHigh180Position {
                        side,
                        entry_ts_utc,
                        entry_price,
                    })
                }
                _ => None,
            };
            self.risk_gate_shadow_open =
                match (*risk_gate_shadow_target_price, *risk_gate_shadow_stop_price) {
                    (Some(target_price), Some(stop_price)) => Some(High180Open {
                        target_price,
                        stop_price,
                        max_hold: High180MrConfig::default().max_hold,
                    }),
                    _ => None,
                };
            self.pending_risk_gate_finalizations.clear();
            if let Some(session_date) = risk_gate_pending_session_date
                .as_deref()
                .and_then(Self::parse_local_day)
            {
                self.pending_risk_gate_finalizations
                    .push(RiskGateSessionFinalization {
                        session_date,
                        shadow_pnl_points: *risk_gate_pending_shadow_pnl_points,
                        shadow_trade_count: *risk_gate_pending_shadow_trade_count,
                    });
            }
            self.risk_gate_mr_enabled_current_session = *risk_gate_mr_enabled_current_session;
            self.risk_gate_rolling_sum_lb120 = *risk_gate_rolling_sum_lb120;
            self.risk_gate_last_finalized_session_date = risk_gate_last_finalized_session_date
                .as_deref()
                .and_then(Self::parse_local_day);
            self.risk_gate_ledger_rows_count = *risk_gate_ledger_rows_count;
            if *entry_ready && !self.signals_warmed() {
                needs_resync = true;
            }
        }
        self.state = state;
        if needs_resync {
            self.sync_state();
        }
    }
}
