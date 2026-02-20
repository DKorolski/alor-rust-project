use anyhow::Context;
use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use tracing::{debug, warn};

pub use alor_types::{
    Action, OrderSnapshot, OrdersSnapshot, PositionSnapshot, PositionsSnapshot, Side, StrategyBar,
    StrategyContext, StrategyCore,
};

#[derive(Debug, Clone, Copy)]
pub struct Bar {
    pub time: DateTime<FixedOffset>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone)]
struct PendingEntry {
    direction: Direction,
    size: i64,
}

#[derive(Debug, Clone)]
struct Position {
    direction: Direction,
    size: i64,
    entry_price: f64,
    entry_time: DateTime<FixedOffset>,
    tp: f64,
    sl: f64,
}

#[derive(Debug, Clone)]
pub struct StrategyConfig {
    k_long: f64,
    k_short: f64,
    wait_hours: i64,
    k_tp_long: f64,
    k_sl_long: f64,
    k_tp_short: f64,
    k_sl_short: f64,
    long_ex_pct: f64,
    short_ex_pct: f64,
    max_entry_hour: u32,
    close_hour: u32,
    close_minute: u32,
    session_gap_min: f64,
    exit_offset_min: i64,
    stake: i64,
    test: bool,
    work_weekends: bool,
    cash_factor: f64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
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
            stake: 1,
            test: false,
            work_weekends: false,
            cash_factor: 0.9,
        }
    }
}

pub struct TradeLogger {
    writer: csv::Writer<std::fs::File>,
}

impl TradeLogger {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        let csv_path = std::path::Path::new(path);
        let create_header = !csv_path.exists();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(csv_path)
            .with_context(|| format!("failed to open trade log {path}"))?;
        let mut writer = csv::Writer::from_writer(file);
        if create_header {
            writer.write_record([
                "entry_time",
                "exit_time",
                "direction",
                "size",
                "entry_price",
                "exit_price",
                "reason",
                "pnl",
                "cash_after",
            ])?;
            writer.flush()?;
        }
        Ok(Self { writer })
    }

    fn log_trade(
        &mut self,
        position: &Position,
        exit_time: DateTime<FixedOffset>,
        exit_price: f64,
        reason: &str,
        cash_after: f64,
    ) -> anyhow::Result<()> {
        let pnl = match position.direction {
            Direction::Long => (exit_price - position.entry_price) * position.size as f64,
            Direction::Short => (position.entry_price - exit_price) * position.size as f64,
        };

        self.writer.write_record([
            position.entry_time.to_rfc3339(),
            exit_time.to_rfc3339(),
            format!("{:?}", position.direction),
            position.size.to_string(),
            format!("{:.4}", position.entry_price),
            format!("{:.4}", exit_price),
            reason.to_string(),
            format!("{:.4}", pnl),
            format!("{:.2}", cash_after),
        ])?;
        self.writer.flush()?;
        Ok(())
    }
}

