use alor_protocol::{CommandAck, Side};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};

use crate::state::StrategyState;
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, Intent, OrderEvent, PositionEvent, RuntimeStateRestored, Strategy,
    StrategyCtx, StopOrderEvent,
};

#[derive(Debug, Clone)]
pub struct AlorUsdrubfHybridConfig {
    pub symbol: String,
    pub timezone_offset_hours: i32,
    pub tick_size: f64,
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

#[derive(Debug)]
pub struct AlorUsdrubfHybridStrategy {
    config: AlorUsdrubfHybridConfig,
    state: StrategyState,
    lifecycle_stage: String,
    last_bar_ts: Option<i64>,
    bootstrap_seen: bool,
    runtime_state_restored: bool,
    hybrid_state: HybridState,
    current_date_local: Option<NaiveDate>,
    day_open: Option<f64>,
    day_high: Option<f64>,
    day_low: Option<f64>,
    day_volume_sum: f64,
    day_vwap_num: f64,
    session_start_local: Option<NaiveDateTime>,
    pending_entry: Option<PendingEntry>,
    open_position: Option<OpenPosition>,
    cash: f64,
    bo_was_long_today: bool,
    bo_was_short_today: bool,
}

impl AlorUsdrubfHybridStrategy {
    pub fn new(config: AlorUsdrubfHybridConfig) -> Self {
        let mut strategy = Self {
            config,
            state: StrategyState::AlorUsdrubfHybrid {
                lifecycle_stage: "created".to_string(),
                last_bar_ts: None,
                bootstrap_seen: false,
                runtime_state_restored: false,
                hybrid_state: "flat".to_string(),
                current_date_local: None,
                cash: 0.0,
                pending_entry_side: None,
                open_position_side: None,
                open_position_qty: 0.0,
            },
            lifecycle_stage: "created".to_string(),
            last_bar_ts: None,
            bootstrap_seen: false,
            runtime_state_restored: false,
            hybrid_state: HybridState::Flat,
            current_date_local: None,
            day_open: None,
            day_high: None,
            day_low: None,
            day_volume_sum: 0.0,
            day_vwap_num: 0.0,
            session_start_local: None,
            pending_entry: None,
            open_position: None,
            cash: 0.0,
            bo_was_long_today: false,
            bo_was_short_today: false,
        };
        strategy.cash = strategy.config.initial_cash;
        strategy.sync_state();
        strategy
    }

    fn sync_state(&mut self) {
        self.state = StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage: self.lifecycle_stage.clone(),
            last_bar_ts: self.last_bar_ts,
            bootstrap_seen: self.bootstrap_seen,
            runtime_state_restored: self.runtime_state_restored,
            hybrid_state: self.hybrid_state.as_str().to_string(),
            current_date_local: self.current_date_local.map(|d| d.to_string()),
            cash: self.cash,
            pending_entry_side: self
                .pending_entry
                .as_ref()
                .map(|entry| entry.side.as_str().to_string()),
            open_position_side: self
                .open_position
                .as_ref()
                .map(|position| position.side.as_str().to_string()),
            open_position_qty: self
                .open_position
                .as_ref()
                .map(|position| position.size as f64)
                .unwrap_or(0.0),
        };
    }

    fn update_session_metrics(&mut self, bar: &BarEvent, local_dt: NaiveDateTime) {
        self.day_open.get_or_insert(bar.o);
        self.day_high = Some(self.day_high.unwrap_or(bar.h).max(bar.h));
        self.day_low = Some(self.day_low.unwrap_or(bar.l).min(bar.l));
        self.day_volume_sum += bar.v.max(0.0);
        self.day_vwap_num += bar.close * bar.v.max(0.0);
        self.session_start_local.get_or_insert(local_dt);
    }

