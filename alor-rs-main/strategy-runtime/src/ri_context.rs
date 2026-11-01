//! Opt-in, context-only RI recovery core. Not wired to live/shadow ingress yet.
//!
//! Receipt evidence and the cutoff barrier must come from a durable ingress journal,
//! not Utc::now() or a bar's label. This module cannot attest the upstream journal.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::strategy_host::{BarEvent, DataOrigin};

pub mod ingress;
pub mod runner;

const VERSION: &str = "ri_frozen_context_checkpoint_v1";
const LIMIT: usize = 50_000;
const MAX_FROZEN_SESSIONS: usize = 128;
const MAX_CHECKPOINT_BYTES: usize = 64 * 1024 * 1024;

fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn local(epoch: i64) -> Result<NaiveDateTime> {
    DateTime::from_timestamp(epoch, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(Duration::hours(3)))
        .context("invalid Moscow timestamp")
}

fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPolicy {
    RuntimeAcceptedOnly,
    RiConfirmedClosingContextV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextIdentity {
    pub profile_id: String,
    pub symbol: String,
    pub numerical_config_sha256: String,
    pub model_feed_policy: String,
    pub receipt_stream: String,
    pub policy: ContextPolicy,
    pub model_open_hour: u32,
    pub excluded_dates: BTreeSet<NaiveDate>,
}

impl ContextIdentity {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.profile_id.is_empty()
                && !self.symbol.is_empty()
                && !self.receipt_stream.is_empty(),
            "empty context identity"
        );
        ensure!(
            self.numerical_config_sha256.len() == 64
                && self
                    .numerical_config_sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit()),
            "invalid config hash"
        );
        ensure!(
            self.model_feed_policy == "micro_regular_v1",
            "context comparison requires micro_regular_v1"
        );
        ensure!(
            matches!(self.model_open_hour, 7 | 9),
            "unsupported model clock"
        );
        Ok(())
    }

    fn opening_hour(&self, date: NaiveDate) -> u32 {
        if date < NaiveDate::from_ymd_opt(2026, 7, 14).unwrap() {
            9
        } else {
            self.model_open_hour
        }
    }

    fn regular_date(&self, date: NaiveDate) -> bool {
        date.weekday().num_days_from_monday() < 5 && !self.excluded_dates.contains(&date)
    }

    pub fn cutoff(&self, date: NaiveDate) -> i64 {
        (date.and_time(time(self.opening_hour(date), 0)) - Duration::hours(3))
            .and_utc()
            .timestamp()
    }

    fn next_cutoff(&self, date: NaiveDate) -> Result<i64> {
        let mut next = date.succ_opt().context("date overflow")?;
        while !self.regular_date(next) {
            next = next.succ_opt().context("date overflow")?;
        }
        Ok(self.cutoff(next))
    }
}

/// Original durable ingestion evidence; processing time is deliberately separate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContextInput {
    pub sequence: u64,
    pub bar: BarEvent,
    pub received_at_utc: Option<i64>,
    pub durable_at_utc: Option<i64>,
}

