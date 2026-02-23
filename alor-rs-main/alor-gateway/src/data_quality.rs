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
    EmitFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum IgnoredReason {
    OldGeneration,
    InactiveGuid,
    UnknownGuid,
    JsonParseError,
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
    pub ignored: HashMap<(String, IgnoredReason), u64>,
    pub last_close_time: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct DropReasonCount {
    pub symbol: String,
    pub reason: RejectReason,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct IgnoredReasonCount {
    pub symbol: String,
    pub reason: IgnoredReason,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PendingAcks {
    pub bars: u64,
    pub positions: u64,
    pub orders: u64,
    pub authorize: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct PendingReport {
    pub buffered_bars: HashMap<String, u64>,
    pub pending_acks: PendingAcks,
}

#[derive(Debug, Serialize)]
pub struct DataQualityReport {
    pub received: HashMap<String, u64>,
    pub emitted: HashMap<String, u64>,
    pub dropped: Vec<DropReasonCount>,
    pub ignored: Vec<IgnoredReasonCount>,
    pub pending: PendingReport,
    pub last_close_time: HashMap<String, i64>,
}

#[derive(Debug)]
pub struct ReportImbalance {
    pub symbol: String,
    pub received: u64,
    pub emitted: u64,
    pub dropped: u64,
    pub ignored: u64,
    pub pending: u64,
    pub delta: i64,
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

    pub fn record_ignored(&mut self, symbol: &str, reason: IgnoredReason) {
        *self
            .ignored
            .entry((symbol.to_string(), reason))
            .or_insert(0) += 1;
    }

    pub fn to_report(&self, pending: PendingReport) -> DataQualityReport {
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
        let mut ignored: Vec<IgnoredReasonCount> = self
            .ignored
            .iter()
            .map(|((symbol, reason), count)| IgnoredReasonCount {
                symbol: symbol.clone(),
                reason: *reason,
                count: *count,
            })
            .collect();
        ignored.sort_by(|a, b| b.count.cmp(&a.count));
        DataQualityReport {
            received: self.received.clone(),
            emitted: self.emitted.clone(),
            dropped,
            ignored,
            pending,
            last_close_time: self.last_close_time.clone(),
        }
    }
}

pub fn build_data_report(
    stats: &BarQualityStats,
    pending: PendingReport,
) -> (DataQualityReport, Vec<ReportImbalance>) {
    let report = stats.to_report(pending);
    let mut imbalances = Vec::new();
    for (symbol, received) in &report.received {
        let emitted = report.emitted.get(symbol).copied().unwrap_or(0);
        let dropped = report
            .dropped
            .iter()
            .filter(|entry| entry.symbol == *symbol)
            .map(|entry| entry.count)
            .sum::<u64>();
        let ignored = report
            .ignored
            .iter()
            .filter(|entry| entry.symbol == *symbol)
            .map(|entry| entry.count)
            .sum::<u64>();
        let pending_buffered = report
            .pending
            .buffered_bars
            .get(symbol)
            .copied()
            .unwrap_or(0);
        let pending_total = pending_buffered;
        let rhs = emitted + dropped + ignored + pending_total;
        if *received != rhs {
            let delta = *received as i64 - rhs as i64;
            imbalances.push(ReportImbalance {
                symbol: symbol.clone(),
                received: *received,
                emitted,
                dropped,
                ignored,
                pending: pending_total,
                delta,
            });
        }
    }
    (report, imbalances)
}

pub fn write_data_report(
    path: &str,
    report: &DataQualityReport,
) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    serde_json::to_writer_pretty(file, report)?;
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