    fn reset_day(&mut self, local_date: NaiveDate) {
        self.current_date_local = Some(local_date);
        self.day_open = None;
        self.day_high = None;
        self.day_low = None;
        self.day_volume_sum = 0.0;
        self.day_vwap_num = 0.0;
        self.session_start_local = None;
        self.pending_entry = None;
        self.open_position = None;
        self.hybrid_state = HybridState::Flat;
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

    fn maybe_fill_pending_entry(&mut self, bar: &BarEvent, intents: &mut Vec<Intent>) {
        let Some(pending) = self.pending_entry.clone() else {
            return;
        };
        let size = if self.config.use_fixed_live_size {
            self.config.live_fixed_units.max(1.0).floor() as i64
        } else {
            ((self.cash * self.config.position_size_fraction) / bar.o)
                .floor()
                .max(1.0) as i64
        };
        let (stop_price, take_price) = if pending.owner == Owner::MeanRev {
            (
                Some(self.round_to_tick(
                    bar.o + self.config.mr_stop_k_short * pending.scale_at_signal,
                    self.config.tick_size,
                )),
                Some(self.round_to_tick(
                    bar.o - self.config.mr_take_k_short * pending.scale_at_signal,
                    self.config.tick_size,
                )),
            )
        } else {
            (None, None)
        };

        intents.push(Intent::Market {
            qty: size as f64,
            side: pending.side.entry_side(),
            fill_price: Some(bar.o),
            comment: Some(format!("{}|entry|{}", self.config.symbol, pending.owner.as_str())),
        });
        self.open_position = Some(OpenPosition {
            owner: pending.owner,
            side: pending.side,
            entry_ts: utc_to_local(bar.close_time_utc, self.config.timezone_offset_hours),
            entry_price: bar.o,
            size,
            stop_price,
            take_price,
            stop1: pending.stop1,
            stop2: pending.stop2,
        });
        self.pending_entry = None;
        self.hybrid_state = HybridState::Open;
    }

    fn evaluate_mr_signal(&self, bar: &BarEvent, local_dt: NaiveDateTime) -> Option<PendingEntry> {
        if local_dt.time() > self.config.mr_last_entry_time {
            return None;
        }
        let scale = self.session_range()?;
        if !scale.is_finite() || scale <= 0.0 || bar.close.abs() <= f64::EPSILON {
            return None;
        }
        let session_vwap = self.session_vwap(bar.close);
        let rel_scale = scale / bar.close;
        let dist = bar.close - session_vwap;
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
            signal_price: bar.close,
            stop1: None,
            stop2: None,
        })
    }

    fn evaluate_bo_signal(&self, bar: &BarEvent, local_dt: NaiveDateTime) -> Option<PendingEntry> {
        let _ = local_dt;
        let scale = self.session_range()?;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        if self.elapsed_hours(local_dt)? < self.config.bo_wait_hours {
            return None;
        }
        let session_open = self.day_open?;
        let ret_from_open = self.ret_from_open(bar.close)?;
        let can_long = ret_from_open >= -self.config.bo_big_move_threshold;
        let can_short = ret_from_open <= self.config.bo_big_move_threshold;
        let long_level = session_open + self.config.bo_k * scale;
        let short_level = session_open - self.config.bo_k * scale;

        if can_long && !self.bo_was_long_today && bar.close > long_level {
            return Some(PendingEntry {
                owner: Owner::Breakout,
                side: PositionSide::Long,
                reason: "bo_long_signal".to_string(),
                scale_at_signal: scale,
                signal_price: bar.close,
                stop1: Some(session_open + self.config.bo_stop1_range * scale),
                stop2: Some(session_open - self.config.bo_stop2_range * scale),
            });
        }
        if can_short && !self.bo_was_short_today && bar.close < short_level {
            return Some(PendingEntry {
                owner: Owner::Breakout,
                side: PositionSide::Short,
                reason: "bo_short_signal".to_string(),
                scale_at_signal: scale,
                signal_price: bar.close,
                stop1: Some(session_open - self.config.bo_stop1_range * scale),
                stop2: Some(session_open + self.config.bo_stop2_range * scale),
            });
        }
        None
    }

    fn evaluate_exit(&self, bar: &BarEvent, local_dt: NaiveDateTime) -> Option<(String, f64)> {
        let pos = self.open_position.as_ref()?;
        if pos.owner == Owner::MeanRev {
            let stop_price = pos.stop_price?;
            let take_price = pos.take_price?;
            if bar.h >= stop_price {
                return Some(("mr_stop".to_string(), stop_price));
            }
            if bar.l <= take_price {
                return Some(("mr_take".to_string(), take_price));
            }
            if local_dt.time() >= self.config.mr_force_exit_time {
                return Some(("mr_time_cutoff".to_string(), bar.close));
            }
            return None;
        }

        let stop1 = pos.stop1?;
        let stop2 = pos.stop2?;
        if pos.side == PositionSide::Long {
            if bar.l <= stop2 {
                return Some(("bo_stop2_long".to_string(), stop2));
            }
            if local_dt.minute() == 50 && bar.close < stop1 {
                return Some(("bo_stop1_long".to_string(), bar.close));
            }
            if local_dt.time() >= self.config.bo_eod_exit_time {
                return Some(("bo_eod_exit".to_string(), bar.close));
            }
            return None;
        }

        if bar.h >= stop2 {
            return Some(("bo_stop2_short".to_string(), stop2));
        }
        if local_dt.minute() == 50 && bar.close > stop1 {
            return Some(("bo_stop1_short".to_string(), bar.close));
        }
        if local_dt.time() >= self.config.bo_eod_exit_time {
            return Some(("bo_eod_exit".to_string(), bar.close));
        }
        None
    }

    fn apply_exit(&mut self, reason: String, exit_price: f64, bar: &BarEvent, intents: &mut Vec<Intent>) {
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
        self.hybrid_state = HybridState::Flat;
        let _ = bar;
    }
}

impl Strategy for AlorUsdrubfHybridStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            return Vec::new();
        }
        self.lifecycle_stage = "live".to_string();
        self.last_bar_ts = Some(bar.close_time_utc);

        if matches!(ctx.trade_mode, crate::TradeMode::Live) && !self.config.enable_live_execution {
            self.sync_state();
            return Vec::new();
        }

        let local_dt = utc_to_local(bar.close_time_utc, self.config.timezone_offset_hours);
        let local_date = local_dt.date();
        if self.current_date_local != Some(local_date) {
            self.reset_day(local_date);
        }
        self.update_session_metrics(bar, local_dt);

        let mut intents = Vec::new();
        self.maybe_fill_pending_entry(bar, &mut intents);
        if let Some((reason, exit_price)) = self.evaluate_exit(bar, local_dt) {
            self.apply_exit(reason, exit_price, bar, &mut intents);
            self.sync_state();
            return intents;
        }

        if self.open_position.is_none() && self.pending_entry.is_none() {
            let signal = self
                .evaluate_mr_signal(bar, local_dt)
                .or_else(|| self.evaluate_bo_signal(bar, local_dt));
            if let Some(signal) = signal {
                if signal.owner == Owner::Breakout {
                    if signal.side == PositionSide::Long {
                        self.bo_was_long_today = true;
                    } else if signal.side == PositionSide::Short {
                        self.bo_was_short_today = true;
                    }
                }
                self.pending_entry = Some(signal);
                self.hybrid_state = HybridState::Pending;
            }
        }
        self.sync_state();
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &CommandAck) -> Vec<Intent> {
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_stop_order(&mut self, _ctx: &StrategyCtx, _ord: &StopOrderEvent) -> Vec<Intent> {
        self.lifecycle_stage = "stop_order_observed".to_string();
        self.sync_state();
        Vec::new()
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        _ctx: &StrategyCtx,
        _snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        self.lifecycle_stage = "bootstrapped".to_string();
        self.bootstrap_seen = true;
        self.sync_state();
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        _state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        self.lifecycle_stage = "runtime_state_restored".to_string();
        self.runtime_state_restored = true;
        self.sync_state();
        Vec::new()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        if let StrategyState::AlorUsdrubfHybrid {
            lifecycle_stage,
            last_bar_ts,
            bootstrap_seen,
            runtime_state_restored,
            hybrid_state,
            current_date_local,
            cash,
            pending_entry_side: _,
            open_position_side: _,
            open_position_qty: _,
        } = &state
        {
            self.lifecycle_stage = lifecycle_stage.clone();
            self.last_bar_ts = *last_bar_ts;
            self.bootstrap_seen = *bootstrap_seen;
            self.runtime_state_restored = *runtime_state_restored;
            self.hybrid_state = match hybrid_state.as_str() {
                "pending" => HybridState::Pending,
                "open" => HybridState::Open,
                _ => HybridState::Flat,
            };
            self.current_date_local = current_date_local
                .as_ref()
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok());
            if *cash > 0.0 {
                self.cash = *cash;
            }
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
