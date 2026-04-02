use chrono::{NaiveDate, NaiveDateTime, Timelike};

use super::types::{EntrySignal, EntryStyle, ExitSignal, Owner, ReasonCode, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinRangeMode {
    Disabled,
    Absolute,
    RelativePrevClose,
}

#[derive(Debug, Clone, Copy)]
pub struct IntradayBreakoutConfig {
    pub k: f64,
    pub stop1_range: f64,
    pub stop2_range: f64,
    pub big_move_threshold: f64,
    pub min_range: f64,
    pub min_range_mode: MinRangeMode,
    pub exclude_weekends: bool,
    pub wait_hours: f64,
}

impl Default for IntradayBreakoutConfig {
    fn default() -> Self {
        Self {
            k: 0.65,
            stop1_range: 0.51,
            stop2_range: 0.35,
            big_move_threshold: 0.025,
            min_range: 1.01,
            min_range_mode: MinRangeMode::Absolute,
            exclude_weekends: true,
            wait_hours: 3.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IntradayBreakoutEngine {
    pub config: IntradayBreakoutConfig,
    cur_day_date: Option<NaiveDate>,
    cur_day_high: Option<f64>,
    cur_day_low: Option<f64>,
    cur_day_close: Option<f64>,
    yesterday_close: Option<f64>,
    yesterday_range: Option<f64>,
    yesterday_return: Option<f64>,
    day_before_close: Option<f64>,
    was_long_today: bool,
    was_short_today: bool,
    today_start: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct IntradayBreakoutSnapshot {
    pub cur_day_date: Option<NaiveDate>,
    pub cur_day_high: Option<f64>,
    pub cur_day_low: Option<f64>,
    pub cur_day_close: Option<f64>,
    pub yesterday_close: Option<f64>,
    pub yesterday_range: Option<f64>,
    pub yesterday_return: Option<f64>,
    pub day_before_close: Option<f64>,
    pub was_long_today: bool,
    pub was_short_today: bool,
    pub today_start: Option<NaiveDateTime>,
}

impl IntradayBreakoutEngine {
    pub fn new(config: IntradayBreakoutConfig) -> Self {
        Self {
            config,
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
        }
    }

    pub fn on_bar(&mut self, dt: NaiveDateTime, open: f64, high: f64, low: f64, close: f64) {
        if self.config.exclude_weekends && is_weekend(dt) {
            return;
        }
        if self.cur_day_date.is_none() {
            self.init_new_day(dt, open);
        } else if Some(dt.date()) != self.cur_day_date {
            self.on_day_close();
            self.init_new_day(dt, open);
        }
        self.update_cur_day(high, low, close);
    }

    pub fn mark_entry(&mut self, side: Side) {
        match side {
            Side::Long => self.was_long_today = true,
            Side::Short => self.was_short_today = true,
        }
    }

    pub fn snapshot(&self) -> IntradayBreakoutSnapshot {
        IntradayBreakoutSnapshot {
            cur_day_date: self.cur_day_date,
            cur_day_high: self.cur_day_high,
            cur_day_low: self.cur_day_low,
            cur_day_close: self.cur_day_close,
            yesterday_close: self.yesterday_close,
            yesterday_range: self.yesterday_range,
            yesterday_return: self.yesterday_return,
            day_before_close: self.day_before_close,
            was_long_today: self.was_long_today,
            was_short_today: self.was_short_today,
            today_start: self.today_start,
        }
    }

    pub fn restore_snapshot(&mut self, snapshot: IntradayBreakoutSnapshot) {
        self.cur_day_date = snapshot.cur_day_date;
        self.cur_day_high = snapshot.cur_day_high;
        self.cur_day_low = snapshot.cur_day_low;
        self.cur_day_close = snapshot.cur_day_close;
        self.yesterday_close = snapshot.yesterday_close;
        self.yesterday_range = snapshot.yesterday_range;
        self.yesterday_return = snapshot.yesterday_return;
        self.day_before_close = snapshot.day_before_close;
        self.was_long_today = snapshot.was_long_today;
        self.was_short_today = snapshot.was_short_today;
        self.today_start = snapshot.today_start;
    }

    pub fn evaluate_entry(&self, dt: NaiveDateTime, close: f64) -> Option<EntrySignal> {
        if !self.can_check_entry(dt) {
            return None;
        }
        let yesterday_close = self.yesterday_close?;
        let yesterday_range = self.yesterday_range?;
        if !self.passes_min_range_filter() {
            return None;
        }

        let mut can_long = true;
        let mut can_short = true;
        if let Some(ret) = self.yesterday_return {
            if ret < -self.config.big_move_threshold {
                can_long = false;
            }
            if ret > self.config.big_move_threshold {
                can_short = false;
            }
        }

        let long_level = yesterday_close + self.config.k * yesterday_range;
        let short_level = yesterday_close - self.config.k * yesterday_range;

        if can_long && !self.was_long_today && close > long_level {
            return Some(EntrySignal {
                owner: Owner::IntradayBreakout,
                side: Side::Long,
                entry_style: EntryStyle::Market,
                reason: ReasonCode::BreakoutLong,
                stop_price: None,
                take_price: None,
            });
        }
        if can_short && !self.was_short_today && close < short_level {
            return Some(EntrySignal {
                owner: Owner::IntradayBreakout,
                side: Side::Short,
                entry_style: EntryStyle::Market,
                reason: ReasonCode::BreakoutShort,
                stop_price: None,
                take_price: None,
            });
        }
        None
    }

    pub fn evaluate_exit(&self, dt: NaiveDateTime, close: f64, side: Side) -> Option<ExitSignal> {
        if self.config.exclude_weekends && is_weekend(dt) {
            return None;
        }
        let yesterday_close = self.yesterday_close?;
        let yesterday_range = self.yesterday_range?;

        if dt.hour() == 23 && dt.minute() == 30 {
            return Some(ExitSignal {
                owner: Owner::IntradayBreakout,
                reason: ReasonCode::BreakoutEodExit,
            });
        }

        match side {
            Side::Long => {
                let stop2_level = yesterday_close - self.config.stop2_range * yesterday_range;
                if close < stop2_level {
                    return Some(ExitSignal {
                        owner: Owner::IntradayBreakout,
                        reason: ReasonCode::BreakoutStop2Long,
                    });
                }
                if dt.minute() == 50 {
                    let stop1_level = yesterday_close + self.config.stop1_range * yesterday_range;
                    if close < stop1_level {
                        return Some(ExitSignal {
                            owner: Owner::IntradayBreakout,
                            reason: ReasonCode::BreakoutStop1Long,
                        });
                    }
                }
            }
            Side::Short => {
                let stop2_level = yesterday_close + self.config.stop2_range * yesterday_range;
                if close > stop2_level {
                    return Some(ExitSignal {
                        owner: Owner::IntradayBreakout,
                        reason: ReasonCode::BreakoutStop2Short,
                    });
                }
                if dt.minute() == 50 {
                    let stop1_level = yesterday_close - self.config.stop1_range * yesterday_range;
                    if close > stop1_level {
                        return Some(ExitSignal {
                            owner: Owner::IntradayBreakout,
                            reason: ReasonCode::BreakoutStop1Short,
                        });
                    }
                }
            }
        }

        None
    }

    fn init_new_day(&mut self, dt: NaiveDateTime, open: f64) {
        self.cur_day_date = Some(dt.date());
        self.cur_day_high = Some(open);
        self.cur_day_low = Some(open);
        self.cur_day_close = Some(open);
        self.today_start = Some(dt);
        self.was_long_today = false;
        self.was_short_today = false;
    }

    fn update_cur_day(&mut self, high: f64, low: f64, close: f64) {
        let cur_high = self.cur_day_high.unwrap_or(high);
        let cur_low = self.cur_day_low.unwrap_or(low);
        self.cur_day_high = Some(cur_high.max(high));
        self.cur_day_low = Some(cur_low.min(low));
        self.cur_day_close = Some(close);
    }

    fn on_day_close(&mut self) {
        let close = self.cur_day_close;
        self.yesterday_close = close;
        self.yesterday_range = match (self.cur_day_high, self.cur_day_low) {
            (Some(high), Some(low)) => Some(high - low),
            _ => None,
        };

        if let (Some(day_before_close), Some(yesterday_close)) =
            (self.day_before_close, self.yesterday_close)
        {
            if day_before_close != 0.0 {
                self.yesterday_return =
                    Some((yesterday_close - day_before_close) / day_before_close);
            } else {
                self.yesterday_return = None;
            }
        } else {
            self.yesterday_return = None;
        }

        self.day_before_close = close;
    }

    fn can_check_entry(&self, dt: NaiveDateTime) -> bool {
        if self.config.exclude_weekends && is_weekend(dt) {
            return false;
        }
        let Some(today_start) = self.today_start else {
            return false;
        };
        let delta_h = (dt - today_start).num_seconds() as f64 / 3600.0;
        delta_h >= self.config.wait_hours
    }

    fn passes_min_range_filter(&self) -> bool {
        match self.config.min_range_mode {
            MinRangeMode::Disabled => true,
            MinRangeMode::Absolute => self.yesterday_range.unwrap_or(0.0) >= self.config.min_range,
            MinRangeMode::RelativePrevClose => {
                let Some(yesterday_close) = self.yesterday_close else {
                    return false;
                };
                if yesterday_close == 0.0 {
                    return false;
                }
                self.yesterday_range.unwrap_or(0.0) / yesterday_close >= self.config.min_range
            }
        }
    }
}

fn is_weekend(dt: NaiveDateTime) -> bool {
    let weekday = chrono::Datelike::weekday(&dt).number_from_monday();
    weekday >= 6
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn dt(y: i32, mo: u32, d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap_or(NaiveDate::MIN)
            .and_hms_opt(h, m, 0)
            .unwrap_or(NaiveDateTime::MIN)
    }

    #[test]
    fn blocks_entry_before_wait_hours() {
        let mut engine = IntradayBreakoutEngine::new(IntradayBreakoutConfig {
            wait_hours: 3.0,
            ..IntradayBreakoutConfig::default()
        });
        engine.on_bar(dt(2026, 1, 5, 9, 0), 100.0, 101.0, 99.0, 100.5);
        let sig = engine.evaluate_entry(dt(2026, 1, 5, 10, 0), 102.0);
        assert!(sig.is_none());
    }

    #[test]
    fn emits_eod_exit_at_2330() {
        let mut engine = IntradayBreakoutEngine::new(IntradayBreakoutConfig::default());
        engine.on_bar(dt(2026, 1, 5, 9, 0), 100.0, 100.0, 99.0, 99.5);
        engine.on_bar(dt(2026, 1, 6, 9, 0), 101.0, 103.0, 100.0, 102.0);
        let exit = engine.evaluate_exit(dt(2026, 1, 6, 23, 30), 102.0, Side::Long);
        assert!(exit.is_some());
        assert_eq!(
            exit.expect("exit expected").reason,
            ReasonCode::BreakoutEodExit
        );
    }
}
