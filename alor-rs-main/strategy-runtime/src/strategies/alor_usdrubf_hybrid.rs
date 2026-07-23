use std::collections::BTreeSet;

use alor_protocol::{AckStatus, CommandAck, IntentClass, Side, StopLimitCondition};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::state::StrategyState;
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
    RuntimeStateRestored, StopOrderEvent, Strategy, StrategyCtx,
};

#[derive(Debug, Clone)]
pub struct AlorUsdrubfHybridConfig {
    pub symbol: String,
    pub timezone_offset_hours: i32,
    pub tick_size: f64,
    pub model_session_start_time: NaiveTime,
    pub model_session_end_time: NaiveTime,
    pub mr_min_rel_range: f64,
    pub mr_max_rel_range: f64,
    pub mr_k_short: f64,
    pub mr_take_k_short: f64,
    pub mr_stop_k_short: f64,
    pub mr_last_entry_time: NaiveTime,
    pub mr_force_exit_time: NaiveTime,
    pub bo_k: f64,
    pub bo_stop1_range: f64,
    pub bo_stop2_range: f64,
    pub bo_big_move_threshold: f64,
    pub bo_wait_hours: f64,
    pub bo_eod_exit_time: NaiveTime,
    pub commission_pct_per_side: f64,
    pub position_size_fraction: f64,
    pub initial_cash: f64,
    pub enable_live_execution: bool,
    pub use_fixed_live_size: bool,
    pub live_fixed_units: f64,
    pub max_silence_bars_sec: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HybridState {
    Flat,
    Pending,
    Open,
}

impl HybridState {
    fn as_str(self) -> &'static str {
        match self {
            HybridState::Flat => "flat",
            HybridState::Pending => "pending",
            HybridState::Open => "open",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    MeanRev,
    Breakout,
}

impl Owner {
    fn as_str(self) -> &'static str {
        match self {
            Owner::MeanRev => "mean_rev",
            Owner::Breakout => "day_breakout_waitfix",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionSide {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtectionRole {
    Tp,
    Sl,
}

impl ProtectionRole {
    fn as_str(self) -> &'static str {
        match self {
            ProtectionRole::Tp => "TP",
            ProtectionRole::Sl => "SL",
        }
    }
}

impl PositionSide {
    fn as_str(self) -> &'static str {
        match self {
            PositionSide::Long => "long",
            PositionSide::Short => "short",
        }
    }

    fn entry_side(self) -> Side {
        match self {
            PositionSide::Long => Side::Buy,
            PositionSide::Short => Side::Sell,
        }
    }

    fn exit_side(self) -> Side {
        match self {
            PositionSide::Long => Side::Sell,
            PositionSide::Short => Side::Buy,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct PendingEntry {
    owner: Owner,
    side: PositionSide,
    reason: String,
    scale_at_signal: f64,
    signal_price: f64,
    stop1: Option<f64>,
    stop2: Option<f64>,
    target_qty: f64,
    partial_started_at_ms: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct OpenPosition {
    owner: Owner,
    side: PositionSide,
    entry_ts: NaiveDateTime,
    entry_price: f64,
    size: i64,
    stop_price: Option<f64>,
    take_price: Option<f64>,
    stop1: Option<f64>,
    stop2: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
struct ResearchSnapshot {
    close: f64,
    high: f64,
    low: f64,
    local_dt: NaiveDateTime,
    session_open: Option<f64>,
    session_vwap: f64,
    session_range: Option<f64>,
    elapsed_hours: Option<f64>,
    ret_from_open: Option<f64>,
    bo_was_long_today: bool,
    bo_was_short_today: bool,
}

#[derive(Debug)]
pub struct AlorUsdrubfHybridStrategy {
    config: AlorUsdrubfHybridConfig,
    state: StrategyState,
    lifecycle_stage: String,
    last_bar_ts: Option<i64>,
    last_processed_bar_ts: Option<i64>,
    bootstrap_seen: bool,
    runtime_state_restored: bool,
    live_ready: bool,
    hybrid_state: HybridState,
    current_date_local: Option<NaiveDate>,
    day_open: Option<f64>,
    day_high: Option<f64>,
    day_low: Option<f64>,
    day_volume_sum: f64,
    day_vwap_num: f64,
    session_start_local: Option<NaiveDateTime>,
    pending_entry: Option<PendingEntry>,
    pending_request_ids: BTreeSet<Uuid>,
    tracked_order_ids: BTreeSet<i64>,
    entry_intent_inflight: bool,
    entry_reject_deferred_until_bar_ts: Option<i64>,
    open_position: Option<OpenPosition>,
    exit_intent_inflight: bool,
    exit_reject_deferred_until_bar_ts: Option<i64>,
    owner_confirmed_by_live_event: bool,
    pending_tp_bar_ts_utc: Option<i64>,
    pending_sl_bar_ts_utc: Option<i64>,
    tp_order_id: Option<i64>,
    sl_stop_order_id: Option<String>,
    sl_exchange_order_id: Option<i64>,
    bracket_terminal_reconcile_started_ms: Option<i64>,
    cash: f64,
    bo_was_long_today: bool,
    bo_was_short_today: bool,
    /// Last broker qty/avg for position_transition / duplicate suppression (not persisted).
    last_logged_broker_qty: f64,
    last_logged_broker_avg: f64,
    last_logged_broker_initialized: bool,
    startup_replay_tail_info_emitted: bool,
    awaiting_live_bar_info_emitted: bool,
    recovered_bar_suppressed_info_emitted: bool,
    last_logged_entry_inflight: bool,
    last_logged_exit_inflight: bool,
    last_logged_entry_reject_defer_ts: Option<i64>,
    last_logged_exit_reject_defer_ts: Option<i64>,
}

impl AlorUsdrubfHybridStrategy {
    const BRACKET_TERMINAL_RECONCILE_GRACE_MS: i64 = 3_000;

    pub fn new(config: AlorUsdrubfHybridConfig) -> Self {
        let mut strategy = Self {
            config,
            state: StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage: "created".to_string(),
                last_bar_ts: None,
                last_processed_bar_ts: None,
                bootstrap_seen: false,
                runtime_state_restored: false,
                live_ready: false,
                hybrid_state: "flat".to_string(),
                current_date_local: None,
                day_open: None,
                day_high: None,
                day_low: None,
                day_volume_sum: 0.0,
                day_vwap_num: 0.0,
                session_start_local: None,
                bo_was_long_today: false,
                bo_was_short_today: false,
                cash: 0.0,
                pending_entry_owner: None,
                pending_entry_side: None,
                pending_request_ids: Vec::new(),
                tracked_order_ids: Vec::new(),
                entry_intent_inflight: false,
                pending_entry_reason: None,
                pending_entry_scale_at_signal: None,
                pending_entry_signal_price: None,
                pending_entry_stop1: None,
                pending_entry_stop2: None,
                open_position_owner: None,
                open_position_side: None,
                exit_intent_inflight: false,
                open_position_qty: 0.0,
                open_position_entry_ts: None,
                open_position_entry_price: None,
                open_position_stop_price: None,
                open_position_take_price: None,
                open_position_stop1: None,
                open_position_stop2: None,
            },
            lifecycle_stage: "created".to_string(),
            last_bar_ts: None,
            last_processed_bar_ts: None,
            bootstrap_seen: false,
            runtime_state_restored: false,
            live_ready: false,
            hybrid_state: HybridState::Flat,
            current_date_local: None,
            day_open: None,
            day_high: None,
            day_low: None,
            day_volume_sum: 0.0,
            day_vwap_num: 0.0,
            session_start_local: None,
            pending_entry: None,
            pending_request_ids: BTreeSet::new(),
            tracked_order_ids: BTreeSet::new(),
            entry_intent_inflight: false,
            entry_reject_deferred_until_bar_ts: None,
            open_position: None,
            exit_intent_inflight: false,
            exit_reject_deferred_until_bar_ts: None,
            owner_confirmed_by_live_event: true,
            pending_tp_bar_ts_utc: None,
            pending_sl_bar_ts_utc: None,
            tp_order_id: None,
            sl_stop_order_id: None,
            sl_exchange_order_id: None,
            bracket_terminal_reconcile_started_ms: None,
            cash: 0.0,
            bo_was_long_today: false,
            bo_was_short_today: false,
            last_logged_broker_qty: 0.0,
            last_logged_broker_avg: 0.0,
            last_logged_broker_initialized: false,
            startup_replay_tail_info_emitted: false,
            awaiting_live_bar_info_emitted: false,
            recovered_bar_suppressed_info_emitted: false,
            last_logged_entry_inflight: false,
            last_logged_exit_inflight: false,
            last_logged_entry_reject_defer_ts: None,
            last_logged_exit_reject_defer_ts: None,
        };
        strategy.cash = strategy.config.initial_cash;
        strategy.sync_state();
        strategy
    }

    fn sync_state(&mut self) {
        self.state = StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: self.lifecycle_stage.clone(),
            last_bar_ts: self.last_bar_ts,
            last_processed_bar_ts: self.last_processed_bar_ts,
            bootstrap_seen: self.bootstrap_seen,
            runtime_state_restored: self.runtime_state_restored,
            live_ready: self.live_ready,
            hybrid_state: self.hybrid_state.as_str().to_string(),
            current_date_local: self.current_date_local.map(|d| d.to_string()),
            day_open: self.day_open,
            day_high: self.day_high,
            day_low: self.day_low,
            day_volume_sum: self.day_volume_sum,
            day_vwap_num: self.day_vwap_num,
            session_start_local: self.session_start_local.map(|dt| dt.to_string()),
            bo_was_long_today: self.bo_was_long_today,
            bo_was_short_today: self.bo_was_short_today,
            cash: self.cash,
            pending_entry_owner: self
                .pending_entry
                .as_ref()
                .map(|entry| entry.owner.as_str().to_string()),
            pending_entry_side: self
                .pending_entry
                .as_ref()
                .map(|entry| entry.side.as_str().to_string()),
            pending_request_ids: self.pending_request_ids.iter().copied().collect(),
            tracked_order_ids: self.tracked_order_ids.iter().copied().collect(),
            entry_intent_inflight: self.entry_intent_inflight,
            pending_entry_reason: self
                .pending_entry
                .as_ref()
                .map(|entry| entry.reason.clone()),
            pending_entry_scale_at_signal: self
                .pending_entry
                .as_ref()
                .map(|entry| entry.scale_at_signal),
            pending_entry_signal_price: self.pending_entry.as_ref().map(|entry| entry.signal_price),
            pending_entry_stop1: self.pending_entry.as_ref().and_then(|entry| entry.stop1),
            pending_entry_stop2: self.pending_entry.as_ref().and_then(|entry| entry.stop2),
            open_position_owner: self
                .open_position
                .as_ref()
                .map(|position| position.owner.as_str().to_string()),
            open_position_side: self
                .open_position
                .as_ref()
                .map(|position| position.side.as_str().to_string()),
            exit_intent_inflight: self.exit_intent_inflight,
            open_position_qty: self
                .open_position
                .as_ref()
                .map(|position| position.size as f64)
                .unwrap_or(0.0),
            open_position_entry_ts: self
                .open_position
                .as_ref()
                .map(|position| position.entry_ts.to_string()),
            open_position_entry_price: self
                .open_position
                .as_ref()
                .map(|position| position.entry_price),
            open_position_stop_price: self
                .open_position
                .as_ref()
                .and_then(|position| position.stop_price),
            open_position_take_price: self
                .open_position
                .as_ref()
                .and_then(|position| position.take_price),
            open_position_stop1: self
                .open_position
                .as_ref()
                .and_then(|position| position.stop1),
            open_position_stop2: self
                .open_position
                .as_ref()
                .and_then(|position| position.stop2),
        };
    }

    fn is_live_startup_bar_stale(&self, ctx: &StrategyCtx, bar_ts_utc: i64) -> bool {
        let age_sec = ctx.now_ts_utc().saturating_sub(bar_ts_utc);
        age_sec > self.config.max_silence_bars_sec as i64
    }

    fn is_recovered_or_non_live_bar_origin(origin: &DataOrigin) -> bool {
        matches!(
            origin,
            DataOrigin::History | DataOrigin::HistoryGap | DataOrigin::Replay
        )
    }

    fn snapshot_position_side(qty: f64) -> PositionSide {
        if qty >= 0.0 {
            PositionSide::Long
        } else {
            PositionSide::Short
        }
    }

    fn snapshot_owner_fallback(&self) -> Owner {
        self.open_position
            .as_ref()
            .map(|position| position.owner)
            .or_else(|| self.pending_entry.as_ref().map(|pending| pending.owner))
            .unwrap_or(Owner::Breakout)
    }

    fn infer_owner_from_existing_state(&self) -> Option<Owner> {
        self.open_position
            .as_ref()
            .map(|position| position.owner)
            .or_else(|| self.pending_entry.as_ref().map(|pending| pending.owner))
    }

    fn update_session_metrics(&mut self, bar: &BarEvent, local_dt: NaiveDateTime) {
        self.day_open.get_or_insert(bar.o);
        self.day_high = Some(self.day_high.unwrap_or(bar.h).max(bar.h));
        self.day_low = Some(self.day_low.unwrap_or(bar.l).min(bar.l));
        let volume = bar.v.max(0.0);
        let typical_price = (bar.h + bar.l + bar.close) / 3.0;
        self.day_volume_sum += volume;
        self.day_vwap_num += typical_price * volume;
        self.session_start_local.get_or_insert(local_dt);
    }

    fn is_model_session_bar(&self, local_dt: NaiveDateTime) -> bool {
        let time = local_dt.time();
        time >= self.config.model_session_start_time && time <= self.config.model_session_end_time
    }

    fn reset_day(&mut self, local_date: NaiveDate) {
        self.reset_day_aggregates(local_date);
        // Strategy has no cross-session carry by design.
        self.pending_entry = None;
        self.open_position = None;
        self.pending_request_ids.clear();
        self.tracked_order_ids.clear();
        self.entry_intent_inflight = false;
        self.exit_intent_inflight = false;
        self.hybrid_state = HybridState::Flat;
        self.last_logged_broker_qty = 0.0;
        self.last_logged_broker_avg = 0.0;
        self.last_logged_broker_initialized = false;
    }

    fn reset_day_aggregates(&mut self, local_date: NaiveDate) {
        self.current_date_local = Some(local_date);
        self.day_open = None;
        self.day_high = None;
        self.day_low = None;
        self.day_volume_sum = 0.0;
        self.day_vwap_num = 0.0;
        self.session_start_local = None;
        self.bo_was_long_today = false;
        self.bo_was_short_today = false;
    }

    fn session_vwap(&self, fallback_close: f64) -> f64 {
        if self.day_volume_sum > 0.0 {
            self.day_vwap_num / self.day_volume_sum
        } else {
            fallback_close
        }
    }

    fn session_range(&self) -> Option<f64> {
        Some(self.day_high? - self.day_low?)
    }

    fn elapsed_hours(&self, local_dt: NaiveDateTime) -> Option<f64> {
        let start = self.session_start_local?;
        let mins = (local_dt - start).num_minutes().max(0) as f64;
        Some(mins / 60.0)
    }

    fn ret_from_open(&self, close: f64) -> Option<f64> {
        let open = self.day_open?;
        if open.abs() <= f64::EPSILON {
            None
        } else {
            Some((close - open) / open)
        }
    }

    fn round_to_tick(&self, price: f64, tick_size: f64) -> f64 {
        if tick_size <= 0.0 {
            price
        } else {
            ((price / tick_size) + 0.5).floor() * tick_size
        }
    }

    fn compute_entry_size(&self, bar_open: f64) -> i64 {
        if self.config.use_fixed_live_size {
            self.config.live_fixed_units.max(1.0).floor() as i64
        } else {
            ((self.cash * self.config.position_size_fraction) / bar_open)
                .floor()
                .max(1.0) as i64
        }
    }

    fn configured_live_target_qty(&self) -> f64 {
        if self.config.use_fixed_live_size {
            self.config.live_fixed_units.max(1.0).floor()
        } else {
            1.0
        }
    }

    fn build_open_position_from_pending(
        &self,
        pending: &PendingEntry,
        entry_ts_utc: i64,
        entry_price: f64,
        size: i64,
    ) -> OpenPosition {
        let (stop_price, take_price) = if pending.owner == Owner::MeanRev {
            (
                Some(self.round_to_tick(
                    entry_price + self.config.mr_stop_k_short * pending.scale_at_signal,
                    self.config.tick_size,
                )),
                Some(self.round_to_tick(
                    entry_price - self.config.mr_take_k_short * pending.scale_at_signal,
                    self.config.tick_size,
                )),
            )
        } else {
            (None, None)
        };

        OpenPosition {
            owner: pending.owner,
            side: pending.side,
            entry_ts: utc_to_local(entry_ts_utc, self.config.timezone_offset_hours),
            entry_price,
            size,
            stop_price,
            take_price,
            stop1: pending.stop1,
            stop2: pending.stop2,
        }
    }

    fn maybe_fill_pending_entry(&mut self, bar: &BarEvent, intents: &mut Vec<Intent>) {
        let Some(pending) = self.pending_entry.clone() else {
            return;
        };
        let size = self.compute_entry_size(bar.o);

        intents.push(Intent::Market {
            qty: size as f64,
            side: pending.side.entry_side(),
            fill_price: Some(bar.o),
            comment: Some(format!(
                "{}|entry|{}",
                self.config.symbol,
                pending.owner.as_str()
            )),
        });
        self.open_position =
            Some(self.build_open_position_from_pending(&pending, bar.close_time_utc, bar.o, size));
        self.pending_entry = None;
        self.entry_intent_inflight = false;
        self.exit_intent_inflight = false;
        self.clear_mr_protection_tracking();
        self.hybrid_state = HybridState::Open;
    }

    fn mr_protection_comment(&self, ctx: &StrategyCtx, role: ProtectionRole) -> String {
        let comment = format!("AUS|sid={}|o=MR|r={}", ctx.strategy_id, role.as_str());
        comment.chars().filter(|c| c.is_ascii()).take(100).collect()
    }

    fn parse_mr_protection_role(comment: Option<&str>) -> Option<ProtectionRole> {
        let comment = comment?;
        if comment.contains("|o=MR|r=TP") {
            return Some(ProtectionRole::Tp);
        }
        if comment.contains("|o=MR|r=SL") {
            return Some(ProtectionRole::Sl);
        }
        None
    }

    fn stop_condition_for_position_side(side: PositionSide) -> StopLimitCondition {
        match side {
            PositionSide::Long => StopLimitCondition::LessOrEqual,
            PositionSide::Short => StopLimitCondition::MoreOrEqual,
        }
    }

    fn stop_limit_price(side: Side, trigger_price: f64, tick_size: f64) -> f64 {
        let offset = tick_size.max(0.000_000_1);
        match side {
            Side::Buy => trigger_price + offset,
            Side::Sell => trigger_price - offset,
        }
    }

    fn stop_end_unix_time(&self, created_ts_utc: i64) -> i64 {
        created_ts_utc.saturating_add(12 * 60 * 60)
    }

    fn clear_mr_protection_tracking(&mut self) {
        self.pending_tp_bar_ts_utc = None;
        self.pending_sl_bar_ts_utc = None;
        self.tp_order_id = None;
        self.sl_stop_order_id = None;
        self.sl_exchange_order_id = None;
    }

    fn emit_cancel_all_protection(&mut self, side: Option<Side>) -> Vec<Intent> {
        let mut intents = Vec::new();
        self.pending_tp_bar_ts_utc = None;
        self.pending_sl_bar_ts_utc = None;
        if let Some(tp_order_id) = self.tp_order_id.take() {
            intents.push(
                Intent::Cancel {
                    order_id: tp_order_id,
                }
                .with_class(IntentClass::CancelCleanup),
            );
        }
        if let Some(stop_order_id) = self.sl_stop_order_id.take() {
            intents.push(
                Intent::DeleteStopLimit {
                    order_id: stop_order_id,
                    side,
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

    fn maybe_emit_live_mr_brackets(
        &mut self,
        ctx: &StrategyCtx,
        bar_ts_utc: i64,
        intents: &mut Vec<Intent>,
    ) {
        // A filled TP/SL is terminal for the active bracket, but broker position
        // truth can arrive slightly later. Do not repair protection in that gap.
        if self.exit_intent_inflight {
            return;
        }
        let Some(pos) = self.open_position.as_ref() else {
            return;
        };
        if pos.owner != Owner::MeanRev {
            return;
        }
        let exit_side = pos.side.exit_side();
        if let Some(take_price) = pos.take_price {
            // An unknown create-limit outcome must remain pending until broker truth resolves it.
            // Retrying on the next bar can create a second TP and over-close the position.
            let tp_ready = self.tp_order_id.is_none() && self.pending_tp_bar_ts_utc.is_none();
            if tp_ready {
                intents.push(
                    Intent::Place {
                        price: take_price,
                        qty: pos.size as f64,
                        side: exit_side,
                        comment: Some(self.mr_protection_comment(ctx, ProtectionRole::Tp)),
                    }
                    .with_class(IntentClass::ProtectiveRepair),
                );
                self.pending_tp_bar_ts_utc = Some(bar_ts_utc);
            }
        }
        if let Some(stop_price) = pos.stop_price {
            let sl_ready = self.sl_stop_order_id.is_none() && self.pending_sl_bar_ts_utc.is_none();
            if sl_ready {
                intents.push(
                    Intent::CreateStopLimit {
                        side: exit_side,
                        qty: pos.size as f64,
                        trigger_price: stop_price,
                        price: Self::stop_limit_price(exit_side, stop_price, ctx.tick_size),
                        condition: Self::stop_condition_for_position_side(pos.side),
                        stop_end_unix_time: self.stop_end_unix_time(bar_ts_utc),
                        comment: Some(self.mr_protection_comment(ctx, ProtectionRole::Sl)),
                        instrument_group: None,
                        check_duplicates: Some(true),
                    }
                    .with_class(IntentClass::ProtectiveRepair),
                );
                self.pending_sl_bar_ts_utc = Some(bar_ts_utc);
            }
        }
    }

    fn emit_broker_residual_emergency_exit(
        &mut self,
        ctx: &StrategyCtx,
        pos: &PositionEvent,
        reason: &str,
    ) -> Vec<Intent> {
        if self.exit_intent_inflight || pos.qty.abs() < 1e-9 {
            return Vec::new();
        }
        let side = if pos.qty > 0.0 { Side::Sell } else { Side::Buy };
        let mut intents = self.emit_cancel_all_protection(Some(side));
        intents.push(
            Intent::Market {
                qty: pos.qty.abs(),
                side,
                fill_price: None,
                comment: Some(format!("{}|exit|{}", self.config.symbol, reason)),
            }
            .with_class(IntentClass::Exit),
        );
        self.exit_intent_inflight = true;
        self.lifecycle_stage = reason.to_string();
        warn!(
            strategy_id = ctx.strategy_id.as_str(),
            strategy = "alor_usdrubf_hybrid",
            action = "broker_residual_emergency_exit",
            reason,
            broker_qty = pos.qty,
            exit_side = ?side,
            exit_qty = pos.qty.abs(),
            "broker position requires residual flatten; cancel protection and flatten residual"
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

    fn mr_bracket_protective_partial_progress(open: &OpenPosition, broker_qty: f64) -> bool {
        if open.owner != Owner::MeanRev || broker_qty.abs() < 1e-9 {
            return false;
        }
        let previous_qty = match open.side {
            PositionSide::Long => open.size as f64,
            PositionSide::Short => -(open.size as f64),
        };
        if previous_qty.abs() <= 1e-9 {
            return false;
        }
        previous_qty.signum() == broker_qty.signum()
            && broker_qty.abs() + f64::EPSILON < previous_qty.abs()
    }

    fn mark_mr_bracket_partial_reconcile(&mut self, broker_qty: f64) {
        self.mark_bracket_terminal_reconcile();
        self.exit_intent_inflight = true;
        self.lifecycle_stage = "mr_bracket_partial_awaiting_broker_flat".to_string();
        info!(
            target: "strategy_runtime::alor_usdrubf_hybrid",
            action = "bracket_partial_reconcile_wait",
            broker_qty,
            "MR protective partial fill is settling; suppress residual emergency exit until reconcile grace expires"
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
        let qty = ctx.position_qty.unwrap_or(0.0);
        if qty.abs() < 1e-9 {
            self.clear_bracket_terminal_reconcile();
            return Vec::new();
        }
        self.exit_intent_inflight = false;
        self.clear_bracket_terminal_reconcile();
        let pos = PositionEvent {
            symbol: self.config.symbol.clone(),
            qty,
            existing: false,
            avg_price: self
                .open_position
                .as_ref()
                .map(|position| position.entry_price)
                .unwrap_or(0.0),
            ts_utc: now_ts_utc_ms.div_euclid(1_000),
        };
        self.emit_broker_residual_emergency_exit(ctx, &pos, "bracket_terminal_reconcile_timeout")
    }

    fn maybe_emit_live_entry_intent(
        &mut self,
        ctx: &StrategyCtx,
        bar: &BarEvent,
        reference_price: f64,
        intents: &mut Vec<Intent>,
    ) {
        let Some(mut pending) = self.pending_entry.clone() else {
            return;
        };
        if self
            .entry_reject_deferred_until_bar_ts
            .is_some_and(|ts| ts == bar.close_time_utc)
        {
            return;
        }
        if self.entry_intent_inflight || self.open_position.is_some() {
            return;
        }
        let size = self.compute_entry_size(reference_price);
        // The order command is authoritative: both MR and BO must wait for the
        // quantity actually sent to the broker, rather than a signal-time default.
        pending.target_qty = size as f64;
        pending.partial_started_at_ms = None;
        self.pending_entry = Some(pending.clone());
        let side = pending.side.entry_side();
        intents.push(Intent::Market {
            qty: size as f64,
            side,
            fill_price: Some(reference_price),
            comment: Some(format!(
                "{}|entry|{}",
                self.config.symbol,
                pending.owner.as_str()
            )),
        });
        self.entry_intent_inflight = true;
        self.hybrid_state = HybridState::Pending;
        self.lifecycle_stage = "live_entry_intent_emitted".to_string();
        self.log_live_intent_emitted_entry(ctx, bar, reference_price, size as f64, side);
    }

    fn log_live_intent_emitted_entry(
        &self,
        ctx: &StrategyCtx,
        bar: &BarEvent,
        reference_price: f64,
        qty: f64,
        side: Side,
    ) {
        info!(
            strategy_id = ctx.strategy_id.as_str(),
            strategy = "alor_usdrubf_hybrid",
            action = "intent_emitted",
            intent_class = "entry",
            symbol = self.config.symbol.as_str(),
            bar_ts_utc = bar.close_time_utc,
            qty,
            side = ?side,
            reference_price,
            "live entry intent emitted (reference price is model execution reference, not fill)"
        );
    }

    fn log_live_intent_emitted_exit(
        &self,
        ctx: &StrategyCtx,
        bar_ts_utc: i64,
        qty: f64,
        side: Side,
        reference_price: f64,
        reason: &str,
    ) {
        info!(
            strategy_id = ctx.strategy_id.as_str(),
            strategy = "alor_usdrubf_hybrid",
            action = "intent_emitted",
            intent_class = "exit",
            symbol = self.config.symbol.as_str(),
            bar_ts_utc,
            qty,
            side = ?side,
            reference_price_from_signal = reference_price,
            exit_reason = reason,
            "live exit intent emitted (reference price is signal path, not fill)"
        );
    }

    fn maybe_emit_live_exit_intent(
        &mut self,
        ctx: &StrategyCtx,
        reason: String,
        exit_price: f64,
        bar_ts_utc: i64,
        intents: &mut Vec<Intent>,
    ) {
        let Some(pos) = self.open_position.as_ref() else {
            return;
        };
        if self
            .exit_reject_deferred_until_bar_ts
            .is_some_and(|ts| ts == bar_ts_utc)
        {
            return;
        }
        if self.exit_intent_inflight {
            return;
        }
        let side = pos.side.exit_side();
        let qty = pos.size as f64;
        intents.extend(self.emit_cancel_all_protection(Some(side)));
        intents.push(Intent::Market {
            qty,
            side,
            fill_price: Some(exit_price),
            comment: Some(format!("{}|exit|{}", self.config.symbol, reason)),
        });
        self.exit_intent_inflight = true;
        self.lifecycle_stage = "live_exit_intent_emitted".to_string();
        self.log_live_intent_emitted_exit(ctx, bar_ts_utc, qty, side, exit_price, reason.as_str());
    }

    fn build_research_snapshot(&self, bar: &BarEvent, local_dt: NaiveDateTime) -> ResearchSnapshot {
        ResearchSnapshot {
            close: bar.close,
            high: bar.h,
            low: bar.l,
            local_dt,
            session_open: self.day_open,
            session_vwap: self.session_vwap(bar.close),
            session_range: self.session_range(),
            elapsed_hours: self.elapsed_hours(local_dt),
            ret_from_open: self.ret_from_open(bar.close),
            bo_was_long_today: self.bo_was_long_today,
            bo_was_short_today: self.bo_was_short_today,
        }
    }

    fn evaluate_mr_from_snapshot(&self, rs: &ResearchSnapshot) -> Option<PendingEntry> {
        if rs.local_dt.time() > self.config.mr_last_entry_time {
            return None;
        }
        let scale = rs.session_range?;
        if !scale.is_finite() || scale <= 0.0 || rs.close.abs() <= f64::EPSILON {
            return None;
        }
        let rel_scale = scale / rs.close;
        let dist = rs.close - rs.session_vwap;
        if !(self.config.mr_min_rel_range < rel_scale && rel_scale < self.config.mr_max_rel_range) {
            return None;
        }
        if !(dist > 0.0 && dist < self.config.mr_k_short * scale) {
            return None;
        }
        Some(PendingEntry {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            reason: "mr_short_signal".to_string(),
            scale_at_signal: scale,
            signal_price: rs.close,
            stop1: None,
            stop2: None,
            target_qty: self.configured_live_target_qty(),
            partial_started_at_ms: None,
        })
    }

    fn evaluate_bo_from_snapshot(&self, rs: &ResearchSnapshot) -> Option<PendingEntry> {
        let scale = rs.session_range?;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        if rs.elapsed_hours? < self.config.bo_wait_hours {
            return None;
        }
        let session_open = rs.session_open?;
        let ret_from_open = rs.ret_from_open?;
        let can_long = ret_from_open >= -self.config.bo_big_move_threshold;
        let can_short = ret_from_open <= self.config.bo_big_move_threshold;
        let long_level = session_open + self.config.bo_k * scale;
        let short_level = session_open - self.config.bo_k * scale;

        if can_long && !rs.bo_was_long_today && rs.close > long_level {
            return Some(PendingEntry {
                owner: Owner::Breakout,
                side: PositionSide::Long,
                reason: "bo_long_signal".to_string(),
                scale_at_signal: scale,
                signal_price: rs.close,
                stop1: Some(session_open + self.config.bo_stop1_range * scale),
                stop2: Some(session_open - self.config.bo_stop2_range * scale),
                target_qty: self.configured_live_target_qty(),
                partial_started_at_ms: None,
            });
        }
        if can_short && !rs.bo_was_short_today && rs.close < short_level {
            return Some(PendingEntry {
                owner: Owner::Breakout,
                side: PositionSide::Short,
                reason: "bo_short_signal".to_string(),
                scale_at_signal: scale,
                signal_price: rs.close,
                stop1: Some(session_open - self.config.bo_stop1_range * scale),
                stop2: Some(session_open + self.config.bo_stop2_range * scale),
                target_qty: self.configured_live_target_qty(),
                partial_started_at_ms: None,
            });
        }
        None
    }

    fn evaluate_signal_research(&self, rs: &ResearchSnapshot) -> Option<PendingEntry> {
        self.evaluate_mr_from_snapshot(rs)
            .or_else(|| self.evaluate_bo_from_snapshot(rs))
    }

    fn evaluate_exit_research(&self, rs: &ResearchSnapshot) -> Option<(String, f64)> {
        let pos = self.open_position.as_ref()?;
        if !self.owner_confirmed_by_live_event {
            // Conservative bootstrap mode: avoid owner-dependent exits until live broker truth confirms position context.
            return None;
        }
        if pos.owner == Owner::MeanRev {
            let stop_price = pos.stop_price?;
            let take_price = pos.take_price?;
            if rs.high >= stop_price {
                return Some(("mr_stop".to_string(), stop_price));
            }
            if rs.low <= take_price {
                return Some(("mr_take".to_string(), take_price));
            }
            if rs.local_dt.time() >= self.config.mr_force_exit_time {
                return Some(("mr_time_cutoff".to_string(), rs.close));
            }
            return None;
        }

        let stop1 = pos.stop1?;
        let stop2 = pos.stop2?;
        if pos.side == PositionSide::Long {
            if rs.low <= stop2 {
                return Some(("bo_stop2_long".to_string(), stop2));
            }
            if rs.local_dt.minute() == 50 && rs.close < stop1 {
                return Some(("bo_stop1_long".to_string(), rs.close));
            }
            if rs.local_dt.time() >= self.config.bo_eod_exit_time {
                return Some(("bo_eod_exit".to_string(), rs.close));
            }
            return None;
        }

        if rs.high >= stop2 {
            return Some(("bo_stop2_short".to_string(), stop2));
        }
        if rs.local_dt.minute() == 50 && rs.close > stop1 {
            return Some(("bo_stop1_short".to_string(), rs.close));
        }
        if rs.local_dt.time() >= self.config.bo_eod_exit_time {
            return Some(("bo_eod_exit".to_string(), rs.close));
        }
        None
    }

    fn apply_exit(
        &mut self,
        reason: String,
        exit_price: f64,
        bar: &BarEvent,
        intents: &mut Vec<Intent>,
    ) {
        let Some(pos) = self.open_position.clone() else {
            return;
        };
        let gross = if pos.side == PositionSide::Long {
            (exit_price - pos.entry_price) * pos.size as f64
        } else {
            (pos.entry_price - exit_price) * pos.size as f64
        };
        let commission = (pos.entry_price + exit_price)
            * pos.size as f64
            * (self.config.commission_pct_per_side / 100.0);
        self.cash += gross - commission;
        intents.push(Intent::Market {
            qty: pos.size as f64,
            side: pos.side.exit_side(),
            fill_price: Some(exit_price),
            comment: Some(format!("{}|exit|{}", self.config.symbol, reason)),
        });
        self.open_position = None;
        self.exit_intent_inflight = false;
        self.entry_intent_inflight = false;
        self.clear_mr_protection_tracking();
        self.hybrid_state = HybridState::Flat;
        let _ = bar;
    }

    fn classify_broker_position_transition(
        &self,
        prev_qty: f64,
        prev_avg: f64,
        initialized: bool,
        pos: &PositionEvent,
    ) -> &'static str {
        let q = pos.qty;
        let avg = pos.avg_price;
        let flat = q.abs() < 1e-9;
        let prev_flat = prev_qty.abs() < 1e-9;
        if !initialized {
            return if flat {
                "initial_broker_sync_flat"
            } else {
                "initial_broker_sync_open"
            };
        }
        if prev_flat && flat {
            "flat_reconfirm"
        } else if prev_flat && !flat {
            "flat_to_open"
        } else if !prev_flat && flat {
            "open_to_flat"
        } else {
            let sp = prev_qty.signum();
            let sn = q.signum();
            if sp != sn && sn != 0.0 && sp != 0.0 {
                "direction_flip"
            } else {
                let tick = self.config.tick_size.max(1e-12);
                let qty_eps = 0.25_f64;
                if (q - prev_qty).abs() >= qty_eps || (avg - prev_avg).abs() >= tick {
                    "qty_or_avg_material_change"
                } else {
                    "duplicate_reconfirm"
                }
            }
        }
    }

    fn log_broker_position_transition_if_needed(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) {
        let transition = self.classify_broker_position_transition(
            self.last_logged_broker_qty,
            self.last_logged_broker_avg,
            self.last_logged_broker_initialized,
            pos,
        );
        let loud = !matches!(transition, "flat_reconfirm" | "duplicate_reconfirm");
        if loud {
            info!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "position_transition",
                transition,
                symbol = pos.symbol.as_str(),
                event_ts_utc = pos.ts_utc,
                qty = pos.qty,
                avg_price = pos.avg_price,
                lifecycle_stage = self.lifecycle_stage.as_str(),
                "broker position transition"
            );
        } else {
            debug!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "broker_position_duplicate",
                transition,
                symbol = pos.symbol.as_str(),
                qty = pos.qty,
                avg_price = pos.avg_price,
                "broker position reconfirm"
            );
        }
        let flat = pos.qty.abs() < 1e-9;
        self.last_logged_broker_qty = pos.qty;
        self.last_logged_broker_avg = if flat { 0.0 } else { pos.avg_price };
        self.last_logged_broker_initialized = true;
    }

    fn log_entry_exit_inflight_transitions(&mut self, ctx: &StrategyCtx) {
        if self.entry_intent_inflight != self.last_logged_entry_inflight {
            info!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "risk_state_changed",
                field = "entry_intent_inflight",
                value = self.entry_intent_inflight,
                lifecycle_stage = self.lifecycle_stage.as_str(),
                "entry intent inflight changed"
            );
            self.last_logged_entry_inflight = self.entry_intent_inflight;
        }
        if self.exit_intent_inflight != self.last_logged_exit_inflight {
            info!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "risk_state_changed",
                field = "exit_intent_inflight",
                value = self.exit_intent_inflight,
                lifecycle_stage = self.lifecycle_stage.as_str(),
                "exit intent inflight changed"
            );
            self.last_logged_exit_inflight = self.exit_intent_inflight;
        }
        let entry_ts = self.entry_reject_deferred_until_bar_ts;
        if entry_ts != self.last_logged_entry_reject_defer_ts {
            info!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "risk_state_changed",
                field = "entry_reject_deferred_until_bar_ts",
                entry_reject_deferred_until_bar_ts = ?entry_ts,
                lifecycle_stage = self.lifecycle_stage.as_str(),
                "entry reject defer bar ts changed"
            );
            self.last_logged_entry_reject_defer_ts = entry_ts;
        }
        let exit_ts = self.exit_reject_deferred_until_bar_ts;
        if exit_ts != self.last_logged_exit_reject_defer_ts {
            info!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "risk_state_changed",
                field = "exit_reject_deferred_until_bar_ts",
                exit_reject_deferred_until_bar_ts = ?exit_ts,
                lifecycle_stage = self.lifecycle_stage.as_str(),
                "exit reject defer bar ts changed"
            );
            self.last_logged_exit_reject_defer_ts = exit_ts;
        }
    }

    fn reset_startup_log_gates(&mut self) {
        self.startup_replay_tail_info_emitted = false;
        self.awaiting_live_bar_info_emitted = false;
        self.recovered_bar_suppressed_info_emitted = false;
    }
}

impl Strategy for AlorUsdrubfHybridStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            return Vec::new();
        }
        if self
            .last_processed_bar_ts
            .is_some_and(|ts| bar.close_time_utc <= ts)
        {
            return Vec::new();
        }
        self.lifecycle_stage = "live".to_string();
        self.last_bar_ts = Some(bar.close_time_utc);

        if matches!(ctx.trade_mode, crate::TradeMode::Live) && !self.config.enable_live_execution {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            self.sync_state();
            return Vec::new();
        }
        if matches!(ctx.trade_mode, crate::TradeMode::Live) && !self.live_ready {
            if self.is_live_startup_bar_stale(ctx, bar.close_time_utc) {
                self.lifecycle_stage = "replay_tail_suppressed".to_string();
                if !self.startup_replay_tail_info_emitted {
                    self.startup_replay_tail_info_emitted = true;
                    info!(
                        strategy_id = ctx.strategy_id.as_str(),
                        strategy = "alor_usdrubf_hybrid",
                        action = "replay_guard_armed",
                        guard = "startup_replay_tail",
                        bar_ts_utc = bar.close_time_utc,
                        now_ts_utc = ctx.now_ts_utc(),
                        max_silence_bars_sec = self.config.max_silence_bars_sec,
                        "startup replay tail bar suppressed (first)"
                    );
                } else {
                    debug!(
                        strategy_id = ctx.strategy_id.as_str(),
                        strategy = "alor_usdrubf_hybrid",
                        action = "replay_guard_armed",
                        guard = "startup_replay_tail",
                        bar_ts_utc = bar.close_time_utc,
                        "startup replay tail bar suppressed (repeat)"
                    );
                }
                self.last_processed_bar_ts = Some(bar.close_time_utc);
                self.sync_state();
                return Vec::new();
            }
            if !matches!(bar.origin, DataOrigin::Live) {
                self.lifecycle_stage = "awaiting_fresh_live_origin_bar".to_string();
                if !self.awaiting_live_bar_info_emitted {
                    self.awaiting_live_bar_info_emitted = true;
                    info!(
                        strategy_id = ctx.strategy_id.as_str(),
                        strategy = "alor_usdrubf_hybrid",
                        action = "replay_guard_armed",
                        guard = "awaiting_fresh_live_bar",
                        bar_ts_utc = bar.close_time_utc,
                        origin = ?bar.origin,
                        "replay guard armed; waiting for live-origin bar"
                    );
                } else {
                    debug!(
                        strategy_id = ctx.strategy_id.as_str(),
                        strategy = "alor_usdrubf_hybrid",
                        action = "replay_guard_armed",
                        guard = "awaiting_fresh_live_bar",
                        bar_ts_utc = bar.close_time_utc,
                        origin = ?bar.origin,
                        "still awaiting live-origin bar"
                    );
                }
                self.last_processed_bar_ts = Some(bar.close_time_utc);
                self.sync_state();
                return Vec::new();
            }
            self.live_ready = true;
            self.lifecycle_stage = "live_ready".to_string();
            self.recovered_bar_suppressed_info_emitted = false;
            info!(
                strategy_id = ctx.strategy_id.as_str(),
                strategy = "alor_usdrubf_hybrid",
                action = "replay_guard_cleared",
                symbol = self.config.symbol.as_str(),
                event_ts_utc = bar.close_time_utc,
                now_ts_utc = ctx.now_ts_utc(),
                live_ready = true,
                "replay guard cleared; first live-origin bar"
            );
        }
        if matches!(ctx.trade_mode, crate::TradeMode::Live)
            && Self::is_recovered_or_non_live_bar_origin(&bar.origin)
        {
            self.lifecycle_stage = "recovered_bar_suppressed".to_string();
            if !self.recovered_bar_suppressed_info_emitted {
                self.recovered_bar_suppressed_info_emitted = true;
                info!(
                    strategy_id = ctx.strategy_id.as_str(),
                    strategy = "alor_usdrubf_hybrid",
                    action = "lifecycle_stage",
                    stage = "recovered_bar_suppressed",
                    bar_ts_utc = bar.close_time_utc,
                    origin = ?bar.origin,
                    "non-live bar origin suppressed in live mode (first)"
                );
            } else {
                debug!(
                    strategy_id = ctx.strategy_id.as_str(),
                    strategy = "alor_usdrubf_hybrid",
                    action = "lifecycle_stage",
                    stage = "recovered_bar_suppressed",
                    bar_ts_utc = bar.close_time_utc,
                    origin = ?bar.origin,
                    "non-live bar suppressed (repeat)"
                );
            }
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            self.sync_state();
            return Vec::new();
        }

        let local_dt = utc_to_local(bar.close_time_utc, self.config.timezone_offset_hours);
        if !self.is_model_session_bar(local_dt) {
            self.lifecycle_stage = "outside_model_session_suppressed".to_string();
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            self.sync_state();
            return Vec::new();
        }
        let local_date = local_dt.date();
        if self.current_date_local != Some(local_date) {
            self.reset_day(local_date);
        }
        self.update_session_metrics(bar, local_dt);
        let research = self.build_research_snapshot(bar, local_dt);

        let mut intents = Vec::new();
        if matches!(ctx.trade_mode, crate::TradeMode::Live) {
            if self
                .entry_reject_deferred_until_bar_ts
                .is_some_and(|ts| bar.close_time_utc > ts)
            {
                self.entry_reject_deferred_until_bar_ts = None;
            }
            if self
                .exit_reject_deferred_until_bar_ts
                .is_some_and(|ts| bar.close_time_utc > ts)
            {
                self.exit_reject_deferred_until_bar_ts = None;
            }
            self.maybe_emit_live_mr_brackets(ctx, bar.close_time_utc, &mut intents);
            if let Some((reason, exit_price)) = self.evaluate_exit_research(&research) {
                let suppress_market_exit = self.open_position.as_ref().is_some_and(|position| {
                    position.owner == Owner::MeanRev
                        && matches!(reason.as_str(), "mr_take" | "mr_stop")
                });
                if suppress_market_exit {
                    self.log_entry_exit_inflight_transitions(ctx);
                    self.last_processed_bar_ts = Some(bar.close_time_utc);
                    self.sync_state();
                    return intents;
                }
                self.maybe_emit_live_exit_intent(
                    ctx,
                    reason,
                    exit_price,
                    bar.close_time_utc,
                    &mut intents,
                );
                self.log_entry_exit_inflight_transitions(ctx);
                self.last_processed_bar_ts = Some(bar.close_time_utc);
                self.sync_state();
                return intents;
            }
            self.maybe_emit_live_entry_intent(ctx, bar, bar.o, &mut intents);
        } else {
            self.maybe_fill_pending_entry(bar, &mut intents);
            if let Some((reason, exit_price)) = self.evaluate_exit_research(&research) {
                self.apply_exit(reason, exit_price, bar, &mut intents);
                self.last_processed_bar_ts = Some(bar.close_time_utc);
                self.sync_state();
                return intents;
            }
        }

        if self.open_position.is_none()
            && self.pending_entry.is_none()
            && !(matches!(ctx.trade_mode, crate::TradeMode::Live) && self.entry_intent_inflight)
        {
            let signal = self.evaluate_signal_research(&research);
            if let Some(signal) = signal {
                if signal.owner == Owner::Breakout {
                    if signal.side == PositionSide::Long {
                        self.bo_was_long_today = true;
                    } else if signal.side == PositionSide::Short {
                        self.bo_was_short_today = true;
                    }
                }
                info!(
                    strategy_id = ctx.strategy_id.as_str(),
                    strategy = "alor_usdrubf_hybrid",
                    action = "signal_generated",
                    owner = signal.owner.as_str(),
                    side = signal.side.as_str(),
                    reason = signal.reason.as_str(),
                    signal_price = signal.signal_price,
                    scale_at_signal = signal.scale_at_signal,
                    symbol = self.config.symbol.as_str(),
                    bar_ts_utc = bar.close_time_utc,
                    "research signal accepted into pending entry"
                );
                self.pending_entry = Some(signal);
                self.hybrid_state = HybridState::Pending;
                if matches!(ctx.trade_mode, crate::TradeMode::Live) {
                    // A completed signal bar arrives at the boundary where the
                    // research replay enters on the next bar open. Emit now,
                    // using the signal close as the observable next-open proxy,
                    // instead of waiting for one more completed bar event.
                    self.maybe_emit_live_entry_intent(ctx, bar, bar.close, &mut intents);
                }
            }
        }
        self.log_entry_exit_inflight_transitions(ctx);
        self.last_processed_bar_ts = Some(bar.close_time_utc);
        self.sync_state();
        intents
    }

    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        self.pending_request_ids.remove(&ack.request_id);
        if let Some(order_id) = ack.broker_order_id {
            self.tracked_order_ids.insert(order_id);
        }
        if matches!(
            ack.status,
            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
        ) {
            let reject_ts = self.last_bar_ts.unwrap_or(0);
            if self.exit_intent_inflight && self.open_position.is_some() {
                self.entry_intent_inflight = false;
                self.exit_intent_inflight = false;
                self.exit_reject_deferred_until_bar_ts = Some(reject_ts);
                self.lifecycle_stage = "exit_reject_deferred_retry".to_string();
                warn!(
                    strategy_id = ctx.strategy_id.as_str(),
                    strategy = "alor_usdrubf_hybrid",
                    action = "command_ack_rejected",
                    reject_policy = "exit_reject_deferred_next_bar",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    defer_until_after_bar_ts_utc = reject_ts,
                    "broker rejected exit; defer retry to following bar"
                );
            } else if self.entry_intent_inflight && self.pending_entry.is_some() {
                self.entry_intent_inflight = false;
                self.exit_intent_inflight = false;
                self.entry_reject_deferred_until_bar_ts = Some(reject_ts);
                self.lifecycle_stage = "entry_reject_deferred_retry".to_string();
                warn!(
                    strategy_id = ctx.strategy_id.as_str(),
                    strategy = "alor_usdrubf_hybrid",
                    action = "command_ack_rejected",
                    reject_policy = "entry_reject_deferred_next_bar",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    defer_until_after_bar_ts_utc = reject_ts,
                    "broker rejected entry; defer retry to following bar"
                );
            } else {
                self.entry_intent_inflight = false;
                self.exit_intent_inflight = false;
                self.lifecycle_stage = "broker_ack_rejected".to_string();
                warn!(
                    strategy_id = ctx.strategy_id.as_str(),
                    strategy = "alor_usdrubf_hybrid",
                    action = "command_ack_rejected",
                    reject_policy = "clear_inflight_terminal",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    "broker ack terminal reject; inflight flags cleared"
                );
            }
            self.sync_state();
        }
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent> {
        let intents = Vec::new();
        if ord.symbol == self.config.symbol && ord.order_id > 0 {
            self.tracked_order_ids.insert(ord.order_id);
            if let Some(request_id) = ord.request_id {
                self.pending_request_ids.remove(&request_id);
            }
            let status = ord.status.to_ascii_lowercase();
            if self
                .open_position
                .as_ref()
                .is_some_and(|pos| pos.owner == Owner::MeanRev)
            {
                match Self::parse_mr_protection_role(ord.comment.as_deref()) {
                    Some(ProtectionRole::Tp) => {
                        self.pending_tp_bar_ts_utc = None;
                        self.tp_order_id = Some(ord.order_id);
                        if status == "filled" {
                            self.mark_bracket_terminal_reconcile();
                            self.exit_intent_inflight = true;
                            self.lifecycle_stage = "mr_tp_filled_awaiting_broker_flat".to_string();
                            self.tp_order_id = None;
                        } else if matches!(
                            status.as_str(),
                            "cancelled" | "canceled" | "rejected" | "expired"
                        ) {
                            self.tp_order_id = None;
                        }
                    }
                    Some(ProtectionRole::Sl) => {}
                    None => {}
                }
            }
            if status == "filled"
                || status == "cancelled"
                || status == "canceled"
                || status == "rejected"
            {
                self.tracked_order_ids.remove(&ord.order_id);
            }
            if self.tracked_order_ids.is_empty() && self.open_position.is_none() {
                self.entry_intent_inflight = false;
                if self.pending_entry.is_none() {
                    self.hybrid_state = HybridState::Flat;
                }
            }
            self.sync_state();
        }
        intents
    }

    fn on_stop_order(&mut self, ctx: &StrategyCtx, ord: &StopOrderEvent) -> Vec<Intent> {
        let mut intents = Vec::new();
        debug!(
            strategy_id = ctx.strategy_id.as_str(),
            strategy = "alor_usdrubf_hybrid",
            action = "stop_order_event",
            symbol = ord.symbol.as_str(),
            stop_order_id = ord.stop_order_id,
            status = %ord.status,
            "stop order stream update"
        );
        if ord.symbol == self.config.symbol
            && self
                .open_position
                .as_ref()
                .is_some_and(|pos| pos.owner == Owner::MeanRev)
            && matches!(
                Self::parse_mr_protection_role(ord.comment.as_deref()),
                Some(ProtectionRole::Sl)
            )
        {
            let status = ord.status.to_ascii_lowercase();
            self.pending_sl_bar_ts_utc = None;
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
                self.exit_intent_inflight = true;
                self.lifecycle_stage = "mr_sl_filled_awaiting_broker_flat".to_string();
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
                    | "executed"
                    | "triggered"
                    | "done"
                    | "completed"
                    | "cancelled"
                    | "canceled"
                    | "rejected"
                    | "expired"
            ) {
                self.sl_stop_order_id = None;
            }
            if matches!(
                status.as_str(),
                "cancelled" | "canceled" | "rejected" | "expired"
            ) {
                self.sl_exchange_order_id = None;
            }
        }
        self.sync_state();
        intents
    }

    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }
        self.log_broker_position_transition_if_needed(ctx, pos);
        if pos.qty.abs() < 1e-9 {
            let closing_side = self
                .open_position
                .as_ref()
                .map(|open| open.side.exit_side());
            let intents = self.emit_cancel_all_protection(closing_side);
            self.open_position = None;
            self.owner_confirmed_by_live_event = true;
            self.exit_intent_inflight = false;
            self.clear_bracket_terminal_reconcile();
            self.tracked_order_ids.clear();
            self.hybrid_state = if self.pending_entry.is_some() {
                HybridState::Pending
            } else {
                HybridState::Flat
            };
            self.lifecycle_stage = "broker_position_flat".to_string();
            self.sync_state();
            return intents;
        }

