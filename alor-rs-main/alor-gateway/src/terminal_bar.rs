//! As-observed WS candle release core. No clock, network, publisher or broker.
//! Proposals do not advance watermarks: the owner must reconcile durable writes
//! before commit/restore. Existing Router::start does not enable this policy.
use std::collections::{BTreeMap, VecDeque};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{BarEvent, DataOrigin};

const TF: i64 = 600;
const MAX_SYMBOLS: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCalendar {
    pub weekday_end: NaiveTime,
    pub weekend_end: Option<NaiveTime>,
    #[serde(default)]
    pub overrides: BTreeMap<NaiveDate, Option<NaiveTime>>,
}

impl SessionCalendar {
    pub fn terminal_label(&self, date: NaiveDate) -> Option<i64> {
        let end = self.overrides.get(&date).copied().unwrap_or_else(|| {
            if date.weekday().num_days_from_monday() >= 5 {
                self.weekend_end
            } else {
                Some(self.weekday_end)
            }
        })?;
        Some(date.and_time(end).and_utc().timestamp() - 3 * 3600 - TF)
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.overrides.len() <= 64, "calendar override budget");
        for end in std::iter::once(Some(self.weekday_end))
            .chain(std::iter::once(self.weekend_end))
            .chain(self.overrides.values().copied())
            .flatten()
        {
            let seconds = end.signed_duration_since(NaiveTime::MIN).num_seconds();
            ensure!(
                seconds >= TF && seconds % TF == 0,
                "session end is not on M10 grid"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalPolicy {
    #[serde(default)]
    pub enabled: bool,
    pub first_live_session: NaiveDate,
    pub grace_seconds: i64,
    pub max_lateness_seconds: i64,
    pub max_heartbeat_age_seconds: i64,
    pub max_clock_step_seconds: i64,
    pub symbols: BTreeMap<String, SessionCalendar>,
}

impl TerminalPolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_SYMBOLS).contains(&self.symbols.len()),
            "symbol budget"
        );
        ensure!(
            (5..=30).contains(&self.grace_seconds),
            "terminal grace budget"
        );
        ensure!(
            (30..=90).contains(&self.max_lateness_seconds),
            "terminal lateness budget"
        );
        ensure!(
            self.grace_seconds < self.max_lateness_seconds,
            "grace/lateness order"
        );
        ensure!(
            (1..=60).contains(&self.max_heartbeat_age_seconds),
            "heartbeat budget"
        );
        ensure!(
            (1..=5).contains(&self.max_clock_step_seconds),
            "clock budget"
        );
        for (symbol, calendar) in &self.symbols {
            ensure!(!symbol.is_empty() && symbol.len() <= 64, "invalid symbol");
            calendar.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub utc: i64,
    pub monotonic_ms: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct FeedEvidence {
    pub generation: u64,
    pub connected: bool,
    pub subscribed: bool,
    pub gap_resolved: bool,
    pub last_rx_utc: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    NextTimestamp,
    SessionBoundaryWs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseProof {
    pub policy_sha256: String,
    pub reason: ReleaseReason,
    pub generation: u64,
    pub first_received_at_utc: i64,
    pub last_received_at_utc: i64,
    pub observation_cutoff_utc: i64,
    pub closed_at_utc: i64,
    pub next_label_utc: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub id: String,
    pub bar: BarEvent,
    pub proof: ReleaseProof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevisionKind {
    VolumeOnly,
    Price,
    OlderThanCache,
}

#[derive(Clone, Debug)]
pub enum Observation {
    Buffered,
    Duplicate,
    Ready(Release),
    Revision {
        original: Option<BarEvent>,
        observed: BarEvent,
        kind: RevisionKind,
    },
}

#[derive(Clone)]
struct Buffered {
    bar: BarEvent,
    generation: u64,
    first_received: i64,
    last_received: i64,
}

#[derive(Clone)]
struct Slot {
    current: Buffered,
    next: Option<Buffered>,
    proposal: Option<Release>,
}

#[derive(Clone)]
pub struct TerminalFinalizer {
    policy: TerminalPolicy,
    policy_sha256: String,
    clock: Option<Clock>,
    feed: Option<FeedEvidence>,
    buffers: BTreeMap<String, Slot>,
    committed: BTreeMap<String, VecDeque<Release>>,
}

fn hash(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn date(ts: i64) -> Result<NaiveDate> {
    DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(Duration::hours(3)))
        .map(|d| d.date())
        .context("invalid calendar timestamp")
}

fn same_bar(a: &BarEvent, b: &BarEvent) -> bool {
    a.symbol == b.symbol
        && a.close_time_utc == b.close_time_utc
        && a.origin == b.origin
        && [a.o, a.h, a.l, a.c, a.v].map(f64::to_bits)
            == [b.o, b.h, b.l, b.c, b.v].map(f64::to_bits)
}

impl TerminalFinalizer {
    pub fn new(policy: TerminalPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy_sha256: hash(&policy)?,
            policy,
            clock: None,
            feed: None,
            buffers: BTreeMap::new(),
            committed: BTreeMap::new(),
        })
    }

    fn check_clock(&mut self, now: Clock) -> Result<()> {
        date(now.utc)?;
        if let Some(prev) = self.clock {
            ensure!(
                now.monotonic_ms >= prev.monotonic_ms,
                "monotonic clock reversed"
            );
            let elapsed = i128::from(now.monotonic_ms - prev.monotonic_ms);
            let wall = (i128::from(now.utc) - i128::from(prev.utc)) * 1000;
            ensure!(
                (wall - elapsed).abs() <= i128::from(self.policy.max_clock_step_seconds) * 1000,
                "wall clock stepped"
            );
        }
        self.clock = Some(now);
        Ok(())
    }

    fn validate_bar(&self, bar: &BarEvent, now: i64) -> Result<()> {
        ensure!(
            self.policy.symbols.contains_key(&bar.symbol),
            "unapproved symbol"
        );
        ensure!(
            bar.origin == DataOrigin::Live,
            "history must not enter live buffer"
        );
        ensure!(
            bar.close_time_utc > 0 && bar.close_time_utc % TF == 0 && bar.close_time_utc <= now,
            "invalid/future M10 label"
        );
        ensure!(
            date(bar.close_time_utc)? >= self.policy.first_live_session,
            "pre-activation data must remain History"
        );
        ensure!(
            [bar.o, bar.h, bar.l, bar.c, bar.v]
                .iter()
                .all(|x| x.is_finite())
                && bar.l > 0.0
                && bar.l <= bar.o.min(bar.c)
                && bar.h >= bar.o.max(bar.c)
                && bar.v >= 0.0,
            "invalid OHLCV"
        );
        Ok(())
    }

    fn healthy_feed(&self, now: Clock) -> Result<FeedEvidence> {
        let feed = self.feed.context("missing feed evidence")?;
        ensure!(
            feed.connected && feed.subscribed && feed.gap_resolved,
            "feed unconfirmed"
        );
        ensure!(
            feed.last_rx_utc <= now.utc
                && now.utc - feed.last_rx_utc <= self.policy.max_heartbeat_age_seconds,
            "stale/future WS heartbeat"
        );
        Ok(feed)
    }

    /// State changes are transactional. A reconnect never releases old buffers.
    /// An uncertain publication must first be reconciled, not cleared on reconnect.
    pub fn update_feed(&mut self, feed: FeedEvidence, now: Clock) -> Result<usize> {
        let mut staged = self.clone();
        staged.check_clock(now)?;
        ensure!(
            feed.generation > 0 && feed.last_rx_utc <= now.utc,
            "invalid feed evidence"
        );
        if let Some(old) = staged.feed {
            ensure!(feed.generation >= old.generation, "old feed generation");
            if old.generation == feed.generation {
                ensure!(
                    feed.last_rx_utc >= old.last_rx_utc,
                    "WS receipt clock reversed"
                );
            }
        }
        let reset = staged
            .feed
            .is_some_and(|old| old.generation != feed.generation)
            || !feed.connected
            || !feed.subscribed
            || !feed.gap_resolved;
        let discarded = if reset {
            ensure!(
                staged.pending_releases().is_empty(),
                "publication unresolved at feed reset"
            );
            let count = staged.buffers.len();
            staged.buffers.clear();
            count
        } else {
            0
        };
        staged.feed = Some(feed);
        *self = staged;
        Ok(discarded)
    }

    fn revision(old: Option<&BarEvent>, new: BarEvent) -> Observation {
        if old.is_some_and(|b| same_bar(b, &new)) {
            return Observation::Duplicate;
        }
        let kind = match old {
            Some(b)
                if [b.o, b.h, b.l, b.c].map(f64::to_bits)
                    == [new.o, new.h, new.l, new.c].map(f64::to_bits) =>
            {
                RevisionKind::VolumeOnly
            }
            Some(_) => RevisionKind::Price,
            None => RevisionKind::OlderThanCache,
        };
        Observation::Revision {
            original: old.cloned(),
            observed: new,
            kind,
        }
    }

    fn proposal(
        &self,
        b: &Buffered,
        now: Clock,
        reason: ReleaseReason,
        next: Option<i64>,
    ) -> Result<Release> {
        let feed = self.healthy_feed(now)?;
        ensure!(
            b.generation == feed.generation,
            "buffer from old generation"
        );
        let closed = b
            .bar
            .close_time_utc
            .checked_add(TF)
            .context("close timestamp overflow")?;
        ensure!(
            now.utc >= closed && now.utc - closed <= self.policy.max_lateness_seconds,
            "closed candle outside publication deadline"
        );
        ensure!(
            date(b.bar.close_time_utc)? == date(now.utc)?,
            "prior-day candle is not timely Live"
        );
        let proof = ReleaseProof {
            policy_sha256: self.policy_sha256.clone(),
            reason,
            generation: b.generation,
            first_received_at_utc: b.first_received,
            last_received_at_utc: b.last_received,
            observation_cutoff_utc: now.utc,
            closed_at_utc: closed,
            next_label_utc: next,
        };
        Ok(Release {
            id: hash(&(&b.bar, &proof))?,
            bar: b.bar.clone(),
            proof,
        })
    }

    pub fn observe(&mut self, bar: BarEvent, now: Clock) -> Result<Observation> {
        let mut staged = self.clone();
        let result = staged.observe_inner(bar, now)?;
        *self = staged;
        Ok(result)
    }

    fn observe_inner(&mut self, bar: BarEvent, now: Clock) -> Result<Observation> {
        self.check_clock(now)?;
        self.validate_bar(&bar, now.utc)?;
        let feed = self.healthy_feed(now)?;
        if let Some(rows) = self.committed.get(&bar.symbol)
            && rows
                .back()
                .is_some_and(|r| bar.close_time_utc <= r.bar.close_time_utc)
        {
            return Ok(Self::revision(
                rows.iter()
                    .find(|r| r.bar.close_time_utc == bar.close_time_utc)
                    .map(|r| &r.bar),
                bar,
            ));
        }
        let new = Buffered {
            bar: bar.clone(),
            generation: feed.generation,
            first_received: now.utc,
            last_received: now.utc,
        };
        let Some(slot) = self.buffers.get(&bar.symbol) else {
            self.buffers.insert(
                bar.symbol.clone(),
                Slot {
                    current: new,
                    next: None,
                    proposal: None,
                },
            );
            return Ok(Observation::Buffered);
        };
        if let Some(pending) = &slot.proposal {
            if bar.close_time_utc <= pending.bar.close_time_utc {
                return Ok(Self::revision(
                    (bar.close_time_utc == pending.bar.close_time_utc).then_some(&pending.bar),
                    bar,
                ));
            }
            let next = slot
                .next
                .as_ref()
                .context("publication unresolved before next buffer")?;
            ensure!(
                next.bar.close_time_utc == bar.close_time_utc,
                "publication unresolved before later bar"
            );
            let first_received = next.first_received;
            let slot = self.buffers.get_mut(&bar.symbol).unwrap();
            slot.next = Some(Buffered {
                first_received,
                ..new
            });
            return Ok(Observation::Buffered);
        }
        if bar.close_time_utc < slot.current.bar.close_time_utc {
            return Ok(Self::revision(None, bar));
        }
        if bar.close_time_utc == slot.current.bar.close_time_utc {
            let slot = self.buffers.get_mut(&bar.symbol).unwrap();
            slot.current = Buffered {
                first_received: slot.current.first_received,
                ..new
            };
            return Ok(Observation::Buffered);
        }
        let release = self.proposal(
            &slot.current,
            now,
            ReleaseReason::NextTimestamp,
            Some(bar.close_time_utc),
        )?;
        let slot = self.buffers.get_mut(&bar.symbol).unwrap();
        slot.next = Some(new);
        slot.proposal = Some(release.clone());
        Ok(Observation::Ready(release))
    }

    /// The caller attests that its WS event queue has been drained at this cutoff.
    /// No wall-clock-only or queued-prefix-unknown terminal release is possible.
    pub fn tick(&mut self, now: Clock, queue_drained: bool) -> Result<Vec<Release>> {
        let mut staged = self.clone();
        staged.check_clock(now)?;
        let mut ready = vec![];
        let today = date(now.utc)?;
        if staged.policy.enabled && today >= staged.policy.first_live_session {
            for (symbol, calendar) in &staged.policy.symbols {
                let Some(label) = calendar.terminal_label(today) else {
                    continue;
                };
                let close = label + TF;
                if now.utc < close + staged.policy.grace_seconds {
                    continue;
                }
                if staged
                    .committed
                    .get(symbol)
                    .and_then(|r| r.back())
                    .is_some_and(|r| r.bar.close_time_utc >= label)
                {
                    continue;
                }
                if !queue_drained {
                    ensure!(
                        now.utc <= close + staged.policy.max_lateness_seconds,
                        "WS queue not drained past terminal deadline: {symbol}"
                    );
                    continue;
                }
                let slot = staged.buffers.get(symbol);
                if slot.is_some_and(|s| s.proposal.is_some()) {
                    ensure!(
                        now.utc <= close + staged.policy.max_lateness_seconds,
                        "terminal publication unresolved after deadline: {symbol}"
                    );
                    continue;
                }
                let Some(slot) = slot.filter(|s| s.current.bar.close_time_utc == label) else {
                    ensure!(
                        now.utc <= close + staged.policy.max_lateness_seconds,
                        "terminal candle missing after deadline: {symbol}"
                    );
                    continue;
                };
                ready.push(staged.proposal(
                    &slot.current,
                    now,
                    ReleaseReason::SessionBoundaryWs,
                    None,
                )?);
            }
            for release in &ready {
                staged
                    .buffers
                    .get_mut(&release.bar.symbol)
                    .unwrap()
                    .proposal = Some(release.clone());
            }
        }
        *self = staged;
        Ok(ready)
    }

    pub fn pending_releases(&self) -> Vec<&Release> {
        self.buffers
            .values()
            .filter_map(|s| s.proposal.as_ref())
            .collect()
    }

    pub fn last_committed_label(&self, symbol: &str) -> Option<i64> {
        self.committed
            .get(symbol)?
            .back()
            .map(|r| r.bar.close_time_utc)
    }

    /// Only call after the exact proposal is durably published/reconciled.
    pub fn commit(&mut self, release: &Release, now: Clock) -> Result<()> {
        let mut staged = self.clone();
        staged.check_clock(now)?;
        let slot = staged
            .buffers
            .get(&release.bar.symbol)
            .context("no pending publication")?;
        let expected = slot.proposal.as_ref().context("no pending proposal")?;
        ensure!(
            hash(expected)? == hash(release)?,
            "publication identity mismatch"
        );
        ensure!(
            now.utc >= release.proof.observation_cutoff_utc,
            "ACK precedes proposal"
        );
        let mut slot = staged.buffers.remove(&release.bar.symbol).unwrap();
        let rows = staged
            .committed
            .entry(release.bar.symbol.clone())
            .or_default();
        rows.push_back(release.clone());
        if rows.len() > 2 {
            rows.pop_front();
        }
        if let Some(next) = slot.next.take() {
            staged.buffers.insert(
                next.bar.symbol.clone(),
                Slot {
                    current: next,
                    next: None,
                    proposal: None,
                },
            );
        }
        *self = staged;
        Ok(())
    }

    /// Seed at most two independently verified durable releases per symbol.
    /// Never restore an in-flight WS buffer or an old process's monotonic clock.
    pub fn restore_committed(&mut self, rows: &[Release], now: Clock) -> Result<()> {
        ensure!(
            self.clock.is_none()
                && self.feed.is_none()
                && self.committed.is_empty()
                && self.buffers.is_empty(),
            "restore requires fresh finalizer"
        );
        ensure!(rows.len() <= MAX_SYMBOLS * 2, "restore budget");
        let mut staged = self.clone();
        staged.check_clock(now)?;
        for row in rows {
            staged.validate_bar(&row.bar, now.utc)?;
            let p = &row.proof;
            ensure!(
                p.policy_sha256 == staged.policy_sha256
                    && p.generation > 0
                    && row.id == hash(&(&row.bar, p))?,
                "restored identity mismatch"
            );
            ensure!(
                row.bar.close_time_utc <= p.first_received_at_utc
                    && p.first_received_at_utc <= p.last_received_at_utc
                    && p.last_received_at_utc <= p.observation_cutoff_utc
                    && p.closed_at_utc == row.bar.close_time_utc + TF
                    && p.closed_at_utc <= p.observation_cutoff_utc
                    && p.observation_cutoff_utc - p.closed_at_utc
                        <= staged.policy.max_lateness_seconds
                    && p.observation_cutoff_utc <= now.utc
                    && date(p.observation_cutoff_utc)? == date(row.bar.close_time_utc)?,
                "invalid restored receipt/close proof"
            );
            match p.reason {
                ReleaseReason::NextTimestamp => ensure!(
                    p.next_label_utc.is_some_and(|t| t > row.bar.close_time_utc
                        && t % TF == 0
                        && t <= p.observation_cutoff_utc),
                    "missing/invalid next timestamp proof"
                ),
                ReleaseReason::SessionBoundaryWs => ensure!(
                    staged.policy.enabled
                        && p.next_label_utc.is_none()
                        && date(row.bar.close_time_utc)? >= staged.policy.first_live_session
                        && staged.policy.symbols[&row.bar.symbol]
                            .terminal_label(date(row.bar.close_time_utc)?)
                            == Some(row.bar.close_time_utc)
                        && p.observation_cutoff_utc
                            >= p.closed_at_utc + staged.policy.grace_seconds,
                    "invalid restored terminal proof"
                ),
            }
            let cache = staged.committed.entry(row.bar.symbol.clone()).or_default();
            ensure!(
                cache.len() < 2
                    && cache
                        .back()
                        .is_none_or(|r| r.bar.close_time_utc < row.bar.close_time_utc),
                "duplicate/unordered restore"
            );
            cache.push_back(row.clone());
        }
        *self = staged;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
