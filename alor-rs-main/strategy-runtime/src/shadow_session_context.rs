//! Opt-in context-only core for IMOEXF/USD reaction experiments. No Strategy,
//! Intent or broker route. The collector submodule supplies a read-only durable
//! ingress bridge; the core alone does not attest real receipts or a calendar.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{BarEvent, DataOrigin};

pub mod collector;

const VERSION: &str = "weekend_context_cutoff_v1";
const MAX_INPUTS: usize = 12_000;
const MAX_FREEZES: usize = 128;
const MAX_BYTES: usize = 32 * 1024 * 1024;

fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn local(ts: i64) -> Result<NaiveDateTime> {
    DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(Duration::hours(3)))
        .context("invalid context timestamp")
}
fn transition() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 14).unwrap()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    WeekdayOnly,
    WeekendContextNoTrades,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    ImoexfReaction07,
    UsdrubfPreviousDay07,
    UsdrubfPreviousDay09,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub profile: Profile,
    pub symbol: String,
    pub model_config_sha256: String,
    pub receipt_stream: String,
    pub policy: Policy,
    pub excluded_dates: BTreeSet<NaiveDate>,
}
impl Identity {
    pub fn validate(&self) -> Result<()> {
        let symbol = match self.profile {
            Profile::ImoexfReaction07 => "IMOEXF",
            _ => "USDRUBF",
        };
        ensure!(self.symbol == symbol, "profile/contract mismatch");
        ensure!(
            !self.receipt_stream.is_empty() && self.receipt_stream.len() <= 256,
            "invalid receipt stream"
        );
        ensure!(
            self.model_config_sha256.len() == 64
                && self
                    .model_config_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "invalid config hash"
        );
        ensure!(
            self.excluded_dates.len() <= 256,
            "oversized calendar exclusions"
        );
        Ok(())
    }
    pub fn opening_hour(&self, date: NaiveDate) -> u32 {
        if date < transition() || self.profile == Profile::UsdrubfPreviousDay09 {
            9
        } else {
            7
        }
    }
    fn trading_date(&self, d: NaiveDate) -> bool {
        d.weekday().num_days_from_monday() < 5 && !self.excluded_dates.contains(&d)
    }
    pub fn cutoff(&self, d: NaiveDate) -> i64 {
        (d.and_hms_opt(self.opening_hour(d), 0, 0).unwrap() - Duration::hours(3))
            .and_utc()
            .timestamp()
    }
    fn next_cutoff(&self, mut d: NaiveDate) -> Result<i64> {
        loop {
            d = d.succ_opt().context("date overflow")?;
            if self.trading_date(d) {
                return Ok(self.cutoff(d));
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub sequence: u64,
    pub source_id: String,
    pub bar: BarEvent,
    pub received_at_utc: Option<i64>,
    pub durable_at_utc: Option<i64>,
}
impl Receipt {
    fn payload_hash(&self) -> Result<String> {
        hash(&(
            &self.source_id,
            &self.bar,
            self.received_at_utc,
            self.durable_at_utc,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reject {
    WrongContract,
    NonModelBar,
    WeekendDisabled,
    ExcludedDate,
    InvalidOhlcv,
    MissingReceipt,
    MissingDurability,
    InvalidClock,
    BarNotClosed,
    AtOrAfterCutoff,
    UnsupportedOrigin,
    ConflictingReceipt,
    ConflictingQuote,
    BehindFrozenBarrier,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum Route {
    ContextOnly,
    Duplicate,
    Quarantined(Reject),
}
impl Route {
    pub fn context_update_allowed(&self) -> bool {
        *self == Self::ContextOnly
    }
    pub fn order_emission_allowed(&self) -> bool {
        false
    }
}

/// Completeness attestation supplied by a durable serialized ingress, not max bar time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Barrier {
    pub receipt_stream: String,
    pub through_utc: i64,
    pub last_sequence: u64,
    pub ingress_sha256: String,
    pub attested_at_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub session_date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub range: f64,
    pub distinct_bars: usize,
    pub required_bars: usize,
    pub total_slots: usize,
    pub first_bar_utc: i64,
    pub last_bar_utc: i64,
    pub latest_received_at_utc: i64,
    pub latest_durable_at_utc: i64,
    pub source_sequences: Vec<u64>,
    pub source_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Skipped {
    pub session_date: NaiveDate,
    pub distinct_bars: usize,
    pub required_bars: usize,
    pub first_bar_utc: i64,
    pub last_bar_utc: i64,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Frozen {
    pub identity_sha256: String,
    pub session_date: NaiveDate,
    pub cutoff_utc: i64,
    pub frozen_at_utc: i64,
    pub barrier: Barrier,
    pub previous: Option<Anchor>,
    pub previous2: Option<Anchor>,
    pub skipped: Vec<Skipped>,
}
impl Frozen {
    pub fn ready(&self) -> bool {
        self.previous.is_some() && self.previous2.is_some()
    }
    pub fn previous_return(&self) -> Option<f64> {
        Some(self.previous.as_ref()?.close / self.previous2.as_ref()?.close - 1.)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Bootstrap {
        receipts: Vec<Receipt>,
        first_decision_session: NaiveDate,
        processed_at_utc: i64,
    },
    Ingest {
        receipt: Receipt,
        processed_at_utc: i64,
        route: Route,
    },
    Freeze {
        value: Box<Frozen>,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: String,
    identity: Identity,
    operations: Vec<Operation>,
    clock_floor: Option<i64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: Checkpoint,
    sha256: String,
}

#[derive(Clone)]
pub struct Store {
    identity: Identity,
    operations: Vec<Operation>,
    inputs: Vec<Receipt>,
    canonical: BTreeMap<i64, Receipt>,
    sources: BTreeMap<String, String>,
    frozen: BTreeMap<NaiveDate, Frozen>,
    clock: Option<i64>,
}
impl Store {
    pub fn new(identity: Identity) -> Result<Self> {
        identity.validate()?;
        Ok(Self {
            identity,
            operations: vec![],
            inputs: vec![],
            canonical: BTreeMap::new(),
            sources: BTreeMap::new(),
            frozen: BTreeMap::new(),
            clock: None,
        })
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn last_sequence(&self) -> u64 {
        self.inputs.len() as u64
    }
    pub fn ingress_sha256(&self) -> Result<String> {
        hash(&self.inputs)
    }
    pub fn accepted_count(&self) -> usize {
        self.canonical.len()
    }
    pub fn frozen(&self, date: NaiveDate) -> Option<&Frozen> {
        self.frozen.get(&date)
    }
    fn check_clock(&self, now: i64) -> Result<()> {
        local(now)?;
        ensure!(
            self.clock.is_none_or(|last| now >= last),
            "processing clock regression"
        );
        Ok(())
    }
    fn rejection(
        &self,
        i: &Receipt,
        now: i64,
        bootstrap_cutoff: Option<i64>,
    ) -> Result<Option<Reject>> {
        use Reject::*;
        let b = &i.bar;
        if b.symbol != self.identity.symbol {
            return Ok(Some(WrongContract));
        }
        let dt = local(b.close_time_utc)?;
        let d = dt.date();
        if self.identity.excluded_dates.contains(&d) {
            return Ok(Some(ExcludedDate));
        }
        let weekend = d.weekday().num_days_from_monday() >= 5;
        if weekend && self.identity.policy == Policy::WeekdayOnly {
            return Ok(Some(WeekendDisabled));
        }
        let minute = dt.hour() * 60 + dt.minute();
        let (first, last) = if weekend {
            (590, 1130)
        } else {
            (self.identity.opening_hour(d) * 60, 1420)
        };
        if minute < first || minute > last || !dt.minute().is_multiple_of(10) || dt.second() != 0 {
            return Ok(Some(NonModelBar));
        }
        if b.l <= 0.
            || b.v < 0.
            || b.o < b.l
            || b.o > b.h
            || b.close < b.l
            || b.close > b.h
            || b.h < b.l
        {
            return Ok(Some(InvalidOhlcv));
        }
        let Some(received) = i.received_at_utc else {
            return Ok(Some(MissingReceipt));
        };
        let Some(durable) = i.durable_at_utc else {
            return Ok(Some(MissingDurability));
        };
        if local(received).is_err()
            || local(durable).is_err()
            || received > durable
            || durable > now
        {
            return Ok(Some(InvalidClock));
        }
        if received < b.close_time_utc + 600 {
            return Ok(Some(BarNotClosed));
        }
        if durable >= bootstrap_cutoff.unwrap_or(self.identity.next_cutoff(d)?) {
            return Ok(Some(AtOrAfterCutoff));
        }
        if b.origin == DataOrigin::Replay {
            return Ok(Some(UnsupportedOrigin));
        }
        Ok(None)
    }
    pub fn ingest(&mut self, i: Receipt, now: i64) -> Result<Route> {
        self.ingest_inner(i, now, None)
    }
    fn ingest_inner(
        &mut self,
        i: Receipt,
        now: i64,
        bootstrap_cutoff: Option<i64>,
    ) -> Result<Route> {
        self.check_clock(now)?;
        ensure!(
            self.inputs.len() < MAX_INPUTS,
            "context input limit; rotate explicitly"
        );
        ensure!(
            i.sequence == self.last_sequence() + 1,
            "missing/repeated ingress sequence"
        );
        ensure!(
            !i.source_id.is_empty() && i.source_id.len() <= 256,
            "invalid source id"
        );
        ensure!(
            [i.bar.o, i.bar.h, i.bar.l, i.bar.close, i.bar.v]
                .iter()
                .all(|v| v.is_finite()),
            "nonfinite raw input; preserve separately, not JSON null"
        );
        let fingerprint = i.payload_hash()?;
        let route = if let Some(old) = self.sources.get(&i.source_id) {
            if old == &fingerprint {
                Route::Duplicate
            } else {
                Route::Quarantined(Reject::ConflictingReceipt)
            }
        } else if let Some(reason) = self.rejection(&i, now, bootstrap_cutoff)? {
            Route::Quarantined(reason)
        } else if let Some(old) = self.canonical.get(&i.bar.close_time_utc) {
            let quote =
                |r: &Receipt| [r.bar.o, r.bar.h, r.bar.l, r.bar.close, r.bar.v].map(f64::to_bits);
            if quote(old) == quote(&i) {
                Route::Duplicate
            } else {
                Route::Quarantined(Reject::ConflictingQuote)
            }
        } else if self.frozen.values().any(|f| {
            i.bar.close_time_utc < f.cutoff_utc
                && i.durable_at_utc.is_some_and(|d| d < f.cutoff_utc)
        }) {
            Route::Quarantined(Reject::BehindFrozenBarrier)
        } else {
            Route::ContextOnly
        };
        if route.context_update_allowed() {
            self.canonical.insert(i.bar.close_time_utc, i.clone());
        }
        self.sources
            .entry(i.source_id.clone())
            .or_insert(fingerprint);
        self.inputs.push(i.clone());
        self.operations.push(Operation::Ingest {
            receipt: i,
            processed_at_utc: now,
            route: route.clone(),
        });
        self.clock = Some(now);
        Ok(route)
    }

    // Not a retrospective receipt override: this one-time context import is
    // admitted only before a future activation and retains the actual clocks.
    fn bootstrap(&mut self, receipts: &[Receipt], day: NaiveDate, now: i64) -> Result<()> {
        self.check_clock(now)?;
        ensure!(
            self.inputs.is_empty() && self.frozen.is_empty(),
            "bootstrap requires empty context"
        );
        ensure!(
            self.identity.trading_date(day) && now < self.identity.cutoff(day),
            "bootstrap must precede first decision cutoff"
        );
        ensure!(
            !receipts.is_empty() && receipts.len() <= 1024,
            "bootstrap row bound"
        );
        let mut staged = self.clone();
        for input in receipts {
            let dt = local(input.bar.close_time_utc)?;
            let date = dt.date();
            ensure!(
                dt.minute().is_multiple_of(10) && dt.second() == 0,
                "bootstrap requires M10 source grid"
            );
            ensure!(
                date < day && day.signed_duration_since(date).num_days() <= 14,
                "bootstrap outside bounded historical window"
            );
            // Validate source/clock even for a service/weekend bar that will be
            // quarantined by the ordinary calendar/model-feed policy.
            ensure!(
                input.bar.symbol == self.identity.symbol && input.bar.origin != DataOrigin::Replay,
                "bootstrap contract/origin mismatch"
            );
            let received = input.received_at_utc.context("bootstrap missing receipt")?;
            let durable = input
                .durable_at_utc
                .context("bootstrap missing durability")?;
            ensure!(
                received >= input.bar.close_time_utc + 600 && received <= durable && durable <= now,
                "invalid bootstrap clocks"
            );
            ensure!(
                input.bar.l > 0.
                    && input.bar.v >= 0.
                    && input.bar.o >= input.bar.l
                    && input.bar.o <= input.bar.h
                    && input.bar.close >= input.bar.l
                    && input.bar.close <= input.bar.h,
                "invalid bootstrap OHLCV"
            );
            let route = staged.ingest_inner(input.clone(), now, Some(self.identity.cutoff(day)))?;
            ensure!(
                matches!(
                    route,
                    Route::ContextOnly
                        | Route::Duplicate
                        | Route::Quarantined(
                            Reject::NonModelBar | Reject::WeekendDisabled | Reject::ExcludedDate
                        )
                ),
                "invalid bootstrap receipt: {route:?}"
            );
        }
        let (anchors, _) = staged.anchors_before(day)?;
        ensure!(anchors.len() >= 2, "bootstrap paired anchors not ready");
        let regular: BTreeSet<_> = anchors
            .iter()
            .filter(|a| self.identity.trading_date(a.session_date))
            .map(|a| a.session_date)
            .collect();
        ensure!(
            (2..=4).contains(&regular.len()),
            "bootstrap requires 2..4 complete regular sessions"
        );
        let mut date = *regular.first().unwrap();
        while date < day {
            ensure!(
                !self.identity.trading_date(date) || regular.contains(&date),
                "bootstrap missing regular session; approve calendar/history explicitly"
            );
            date = date.succ_opt().context("date overflow")?;
        }
        staged.operations = vec![Operation::Bootstrap {
            receipts: receipts.to_vec(),
            first_decision_session: day,
            processed_at_utc: now,
        }];
        *self = staged;
        Ok(())
    }

    fn anchors_before(&self, day: NaiveDate) -> Result<(Vec<Anchor>, Vec<Skipped>)> {
        let cutoff = self.identity.cutoff(day);
        self.collect_anchors(day, cutoff)
    }
    fn freeze(&mut self, day: NaiveDate, now: i64, barrier: &Barrier) -> Result<Frozen> {
        self.check_clock(now)?;
        ensure!(self.identity.trading_date(day), "non-trading freeze date");
        ensure!(
            self.frozen.len() < MAX_FREEZES
                && self.frozen.keys().next_back().is_none_or(|d| *d < day),
            "repeated/backdated freeze or capacity limit"
        );
        let cutoff = self.identity.cutoff(day);
        ensure!(
            barrier.receipt_stream == self.identity.receipt_stream
                && barrier.last_sequence == self.last_sequence()
                && barrier.ingress_sha256 == self.ingress_sha256()?,
            "incomplete/wrong cutoff prefix"
        );
        ensure!(
            cutoff <= barrier.through_utc
                && barrier.through_utc <= barrier.attested_at_utc
                && barrier.attested_at_utc <= now,
            "missing/premature cutoff attestation"
        );
        let (mut anchors, skipped) = self.anchors_before(day)?;
        let previous = anchors.pop();
        let previous2 = anchors.pop();
        let value = Frozen {
            identity_sha256: hash(&(VERSION, &self.identity))?,
            session_date: day,
            cutoff_utc: cutoff,
            frozen_at_utc: now,
            barrier: barrier.clone(),
            previous,
            previous2,
            skipped,
        };
        self.frozen.insert(day, value.clone());
        self.operations.push(Operation::Freeze {
            value: Box::new(value.clone()),
        });
        self.clock = Some(now);
        Ok(value)
    }
    fn collect_anchors(&self, day: NaiveDate, cutoff: i64) -> Result<(Vec<Anchor>, Vec<Skipped>)> {
        let mut days: BTreeMap<NaiveDate, Vec<&Receipt>> = BTreeMap::new();
        for i in self.canonical.values() {
            let d = local(i.bar.close_time_utc)?.date();
            if d < day && i.durable_at_utc.is_some_and(|r| r < cutoff) {
                days.entry(d).or_default().push(i);
            }
        }
        let (mut anchors, mut skipped) = (vec![], vec![]);
        for (d, bars) in days {
            let weekend = d.weekday().num_days_from_monday() >= 5;
            let total = if weekend {
                55
            } else {
                ((1420 - self.identity.opening_hour(d) * 60) / 10 + 1) as usize
            };
            let required = if weekend { 44 } else { total };
            let (first, last) = (bars[0], bars[bars.len() - 1]);
            let first_local = local(first.bar.close_time_utc)?;
            let last_local = local(last.bar.close_time_utc)?;
            let first_limit = if weekend {
                600
            } else {
                self.identity.opening_hour(d) * 60
            };
            let end = if weekend { 1130 } else { 1420 };
            if bars.len() < required
                || first_local.hour() * 60 + first_local.minute() > first_limit
                || last_local.hour() * 60 + last_local.minute() < end
            {
                skipped.push(Skipped {
                    session_date: d,
                    distinct_bars: bars.len(),
                    required_bars: required,
                    first_bar_utc: first.bar.close_time_utc,
                    last_bar_utc: last.bar.close_time_utc,
                    reason: "incomplete_context".into(),
                });
                continue;
            }
            let high = bars
                .iter()
                .map(|i| i.bar.h)
                .fold(f64::NEG_INFINITY, f64::max);
            let low = bars.iter().map(|i| i.bar.l).fold(f64::INFINITY, f64::min);
            anchors.push(Anchor {
                session_date: d,
                open: first.bar.o,
                high,
                low,
                close: last.bar.close,
                range: high - low,
                distinct_bars: bars.len(),
                required_bars: required,
                total_slots: total,
                first_bar_utc: first.bar.close_time_utc,
                last_bar_utc: last.bar.close_time_utc,
                latest_received_at_utc: bars
                    .iter()
                    .map(|i| i.received_at_utc.unwrap())
                    .max()
                    .unwrap(),
                latest_durable_at_utc: bars
                    .iter()
                    .map(|i| i.durable_at_utc.unwrap())
                    .max()
                    .unwrap(),
                source_sequences: bars.iter().map(|i| i.sequence).collect(),
                source_sha256: hash(&bars)?,
            });
        }
        Ok((anchors, skipped))
    }
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>> {
        let payload = Checkpoint {
            version: VERSION.into(),
            identity: self.identity.clone(),
            operations: self.operations.clone(),
            clock_floor: self.clock,
        };
        let bytes = serde_json::to_vec(&Envelope {
            sha256: hash(&payload)?,
            payload,
        })?;
        ensure!(bytes.len() <= MAX_BYTES, "context checkpoint limit");
        Ok(bytes)
    }
    pub fn restore(expected: &Identity, bytes: &[u8], now: i64) -> Result<Self> {
        expected.validate()?;
        ensure!(bytes.len() <= MAX_BYTES, "context checkpoint limit");
        let saved: Envelope = serde_json::from_slice(bytes)?;
        let p = saved.payload;
        ensure!(
            p.version == VERSION && p.identity == *expected && hash(&p)? == saved.sha256,
            "checkpoint identity/checksum mismatch"
        );
        ensure!(
            p.operations.len() <= MAX_INPUTS + MAX_FREEZES,
            "context operation limit"
        );
        let mut result = Self::new(expected.clone())?;
        for op in &p.operations {
            match op {
                Operation::Bootstrap {
                    receipts,
                    first_decision_session,
                    processed_at_utc,
                } => {
                    result.bootstrap(receipts, *first_decision_session, *processed_at_utc)?;
                }
                Operation::Ingest {
                    receipt,
                    processed_at_utc,
                    route,
                } => {
                    ensure!(
                        &result.ingest(receipt.clone(), *processed_at_utc)? == route,
                        "replayed routing mismatch"
                    );
                }
                Operation::Freeze { value } => {
                    ensure!(
                        &result.freeze(value.session_date, value.frozen_at_utc, &value.barrier)?
                            == value.as_ref(),
                        "replayed frozen context mismatch"
                    );
                }
            }
        }
        ensure!(
            result.clock <= p.clock_floor && p.clock_floor.is_none_or(|t| t <= now),
            "invalid restart clock"
        );
        local(now)?;
        result.clock = Some(now);
        Ok(result)
    }
    pub fn save_new_checkpoint(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let tmp = parent.join(format!(".weekend-context-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            file.write_all(&self.checkpoint_bytes()?)?;
            file.sync_all()?;
            fs::hard_link(&tmp, path)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        let cleanup = fs::remove_file(&tmp);
        result?;
        cleanup?;
        Ok(())
    }
    pub fn freeze_and_save_new(
        &mut self,
        day: NaiveDate,
        now: i64,
        barrier: &Barrier,
        path: &Path,
    ) -> Result<&Frozen> {
        let mut staged = self.clone();
        staged.freeze(day, now, barrier)?;
        staged.save_new_checkpoint(path)?;
        *self = staged;
        Ok(self.frozen.get(&day).unwrap())
    }
}

#[cfg(test)]
#[path = "shadow_session_context/tests.rs"]
mod tests;
