use std::collections::HashSet;

use alor_protocol::{AckStatus, CommandAck, IntentClass, Side as OrderSide, StopLimitCondition};
use chrono::{
    Datelike, Duration as ChronoDuration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Utc, Weekday,
};
use tracing::{debug, info};
use uuid::Uuid;

use crate::state::StrategyState;
use crate::strategies::hybrid_intraday::{
    Action, EntryStyle, HybridOrchestrator, HybridOrchestratorConfig, IntradayBreakoutConfig,
    IntradayBreakoutEngine, MeanReversionConfig, MeanReversionEngine, Owner, Side,
};
use crate::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle;
use crate::{
    BarEvent, DataOrigin, Intent, OrderEvent, PositionEvent, StopOrderEvent, Strategy, StrategyCtx,
};

#[derive(Debug, Clone)]
pub struct HybridIntradayRuntimeConfig {
    pub symbol: String,
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
    pub mr_config: MeanReversionConfig,
    pub breakout_config: IntradayBreakoutConfig,
    pub orchestrator_config: HybridOrchestratorConfig,
}

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    owner: Owner,
    side: Side,
    cycle_id: [u8; 10],
    entry_style: EntryStyle,
    stop_price: Option<f64>,
    take_price: Option<f64>,
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
    state: StrategyState,
    last_processed_bar_ts: Option<i64>,
    last_position_qty: f64,
    current_owner: Option<Owner>,
    current_side: Option<Side>,
    pending_entry: Option<PendingEntry>,
    pending_entry_request_id: Option<Uuid>,
    pending_entry_created_ts_utc: Option<i64>,
    pending_exit_request_id: Option<Uuid>,
    pending_exit_created_ts_utc: Option<i64>,
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
    active_cycle_id: Option<[u8; 10]>,
    safe_mode_close_only: bool,
    safe_mode_reason: Option<String>,
    next_cycle_seq: u32,
    last_bar_close: Option<f64>,
    last_day_local: Option<NaiveDate>,
    current_day_high: Option<f64>,
    current_day_low: Option<f64>,
    prev_day_range: Option<f64>,
    entry_ready: bool,
    working_orders: HashSet<i64>,
    working_stop_orders: HashSet<String>,
    startup_live_replay_boundary_ts_utc: Option<i64>,
    startup_replay_suppressed_bars: u32,
}