        let side = if pos.qty > 0.0 {
            PositionSide::Long
        } else {
            PositionSide::Short
        };
        let size = pos.qty.abs().floor().max(1.0) as i64;
        if let Some(mut pending) = self.pending_entry.clone() {
            if pending.target_qty > 1.0 {
                let expected_sign = match pending.side {
                    PositionSide::Long => 1.0,
                    PositionSide::Short => -1.0,
                };
                if pos.qty.signum() != expected_sign
                    || pos.qty.abs() > pending.target_qty + f64::EPSILON
                {
                    self.pending_entry = None;
                    let intents =
                        self.emit_broker_residual_emergency_exit(ctx, pos, "partial_entry_invalid");
                    self.sync_state();
                    return intents;
                }
                if pos.qty.abs() + f64::EPSILON < pending.target_qty {
                    if pending.partial_started_at_ms.is_none() {
                        pending.partial_started_at_ms = Some(Utc::now().timestamp_millis());
                    }
                    self.pending_entry = Some(pending.clone());
                    self.lifecycle_stage = "partial_entry_waiting_target".to_string();
                    info!(
                        target: "strategy_runtime::alor_usdrubf_hybrid",
                        action = "partial_entry_progress",
                        broker_qty = pos.qty,
                        target_qty = pending.target_qty,
                        "live entry partially filled; waiting for target before opening strategy position"
                    );
                    self.sync_state();
                    return Vec::new();
                }
            }
        }
        if self.open_position.is_none() && self.pending_entry.is_none() && !pos.existing {
            let synthetic_pending = PendingEntry {
                owner: Owner::Breakout,
                side,
                reason: "unexpected_broker_residual".to_string(),
                scale_at_signal: 0.0,
                signal_price: pos.avg_price,
                stop1: None,
                stop2: None,
                target_qty: 1.0,
                partial_started_at_ms: None,
            };
            self.open_position = Some(self.build_open_position_from_pending(
                &synthetic_pending,
                pos.ts_utc,
                pos.avg_price,
                size,
            ));
            let intents =
                self.emit_broker_residual_emergency_exit(ctx, pos, "unexpected_broker_residual");
            self.owner_confirmed_by_live_event = true;
            self.hybrid_state = HybridState::Open;
            self.sync_state();
            return intents;
        }
        if let Some(open) = self.open_position.as_ref() {
            if self.exit_intent_inflight
                || self.bracket_terminal_reconcile_active(Utc::now().timestamp_millis())
            {
                self.lifecycle_stage = "exit_filled_awaiting_broker_flat".to_string();
                self.sync_state();
                return Vec::new();
            }
            if Self::mr_bracket_protective_partial_progress(open, pos.qty) {
                self.mark_mr_bracket_partial_reconcile(pos.qty);
                self.sync_state();
                return Vec::new();
            }
            let previous_qty = match open.side {
                PositionSide::Long => open.size as f64,
                PositionSide::Short => -(open.size as f64),
            };
            if (previous_qty - pos.qty).abs() >= 0.5 {
                let reason = if previous_qty.signum() != pos.qty.signum() {
                    "broker_position_sign_flip"
                } else {
                    "broker_position_size_changed"
                };
                let intents = self.emit_broker_residual_emergency_exit(ctx, pos, reason);
                if let Some(open) = self.open_position.as_mut() {
                    open.side = side;
                    open.size = size;
                    if pos.avg_price > 0.0 {
                        open.entry_price = pos.avg_price;
                    }
                }
                self.owner_confirmed_by_live_event = true;
                self.hybrid_state = HybridState::Open;
                self.sync_state();
                return intents;
            }
        }
        let entry_price = if pos.avg_price > 0.0 {
            pos.avg_price
        } else if let Some(open) = self.open_position.as_ref() {
            open.entry_price
        } else {
            self.pending_entry
                .as_ref()
                .map(|pending| pending.signal_price)
                .unwrap_or(0.0)
        };
        if let Some(pending) = self.pending_entry.clone() {
            self.open_position = Some(self.build_open_position_from_pending(
                &pending,
                pos.ts_utc,
                entry_price,
                size,
            ));
            self.pending_entry = None;
        } else if let Some(open) = self.open_position.as_mut() {
            open.side = side;
            open.size = size;
            open.entry_price = entry_price;
        } else {
            let synthetic_pending = PendingEntry {
                owner: Owner::Breakout,
                side,
                reason: "broker_position_restore".to_string(),
                scale_at_signal: 0.0,
                signal_price: entry_price,
                stop1: None,
                stop2: None,
                target_qty: 1.0,
                partial_started_at_ms: None,
            };
            self.open_position = Some(self.build_open_position_from_pending(
                &synthetic_pending,
                pos.ts_utc,
                entry_price,
                size,
            ));
        }
        self.entry_intent_inflight = false;
        self.exit_intent_inflight = false;
        self.owner_confirmed_by_live_event = true;
        if self
            .open_position
            .as_ref()
            .is_some_and(|position| position.owner != Owner::MeanRev)
        {
            self.clear_mr_protection_tracking();
        }
        self.hybrid_state = HybridState::Open;
        self.lifecycle_stage = "broker_position_open".to_string();
        let mut intents = Vec::new();
        self.maybe_emit_live_mr_brackets(ctx, pos.ts_utc, &mut intents);
        self.sync_state();
        intents
    }

