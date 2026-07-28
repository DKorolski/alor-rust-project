use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Timelike, Utc};

use alor_protocol::{AckStatus, CommandAck, IntentClass, Side};
use tracing::{info, warn};

use crate::live_guard::GatewayPhase;
use crate::state::{SessionGapLivePhase, StrategyState};
use crate::strategy_host::{BarEvent, Intent, PositionEvent, Strategy, StrategyCtx};
use crate::TradeMode;

#[derive(Debug, Clone)]
pub struct SessionGapStandaloneConfig {
    pub symbol: String,
    pub timezone_offset_hours: i32,
    pub signal_minute: u32,
    pub k_long: f64,
    pub k_short: f64,
    pub wait_hours: i64,
    pub k_tp_long: f64,
    pub k_sl_long: f64,
    pub k_tp_short: f64,
    pub k_sl_short: f64,
    pub long_ex_pct: f64,
    pub short_ex_pct: f64,
    pub max_entry_hour: u32,
    pub close_hour: u32,
    pub close_minute: u32,
    pub session_gap_min: f64,
    pub exit_offset_min: i64,
    pub work_weekends: bool,
    pub cash_factor: f64,
    pub start_cash: f64,
    pub entry_ack_timeout_ms: u64,
    pub entry_fill_timeout_ms: u64,
    pub exit_ack_timeout_ms: u64,
    pub exit_fill_timeout_ms: u64,
    pub place_offset_ticks: i64,
    pub tick_size: f64,
}

impl Default for SessionGapStandaloneConfig {
    fn default() -> Self {
        Self {
            symbol: "USDRUBF".to_string(),
            timezone_offset_hours: 3,
            signal_minute: 59,
            k_long: 0.5,
            k_short: 0.46,
            wait_hours: 3,
            k_tp_long: 0.28,
            k_sl_long: 0.68,
            k_tp_short: 0.28,
            k_sl_short: 0.65,
            long_ex_pct: 2.2,
            short_ex_pct: 2.2,
            max_entry_hour: 19,
            close_hour: 23,
            close_minute: 49,
            session_gap_min: 60.0,
            exit_offset_min: 20,
            work_weekends: false,
            cash_factor: 0.9,
            start_cash: 30_000.0,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
            place_offset_ticks: 0,
            tick_size: 0.01,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    direction: Direction,
    size: i64,
}

#[derive(Debug, Clone)]
struct Position {
    direction: Direction,
    size: i64,
    tp: f64,
    sl: f64,
}

#[derive(Debug, Clone)]
struct SessionGapTestHookConfig {
    session_date: String,
    direction: Direction,
    auto_flatten_next_bar: bool,
}

#[derive(Debug, Clone, Default)]
struct SessionGapTestHookState {
    config: Option<SessionGapTestHookConfig>,
    entry_dispatched: bool,
    exit_dispatched: bool,
}

impl SessionGapTestHookState {
    fn from_env() -> Self {
        if !runtime_test_hooks_enabled() {
            return Self::default();
        }
        let Some(session_date) = std::env::var("SESSION_GAP_TEST_FORCE_SESSION_DATE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            return Self::default();
        };
        let direction = match std::env::var("SESSION_GAP_TEST_FORCE_SIDE")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("sell") | Some("short") => Direction::Short,
            _ => Direction::Long,
        };
        let auto_flatten_next_bar = parse_bool_env("SESSION_GAP_TEST_AUTO_FLATTEN", true);
        Self {
            config: Some(SessionGapTestHookConfig {
                session_date,
                direction,
                auto_flatten_next_bar,
            }),
            entry_dispatched: false,
            exit_dispatched: false,
        }
    }

    fn should_force_entry(&self, session_date: &str) -> Option<Direction> {
        let config = self.config.as_ref()?;
        if self.entry_dispatched || config.session_date != session_date {
            return None;
        }
        Some(config.direction)
    }

    fn should_force_exit(&self, session_date: &str, opened_ts: i64, bar_ts: i64) -> bool {
        let Some(config) = &self.config else {
            return false;
        };
        config.auto_flatten_next_bar
            && self.entry_dispatched
            && !self.exit_dispatched
            && config.session_date == session_date
            && bar_ts > opened_ts
    }

    fn mark_entry_dispatched(&mut self) {
        self.entry_dispatched = true;
    }

    fn mark_exit_dispatched(&mut self) {
        self.exit_dispatched = true;
    }
}

fn parse_bool_env(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn runtime_test_hooks_enabled() -> bool {
    parse_bool_env("RUNTIME_ENABLE_TEST_HOOKS", false)
}

#[derive(Debug)]
pub struct SessionGapStandaloneStrategy {
    pub config: SessionGapStandaloneConfig,
    pub state: StrategyState,
    last_processed_bar_ts: Option<i64>,
    cash: f64,
    last_dt: Option<DateTime<FixedOffset>>,
    session_start_dt: Option<DateTime<FixedOffset>>,
    session_end_dt: Option<DateTime<FixedOffset>>,
    session_high: Option<f64>,
    session_low: Option<f64>,
    session_close: Option<f64>,
    yesterday_close: Option<f64>,
    yesterday_range: Option<f64>,
    pre_prev_close: Option<f64>,
    first_min_high: Option<f64>,
    first_min_low: Option<f64>,
    first_hour_price: Option<f64>,
    phase_last_change_ts_utc: Option<i64>,
    traded_session: bool,
    pending_entry: Option<PendingEntry>,
    position: Option<Position>,
    last_warmup_log: Option<bool>,
    test_hook: SessionGapTestHookState,
}

impl SessionGapStandaloneStrategy {
    const MIN_ENTRY_RECOVERY_VERIFICATION_MS: u64 = 5_000;
    const EXIT_RECOVERY_MAX_RETRIES: u32 = 1;

    pub fn new(config: SessionGapStandaloneConfig) -> Self {
        let test_hook = SessionGapTestHookState::from_env();
        Self {
            cash: config.start_cash,
            config,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
            last_dt: None,
            session_start_dt: None,
            session_end_dt: None,
            session_high: None,
            session_low: None,
            session_close: None,
            yesterday_close: None,
            yesterday_range: None,
            pre_prev_close: None,
            first_min_high: None,
            first_min_low: None,
            first_hour_price: None,
            phase_last_change_ts_utc: None,
            traded_session: false,
            pending_entry: None,
            position: None,
            last_warmup_log: None,
            test_hook,
        }
    }

    fn signals_warmed(&self) -> bool {
        self.yesterday_close.is_some()
            && self.yesterday_range.is_some()
            && self.pre_prev_close.is_some()
    }

    fn log_signal_warmup_status_if_changed(&mut self, session_date: &str, last_bar_ts: i64) {
        let warmed = self.signals_warmed();
        if self.last_warmup_log == Some(warmed) {
            return;
        }
        self.last_warmup_log = Some(warmed);
        if warmed {
            info!(
                strategy = "session_gap_standalone",
                symbol = self.config.symbol,
                session_date,
                prev_close = self.yesterday_close,
                pre_prev_close = self.pre_prev_close,
                yesterday_range = self.yesterday_range,
                signals_warmed = true,
                "signal warmup complete"
            );
        } else {
            warn!(
                strategy = "session_gap_standalone",
                symbol = self.config.symbol,
                session_date,
                last_bar_ts,
                prev_close = self.yesterday_close,
                pre_prev_close = self.pre_prev_close,
                yesterday_range = self.yesterday_range,
                signals_warmed = false,
                action = "signal_warmup_incomplete",
                reason = "indicators_not_warmed",
                "signal warmup incomplete"
            );
        }
    }

    fn to_session_dt(&self, ts_utc: i64) -> DateTime<FixedOffset> {
        let offset = FixedOffset::east_opt(self.config.timezone_offset_hours * 3600)
            .expect("valid fixed offset");
        Utc.timestamp_opt(ts_utc, 0)
            .single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
            .with_timezone(&offset)
    }

    fn update_session(&mut self, bar_dt: DateTime<FixedOffset>, bar: &BarEvent) {
        match self.last_dt {
            None => self.reset_session(bar_dt, bar),
            Some(last_dt) => {
                let diff_min = (bar_dt - last_dt).num_seconds() as f64 / 60.0;
                if diff_min > self.config.session_gap_min {
                    let old_session_close = self.session_close;
                    let old_session_high = self.session_high;
                    let old_session_low = self.session_low;
                    self.pre_prev_close = self.yesterday_close;
                    self.yesterday_close = self.session_close;
                    self.yesterday_range = match (self.session_high, self.session_low) {
                        (Some(high), Some(low)) => Some(high - low),
                        _ => None,
                    };
                    self.reset_session(bar_dt, bar);
                    info!(
                        strategy = "session_gap_standalone",
                        rollover_reason = "gap",
                        gap_minutes = diff_min,
                        old_session_close,
                        old_session_high,
                        old_session_low,
                        prev_close = self.yesterday_close,
                        pre_prev_close = self.pre_prev_close,
                        yesterday_range = self.yesterday_range,
                        new_session_date = self.session_date(bar_dt),
                        session_start_ts_utc = self
                            .session_start_dt
                            .map(|dt| dt.with_timezone(&Utc).timestamp()),
                        session_end_ts_utc = self
                            .session_end_dt
                            .map(|dt| dt.with_timezone(&Utc).timestamp()),
                        "session rollover summary"
                    );
                } else {
                    self.session_high = Some(self.session_high.unwrap_or(bar.h).max(bar.h));
                    self.session_low = Some(self.session_low.unwrap_or(bar.l).min(bar.l));
                    self.session_close = Some(bar.close);
                    if let Some(session_start_dt) = self.session_start_dt {
                        if bar_dt.time() == session_start_dt.time() {
                            self.first_min_high = Some(bar.h);
                            self.first_min_low = Some(bar.l);
                        }
                        if self.first_hour_price.is_none()
                            && (bar_dt - session_start_dt).num_seconds() >= 3600
                        {
                            self.first_hour_price = Some(bar.close);
                        }
                    }
                }
            }
        }
        self.last_dt = Some(bar_dt);
    }

    fn reset_session(&mut self, bar_dt: DateTime<FixedOffset>, bar: &BarEvent) {
        let offset = *bar_dt.offset();
        let session_end_dt = offset
            .with_ymd_and_hms(
                bar_dt.year(),
                bar_dt.month(),
                bar_dt.day(),
                self.config.close_hour,
                self.config.close_minute,
                0,
            )
            .single()
            .unwrap_or(bar_dt);

        self.session_start_dt = Some(bar_dt);
        self.session_end_dt = Some(session_end_dt);
        self.session_high = Some(bar.h);
        self.session_low = Some(bar.l);
        self.session_close = Some(bar.close);
        self.first_min_high = Some(bar.h);
        self.first_min_low = Some(bar.l);
        self.first_hour_price = None;
        self.traded_session = false;
        self.pending_entry = None;
        self.position = None;
    }

    fn entry_side(direction: Direction) -> Side {
        match direction {
            Direction::Long => Side::Buy,
            Direction::Short => Side::Sell,
        }
    }

    fn exit_side(direction: Direction) -> Side {
        match direction {
            Direction::Long => Side::Sell,
            Direction::Short => Side::Buy,
        }
    }

    fn maybe_open_position(&mut self, bar: &BarEvent) -> Option<Intent> {
        if self.position.is_some() {
            self.pending_entry = None;
            return None;
        }
        let pending = self.pending_entry.take()?;
        let range = self.yesterday_range?;

        let (tp, sl) = match pending.direction {
            Direction::Long => (
                bar.o + self.config.k_tp_long * range,
                bar.o - self.config.k_sl_long * range,
            ),
            Direction::Short => (
                bar.o - self.config.k_tp_short * range,
                bar.o + self.config.k_sl_short * range,
            ),
        };

        let position = Position {
            direction: pending.direction,
            size: pending.size,
            tp,
            sl,
        };
        match pending.direction {
            Direction::Long => self.cash -= bar.o * pending.size as f64,
            Direction::Short => self.cash += bar.o * pending.size as f64,
        }
        self.position = Some(position);

        Some(Intent::Market {
            qty: pending.size as f64,
            side: Self::entry_side(pending.direction),
            fill_price: Some(bar.o),
            comment: None,
        })
    }

    fn live_marketable_price(&self, side: Side, bar: &BarEvent) -> f64 {
        if self.config.tick_size <= 0.0 {
            return bar.close;
        }
        // Keep at least one extra tick of aggressiveness, because gateway normalization
        // (buy=floor, sell=ceil) can otherwise de-aggress price to passive limit.
        let aggressive_ticks = self.config.place_offset_ticks.max(0) + 1;
        let shift = aggressive_ticks as f64 * self.config.tick_size;
        match side {
            Side::Buy => bar.close + shift,
            Side::Sell => bar.close - shift,
        }
    }

    fn maybe_close_position(
        &mut self,
        bar_dt: DateTime<FixedOffset>,
        bar: &BarEvent,
    ) -> Option<Intent> {
        let position = self.position.clone()?;
        let mut exit_price_reason = None;

        if let Some(session_end_dt) = self.session_end_dt {
            let exit_threshold = session_end_dt - Duration::minutes(self.config.exit_offset_min);
            if bar_dt >= exit_threshold {
                exit_price_reason = Some(bar.close);
            }
        }

        if exit_price_reason.is_none() {
            match position.direction {
                Direction::Long => {
                    if bar.h >= position.tp {
                        exit_price_reason = Some(position.tp);
                    } else if bar.l <= position.sl {
                        exit_price_reason = Some(position.sl);
                    }
                }
                Direction::Short => {
                    if bar.l <= position.tp {
                        exit_price_reason = Some(position.tp);
                    } else if bar.h >= position.sl {
                        exit_price_reason = Some(position.sl);
                    }
                }
            }
        }

        let exit_price = exit_price_reason?;

        match position.direction {
            Direction::Long => self.cash += exit_price * position.size as f64,
            Direction::Short => self.cash -= exit_price * position.size as f64,
        }

        self.position = None;

        Some(Intent::Market {
            qty: position.size as f64,
            side: Self::exit_side(position.direction),
            fill_price: Some(exit_price),
            comment: None,
        })
    }

    fn maybe_generate_signal(
        &mut self,
        bar_dt: DateTime<FixedOffset>,
        bar: &BarEvent,
        force_single_size: bool,
    ) {
        if self.traded_session {
            return;
        }

        let (session_start_dt, session_end_dt) = match (self.session_start_dt, self.session_end_dt)
        {
            (Some(start), Some(end)) => (start, end),
            _ => return,
        };

        if bar_dt >= session_end_dt || bar_dt.hour() >= self.config.max_entry_hour {
            return;
        }

        let (yesterday_close, yesterday_range, pre_prev_close) = match (
            self.yesterday_close,
            self.yesterday_range,
            self.pre_prev_close,
        ) {
            (Some(close), Some(range), Some(prev_close)) => (close, range, prev_close),
            _ => {
                if bar_dt.minute() == self.config.signal_minute
                    && (bar_dt - session_start_dt).num_seconds() >= self.config.wait_hours * 3600
                {
                    warn!(
                        strategy = "session_gap_standalone",
                        symbol = self.config.symbol,
                        ts_utc = bar.close_time_utc,
                        action = "entry_blocked",
                        reason = "indicators_not_warmed",
                        prev_close = self.yesterday_close,
                        pre_prev_close = self.pre_prev_close,
                        yesterday_range = self.yesterday_range,
                        "entry blocked: indicators not warmed"
                    );
                }
                return;
            }
        };

        if bar_dt.minute() == self.config.signal_minute
            && (bar_dt - session_start_dt).num_seconds() >= self.config.wait_hours * 3600
        {
            let price = bar.close;
            let signal = if price > yesterday_close + self.config.k_long * yesterday_range
                && self.first_min_high.is_some_and(|high| price > high)
                && self
                    .first_hour_price
                    .is_some_and(|first_hour| price > first_hour)
                && yesterday_close > (1.0 - self.config.long_ex_pct / 100.0) * pre_prev_close
            {
                Some(Direction::Long)
            } else if price < yesterday_close - self.config.k_short * yesterday_range
                && self.first_min_low.is_some_and(|low| price < low)
                && self
                    .first_hour_price
                    .is_some_and(|first_hour| price < first_hour)
                && yesterday_close < (1.0 + self.config.short_ex_pct / 100.0) * pre_prev_close
            {
                Some(Direction::Short)
            } else {
                None
            };

            if let Some(direction) = signal {
                if self.pending_entry.is_some() {
                    return;
                }
                let mut size = if force_single_size {
                    1
                } else {
                    let available_cash = self.config.cash_factor * self.cash;
                    (available_cash / bar.close).floor() as i64
                };
                if size < 1 {
                    size = 1;
                }
                self.pending_entry = Some(PendingEntry { direction, size });
                self.traded_session = true;
            }
        }
    }

    fn session_date(&self, dt: DateTime<FixedOffset>) -> String {
        format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
    }

    fn compute_tp_sl(&self, direction: Direction, open: f64) -> (Option<f64>, Option<f64>) {
        let Some(range) = self.yesterday_range else {
            return (None, None);
        };
        let (tp, sl) = match direction {
            Direction::Long => (
                open + self.config.k_tp_long * range,
                open - self.config.k_sl_long * range,
            ),
            Direction::Short => (
                open - self.config.k_tp_short * range,
                open + self.config.k_sl_short * range,
            ),
        };
        (Some(tp), Some(sl))
    }

    fn entry_recovery_verification_window_ms(&self) -> u64 {
        self.config
            .entry_ack_timeout_ms
            .max(Self::MIN_ENTRY_RECOVERY_VERIFICATION_MS)
    }

    fn is_transient_entry_transport_error(ack: &CommandAck) -> bool {
        matches!(ack.status, AckStatus::Error)
            && ack.error_code.as_deref() == Some("cws_error")
            && ack.broker_order_id.is_none()
    }

    fn is_window_closed_recoverable_reject(ack: &CommandAck) -> bool {
        ack.error_code.as_deref() == Some("trading_window_closed") && ack.broker_order_id.is_none()
    }

    fn is_retryable_exit_recycle_failure(ack: &CommandAck) -> bool {
        matches!(ack.status, AckStatus::Error)
            && ack.error_code.as_deref() == Some("control_path_recycle_failed")
            && ack.broker_order_id.is_none()
    }

    #[allow(clippy::too_many_arguments)]
    fn close_only_degraded_phase(
        side: Side,
        qty: f64,
        baseline_qty: f64,
        reason: impl Into<String>,
        entered_ts_utc: i64,
        retry_attempts_exhausted: u32,
        last_error_code: Option<String>,
        last_error_msg: Option<String>,
    ) -> SessionGapLivePhase {
        SessionGapLivePhase::CloseOnlyDegraded {
            side,
            qty,
            baseline_qty,
            reason: reason.into(),
            entered_ts_utc,
            retry_attempts_exhausted,
            last_error_code,
            last_error_msg,
            operator_intervention_required: true,
        }
    }

    fn exit_retry_request_id(ctx: &StrategyCtx, ts_utc: i64) -> uuid::Uuid {
        crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ctx.symbol,
            "place",
            ts_utc,
            0,
        )
    }

    fn build_live_exit_intent(price: f64, qty: f64, side: Side) -> Intent {
        Intent::Place {
            price,
            qty,
            side,
            comment: None,
        }
        .with_class(IntentClass::Exit)
    }

    fn log_entry_terminal_failure(ack: &CommandAck) {
        warn!(
            strategy = "session_gap_standalone",
            action = "entry_failed_terminal",
            request_id = %ack.request_id,
            status = ?ack.status,
            error_code = ?ack.error_code,
            error_msg = ?ack.error_msg,
            broker_order_id = ack.broker_order_id,
            cws_request_guid = ?ack.cws_request_guid,
            "session gap entry failed terminally"
        );
    }

    fn log_entry_transport_failure(ack: &CommandAck) {
        warn!(
            strategy = "session_gap_standalone",
            action = "entry_failed_transport_transient",
            request_id = %ack.request_id,
            status = ?ack.status,
            error_code = ?ack.error_code,
            error_msg = ?ack.error_msg,
            broker_order_id = ack.broker_order_id,
            cws_request_guid = ?ack.cws_request_guid,
            "session gap entry hit transient transport failure"
        );
    }

    fn log_entry_recovery_pending(
        request_id: uuid::Uuid,
        verification_started_ts: i64,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "entry_recovery_verification_pending",
            request_id = %request_id,
            verification_started_ts,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap entry recovery verification started"
        );
    }

