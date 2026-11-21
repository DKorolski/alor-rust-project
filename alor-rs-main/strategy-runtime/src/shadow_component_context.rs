//! Independent IMOEXF A/A and A/B feature snapshots, not a signal engine or
//! admission authority. Existing context stores and numerical engines are unchanged.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::shadow_calendar::{Calendar, ContextPolicy, Coverage, Selection, SessionKind};
use crate::shadow_session_context::{Anchor, Barrier, Receipt};
use crate::DataOrigin;

const MAX_RECEIPTS: usize = 12_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Arm {
    ImoAa,
    ImoAb,
    ImoAbOi,
}

impl Arm {
    fn policies(self) -> (ContextPolicy, ContextPolicy) {
        (
            ContextPolicy::RegularOnly,
            if self == Self::ImoAa {
                ContextPolicy::RegularOnly
            } else {
                ContextPolicy::RegularAndDsvd
            },
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    DeclaredAsObserved,
    NumericalFixture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub arm: Arm,
    pub model_sha256: String,
    pub receipt_stream: String,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionMetrics {
    pub anchor: Anchor,
    pub main_range: Option<f64>,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LevelInputs {
    pub previous_close: f64,
    pub previous_high: f64,
    pub previous_low: f64,
    pub previous_range: f64,
    pub previous2_close: f64,
    pub previous_return: f64,
    pub entry_scale: f64,
    pub stop_scale: f64,
    /// Complete flat sessions are data-ready, but must not pass a positive-range predicate.
    pub signal_context_valid: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Component {
    pub selection: Selection,
    pub previous: Option<SessionMetrics>,
    pub previous2: Option<SessionMetrics>,
    pub inputs: Option<LevelInputs>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Previous,
    Previous2,
    OlderCandidate,
    CalendarExcluded,
    PolicyExcluded,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionTrace {
    pub date: NaiveDate,
    pub kind: SessionKind,
    pub coverage: Coverage,
    pub mr: Disposition,
    pub bo: Disposition,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrozenComponents {
    pub schema_version: &'static str,
    pub request: Request,
    pub symbol: String,
    pub calendar_sha256: String,
    pub session_date: NaiveDate,
    pub cutoff_utc: i64,
    pub session_close_utc: i64,
    pub expected_execution_labels_utc: Vec<i64>,
    pub frozen_at_utc: i64,
    pub barrier: Barrier,
    pub mr: Component,
    pub bo: Component,
    pub dependency_ready: bool,
    pub session_trace: Vec<SessionTrace>,
}

pub fn prefix_sha256(receipts: &[Receipt]) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(receipts)?)
    ))
}

fn local(ts: i64) -> Result<chrono::NaiveDateTime> {
    DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(Duration::hours(3)))
        .context("invalid component context timestamp")
}

fn metrics(
    calendar: &Calendar,
    date: NaiveDate,
    rows: &[&Receipt],
    coverage: &Coverage,
) -> Result<Option<SessionMetrics>> {
    if !coverage.accepted {
        return Ok(None);
    }
    let first = rows.first().context("accepted session has no first bar")?;
    let last = rows.last().context("accepted session has no last bar")?;
    let high = rows
        .iter()
        .map(|r| r.bar.h)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = rows.iter().map(|r| r.bar.l).fold(f64::INFINITY, f64::min);
    let session = calendar.session(date);
    let main: Vec<_> = rows
        .iter()
        .filter(|r| {
            let t = local(r.bar.close_time_utc).unwrap().time();
            let minute = (t.hour() * 60 + t.minute()) as u16;
            session
                .main_open_minute
                .is_some_and(|start| start <= minute)
                && session.main_close_minute.is_some_and(|end| minute < end)
        })
        .collect();
    let main_range = (!main.is_empty()).then(|| {
        main.iter()
            .map(|r| r.bar.h)
            .fold(f64::NEG_INFINITY, f64::max)
            - main.iter().map(|r| r.bar.l).fold(f64::INFINITY, f64::min)
    });
    Ok(Some(SessionMetrics {
        anchor: Anchor {
            session_date: date,
            open: first.bar.o,
            high,
            low,
            close: last.bar.close,
            range: high - low,
            distinct_bars: rows.len(),
            required_bars: coverage.required_bins,
            total_slots: coverage.expected_bins,
            first_bar_utc: first.bar.close_time_utc,
            last_bar_utc: last.bar.close_time_utc,
            latest_received_at_utc: rows
                .iter()
                .map(|r| r.received_at_utc.unwrap())
                .max()
                .unwrap(),
            latest_durable_at_utc: rows
                .iter()
                .map(|r| r.durable_at_utc.unwrap())
                .max()
                .unwrap(),
            source_sequences: rows.iter().map(|r| r.sequence).collect(),
            source_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(rows)?)),
        },
        main_range,
        source_ids: rows.iter().map(|r| r.source_id.clone()).collect(),
    }))
}

