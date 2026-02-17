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
                    self.pre_prev_close = self.yesterday_close;
                    self.yesterday_close = self.session_close;
                    self.yesterday_range = match (self.session_high, self.session_low) {
                        (Some(high), Some(low)) => Some(high - low),
                        _ => None,
                    };
                    self.reset_session(bar_dt, bar);
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
        let phase = match &self.state {
            StrategyState::SessionGapStandalone { phase, .. } => phase.clone(),
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
                phase,
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
            return Vec::new();
        }

        self.update_session(bar_dt, bar);

        if ctx.trade_mode != TradeMode::Live {
            let mut intents = Vec::new();
            if let Some(entry) = self.maybe_open_position(bar) {
                intents.push(entry);
            }
            if let Some(exit) = self.maybe_close_position(bar_dt, bar) {
                intents.push(exit);
            }
            self.maybe_generate_signal(bar_dt, bar);
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return intents;
        }

        if self.should_block_live(ctx, bar) {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        }

        let mut traded_session = self.traded_session;
        let mut phase = SessionGapLivePhase::Flat;
        if let StrategyState::SessionGapStandalone {
            session_date,
            traded_session: persisted_traded,
            prev_close,
            yesterday_range,
            phase: persisted_phase,
            ..
        } = &self.state
        {
            traded_session = if session_date.as_deref() == Some(current_session_date.as_str()) {
                *persisted_traded
            } else {
                false
            };
            self.yesterday_close = *prev_close;
            self.yesterday_range = *yesterday_range;
            phase = persisted_phase.clone();
        }

        self.traded_session = traded_session;
        self.maybe_generate_signal(bar_dt, bar);

        let mut intents = Vec::new();
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
            SessionGapLivePhase::PendingEntry {
                sent_ts,
                acked,
                ..
            } => {
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
                    let exit_threshold = session_end_dt - Duration::minutes(self.config.exit_offset_min);
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
            SessionGapLivePhase::PendingExit {
                sent_ts,
                acked,
                ..
            } => {
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

        self.state = StrategyState::SessionGapStandalone {
            session_date: Some(current_session_date),
            traded_session: self.traded_session,
            prev_close: self.yesterday_close,
            yesterday_range: self.yesterday_range,
            phase: next_phase,
            last_bar_ts: Some(bar.close_time_utc),
        };

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        if let StrategyState::SessionGapStandalone { phase, .. } = &mut self.state {
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
        if let StrategyState::SessionGapStandalone { phase, .. } = &mut self.state {
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
                            side: if pos.qty >= 0.0 { Side::Buy } else { Side::Sell },
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
        }
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        ctx: &StrategyCtx,
        snapshot: &crate::BootstrapSnapshot,
    ) -> Vec<Intent> {
        let snapshot_qty = snapshot
            .positions_strategy
            .get(&self.config.symbol)
            .map(|p| p.qty)
            .unwrap_or(0.0);
        let ts = snapshot.snapshot_ts_utc.unwrap_or(0);
        self.transition_live_reconcile_with_snapshot(ctx, snapshot_qty, ts);
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

    fn ctx_live(allow_live_orders: bool, gateway_phase: crate::live_guard::GatewayPhase) -> StrategyCtx {
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
            phase: SessionGapLivePhase::Flat,
            last_bar_ts: None,
        };
        strategy.yesterday_range = Some(2.0);

        let mut b = bar(1_733_386_740, 101.0, 101.0, 100.0, 101.0);
        b.origin = DataOrigin::Replay;
        let intents = strategy.on_bar(&ctx_live(true, crate::live_guard::GatewayPhase::LiveReady), &b);

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
            last_bar_ts: Some(10),
        };
        let mut b = bar(12, 101.0, 102.0, 100.0, 101.0);
        b.origin = DataOrigin::Live;

        let _ = strategy.on_bar(&ctx_live(true, crate::live_guard::GatewayPhase::LiveReady), &b);
        assert!(matches!(
            strategy.state,
            StrategyState::SessionGapStandalone {
                phase: SessionGapLivePhase::Blocked { .. },
                ..
            }
        ));
    }
}
