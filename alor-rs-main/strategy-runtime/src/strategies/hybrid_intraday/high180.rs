use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use super::types::{ReasonCode, Side};

#[derive(Debug, Clone, Copy)]
pub struct High180MrConfig {
    pub min_rel_range: f64,
    pub max_rel_range: f64,
    pub k_long: f64,
    pub k_short: f64,
    pub stop_loss_mult: f64,
    pub max_hold: Duration,
    pub entry_end_time: NaiveTime,
}

impl Default for High180MrConfig {
    fn default() -> Self {
        Self {
            min_rel_range: 0.005,
            max_rel_range: 0.050,
            k_long: 0.085,
            k_short: 0.090,
            stop_loss_mult: 7.0,
            max_hold: Duration::minutes(180),
            entry_end_time: NaiveTime::from_hms_opt(11, 59, 59).unwrap_or(NaiveTime::MIN),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct High180Signal {
    pub side: Side,
    pub target_price: f64,
    pub stop_price: f64,
    pub reason: ReasonCode,
}

#[derive(Debug, Clone, Copy)]
pub struct High180Open {
    pub target_price: f64,
    pub stop_price: f64,
    pub max_hold: Duration,
}

impl High180Open {
    pub fn from_signal(signal: &High180Signal, config: High180MrConfig) -> Self {
        Self {
            target_price: signal.target_price,
            stop_price: signal.stop_price,
            max_hold: config.max_hold,
        }
    }
}

#[derive(Debug, Clone)]
pub struct High180MrEngine {
    config: High180MrConfig,
    current_day: Option<NaiveDate>,
    day_high: Option<f64>,
    day_low: Option<f64>,
}

impl High180MrEngine {
    pub fn new(config: High180MrConfig) -> Self {
        Self {
            config,
            current_day: None,
            day_high: None,
            day_low: None,
        }
    }

    pub fn config(&self) -> High180MrConfig {
        self.config
    }

    pub fn on_bar(&mut self, ts: NaiveDateTime, high: f64, low: f64) {
        if self.current_day != Some(ts.date()) {
            self.current_day = Some(ts.date());
            self.day_high = Some(high);
            self.day_low = Some(low);
            return;
        }
        self.day_high = Some(self.day_high.unwrap_or(high).max(high));
        self.day_low = Some(self.day_low.unwrap_or(low).min(low));
    }

    pub fn evaluate_entry(
        &self,
        ts: NaiveDateTime,
        close: f64,
        close_prev: f64,
        day_range_prev: f64,
    ) -> Option<High180Signal> {
        if ts.time() > self.config.entry_end_time {
            return None;
        }
        if close == 0.0 || close_prev.is_nan() || day_range_prev.is_nan() {
            return None;
        }
        let rel_range = day_range_prev / close;
        if !(self.config.min_rel_range < rel_range && rel_range < self.config.max_rel_range) {
            return None;
        }
        let midpoint = (self.day_high? + self.day_low?) / 2.0;
        if close < close_prev && close > close_prev - self.config.k_long * day_range_prev {
            if midpoint <= close {
                return None;
            }
            let take_distance = midpoint - close;
            let stop_distance = self.config.stop_loss_mult * take_distance;
            return Some(High180Signal {
                side: Side::Long,
                target_price: midpoint,
                stop_price: close - stop_distance,
                reason: ReasonCode::MorningMeanReversionLong,
            });
        }
        if close > close_prev && close < close_prev + self.config.k_short * day_range_prev {
            if midpoint >= close {
                return None;
            }
            let take_distance = close - midpoint;
            let stop_distance = self.config.stop_loss_mult * take_distance;
            return Some(High180Signal {
                side: Side::Short,
                target_price: midpoint,
                stop_price: close + stop_distance,
                reason: ReasonCode::MorningMeanReversionShort,
            });
        }
        None
    }

    pub fn evaluate_exit(
        &self,
        open: &High180Open,
        entry_ts: NaiveDateTime,
        side: Side,
        ts: NaiveDateTime,
        close: f64,
    ) -> Option<(&'static str, f64)> {
        if ts <= entry_ts {
            return None;
        }
        match side {
            Side::Long => {
                if close >= open.target_price {
                    return Some(("midpoint_take", close));
                }
                if close <= open.stop_price {
                    return Some(("stop", close));
                }
            }
            Side::Short => {
                if close <= open.target_price {
                    return Some(("midpoint_take", close));
                }
                if close >= open.stop_price {
                    return Some(("stop", close));
                }
            }
        }
        if ts - entry_ts >= open.max_hold {
            return Some(("time_stop", close));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn ts(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap_or(NaiveDate::MIN)
            .and_hms_opt(hour, minute, 0)
            .unwrap_or(NaiveDateTime::MIN)
    }

    #[test]
    fn long_requires_midpoint_above_entry_close() {
        let mut engine = High180MrEngine::new(High180MrConfig::default());
        engine.on_bar(ts(9, 0), 100.0, 90.0);

        assert!(engine.evaluate_entry(ts(9, 0), 99.0, 100.0, 50.0).is_none());
    }

    #[test]
    fn short_requires_midpoint_below_entry_close() {
        let mut engine = High180MrEngine::new(High180MrConfig::default());
        engine.on_bar(ts(9, 0), 110.0, 100.0);

        assert!(engine
            .evaluate_entry(ts(9, 0), 101.0, 100.0, 50.0)
            .is_none());
    }

    #[test]
    fn exits_after_max_hold() {
        let engine = High180MrEngine::new(High180MrConfig::default());
        let open = High180Open {
            target_price: 105.0,
            stop_price: 90.0,
            max_hold: Duration::minutes(180),
        };

        assert_eq!(
            engine.evaluate_exit(&open, ts(9, 0), Side::Long, ts(12, 0), 100.0),
            Some(("time_stop", 100.0))
        );
    }
}