fn component(
    selection: Selection,
    values: &BTreeMap<NaiveDate, SessionMetrics>,
) -> Result<Component> {
    let previous = selection.previous.and_then(|d| values.get(&d)).cloned();
    let previous2 = selection.previous2.and_then(|d| values.get(&d)).cloned();
    let inputs = if selection.ready {
        previous
            .as_ref()
            .zip(previous2.as_ref())
            .map(|(p, p2)| LevelInputs {
                previous_close: p.anchor.close,
                previous_high: p.anchor.high,
                previous_low: p.anchor.low,
                previous_range: p.anchor.range,
                previous2_close: p2.anchor.close,
                previous_return: p.anchor.close / p2.anchor.close - 1.,
                entry_scale: p.anchor.range,
                stop_scale: p.anchor.range,
                signal_context_valid: p.anchor.range > 0.
                    && (p.anchor.close + p.anchor.range + p.anchor.low).is_finite(),
            })
    } else {
        None
    };
    if let Some(inputs) = &inputs {
        ensure!(
            inputs.previous_return.is_finite(),
            "non-finite derived component return"
        );
    }
    Ok(Component {
        selection,
        previous,
        previous2,
        inputs,
    })
}

fn disposition(kind: SessionKind, date: NaiveDate, selection: &Selection) -> Disposition {
    if selection.previous == Some(date) {
        Disposition::Previous
    } else if selection.previous2 == Some(date) {
        Disposition::Previous2
    } else if kind == SessionKind::Closed {
        Disposition::CalendarExcluded
    } else if kind == SessionKind::Dsvd && selection.policy == ContextPolicy::RegularOnly {
        Disposition::PolicyExcluded
    } else {
        Disposition::OlderCandidate
    }
}

