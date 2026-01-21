use std::collections::HashMap;

use chrono::Utc;
use serde::Serialize;

use crate::models::BarEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum RejectReason {
    DuplicateOrOld,
    BadTimestamp,
    BadOhlc,
    NonFinite,
    NegativeVolume,
    FourPriceDoji,
    NotClosedYet,
}

#[derive(Debug, Clone)]
pub struct BarValidationConfig {
    pub tf_sec: i64,
    pub allow_four_price_doji: bool,
    pub close_grace_sec: i64,
}

impl BarValidationConfig {
    pub fn new(tf_sec: i64) -> Self {
        Self {
            tf_sec,
            allow_four_price_doji: true,
            close_grace_sec: 2,
        }
    }
}

#[derive(Debug, Default)]
pub struct BarQualityStats {
    pub received: HashMap<String, u64>,
    pub emitted: HashMap<String, u64>,
    pub dropped: HashMap<(String, RejectReason), u64>,
    pub last_close_time: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct DropReasonCount {
    pub symbol: String,
    pub reason: RejectReason,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct DataQualityReport {
    pub received: HashMap<String, u64>,
    pub emitted: HashMap<String, u64>,
    pub dropped: Vec<DropReasonCount>,
    pub last_close_time: HashMap<String, i64>,
}

impl BarQualityStats {
    pub fn record_received(&mut self, symbol: &str) {
        *self.received.entry(symbol.to_string()).or_insert(0) += 1;
    }

    pub fn record_emitted(&mut self, symbol: &str, close_time: i64) {
        *self.emitted.entry(symbol.to_string()).or_insert(0) += 1;
        self.last_close_time.insert(symbol.to_string(), close_time);
    }

    pub fn record_dropped(&mut self, symbol: &str, reason: RejectReason) {
        *self
            .dropped
            .entry((symbol.to_string(), reason))
            .or_insert(0) += 1;
    }

    pub fn to_report(&self) -> DataQualityReport {
        let mut dropped: Vec<DropReasonCount> = self
            .dropped
            .iter()
            .map(|((symbol, reason), count)| DropReasonCount {
                symbol: symbol.clone(),
                reason: *reason,
                count: *count,
            })
            .collect();
        dropped.sort_by(|a, b| b.count.cmp(&a.count));
        DataQualityReport {
            received: self.received.clone(),
            emitted: self.emitted.clone(),
            dropped,
            last_close_time: self.last_close_time.clone(),
        }
    }
}

pub fn write_data_report(
    path: &str,
    stats: &BarQualityStats,
) -> anyhow::Result<()> {
    let report = stats.to_report();
    let file = std::fs::File::create(path)?;
    serde_json::to_writer_pretty(file, &report)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BarValidator {
    cfg: BarValidationConfig,
}

impl BarValidator {
    pub fn new(cfg: BarValidationConfig) -> Self {
        Self { cfg }
    }

    pub fn validate(
        &self,
        bar: &BarEvent,
        last_emitted_ts: Option<i64>,
    ) -> Result<(), RejectReason> {
        if let Some(last_ts) = last_emitted_ts {
            if bar.close_time_utc <= last_ts {
                return Err(RejectReason::DuplicateOrOld);
            }
        }

        if self.cfg.tf_sec > 0 && bar.close_time_utc % self.cfg.tf_sec != 0 {
            return Err(RejectReason::BadTimestamp);
        }

        let now = Utc::now().timestamp();
        if bar.close_time_utc > now + self.cfg.close_grace_sec {
            return Err(RejectReason::NotClosedYet);
        }

        if !bar.o.is_finite()
            || !bar.h.is_finite()
            || !bar.l.is_finite()
            || !bar.c.is_finite()
        {
            return Err(RejectReason::NonFinite);
        }

        let max_oc = bar.o.max(bar.c);
        let min_oc = bar.o.min(bar.c);
        if bar.h < max_oc || bar.l > min_oc || bar.h < bar.l {
            return Err(RejectReason::BadOhlc);
        }

        if bar.v < 0.0 {
            return Err(RejectReason::NegativeVolume);
        }

        if !self.cfg.allow_four_price_doji && bar.h == bar.l {
            return Err(RejectReason::FourPriceDoji);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BarEvent, DataOrigin};

    fn bar(close_time_utc: i64, o: f64, h: f64, l: f64, c: f64, v: f64) -> BarEvent {
        BarEvent {
            symbol: "TEST".to_string(),
            close_time_utc,
            o,
            h,
            l,
            c,
            v,
            origin: DataOrigin::Live,
        }
    }

    #[test]
    fn rejects_duplicate_or_old() {
        let validator = BarValidator::new(BarValidationConfig::new(60));
        let bar = bar(120, 1.0, 2.0, 1.0, 1.5, 10.0);
        let result = validator.validate(&bar, Some(120));
        assert_eq!(result, Err(RejectReason::DuplicateOrOld));
    }

    #[test]
    fn rejects_bad_ohlc() {
        let validator = BarValidator::new(BarValidationConfig::new(60));
        let bar = bar(120, 2.0, 1.5, 1.0, 2.0, 1.0);
        let result = validator.validate(&bar, None);
        assert_eq!(result, Err(RejectReason::BadOhlc));
    }

    #[test]
    fn rejects_four_price_doji_when_disabled() {
        let mut cfg = BarValidationConfig::new(60);
        cfg.allow_four_price_doji = false;
        let validator = BarValidator::new(cfg);
        let bar = bar(120, 1.0, 1.0, 1.0, 1.0, 1.0);
        let result = validator.validate(&bar, None);
        assert_eq!(result, Err(RejectReason::FourPriceDoji));
    }

    #[test]
    fn accepts_happy_path() {
        let validator = BarValidator::new(BarValidationConfig::new(60));
        let bar = bar(120, 1.0, 2.0, 1.0, 1.5, 10.0);
        let result = validator.validate(&bar, None);
        assert!(result.is_ok());
    }
}
