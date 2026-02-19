use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Timelike, Utc};

use alor_protocol::{AckStatus, CommandAck, Side};
use tracing::{info, warn};

use crate::live_guard::GatewayPhase;
use crate::state::{SessionGapLivePhase, StrategyState};
use crate::{BarEvent, Intent, PositionEvent, Strategy, StrategyCtx, TradeMode};

#[derive(Debug, Clone)]
pub struct SessionGapStandaloneConfig {
    pub symbol: String,
    pub timezone_offset_hours: i32,
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
}

impl Default for SessionGapStandaloneConfig {
    fn default() -> Self {
        Self {
            symbol: "USDRUBF".to_string(),
            timezone_offset_hours: 3,
            k_long: 0.5,
            k_short: 0.46,
            wait_hours: 2,
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
}

impl SessionGapStandaloneStrategy {
    pub fn new(config: SessionGapStandaloneConfig) -> Self {
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
        })
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
        })
    }

    fn maybe_generate_signal(&mut self, bar_dt: DateTime<FixedOffset>, bar: &BarEvent) {
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
            _ => return,
        };

        if bar_dt.minute() == 59
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
                let available_cash = self.config.cash_factor * self.cash;
                let mut size = (available_cash / bar.close).floor() as i64;
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

    fn transition_live_reconcile_with_snapshot(
        &mut self,
        ctx: &StrategyCtx,
        snapshot_qty: f64,
        ts_utc: i64,
    ) {
        let (
            phase,
            persisted_session_start_ts_utc,
            persisted_session_end_ts_utc,
            persisted_last_dt_ts_utc,
        ) = match &self.state {
            StrategyState::SessionGapStandalone {
                phase,
                session_start_ts_utc,
                session_end_ts_utc,
                last_dt_ts_utc,
                ..
            } => (
                phase.clone(),
                *session_start_ts_utc,
                *session_end_ts_utc,
                *last_dt_ts_utc,
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
            SessionGapLivePhase::InPosition { baseline_qty, .. }
            | SessionGapLivePhase::PendingExit { baseline_qty, .. } => {
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
                    .last_dt
                    .map(|dt| dt.with_timezone(&chrono::Utc).timestamp())
                    .or(persisted_last_dt_ts_utc),
                phase,
                phase_last_change_ts_utc: self.phase_last_change_ts_utc,
                last_bar_ts: Some(ts_utc),
            };
            let _ = ctx;
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
            SessionGapLivePhase::InPosition { .. } => "InPosition",
            SessionGapLivePhase::PendingExit { .. } => "PendingExit",
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
            last_dt_ts_utc: self
                .last_dt
                .map(|dt| dt.with_timezone(&chrono::Utc).timestamp()),
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
                    .last_dt
                    .map(|dt| dt.with_timezone(&chrono::Utc).timestamp()),
                phase,
                phase_last_change_ts_utc: self.phase_last_change_ts_utc,
                last_bar_ts: *last_bar_ts,
            };
        }
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

        if ctx.trade_mode != TradeMode::Live {
            let phase = self.persisted_phase_or_flat();
            let mut intents = Vec::new();
            if let Some(entry) = self.maybe_open_position(bar) {
                intents.push(entry);
            }
            if let Some(exit) = self.maybe_close_position(bar_dt, bar) {
                intents.push(exit);
            }
            self.maybe_generate_signal(bar_dt, bar);
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

        let phase = self.persisted_phase_or_flat();
        self.maybe_generate_signal(bar_dt, bar);

        let mut intents = Vec::new();
        let previous_phase = phase.clone();
        let next_phase = match phase {
            SessionGapLivePhase::Flat => {
                if let Some(pending) = self.pending_entry.take() {
                    let side = Self::entry_side(pending.direction);
                    let (tp, sl) = self.compute_tp_sl(pending.direction, bar.o);
                    let qty = pending.size as f64;
                    intents.push(Intent::Market {
                        qty,
                        side,
                        fill_price: None,
                    });
                    SessionGapLivePhase::PendingEntry {
                        request_id: crate::deterministic_market_request_id(
                            &ctx.strategy_id,
                            &ctx.portfolio,
                            &bar.symbol,
                            bar.close_time_utc,
                            side,
                        ),
                        side,
                        qty,
                        baseline_qty: ctx.position_qty.unwrap_or(0.0),
                        tp,
                        sl,
                        sent_ts: bar.close_time_utc,
                        acked: false,
                    }
                } else {
                    SessionGapLivePhase::Flat
                }
            }
            SessionGapLivePhase::PendingEntry { sent_ts, acked, .. } => {
                let elapsed = (bar.close_time_utc - sent_ts).saturating_mul(1000) as u64;
                if !acked && elapsed > self.config.entry_ack_timeout_ms {
                    SessionGapLivePhase::Blocked {
                        reason: "entry_ack_timeout".to_string(),
                        ts_utc: bar.close_time_utc,
                    }
                } else if acked && elapsed > self.config.entry_fill_timeout_ms {
                    SessionGapLivePhase::Blocked {
                        reason: "entry_fill_timeout".to_string(),
                        ts_utc: bar.close_time_utc,
                    }
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
                if let Some(session_end_dt) = self.session_end_dt {
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
                    intents.push(Intent::Market {
                        qty,
                        side: exit_side,
                        fill_price: None,
                    });
                    SessionGapLivePhase::PendingExit {
                        request_id: crate::deterministic_market_request_id(
                            &ctx.strategy_id,
                            &ctx.portfolio,
                            &bar.symbol,
                            bar.close_time_utc,
                            exit_side,
                        ),
                        side: exit_side,
                        qty,
                        baseline_qty,
                        reason: reason.to_string(),
                        sent_ts: bar.close_time_utc,
                        acked: false,
                    }
                } else {
                    phase
                }
            }
            SessionGapLivePhase::PendingExit { sent_ts, acked, .. } => {
                let elapsed = (bar.close_time_utc - sent_ts).saturating_mul(1000) as u64;
                if !acked && elapsed > self.config.exit_ack_timeout_ms {
                    SessionGapLivePhase::Blocked {
                        reason: "exit_ack_timeout".to_string(),
                        ts_utc: bar.close_time_utc,
                    }
                } else if acked && elapsed > self.config.exit_fill_timeout_ms {
                    SessionGapLivePhase::Blocked {
                        reason: "exit_fill_timeout".to_string(),
                        ts_utc: bar.close_time_utc,
                    }
                } else {
                    phase
                }
            }
            SessionGapLivePhase::Blocked { .. } => phase,
        };

        Self::log_phase_transition(&previous_phase, &next_phase, bar.close_time_utc);
        if Self::phase_name(&previous_phase) != Self::phase_name(&next_phase) {
            self.phase_last_change_ts_utc = Some(bar.close_time_utc);
        }
        self.persist_state_snapshot(current_session_date, next_phase, bar.close_time_utc);

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        let mut phase_to_persist: Option<SessionGapLivePhase> = None;
        if let StrategyState::SessionGapStandalone { phase, .. } = &mut self.state {
            let previous_phase = phase.clone();
            match phase {
                SessionGapLivePhase::PendingEntry {
                    request_id, acked, ..
                }
                | SessionGapLivePhase::PendingExit {
                    request_id, acked, ..
                } => {
                    if *request_id == ack.request_id {
                        match ack.status {
                            AckStatus::Accepted | AckStatus::Confirmed | AckStatus::Duplicate => {
                                *acked = true;
                            }
                            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                                *phase = SessionGapLivePhase::Blocked {
                                    reason: format!("ack_failed:{:?}", ack.status),
                                    ts_utc: ack.processed_ts_utc,
                                };
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
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &crate::OrderEvent) -> Vec<Intent> {
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
                SessionGapLivePhase::InPosition { baseline_qty, .. }
                | SessionGapLivePhase::PendingExit { baseline_qty, .. } => {
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
        ctx: &StrategyCtx,
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
        let ts = snapshot.snapshot_ts_utc.unwrap_or(0);
        self.transition_live_reconcile_with_snapshot(ctx, snapshot_qty, ts);
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

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BarEvent, DataOrigin};

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
            allow_live_orders,
            gateway_phase,
            position_qty: Some(0.0),
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
            allow_live_orders: false,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty: None,
            last_bar_ts: None,
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

    #[test]
    fn emits_market_entry_after_pending_signal() {
        let mut strategy = SessionGapStandaloneStrategy::new(SessionGapStandaloneConfig::default());
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

        let _ = strategy.on_bar(
            &ctx_live(true, crate::live_guard::GatewayPhase::LiveReady),
            &b,
        );
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Blocked { .. },
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
        assert!(matches!(
            intents[0],
            Intent::Market {
                side: Side::Sell,
                qty: 1.0,
                ..
            }
        ));

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
        assert!(matches!(
            intents[0],
            Intent::Market {
                side: Side::Sell,
                qty: 1.0,
                ..
            }
        ));
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
        let first_hour_bar = bar(first_hour_dt.with_timezone(&Utc).timestamp(), 100.0, 101.0, 99.5, 101.5);
        strategy.update_session(first_hour_dt, &first_hour_bar);
        assert_eq!(strategy.first_hour_price, Some(101.5));

        let later_dt = offset
            .with_ymd_and_hms(2025, 12, 5, 12, 30, 0)
            .single()
            .unwrap();
        let later_bar = bar(later_dt.with_timezone(&Utc).timestamp(), 100.5, 103.0, 100.0, 103.2);
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
}