impl ContextInput {
    fn digest(&self) -> Result<String> {
        // Bits retain rejected NaN/Infinity payload fingerprints without serializing
        // them as JSON null. Rejected raw payloads belong to the ingress audit log.
        hash(&(
            self.sequence,
            &self.bar.symbol,
            self.bar.close_time_utc,
            [
                self.bar.o,
                self.bar.h,
                self.bar.l,
                self.bar.close,
                self.bar.v,
            ]
            .map(f64::to_bits),
            &self.bar.origin,
            self.received_at_utc,
            self.durable_at_utc,
        ))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineReason {
    WrongContract,
    NonModelBar,
    InvalidOhlcv,
    MissingReceipt,
    MissingDurability,
    InvalidReceiptClock,
    BarNotClosed,
    AtOrAfterCutoff,
    RecoveryDisabled,
    NonClosingRecovery,
    UnsupportedOrigin,
    ConflictingQuote,
    BehindFrozenBarrier,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum ContextRoute {
    ContextOnly,
    DuplicateQuote,
    Quarantined(QuarantineReason),
}

impl ContextRoute {
    pub fn context_update_allowed(self) -> bool {
        self == Self::ContextOnly
    }
    pub fn order_emission_allowed(self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReceiptRecord {
    sequence: u64,
    input_sha256: String,
    processed_at_utc: i64,
    route: ContextRoute,
}

/// An upstream attestation of COMPLETE journal consumption through the cutoff.
/// A runtime must not manufacture this from its latest observed bar timestamp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReceiptBarrier {
    pub receipt_stream: String,
    pub through_utc: i64,
    pub last_sequence: u64,
    pub ingress_sha256: String,
    pub attested_at_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionAnchor {
    pub session_date: NaiveDate,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub range: f64,
    pub distinct_bars: usize,
    pub first_bar_utc: i64,
    pub last_bar_utc: i64,
    pub latest_durable_at_utc: i64,
    pub required_bars: usize,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkippedSession {
    pub date: NaiveDate,
    pub distinct_bars: usize,
    pub first_bar_utc: i64,
    pub last_bar_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FrozenContext {
    pub identity_sha256: String,
    pub session_date: NaiveDate,
    pub cutoff_utc: i64,
    pub frozen_at_utc: i64,
    pub barrier: ReceiptBarrier,
    pub previous: Option<SessionAnchor>,
    pub previous2: Option<SessionAnchor>,
    pub skipped_incomplete: Vec<SkippedSession>,
}

impl FrozenContext {
    pub fn ready(&self) -> bool {
        self.previous.is_some() && self.previous2.is_some()
    }
    pub fn previous_return(&self) -> Option<f64> {
        Some(self.previous.as_ref()?.close / self.previous2.as_ref()?.close - 1.0)
    }
    pub fn sha256(&self) -> Result<String> {
        hash(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: String,
    identity: ContextIdentity,
    canonical: BTreeMap<i64, ContextInput>,
    journal: Vec<ReceiptRecord>,
    frozen: BTreeMap<NaiveDate, FrozenContext>,
    last_processed_at_utc: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointEnvelope {
    payload: Checkpoint,
    payload_sha256: String,
}

/// No Strategy implementation or Intent return type: this gate cannot trade.
#[derive(Clone)]
pub struct RiContextStore {
    state: Checkpoint,
}

impl RiContextStore {
    pub fn identity(&self) -> &ContextIdentity {
        &self.state.identity
    }

    pub fn verified_route(&self, input: &ContextInput) -> Result<ContextRoute> {
        ensure!(input.sequence > 0, "invalid ingress sequence");
        let record = self
            .state
            .journal
            .get((input.sequence - 1) as usize)
            .context("input has not passed the context gate")?;
        ensure!(
            record.input_sha256 == input.digest()?,
            "context input digest mismatch"
        );
        Ok(record.route)
    }

    pub fn input_prefix_sha256(&self, sequence: u64) -> Result<String> {
        self.ingress_hash_through(sequence)
    }

    pub fn new(identity: ContextIdentity) -> Result<Self> {
        identity.validate()?;
        Ok(Self {
            state: Checkpoint {
                version: VERSION.into(),
                identity,
                canonical: BTreeMap::new(),
                journal: vec![],
                frozen: BTreeMap::new(),
                last_processed_at_utc: None,
            },
        })
    }

    fn validate_input(
        &self,
        input: &ContextInput,
        processed: i64,
    ) -> Result<Option<QuarantineReason>> {
        use QuarantineReason::*;
        let cfg = &self.state.identity;
        let b = &input.bar;
        if b.symbol != cfg.symbol {
            return Ok(Some(WrongContract));
        }
        let Ok(dt) = local(b.close_time_utc) else {
            return Ok(Some(NonModelBar));
        };
        if !cfg.regular_date(dt.date())
            || dt.time() < time(cfg.opening_hour(dt.date()), 0)
            || dt.time() > time(23, 49)
            || !dt.minute().is_multiple_of(10)
            || dt.second() != 0
        {
            return Ok(Some(NonModelBar));
        }
        if [b.o, b.h, b.l, b.close, b.v].iter().any(|v| !v.is_finite())
            || b.l <= 0.0
            || b.v < 0.0
            || b.o < b.l
            || b.o > b.h
            || b.close < b.l
            || b.close > b.h
            || b.h < b.l
        {
            return Ok(Some(InvalidOhlcv));
        }
        let Some(received) = input.received_at_utc else {
            return Ok(Some(MissingReceipt));
        };
        let Some(durable) = input.durable_at_utc else {
            return Ok(Some(MissingDurability));
        };
        if local(received).is_err()
            || local(durable).is_err()
            || received > durable
            || durable > processed
        {
            return Ok(Some(InvalidReceiptClock));
        }
        if received < b.close_time_utc + 600 {
            return Ok(Some(BarNotClosed));
        }
        if durable >= cfg.next_cutoff(dt.date())? {
            return Ok(Some(AtOrAfterCutoff));
        }
        match b.origin {
            DataOrigin::HistoryGap if cfg.policy == ContextPolicy::RuntimeAcceptedOnly => {
                return Ok(Some(RecoveryDisabled))
            }
            DataOrigin::HistoryGap if dt.time() != time(23, 40) => {
                return Ok(Some(NonClosingRecovery))
            }
            DataOrigin::Replay => return Ok(Some(UnsupportedOrigin)),
            _ => {}
        }
        Ok(None)
    }

    fn check_clock(&self, processed: i64) -> Result<()> {
        local(processed)?;
        ensure!(
            self.state
                .last_processed_at_utc
                .is_none_or(|p| p <= processed),
            "processing clock regression"
        );
        Ok(())
    }

    pub fn ingest(&mut self, input: ContextInput, processed_at_utc: i64) -> Result<ContextRoute> {
        self.check_clock(processed_at_utc)?;
        let digest = input.digest()?;
        ensure!(input.sequence > 0, "ingress sequence starts at one");
        if input.sequence <= self.state.journal.len() as u64 {
            let old = &self.state.journal[(input.sequence - 1) as usize];
            ensure!(old.input_sha256 == digest, "conflicting ingress sequence");
            // Re-delivery never grants a second context update, including after restore.
            self.state.last_processed_at_utc = Some(processed_at_utc);
            return Ok(ContextRoute::DuplicateQuote);
        }
        ensure!(
            input.sequence == self.state.journal.len() as u64 + 1,
            "missing ingress sequence"
        );
        ensure!(
            self.state.journal.len() < LIMIT,
            "context journal full; explicit rotation required"
        );
        let reason = self.validate_input(&input, processed_at_utc)?;
        let route = if let Some(reason) = reason {
            ContextRoute::Quarantined(reason)
        } else if let Some(old) = self.state.canonical.get(&input.bar.close_time_utc) {
            let values = |i: &ContextInput| {
                [i.bar.o, i.bar.h, i.bar.l, i.bar.close, i.bar.v].map(f64::to_bits)
            };
            if values(old) == values(&input) {
                ContextRoute::DuplicateQuote
            } else {
                ContextRoute::Quarantined(QuarantineReason::ConflictingQuote)
            }
        } else if self.state.frozen.values().any(|f| {
            input.bar.close_time_utc < f.cutoff_utc
                && input.durable_at_utc.is_some_and(|d| d < f.cutoff_utc)
        }) {
            // A new pre-cutoff input contradicts a previously complete barrier.
            ContextRoute::Quarantined(QuarantineReason::BehindFrozenBarrier)
        } else {
            ContextRoute::ContextOnly
        };
        let sequence = input.sequence;
        if route.context_update_allowed() {
            self.state.canonical.insert(input.bar.close_time_utc, input);
        }
        self.state.journal.push(ReceiptRecord {
            sequence,
            input_sha256: digest,
            processed_at_utc,
            route,
        });
        self.state.last_processed_at_utc = Some(processed_at_utc);
        Ok(route)
    }

    pub fn last_sequence(&self) -> u64 {
        self.state.journal.len() as u64
    }

    fn ingress_hash_through(&self, last: u64) -> Result<String> {
        ensure!(last <= self.last_sequence(), "incomplete ingress journal");
        hash(
            &self.state.journal[..last as usize]
                .iter()
                .map(|r| (r.sequence, &r.input_sha256))
                .collect::<Vec<_>>(),
        )
    }

    /// Comparison digest only, not an upstream completeness attestation.
    pub fn ingress_sha256(&self) -> Result<String> {
        self.ingress_hash_through(self.last_sequence())
    }

    fn build_frozen(
        &self,
        date: NaiveDate,
        now: i64,
        barrier: &ReceiptBarrier,
    ) -> Result<FrozenContext> {
        let cfg = &self.state.identity;
        let cutoff = cfg.cutoff(date);
        ensure!(
            cfg.regular_date(date),
            "cannot freeze a non-regular decision session"
        );
        ensure!(now >= cutoff, "freeze before model-open cutoff");
        ensure!(
            barrier.receipt_stream == cfg.receipt_stream,
            "wrong receipt stream"
        );
        ensure!(
            barrier.through_utc >= cutoff
                && barrier.attested_at_utc >= barrier.through_utc
                && barrier.attested_at_utc <= now,
            "missing or premature cutoff attestation"
        );
        ensure!(
            barrier.ingress_sha256 == self.ingress_hash_through(barrier.last_sequence)?,
            "ingress barrier hash mismatch"
        );
        ensure!(
            barrier.last_sequence == 0
                || self.state.journal[(barrier.last_sequence - 1) as usize].processed_at_utc <= now,
            "barrier includes unprocessed future evidence"
        );
        let mut days: BTreeMap<NaiveDate, Vec<&ContextInput>> = BTreeMap::new();
        for input in self.state.canonical.values() {
            let d = local(input.bar.close_time_utc)?.date();
            if d < date
                && input.sequence <= barrier.last_sequence
                && input.durable_at_utc.is_some_and(|r| r < cutoff)
            {
                days.entry(d).or_default().push(input);
            }
        }
        let mut anchors = vec![];
        let mut skipped = vec![];
        for (d, bars) in days {
            let first = bars[0];
            let last = bars[bars.len() - 1];
            let required = if cfg.opening_hour(d) == 7 { 92 } else { 80 };
            if bars.len() < required
                || local(first.bar.close_time_utc)?.time() > time(cfg.opening_hour(d), 10)
                || local(last.bar.close_time_utc)?.time() < time(23, 30)
            {
                skipped.push(SkippedSession {
                    date: d,
                    distinct_bars: bars.len(),
                    first_bar_utc: first.bar.close_time_utc,
                    last_bar_utc: last.bar.close_time_utc,
                });
                continue;
            }
            let high = bars
                .iter()
                .map(|b| b.bar.h)
                .fold(f64::NEG_INFINITY, f64::max);
            let low = bars.iter().map(|b| b.bar.l).fold(f64::INFINITY, f64::min);
            anchors.push(SessionAnchor {
                session_date: d,
                high,
                low,
                close: last.bar.close,
                range: high - low,
                distinct_bars: bars.len(),
                first_bar_utc: first.bar.close_time_utc,
                last_bar_utc: last.bar.close_time_utc,
                latest_durable_at_utc: bars
                    .iter()
                    .map(|i| i.durable_at_utc.unwrap())
                    .max()
                    .unwrap(),
                required_bars: required,
                source_sha256: hash(&bars)?,
            });
        }
        let previous = anchors.pop();
        let previous2 = anchors.pop();
        Ok(FrozenContext {
            identity_sha256: hash(cfg)?,
            session_date: date,
            cutoff_utc: cutoff,
            frozen_at_utc: now,
            barrier: barrier.clone(),
            previous,
            previous2,
            skipped_incomplete: skipped,
        })
    }

    fn freeze(&mut self, date: NaiveDate, now: i64, barrier: &ReceiptBarrier) -> Result<()> {
        self.check_clock(now)?;
        if self.state.frozen.contains_key(&date) {
            return Ok(());
        }
        ensure!(
            self.state.frozen.len() < MAX_FROZEN_SESSIONS,
            "frozen context capacity reached; explicit rotation required"
        );
        ensure!(
            self.state
                .frozen
                .keys()
                .next_back()
                .is_none_or(|last| *last < date),
            "cannot backdate context freeze"
        );
        ensure!(
            barrier.last_sequence == self.last_sequence(),
            "barrier does not cover the consumed journal"
        );
        let snapshot = self.build_frozen(date, now, barrier)?;
        self.state.frozen.insert(date, snapshot);
        self.state.last_processed_at_utc = Some(now);
        Ok(())
    }

    pub fn frozen(&self, date: NaiveDate) -> Option<&FrozenContext> {
        self.state.frozen.get(&date)
    }

    /// Exact original inputs for the two selected anchors, not a relabelled trading feed.
    pub fn frozen_anchor_inputs(&self, date: NaiveDate) -> Result<Vec<&ContextInput>> {
        let frozen = self
            .frozen(date)
            .context("context_unavailable: session not frozen")?;
        ensure!(
            frozen.ready(),
            "context_unavailable: two complete prior sessions required"
        );
        let dates = [
            frozen.previous2.as_ref().unwrap().session_date,
            frozen.previous.as_ref().unwrap().session_date,
        ];
        self.state
            .canonical
            .values()
            .filter_map(|input| {
                let result = local(input.bar.close_time_utc);
                match result {
                    Err(err) => Some(Err(err)),
                    Ok(dt)
                        if dates.contains(&dt.date())
                            && input.sequence <= frozen.barrier.last_sequence
                            && input.durable_at_utc.is_some_and(|d| d < frozen.cutoff_utc) =>
                    {
                        Some(Ok(input))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(&CheckpointEnvelope {
            payload: self.state.clone(),
            payload_sha256: hash(&self.state)?,
        })?;
        ensure!(
            bytes.len() <= MAX_CHECKPOINT_BYTES,
            "oversized context checkpoint"
        );
        Ok(bytes)
    }

    pub fn restore(expected: &ContextIdentity, bytes: &[u8], now: i64) -> Result<Self> {
        expected.validate()?;
        ensure!(
            bytes.len() <= MAX_CHECKPOINT_BYTES,
            "oversized context checkpoint"
        );
        let saved: CheckpointEnvelope = serde_json::from_slice(bytes)?;
        ensure!(
            saved.payload.version == VERSION && saved.payload.identity == *expected,
            "context checkpoint identity mismatch"
        );
        ensure!(
            hash(&saved.payload)? == saved.payload_sha256,
            "context checkpoint checksum mismatch"
        );
        let mut store = Self {
            state: saved.payload,
        };
        ensure!(
            store.state.journal.len() <= LIMIT
                && store.state.frozen.len() <= MAX_FROZEN_SESSIONS
                && store.state.canonical.len() <= store.state.journal.len(),
            "oversized context journal"
        );
        store.check_clock(now)?;
        let mut last = None;
        for (i, record) in store.state.journal.iter().enumerate() {
            ensure!(
                record.sequence == i as u64 + 1
                    && last.is_none_or(|p| p <= record.processed_at_utc)
                    && Some(record.processed_at_utc) <= store.state.last_processed_at_utc,
                "invalid checkpoint receipt ordering"
            );
            last = Some(record.processed_at_utc);
        }
        for (ts, input) in &store.state.canonical {
            ensure!(
                *ts == input.bar.close_time_utc
                    && input.sequence > 0
                    && input.sequence <= store.last_sequence(),
                "invalid canonical key"
            );
            let record = &store.state.journal[(input.sequence - 1) as usize];
            ensure!(
                record.route == ContextRoute::ContextOnly
                    && input.digest()? == record.input_sha256
                    && store
                        .validate_input(input, record.processed_at_utc)?
                        .is_none(),
                "invalid canonical provenance"
            );
        }
        ensure!(
            store
                .state
                .journal
                .iter()
                .filter(|r| r.route == ContextRoute::ContextOnly)
                .count()
                == store.state.canonical.len(),
            "missing canonical payload"
        );
        for (date, snapshot) in &store.state.frozen {
            ensure!(
                *date == snapshot.session_date
                    && Some(snapshot.frozen_at_utc) <= store.state.last_processed_at_utc,
                "invalid frozen date/clock"
            );
            ensure!(
                store.build_frozen(*date, snapshot.frozen_at_utc, &snapshot.barrier)? == *snapshot,
                "frozen context reconstruction mismatch"
            );
        }
        store.state.last_processed_at_utc = Some(now);
        Ok(store)
    }

    /// Publish an immutable complete generation atomically; never overwrite a path.
    pub fn save_new_checkpoint(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        ensure!(!path.exists(), "checkpoint destination exists");
        let temporary = parent.join(format!(".ri-context-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&self.checkpoint_bytes()?)?;
            file.sync_all()?;
            // Same-directory hard link is atomic and fails if the target already exists.
            fs::hard_link(&temporary, path)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        let cleanup = fs::remove_file(&temporary);
        result?;
        cleanup?;
        Ok(())
    }

    /// The caller cannot observe a new frozen context until its complete checkpoint is durable.
    pub fn freeze_and_save_new(
        &mut self,
        date: NaiveDate,
        now: i64,
        barrier: &ReceiptBarrier,
        path: &Path,
    ) -> Result<&FrozenContext> {
        let mut candidate = Self {
            state: self.state.clone(),
        };
        candidate.freeze(date, now, barrier)?;
        candidate.save_new_checkpoint(path)?;
        self.state = candidate.state;
        Ok(self
            .state
            .frozen
            .get(&date)
            .expect("frozen context committed"))
    }
}

#[cfg(test)]
#[path = "ri_context/tests.rs"]
mod tests;
