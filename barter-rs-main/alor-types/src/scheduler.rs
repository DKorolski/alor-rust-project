use chrono::{Datelike, NaiveDateTime, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TradingPeriods {
    pub session_start: NaiveTime,
    pub session_end: NaiveTime,
    pub break_start_1: NaiveTime,
    pub break_end_1: NaiveTime,
    pub break_start_2: NaiveTime,
    pub break_end_2: NaiveTime,
    #[serde(default)]
    pub weekends_off: bool,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    config: TradingPeriods,
}

impl Scheduler {
    pub fn new(config: TradingPeriods) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &TradingPeriods {
        &self.config
    }

    pub fn is_market_open(&self, current_time: NaiveTime, current_day: Weekday) -> bool {
        if self.config.weekends_off && matches!(current_day, Weekday::Sat | Weekday::Sun) {
            return false;
        }

        let in_session = in_window(
            current_time,
            self.config.session_start,
            self.config.session_end,
        );
        if !in_session {
            return false;
        }

        if in_window(
            current_time,
            self.config.break_start_1,
            self.config.break_end_1,
        ) {
            return false;
        }

        if in_window(
            current_time,
            self.config.break_start_2,
            self.config.break_end_2,
        ) {
            return false;
        }

        true
    }

    pub fn check_silence_period(
        &self,
        last_received_bar: NaiveDateTime,
        max_silence_bars_sec: i64,
    ) -> bool {
        let now = Utc::now().naive_utc();
        self.check_silence_period_at(now, last_received_bar, max_silence_bars_sec)
    }

    pub fn check_silence_period_at(
        &self,
        now: NaiveDateTime,
        last_received_bar: NaiveDateTime,
        max_silence_bars_sec: i64,
    ) -> bool {
        if !self.is_market_open(now.time(), now.weekday()) {
            return true;
        }

        let elapsed = now.signed_duration_since(last_received_bar).num_seconds();
        elapsed <= max_silence_bars_sec
    }
}

fn in_window(now: NaiveTime, start: NaiveTime, end: NaiveTime) -> bool {
    if start <= end {
        now >= start && now <= end
    } else {
        now >= start || now <= end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn t(v: &str) -> NaiveTime {
        NaiveTime::parse_from_str(v, "%H:%M").unwrap()
    }

    fn scheduler() -> Scheduler {
        Scheduler::new(TradingPeriods {
            session_start: t("09:00"),
            session_end: t("23:49"),
            break_start_1: t("23:50"),
            break_end_1: t("09:00"),
            break_start_2: t("14:00"),
            break_end_2: t("14:05"),
            weekends_off: true,
        })
    }

    #[test]
    fn market_open_on_weekday_inside_session_and_outside_breaks() {
        let scheduler = scheduler();
        assert!(scheduler.is_market_open(t("10:30"), Weekday::Mon));
    }

    #[test]
    fn market_closed_on_weekend_when_weekends_off() {
        let scheduler = scheduler();
        assert!(!scheduler.is_market_open(t("10:30"), Weekday::Sun));
    }

    #[test]
    fn market_closed_during_clearing_break() {
        let scheduler = scheduler();
        assert!(!scheduler.is_market_open(t("14:02"), Weekday::Tue));
    }

    #[test]
    fn silence_check_ignored_when_market_closed() {
        let scheduler = scheduler();
        let now = NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(14, 2, 0)
            .unwrap();
        let last_bar = NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(13, 0, 0)
            .unwrap();

        assert!(scheduler.check_silence_period_at(now, last_bar, 60));
    }

    #[test]
    fn silence_check_fails_when_market_open_and_threshold_exceeded() {
        let scheduler = scheduler();
        let now = NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        let last_bar = NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(9, 40, 0)
            .unwrap();

        assert!(!scheduler.check_silence_period_at(now, last_bar, 900));
    }
}
