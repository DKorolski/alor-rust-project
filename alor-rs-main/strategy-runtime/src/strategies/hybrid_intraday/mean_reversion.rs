use chrono::{Duration, NaiveDateTime, NaiveTime, Timelike};

use super::types::{EntrySignal, EntryStyle, Owner, ReasonCode, Side};

#[derive(Debug, Clone, Copy)]
pub struct MeanReversionConfig {
    pub min_range_long: f64,
    pub max_range_long: f64,
    pub k_long: f64,
    pub take_k_long: f64,
    pub stop_k_long: f64,
    pub min_range_short: f64,
    pub max_range_short: f64,
    pub k_short: f64,
    pub take_k_short: f64,
    pub stop_k_short: f64,
    pub tick_size: f64,
    pub session_end_time: NaiveTime,
    pub exit_offset: Duration,
}

impl Default for MeanReversionConfig {
    fn default() -> Self {
        Self {
            min_range_long: 0.013,
            max_range_long: 0.035,
            k_long: 0.032,
            take_k_long: 0.11,
            stop_k_long: 0.44,
            min_range_short: 0.010,
            max_range_short: 0.045,
            k_short: 0.055,
            take_k_short: 0.16,
            stop_k_short: 0.43,
            tick_size: 0.5,
            session_end_time: NaiveTime::from_hms_opt(11, 59, 0).unwrap_or(NaiveTime::MIN),
            exit_offset: Duration::minutes(5),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MeanReversionEngine {
    pub config: MeanReversionConfig,
}

impl MeanReversionEngine {
    pub fn new(config: MeanReversionConfig) -> Self {
        Self { config }
    }

    pub fn is_active_window(&self, dt: NaiveDateTime) -> bool {
        dt.hour() < 12
    }

    pub fn should_force_exit(&self, dt: NaiveDateTime) -> bool {
        let session_end_dt = dt.date().and_time(self.config.session_end_time);
        dt >= (session_end_dt - self.config.exit_offset)
    }

    pub fn evaluate_entry(
        &self,
        dt: NaiveDateTime,
        close: f64,
        close_prev: f64,
        day_range_prev: f64,
    ) -> Option<EntrySignal> {
        if !self.is_active_window(dt) {
            return None;
        }
        if close == 0.0 || close_prev.is_nan() || day_range_prev.is_nan() {
            return None;
        }

        let rel_day_range = day_range_prev / close;
        let trigger1_long = self.config.min_range_long < rel_day_range
            && rel_day_range < self.config.max_range_long;
        let trigger2_long = close < close_prev;
        let trigger3_long = close > (close_prev - self.config.k_long * day_range_prev);
        if trigger1_long && trigger2_long && trigger3_long {
            let stop_price = self.round_to_tick(close - self.config.stop_k_long * day_range_prev);
            let take_price = self.round_to_tick(close + self.config.take_k_long * day_range_prev);
            return Some(EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Long,
                entry_style: EntryStyle::Bracket,
                reason: ReasonCode::MorningMeanReversionLong,
                stop_price: Some(stop_price),
                take_price: Some(take_price),
            });
        }

        let trigger1_short = self.config.min_range_short < rel_day_range
            && rel_day_range < self.config.max_range_short;
        let trigger2_short = close > close_prev;
        let trigger3_short = close < (close_prev + self.config.k_short * day_range_prev);
        if trigger1_short && trigger2_short && trigger3_short {
            let stop_price = self.round_to_tick(close + self.config.stop_k_short * day_range_prev);
            let take_price = self.round_to_tick(close - self.config.take_k_short * day_range_prev);
            return Some(EntrySignal {
                owner: Owner::MeanReversion,
                side: Side::Short,
                entry_style: EntryStyle::Bracket,
                reason: ReasonCode::MorningMeanReversionShort,
                stop_price: Some(stop_price),
                take_price: Some(take_price),
            });
        }

        None
    }

    fn round_to_tick(&self, price: f64) -> f64 {
        let tick = self.config.tick_size;
        if tick <= 0.0 {
            return price;
        }
        ((price / tick) + 0.5).floor() * tick
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn dt(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap_or(NaiveDate::MIN)
            .and_hms_opt(h, m, 0)
            .unwrap_or(NaiveDateTime::MIN)
    }

    #[test]
    fn rounds_half_up_to_tick() {
        let engine = MeanReversionEngine::new(MeanReversionConfig::default());
        // 10.24 -> 10.0, 10.26 -> 10.5 for tick=0.5
        assert_eq!(engine.round_to_tick(10.24), 10.0);
        assert_eq!(engine.round_to_tick(10.26), 10.5);
    }

    #[test]
    fn blocks_entries_after_active_window() {
        let engine = MeanReversionEngine::new(MeanReversionConfig::default());
        let signal = engine.evaluate_entry(dt(12, 0), 100.0, 101.0, 2.0);
        assert!(signal.is_none());
    }

    #[test]
    fn generates_long_signal() {
        let engine = MeanReversionEngine::new(MeanReversionConfig::default());
        let signal = engine
            .evaluate_entry(dt(10, 0), 101.95, 102.0, 2.0)
            .expect("expected signal");
        assert_eq!(signal.owner, Owner::MeanReversion);
        assert_eq!(signal.side, Side::Long);
        assert_eq!(signal.reason, ReasonCode::MorningMeanReversionLong);
    }
}