impl HybridIntradayRuntimeStrategy {
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
        Self {
            config,
            orchestrator,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
            last_position_qty: 0.0,
            current_owner: None,
            current_side: None,
            pending_entry: None,
            pending_entry_request_id: None,
            pending_entry_created_ts_utc: None,
            pending_exit_request_id: None,
            pending_exit_created_ts_utc: None,
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
            active_cycle_id: None,
            safe_mode_close_only: false,
            safe_mode_reason: None,
            next_cycle_seq: 0,
            last_bar_close: None,
            last_day_local: None,
            current_day_high: None,
            current_day_low: None,
            prev_day_range: None,
            entry_ready: false,
            working_orders: HashSet::new(),
            working_stop_orders: HashSet::new(),
            startup_live_replay_boundary_ts_utc: None,
            startup_replay_suppressed_bars: 0,
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
            .unwrap_or_else(|| {
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

    fn sync_state(&mut self) {
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
            pending_exit_request_id: self.pending_exit_request_id,
            pending_exit_created_ts_utc: self.pending_exit_created_ts_utc,
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
            last_day_local: self.last_day_local.map(Self::format_local_day),
            current_day_high: self.current_day_high,
            current_day_low: self.current_day_low,
            prev_day_range: self.prev_day_range,
        };
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
                info!(
                    target: "strategy_runtime::hybrid_intraday_runtime",
                    prev_day = %Self::format_local_day(prev_day),
                    next_day = %Self::format_local_day(day),
                    prev_day_high = self.current_day_high,
                    prev_day_low = self.current_day_low,
                    prev_day_range = computed_prev_day_range,
                    "hybrid day rollover: recalculated day features"
                );
                self.last_day_local = Some(day);
                self.current_day_high = Some(high);
                self.current_day_low = Some(low);
            }
        }
        self.entry_ready = self.prev_day_range.is_some();
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
                        entry_style: entry.entry_style,
                        stop_price: entry.stop_price,
                        take_price: entry.take_price,
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
            Action::SubmitExit { owner, .. } => {
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
        self.pending_exit_request_id = None;
        self.pending_exit_created_ts_utc = None;
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
mod tests {
    use super::*;
    use crate::{StrategyCtx, TradeMode};

    fn test_config() -> HybridIntradayRuntimeConfig {
        HybridIntradayRuntimeConfig {
            symbol: "IMOEXF".to_string(),
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
            mr_config: MeanReversionConfig::default(),
            breakout_config: IntradayBreakoutConfig::default(),
            orchestrator_config: HybridOrchestratorConfig::default(),
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
        BarEvent {
            symbol: "IMOEXF".to_string(),
            close_time_utc: ts_utc,
            close,
            o: close,
            h: close,
            l: close,
            v: 1.0,
            origin,
        }
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
        strategy.prev_day_range = Some(2.0);
        strategy.last_bar_close = Some(102.0);
        strategy.last_day_local = chrono::NaiveDate::from_ymd_opt(2026, 1, 4);

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
        strategy.sync_state();
        let mut restored = strategy.state().clone();
        if let StrategyState::HybridIntradayRuntime {
            entry_ready,
            prev_day_range,
            ..
        } = &mut restored
        {
            *entry_ready = true;
            *prev_day_range = None;
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
            pending_exit_request_id: None,
            pending_exit_created_ts_utc: None,
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
            last_day_local: Some("2026-03-08".to_string()),
            current_day_high: Some(102.0),
            current_day_low: Some(100.5),
            prev_day_range: Some(12.5),
        };
        strategy.set_state(restored.clone());
        assert!(strategy.entry_ready);
        assert_eq!(strategy.last_bar_close, Some(101.5));
        assert_eq!(strategy.current_day_high, Some(102.0));
        assert_eq!(strategy.current_day_low, Some(100.5));
        assert_eq!(strategy.prev_day_range, Some(12.5));
        assert_eq!(
            strategy.last_day_local,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap())
        );

        if let StrategyState::HybridIntradayRuntime {
            last_day_local,
            prev_day_range,
            ..
        } = &mut restored
        {
            *last_day_local = Some("bad-date".to_string());
            *prev_day_range = Some(11.0);
        }
        strategy.set_state(restored);
        assert_eq!(strategy.last_day_local, None);
        assert_eq!(strategy.prev_day_range, Some(11.0));
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
        self.update_day_aggregates(dt_local, bar.h, bar.l);

        let close_prev = self.last_bar_close.unwrap_or(bar.close);
        let day_range_prev = self.prev_day_range.unwrap_or(0.0);
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
        let actions = self.orchestrator.on_bar(self.bar_input(
            dt_local,
            bar,
            close_prev,
            day_range_prev,
            has_open_position,
        ));
        self.last_processed_bar_ts = Some(bar.close_time_utc);
        self.last_bar_close = Some(bar.close);
        self.clear_stale_pending_tail(bar.close_time_utc, ctx.position_qty.unwrap_or(0.0));
        let can_emit = self.can_emit_now(ctx, bar.origin == DataOrigin::Live);
        let can_execute = self.can_execute_now(ctx, bar.origin == DataOrigin::Live);
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
        let mut intents = self.maybe_emit_repair_intents(ctx, bar.close_time_utc);
        intents.extend(actions.into_iter().flat_map(|action| {
            self.map_action_to_intents(ctx, bar.close_time_utc, can_emit, can_execute, action)
        }));
        self.sync_state();
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        if Some(ack.request_id) == self.pending_entry_request_id {
            if matches!(
                ack.status,
                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
            ) {
                self.orchestrator.on_order_rejected("entry");
                self.pending_entry = None;
                self.pending_entry_request_id = None;
                self.pending_entry_created_ts_utc = None;
                self.active_cycle_id = None;
                self.tp_order_id = None;
                self.sl_stop_order_id = None;
                self.sl_exchange_order_id = None;
                self.enter_safe_mode("entry_rejected");
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
        self.ensure_active_cycle_from_comment(ord.comment.as_deref());
        let mut intents = Vec::new();
        let status = ord.status.to_ascii_lowercase();
        let tag = Self::parse_hybrid_tag(ord.comment.as_deref());
        if tag.as_ref().and_then(|v| v.role) == Some(TagRole::Tp) {
            self.pending_tp_request_id = None;
            self.pending_tp_created_ts_utc = None;
            if ord.order_id > 0 {
                self.tp_order_id = Some(ord.order_id);
            }
            if status == "filled" {
                if let Some(stop_order_id) = self.sl_stop_order_id.take() {
                    intents.push(
                        Intent::DeleteStopLimit {
                            order_id: stop_order_id,
                            side: self.current_side.map(Self::stop_side_for_entry_side),
                            check_duplicates: Some(true),
                        }
                        .with_class(IntentClass::CancelCleanup),
                    );
                }
                self.sl_exchange_order_id = None;
            }
            if matches!(
                status.as_str(),
                "filled" | "canceled" | "cancelled" | "expired" | "rejected"
            ) {
                self.tp_order_id = None;
            }
        }
        if matches!(
            status.as_str(),
            "filled" | "canceled" | "cancelled" | "expired" | "rejected"
        ) {
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
        self.ensure_active_cycle_from_comment(ord.comment.as_deref());
        let mut intents = Vec::new();
        let status = ord.status.to_ascii_lowercase();
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
            self.working_stop_orders.remove(&ord.stop_order_id);
        } else if !ord.stop_order_id.trim().is_empty() {
            self.working_stop_orders.insert(ord.stop_order_id.clone());
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
        if prev.abs() <= f64::EPSILON && cur.abs() > f64::EPSILON {
            let filled = self.pending_entry.take();
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
            }
        } else if prev.abs() > f64::EPSILON && cur.abs() <= f64::EPSILON {
            let closing_side = self.current_side;
            let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
            self.orchestrator.on_order_filled("exit", owner, None);
            self.current_owner = None;
            self.current_side = None;
            self.pending_entry = None;
            self.pending_entry_request_id = None;
            self.pending_entry_created_ts_utc = None;
            self.pending_exit_request_id = None;
            self.pending_exit_created_ts_utc = None;
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
            pending_exit_request_id,
            pending_exit_created_ts_utc,
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
            last_day_local,
            current_day_high,
            current_day_low,
            prev_day_range,
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
                    entry_style: EntryStyle::Market,
                    stop_price: None,
                    take_price: None,
                }),
                _ => None,
            };
            self.pending_entry_request_id = *pending_entry_request_id;
            self.pending_entry_created_ts_utc = *pending_entry_created_ts_utc;
            self.pending_exit_request_id = *pending_exit_request_id;
            self.pending_exit_created_ts_utc = *pending_exit_created_ts_utc;
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
            self.last_day_local = last_day_local.as_deref().and_then(Self::parse_local_day);
            self.current_day_high = *current_day_high;
            self.current_day_low = *current_day_low;
            self.prev_day_range = *prev_day_range;
            self.entry_ready = *entry_ready && self.prev_day_range.is_some();
            if *entry_ready && self.prev_day_range.is_none() {
                needs_resync = true;
            }
        }
        self.state = state;
        if needs_resync {
            self.sync_state();
        }
    }
}
