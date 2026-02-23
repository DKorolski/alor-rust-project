use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MarketState {
    Open,
    Weekend,
    OutsideSession,
    Break1,
    Break2,
}

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
    #[serde(default)]
    pub timezone_offset_hours: i32,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    config: TradingPeriods,
    timezone_offset_hours: i32,
}

impl Scheduler {
    pub fn new(config: TradingPeriods) -> Self {
        let timezone_offset_hours = config.timezone_offset_hours;
        Self {
            config,
            timezone_offset_hours,
        }
    }

    pub fn new_with_fallback_offset_hours(config: TradingPeriods, fallback: i32) -> Self {
        let timezone_offset_hours = if config.timezone_offset_hours == 0 {
            fallback
        } else {
            config.timezone_offset_hours
        };
        Self {
            config,
            timezone_offset_hours,
        }
    }

    pub fn config(&self) -> &TradingPeriods {
        &self.config
    }

    pub fn local_datetime_utc(&self, ts_utc: i64) -> Option<DateTime<FixedOffset>> {
        let offset = FixedOffset::east_opt(self.timezone_offset_hours.saturating_mul(3600))?;
        Utc.timestamp_opt(ts_utc, 0)
            .single()
            .map(|utc| utc.with_timezone(&offset))
    }

    pub fn is_market_open(&self, current_time: NaiveTime, current_day: Weekday) -> bool {
        self.market_state(current_time, current_day) == MarketState::Open
    }

    pub fn is_market_open_utc(&self, ts_utc: i64) -> bool {
        self.market_state_utc(ts_utc) == MarketState::Open
    }

    pub fn market_state_utc(&self, ts_utc: i64) -> MarketState {
        let Some(local) = self.local_datetime_utc(ts_utc) else {
            return MarketState::OutsideSession;
        };
        self.market_state(local.time(), local.weekday())
    }

    pub fn market_state(&self, current_time: NaiveTime, current_day: Weekday) -> MarketState {
        if self.config.weekends_off && matches!(current_day, Weekday::Sat | Weekday::Sun) {
            return MarketState::Weekend;
        }

        if !in_window(
            current_time,
            self.config.session_start,
            self.config.session_end,
        ) {
            return MarketState::OutsideSession;
        }

        if in_window(
            current_time,
            self.config.break_start_1,
            self.config.break_end_1,
        ) {
            return MarketState::Break1;
        }

        if in_window(
            current_time,
            self.config.break_start_2,
            self.config.break_end_2,
        ) {
            return MarketState::Break2;
        }

        MarketState::Open
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
        if self.market_state_utc(now.and_utc().timestamp()) != MarketState::Open {
            return true;
        }

        let elapsed = now.signed_duration_since(last_received_bar).num_seconds();
        elapsed <= max_silence_bars_sec
    }
}

fn in_window(now: NaiveTime, start: NaiveTime, end: NaiveTime) -> bool {
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
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
            break_start_1: t("14:00"),
            break_end_1: t("14:05"),
            break_start_2: t("18:50"),
            break_end_2: t("19:05"),
            weekends_off: true,
            timezone_offset_hours: 3,
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
        assert_eq!(
            scheduler.market_state(t("10:30"), Weekday::Sun),
            MarketState::Weekend
        );
    }

    #[test]
    fn scheduler_break2_blocks_market_open() {
        let scheduler = scheduler();
        assert_eq!(
            scheduler.market_state(t("18:55"), Weekday::Tue),
            MarketState::Break2
        );
    }

    #[test]
    fn scheduler_respects_timezone_offset_hours() {
        let scheduler = scheduler();
        // 06:00 UTC == 09:00 MSK for +3 offset
        let ts_utc = NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(6, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert!(scheduler.is_market_open_utc(ts_utc));
    }

    #[test]
    fn boundaries_are_start_inclusive_and_end_exclusive() {
        let scheduler = scheduler();
        assert_eq!(
            scheduler.market_state(t("09:00"), Weekday::Mon),
            MarketState::Open
        );
        assert_eq!(
            scheduler.market_state(t("14:00"), Weekday::Mon),
            MarketState::Break1
        );
        assert_eq!(
            scheduler.market_state(t("14:05"), Weekday::Mon),
            MarketState::Open
        );
    }

    #[test]
    fn end_exclusive_window_logic_supports_wraparound() {
        let start = NaiveTime::from_hms_opt(23, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(1, 0, 0).unwrap();

        assert!(in_window(
            NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
            start,
            end
        ));
        assert!(in_window(
            NaiveTime::from_hms_opt(0, 59, 59).unwrap(),
            start,
            end
        ));
        assert!(!in_window(
            NaiveTime::from_hms_opt(1, 0, 0).unwrap(),
            start,
            end
        ));
    }

    #[test]
    fn silence_check_ignored_when_market_closed() {
        let scheduler = scheduler();
        let now = NaiveDate::from_ymd_opt(2025, 1, 7)
            .unwrap()
            .and_hms_opt(11, 2, 0)
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