    fn log_entry_window_closed_deferred(
        request_id: uuid::Uuid,
        side: Side,
        qty: f64,
        ts_utc: i64,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "entry_deferred_window_closed",
            request_id = %request_id,
            side = ?side,
            qty,
            ts_utc,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap entry deferred until trading resumes"
        );
    }

    fn log_entry_window_closed_reissued(
        original_request_id: uuid::Uuid,
        request_id: uuid::Uuid,
        side: Side,
        qty: f64,
        ts_utc: i64,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "entry_reissued_after_window_closed",
            original_request_id = %original_request_id,
            request_id = %request_id,
            side = ?side,
            qty,
            ts_utc,
            "session gap entry reissued after trading resumed"
        );
    }

    fn log_entry_window_closed_expired(
        original_request_id: uuid::Uuid,
        ts_utc: i64,
        reason: &'static str,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "entry_deferred_window_closed_expired",
            original_request_id = %original_request_id,
            ts_utc,
            reason,
            "session gap deferred entry expired without reissue"
        );
    }

    fn log_entry_recovered_to_flat(request_id: uuid::Uuid, ts_utc: i64, reason: &'static str) {
        info!(
            strategy = "session_gap_standalone",
            action = "entry_recovered_to_flat",
            request_id = %request_id,
            ts_utc,
            reason,
            "session gap entry recovered safely to flat"
        );
    }

    fn log_exit_recovery_started(
        request_id: uuid::Uuid,
        retry_attempt: u32,
        error_code: Option<&str>,
        error_msg: Option<&str>,
        ts_utc: i64,
    ) {
        warn!(
            strategy = "session_gap_standalone",
            action = "exit_recovery_started",
            request_id = %request_id,
            retry_attempt,
            ts_utc,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap exit recovery started"
        );
    }

    fn log_exit_recycle_retry_started(
        previous_request_id: uuid::Uuid,
        request_id: uuid::Uuid,
        retry_attempt: u32,
        ts_utc: i64,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        warn!(
            strategy = "session_gap_standalone",
            action = "exit_recycle_retry_started",
            previous_request_id = %previous_request_id,
            request_id = %request_id,
            retry_attempt,
            ts_utc,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap exit recycle retry started"
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn log_exit_window_closed_deferred(
        request_id: uuid::Uuid,
        side: Side,
        qty: f64,
        baseline_qty: f64,
        reason: &str,
        ts_utc: i64,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "exit_deferred_window_closed",
            request_id = %request_id,
            side = ?side,
            qty,
            baseline_qty,
            reason,
            ts_utc,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap exit deferred until trading resumes"
        );
    }

    fn log_exit_window_closed_reissued(
        original_request_id: uuid::Uuid,
        request_id: uuid::Uuid,
        side: Side,
        qty: f64,
        baseline_qty: f64,
        reason: &str,
        ts_utc: i64,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "exit_reissued_after_window_closed",
            original_request_id = %original_request_id,
            request_id = %request_id,
            side = ?side,
            qty,
            baseline_qty,
            reason,
            ts_utc,
            "session gap exit reissued after trading resumed"
        );
    }

    fn log_exit_recycle_retry_success(
        request_id: uuid::Uuid,
        retry_attempt: u32,
        status: AckStatus,
        broker_order_id: Option<i64>,
    ) {
        info!(
            strategy = "session_gap_standalone",
            action = "exit_recycle_retry_success",
            request_id = %request_id,
            retry_attempt,
            status = ?status,
            broker_order_id,
            "session gap exit recycle retry succeeded"
        );
    }

    fn log_exit_recycle_retry_failed(
        request_id: uuid::Uuid,
        retry_attempt: u32,
        status: AckStatus,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        warn!(
            strategy = "session_gap_standalone",
            action = "exit_recycle_retry_failed",
            request_id = %request_id,
            retry_attempt,
            status = ?status,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap exit recycle retry failed"
        );
    }

    fn log_exit_close_only_degraded_entered(
        reason: &str,
        retry_attempts_exhausted: u32,
        ts_utc: i64,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        warn!(
            strategy = "session_gap_standalone",
            action = "exit_close_only_degraded_entered",
            reason,
            retry_attempts_exhausted,
            ts_utc,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap entered close-only degraded state"
        );
    }

    fn log_exit_operator_intervention_required(
        reason: &str,
        ts_utc: i64,
        error_code: Option<&str>,
        error_msg: Option<&str>,
    ) {
        warn!(
            strategy = "session_gap_standalone",
            action = "exit_operator_intervention_required",
            reason,
            ts_utc,
            error_code = ?error_code,
            error_msg = ?error_msg,
            "session gap exit requires operator intervention"
        );
    }

    fn transition_live_reconcile_with_snapshot(
        &mut self,
        snapshot_qty: f64,
        has_working_order: bool,
        ts_utc: i64,
    ) {
        let (
            phase,
            persisted_session_start_ts_utc,
            persisted_session_end_ts_utc,
            persisted_last_dt_ts_utc,
            persisted_last_bar_ts,
            persisted_phase_last_change_ts_utc,
        ) = match &self.state {
            StrategyState::SessionGapStandalone {
                phase,
                session_start_ts_utc,
                session_end_ts_utc,
                last_dt_ts_utc,
                last_bar_ts,
                phase_last_change_ts_utc,
                ..
            } => (
                phase.clone(),
                *session_start_ts_utc,
                *session_end_ts_utc,
                *last_dt_ts_utc,
                *last_bar_ts,
                *phase_last_change_ts_utc,
            ),
            _ => return,
        };
        let corrected = match phase {
            SessionGapLivePhase::PendingEntry {
                side,
                qty,
                baseline_qty,
                tp,
                sl,
                ..
            } => {
                if (snapshot_qty - baseline_qty).abs() > f64::EPSILON {
                    Some(SessionGapLivePhase::InPosition {
                        side,
                        qty,
                        avg_price: 0.0,
                        baseline_qty,
                        tp,
                        sl,
                        opened_ts: ts_utc,
                    })
                } else {
                    None
                }
            }
            SessionGapLivePhase::EntryRecoveryVerificationPending {
                request_id,
                side,
                qty,
                baseline_qty,
                tp,
                sl,
                ..
            } => {
                if (snapshot_qty - baseline_qty).abs() > f64::EPSILON {
                    Some(SessionGapLivePhase::InPosition {
                        side,
                        qty,
                        avg_price: 0.0,
                        baseline_qty,
                        tp,
                        sl,
                        opened_ts: ts_utc,
                    })
                } else if !has_working_order {
                    Self::log_entry_recovered_to_flat(
                        request_id,
                        ts_utc,
                        "bootstrap_snapshot_no_position_no_working_order",
                    );
                    Some(SessionGapLivePhase::Flat)
                } else {
                    None
                }
            }
            SessionGapLivePhase::EntryDeferredWindowClosed { .. } => None,
            SessionGapLivePhase::InPosition { baseline_qty, .. }
            | SessionGapLivePhase::PendingExit { baseline_qty, .. }
            | SessionGapLivePhase::ExitDeferredWindowClosed { baseline_qty, .. }
            | SessionGapLivePhase::ExitRecoveryPending { baseline_qty, .. }
            | SessionGapLivePhase::CloseOnlyDegraded { baseline_qty, .. } => {
                if (snapshot_qty - baseline_qty).abs() <= f64::EPSILON {
                    Some(SessionGapLivePhase::Flat)
                } else {
                    None
                }
            }
            SessionGapLivePhase::Flat | SessionGapLivePhase::Blocked { .. } => None,
        };

        if let Some(phase) = corrected {
            info!(
                strategy = "session_gap_standalone",
                broker_qty = snapshot_qty,
                "state corrected by broker snapshot"
            );
            self.state = StrategyState::SessionGapStandalone {
                session_date: Some(self.session_date(self.to_session_dt(ts_utc))),
                traded_session: self.traded_session,
                prev_close: self.yesterday_close,
                yesterday_range: self.yesterday_range,
                pre_prev_close: self.pre_prev_close,
                first_min_high: self.first_min_high,
                first_min_low: self.first_min_low,
                first_hour_price: self.first_hour_price,
                session_start_ts_utc: self
                    .session_start_dt
                    .map(|dt| dt.with_timezone(&chrono::Utc).timestamp())
                    .or(persisted_session_start_ts_utc),
                session_end_ts_utc: self
                    .session_end_dt
                    .map(|dt| dt.with_timezone(&chrono::Utc).timestamp())
                    .or(persisted_session_end_ts_utc),
                session_high: self.session_high,
                session_low: self.session_low,
                session_close: self.session_close,
                last_dt_ts_utc: self
                    .synced_last_dt_ts_utc(persisted_last_bar_ts.unwrap_or(ts_utc))
                    .or(persisted_last_dt_ts_utc)
                    .or(persisted_last_bar_ts),
                phase,
                phase_last_change_ts_utc: self
                    .phase_last_change_ts_utc
                    .or(persisted_phase_last_change_ts_utc),
                last_bar_ts: persisted_last_bar_ts,
            };
        }
    }

    fn should_block_live(&self, ctx: &StrategyCtx, bar: &BarEvent) -> bool {
        ctx.trade_mode == TradeMode::Live
            && (bar.origin != crate::DataOrigin::Live
                || !ctx.allow_live_orders
                || ctx.gateway_phase != GatewayPhase::LiveReady)
    }

    fn phase_name(phase: &SessionGapLivePhase) -> &'static str {
        match phase {
            SessionGapLivePhase::Flat => "Flat",
            SessionGapLivePhase::PendingEntry { .. } => "PendingEntry",
            SessionGapLivePhase::EntryDeferredWindowClosed { .. } => "EntryDeferredWindowClosed",
            SessionGapLivePhase::EntryRecoveryVerificationPending { .. } => {
                "EntryRecoveryVerificationPending"
            }
            SessionGapLivePhase::InPosition { .. } => "InPosition",
            SessionGapLivePhase::PendingExit { .. } => "PendingExit",
            SessionGapLivePhase::ExitDeferredWindowClosed { .. } => "ExitDeferredWindowClosed",
            SessionGapLivePhase::ExitRecoveryPending { .. } => "ExitRecoveryPending",
            SessionGapLivePhase::CloseOnlyDegraded { .. } => "CloseOnlyDegraded",
            SessionGapLivePhase::Blocked { .. } => "Blocked",
        }
    }

    fn log_phase_transition(from: &SessionGapLivePhase, to: &SessionGapLivePhase, ts_utc: i64) {
        if Self::phase_name(from) == Self::phase_name(to) {
            return;
        }
        info!(
            strategy = "session_gap_standalone",
            transition_from = Self::phase_name(from),
            transition_to = Self::phase_name(to),
            ts_utc,
            "live phase transition"
        );
    }

    fn persisted_phase_or_flat(&self) -> SessionGapLivePhase {
        match &self.state {
            StrategyState::SessionGapStandalone { phase, .. } => phase.clone(),
            _ => SessionGapLivePhase::Flat,
        }
    }

    fn synced_last_dt_ts_utc(&self, last_bar_ts: i64) -> Option<i64> {
        let last_dt_ts = self
            .last_dt
            .map(|dt| dt.with_timezone(&chrono::Utc).timestamp());
        Some(last_dt_ts.unwrap_or(last_bar_ts).max(last_bar_ts))
    }

    fn persist_state_snapshot(
        &mut self,
        session_date: String,
        phase: SessionGapLivePhase,
        last_bar_ts: i64,
    ) {
        self.state = StrategyState::SessionGapStandalone {
            session_date: Some(session_date),
            traded_session: self.traded_session,
            prev_close: self.yesterday_close,
            yesterday_range: self.yesterday_range,
            pre_prev_close: self.pre_prev_close,
            first_min_high: self.first_min_high,
            first_min_low: self.first_min_low,
            first_hour_price: self.first_hour_price,
            session_start_ts_utc: self
                .session_start_dt
                .map(|dt| dt.with_timezone(&chrono::Utc).timestamp()),
            session_end_ts_utc: self
                .session_end_dt
                .map(|dt| dt.with_timezone(&chrono::Utc).timestamp()),
            session_high: self.session_high,
            session_low: self.session_low,
            session_close: self.session_close,
            last_dt_ts_utc: self.synced_last_dt_ts_utc(last_bar_ts),
            phase,
            phase_last_change_ts_utc: self.phase_last_change_ts_utc,
            last_bar_ts: Some(last_bar_ts),
        };
    }

    fn persist_state_with_existing_last_bar(&mut self, phase: SessionGapLivePhase) {
        if let StrategyState::SessionGapStandalone {
            session_date,
            last_bar_ts,
            ..
        } = &self.state
        {
            self.state = StrategyState::SessionGapStandalone {
                session_date: session_date.clone(),
                traded_session: self.traded_session,
                prev_close: self.yesterday_close,
                yesterday_range: self.yesterday_range,
                pre_prev_close: self.pre_prev_close,
                first_min_high: self.first_min_high,
                first_min_low: self.first_min_low,
                first_hour_price: self.first_hour_price,
                session_start_ts_utc: self
                    .session_start_dt
                    .map(|dt| dt.with_timezone(&chrono::Utc).timestamp()),
                session_end_ts_utc: self
                    .session_end_dt
                    .map(|dt| dt.with_timezone(&chrono::Utc).timestamp()),
                session_high: self.session_high,
                session_low: self.session_low,
                session_close: self.session_close,
                last_dt_ts_utc: self
                    .synced_last_dt_ts_utc(last_bar_ts.unwrap_or(0))
                    .or(*last_bar_ts),
                phase,
                phase_last_change_ts_utc: self.phase_last_change_ts_utc,
                last_bar_ts: *last_bar_ts,
            };
        }
    }

    fn maybe_force_test_hook_entry(
        &mut self,
        phase: &SessionGapLivePhase,
        ctx: &StrategyCtx,
        session_date: &str,
        bar: &BarEvent,
    ) -> bool {
        let Some(direction) = self.test_hook.should_force_entry(session_date) else {
            return false;
        };
        let phase_allows_entry = matches!(phase, SessionGapLivePhase::Flat)
            || matches!(
                phase,
                SessionGapLivePhase::Blocked { reason, .. } if reason == "indicators_not_warmed"
            );
        if !phase_allows_entry
            || self.pending_entry.is_some()
            || ctx.position_qty.unwrap_or(0.0).abs() > f64::EPSILON
        {
            return false;
        }
        self.pending_entry = Some(PendingEntry { direction, size: 1 });
        self.traded_session = true;
        self.test_hook.mark_entry_dispatched();
        info!(
            strategy = "session_gap_standalone",
            action = "test_hook_force_entry",
            session_date,
            side = match direction {
                Direction::Long => "buy",
                Direction::Short => "sell",
            },
            ts_utc = bar.close_time_utc,
            "session_gap test hook armed forced entry"
        );
        true
    }
}