    fn on_timer(&mut self, ctx: &StrategyCtx, now_ts_utc_ms: i64) -> Vec<Intent> {
        let reconcile_intents = self.emit_bracket_reconcile_timeout_exit(ctx, now_ts_utc_ms);
        if !reconcile_intents.is_empty() {
            self.sync_state();
            return reconcile_intents;
        }
        let Some(pending) = self.pending_entry.clone() else {
            return Vec::new();
        };
        let Some(started_at_ms) = pending.partial_started_at_ms else {
            return Vec::new();
        };
        let position_qty = ctx.position_qty.unwrap_or(0.0);
        if pending.target_qty <= 1.0
            || position_qty.abs() <= f64::EPSILON
            || now_ts_utc_ms.saturating_sub(started_at_ms) < 3_000
        {
            return Vec::new();
        }
        let mut intents = self
            .tracked_order_ids
            .iter()
            .copied()
            .map(|order_id| Intent::Cancel { order_id }.with_class(IntentClass::CancelCleanup))
            .collect::<Vec<_>>();
        let pos = PositionEvent {
            symbol: self.config.symbol.clone(),
            qty: position_qty,
            existing: false,
            avg_price: pending.signal_price,
            ts_utc: now_ts_utc_ms.div_euclid(1_000),
        };
        self.pending_entry = None;
        intents.extend(self.emit_broker_residual_emergency_exit(
            ctx,
            &pos,
            "partial_entry_fill_timeout",
        ));
        self.lifecycle_stage = "partial_entry_timeout_flatten".to_string();
        self.sync_state();
        intents
    }