pub struct StrategyState {
    cfg: StrategyConfig,
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

impl StrategyState {
    pub fn new(cfg: StrategyConfig, cash: f64) -> Self {
        Self {
            cfg,
            cash,
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

    pub fn on_bar(&mut self, bar: Bar, trade_log: &mut TradeLogger) -> anyhow::Result<()> {
        if !self.cfg.work_weekends && bar.time.weekday().number_from_monday() >= 6 {
            debug!("weekend bar skipped");
            return Ok(());
        }

        self.update_session(&bar);
        self.execute_pending_entry(&bar);
        self.handle_exit_conditions(&bar, trade_log)?;
        self.maybe_generate_signal(&bar);

        Ok(())
    }

    fn update_session(&mut self, bar: &Bar) {
        match self.last_dt {
            None => {
                self.reset_session(bar);
            }
            Some(last_dt) => {
                let diff_min = (bar.time - last_dt).num_seconds() as f64 / 60.0;
                if diff_min < 0.0 {
                    warn!(
                        "time moved backwards: last_dt={} current_dt={} diff_min={:.2}",
                        last_dt, bar.time, diff_min
                    );
                }
                if diff_min > self.cfg.session_gap_min {
                    self.reset_session(bar);
                }
            }
        }

        if self.session_start_dt.is_none() {
            self.session_start_dt = Some(bar.time);
        }
        self.session_end_dt = Some(bar.time);
        self.session_high = Some(self.session_high.unwrap_or(bar.high).max(bar.high));
        self.session_low = Some(self.session_low.unwrap_or(bar.low).min(bar.low));
        self.session_close = Some(bar.close);

        let time = bar.time.time();
        if time.minute() == 0 && time.hour() == 10 {
            self.first_hour_price = Some(bar.close);
        }
        if time.minute() == 0 && time.hour() == 10 && self.first_min_high.is_none() {
            self.first_min_high = Some(bar.high);
            self.first_min_low = Some(bar.low);
        }

        self.last_dt = Some(bar.time);
    }

    fn reset_session(&mut self, bar: &Bar) {
        if let Some(close) = self.session_close {
            self.pre_prev_close = self.yesterday_close;
            self.yesterday_close = Some(close);
            if let (Some(high), Some(low)) = (self.session_high, self.session_low) {
                self.yesterday_range = Some(high - low);
            }
        }

        self.session_start_dt = Some(bar.time);
        self.session_end_dt = Some(bar.time);
        self.session_high = Some(bar.high);
        self.session_low = Some(bar.low);
        self.session_close = Some(bar.close);
        self.traded_session = false;
        self.pending_entry = None;
        self.position = None;
        self.first_min_high = None;
        self.first_min_low = None;
        self.first_hour_price = None;
    }

    fn execute_pending_entry(&mut self, bar: &Bar) {
        if self.position.is_some() {
            return;
        }

        let Some(pending) = self.pending_entry.clone() else {
            return;
        };

        let entry_price = match pending.direction {
            Direction::Long => bar.open,
            Direction::Short => bar.open,
        };

        let (tp, sl) = match pending.direction {
            Direction::Long => {
                let tp = entry_price + entry_price * self.cfg.k_tp_long / 100.0;
                let sl = entry_price - entry_price * self.cfg.k_sl_long / 100.0;
                (tp, sl)
            }
            Direction::Short => {
                let tp = entry_price - entry_price * self.cfg.k_tp_short / 100.0;
                let sl = entry_price + entry_price * self.cfg.k_sl_short / 100.0;
                (tp, sl)
            }
        };

        self.position = Some(Position {
            direction: pending.direction,
            size: pending.size,
            entry_price,
            entry_time: bar.time,
            tp,
            sl,
        });
        self.pending_entry = None;
        self.traded_session = true;

        debug!(
            "entry executed: direction={:?} price={:.4} size={}",
            pending.direction, entry_price, pending.size
        );
    }

    fn handle_exit_conditions(
        &mut self,
        bar: &Bar,
        trade_log: &mut TradeLogger,
    ) -> anyhow::Result<()> {
        let Some(position) = self.position.clone() else {
            return Ok(());
        };

        let exit_reason = match position.direction {
            Direction::Long => {
                if bar.low <= position.sl {
                    Some((position.sl, "stop_loss"))
                } else if bar.high >= position.tp {
                    Some((position.tp, "take_profit"))
                } else {
                    None
                }
            }
            Direction::Short => {
                if bar.high >= position.sl {
                    Some((position.sl, "stop_loss"))
                } else if bar.low <= position.tp {
                    Some((position.tp, "take_profit"))
                } else {
                    None
                }
            }
        };

        if let Some((exit_price, reason)) = exit_reason {
            self.close_position(position, bar.time, exit_price, reason, trade_log)?;
        }

        Ok(())
    }

    fn close_position(
        &mut self,
        position: Position,
        exit_time: DateTime<FixedOffset>,
        exit_price: f64,
        reason: &str,
        trade_log: &mut TradeLogger,
    ) -> anyhow::Result<()> {
        let pnl = match position.direction {
            Direction::Long => (exit_price - position.entry_price) * position.size as f64,
            Direction::Short => (position.entry_price - exit_price) * position.size as f64,
        };
        self.cash += pnl;

        trade_log.log_trade(&position, exit_time, exit_price, reason, self.cash)?;
        self.position = None;

        debug!(
            "exit: reason={} price={:.4} pnl={:.2} cash={:.2}",
            reason, exit_price, pnl, self.cash
        );

        Ok(())
    }

    fn maybe_generate_signal(&mut self, bar: &Bar) {
        let Some(yesterday_close) = self.yesterday_close else {
            return;
        };
        let Some(yesterday_range) = self.yesterday_range else {
            return;
        };
        let Some(first_min_high) = self.first_min_high else {
            return;
        };
        let Some(first_min_low) = self.first_min_low else {
            return;
        };
        let Some(first_hour_price) = self.first_hour_price else {
            return;
        };

        if self.traded_session {
            return;
        }

        if bar.time.hour() >= self.cfg.max_entry_hour {
            return;
        }

        let range = yesterday_range;
        let long_ex = yesterday_close + range * self.cfg.long_ex_pct / 100.0;
        let short_ex = yesterday_close - range * self.cfg.short_ex_pct / 100.0;

        if bar.high >= long_ex {
            self.pending_entry = Some(PendingEntry {
                direction: Direction::Long,
                size: self.cfg.stake,
            });
            debug!(
                "long signal: bar_high={:.4} long_ex={:.4} first_hour_price={:.4}",
                bar.high, long_ex, first_hour_price
            );
        } else if bar.low <= short_ex {
            self.pending_entry = Some(PendingEntry {
                direction: Direction::Short,
                size: self.cfg.stake,
            });
            debug!(
                "short signal: bar_low={:.4} short_ex={:.4} first_hour_price={:.4}",
                bar.low, short_ex, first_hour_price
            );
        }

        if bar.high >= first_min_high {
            debug!("first min high breached");
        }
        if bar.low <= first_min_low {
            debug!("first min low breached");
        }
    }
}

impl StrategyCore for StrategyState {
    fn on_bar(&mut self, _bar: StrategyBar, _ctx: StrategyContext) -> Vec<Action> {
        Vec::new()
    }
}