impl Strategy for SessionGapStandaloneStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        }
        if self
            .last_processed_bar_ts
            .is_some_and(|last_ts| bar.close_time_utc <= last_ts)
        {
            return Vec::new();
        }

        let bar_dt = self.to_session_dt(bar.close_time_utc);
        let current_session_date = self.session_date(bar_dt);
        if !self.config.work_weekends && bar_dt.weekday().number_from_monday() >= 6 {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            let phase = self.persisted_phase_or_flat();
            self.persist_state_snapshot(current_session_date, phase, bar.close_time_utc);
            return Vec::new();
        }

        self.update_session(bar_dt, bar);
        self.log_signal_warmup_status_if_changed(&current_session_date, bar.close_time_utc);

        if ctx.trade_mode != TradeMode::Live {
            let phase = self.persisted_phase_or_flat();
            let mut intents = Vec::new();
            if let Some(entry) = self.maybe_open_position(bar) {
                intents.push(entry);
            }
            if let Some(exit) = self.maybe_close_position(bar_dt, bar) {
                intents.push(exit);
            }
            self.maybe_generate_signal(bar_dt, bar, false);
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            self.persist_state_snapshot(current_session_date, phase, bar.close_time_utc);
            return intents;
        }

        if self.should_block_live(ctx, bar) {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            let phase = self.persisted_phase_or_flat();
            self.persist_state_snapshot(current_session_date, phase, bar.close_time_utc);
            return Vec::new();
        }

        let actual_phase = self.persisted_phase_or_flat();
        let forced_entry_armed =
            self.maybe_force_test_hook_entry(&actual_phase, ctx, &current_session_date, bar);
        if !forced_entry_armed {
            self.maybe_generate_signal(bar_dt, bar, true);
        }
        let phase = if forced_entry_armed
            && matches!(
                &actual_phase,
                SessionGapLivePhase::Blocked { reason, .. } if reason == "indicators_not_warmed"
            ) {
            SessionGapLivePhase::Flat
        } else {
            actual_phase.clone()
        };

        let mut intents = Vec::new();
        let previous_phase = actual_phase.clone();
        let next_phase = match phase {
            SessionGapLivePhase::Flat => {
                if !self.signals_warmed() && !forced_entry_armed {
                    SessionGapLivePhase::Blocked {
                        reason: "indicators_not_warmed".to_string(),
                        ts_utc: bar.close_time_utc,
                    }
                } else if let Some(pending) = self.pending_entry.take() {
                    let side = Self::entry_side(pending.direction);
                    let (tp, sl) = self.compute_tp_sl(pending.direction, bar.o);
                    let qty = pending.size as f64;
                    let request_id = crate::deterministic_request_id(
                        &ctx.strategy_id,
                        &ctx.portfolio,
                        &bar.symbol,
                        "place",
                        ctx.event_ts_utc(),
                        0,
                    );
                    intents.push(Intent::Place {
                        price: self.live_marketable_price(side, bar),
                        qty,
                        side,
                        comment: None,
                    });
                    SessionGapLivePhase::PendingEntry {
                        request_id,
                        side,
                        qty,
                        baseline_qty: ctx.position_qty.unwrap_or(0.0),
                        tp,
                        sl,
                        sent_ts: ctx.now_ts_utc(),
                        acked: false,
                    }
                } else {
                    SessionGapLivePhase::Flat
                }
            }
            SessionGapLivePhase::EntryDeferredWindowClosed {
                side,
                qty,
                original_request_id,
                ..
            } => {
                let session_closed = self.session_end_dt.is_some_and(|end| bar_dt >= end);
                if session_closed || bar_dt.hour() >= self.config.max_entry_hour {
                    Self::log_entry_window_closed_expired(
                        original_request_id,
                        bar.close_time_utc,
                        if session_closed {
                            "session_closed_before_reissue"
                        } else {
                            "max_entry_hour_reached_before_reissue"
                        },
                    );
                    SessionGapLivePhase::Flat
                } else {
                    let direction = match side {
                        Side::Buy => Direction::Long,
                        Side::Sell => Direction::Short,
                    };
                    let (tp, sl) = self.compute_tp_sl(direction, bar.o);
                    let request_id = crate::deterministic_request_id(
                        &ctx.strategy_id,
                        &ctx.portfolio,
                        &bar.symbol,
                        "place",
                        ctx.event_ts_utc(),
                        0,
                    );
                    intents.push(Intent::Place {
                        price: self.live_marketable_price(side, bar),
                        qty,
                        side,
                        comment: None,
                    });
                    Self::log_entry_window_closed_reissued(
                        original_request_id,
                        request_id,
                        side,
                        qty,
                        ctx.now_ts_utc(),
                    );
                    SessionGapLivePhase::PendingEntry {
                        request_id,
                        side,
                        qty,
                        baseline_qty: ctx.position_qty.unwrap_or(0.0),
                        tp,
                        sl,
                        sent_ts: ctx.now_ts_utc(),
                        acked: false,
                    }
                }
            }
            SessionGapLivePhase::Blocked { reason, .. }
                if reason == "indicators_not_warmed" && self.signals_warmed() =>
            {
                SessionGapLivePhase::Flat
            }
            SessionGapLivePhase::Blocked { .. } | SessionGapLivePhase::CloseOnlyDegraded { .. } => {
                phase
            }
            SessionGapLivePhase::PendingEntry { sent_ts, acked, .. } => {
                let elapsed = ctx
                    .now_ts_utc()
                    .saturating_sub(sent_ts)
                    .saturating_mul(1000) as u64;
                if !acked && elapsed > self.config.entry_ack_timeout_ms {
                    SessionGapLivePhase::Blocked {
                        reason: "entry_ack_timeout".to_string(),
                        ts_utc: ctx.now_ts_utc(),
                    }
                } else if acked && elapsed > self.config.entry_fill_timeout_ms {
                    SessionGapLivePhase::Blocked {
                        reason: "entry_fill_timeout".to_string(),
                        ts_utc: ctx.now_ts_utc(),
                    }
                } else {
                    phase
                }
            }
            SessionGapLivePhase::EntryRecoveryVerificationPending {
                request_id,
                qty,
                baseline_qty,
                tp,
                sl,
                verification_started_ts,
                ..
            } => {
                let current_qty = ctx.position_qty.unwrap_or(baseline_qty);
                let delta = current_qty - baseline_qty;
                let elapsed = ctx
                    .now_ts_utc()
                    .saturating_sub(verification_started_ts)
                    .saturating_mul(1000) as u64;
                if delta.abs() > f64::EPSILON {
                    SessionGapLivePhase::InPosition {
                        side: if delta >= 0.0 { Side::Buy } else { Side::Sell },
                        qty: delta.abs().max(qty),
                        avg_price: 0.0,
                        baseline_qty,
                        tp,
                        sl,
                        opened_ts: ctx.now_ts_utc(),
                    }
                } else if elapsed > self.entry_recovery_verification_window_ms() {
                    Self::log_entry_recovered_to_flat(
                        request_id,
                        ctx.now_ts_utc(),
                        "verification_window_elapsed_without_position_or_order",
                    );
                    SessionGapLivePhase::Flat
                } else {
                    phase
                }
            }
            SessionGapLivePhase::InPosition {
                side,
                qty,
                baseline_qty,
                tp,
                sl,
                opened_ts,
                ..
            } => {
                let mut should_exit = false;
                let mut reason = "";
                if self.test_hook.should_force_exit(
                    &current_session_date,
                    opened_ts,
                    bar.close_time_utc,
                ) {
                    should_exit = true;
                    reason = "test_hook_exit";
                } else if let Some(session_end_dt) = self.session_end_dt {
                    let exit_threshold =
                        session_end_dt - Duration::minutes(self.config.exit_offset_min);
                    if bar_dt >= exit_threshold {
                        should_exit = true;
                        reason = "session_exit";
                    }
                }
                if !should_exit {
                    match side {
                        Side::Buy => {
                            if tp.is_some_and(|tpv| bar.h >= tpv) {
                                should_exit = true;
                                reason = "tp";
                            } else if sl.is_some_and(|slv| bar.l <= slv) {
                                should_exit = true;
                                reason = "sl";
                            }
                        }
                        Side::Sell => {
                            if tp.is_some_and(|tpv| bar.l <= tpv) {
                                should_exit = true;
                                reason = "tp";
                            } else if sl.is_some_and(|slv| bar.h >= slv) {
                                should_exit = true;
                                reason = "sl";
                            }
                        }
                    }
                }
                if should_exit && bar.close_time_utc > opened_ts {
                    let exit_side = match side {
                        Side::Buy => Side::Sell,
                        Side::Sell => Side::Buy,
                    };
                    let exit_price = self.live_marketable_price(exit_side, bar);
                    if reason == "test_hook_exit" {
                        self.test_hook.mark_exit_dispatched();
                    }
                    let request_id = crate::deterministic_request_id(
                        &ctx.strategy_id,
                        &ctx.portfolio,
                        &bar.symbol,
                        "place",
                        ctx.event_ts_utc(),
                        0,
                    );
                    intents.push(Self::build_live_exit_intent(exit_price, qty, exit_side));
                    SessionGapLivePhase::PendingExit {
                        request_id,
                        side: exit_side,
                        qty,
                        price: exit_price,
                        baseline_qty,
                        reason: reason.to_string(),
                        sent_ts: ctx.now_ts_utc(),
                        acked: false,
                    }
                } else {
                    phase
                }
            }
            SessionGapLivePhase::ExitDeferredWindowClosed {
                baseline_qty,
                ref reason,
                original_request_id,
                ..
            } => {
                let current_qty = ctx.position_qty.unwrap_or(baseline_qty);
                let delta = current_qty - baseline_qty;
                if delta.abs() <= f64::EPSILON {
                    SessionGapLivePhase::Flat
                } else {
                    let side = if delta >= 0.0 { Side::Sell } else { Side::Buy };
                    let qty = delta.abs();
                    let exit_price = self.live_marketable_price(side, bar);
                    let request_id = crate::deterministic_request_id(
                        &ctx.strategy_id,
                        &ctx.portfolio,
                        &bar.symbol,
                        "place",
                        ctx.event_ts_utc(),
                        0,
                    );
                    intents.push(Self::build_live_exit_intent(exit_price, qty, side));
                    Self::log_exit_window_closed_reissued(
                        original_request_id,
                        request_id,
                        side,
                        qty,
                        baseline_qty,
                        reason,
                        ctx.now_ts_utc(),
                    );
                    SessionGapLivePhase::PendingExit {
                        request_id,
                        side,
                        qty,
                        price: exit_price,
                        baseline_qty,
                        reason: reason.clone(),
                        sent_ts: ctx.now_ts_utc(),
                        acked: false,
                    }
                }
            }
            SessionGapLivePhase::PendingExit {
                side,
                qty,
                baseline_qty,
                ref reason,
                sent_ts,
                acked,
                ..
            } => {
                let elapsed = ctx
                    .now_ts_utc()
                    .saturating_sub(sent_ts)
                    .saturating_mul(1000) as u64;
                if !acked && elapsed > self.config.exit_ack_timeout_ms {
                    let phase = Self::close_only_degraded_phase(
                        side,
                        qty,
                        baseline_qty,
                        format!("exit_ack_timeout:{reason}"),
                        ctx.now_ts_utc(),
                        0,
                        Some("exit_ack_timeout".to_string()),
                        Some("exit ack timeout exceeded".to_string()),
                    );
                    Self::log_exit_close_only_degraded_entered(
                        "exit_ack_timeout",
                        0,
                        ctx.now_ts_utc(),
                        Some("exit_ack_timeout"),
                        Some("exit ack timeout exceeded"),
                    );
                    Self::log_exit_operator_intervention_required(
                        "exit_ack_timeout",
                        ctx.now_ts_utc(),
                        Some("exit_ack_timeout"),
                        Some("exit ack timeout exceeded"),
                    );
                    phase
                } else if acked && elapsed > self.config.exit_fill_timeout_ms {
                    let phase = Self::close_only_degraded_phase(
                        side,
                        qty,
                        baseline_qty,
                        format!("exit_fill_timeout:{reason}"),
                        ctx.now_ts_utc(),
                        0,
                        Some("exit_fill_timeout".to_string()),
                        Some("exit fill timeout exceeded".to_string()),
                    );
                    Self::log_exit_close_only_degraded_entered(
                        "exit_fill_timeout",
                        0,
                        ctx.now_ts_utc(),
                        Some("exit_fill_timeout"),
                        Some("exit fill timeout exceeded"),
                    );
                    Self::log_exit_operator_intervention_required(
                        "exit_fill_timeout",
                        ctx.now_ts_utc(),
                        Some("exit_fill_timeout"),
                        Some("exit fill timeout exceeded"),
                    );
                    phase
                } else {
                    phase
                }
            }
            SessionGapLivePhase::ExitRecoveryPending {
                side,
                qty,
                baseline_qty,
                ref reason,
                sent_ts,
                acked,
                retry_attempt,
                ..
            } => {
                let elapsed = ctx
                    .now_ts_utc()
                    .saturating_sub(sent_ts)
                    .saturating_mul(1000) as u64;
                if !acked && elapsed > self.config.exit_ack_timeout_ms {
                    let phase = Self::close_only_degraded_phase(
                        side,
                        qty,
                        baseline_qty,
                        format!("exit_recovery_ack_timeout:{reason}"),
                        ctx.now_ts_utc(),
                        retry_attempt,
                        Some("exit_recovery_ack_timeout".to_string()),
                        Some("exit recovery ack timeout exceeded".to_string()),
                    );
                    Self::log_exit_close_only_degraded_entered(
                        "exit_recovery_ack_timeout",
                        retry_attempt,
                        ctx.now_ts_utc(),
                        Some("exit_recovery_ack_timeout"),
                        Some("exit recovery ack timeout exceeded"),
                    );
                    Self::log_exit_operator_intervention_required(
                        "exit_recovery_ack_timeout",
                        ctx.now_ts_utc(),
                        Some("exit_recovery_ack_timeout"),
                        Some("exit recovery ack timeout exceeded"),
                    );
                    phase
                } else if acked && elapsed > self.config.exit_fill_timeout_ms {
                    let phase = Self::close_only_degraded_phase(
                        side,
                        qty,
                        baseline_qty,
                        format!("exit_recovery_fill_timeout:{reason}"),
                        ctx.now_ts_utc(),
                        retry_attempt,
                        Some("exit_recovery_fill_timeout".to_string()),
                        Some("exit recovery fill timeout exceeded".to_string()),
                    );
                    Self::log_exit_close_only_degraded_entered(
                        "exit_recovery_fill_timeout",
                        retry_attempt,
                        ctx.now_ts_utc(),
                        Some("exit_recovery_fill_timeout"),
                        Some("exit recovery fill timeout exceeded"),
                    );
                    Self::log_exit_operator_intervention_required(
                        "exit_recovery_fill_timeout",
                        ctx.now_ts_utc(),
                        Some("exit_recovery_fill_timeout"),
                        Some("exit recovery fill timeout exceeded"),
                    );
                    phase
                } else {
                    phase
                }
            }
        };

        Self::log_phase_transition(&previous_phase, &next_phase, bar.close_time_utc);
        if Self::phase_name(&previous_phase) != Self::phase_name(&next_phase) {
            self.phase_last_change_ts_utc = Some(bar.close_time_utc);
        }
        self.persist_state_snapshot(current_session_date, next_phase, bar.close_time_utc);

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intents
    }

    #[allow(clippy::collapsible_match)]
    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        let mut intents = Vec::new();
        let mut phase_to_persist: Option<SessionGapLivePhase> = None;
        if let StrategyState::SessionGapStandalone { phase, .. } = &mut self.state {
            let previous_phase = phase.clone();
            match phase {
                SessionGapLivePhase::PendingEntry {
                    request_id,
                    side,
                    qty,
                    baseline_qty,
                    tp,
                    sl,
                    acked,
                    ..
                } => {
                    if *request_id == ack.request_id {
                        if Self::is_window_closed_recoverable_reject(ack) {
                            Self::log_entry_window_closed_deferred(
                                *request_id,
                                *side,
                                *qty,
                                ack.processed_ts_utc,
                                ack.error_code.as_deref(),
                                ack.error_msg.as_deref(),
                            );
                            *phase = SessionGapLivePhase::EntryDeferredWindowClosed {
                                side: *side,
                                qty: *qty,
                                deferred_ts_utc: ack.processed_ts_utc,
                                original_request_id: *request_id,
                                last_error_code: ack.error_code.clone(),
                                last_error_msg: ack.error_msg.clone(),
                            };
                        } else {
                            match ack.status {
                                AckStatus::Accepted
                                | AckStatus::Confirmed
                                | AckStatus::Duplicate => {
                                    *acked = true;
                                }
                                AckStatus::Rejected | AckStatus::Expired => {
                                    Self::log_entry_terminal_failure(ack);
                                    *phase = SessionGapLivePhase::Blocked {
                                        reason: format!("ack_failed:{:?}", ack.status),
                                        ts_utc: ack.processed_ts_utc,
                                    };
                                }
                                AckStatus::Error => {
                                    if Self::is_transient_entry_transport_error(ack) {
                                        Self::log_entry_transport_failure(ack);
                                        let request_id = *request_id;
                                        *phase =
                                            SessionGapLivePhase::EntryRecoveryVerificationPending {
                                                request_id,
                                                side: *side,
                                                qty: *qty,
                                                baseline_qty: *baseline_qty,
                                                tp: *tp,
                                                sl: *sl,
                                                verification_started_ts: ack.processed_ts_utc,
                                                transport_error_code: ack.error_code.clone(),
                                                transport_error_msg: ack.error_msg.clone(),
                                            };
                                        Self::log_entry_recovery_pending(
                                            request_id,
                                            ack.processed_ts_utc,
                                            ack.error_code.as_deref(),
                                            ack.error_msg.as_deref(),
                                        );
                                    } else {
                                        Self::log_entry_terminal_failure(ack);
                                        *phase = SessionGapLivePhase::Blocked {
                                            reason: format!("ack_failed:{:?}", ack.status),
                                            ts_utc: ack.processed_ts_utc,
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
                SessionGapLivePhase::PendingExit {
                    request_id,
                    side,
                    qty,
                    price,
                    baseline_qty,
                    reason,
                    acked,
                    ..
                } => {
                    if *request_id == ack.request_id {
                        if Self::is_window_closed_recoverable_reject(ack) {
                            Self::log_exit_window_closed_deferred(
                                *request_id,
                                *side,
                                *qty,
                                *baseline_qty,
                                reason,
                                ack.processed_ts_utc,
                                ack.error_code.as_deref(),
                                ack.error_msg.as_deref(),
                            );
                            *phase = SessionGapLivePhase::ExitDeferredWindowClosed {
                                side: *side,
                                qty: *qty,
                                baseline_qty: *baseline_qty,
                                reason: reason.clone(),
                                deferred_ts_utc: ack.processed_ts_utc,
                                original_request_id: *request_id,
                                last_error_code: ack.error_code.clone(),
                                last_error_msg: ack.error_msg.clone(),
                            };
                        } else {
                            match ack.status {
                                AckStatus::Accepted
                                | AckStatus::Confirmed
                                | AckStatus::Duplicate => {
                                    *acked = true;
                                }
                                AckStatus::Rejected | AckStatus::Expired => {
                                    Self::log_exit_recycle_retry_failed(
                                        *request_id,
                                        0,
                                        ack.status.clone(),
                                        ack.error_code.as_deref(),
                                        ack.error_msg.as_deref(),
                                    );
                                    *phase = Self::close_only_degraded_phase(
                                        *side,
                                        *qty,
                                        *baseline_qty,
                                        format!("ack_failed:{:?}:{reason}", ack.status),
                                        ack.processed_ts_utc,
                                        0,
                                        ack.error_code.clone(),
                                        ack.error_msg.clone(),
                                    );
                                    Self::log_exit_close_only_degraded_entered(
                                        "exit_terminal_ack_failure",
                                        0,
                                        ack.processed_ts_utc,
                                        ack.error_code.as_deref(),
                                        ack.error_msg.as_deref(),
                                    );
                                    Self::log_exit_operator_intervention_required(
                                        "exit_terminal_ack_failure",
                                        ack.processed_ts_utc,
                                        ack.error_code.as_deref(),
                                        ack.error_msg.as_deref(),
                                    );
                                }
                                AckStatus::Error => {
                                    let current_request_id = *request_id;
                                    let current_side = *side;
                                    let current_qty = *qty;
                                    let current_price = *price;
                                    let current_baseline_qty = *baseline_qty;
                                    let current_reason = reason.clone();
                                    if Self::is_retryable_exit_recycle_failure(ack)
                                        && Self::EXIT_RECOVERY_MAX_RETRIES > 0
                                    {
                                        let next_retry_attempt = 1;
                                        let next_request_id =
                                            Self::exit_retry_request_id(ctx, ack.processed_ts_utc);
                                        *phase = SessionGapLivePhase::ExitRecoveryPending {
                                            request_id: next_request_id,
                                            side: current_side,
                                            qty: current_qty,
                                            price: current_price,
                                            baseline_qty: current_baseline_qty,
                                            reason: current_reason.clone(),
                                            sent_ts: ack.processed_ts_utc,
                                            acked: false,
                                            retry_attempt: next_retry_attempt,
                                            last_error_code: ack.error_code.clone(),
                                            last_error_msg: ack.error_msg.clone(),
                                        };
                                        Self::log_exit_recovery_started(
                                            current_request_id,
                                            next_retry_attempt,
                                            ack.error_code.as_deref(),
                                            ack.error_msg.as_deref(),
                                            ack.processed_ts_utc,
                                        );
                                        Self::log_exit_recycle_retry_started(
                                            current_request_id,
                                            next_request_id,
                                            next_retry_attempt,
                                            ack.processed_ts_utc,
                                            ack.error_code.as_deref(),
                                            ack.error_msg.as_deref(),
                                        );
                                        intents.push(Self::build_live_exit_intent(
                                            current_price,
                                            current_qty,
                                            current_side,
                                        ));
                                    } else {
                                        Self::log_exit_recycle_retry_failed(
                                            current_request_id,
                                            0,
                                            ack.status.clone(),
                                            ack.error_code.as_deref(),
                                            ack.error_msg.as_deref(),
                                        );
                                        *phase = Self::close_only_degraded_phase(
                                            current_side,
                                            current_qty,
                                            current_baseline_qty,
                                            format!("ack_failed:{:?}:{current_reason}", ack.status),
                                            ack.processed_ts_utc,
                                            0,
                                            ack.error_code.clone(),
                                            ack.error_msg.clone(),
                                        );
                                        Self::log_exit_close_only_degraded_entered(
                                            "exit_error_without_retry_lane",
                                            0,
                                            ack.processed_ts_utc,
                                            ack.error_code.as_deref(),
                                            ack.error_msg.as_deref(),
                                        );
                                        Self::log_exit_operator_intervention_required(
                                            "exit_error_without_retry_lane",
                                            ack.processed_ts_utc,
                                            ack.error_code.as_deref(),
                                            ack.error_msg.as_deref(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                SessionGapLivePhase::ExitRecoveryPending {
                    request_id,
                    side,
                    qty,
                    baseline_qty,
                    reason,
                    retry_attempt,
                    acked,
                    ..
                } => {
                    if *request_id == ack.request_id {
                        if Self::is_window_closed_recoverable_reject(ack) {
                            Self::log_exit_window_closed_deferred(
                                *request_id,
                                *side,
                                *qty,
                                *baseline_qty,
                                reason,
                                ack.processed_ts_utc,
                                ack.error_code.as_deref(),
                                ack.error_msg.as_deref(),
                            );
                            *phase = SessionGapLivePhase::ExitDeferredWindowClosed {
                                side: *side,
                                qty: *qty,
                                baseline_qty: *baseline_qty,
                                reason: reason.clone(),
                                deferred_ts_utc: ack.processed_ts_utc,
                                original_request_id: *request_id,
                                last_error_code: ack.error_code.clone(),
                                last_error_msg: ack.error_msg.clone(),
                            };
                        } else {
                            match ack.status {
                                AckStatus::Accepted
                                | AckStatus::Confirmed
                                | AckStatus::Duplicate => {
                                    *acked = true;
                                    Self::log_exit_recycle_retry_success(
                                        *request_id,
                                        *retry_attempt,
                                        ack.status.clone(),
                                        ack.broker_order_id,
                                    );
                                }
                                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                                    let current_request_id = *request_id;
                                    let current_retry_attempt = *retry_attempt;
                                    let current_side = *side;
                                    let current_qty = *qty;
                                    let current_baseline_qty = *baseline_qty;
                                    let current_reason = reason.clone();
                                    Self::log_exit_recycle_retry_failed(
                                        current_request_id,
                                        current_retry_attempt,
                                        ack.status.clone(),
                                        ack.error_code.as_deref(),
                                        ack.error_msg.as_deref(),
                                    );
                                    *phase = Self::close_only_degraded_phase(
                                        current_side,
                                        current_qty,
                                        current_baseline_qty,
                                        format!("ack_failed:{:?}:{current_reason}", ack.status),
                                        ack.processed_ts_utc,
                                        current_retry_attempt,
                                        ack.error_code.clone(),
                                        ack.error_msg.clone(),
                                    );
                                    Self::log_exit_close_only_degraded_entered(
                                        "exit_recovery_exhausted",
                                        current_retry_attempt,
                                        ack.processed_ts_utc,
                                        ack.error_code.as_deref(),
                                        ack.error_msg.as_deref(),
                                    );
                                    Self::log_exit_operator_intervention_required(
                                        "exit_recovery_exhausted",
                                        ack.processed_ts_utc,
                                        ack.error_code.as_deref(),
                                        ack.error_msg.as_deref(),
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            Self::log_phase_transition(&previous_phase, phase, ack.processed_ts_utc);
            if Self::phase_name(&previous_phase) != Self::phase_name(phase) {
                self.phase_last_change_ts_utc = Some(ack.processed_ts_utc);
            }
            phase_to_persist = Some(phase.clone());
        }
        if let Some(phase) = phase_to_persist {
            self.persist_state_with_existing_last_bar(phase);
        }
        intents
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, ord: &crate::OrderEvent) -> Vec<Intent> {
        let mut phase_to_persist: Option<SessionGapLivePhase> = None;
        if let StrategyState::SessionGapStandalone { phase, .. } = &mut self.state {
            let previous_phase = phase.clone();
            if let SessionGapLivePhase::EntryRecoveryVerificationPending {
                request_id,
                side,
                qty,
                baseline_qty,
                tp,
                sl,
                verification_started_ts,
                ..
            } = phase.clone()
            {
                if ord.request_id == Some(request_id) {
                    let status = ord.status.to_ascii_lowercase();
                    if status == "filled" {
                        *phase = SessionGapLivePhase::InPosition {
                            side,
                            qty,
                            avg_price: ord.price,
                            baseline_qty,
                            tp,
                            sl,
                            opened_ts: ord.ts_utc.max(verification_started_ts),
                        };
                    } else if status == "working" {
                        *phase = SessionGapLivePhase::PendingEntry {
                            request_id,
                            side,
                            qty,
                            baseline_qty,
                            tp,
                            sl,
                            sent_ts: verification_started_ts,
                            acked: true,
                        };
                    }
                }
            }
            Self::log_phase_transition(&previous_phase, phase, ord.ts_utc);
            if Self::phase_name(&previous_phase) != Self::phase_name(phase) {
                self.phase_last_change_ts_utc = Some(ord.ts_utc);
            }
            phase_to_persist = Some(phase.clone());
        }
        if let Some(phase) = phase_to_persist {
            self.persist_state_with_existing_last_bar(phase);
        }
        Vec::new()
    }

    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }
        let now_ts = ctx.last_bar_ts().unwrap_or(pos.ts_utc);
        let mut phase_to_persist: Option<SessionGapLivePhase> = None;
        if let StrategyState::SessionGapStandalone { phase, .. } = &mut self.state {
            let previous_phase = phase.clone();
            match phase.clone() {
                SessionGapLivePhase::PendingEntry {
                    baseline_qty,
                    tp,
                    sl,
                    ..
                } => {
                    let delta = pos.qty - baseline_qty;
                    if delta.abs() > f64::EPSILON {
                        let side = if delta >= 0.0 { Side::Buy } else { Side::Sell };
                        *phase = SessionGapLivePhase::InPosition {
                            side,
                            qty: delta.abs(),
                            avg_price: pos.avg_price,
                            baseline_qty,
                            tp,
                            sl,
                            opened_ts: now_ts,
                        };
                    }
                }
                SessionGapLivePhase::EntryDeferredWindowClosed { .. } => {}
                SessionGapLivePhase::EntryRecoveryVerificationPending {
                    baseline_qty,
                    tp,
                    sl,
                    ..
                } => {
                    let delta = pos.qty - baseline_qty;
                    if delta.abs() > f64::EPSILON {
                        *phase = SessionGapLivePhase::InPosition {
                            side: if delta >= 0.0 { Side::Buy } else { Side::Sell },
                            qty: delta.abs(),
                            avg_price: pos.avg_price,
                            baseline_qty,
                            tp,
                            sl,
                            opened_ts: now_ts,
                        };
                    }
                }
                SessionGapLivePhase::InPosition { baseline_qty, .. }
                | SessionGapLivePhase::PendingExit { baseline_qty, .. }
                | SessionGapLivePhase::ExitDeferredWindowClosed { baseline_qty, .. }
                | SessionGapLivePhase::ExitRecoveryPending { baseline_qty, .. }
                | SessionGapLivePhase::CloseOnlyDegraded { baseline_qty, .. } => {
                    if (pos.qty - baseline_qty).abs() <= f64::EPSILON {
                        *phase = SessionGapLivePhase::Flat;
                    }
                }
                SessionGapLivePhase::Flat => {
                    if pos.qty.abs() > f64::EPSILON {
                        warn!(
                            strategy = "session_gap_standalone",
                            qty = pos.qty,
                            "state corrected by broker"
                        );
                        *phase = SessionGapLivePhase::InPosition {
                            side: if pos.qty >= 0.0 {
                                Side::Buy
                            } else {
                                Side::Sell
                            },
                            qty: pos.qty.abs(),
                            avg_price: pos.avg_price,
                            baseline_qty: 0.0,
                            tp: None,
                            sl: None,
                            opened_ts: now_ts,
                        };
                    }
                }
                SessionGapLivePhase::Blocked { .. } => {}
            }
            Self::log_phase_transition(&previous_phase, phase, now_ts);
            if Self::phase_name(&previous_phase) != Self::phase_name(phase) {
                self.phase_last_change_ts_utc = Some(now_ts);
            }
            phase_to_persist = Some(phase.clone());
        }
        if let Some(phase) = phase_to_persist {
            self.persist_state_with_existing_last_bar(phase);
        }
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        _ctx: &StrategyCtx,
        snapshot: &crate::BootstrapSnapshot,
    ) -> Vec<Intent> {
        info!(
            strategy = "session_gap_standalone",
            snapshot_ts_utc = snapshot.snapshot_ts_utc,
            positions = snapshot.positions_strategy.len(),
            working_orders = snapshot.working_orders_strategy.len(),
            "bootstrap snapshot received"
        );
        for (symbol, position) in &snapshot.positions_strategy {
            info!(
                strategy = "session_gap_standalone",
                symbol,
                qty = position.qty,
                avg_price = position.avg_price,
                ts_utc = position.ts_utc,
                "restored position from snapshot"
            );
        }
        info!(
            strategy = "session_gap_standalone",
            prev_close = self.yesterday_close,
            yesterday_range = self.yesterday_range,
            first_hour_price = self.first_hour_price,
            "indicators at snapshot restore"
        );
        let snapshot_qty = snapshot
            .positions_strategy
            .get(&self.config.symbol)
            .map(|p| p.qty)
            .unwrap_or(0.0);
        let has_working_order = snapshot
            .working_orders_strategy
            .values()
            .any(|order| order.symbol == self.config.symbol);
        let ts = snapshot.snapshot_ts_utc.unwrap_or(0);
        self.transition_live_reconcile_with_snapshot(snapshot_qty, has_working_order, ts);
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        _state: &crate::RuntimeStateRestored,
    ) -> Vec<Intent> {
        if let StrategyState::SessionGapStandalone {
            traded_session,
            prev_close,
            yesterday_range,
            pre_prev_close,
            first_min_high,
            first_min_low,
            first_hour_price,
            session_start_ts_utc,
            session_end_ts_utc,
            session_high,
            session_low,
            session_close,
            last_dt_ts_utc,
            phase,
            phase_last_change_ts_utc,
            last_bar_ts,
            ..
        } = &self.state
        {
            self.traded_session = *traded_session;
            self.yesterday_close = *prev_close;
            self.yesterday_range = *yesterday_range;
            self.pre_prev_close = *pre_prev_close;
            self.last_warmup_log = Some(self.signals_warmed());
            self.first_min_high = *first_min_high;
            self.first_min_low = *first_min_low;
            self.first_hour_price = *first_hour_price;
            self.session_high = *session_high;
            self.session_low = *session_low;
            self.session_close = *session_close;
            self.phase_last_change_ts_utc = *phase_last_change_ts_utc;
            let offset = chrono::FixedOffset::east_opt(self.config.timezone_offset_hours * 3600)
                .expect("valid fixed offset");
            self.session_start_dt = session_start_ts_utc.and_then(|ts| {
                chrono::Utc
                    .timestamp_opt(ts, 0)
                    .single()
                    .map(|dt| dt.with_timezone(&offset))
            });
            self.session_end_dt = session_end_ts_utc.and_then(|ts| {
                chrono::Utc
                    .timestamp_opt(ts, 0)
                    .single()
                    .map(|dt| dt.with_timezone(&offset))
            });
            self.last_dt = last_dt_ts_utc.and_then(|ts| {
                chrono::Utc
                    .timestamp_opt(ts, 0)
                    .single()
                    .map(|dt| dt.with_timezone(&offset))
            });
            self.last_processed_bar_ts = *last_bar_ts;
            info!(
                strategy = "session_gap_standalone",
                traded_session,
                prev_close,
                yesterday_range,
                pre_prev_close,
                first_min_high,
                first_min_low,
                first_hour_price,
                session_start_ts_utc,
                session_end_ts_utc,
                session_high,
                session_low,
                session_close,
                last_dt_ts_utc,
                phase = Self::phase_name(phase),
                phase_last_change_ts_utc,
                last_bar_ts,
                "restored indicators from runtime state"
            );
        }
        Vec::new()
    }

    fn warmup_from_history(&mut self, _ctx: &StrategyCtx, bars: &[BarEvent]) -> usize {
        let mut warmup = SessionGapStandaloneStrategy::new(self.config.clone());
        let mut processed = 0usize;
        let mut last_bar_ts = None;

        for bar in bars {
            if bar.symbol != self.config.symbol {
                continue;
            }
            let bar_dt = warmup.to_session_dt(bar.close_time_utc);
            if !warmup.config.work_weekends && bar_dt.weekday().number_from_monday() >= 6 {
                continue;
            }
            warmup.update_session(bar_dt, bar);
            processed += 1;
            last_bar_ts = Some(bar.close_time_utc);
        }

        let Some(last_bar_ts) = last_bar_ts else {
            return 0;
        };

        self.yesterday_close = warmup.yesterday_close;
        self.yesterday_range = warmup.yesterday_range;
        self.pre_prev_close = warmup.pre_prev_close;
        self.first_min_high = warmup.first_min_high;
        self.first_min_low = warmup.first_min_low;
        self.first_hour_price = warmup.first_hour_price;
        self.session_start_dt = warmup.session_start_dt;
        self.session_end_dt = warmup.session_end_dt;
        self.session_high = warmup.session_high;
        self.session_low = warmup.session_low;
        self.session_close = warmup.session_close;
        self.last_dt = warmup.last_dt;
        self.last_warmup_log = None;

        let previous_phase = self.persisted_phase_or_flat();
        let next_phase = match &previous_phase {
            SessionGapLivePhase::Blocked { reason, .. }
                if reason == "indicators_not_warmed" && self.signals_warmed() =>
            {
                SessionGapLivePhase::Flat
            }
            _ => previous_phase.clone(),
        };
        if Self::phase_name(&previous_phase) != Self::phase_name(&next_phase) {
            Self::log_phase_transition(&previous_phase, &next_phase, last_bar_ts);
            self.phase_last_change_ts_utc = Some(last_bar_ts);
        }
        let session_date = self.session_date(self.to_session_dt(last_bar_ts));
        self.persist_state_snapshot(session_date, next_phase, last_bar_ts);

        info!(
            strategy = "session_gap_standalone",
            symbol = self.config.symbol,
            processed,
            prev_close = self.yesterday_close,
            yesterday_range = self.yesterday_range,
            pre_prev_close = self.pre_prev_close,
            first_min_high = self.first_min_high,
            first_min_low = self.first_min_low,
            first_hour_price = self.first_hour_price,
            session_high = self.session_high,
            session_low = self.session_low,
            session_close = self.session_close,
            "session gap history warmup applied"
        );

        processed
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn pending_request_ids(&self) -> Vec<uuid::Uuid> {
        match self.persisted_phase_or_flat() {
            SessionGapLivePhase::PendingEntry { request_id, .. }
            | SessionGapLivePhase::PendingExit { request_id, .. }
            | SessionGapLivePhase::ExitRecoveryPending { request_id, .. } => vec![request_id],
            _ => Vec::new(),
        }
    }

    fn exit_risk_status(
        &self,
        has_open_position: bool,
    ) -> crate::strategy_host::StrategyExitRiskStatus {
        match self.persisted_phase_or_flat() {
            SessionGapLivePhase::ExitRecoveryPending { .. } => {
                crate::strategy_host::StrategyExitRiskStatus {
                    phase_override: Some("ExitRecoveryPending".to_string()),
                    exit_recovery_active: true,
                    operator_intervention_required: false,
                    open_risk_position_unflattened: has_open_position,
                }
            }
            SessionGapLivePhase::CloseOnlyDegraded {
                operator_intervention_required,
                ..
            } => crate::strategy_host::StrategyExitRiskStatus {
                phase_override: Some("CloseOnlyDegraded".to_string()),
                exit_recovery_active: false,
                operator_intervention_required,
                open_risk_position_unflattened: has_open_position,
            },
            _ => crate::strategy_host::StrategyExitRiskStatus::default(),
        }
    }

    fn set_state(&mut self, state: StrategyState) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use crate::{BarEvent, DataOrigin};
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard};

    static TEST_HOOK_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TestHookEnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl TestHookEnvGuard {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let lock = TEST_HOOK_ENV_LOCK.lock().expect("env test lock");
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                saved.push((*key, std::env::var(key).ok()));
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for TestHookEnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn ctx_live(
        allow_live_orders: bool,
        gateway_phase: crate::live_guard::GatewayPhase,
    ) -> StrategyCtx {
        StrategyCtx {
            strategy_id: "s".into(),
            portfolio: "p".into(),
            exchange: "e".into(),
            symbol: "USDRUBF".into(),
            tick_size: 0.01,
            trade_mode: crate::TradeMode::Live,
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders,
            gateway_phase,
            position_qty: Some(0.0),
            event_ts_utc: 0,
            now_ts_utc: 0,
            last_bar_ts: None,
        }
    }

    fn ctx_backtest() -> StrategyCtx {
        StrategyCtx {
            strategy_id: "s".into(),
            portfolio: "p".into(),
            exchange: "e".into(),
            symbol: "USDRUBF".into(),
            tick_size: 0.01,
            trade_mode: crate::TradeMode::Backtest,
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders: false,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty: None,
            event_ts_utc: 0,
            now_ts_utc: 0,
            last_bar_ts: None,
        }
    }

    fn ctx_live_at(now_ts_utc: i64, last_bar_ts: Option<i64>, position_qty: f64) -> StrategyCtx {
        StrategyCtx {
            strategy_id: "s".into(),
            portfolio: "p".into(),
            exchange: "e".into(),
            symbol: "USDRUBF".into(),
            tick_size: 0.01,
            trade_mode: crate::TradeMode::Live,
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders: true,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty: Some(position_qty),
            event_ts_utc: now_ts_utc,
            now_ts_utc,
            last_bar_ts,
        }
    }

    fn bar(ts: i64, o: f64, h: f64, l: f64, c: f64) -> BarEvent {
        BarEvent {
            symbol: "USDRUBF".to_string(),
            close_time_utc: ts,
            close: c,
            o,
            h,
            l,
            v: 0.0,
            origin: DataOrigin::Replay,
        }
    }

    fn assert_exit_place_intent(intent: &Intent, side: Side, qty: f64) {
        assert_eq!(intent.explicit_class(), Some(IntentClass::Exit));
        assert!(matches!(
            intent.base_intent(),
            Intent::Place {
                side: actual_side,
                qty: actual_qty,
                ..
            } if *actual_side == side && (*actual_qty - qty).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn history_warmup_rebuilds_indicators_and_clears_warmup_block() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2026-04-02".to_string()),
            traded_session: false,
            prev_close: None,
            yesterday_range: None,
            pre_prev_close: None,
            first_min_high: None,
            first_min_low: None,
            first_hour_price: None,
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::Blocked {
                reason: "indicators_not_warmed".to_string(),
                ts_utc: 1,
            },
            phase_last_change_ts_utc: Some(1),
            last_bar_ts: Some(1),
        };

        let processed = strategy.warmup_from_history(
            &ctx_live(false, crate::live_guard::GatewayPhase::LiveReady),
            &[
                bar(1_000, 100.0, 100.0, 100.0, 100.0),
                bar(10_000, 101.0, 101.0, 101.0, 101.0),
                bar(20_000, 102.0, 102.0, 102.0, 102.0),
            ],
        );

        assert_eq!(processed, 3);
        assert_eq!(strategy.yesterday_close, Some(101.0));
        assert_eq!(strategy.pre_prev_close, Some(100.0));
        assert_eq!(strategy.yesterday_range, Some(0.0));
        assert_eq!(strategy.first_min_high, Some(102.0));
        assert_eq!(strategy.first_min_low, Some(102.0));
        match strategy.state() {
            StrategyState::SessionGapStandalone { phase, .. } => {
                assert!(matches!(phase, SessionGapLivePhase::Flat));
            }
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn live_test_hook_forces_entry_without_warmed_indicators() {
        let _env = TestHookEnvGuard::set(&[
            ("RUNTIME_ENABLE_TEST_HOOKS", Some("true")),
            ("SESSION_GAP_TEST_FORCE_SESSION_DATE", Some("2025-12-05")),
            ("SESSION_GAP_TEST_FORCE_SIDE", Some("buy")),
            ("SESSION_GAP_TEST_AUTO_FLATTEN", Some("true")),
        ]);
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let bar_ts = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let mut live_bar = bar(bar_ts, 81.0, 81.1, 80.9, 81.05);
        live_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &live_bar);

        assert_eq!(intents.len(), 1);
        assert!(matches!(
            intents[0],
            Intent::Place {
                side: Side::Buy,
                qty: 1.0,
                ..
            }
        ));
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingEntry {
                    side: Side::Buy,
                    qty: 1.0,
                    ..
                },
                traded_session: true,
                ..
            }
        ));
    }

    #[test]
    fn live_test_hook_forces_flatten_on_next_bar_after_fill() {
        let _env = TestHookEnvGuard::set(&[
            ("RUNTIME_ENABLE_TEST_HOOKS", Some("true")),
            ("SESSION_GAP_TEST_FORCE_SESSION_DATE", Some("2025-12-05")),
            ("SESSION_GAP_TEST_FORCE_SIDE", Some("buy")),
            ("SESSION_GAP_TEST_AUTO_FLATTEN", Some("true")),
        ]);
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let entry_bar_ts = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let next_bar_ts = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 1, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let mut entry_bar = bar(entry_bar_ts, 81.0, 81.1, 80.9, 81.05);
        entry_bar.origin = DataOrigin::Live;
        let _ = strategy.on_bar(&ctx, &entry_bar);

        let opened = PositionEvent {
            symbol: "USDRUBF".into(),
            qty: 1.0,
            existing: false,
            avg_price: 81.07,
            ts_utc: entry_bar_ts,
        };
        let _ = strategy.on_position(&ctx, &opened);

        let mut next_bar = bar(next_bar_ts, 81.1, 81.2, 81.0, 81.15);
        next_bar.origin = DataOrigin::Live;
        let intents = strategy.on_bar(&ctx, &next_bar);

        assert_eq!(intents.len(), 1);
        assert_exit_place_intent(&intents[0], Side::Sell, 1.0);
        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingExit { reason, .. },
                ..
            } => assert_eq!(reason, "test_hook_exit"),
            other => panic!("unexpected state after test-hook exit: {other:?}"),
        }
    }

    #[test]
    fn emits_market_entry_after_pending_signal() {
        let mut cfg = SessionGapStandaloneConfig::default();
        cfg.wait_hours = 2;
        let mut strategy = SessionGapStandaloneStrategy::new(cfg);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let session_start = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap();
        let b1_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 59, 0)
            .single()
            .unwrap();
        let b2_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 13, 0, 0)
            .single()
            .unwrap();

        strategy.last_dt = Some(b1_dt - Duration::minutes(1));
        strategy.session_start_dt = Some(session_start);
        strategy.session_end_dt = Some(
            offset
                .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
                .single()
                .unwrap(),
        );
        strategy.yesterday_close = Some(100.0);
        strategy.pre_prev_close = Some(99.0);
        strategy.yesterday_range = Some(2.0);
        strategy.first_min_high = Some(100.0);
        strategy.first_min_low = Some(90.0);
        strategy.first_hour_price = Some(100.0);

        let b1 = bar(b1_dt.timestamp(), 101.0, 101.1, 100.9, 101.2);
        let _ = strategy.on_bar(&ctx_backtest(), &b1);

        let b2 = bar(b2_dt.timestamp(), 101.5, 101.8, 101.3, 101.7);
        let intents = strategy.on_bar(&ctx_backtest(), &b2);

        assert_eq!(intents.len(), 1);
        assert!(matches!(
            intents[0],
            Intent::Market {
                side: Side::Buy,
                ..
            }
        ));
    }

    #[test]
    fn live_guard_blocked_does_not_emit_or_mutate_phase() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        strategy.pending_entry = Some(PendingEntry {
            direction: Direction::Long,
            size: 1,
        });
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: None,
            last_bar_ts: None,
        };
        strategy.yesterday_range = Some(2.0);

        let mut b = bar(1_733_386_740, 101.0, 101.0, 100.0, 101.0);
        b.origin = DataOrigin::Replay;
        let intents = strategy.on_bar(
            &ctx_live(true, crate::live_guard::GatewayPhase::LiveReady),
            &b,
        );

        assert!(intents.is_empty());
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Flat,
                ..
            }
        ));
    }

    #[test]
    fn live_pending_entry_uses_entry_ack_timeout_ms() {
        let mut cfg = SessionGapStandaloneConfig::default();
        cfg.entry_ack_timeout_ms = 1000;
        cfg.entry_fill_timeout_ms = 10_000;
        let mut strategy = SessionGapStandaloneStrategy::new(cfg);
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::PendingEntry {
                request_id: uuid::Uuid::nil(),
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: None,
                sl: None,
                sent_ts: 10,
                acked: false,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(10),
        };
        let mut b = bar(12, 101.0, 102.0, 100.0, 101.0);
        b.origin = DataOrigin::Live;
        let mut ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        ctx.now_ts_utc = 12;

        let _ = strategy.on_bar(&ctx, &b);
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Blocked { .. },
                ..
            }
        ));
    }

    #[test]
    fn live_pending_entry_does_not_timeout_from_bar_clock_alone() {
        let mut cfg = SessionGapStandaloneConfig::default();
        cfg.entry_ack_timeout_ms = 1000;
        let mut strategy = SessionGapStandaloneStrategy::new(cfg);
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::PendingEntry {
                request_id: uuid::Uuid::nil(),
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: None,
                sl: None,
                sent_ts: 10,
                acked: false,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(10),
        };
        let mut b = bar(70, 101.0, 102.0, 100.0, 101.0);
        b.origin = DataOrigin::Live;
        let mut ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        ctx.now_ts_utc = 10;

        let _ = strategy.on_bar(&ctx, &b);
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingEntry { .. },
                ..
            }
        ));
    }

    #[test]
    fn restart_runtime_state_keeps_cycle_and_closes_position() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let session_start_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let session_end_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let last_dt_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 18, 59, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();

        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: Some(session_start_ts_utc),
            session_end_ts_utc: Some(session_end_ts_utc),
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: Some(last_dt_ts_utc),
            phase: SessionGapLivePhase::PendingEntry {
                request_id: uuid::Uuid::new_v4(),
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(120.0),
                sl: Some(80.0),
                sent_ts: 1_000,
                acked: true,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(1_000),
        };

        let opened = PositionEvent {
            symbol: "USDRUBF".into(),
            qty: 1.0,
            existing: false,
            avg_price: 101.0,
            ts_utc: 1_001,
        };
        let _ = strategy.on_position(&ctx, &opened);

        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::InPosition { qty: 1.0, .. },
                ..
            }
        ));

        let ts_exit = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 30, 0)
            .single()
            .unwrap()
            .timestamp();
        let mut exit_bar = bar(ts_exit, 101.0, 102.0, 100.0, 101.5);
        exit_bar.origin = DataOrigin::Live;
        let intents = strategy.on_bar(&ctx, &exit_bar);

        assert_eq!(intents.len(), 1);
        assert_exit_place_intent(&intents[0], Side::Sell, 1.0);

        let closed = PositionEvent {
            symbol: "USDRUBF".into(),
            qty: 0.0,
            existing: false,
            avg_price: 0.0,
            ts_utc: ts_exit + 1,
        };
        let _ = strategy.on_position(&ctx, &closed);

        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Flat,
                ..
            }
        ));
    }

    #[test]
    fn snapshot_reconcile_preserves_persisted_last_bar_ts() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let ts_snapshot = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 20, 0)
            .single()
            .unwrap()
            .timestamp();
        let persisted_last_bar_ts = ts_snapshot - 25;

        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: Some(ts_snapshot),
            phase: SessionGapLivePhase::PendingEntry {
                request_id: uuid::Uuid::new_v4(),
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(120.0),
                sl: Some(80.0),
                sent_ts: ts_snapshot - 10,
                acked: true,
            },
            phase_last_change_ts_utc: Some(ts_snapshot - 30),
            last_bar_ts: Some(persisted_last_bar_ts),
        };

        let snapshot = crate::BootstrapSnapshot {
            positions_strategy: std::collections::HashMap::from([(
                "USDRUBF".to_string(),
                PositionEvent {
                    symbol: "USDRUBF".into(),
                    qty: 1.0,
                    existing: true,
                    avg_price: 101.0,
                    ts_utc: ts_snapshot,
                },
            )]),
            working_orders_strategy: std::collections::HashMap::new(),
            working_stop_orders_strategy: std::collections::HashMap::new(),
            snapshot_ts_utc: Some(ts_snapshot),
        };

        let _ = strategy.on_bootstrap_snapshot(&ctx, &snapshot);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::InPosition { .. },
                last_bar_ts,
                ..
            } => {
                assert_eq!(*last_bar_ts, Some(persisted_last_bar_ts));
            }
            other => panic!("unexpected state after reconcile: {other:?}"),
        }
    }

    #[test]
    fn restart_snapshot_reconciles_to_in_position_then_closes() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let session_start_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let session_end_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let ts_snapshot = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 20, 0)
            .single()
            .unwrap()
            .timestamp();

        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: Some(session_start_ts_utc),
            session_end_ts_utc: Some(session_end_ts_utc),
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: Some(ts_snapshot),
            phase: SessionGapLivePhase::PendingEntry {
                request_id: uuid::Uuid::new_v4(),
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(120.0),
                sl: Some(80.0),
                sent_ts: ts_snapshot - 10,
                acked: true,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(ts_snapshot - 10),
        };

        let snapshot = crate::BootstrapSnapshot {
            positions_strategy: std::collections::HashMap::from([(
                "USDRUBF".to_string(),
                PositionEvent {
                    symbol: "USDRUBF".into(),
                    qty: 1.0,
                    existing: true,
                    avg_price: 101.0,
                    ts_utc: ts_snapshot,
                },
            )]),
            working_orders_strategy: std::collections::HashMap::new(),
            working_stop_orders_strategy: std::collections::HashMap::new(),
            snapshot_ts_utc: Some(ts_snapshot),
        };
        let _ = strategy.on_bootstrap_snapshot(&ctx, &snapshot);

        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::InPosition { qty: 1.0, .. },
                ..
            }
        ));

        let ts_exit = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 30, 0)
            .single()
            .unwrap()
            .timestamp();
        let mut exit_bar = bar(ts_exit, 101.0, 102.0, 100.0, 101.5);
        exit_bar.origin = DataOrigin::Live;
        let intents = strategy.on_bar(&ctx, &exit_bar);

        assert_eq!(intents.len(), 1);
        assert_exit_place_intent(&intents[0], Side::Sell, 1.0);
    }

    #[test]
    fn live_rollover_does_not_get_overwritten_by_persisted_state() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();

        let last_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap();
        strategy.last_dt = Some(last_dt);
        strategy.session_start_dt = Some(last_dt);
        strategy.session_end_dt = Some(
            offset
                .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
                .single()
                .unwrap(),
        );
        strategy.session_high = Some(120.0);
        strategy.session_low = Some(80.0);
        strategy.session_close = Some(110.0);
        strategy.yesterday_close = Some(105.0);
        strategy.yesterday_range = Some(8.0);
        strategy.pre_prev_close = Some(98.0);
        strategy.first_hour_price = Some(101.0);
        strategy.first_min_high = Some(102.0);
        strategy.first_min_low = Some(99.0);

        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(999.0),
            yesterday_range: Some(777.0),
            pre_prev_close: Some(555.0),
            first_min_high: Some(444.0),
            first_min_low: Some(333.0),
            first_hour_price: Some(222.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(last_dt.timestamp()),
        };

        let rollover_ts = (last_dt + Duration::minutes(120))
            .with_timezone(&Utc)
            .timestamp();
        let mut rollover_bar = bar(rollover_ts, 130.0, 131.0, 129.0, 130.5);
        rollover_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &rollover_bar);
        assert!(intents.is_empty());

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                prev_close,
                yesterday_range,
                pre_prev_close,
                first_hour_price,
                first_min_high,
                first_min_low,
                ..
            } => {
                assert_eq!(*prev_close, Some(110.0));
                assert_eq!(*yesterday_range, Some(40.0));
                assert_eq!(*pre_prev_close, Some(105.0));
                assert_ne!(*prev_close, Some(999.0));
                assert_ne!(*yesterday_range, Some(777.0));
                assert_ne!(*pre_prev_close, Some(555.0));
                assert_eq!(*first_hour_price, None);
                assert_eq!(*first_min_high, Some(131.0));
                assert_eq!(*first_min_low, Some(129.0));
            }
            other => panic!("unexpected state after rollover: {other:?}"),
        }
    }

    #[test]
    fn blocked_bar_still_updates_session_and_persists_state() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(false, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();

        let last_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap();
        strategy.last_dt = Some(last_dt);
        strategy.session_start_dt = Some(last_dt);
        strategy.session_end_dt = Some(
            offset
                .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
                .single()
                .unwrap(),
        );
        strategy.session_high = Some(100.0);
        strategy.session_low = Some(95.0);
        strategy.session_close = Some(98.0);
        strategy.yesterday_close = Some(94.0);
        strategy.yesterday_range = Some(10.0);
        strategy.pre_prev_close = Some(90.0);

        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: strategy.yesterday_close,
            yesterday_range: strategy.yesterday_range,
            pre_prev_close: strategy.pre_prev_close,
            first_min_high: strategy.first_min_high,
            first_min_low: strategy.first_min_low,
            first_hour_price: strategy.first_hour_price,
            session_start_ts_utc: strategy
                .session_start_dt
                .map(|dt| dt.with_timezone(&Utc).timestamp()),
            session_end_ts_utc: strategy
                .session_end_dt
                .map(|dt| dt.with_timezone(&Utc).timestamp()),
            session_high: strategy.session_high,
            session_low: strategy.session_low,
            session_close: strategy.session_close,
            last_dt_ts_utc: strategy
                .last_dt
                .map(|dt| dt.with_timezone(&Utc).timestamp()),
            phase: SessionGapLivePhase::PendingEntry {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: None,
                sl: None,
                sent_ts: last_dt.with_timezone(&Utc).timestamp(),
                acked: false,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(last_dt.with_timezone(&Utc).timestamp()),
        };

        let next_ts = (last_dt + Duration::minutes(1))
            .with_timezone(&Utc)
            .timestamp();
        let mut blocked_bar = bar(next_ts, 99.0, 103.0, 94.0, 101.0);
        blocked_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &blocked_bar);
        assert!(intents.is_empty());

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase,
                last_bar_ts,
                last_dt_ts_utc,
                session_date,
                ..
            } => {
                assert!(matches!(
                    phase,
                    SessionGapLivePhase::PendingEntry {
                        request_id: phase_request,
                        ..
                    } if *phase_request == request_id
                ));
                assert_eq!(*last_bar_ts, Some(next_ts));
                assert_eq!(*last_dt_ts_utc, Some(next_ts));
                assert_eq!(session_date.as_deref(), Some("2025-12-05"));
            }
            other => panic!("unexpected state on blocked bar: {other:?}"),
        }

        assert_eq!(strategy.session_high, Some(103.0));
        assert_eq!(strategy.session_low, Some(94.0));
        assert_eq!(strategy.session_close, Some(101.0));
        assert_eq!(strategy.last_processed_bar_ts, Some(next_ts));
    }

    #[test]
    fn live_false_ready_without_indicators_is_blocked() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let mut b = bar(1_773_140_340, 79.0, 79.1, 78.9, 79.05);
        b.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &b);
        assert!(intents.is_empty());
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Blocked { ref reason, .. },
                ..
            } if reason == "indicators_not_warmed"
        ));
    }

    #[test]
    fn live_warmup_with_history_bars_reconstructs_signal_indicators() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();

        let day1_start = offset
            .with_ymd_and_hms(2026, 3, 9, 10, 0, 0)
            .single()
            .unwrap();
        let day2_start = day1_start + Duration::days(1);
        let day3_start = day2_start + Duration::days(1);

        let mut history = vec![
            bar(
                day1_start.with_timezone(&Utc).timestamp(),
                78.0,
                78.4,
                77.8,
                78.2,
            ),
            bar(
                (day1_start + Duration::minutes(10))
                    .with_timezone(&Utc)
                    .timestamp(),
                78.2,
                78.6,
                77.9,
                78.5,
            ),
            bar(
                day2_start.with_timezone(&Utc).timestamp(),
                78.6,
                79.0,
                78.5,
                78.9,
            ),
            bar(
                (day2_start + Duration::minutes(10))
                    .with_timezone(&Utc)
                    .timestamp(),
                78.9,
                79.2,
                78.7,
                79.1,
            ),
            bar(
                day3_start.with_timezone(&Utc).timestamp(),
                79.0,
                79.3,
                78.8,
                79.2,
            ),
        ];
        for event in &mut history {
            event.origin = DataOrigin::History;
        }

        for event in history {
            let _ = strategy.on_bar(&ctx, &event);
        }

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                prev_close,
                pre_prev_close,
                yesterday_range,
                phase,
                ..
            } => {
                assert!(prev_close.is_some());
                assert!(pre_prev_close.is_some());
                assert!(yesterday_range.is_some());
                assert!(matches!(phase, SessionGapLivePhase::Flat));
            }
            other => panic!("unexpected state after history warmup: {other:?}"),
        }
    }

    #[test]
    fn runtime_state_restored_applies_first_hour_price_indicator() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(123.45),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(1_000),
        };

        let restored = crate::RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: Vec::new(),
        };
        let _ = strategy.on_runtime_state_restored(&ctx, &restored);

        assert_eq!(strategy.first_hour_price, Some(123.45));
        assert_eq!(strategy.yesterday_close, Some(100.0));
        assert_eq!(strategy.yesterday_range, Some(2.0));
        assert!(strategy.traded_session);
    }

    #[test]
    fn runtime_state_restored_applies_signal_gating_indicators_and_session_times() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let session_start_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let session_end_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let last_dt_ts_utc = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();

        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(123.45),
            session_start_ts_utc: Some(session_start_ts_utc),
            session_end_ts_utc: Some(session_end_ts_utc),
            session_high: Some(110.0),
            session_low: Some(90.0),
            session_close: Some(105.0),
            last_dt_ts_utc: Some(last_dt_ts_utc),
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: Some(last_dt_ts_utc),
            last_bar_ts: Some(1_000),
        };

        let restored = crate::RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: Vec::new(),
        };
        let _ = strategy.on_runtime_state_restored(&ctx, &restored);

        assert_eq!(strategy.pre_prev_close, Some(99.0));
        assert_eq!(strategy.first_min_high, Some(101.0));
        assert_eq!(strategy.first_min_low, Some(98.0));
        assert_eq!(strategy.first_hour_price, Some(123.45));
        assert_eq!(
            strategy
                .session_start_dt
                .map(|dt| dt.with_timezone(&Utc).timestamp()),
            Some(session_start_ts_utc)
        );
        assert_eq!(
            strategy
                .session_end_dt
                .map(|dt| dt.with_timezone(&Utc).timestamp()),
            Some(session_end_ts_utc)
        );
        assert_eq!(
            strategy
                .last_dt
                .map(|dt| dt.with_timezone(&Utc).timestamp()),
            Some(last_dt_ts_utc)
        );
        assert_eq!(strategy.session_high, Some(110.0));
        assert_eq!(strategy.session_low, Some(90.0));
        assert_eq!(strategy.session_close, Some(105.0));
        assert_eq!(strategy.phase_last_change_ts_utc, Some(last_dt_ts_utc));
    }

    #[test]
    fn on_ack_persists_phase_last_change_without_overwriting_last_bar_ts() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::PendingEntry {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: None,
                sl: None,
                sent_ts: 1_000,
                acked: false,
            },
            phase_last_change_ts_utc: Some(1_000),
            last_bar_ts: Some(1_000),
        };

        let ack = CommandAck::rejected(request_id, "E_REJECT", "rejected");
        let ack_ts = ack.processed_ts_utc;
        let _ = strategy.on_ack(
            &ctx_live(true, crate::live_guard::GatewayPhase::LiveReady),
            &ack,
        );

        assert_eq!(strategy.phase_last_change_ts_utc, Some(ack_ts));
        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase,
                phase_last_change_ts_utc,
                last_bar_ts,
                ..
            } => {
                assert!(matches!(phase, SessionGapLivePhase::Blocked { .. }));
                assert_eq!(*phase_last_change_ts_utc, Some(ack_ts));
                assert_eq!(*last_bar_ts, Some(1_000));
            }
            other => panic!("unexpected state after ack: {other:?}"),
        }
    }

    #[test]
    fn transient_cws_error_enters_recovery_verification_instead_of_blocked() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::PendingEntry {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(101.0),
                sl: Some(99.0),
                sent_ts: 1_000,
                acked: false,
            },
            phase_last_change_ts_utc: Some(1_000),
            last_bar_ts: Some(1_000),
        };

        let mut ack = CommandAck::error(
            request_id,
            "cws_error",
            "cws disconnected: protocol_reset_without_close_handshake",
        );
        ack.processed_ts_utc = 1_005;
        let _ = strategy.on_ack(&ctx_live_at(1_005, Some(1_000), 0.0), &ack);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase:
                    SessionGapLivePhase::EntryRecoveryVerificationPending {
                        request_id: phase_request_id,
                        verification_started_ts,
                        transport_error_code,
                        transport_error_msg,
                        ..
                    },
                ..
            } => {
                assert_eq!(*phase_request_id, request_id);
                assert_eq!(*verification_started_ts, 1_005);
                assert_eq!(transport_error_code.as_deref(), Some("cws_error"));
                assert_eq!(
                    transport_error_msg.as_deref(),
                    Some("cws disconnected: protocol_reset_without_close_handshake")
                );
            }
            other => panic!("unexpected state after transient cws error: {other:?}"),
        }
    }

    #[test]
    fn business_reject_still_blocks_pending_entry() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::PendingEntry {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(101.0),
                sl: Some(99.0),
                sent_ts: 1_000,
                acked: false,
            },
            phase_last_change_ts_utc: Some(1_000),
            last_bar_ts: Some(1_000),
        };

        let _ = strategy.on_ack(
            &ctx_live_at(1_005, Some(1_000), 0.0),
            &CommandAck::rejected(request_id, "business_reject", "limit price invalid"),
        );

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Blocked { .. },
                ..
            } => {}
            other => panic!("business reject must block, got: {other:?}"),
        }
    }

    #[test]
    fn trading_window_closed_entry_reject_enters_deferred_phase() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::PendingEntry {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(101.0),
                sl: Some(99.0),
                sent_ts: 1_000,
                acked: false,
            },
            phase_last_change_ts_utc: Some(1_000),
            last_bar_ts: Some(1_000),
        };

        let ack = CommandAck::rejected(request_id, "trading_window_closed", "validation failed");
        let _ = strategy.on_ack(&ctx_live_at(1_005, Some(1_000), 0.0), &ack);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase:
                    SessionGapLivePhase::EntryDeferredWindowClosed {
                        side,
                        qty,
                        original_request_id,
                        last_error_code,
                        ..
                    },
                ..
            } => {
                assert_eq!(*side, Side::Buy);
                assert!((*qty - 1.0).abs() <= f64::EPSILON);
                assert_eq!(*original_request_id, request_id);
                assert_eq!(last_error_code.as_deref(), Some("trading_window_closed"));
            }
            other => panic!("expected deferred entry phase, got: {other:?}"),
        }
    }

    #[test]
    fn deferred_entry_reissues_after_trading_resumes() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        strategy.yesterday_close = Some(100.0);
        strategy.yesterday_range = Some(2.0);
        strategy.pre_prev_close = Some(99.0);
        strategy.first_min_high = Some(101.0);
        strategy.first_min_low = Some(98.0);
        strategy.first_hour_price = Some(100.5);
        strategy.session_start_dt = Some(
            offset
                .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
                .single()
                .unwrap(),
        );
        strategy.session_end_dt = Some(
            offset
                .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
                .single()
                .unwrap(),
        );
        strategy.traded_session = true;
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: Some(
                strategy
                    .session_start_dt
                    .unwrap()
                    .with_timezone(&Utc)
                    .timestamp(),
            ),
            session_end_ts_utc: Some(
                strategy
                    .session_end_dt
                    .unwrap()
                    .with_timezone(&Utc)
                    .timestamp(),
            ),
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::EntryDeferredWindowClosed {
                side: Side::Buy,
                qty: 1.0,
                deferred_ts_utc: 1_005,
                original_request_id: uuid::Uuid::new_v4(),
                last_error_code: Some("trading_window_closed".into()),
                last_error_msg: Some("validation failed".into()),
            },
            phase_last_change_ts_utc: Some(1_005),
            last_bar_ts: Some(1_000),
        };

        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let mut live_bar = bar(
            offset
                .with_ymd_and_hms(2025, 12, 5, 14, 6, 0)
                .single()
                .unwrap()
                .with_timezone(&Utc)
                .timestamp(),
            101.0,
            101.2,
            100.9,
            101.1,
        );
        live_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &live_bar);
        assert_eq!(intents.len(), 1);
        assert!(matches!(
            &intents[0],
            Intent::Place {
                side: Side::Buy,
                qty,
                ..
            } if (*qty - 1.0).abs() <= f64::EPSILON
        ));
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingEntry {
                    side: Side::Buy,
                    qty,
                    ..
                },
                ..
            } if (qty - 1.0).abs() <= f64::EPSILON
        ));
    }

    #[test]
    fn trading_window_closed_exit_reject_enters_deferred_phase() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::PendingExit {
                request_id,
                side: Side::Sell,
                qty: 1.0,
                price: 100.5,
                baseline_qty: 0.0,
                reason: "session_exit".into(),
                sent_ts: 1_000,
                acked: false,
            },
            phase_last_change_ts_utc: Some(1_000),
            last_bar_ts: Some(1_000),
        };

        let ack = CommandAck::rejected(request_id, "trading_window_closed", "validation failed");
        let _ = strategy.on_ack(&ctx_live_at(1_005, Some(1_000), 1.0), &ack);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase:
                    SessionGapLivePhase::ExitDeferredWindowClosed {
                        side,
                        qty,
                        baseline_qty,
                        reason,
                        original_request_id,
                        ..
                    },
                ..
            } => {
                assert_eq!(*side, Side::Sell);
                assert!((*qty - 1.0).abs() <= f64::EPSILON);
                assert!((*baseline_qty - 0.0).abs() <= f64::EPSILON);
                assert_eq!(reason, "session_exit");
                assert_eq!(*original_request_id, request_id);
            }
            other => panic!("expected deferred exit phase, got: {other:?}"),
        }
    }

    #[test]
    fn deferred_exit_reissues_after_trading_resumes_until_flat() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(1_000),
            phase: SessionGapLivePhase::ExitDeferredWindowClosed {
                side: Side::Sell,
                qty: 1.0,
                baseline_qty: 0.0,
                reason: "session_exit".into(),
                deferred_ts_utc: 1_005,
                original_request_id: uuid::Uuid::new_v4(),
                last_error_code: Some("trading_window_closed".into()),
                last_error_msg: Some("validation failed".into()),
            },
            phase_last_change_ts_utc: Some(1_005),
            last_bar_ts: Some(1_000),
        };

        let ctx = ctx_live_at(1_010, Some(1_000), 1.0);
        let mut live_bar = bar(1_010, 101.0, 101.2, 100.9, 101.1);
        live_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &live_bar);
        assert_eq!(intents.len(), 1);
        assert_exit_place_intent(&intents[0], Side::Sell, 1.0);
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingExit {
                    side: Side::Sell,
                    qty,
                    ..
                },
                ..
            } if (qty - 1.0).abs() <= f64::EPSILON
        ));
    }

    #[test]
    fn transient_error_without_position_recovers_to_flat_after_verification_window() {
        let mut cfg = SessionGapStandaloneConfig::default();
        cfg.entry_ack_timeout_ms = 1_000;
        let mut strategy = SessionGapStandaloneStrategy::new(cfg);
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(100),
            phase: SessionGapLivePhase::EntryRecoveryVerificationPending {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(101.0),
                sl: Some(99.0),
                verification_started_ts: 100,
                transport_error_code: Some("cws_error".into()),
                transport_error_msg: Some(
                    "cws disconnected: protocol_reset_without_close_handshake".into(),
                ),
            },
            phase_last_change_ts_utc: Some(100),
            last_bar_ts: Some(100),
        };

        let mut live_bar = bar(160, 100.0, 100.2, 99.8, 100.1);
        live_bar.origin = DataOrigin::Live;

        let _ = strategy.on_bar(&ctx_live_at(106, Some(100), 0.0), &live_bar);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Flat,
                ..
            } => {}
            other => panic!("transient recovery should return to flat, got: {other:?}"),
        }
    }

    #[test]
    fn transient_error_with_position_update_recovers_to_in_position() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(101.0),
            first_min_low: Some(98.0),
            first_hour_price: Some(100.5),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: Some(102.0),
            session_low: Some(97.0),
            session_close: Some(100.0),
            last_dt_ts_utc: Some(100),
            phase: SessionGapLivePhase::EntryRecoveryVerificationPending {
                request_id,
                side: Side::Buy,
                qty: 1.0,
                baseline_qty: 0.0,
                tp: Some(101.0),
                sl: Some(99.0),
                verification_started_ts: 100,
                transport_error_code: Some("cws_error".into()),
                transport_error_msg: Some(
                    "cws disconnected: protocol_reset_without_close_handshake".into(),
                ),
            },
            phase_last_change_ts_utc: Some(100),
            last_bar_ts: Some(100),
        };

        let pos = PositionEvent {
            symbol: "USDRUBF".into(),
            qty: 1.0,
            existing: false,
            avg_price: 83.14,
            ts_utc: 104,
        };
        let _ = strategy.on_position(&ctx_live_at(104, Some(100), 0.0), &pos);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::InPosition { qty, avg_price, .. },
                ..
            } => {
                assert_eq!(*qty, 1.0);
                assert_eq!(*avg_price, 83.14);
            }
            other => panic!("position tail should recover to in-position, got: {other:?}"),
        }
    }

    #[test]
    fn first_hour_price_is_sticky_within_session() {
        let mut cfg = SessionGapStandaloneConfig::default();
        cfg.session_gap_min = 10_000.0;
        let mut strategy = SessionGapStandaloneStrategy::new(cfg);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();

        let session_start = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap();
        strategy.last_dt = Some(session_start);
        strategy.session_start_dt = Some(session_start);
        strategy.session_end_dt = Some(
            offset
                .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
                .single()
                .unwrap(),
        );
        strategy.session_high = Some(101.0);
        strategy.session_low = Some(99.0);
        strategy.session_close = Some(100.0);
        strategy.yesterday_close = Some(98.0);
        strategy.yesterday_range = Some(2.0);
        strategy.pre_prev_close = Some(97.0);

        let first_hour_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 11, 0, 0)
            .single()
            .unwrap();
        let first_hour_bar = bar(
            first_hour_dt.with_timezone(&Utc).timestamp(),
            100.0,
            101.0,
            99.5,
            101.5,
        );
        strategy.update_session(first_hour_dt, &first_hour_bar);
        assert_eq!(strategy.first_hour_price, Some(101.5));

        let later_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 30, 0)
            .single()
            .unwrap();
        let later_bar = bar(
            later_dt.with_timezone(&Utc).timestamp(),
            100.5,
            103.0,
            100.0,
            103.2,
        );
        strategy.update_session(later_dt, &later_bar);

        assert_eq!(strategy.first_hour_price, Some(101.5));
    }

    #[test]
    fn runtime_state_restored_applies_once_and_on_bar_does_not_override_rollover() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();

        let session_start = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 0, 0)
            .single()
            .unwrap();
        let session_end = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
            .single()
            .unwrap();
        let last_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 10, 1, 0)
            .single()
            .unwrap();

        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(105.0),
            yesterday_range: Some(8.0),
            pre_prev_close: Some(98.0),
            first_min_high: Some(102.0),
            first_min_low: Some(99.0),
            first_hour_price: Some(101.0),
            session_start_ts_utc: Some(session_start.with_timezone(&Utc).timestamp()),
            session_end_ts_utc: Some(session_end.with_timezone(&Utc).timestamp()),
            session_high: Some(120.0),
            session_low: Some(80.0),
            session_close: Some(110.0),
            last_dt_ts_utc: Some(last_dt.with_timezone(&Utc).timestamp()),
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: Some(last_dt.with_timezone(&Utc).timestamp()),
            last_bar_ts: Some(last_dt.with_timezone(&Utc).timestamp()),
        };

        let restored = crate::RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: Vec::new(),
        };
        let _ = strategy.on_runtime_state_restored(&ctx, &restored);

        let rollover_bar_ts = (last_dt + Duration::minutes(120))
            .with_timezone(&Utc)
            .timestamp();
        let mut rollover_bar = bar(rollover_bar_ts, 130.0, 131.0, 129.0, 130.5);
        rollover_bar.origin = DataOrigin::Live;
        let _ = strategy.on_bar(&ctx, &rollover_bar);

        match &strategy.state {
            StrategyState::SessionGapStandalone {
                prev_close,
                yesterday_range,
                pre_prev_close,
                first_hour_price,
                ..
            } => {
                assert_eq!(*prev_close, Some(110.0));
                assert_eq!(*yesterday_range, Some(40.0));
                assert_eq!(*pre_prev_close, Some(105.0));
                assert_eq!(*first_hour_price, None);
            }
            other => panic!("unexpected state after restore+rollover: {other:?}"),
        }
    }

    #[test]
    fn duplicate_bar_is_ignored_using_last_processed_bar_ts() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let t = 1_733_863_740;
        strategy.last_processed_bar_ts = Some(t);
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: false,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: Some(t - 60),
            last_bar_ts: Some(t),
        };
        let before_state = strategy.state.clone();

        let intents_same = strategy.on_bar(&ctx_backtest(), &bar(t, 101.0, 102.0, 100.0, 101.5));
        let intents_older =
            strategy.on_bar(&ctx_backtest(), &bar(t - 60, 101.0, 102.0, 100.0, 101.5));

        assert!(intents_same.is_empty());
        assert!(intents_older.is_empty());
        assert_eq!(strategy.last_processed_bar_ts, Some(t));
        assert_eq!(
            format!("{:?}", strategy.state),
            format!("{:?}", before_state)
        );
    }

    #[test]
    fn restored_last_bar_ts_prevents_reprocessing_last_bar() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let t = 1_733_863_740;
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::Flat,
            phase_last_change_ts_utc: Some(t - 60),
            last_bar_ts: Some(t),
        };
        let restored = crate::RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: Vec::new(),
        };
        let _ = strategy.on_runtime_state_restored(&ctx_backtest(), &restored);
        let before_state = strategy.state.clone();

        let intents = strategy.on_bar(&ctx_backtest(), &bar(t, 101.0, 102.0, 100.0, 101.5));

        assert!(intents.is_empty());
        assert_eq!(strategy.last_processed_bar_ts, Some(t));
        assert_eq!(
            format!("{:?}", strategy.state),
            format!("{:?}", before_state)
        );
    }

    #[test]
    fn tp_and_session_end_same_bar_priority_is_deterministic() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let session_end = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
            .single()
            .unwrap();
        let bar_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 30, 0)
            .single()
            .unwrap();

        strategy.session_end_dt = Some(session_end);
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: Some(session_end.with_timezone(&Utc).timestamp()),
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::InPosition {
                side: Side::Buy,
                qty: 1.0,
                avg_price: 100.0,
                baseline_qty: 0.0,
                tp: Some(102.0),
                sl: Some(95.0),
                opened_ts: bar_dt.with_timezone(&Utc).timestamp() - 60,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(bar_dt.with_timezone(&Utc).timestamp() - 60),
        };
        let mut conflict_bar = bar(
            bar_dt.with_timezone(&Utc).timestamp(),
            101.0,
            103.0,
            100.0,
            102.5,
        );
        conflict_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &conflict_bar);

        assert_eq!(intents.len(), 1);
        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingExit { reason, .. },
                ..
            } => assert_eq!(reason, "session_exit"),
            other => panic!("unexpected phase after conflict bar: {other:?}"),
        }
    }

    #[test]
    fn session_end_forces_exit_once_and_sets_reason() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_live(true, crate::live_guard::GatewayPhase::LiveReady);
        let offset = FixedOffset::east_opt(3 * 3600).unwrap();
        let session_end = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 49, 0)
            .single()
            .unwrap();
        let bar_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 23, 30, 0)
            .single()
            .unwrap();

        strategy.session_end_dt = Some(session_end);
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: Some(session_end.with_timezone(&Utc).timestamp()),
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::InPosition {
                side: Side::Buy,
                qty: 1.0,
                avg_price: 100.0,
                baseline_qty: 0.0,
                tp: None,
                sl: None,
                opened_ts: bar_dt.with_timezone(&Utc).timestamp() - 60,
            },
            phase_last_change_ts_utc: None,
            last_bar_ts: Some(bar_dt.with_timezone(&Utc).timestamp() - 60),
        };
        let mut exit_bar = bar(
            bar_dt.with_timezone(&Utc).timestamp(),
            101.0,
            101.5,
            100.5,
            101.2,
        );
        exit_bar.origin = DataOrigin::Live;

        let intents = strategy.on_bar(&ctx, &exit_bar);

        assert_eq!(intents.len(), 1);
        assert_exit_place_intent(&intents[0], Side::Sell, 1.0);
        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::PendingExit { reason, .. },
                ..
            } => assert_eq!(reason, "session_exit"),
            other => panic!("unexpected phase after forced session exit: {other:?}"),
        }
    }

    #[test]
    fn exit_recycle_failure_enters_exit_recovery_pending_and_retries_once() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::PendingExit {
                request_id,
                side: Side::Sell,
                qty: 1.0,
                price: 81.2,
                baseline_qty: 0.0,
                reason: "session_exit".into(),
                sent_ts: 1_000,
                acked: false,
            },
            phase_last_change_ts_utc: Some(1_000),
            last_bar_ts: Some(1_000),
        };

        let mut ack = CommandAck::error(
            request_id,
            "control_path_recycle_failed",
            "fresh cws session was not ready before recycle timeout",
        );
        ack.processed_ts_utc = 1_005;

        let intents = strategy.on_ack(&ctx_live_at(1_005, Some(1_000), 1.0), &ack);

        assert_eq!(intents.len(), 1);
        assert_exit_place_intent(&intents[0], Side::Sell, 1.0);
        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase:
                    SessionGapLivePhase::ExitRecoveryPending {
                        request_id: retry_request_id,
                        side,
                        qty,
                        price,
                        baseline_qty,
                        reason,
                        sent_ts,
                        acked,
                        retry_attempt,
                        last_error_code,
                        last_error_msg,
                    },
                phase_last_change_ts_utc,
                ..
            } => {
                assert_ne!(*retry_request_id, request_id);
                assert_eq!(*side, Side::Sell);
                assert_eq!(*qty, 1.0);
                assert_eq!(*price, 81.2);
                assert_eq!(*baseline_qty, 0.0);
                assert_eq!(reason, "session_exit");
                assert_eq!(*sent_ts, 1_005);
                assert!(!acked);
                assert_eq!(*retry_attempt, 1);
                assert_eq!(
                    last_error_code.as_deref(),
                    Some("control_path_recycle_failed")
                );
                assert_eq!(
                    last_error_msg.as_deref(),
                    Some("fresh cws session was not ready before recycle timeout")
                );
                assert_eq!(*phase_last_change_ts_utc, Some(1_005));
            }
            other => panic!("unexpected state after exit recycle failure: {other:?}"),
        }
    }

    #[test]
    fn exit_recovery_failure_enters_close_only_degraded_and_operator_required() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let request_id = uuid::Uuid::new_v4();
        strategy.state = StrategyState::SessionGapStandalone {
            session_date: Some("2025-12-05".into()),
            traded_session: true,
            prev_close: Some(100.0),
            yesterday_range: Some(2.0),
            pre_prev_close: Some(99.0),
            first_min_high: Some(100.0),
            first_min_low: Some(90.0),
            first_hour_price: Some(100.0),
            session_start_ts_utc: None,
            session_end_ts_utc: None,
            session_high: None,
            session_low: None,
            session_close: None,
            last_dt_ts_utc: None,
            phase: SessionGapLivePhase::ExitRecoveryPending {
                request_id,
                side: Side::Sell,
                qty: 1.0,
                price: 81.2,
                baseline_qty: 0.0,
                reason: "session_exit".into(),
                sent_ts: 1_005,
                acked: false,
                retry_attempt: 1,
                last_error_code: Some("control_path_recycle_failed".into()),
                last_error_msg: Some(
                    "fresh cws session was not ready before recycle timeout".into(),
                ),
            },
            phase_last_change_ts_utc: Some(1_005),
            last_bar_ts: Some(1_000),
        };

        let mut ack = CommandAck::error(
            request_id,
            "control_path_recycle_failed",
            "fresh cws session was not ready before recycle timeout",
        );
        ack.processed_ts_utc = 1_010;

        let intents = strategy.on_ack(&ctx_live_at(1_010, Some(1_000), 1.0), &ack);

        assert!(intents.is_empty());
        match &strategy.state {
            StrategyState::SessionGapStandalone {
                phase:
                    SessionGapLivePhase::CloseOnlyDegraded {
                        side,
                        qty,
                        baseline_qty,
                        reason,
                        entered_ts_utc,
                        retry_attempts_exhausted,
                        last_error_code,
                        last_error_msg,
                        operator_intervention_required,
                    },
                phase_last_change_ts_utc,
                ..
            } => {
                assert_eq!(*side, Side::Sell);
                assert_eq!(*qty, 1.0);
                assert_eq!(*baseline_qty, 0.0);
                assert!(reason.contains("ack_failed:Error"));
                assert!(reason.contains("session_exit"));
                assert_eq!(*entered_ts_utc, 1_010);
                assert_eq!(*retry_attempts_exhausted, 1);
                assert_eq!(
                    last_error_code.as_deref(),
                    Some("control_path_recycle_failed")
                );
                assert_eq!(
                    last_error_msg.as_deref(),
                    Some("fresh cws session was not ready before recycle timeout")
                );
                assert!(*operator_intervention_required);
                assert_eq!(*phase_last_change_ts_utc, Some(1_010));
            }
            other => panic!("unexpected state after exhausted exit recovery: {other:?}"),
        }
    }

    #[derive(Debug, Deserialize)]
    struct IndicatorBarCsvRow {
        time: String,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    }

    #[derive(Debug, Deserialize)]
    struct IndicatorReferenceCsvRow {
        session_date: String,
        traded_session: bool,
        prev_close: Option<f64>,
        yesterday_range: Option<f64>,
        pre_prev_close: Option<f64>,
        first_min_high: Option<f64>,
        first_min_low: Option<f64>,
        first_hour_price: Option<f64>,
        session_start_ts_utc: Option<i64>,
        session_end_ts_utc: Option<i64>,
        session_high: Option<f64>,
        session_low: Option<f64>,
        session_close: Option<f64>,
    }

    #[derive(Debug, Clone)]
    struct SessionSnapshot {
        traded_session: bool,
        prev_close: Option<f64>,
        yesterday_range: Option<f64>,
        pre_prev_close: Option<f64>,
        first_min_high: Option<f64>,
        first_min_low: Option<f64>,
        first_hour_price: Option<f64>,
        session_start_ts_utc: Option<i64>,
        session_end_ts_utc: Option<i64>,
        session_high: Option<f64>,
        session_low: Option<f64>,
        session_close: Option<f64>,
    }

    #[test]
    fn indicators_match_reference_for_paper_bars_3_stream() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
        let ctx = ctx_backtest();
        let mut snapshots: BTreeMap<String, SessionSnapshot> = BTreeMap::new();

        let bars_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../data_samples/paper_bars_3.csv"
        );
        let mut bars_reader = csv::Reader::from_path(bars_path).expect("open paper bars csv");
        for row in bars_reader.deserialize::<IndicatorBarCsvRow>() {
            let row = row.expect("valid paper bars row");
            let dt = DateTime::parse_from_rfc3339(&row.time).expect("rfc3339 bar time");
            let bar = bar(
                dt.with_timezone(&Utc).timestamp(),
                row.open,
                row.high,
                row.low,
                row.close,
            );
            let _ = strategy.on_bar(&ctx, &bar);

            if let StrategyState::SessionGapStandalone {
                session_date: Some(session_date),
                traded_session,
                prev_close,
                yesterday_range,
                pre_prev_close,
                first_min_high,
                first_min_low,
                first_hour_price,
                session_start_ts_utc,
                session_end_ts_utc,
                session_high,
                session_low,
                session_close,
                ..
            } = &strategy.state
            {
                snapshots.insert(
                    session_date.clone(),
                    SessionSnapshot {
                        traded_session: *traded_session,
                        prev_close: *prev_close,
                        yesterday_range: *yesterday_range,
                        pre_prev_close: *pre_prev_close,
                        first_min_high: *first_min_high,
                        first_min_low: *first_min_low,
                        first_hour_price: *first_hour_price,
                        session_start_ts_utc: *session_start_ts_utc,
                        session_end_ts_utc: *session_end_ts_utc,
                        session_high: *session_high,
                        session_low: *session_low,
                        session_close: *session_close,
                    },
                );
            }
        }

        let indicators_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../data_samples/paper_indicators_3.csv"
        );
        let mut indicators_reader =
            csv::Reader::from_path(indicators_path).expect("open indicators csv");

        let approx_eq = |left: Option<f64>, right: Option<f64>, label: &str, session_date: &str| {
            match (left, right) {
                (None, None) => {}
                (Some(left), Some(right)) => {
                    let diff = (left - right).abs();
                    assert!(
                        diff <= 1e-9,
                        "{label} mismatch for {session_date}: left={left}, right={right}, diff={diff}"
                    );
                }
                (left, right) => {
                    panic!("{label} mismatch for {session_date}: left={left:?}, right={right:?}")
                }
            }
        };

        for row in indicators_reader.deserialize::<IndicatorReferenceCsvRow>() {
            let row = row.expect("valid indicators row");
            let snapshot = snapshots
                .get(&row.session_date)
                .unwrap_or_else(|| panic!("missing snapshot for session {}", row.session_date));

            assert_eq!(snapshot.traded_session, row.traded_session);
            assert_eq!(snapshot.session_start_ts_utc, row.session_start_ts_utc);
            assert_eq!(snapshot.session_end_ts_utc, row.session_end_ts_utc);

            approx_eq(
                snapshot.prev_close,
                row.prev_close,
                "prev_close",
                &row.session_date,
            );
            approx_eq(
                snapshot.yesterday_range,
                row.yesterday_range,
                "yesterday_range",
                &row.session_date,
            );
            approx_eq(
                snapshot.pre_prev_close,
                row.pre_prev_close,
                "pre_prev_close",
                &row.session_date,
            );
            approx_eq(
                snapshot.first_min_high,
                row.first_min_high,
                "first_min_high",
                &row.session_date,
            );
            approx_eq(
                snapshot.first_min_low,
                row.first_min_low,
                "first_min_low",
                &row.session_date,
            );
            approx_eq(
                snapshot.first_hour_price,
                row.first_hour_price,
                "first_hour_price",
                &row.session_date,
            );
            approx_eq(
                snapshot.session_high,
                row.session_high,
                "session_high",
                &row.session_date,
            );
            approx_eq(
                snapshot.session_low,
                row.session_low,
                "session_low",
                &row.session_date,
            );
            approx_eq(
                snapshot.session_close,
                row.session_close,
                "session_close",
                &row.session_date,
            );
        }
    }
}