    fn on_bootstrap_snapshot(
        &mut self,
        ctx: &StrategyCtx,
        snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        self.lifecycle_stage = "bootstrapped".to_string();
        self.bootstrap_seen = true;
        self.live_ready = false;
        self.reset_startup_log_gates();
        self.entry_intent_inflight = false;
        self.exit_intent_inflight = false;
        self.entry_reject_deferred_until_bar_ts = None;
        self.exit_reject_deferred_until_bar_ts = None;
        self.pending_request_ids.clear();
        self.tracked_order_ids.clear();
        self.clear_mr_protection_tracking();
        let symbol = self.config.symbol.as_str();
        let snapshot_position = snapshot.positions_strategy.get(symbol);
        let mut snapshot_working_order_ids = BTreeSet::new();
        for (order_id, order) in &snapshot.working_orders_strategy {
            if order.symbol == symbol {
                snapshot_working_order_ids.insert(*order_id);
                if matches!(
                    Self::parse_mr_protection_role(order.comment.as_deref()),
                    Some(ProtectionRole::Tp)
                ) {
                    self.tp_order_id = Some(*order_id);
                }
            }
        }
        let mut has_snapshot_stop_orders = false;
        for (stop_order_id, order) in &snapshot.working_stop_orders_strategy {
            if order.symbol == symbol {
                has_snapshot_stop_orders = true;
                if matches!(
                    Self::parse_mr_protection_role(order.comment.as_deref()),
                    Some(ProtectionRole::Sl)
                ) {
                    self.sl_stop_order_id = Some(stop_order_id.clone());
                    self.sl_exchange_order_id = order.exchange_order_id;
                }
            }
        }
        self.tracked_order_ids = snapshot_working_order_ids;

        if let Some(position) = snapshot_position {
            if position.qty.abs() >= 1e-9 {
                let inferred_owner = self.infer_owner_from_existing_state();
                let owner = inferred_owner.unwrap_or_else(|| self.snapshot_owner_fallback());
                let side = Self::snapshot_position_side(position.qty);
                let size = position.qty.abs().floor().max(1.0) as i64;
                let entry_price = if position.avg_price > 0.0 {
                    position.avg_price
                } else {
                    self.open_position
                        .as_ref()
                        .map(|open| open.entry_price)
                        .unwrap_or(0.0)
                };
                self.pending_entry = None;
                self.entry_intent_inflight = false;
                self.exit_intent_inflight = false;
                self.open_position = Some(OpenPosition {
                    owner,
                    side,
                    entry_ts: utc_to_local(position.ts_utc, self.config.timezone_offset_hours),
                    entry_price,
                    size,
                    stop_price: None,
                    take_price: None,
                    stop1: None,
                    stop2: None,
                });
                self.hybrid_state = HybridState::Open;
                if inferred_owner.is_some() {
                    self.owner_confirmed_by_live_event = true;
                    self.lifecycle_stage = "bootstrap_non_flat_adopted".to_string();
                } else {
                    self.owner_confirmed_by_live_event = false;
                    self.lifecycle_stage = "bootstrap_non_flat_owner_unconfirmed".to_string();
                }
            } else {
                self.open_position = None;
                self.owner_confirmed_by_live_event = true;
                if self.tracked_order_ids.is_empty() {
                    self.pending_entry = None;
                    self.entry_intent_inflight = false;
                    self.hybrid_state = HybridState::Flat;
                } else {
                    // Safe fallback for unsupported order-ownership restore:
                    // block new entry intents until broker callbacks reconcile terminal order state.
                    self.pending_entry = None;
                    self.entry_intent_inflight = true;
                    self.hybrid_state = HybridState::Pending;
                    self.lifecycle_stage = "bootstrap_working_orders_pending_reconcile".to_string();
                }
            }
        }
        info!(
            strategy_id = ctx.strategy_id.as_str(),
            strategy = "alor_usdrubf_hybrid",
            action = "bootstrap_processed",
            snapshot_ts_utc = snapshot.snapshot_ts_utc,
            positions_count = snapshot.positions_strategy.len(),
            orders_count = snapshot.working_orders_strategy.len(),
            stop_orders_count = snapshot.working_stop_orders_strategy.len(),
            symbol,
            symbol_position_qty = snapshot_position.map(|pos| pos.qty).unwrap_or(0.0),
            symbol_working_orders = self.tracked_order_ids.len(),
            symbol_working_stop_orders = has_snapshot_stop_orders,
            lifecycle_stage = self.lifecycle_stage.as_str(),
            reconcile_precedence = "live_events > bootstrap_snapshot > runtime_state",
            replay_guard_armed = true,
            live_ready = false,
            "bootstrap processed; reconcile applied; replay guard re-armed"
        );
        self.sync_state();
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        ctx: &StrategyCtx,
        state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        self.lifecycle_stage = "runtime_state_restored".to_string();
        self.runtime_state_restored = true;
        self.live_ready = false;
        self.reset_startup_log_gates();
        self.pending_request_ids = state.pending_requests.iter().copied().collect();
        self.tracked_order_ids = state.known_order_ids.iter().copied().collect();
        self.entry_intent_inflight = false;
        self.exit_intent_inflight = false;
        self.entry_reject_deferred_until_bar_ts = None;
        self.exit_reject_deferred_until_bar_ts = None;
        self.clear_mr_protection_tracking();
        self.owner_confirmed_by_live_event = true;
        info!(
            strategy_id = ctx.strategy_id.as_str(),
            strategy = "alor_usdrubf_hybrid",
            action = "runtime_state_restored",
            restored_pending_requests = self.pending_request_ids.len(),
            restored_known_orders = self.tracked_order_ids.len(),
            replay_guard_armed = true,
            live_ready = false,
            "runtime state restored; trackers initialized; await live bar for replay guard"
        );
        self.sync_state();
        Vec::new()
    }