/// Computes a new immutable value from a declared canonical prefix. It never
/// changes that prefix, substitutes older good days, or attaches to a worker.
/// A caller must separately attest the barrier and persist/freeze this value once.
pub fn freeze(
    calendar: &Calendar,
    request: Request,
    day: NaiveDate,
    frozen_at_utc: i64,
    receipts: &[Receipt],
    barrier: &Barrier,
) -> Result<FrozenComponents> {
    ensure!(
        request.model_sha256.len() == 64
            && request.model_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            && !request.receipt_stream.is_empty()
            && request.receipt_stream.len() <= 256,
        "invalid component identity"
    );
    ensure!(receipts.len() <= MAX_RECEIPTS, "component prefix bound");
    let (mr_policy, bo_policy) = request.arm.policies();
    let session = calendar.session(day);
    let cutoff = calendar
        .bounds_utc(day)
        .context("unknown target session")?
        .0;
    ensure!(
        calendar
            .route(
                day,
                session.context_open_minute.unwrap(),
                mr_policy,
                bo_policy
            )
            .new_entry_allowed,
        "component freeze requires a regular weekday decision"
    );
    ensure!(
        frozen_at_utc >= cutoff
            && barrier.through_utc >= cutoff
            && barrier.through_utc <= barrier.attested_at_utc
            && barrier.attested_at_utc <= frozen_at_utc
            && barrier.receipt_stream == request.receipt_stream
            && barrier.last_sequence == receipts.len() as u64
            && barrier.ingress_sha256 == prefix_sha256(receipts)?,
        "incomplete or mismatched component barrier"
    );
    let mut sources = BTreeSet::new();
    let mut last_label = None;
    let mut grouped: BTreeMap<NaiveDate, Vec<&Receipt>> = BTreeMap::new();
    for (index, r) in receipts.iter().enumerate() {
        ensure!(
            r.sequence == index as u64 + 1
                && !r.source_id.is_empty()
                && r.source_id.len() <= 256
                && sources.insert(&r.source_id)
                && last_label.is_none_or(|ts| r.bar.close_time_utc > ts),
            "not an ordered unique canonical prefix"
        );
        last_label = Some(r.bar.close_time_utc);
        ensure!(r.bar.symbol == "IMOEXF", "component instrument mismatch");
        ensure!(
            match request.evidence {
                Evidence::NumericalFixture => r.bar.origin == DataOrigin::Replay,
                Evidence::DeclaredAsObserved =>
                    matches!(r.bar.origin, DataOrigin::Live | DataOrigin::History),
            },
            "origin/evidence mismatch"
        );
        let label = local(r.bar.close_time_utc)?;
        let received = r.received_at_utc.context("missing component receipt")?;
        let durable = r.durable_at_utc.context("missing component durability")?;
        ensure!(
            r.bar
                .close_time_utc
                .checked_add(600)
                .is_some_and(|end| end <= received)
                && received <= durable
                && durable <= barrier.attested_at_utc,
            "invalid component receipt clocks"
        );
        ensure!(
            [r.bar.o, r.bar.h, r.bar.l, r.bar.close, r.bar.v]
                .iter()
                .all(|v| v.is_finite())
                && r.bar.l > 0.
                && r.bar.v >= 0.
                && r.bar.l <= r.bar.o
                && r.bar.o <= r.bar.h
                && r.bar.l <= r.bar.close
                && r.bar.close <= r.bar.h,
            "invalid component OHLCV"
        );
        // Processing a first event late must not move the fixed calendar cutoff.
        if label.date() < day && received < cutoff && durable < cutoff {
            grouped.entry(label.date()).or_default().push(r);
        }
    }
    let mut coverage = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut date = calendar.horizon().0;
    while date < day {
        let raw = grouped.get(&date).map(Vec::as_slice).unwrap_or(&[]);
        let labels: Vec<_> = raw.iter().map(|r| r.bar.close_time_utc).collect();
        let c = calendar.coverage(date, &labels);
        let expected: BTreeSet<_> = calendar.expected_labels_utc(date).into_iter().collect();
        let kept: Vec<_> = raw
            .iter()
            .copied()
            .filter(|r| expected.contains(&r.bar.close_time_utc))
            .collect();
        if let Some(value) = metrics(calendar, date, &kept, &c)? {
            values.insert(date, value);
        }
        coverage.insert(date, c);
        date = date.succ_opt().context("context date overflow")?;
    }
    let selection = calendar.select_components(day, mr_policy, bo_policy, &coverage);
    ensure!(
        selection.symbol == "IMOEXF",
        "calendar/component instrument mismatch"
    );
    let session_trace = coverage
        .into_iter()
        .map(|(date, coverage)| {
            let kind = calendar.session(date).kind;
            SessionTrace {
                date,
                kind,
                coverage,
                mr: disposition(kind, date, &selection.mr),
                bo: disposition(kind, date, &selection.bo),
            }
        })
        .collect();
    Ok(FrozenComponents {
        schema_version: "imoexf_component_features_v1",
        request,
        symbol: selection.symbol,
        calendar_sha256: calendar.sha256().into(),
        session_date: day,
        cutoff_utc: cutoff,
        session_close_utc: calendar.bounds_utc(day).unwrap().1,
        expected_execution_labels_utc: calendar.expected_labels_utc(day),
        frozen_at_utc,
        barrier: barrier.clone(),
        dependency_ready: selection.combo_ready,
        mr: component(selection.mr, &values)?,
        bo: component(selection.bo, &values)?,
        session_trace,
    })
}

#[cfg(test)]
mod tests;