    fn tracked_order_ids(&self) -> Vec<i64> {
        self.tracked_order_ids.iter().copied().collect()
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
        let comment = format!("AUS|sid={}|c={cycle}|r={role}", ctx.strategy_id);
        Some(comment.chars().filter(|c| c.is_ascii()).take(100).collect())
    }

    fn pending_request_ids(&self) -> Vec<Uuid> {
        self.pending_request_ids.iter().copied().collect()
    }

    fn exit_risk_status(
        &self,
        has_open_position: bool,
    ) -> crate::strategy_host::StrategyExitRiskStatus {
        if self.exit_reject_deferred_until_bar_ts.is_some() && has_open_position {
            return crate::strategy_host::StrategyExitRiskStatus {
                phase_override: Some("ExitRejectDeferredRetry".to_string()),
                exit_recovery_active: true,
                operator_intervention_required: false,
                open_risk_position_unflattened: true,
            };
        }
        if self.exit_intent_inflight && has_open_position {
            return crate::strategy_host::StrategyExitRiskStatus {
                phase_override: Some("ExitIntentInflight".to_string()),
                exit_recovery_active: true,
                operator_intervention_required: false,
                open_risk_position_unflattened: true,
            };
        }
        crate::strategy_host::StrategyExitRiskStatus::default()
    }

    fn warmup_from_history(&mut self, _ctx: &StrategyCtx, bars: &[BarEvent]) -> usize {
        // Keep restored live/pending state intact; warmup is only for indicator aggregates.
        if self.pending_entry.is_some() || self.open_position.is_some() {
            return 0;
        }
        let mut processed = 0usize;
        for bar in bars {
            if bar.symbol != self.config.symbol {
                continue;
            }
            if self
                .last_processed_bar_ts
                .is_some_and(|ts| bar.close_time_utc <= ts)
            {
                continue;
            }
            let local_dt = utc_to_local(bar.close_time_utc, self.config.timezone_offset_hours);
            if !self.is_model_session_bar(local_dt) {
                self.last_processed_bar_ts = Some(bar.close_time_utc);
                continue;
            }
            let local_date = local_dt.date();
            if self.current_date_local != Some(local_date) {
                self.reset_day_aggregates(local_date);
            }
            self.update_session_metrics(bar, local_dt);
            self.last_bar_ts = Some(bar.close_time_utc);
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            processed = processed.saturating_add(1);
        }
        if processed > 0 {
            self.lifecycle_stage = "warmed_up".to_string();
            self.sync_state();
        }
        processed
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        self.clear_mr_protection_tracking();
        if let StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage,
            last_bar_ts,
            last_processed_bar_ts,
            bootstrap_seen,
            runtime_state_restored,
            live_ready,
            hybrid_state,
            current_date_local,
            day_open,
            day_high,
            day_low,
            day_volume_sum,
            day_vwap_num,
            session_start_local,
            bo_was_long_today,
            bo_was_short_today,
            cash,
            pending_entry_owner,
            pending_entry_side,
            pending_request_ids,
            tracked_order_ids,
            entry_intent_inflight,
            pending_entry_reason,
            pending_entry_scale_at_signal,
            pending_entry_signal_price,
            pending_entry_stop1,
            pending_entry_stop2,
            open_position_owner,
            open_position_side,
            exit_intent_inflight,
            open_position_qty,
            open_position_entry_ts,
            open_position_entry_price,
            open_position_stop_price,
            open_position_take_price,
            open_position_stop1,
            open_position_stop2,
        } = &state
        {
            self.lifecycle_stage = lifecycle_stage.clone();
            self.last_bar_ts = *last_bar_ts;
            self.last_processed_bar_ts = *last_processed_bar_ts;
            self.bootstrap_seen = *bootstrap_seen;
            self.runtime_state_restored = *runtime_state_restored;
            self.live_ready = *live_ready;
            self.hybrid_state = match hybrid_state.as_str() {
                "pending" => HybridState::Pending,
                "open" => HybridState::Open,
                _ => HybridState::Flat,
            };
            self.current_date_local = current_date_local
                .as_ref()
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok());
            self.day_open = *day_open;
            self.day_high = *day_high;
            self.day_low = *day_low;
            self.day_volume_sum = *day_volume_sum;
            self.day_vwap_num = *day_vwap_num;
            self.session_start_local = session_start_local
                .as_ref()
                .and_then(|value| parse_naive_datetime(value));
            self.bo_was_long_today = *bo_was_long_today;
            self.bo_was_short_today = *bo_was_short_today;
            if *cash > 0.0 {
                self.cash = *cash;
            }
            self.pending_entry = match (
                pending_entry_owner
                    .as_ref()
                    .and_then(|owner| parse_owner(owner)),
                pending_entry_side
                    .as_ref()
                    .and_then(|side| parse_position_side(side)),
            ) {
                (Some(owner), Some(side)) => Some(PendingEntry {
                    owner,
                    side,
                    reason: pending_entry_reason
                        .clone()
                        .unwrap_or_else(|| "restored".to_string()),
                    scale_at_signal: pending_entry_scale_at_signal.unwrap_or(0.0),
                    signal_price: pending_entry_signal_price.unwrap_or(0.0),
                    stop1: *pending_entry_stop1,
                    stop2: *pending_entry_stop2,
                    target_qty: self.configured_live_target_qty(),
                    partial_started_at_ms: None,
                }),
                _ => None,
            };
            self.pending_request_ids = pending_request_ids.iter().copied().collect();
            self.tracked_order_ids = tracked_order_ids.iter().copied().collect();
            self.entry_intent_inflight = *entry_intent_inflight;
            self.open_position = match (
                open_position_owner
                    .as_ref()
                    .and_then(|owner| parse_owner(owner)),
                open_position_side
                    .as_ref()
                    .and_then(|side| parse_position_side(side)),
            ) {
                (Some(owner), Some(side)) if *open_position_qty >= 1.0 => Some(OpenPosition {
                    owner,
                    side,
                    entry_ts: open_position_entry_ts
                        .as_ref()
                        .and_then(|value| parse_naive_datetime(value))
                        .unwrap_or_else(|| {
                            utc_to_local(
                                last_bar_ts.unwrap_or(0),
                                self.config.timezone_offset_hours,
                            )
                        }),
                    entry_price: open_position_entry_price.unwrap_or(0.0),
                    size: open_position_qty.floor() as i64,
                    stop_price: *open_position_stop_price,
                    take_price: *open_position_take_price,
                    stop1: *open_position_stop1,
                    stop2: *open_position_stop2,
                }),
                _ => None,
            };
            self.exit_intent_inflight = *exit_intent_inflight;
            self.owner_confirmed_by_live_event =
                self.lifecycle_stage != "bootstrap_non_flat_owner_unconfirmed";
            self.entry_reject_deferred_until_bar_ts = None;
            self.exit_reject_deferred_until_bar_ts = None;
            self.last_logged_entry_inflight = self.entry_intent_inflight;
            self.last_logged_exit_inflight = self.exit_intent_inflight;
            self.last_logged_entry_reject_defer_ts = self.entry_reject_deferred_until_bar_ts;
            self.last_logged_exit_reject_defer_ts = self.exit_reject_deferred_until_bar_ts;
        }
        self.state = state;
    }
}

fn utc_to_local(ts_utc: i64, timezone_offset_hours: i32) -> NaiveDateTime {
    let utc = DateTime::<Utc>::from_timestamp(ts_utc, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let offset = FixedOffset::east_opt(timezone_offset_hours.saturating_mul(3600))
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset must be valid"));
    utc.with_timezone(&offset).naive_local()
}

fn parse_owner(value: &str) -> Option<Owner> {
    match value {
        "mean_rev" => Some(Owner::MeanRev),
        "day_breakout_waitfix" => Some(Owner::Breakout),
        _ => None,
    }
}

fn parse_position_side(value: &str) -> Option<PositionSide> {
    match value {
        "long" => Some(PositionSide::Long),
        "short" => Some(PositionSide::Short),
        _ => None,
    }
}

fn parse_naive_datetime(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").ok())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        utc_to_local, AlorUsdrubfHybridConfig, AlorUsdrubfHybridStrategy, OpenPosition, Owner,
        PendingEntry, PositionSide,
    };
    use crate::live_guard::GatewayPhase;
    use crate::state::StrategyState;
    use crate::strategy_host::{
        BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
        RuntimeStateRestored, StopOrderEvent, Strategy, StrategyCtx,
    };
    use crate::{PaperExecutionMode, TradeMode};
    use alor_protocol::{CommandAck, IntentClass, Side};
    use chrono::NaiveTime;
    use uuid::Uuid;

    fn test_config() -> AlorUsdrubfHybridConfig {
        AlorUsdrubfHybridConfig {
            symbol: "USDRUBF".to_string(),
            timezone_offset_hours: 3,
            tick_size: 0.01,
            model_session_start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN),
            model_session_end_time: NaiveTime::from_hms_opt(23, 49, 59).unwrap_or(NaiveTime::MIN),
            mr_min_rel_range: 0.006,
            mr_max_rel_range: 0.050,
            mr_k_short: 0.045,
            mr_take_k_short: 0.16,
            mr_stop_k_short: 0.43,
            mr_last_entry_time: NaiveTime::from_hms_opt(11, 40, 0).unwrap_or(NaiveTime::MIN),
            mr_force_exit_time: NaiveTime::from_hms_opt(11, 50, 0).unwrap_or(NaiveTime::MIN),
            bo_k: 0.45,
            bo_stop1_range: 0.51,
            bo_stop2_range: 0.35,
            bo_big_move_threshold: 0.020,
            bo_wait_hours: 2.0,
            bo_eod_exit_time: NaiveTime::from_hms_opt(23, 30, 0).unwrap_or(NaiveTime::MIN),
            commission_pct_per_side: 0.004,
            position_size_fraction: 0.9,
            initial_cash: 100000.0,
            enable_live_execution: true,
            use_fixed_live_size: true,
            live_fixed_units: 1.0,
            max_silence_bars_sec: 1200,
        }
    }

    fn test_ctx(trade_mode: TradeMode, now_ts_utc: i64) -> StrategyCtx {
        StrategyCtx {
            strategy_id: "alor_usdrubf_hybrid_v1".to_string(),
            portfolio: "7502T0U".to_string(),
            exchange: "MOEX".to_string(),
            symbol: "USDRUBF".to_string(),
            tick_size: 0.01,
            trade_mode,
            paper_execution_mode: PaperExecutionMode::LiveOnly,
            allow_live_orders: matches!(trade_mode, TradeMode::Live),
            gateway_phase: GatewayPhase::LiveReady,
            position_qty: None,
            event_ts_utc: now_ts_utc,
            now_ts_utc,
            last_bar_ts: None,
        }
    }

    fn bar(ts_utc: i64, close: f64) -> BarEvent {
        bar_with_origin(ts_utc, close, DataOrigin::Live)
    }

    fn bar_with_origin(ts_utc: i64, close: f64, origin: DataOrigin) -> BarEvent {
        BarEvent {
            symbol: "USDRUBF".to_string(),
            close_time_utc: ts_utc,
            close,
            o: close,
            h: close + 0.01,
            l: close - 0.01,
            v: 100.0,
            origin,
        }
    }

    #[test]
    fn session_vwap_uses_typical_price_weighting() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let local_dt = utc_to_local(1_775_490_000, 3);
        let bar = BarEvent {
            symbol: "USDRUBF".to_string(),
            close_time_utc: 1_775_490_000,
            close: 78.0,
            o: 78.4,
            h: 79.0,
            l: 77.0,
            v: 120.0,
            origin: DataOrigin::Live,
        };

        strategy.update_session_metrics(&bar, local_dt);

        let expected_typical = (79.0 + 77.0 + 78.0) / 3.0;
        assert!((strategy.session_vwap(bar.close) - expected_typical).abs() < 1e-9);
    }

    #[test]
    fn model_session_guard_excludes_service_bars_from_live_and_warmup_state() {
        let mut cfg = test_config();
        cfg.model_session_start_time = NaiveTime::from_hms_opt(7, 0, 0).unwrap_or(NaiveTime::MIN);
        cfg.model_session_end_time = NaiveTime::from_hms_opt(23, 49, 59).unwrap_or(NaiveTime::MIN);
        let ctx = test_ctx(TradeMode::Paper, 1_785_124_800);
        let service_bar = bar(1_785_124_200, 77.0); // 06:50 Moscow.
        let opening_bar = bar(1_785_124_800, 78.0); // 07:00 Moscow.
        let after_session_bar = bar(1_785_185_400, 79.0); // 23:50 Moscow.

        let mut live = AlorUsdrubfHybridStrategy::new(cfg.clone());
        assert!(live.on_bar(&ctx, &service_bar).is_empty());
        assert_eq!(live.day_open, None);
        assert!(live.on_bar(&ctx, &opening_bar).is_empty());
        assert_eq!(live.day_open, Some(78.0));
        assert_eq!(
            live.session_start_local.map(|value| value.time()),
            Some(NaiveTime::from_hms_opt(7, 0, 0).unwrap_or(NaiveTime::MIN))
        );
        assert!(live.on_bar(&ctx, &after_session_bar).is_empty());
        assert_eq!(live.day_open, Some(78.0));

        let mut warmup = AlorUsdrubfHybridStrategy::new(cfg);
        let processed = warmup.warmup_from_history(&ctx, &[service_bar, opening_bar]);
        assert_eq!(processed, 1);
        assert_eq!(warmup.day_open, Some(78.0));
    }

    fn position_event(ts_utc: i64, qty: f64, avg_price: f64) -> PositionEvent {
        PositionEvent {
            symbol: "USDRUBF".to_string(),
            qty,
            existing: true,
            avg_price,
            ts_utc,
        }
    }

    fn order_event(order_id: i64, status: &str, request_id: Option<Uuid>) -> OrderEvent {
        OrderEvent {
            order_id,
            request_id,
            symbol: "USDRUBF".to_string(),
            status: status.to_string(),
            ..OrderEvent::default()
        }
    }

    fn stop_order_event(
        stop_order_id: &str,
        exchange_order_id: Option<i64>,
        status: &str,
        comment: Option<&str>,
    ) -> StopOrderEvent {
        StopOrderEvent {
            stop_order_id: stop_order_id.to_string(),
            exchange_order_id,
            symbol: "USDRUBF".to_string(),
            status: status.to_string(),
            side: String::new(),
            qty: 0.0,
            filled: 0.0,
            stop_price: 0.0,
            price: 0.0,
            existing: false,
            comment: comment.map(str::to_string),
            end_time: None,
            ts_utc: 0,
        }
    }

    fn bootstrap_snapshot_with(
        position_qty: f64,
        avg_price: f64,
        order_ids: &[i64],
        stop_order_ids: &[&str],
        ts_utc: i64,
    ) -> BootstrapSnapshot {
        let mut positions_strategy = HashMap::new();
        if position_qty.abs() > f64::EPSILON {
            positions_strategy.insert(
                "USDRUBF".to_string(),
                PositionEvent {
                    symbol: "USDRUBF".to_string(),
                    qty: position_qty,
                    existing: true,
                    avg_price,
                    ts_utc,
                },
            );
        }
        let mut working_orders_strategy = HashMap::new();
        for order_id in order_ids {
            working_orders_strategy.insert(
                *order_id,
                OrderEvent {
                    order_id: *order_id,
                    symbol: "USDRUBF".to_string(),
                    status: "working".to_string(),
                    ..OrderEvent::default()
                },
            );
        }
        let mut working_stop_orders_strategy = HashMap::new();
        for stop_order_id in stop_order_ids {
            working_stop_orders_strategy.insert(
                (*stop_order_id).to_string(),
                StopOrderEvent {
                    stop_order_id: (*stop_order_id).to_string(),
                    symbol: "USDRUBF".to_string(),
                    status: "working".to_string(),
                    ..StopOrderEvent {
                        stop_order_id: String::new(),
                        exchange_order_id: None,
                        symbol: String::new(),
                        status: String::new(),
                        side: String::new(),
                        qty: 0.0,
                        filled: 0.0,
                        stop_price: 0.0,
                        price: 0.0,
                        existing: false,
                        comment: None,
                        end_time: None,
                        ts_utc: 0,
                    }
                },
            );
        }
        BootstrapSnapshot {
            positions_strategy,
            working_orders_strategy,
            working_stop_orders_strategy,
            snapshot_ts_utc: Some(ts_utc),
        }
    }

    #[test]
    fn duplicate_bar_is_ignored() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Paper, 1_000_000);
        let first = bar(999_000, 80.0);
        let duplicate = bar(999_000, 80.1);

        let _ = strategy.on_bar(&ctx, &first);
        let state_after_first = strategy.state().clone();
        let intents = strategy.on_bar(&ctx, &duplicate);

        assert!(intents.is_empty());
        assert_eq!(
            serde_json::to_string(&state_after_first).ok(),
            serde_json::to_string(strategy.state()).ok()
        );
    }

    #[test]
    fn stale_live_bars_are_suppressed_until_fresh_bar() {
        let mut cfg = test_config();
        cfg.max_silence_bars_sec = 30;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let now = 1_000_000;
        let ctx = test_ctx(TradeMode::Live, now);

        let stale_bar = bar(now - 120, 80.0);
        let fresh_bar = bar(now - 5, 80.0);

        let stale_intents = strategy.on_bar(&ctx, &stale_bar);
        assert!(stale_intents.is_empty());
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                live_ready: false,
                lifecycle_stage,
                ..
            } if lifecycle_stage == "replay_tail_suppressed"
        ));

        let _ = strategy.on_bar(&ctx, &fresh_bar);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                live_ready: true,
                ..
            }
        ));
    }

    #[test]
    fn recovered_origin_bar_is_suppressed_in_live_without_session_reset() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1775400000),
            last_processed_bar_ts: Some(1775400000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "flat".to_string(),
            current_date_local: Some("2026-04-05".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-05 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: false,
            pending_entry_reason: None,
            pending_entry_scale_at_signal: None,
            pending_entry_signal_price: None,
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: false,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        });

        let ctx = test_ctx(TradeMode::Live, 1775486460);
        let recovered_bar = bar_with_origin(1775486400, 81.0, DataOrigin::Replay);
        let intents = strategy.on_bar(&ctx, &recovered_bar);

        assert!(intents.is_empty());
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                current_date_local,
                pending_entry_owner,
                ..
            } if lifecycle_stage == "recovered_bar_suppressed"
                && current_date_local.as_deref() == Some("2026-04-05")
                && pending_entry_owner.is_none()
        ));
    }

    #[test]
    fn set_state_restores_pending_entry_for_next_bar() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let restored = StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1775490000),
            last_processed_bar_ts: Some(1775490000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "pending".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: Some("mean_rev".to_string()),
            pending_entry_side: Some("short".to_string()),
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: false,
            pending_entry_reason: Some("mr_short_signal".to_string()),
            pending_entry_scale_at_signal: Some(0.5),
            pending_entry_signal_price: Some(80.1),
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: false,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        };
        strategy.set_state(restored);

        let ctx = test_ctx(TradeMode::Paper, 1775490120);
        let intents = strategy.on_bar(&ctx, &bar(1775490060, 80.0));

        assert!(!intents.is_empty());
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                last_processed_bar_ts,
                ..
            } if *last_processed_bar_ts == Some(1775490060)
        ));
    }

    #[test]
    fn live_signal_emits_entry_without_waiting_for_an_extra_completed_bar() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.current_date_local = chrono::NaiveDate::from_ymd_opt(2026, 6, 17);
        strategy.day_open = Some(72.90);
        strategy.day_high = Some(73.20);
        strategy.day_low = Some(72.67);
        strategy.day_volume_sum = 10_000.0;
        strategy.day_vwap_num = 729_390.0;
        strategy.session_start_local =
            chrono::NaiveDate::from_ymd_opt(2026, 6, 17).and_then(|date| date.and_hms_opt(9, 0, 0));
        strategy.bootstrap_seen = true;
        strategy.runtime_state_restored = true;
        strategy.live_ready = true;

        let ctx = test_ctx(TradeMode::Live, 1_781_683_801);
        let signal_bar = BarEvent {
            symbol: "USDRUBF".to_string(),
            close_time_utc: 1_781_683_800,
            close: 72.95,
            o: 72.80,
            h: 72.99,
            l: 72.89,
            v: 100.0,
            origin: DataOrigin::Live,
        };

        let intents = strategy.on_bar(&ctx, &signal_bar);

        assert_eq!(intents.len(), 1);
        assert!(matches!(
            &intents[0],
            Intent::Market {
                side: Side::Sell,
                fill_price: Some(price),
                ..
            } if (*price - signal_bar.close).abs() < 1e-9
        ));
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                pending_entry_owner,
                entry_intent_inflight,
                ..
            } if hybrid_state == "pending"
                && pending_entry_owner.as_deref() == Some("mean_rev")
                && *entry_intent_inflight
        ));

        let next_bar = bar(1_781_684_400, 72.90);
        assert!(strategy.on_bar(&ctx, &next_bar).is_empty());
    }

    #[test]
    fn warmup_processes_history_bars_for_aggregates() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1_000_000);
        let bars = vec![bar(999_000, 80.0), bar(999_060, 80.1), bar(999_120, 80.2)];

        let processed = strategy.warmup_from_history(&ctx, &bars);

        assert_eq!(processed, 3);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                day_open,
                day_high,
                day_low,
                day_volume_sum,
                ..
            } if lifecycle_stage == "warmed_up"
                && day_open.is_some()
                && day_high.is_some()
                && day_low.is_some()
                && day_high.unwrap_or(0.0) >= day_open.unwrap_or(0.0)
                && day_low.unwrap_or(0.0) <= day_open.unwrap_or(0.0)
                && *day_volume_sum > 0.0
        ));
    }

    #[test]
    fn live_pending_entry_is_confirmed_by_position_callback() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1775490000),
            last_processed_bar_ts: Some(1775490000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "pending".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: Some("mean_rev".to_string()),
            pending_entry_side: Some("short".to_string()),
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: false,
            pending_entry_reason: Some("mr_short_signal".to_string()),
            pending_entry_scale_at_signal: Some(0.5),
            pending_entry_signal_price: Some(80.1),
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: false,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        });

        let ctx = test_ctx(TradeMode::Live, 1775490120);
        let intents = strategy.on_bar(&ctx, &bar(1775490060, 80.0));
        assert_eq!(intents.len(), 1);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                open_position_qty,
                entry_intent_inflight,
                ..
            } if hybrid_state == "pending" && *open_position_qty == 0.0 && *entry_intent_inflight
        ));

        let _ = strategy.on_position(&ctx, &position_event(1775490070, -1.0, 80.0));
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                open_position_qty,
                entry_intent_inflight,
                ..
            } if hybrid_state == "open" && *open_position_qty >= 1.0 && !*entry_intent_inflight
        ));
    }

    #[test]
    fn live_ack_rejection_clears_inflight_flags() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1775490000),
            last_processed_bar_ts: Some(1775490000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "pending".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: Some("mean_rev".to_string()),
            pending_entry_side: Some("short".to_string()),
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: true,
            pending_entry_reason: Some("mr_short_signal".to_string()),
            pending_entry_scale_at_signal: Some(0.5),
            pending_entry_signal_price: Some(80.1),
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: true,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        });

        let ctx = test_ctx(TradeMode::Live, 1775490120);
        let ack = CommandAck::rejected(Uuid::new_v4(), "reject", "mock reject");
        let _ = strategy.on_ack(&ctx, &ack);

        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                entry_intent_inflight,
                exit_intent_inflight,
                lifecycle_stage,
                ..
            } if !*entry_intent_inflight && !*exit_intent_inflight && lifecycle_stage == "entry_reject_deferred_retry"
        ));
    }

    #[test]
    fn runtime_restore_populates_pending_and_tracked_hooks() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1775490120);
        let req = Uuid::new_v4();
        let restored = RuntimeStateRestored {
            known_order_ids: vec![42, 7],
            pending_requests: vec![req],
        };

        let _ = strategy.on_runtime_state_restored(&ctx, &restored);
        assert_eq!(strategy.pending_request_ids(), vec![req]);
        assert_eq!(strategy.tracked_order_ids(), vec![7, 42]);
    }

    #[test]
    fn ack_and_order_callbacks_update_tracking_sets() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1775490120);
        let req = Uuid::new_v4();
        let restored = RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: vec![req],
        };
        let _ = strategy.on_runtime_state_restored(&ctx, &restored);

        let ack = CommandAck::confirmed(req, Some(1001));
        let _ = strategy.on_ack(&ctx, &ack);
        assert!(strategy.pending_request_ids().is_empty());
        assert_eq!(strategy.tracked_order_ids(), vec![1001]);

        let _ = strategy.on_order(&ctx, &order_event(1001, "cancelled", None));
        assert!(strategy.tracked_order_ids().is_empty());
    }

    #[test]
    fn live_mr_position_emits_tp_and_sl_brackets() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            reason: "mr_short_signal".to_string(),
            scale_at_signal: 0.5,
            signal_price: 80.1,
            stop1: None,
            stop2: None,
            target_qty: 1.0,
            partial_started_at_ms: None,
        });

        let intents = strategy.on_position(&ctx, &position_event(1_775_490_120, -1.0, 80.0));

        assert_eq!(intents.len(), 2);
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified { intent, intent_class }
                    if *intent_class == IntentClass::ProtectiveRepair
                        && matches!(
                            intent.base_intent(),
                            Intent::Place { comment, .. }
                                if comment.as_deref() == Some("AUS|sid=alor_usdrubf_hybrid_v1|o=MR|r=TP")
                        )
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent,
                Intent::Classified { intent, intent_class }
                    if *intent_class == IntentClass::ProtectiveRepair
                        && matches!(
                            intent.base_intent(),
                            Intent::CreateStopLimit { comment, .. }
                                if comment.as_deref() == Some("AUS|sid=alor_usdrubf_hybrid_v1|o=MR|r=SL")
                        )
            )
        }));
    }

    #[test]
    fn live_mr_partial_entry_waits_for_target_before_brackets() {
        let mut cfg = test_config();
        cfg.live_fixed_units = 3.0;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            reason: "mr_short_signal".to_string(),
            scale_at_signal: 0.5,
            signal_price: 80.1,
            stop1: None,
            stop2: None,
            target_qty: 3.0,
            partial_started_at_ms: None,
        });

        let partial = strategy.on_position(&ctx, &position_event(1_775_490_120, -1.0, 80.0));
        assert!(partial.is_empty());
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.open_position.is_none());

        let complete = strategy.on_position(&ctx, &position_event(1_775_490_121, -3.0, 80.0));
        assert_eq!(complete.len(), 2);
        assert!(strategy.pending_entry.is_none());
        assert_eq!(
            strategy
                .open_position
                .as_ref()
                .map(|position| position.size),
            Some(3)
        );
        assert!(complete.iter().all(|intent| match intent.base_intent() {
            Intent::Place { qty, .. } | Intent::CreateStopLimit { qty, .. } =>
                (*qty - 3.0).abs() <= f64::EPSILON,
            _ => false,
        }));
    }

    #[test]
    fn live_bo_entry_records_emitted_target_qty() {
        let mut cfg = test_config();
        cfg.live_fixed_units = 2.0;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::Breakout,
            side: PositionSide::Short,
            reason: "bo_short_signal".to_string(),
            scale_at_signal: 0.5,
            signal_price: 80.1,
            stop1: Some(79.8),
            stop2: Some(80.3),
            // Simulate a pending state written by an older runtime version.
            target_qty: 1.0,
            partial_started_at_ms: None,
        });

        let mut intents = Vec::new();
        strategy.maybe_emit_live_entry_intent(&ctx, &bar(1_775_490_120, 80.1), 80.1, &mut intents);

        assert!(matches!(
            intents.first().map(Intent::base_intent),
            Some(Intent::Market { qty, side: Side::Sell, .. }) if (*qty - 2.0).abs() <= f64::EPSILON
        ));
        assert_eq!(
            strategy
                .pending_entry
                .as_ref()
                .map(|pending| pending.target_qty),
            Some(2.0)
        );
    }

    #[test]
    fn live_bo_partial_entry_waits_for_target_before_opening_position() {
        let mut cfg = test_config();
        cfg.live_fixed_units = 2.0;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::Breakout,
            side: PositionSide::Short,
            reason: "bo_short_signal".to_string(),
            scale_at_signal: 0.5,
            signal_price: 80.1,
            stop1: Some(79.8),
            stop2: Some(80.3),
            target_qty: 2.0,
            partial_started_at_ms: None,
        });

        let partial = strategy.on_position(&ctx, &position_event(1_775_490_120, -1.0, 80.0));
        assert!(partial.is_empty());
        assert!(strategy.pending_entry.is_some());
        assert!(strategy.open_position.is_none());

        let complete = strategy.on_position(&ctx, &position_event(1_775_490_121, -2.0, 80.0));
        assert!(complete.is_empty());
        assert!(strategy.pending_entry.is_none());
        assert_eq!(
            strategy
                .open_position
                .as_ref()
                .map(|position| (position.owner, position.size)),
            Some((Owner::Breakout, 2))
        );
    }

    #[test]
    fn live_bo_partial_entry_timeout_cancels_and_flattens_residual() {
        let mut cfg = test_config();
        cfg.live_fixed_units = 2.0;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        strategy.pending_entry = Some(PendingEntry {
            owner: Owner::Breakout,
            side: PositionSide::Short,
            reason: "bo_short_signal".to_string(),
            scale_at_signal: 0.5,
            signal_price: 80.1,
            stop1: Some(79.8),
            stop2: Some(80.3),
            target_qty: 2.0,
            partial_started_at_ms: Some(10_000),
        });
        strategy.tracked_order_ids.insert(77);
        let mut ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        ctx.position_qty = Some(-1.0);

        let intents = strategy.on_timer(&ctx, 13_001);

        assert!(intents.iter().any(|intent| {
            matches!(intent.base_intent(), Intent::Cancel { order_id } if *order_id == 77)
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::Market { qty, side: Side::Buy, .. } if (*qty - 1.0).abs() <= f64::EPSILON
            )
        }));
        assert!(strategy.pending_entry.is_none());
        assert_eq!(strategy.lifecycle_stage, "partial_entry_timeout_flatten");
    }

    #[test]
    fn live_mr_take_is_serviced_by_bracket_not_market_exit() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 1,
            stop_price: Some(80.2),
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.owner_confirmed_by_live_event = true;
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let mut live_bar = bar(1_775_490_120, 79.95);
        live_bar.l = 79.88;
        live_bar.h = 80.0;

        let intents = strategy.on_bar(&ctx, &live_bar);

        assert!(intents.iter().all(|intent| {
            !matches!(intent.base_intent(), Intent::Market { comment, .. } if comment.as_deref() == Some("USDRUBF|exit|mr_take"))
        }));
    }

    #[test]
    fn mr_tp_fill_waits_for_broker_flat_before_canceling_stop() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 1,
            stop_price: Some(80.2),
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        let mut tp_fill = order_event(111, "filled", None);
        tp_fill.comment = Some("AUS|sid=alor_usdrubf_hybrid_v1|o=MR|r=TP".to_string());

        let intents = strategy.on_order(&test_ctx(TradeMode::Live, 1_775_490_120), &tp_fill);

        assert!(intents.is_empty());
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-1"));
        assert!(strategy.exit_intent_inflight);
        assert_eq!(
            strategy.lifecycle_stage,
            "mr_tp_filled_awaiting_broker_flat"
        );
    }

    #[test]
    fn filled_mr_tp_cannot_be_repaired_before_broker_flat_reconciliation() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 1,
            stop_price: Some(80.2),
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.tp_order_id = Some(111);
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let mut tp_fill = order_event(111, "filled", None);
        tp_fill.comment = Some("AUS|sid=alor_usdrubf_hybrid_v1|o=MR|r=TP".to_string());

        assert!(strategy.on_order(&ctx, &tp_fill).is_empty());

        let mut repair_intents = Vec::new();
        strategy.maybe_emit_live_mr_brackets(&ctx, 1_775_490_121, &mut repair_intents);
        assert!(repair_intents.is_empty());

        let repeated_open = strategy.on_position(&ctx, &position_event(1_775_490_122, -1.0, 80.0));
        assert!(repeated_open.is_empty());
        assert!(strategy.exit_intent_inflight);
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-1"));

        let flat_cleanup = strategy.on_position(&ctx, &position_event(1_775_490_123, 0.0, 0.0));
        assert!(flat_cleanup.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::DeleteStopLimit { order_id, .. } if order_id == "sl-1"
            )
        }));
        assert!(!strategy.exit_intent_inflight);
        assert!(strategy.open_position.is_none());
    }

    #[test]
    fn partial_mr_tp_fill_waits_for_broker_flat_without_residual_churn() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 2,
            stop_price: Some(80.2),
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.tp_order_id = Some(111);
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);

        let partial = strategy.on_position(&ctx, &position_event(1_775_490_121, -1.0, 80.0));

        assert!(partial.is_empty());
        assert!(strategy.exit_intent_inflight);
        assert_eq!(
            strategy.lifecycle_stage,
            "mr_bracket_partial_awaiting_broker_flat"
        );
        assert_eq!(strategy.tp_order_id, Some(111));
        assert_eq!(strategy.sl_stop_order_id.as_deref(), Some("sl-1"));

        let flat_cleanup = strategy.on_position(&ctx, &position_event(1_775_490_122, 0.0, 0.0));

        assert!(flat_cleanup.iter().all(|intent| {
            !matches!(
                intent.base_intent(),
                Intent::Market {
                    comment,
                    ..
                } if comment.as_deref() == Some("USDRUBF|exit|broker_residual")
            )
        }));
        assert!(flat_cleanup.iter().any(|intent| {
            matches!(intent.base_intent(), Intent::Cancel { order_id } if *order_id == 111)
        }));
        assert!(flat_cleanup.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::DeleteStopLimit { order_id, .. } if order_id == "sl-1"
            )
        }));
        assert!(!strategy.exit_intent_inflight);
        assert!(strategy.open_position.is_none());
    }

    #[test]
    fn filled_mr_tp_reconcile_timeout_emits_single_residual_flatten() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 2,
            stop_price: Some(80.2),
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.sl_stop_order_id = Some("sl-1".to_string());
        strategy.exit_intent_inflight = true;
        strategy.bracket_terminal_reconcile_started_ms = Some(10_000);
        let mut ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        ctx.position_qty = Some(-1.0);

        let intents = strategy.on_timer(&ctx, 13_001);

        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::DeleteStopLimit { order_id, .. } if order_id == "sl-1"
            )
        }));
        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::Market { qty, side: Side::Buy, .. } if (*qty - 1.0).abs() <= f64::EPSILON
            )
        }));
        assert!(strategy.exit_intent_inflight);
        assert_eq!(
            strategy.lifecycle_stage,
            "bracket_terminal_reconcile_timeout"
        );

        let repeated = strategy.on_timer(&ctx, 16_500);
        assert!(repeated.is_empty());
    }

    #[test]
    fn unknown_tp_outcome_is_not_retried_on_next_bar() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 1,
            stop_price: None,
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.pending_tp_bar_ts_utc = Some(1_775_490_060);
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let mut intents = Vec::new();

        strategy.maybe_emit_live_mr_brackets(&ctx, 1_775_490_120, &mut intents);

        assert!(intents.is_empty());
    }

    #[test]
    fn unexpected_live_residual_emits_emergency_market_exit() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let mut residual = position_event(1_775_490_120, 1.0, 80.0);
        residual.existing = false;

        let intents = strategy.on_position(&ctx, &residual);

        assert!(intents.iter().any(|intent| {
            matches!(
                intent.base_intent(),
                Intent::Market { qty, side: alor_protocol::Side::Sell, .. } if (*qty - 1.0).abs() <= f64::EPSILON
            )
        }));
        assert!(strategy.exit_intent_inflight);
    }

    #[test]
    fn mr_sl_trigger_cancels_tp_cleanup() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.open_position = Some(OpenPosition {
            owner: Owner::MeanRev,
            side: PositionSide::Short,
            entry_ts: utc_to_local(1_775_490_000, 3),
            entry_price: 80.0,
            size: 1,
            stop_price: Some(80.2),
            take_price: Some(79.9),
            stop1: None,
            stop2: None,
        });
        strategy.tp_order_id = Some(111);

        let intents = strategy.on_stop_order(
            &test_ctx(TradeMode::Live, 1_775_490_120),
            &stop_order_event(
                "sl-1",
                Some(222),
                "triggered",
                Some("AUS|sid=alor_usdrubf_hybrid_v1|o=MR|r=SL"),
            ),
        );

        assert!(intents.iter().any(|intent| {
            matches!(intent.base_intent(), Intent::Cancel { order_id } if *order_id == 111)
        }));
        assert!(strategy.exit_intent_inflight);
        assert_eq!(
            strategy.lifecycle_stage,
            "mr_sl_filled_awaiting_broker_flat"
        );
    }

    #[test]
    fn intent_comment_and_exit_risk_hooks_are_strategy_owned() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1775490120);

        let entry_tag = strategy
            .intent_comment_tag(&ctx, 1775490120, IntentClass::Entry)
            .unwrap_or_default();
        assert!(entry_tag.contains("AUS|sid=alor_usdrubf_hybrid_v1"));
        assert!(entry_tag.contains("r=ENTRY"));

        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1775490000),
            last_processed_bar_ts: Some(1775490000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "open".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_request_ids: Vec::new(),
            tracked_order_ids: vec![77],
            entry_intent_inflight: false,
            pending_entry_reason: None,
            pending_entry_scale_at_signal: None,
            pending_entry_signal_price: None,
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: Some("day_breakout_waitfix".to_string()),
            open_position_side: Some("long".to_string()),
            exit_intent_inflight: true,
            open_position_qty: 1.0,
            open_position_entry_ts: Some("2026-04-06 10:00:00".to_string()),
            open_position_entry_price: Some(80.0),
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: Some(79.7),
            open_position_stop2: Some(79.5),
        });

        let risk = strategy.exit_risk_status(true);
        assert_eq!(risk.phase_override.as_deref(), Some("ExitIntentInflight"));
        assert!(risk.exit_recovery_active);
        assert!(risk.open_risk_position_unflattened);
    }

    #[test]
    fn runtime_guard_blocks_live_path_when_live_execution_disabled() {
        let mut cfg = test_config();
        cfg.enable_live_execution = false;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let ctx = test_ctx(TradeMode::Live, 1_000_000);
        let intents = strategy.on_bar(&ctx, &bar(999_990, 80.0));

        assert!(intents.is_empty());
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                live_ready,
                last_processed_bar_ts,
                ..
            } if !*live_ready && *last_processed_bar_ts == Some(999_990)
        ));
    }

    #[test]
    fn broker_truth_reconciliation_is_stable_with_out_of_order_ack() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let req = Uuid::new_v4();
        let restored = RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: vec![req],
        };
        let _ = strategy.on_runtime_state_restored(&ctx, &restored);
        let _ = strategy.on_position(&ctx, &position_event(1_775_490_130, 1.0, 80.2));
        let ack = CommandAck::rejected(req, "reject", "late reject");
        let _ = strategy.on_ack(&ctx, &ack);

        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                open_position_qty,
                ..
            } if hybrid_state == "open" && *open_position_qty >= 1.0
        ));
    }

    #[test]
    fn dirty_start_suppresses_recovery_tail_then_allows_fresh_live_bar() {
        let mut cfg = test_config();
        cfg.max_silence_bars_sec = 60;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let now = 1_775_490_120;
        let ctx = test_ctx(TradeMode::Live, now);
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(now - 300),
            last_processed_bar_ts: Some(now - 300),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: false,
            hybrid_state: "pending".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: Some("mean_rev".to_string()),
            pending_entry_side: Some("short".to_string()),
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: false,
            pending_entry_reason: Some("mr_short_signal".to_string()),
            pending_entry_scale_at_signal: Some(0.5),
            pending_entry_signal_price: Some(80.1),
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: false,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        });

        let stale_tail = bar_with_origin(now - 120, 80.0, DataOrigin::Replay);
        let stale_intents = strategy.on_bar(&ctx, &stale_tail);
        assert!(stale_intents.is_empty());
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage, ..
            } if lifecycle_stage == "replay_tail_suppressed"
        ));

        let fresh_live = bar_with_origin(now - 5, 80.0, DataOrigin::Live);
        let fresh_intents = strategy.on_bar(&ctx, &fresh_live);
        assert_eq!(fresh_intents.len(), 1);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                live_ready,
                hybrid_state,
                entry_intent_inflight,
                ..
            } if *live_ready && hybrid_state == "pending" && *entry_intent_inflight
        ));
    }

    #[test]
    fn fresh_recovered_origin_bar_does_not_clear_live_ready() {
        let mut cfg = test_config();
        cfg.max_silence_bars_sec = 60;
        let mut strategy = AlorUsdrubfHybridStrategy::new(cfg);
        let now = 1_775_490_120;
        let ctx = test_ctx(TradeMode::Live, now);
        let recovered_fresh = bar_with_origin(now - 3, 80.0, DataOrigin::Replay);

        let intents = strategy.on_bar(&ctx, &recovered_fresh);
        assert!(intents.is_empty());
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                live_ready,
                lifecycle_stage,
                ..
            } if !*live_ready && lifecycle_stage == "awaiting_fresh_live_origin_bar"
        ));
    }

    #[test]
    fn bootstrap_adoption_with_non_flat_snapshot_prevents_blind_entry() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let snapshot = bootstrap_snapshot_with(2.0, 80.25, &[77], &["stp-1"], 1_775_490_110);

        let _ = strategy.on_bootstrap_snapshot(&ctx, &snapshot);

        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                open_position_qty,
                pending_entry_owner,
                tracked_order_ids,
                live_ready,
                ..
            } if hybrid_state == "open"
                && *open_position_qty >= 2.0
                && pending_entry_owner.is_none()
                && tracked_order_ids.contains(&77)
                && !*live_ready
        ));

        let intents = strategy.on_bar(
            &ctx,
            &bar_with_origin(1_775_490_119, 80.3, DataOrigin::Live),
        );
        assert!(intents.is_empty());
    }

    #[test]
    fn restart_with_non_flat_snapshot_keeps_owner_conservative_until_live_confirmation() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        let ctx = test_ctx(TradeMode::Live, 1_775_490_240);
        let snapshot = bootstrap_snapshot_with(1.0, 80.2, &[], &[], 1_775_490_110);
        let _ = strategy.on_bootstrap_snapshot(&ctx, &snapshot);

        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                hybrid_state,
                open_position_qty,
                ..
            } if lifecycle_stage == "bootstrap_non_flat_owner_unconfirmed"
                && hybrid_state == "open"
                && *open_position_qty == 1.0
        ));

        // No blind re-entry should happen while strategy is already non-flat.
        let intents = strategy.on_bar(
            &ctx,
            &bar_with_origin(1_775_490_180, 80.25, DataOrigin::Live),
        );
        assert!(intents.is_empty());

        // First live position confirmation lifts conservative-owner mode.
        let _ = strategy.on_position(&ctx, &position_event(1_775_490_181, 1.0, 80.2));
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid { lifecycle_stage, .. }
                if lifecycle_stage == "broker_position_open"
        ));
    }

    #[test]
    fn terminal_reject_after_entry_intent_clears_inflight_and_defers_retry() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1_775_490_000),
            last_processed_bar_ts: Some(1_775_489_940),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "pending".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: Some("mean_rev".to_string()),
            pending_entry_side: Some("short".to_string()),
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: true,
            pending_entry_reason: Some("mr_short_signal".to_string()),
            pending_entry_scale_at_signal: Some(0.5),
            pending_entry_signal_price: Some(80.1),
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: false,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        });
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let req = Uuid::new_v4();
        let ack = CommandAck::rejected(req, "reject", "entry reject");
        let _ = strategy.on_ack(&ctx, &ack);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                entry_intent_inflight,
                pending_entry_owner,
                ..
            } if lifecycle_stage == "entry_reject_deferred_retry"
                && !*entry_intent_inflight
                && pending_entry_owner.as_deref() == Some("mean_rev")
        ));

        let same_bar_intents = strategy.on_bar(
            &ctx,
            &bar_with_origin(1_775_490_000, 80.1, DataOrigin::Live),
        );
        assert!(same_bar_intents.is_empty());
    }

    #[test]
    fn terminal_reject_after_exit_intent_preserves_open_risk_state() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1_775_490_000),
            last_processed_bar_ts: Some(1_775_489_940),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "open".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_request_ids: Vec::new(),
            tracked_order_ids: vec![77],
            entry_intent_inflight: false,
            pending_entry_reason: None,
            pending_entry_scale_at_signal: None,
            pending_entry_signal_price: None,
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: Some("day_breakout_waitfix".to_string()),
            open_position_side: Some("long".to_string()),
            exit_intent_inflight: true,
            open_position_qty: 1.0,
            open_position_entry_ts: Some("2026-04-06 10:00:00".to_string()),
            open_position_entry_price: Some(80.0),
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: Some(79.7),
            open_position_stop2: Some(79.5),
        });
        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let req = Uuid::new_v4();
        let ack = CommandAck::rejected(req, "reject", "exit reject");
        let _ = strategy.on_ack(&ctx, &ack);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage,
                exit_intent_inflight,
                open_position_qty,
                ..
            } if lifecycle_stage == "exit_reject_deferred_retry"
                && !*exit_intent_inflight
                && *open_position_qty == 1.0
        ));
        let risk = strategy.exit_risk_status(true);
        assert_eq!(
            risk.phase_override.as_deref(),
            Some("ExitRejectDeferredRetry")
        );
        assert!(risk.exit_recovery_active);
        assert!(risk.open_risk_position_unflattened);
    }

    #[test]
    fn set_state_restores_open_position_and_allows_exit_evaluation() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1_775_490_000),
            last_processed_bar_ts: Some(1_775_490_000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "open".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: None,
            pending_entry_side: None,
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: false,
            pending_entry_reason: None,
            pending_entry_scale_at_signal: None,
            pending_entry_signal_price: None,
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: Some("mean_rev".to_string()),
            open_position_side: Some("short".to_string()),
            exit_intent_inflight: false,
            open_position_qty: 1.0,
            open_position_entry_ts: Some("2026-04-06 11:35:00".to_string()),
            open_position_entry_price: Some(80.0),
            open_position_stop_price: Some(80.1),
            open_position_take_price: Some(79.9),
            open_position_stop1: None,
            open_position_stop2: None,
        });

        let ctx = test_ctx(TradeMode::Paper, 1_775_490_720);
        let intents = strategy.on_bar(&ctx, &bar(1_775_490_660, 80.0));
        assert_eq!(intents.len(), 1);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                open_position_qty,
                ..
            } if hybrid_state == "flat" && *open_position_qty == 0.0
        ));
    }

    #[test]
    fn warmup_keeps_trading_state_untouched_when_pending_or_open_exists() {
        let mut strategy = AlorUsdrubfHybridStrategy::new(test_config());
        strategy.set_state(StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: "runtime_state_restored".to_string(),
            last_bar_ts: Some(1_775_490_000),
            last_processed_bar_ts: Some(1_775_490_000),
            bootstrap_seen: true,
            runtime_state_restored: true,
            live_ready: true,
            hybrid_state: "pending".to_string(),
            current_date_local: Some("2026-04-06".to_string()),
            day_open: Some(80.0),
            day_high: Some(80.3),
            day_low: Some(79.8),
            day_volume_sum: 1000.0,
            day_vwap_num: 80_000.0,
            session_start_local: Some("2026-04-06 09:00:00".to_string()),
            bo_was_long_today: false,
            bo_was_short_today: false,
            cash: 100000.0,
            pending_entry_owner: Some("mean_rev".to_string()),
            pending_entry_side: Some("short".to_string()),
            pending_request_ids: Vec::new(),
            tracked_order_ids: Vec::new(),
            entry_intent_inflight: false,
            pending_entry_reason: Some("mr_short_signal".to_string()),
            pending_entry_scale_at_signal: Some(0.5),
            pending_entry_signal_price: Some(80.1),
            pending_entry_stop1: None,
            pending_entry_stop2: None,
            open_position_owner: None,
            open_position_side: None,
            exit_intent_inflight: false,
            open_position_qty: 0.0,
            open_position_entry_ts: None,
            open_position_entry_price: None,
            open_position_stop_price: None,
            open_position_take_price: None,
            open_position_stop1: None,
            open_position_stop2: None,
        });

        let ctx = test_ctx(TradeMode::Live, 1_775_490_120);
        let processed = strategy
            .warmup_from_history(&ctx, &[bar(1_775_490_060, 80.0), bar(1_775_490_120, 80.1)]);
        assert_eq!(processed, 0);
        assert!(matches!(
            strategy.state(),
            StrategyState::AlorUsdrubfHybrid {
                hybrid_state,
                pending_entry_owner,
                open_position_qty,
                ..
            } if hybrid_state == "pending"
                && pending_entry_owner.as_deref() == Some("mean_rev")
                && *open_position_qty == 0.0
        ));
    }
}
