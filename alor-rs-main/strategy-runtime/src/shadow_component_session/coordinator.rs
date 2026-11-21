//! Explicitly dated local driver, not an operational admission or daily scheduler.
//! Reuses read-only Redis polling and the original journal/three-arm session WAL.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use redis::aio::MultiplexedConnection;
use redis::streams::StreamRangeReply;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{hash, local, Output, Session, Spec, Status};
use crate::ri_context::ingress::redis_source::{bar_payload, stream_id, RiRedisSource};
use crate::ri_context::ingress::{IngressUsage, RiIngressJournal};
use crate::ri_context::runner::storage::{LogRecord, TransactionLog};

mod source;
pub use source::{ProcessPin, SourcePin};
use source::{SourceGuard, SourceState};

const VERSION: &str = "imoexf_component_day_driver_v1";
const PREFIX_BATCH: usize = 128;
const MAX_PREFIX_ROWS: usize = 2048;
const MAX_PREFIX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub session: Spec,
    pub source: SourcePin,
    /// Inclusive, independently reviewed bootstrap boundary; never "$"/latest.
    pub first_source_id: String,
    pub ready_by_utc: i64,
    pub max_poll_seconds: u16,
    pub max_queue_age_seconds: u16,
}
impl Plan {
    fn validate(&self) -> Result<()> {
        let calendar = self.session.calendar()?;
        let cutoff = calendar.bounds_utc(self.session.date).unwrap().0;
        ensure!(
            stream_id(&self.first_source_id)? != (0, 0)
                && local(self.ready_by_utc)?.date() == self.session.date
                && self.ready_by_utc < cutoff
                && self.ready_by_utc >= cutoff - 3600
                && (1..=30).contains(&self.max_poll_seconds)
                && (1..=120).contains(&self.max_queue_age_seconds)
                && self.max_poll_seconds < self.max_queue_age_seconds
                && self.max_queue_age_seconds <= self.session.max_execution_lag_seconds,
            "invalid dated driver boundary/budgets"
        );
        self.source.validate(&self.session)
    }
}

#[derive(Serialize)]
pub struct DriverStatus {
    pub session: Status,
    pub ingress: IngressUsage,
    pub source_prefix_transactions: u64,
    pub source_prefix_bytes: u64,
    pub source_caught_up: bool,
    pub source_ready: bool,
    pub source_ws_generation: Option<u64>,
    pub armed_source_only: bool,
    pub waiting_reason: Option<String>,
    pub next_deadline_utc: Option<i64>,
    pub clean_price_session: bool,
    pub clean_oi_comparison_session: bool,
    pub order_emission_allowed: bool,
    pub operational_admission_required: bool,
}

pub struct Driver {
    root: PathBuf,
    plan: Plan,
    journal: RiIngressJournal,
    session: Session,
    reader: RiRedisSource,
    guard: SourceGuard,
    source_witness: TransactionLog<SourceWitness>,
    accepted_envelopes: Vec<(String, String)>,
    collected: Option<(u64, i64)>,
    armed: bool,
    source_ready: bool,
    waiting_reason: Option<String>,
    clock: fn() -> i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArmedWitness {
    plan_sha256: String,
    checked_at_utc: i64,
    through_sequence: u64,
    receipt_prefix_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceWitness {
    index: u64,
    previous_sha256: String,
    envelopes: Vec<(String, String)>,
}
impl LogRecord for SourceWitness {
    fn index(&self) -> u64 {
        self.index
    }
    fn previous_sha256(&self) -> &str {
        &self.previous_sha256
    }
}
fn receipt_prefix_hash(journal: &RiIngressJournal, through: u64) -> Result<String> {
    let rows = journal.identified_input_prefix(through)?;
    let mut data = Vec::with_capacity(rows.len());
    for (id, input) in rows {
        data.push((
            id,
            journal.raw_payload(input.sequence)?,
            input.received_at_utc,
            input.durable_at_utc,
        ));
    }
    hash(&data)
}

fn read_small(path: &Path, max: u64) -> Result<Vec<u8>> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "regular driver input required"
    );
    let mut data = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut data)?;
    ensure!(data.len() as u64 <= max, "driver input too large");
    Ok(data)
}
fn publish_plan(root: &Path, plan: &Plan) -> Result<()> {
    ensure!(
        root.is_absolute()
            && root.parent().context("driver parent")?.canonicalize()? == root.parent().unwrap(),
        "canonical driver parent required"
    );
    fs::create_dir(root).context("driver generation must be NEW")?;
    File::open(root.parent().unwrap())?.sync_all()?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("PLAN.json"))?;
    serde_json::to_writer(&mut file, &json!({"version":VERSION,"plan":plan}))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(root)?.sync_all()?;
    Ok(())
}

impl Driver {
    pub fn create(root: &Path, plan: Plan) -> Result<Self> {
        Self::create_with_clock(root, plan, || Utc::now().timestamp(), source::check_process)
    }
    fn create_with_clock(
        root: &Path,
        plan: Plan,
        clock: fn() -> i64,
        process: fn(&ProcessPin) -> Result<()>,
    ) -> Result<Self> {
        plan.validate()?;
        ensure!(
            clock() < plan.ready_by_utc,
            "driver create missed pre-open readiness window"
        );
        let guard = SourceGuard::new(
            plan.source.clone(),
            &plan.session,
            plan.max_queue_age_seconds,
            process,
        )?;
        publish_plan(root, &plan)?;
        let journal = RiIngressJournal::create_with_clock(
            &root.join("receipts"),
            plan.session.ingress.clone(),
            clock,
        )?;
        let session =
            Session::create_with_clock(&root.join("session"), plan.session.clone(), clock)?;
        let source_witness = TransactionLog::create(
            &root.join("source-prefix"),
            &json!({"version":VERSION,"plan_sha256":hash(&plan)?}),
        )?;
        Self::assemble(root, plan, journal, session, guard, source_witness, clock)
    }
    pub fn open(root: &Path, expected: Plan) -> Result<Self> {
        Self::open_with_clock(
            root,
            expected,
            || Utc::now().timestamp(),
            source::check_process,
        )
    }
    fn open_with_clock(
        root: &Path,
        plan: Plan,
        clock: fn() -> i64,
        process: fn(&ProcessPin) -> Result<()>,
    ) -> Result<Self> {
        plan.validate()?;
        ensure!(
            root.is_absolute() && root.canonicalize()? == root,
            "canonical driver generation required"
        );
        ensure!(
            serde_json::from_slice::<Value>(&read_small(
                &root.join("PLAN.json"),
                5 * 1024 * 1024
            )?)? == json!({"version":VERSION,"plan":plan}),
            "driver plan changed"
        );
        let guard = SourceGuard::new(
            plan.source.clone(),
            &plan.session,
            plan.max_queue_age_seconds,
            process,
        )?;
        let journal = RiIngressJournal::open_with_clock(
            &root.join("receipts"),
            &plan.session.ingress,
            clock,
        )?;
        let session =
            Session::open_with_clock(&root.join("session"), plan.session.clone(), &journal, clock)?;
        let (source_witness, witnesses) = TransactionLog::<SourceWitness>::open(
            &root.join("source-prefix"),
            &json!({"version":VERSION,"plan_sha256":hash(&plan)?}),
        )?;
        ensure!(
            source_witness.count() <= 256 && source_witness.bytes() < MAX_PREFIX_BYTES,
            "source witness recovery budget exhausted"
        );
        let mut accepted: Vec<(String, String)> = Vec::new();
        let rows = journal.identified_input_prefix(journal.last_sequence())?;
        for witness in witnesses {
            ensure!(
                witness.envelopes.len() > accepted.len()
                    && witness.envelopes.len() <= MAX_PREFIX_ROWS
                    && witness.envelopes.len() <= rows.len()
                    && witness.envelopes.starts_with(&accepted)
                    && witness
                        .envelopes
                        .iter()
                        .zip(&rows)
                        .all(|((id, hash), (expected, _))| id == expected
                            && hash.len() == 64
                            && hash.bytes().all(|b| b.is_ascii_hexdigit())),
                "source witness prefix changed"
            );
            accepted = witness.envelopes;
        }
        ensure!(
            session.state.cursor as usize <= accepted.len(),
            "model advanced beyond source witness"
        );
        let mut result =
            Self::assemble(root, plan, journal, session, guard, source_witness, clock)?;
        result.accepted_envelopes = accepted;
        let witness = root.join("ARMED.json");
        if witness.try_exists()? || witness.is_symlink() {
            let armed: ArmedWitness = serde_json::from_slice(&read_small(&witness, 4096)?)?;
            ensure!(
                armed.plan_sha256 == hash(&result.plan)?
                    && armed.checked_at_utc < result.plan.ready_by_utc
                    && armed.checked_at_utc <= clock()
                    && armed.through_sequence > 0
                    && armed.through_sequence as usize <= result.accepted_envelopes.len()
                    && armed.receipt_prefix_sha256
                        == receipt_prefix_hash(&result.journal, armed.through_sequence)?
                    && result
                        .journal
                        .input_prefix(armed.through_sequence)?
                        .iter()
                        .all(|r| r
                            .durable_at_utc
                            .is_some_and(|ts| ts <= armed.checked_at_utc)),
                "invalid pre-open readiness witness"
            );
            result.armed = true;
        }
        ensure!(
            result.session.state.contexts.is_none() || result.armed,
            "frozen session missing durable readiness witness"
        );
        result.check_deadlines()?;
        Ok(result)
    }
    fn assemble(
        root: &Path,
        plan: Plan,
        journal: RiIngressJournal,
        session: Session,
        guard: SourceGuard,
        source_witness: TransactionLog<SourceWitness>,
        clock: fn() -> i64,
    ) -> Result<Self> {
        let mut reader = RiRedisSource::new(plan.session.ingress.clone())?;
        reader.start_at(&plan.first_source_id)?;
        Ok(Self {
            root: root.into(),
            plan,
            journal,
            session,
            reader,
            guard,
            source_witness,
            accepted_envelopes: Vec::new(),
            collected: None,
            armed: false,
            source_ready: false,
            waiting_reason: Some("awaiting_qualified_source_poll".into()),
            clock,
        })
    }
    fn next_deadline(&self) -> Option<i64> {
        if self.session.status().complete {
            return None;
        }
        let calendar = &self.session.calendar;
        let cutoff = calendar.bounds_utc(self.plan.session.date)?.0;
        if !self.armed {
            return Some(self.plan.ready_by_utc);
        }
        if self.session.state.contexts.is_none() {
            return Some(cutoff + 600);
        }
        let engines = self.session.state.engines.as_ref()?;
        let labels = calendar.expected_labels_utc(self.plan.session.date);
        labels
            .get(engines[0].state().observed_bars)
            .map(|label| *label + 600 + i64::from(self.plan.session.max_execution_lag_seconds))
    }
    fn check_deadlines(&mut self) -> Result<()> {
        self.session.log.healthy()?;
        let now = (self.clock)();
        let result = (|| {
            self.journal.now()?;
            ensure!(
                self.next_deadline().is_none_or(|limit| now < limit),
                "required source/readiness/model deadline exhausted"
            );
            ensure!(
                local(now)?.date() <= self.plan.session.date || self.session.status().complete,
                "incomplete session cannot carry to another day"
            );
            Ok(())
        })();
        result.map_err(|e| self.session.log.fail(e))
    }
    pub fn status(&self) -> DriverStatus {
        let session = self.session.status();
        let complete = session.complete;
        let oi_valid = session.oi_input_valid;
        DriverStatus {
            session,
            ingress: self.journal.usage(),
            source_prefix_transactions: self.source_witness.count(),
            source_prefix_bytes: self.source_witness.bytes(),
            source_caught_up: self.reader.caught_up(&self.journal),
            source_ready: self.source_ready,
            source_ws_generation: self.guard.generation(),
            armed_source_only: self.armed,
            waiting_reason: self.waiting_reason.clone(),
            next_deadline_utc: self.next_deadline(),
            clean_price_session: complete,
            clean_oi_comparison_session: complete && oi_valid,
            order_emission_allowed: false,
            operational_admission_required: true,
        }
    }
    pub fn committed_output(&self, index: u64) -> Result<Output> {
        self.session.committed_output(index)
    }

    /// An independently qualified upstream is still required. This reads only
    /// source manifest/health and Redis; no groups, acknowledgments or writes.
    pub async fn collect(
        &mut self,
        connection: &mut MultiplexedConnection,
    ) -> Result<DriverStatus> {
        self.collected = None;
        self.source_ready = false;
        self.check_deadlines()?;
        if self.session.status().complete {
            return Ok(self.status());
        }
        match self
            .guard
            .check((self.clock)(), self.session.state.contexts.is_none())
        {
            Ok(SourceState::Ready) => {}
            Ok(SourceState::Recovering) => {
                self.waiting_reason = Some("upstream_recovering".into());
                return Ok(self.status());
            }
            Err(e) => return Err(self.session.log.fail(e)),
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(u64::from(self.plan.max_poll_seconds)),
            async {
                self.reader.poll(connection, &mut self.journal).await?;
                self.verify_prefix(connection).await
            },
        )
        .await;
        match result {
            Err(_) => self.waiting_reason = Some("readonly_poll_timeout".into()),
            Ok(Err(e)) if e.downcast_ref::<redis::RedisError>().is_some() => {
                self.waiting_reason = Some("readonly_transport_unavailable".into())
            }
            Ok(Err(e)) => return Err(self.session.log.fail(e)),
            Ok(Ok(())) => {
                match self
                    .guard
                    .check((self.clock)(), self.session.state.contexts.is_none())
                {
                    Ok(SourceState::Ready) => {
                        self.source_ready = true;
                        self.collected = Some((self.journal.last_sequence(), (self.clock)()));
                        self.waiting_reason = None;
                    }
                    Ok(SourceState::Recovering) => {
                        self.waiting_reason = Some("upstream_recovering".into())
                    }
                    Err(e) => return Err(self.session.log.fail(e)),
                }
            }
        }
        self.check_deadlines()?;
        Ok(self.status())
    }

    // Compare the ENTIRE accepted prefix, including source_proof and envelope
    // publisher, not just the durable cursor. The existing journal stores raw
    // normalized payload JSON. Never repair a mutated/trimmed upstream range.
    async fn verify_prefix(&mut self, connection: &mut MultiplexedConnection) -> Result<()> {
        self.source_witness.healthy()?;
        let rows = self
            .journal
            .identified_input_prefix(self.journal.last_sequence())?;
        ensure!(
            rows.len() <= MAX_PREFIX_ROWS,
            "dated source prefix needs reviewed maintenance"
        );
        if rows.is_empty() {
            return Ok(());
        }
        ensure!(
            rows[0].0 == self.plan.first_source_id,
            "bootstrap source boundary changed"
        );
        let last = &rows.last().unwrap().0;
        let mut cursor = rows[0].0.clone();
        let mut count = 0;
        let mut observed = Vec::with_capacity(rows.len());
        loop {
            let batch: StreamRangeReply = redis::cmd("XRANGE")
                .arg(&self.plan.session.ingress.source_stream)
                .arg(&cursor)
                .arg(last)
                .arg("COUNT")
                .arg(PREFIX_BATCH)
                .query_async(connection)
                .await?;
            if batch.ids.is_empty() {
                break;
            }
            for r in &batch.ids {
                let expected = rows
                    .get(count)
                    .context("extra source row in accepted prefix")?;
                let envelope: String = r.get("payload").context("source envelope missing")?;
                let e: Value = serde_json::from_str(&envelope)?;
                let envelope_hash = hash(&e)?;
                ensure!(
                    self.accepted_envelopes
                        .get(count)
                        .is_none_or(|(id, sha)| id == &r.id && sha == &envelope_hash),
                    "accepted source envelope changed"
                );
                ensure!(
                    r.id == expected.0
                        && e["payload"]["close_time_utc"]
                            .as_i64()
                            .and_then(|ts| ts.checked_mul(1000))
                            .is_some_and(|ts| r.id == format!("{ts}-0"))
                        && e["source"] == self.guard.publisher()
                        && e["msg_type"] == "bar"
                        && e["schema_version"] == 1
                        && bar_payload(r)? == self.journal.raw_payload(count as u64 + 1)?,
                    "accepted Redis source prefix changed"
                );
                observed.push((r.id.clone(), envelope_hash));
                count += 1;
            }
            cursor = format!("({}", batch.ids.last().unwrap().id);
            if batch.ids.len() < PREFIX_BATCH {
                break;
            }
        }
        ensure!(
            count == rows.len(),
            "accepted Redis source prefix missing/trimmed"
        );
        if observed.len() > self.accepted_envelopes.len() {
            let witness = SourceWitness {
                index: self.source_witness.count() + 1,
                previous_sha256: self.source_witness.head().into(),
                envelopes: observed.clone(),
            };
            ensure!(
                self.source_witness.count() < 256
                    && self.source_witness.bytes()
                        + serde_json::to_vec(&witness)?.len() as u64
                        + 1024
                        < MAX_PREFIX_BYTES,
                "source witness budget exhausted; no rotation/reset"
            );
            self.source_witness.append(&witness)?;
            self.accepted_envelopes = observed;
        }
        Ok(())
    }

    /// Revalidate after collection, then advance only the fixed local prefix.
    /// No Redis await, clock override, broker route or late-start replay here.
    pub fn advance(&mut self, oi_root: Option<&Path>) -> Result<Vec<Output>> {
        self.check_deadlines()?;
        if self.session.status().complete {
            return Ok(Vec::new());
        }
        let result = (|| {
            let (sequence, checked) = self
                .collected
                .take()
                .context("advance requires a checked collected prefix")?;
            let now = (self.clock)();
            ensure!(
                sequence == self.journal.last_sequence()
                    && checked <= now
                    && now - checked < i64::from(self.plan.max_queue_age_seconds),
                "collected prefix changed/expired"
            );
            ensure!(
                matches!(
                    self.guard
                        .check(now, self.session.state.contexts.is_none())?,
                    SourceState::Ready
                ),
                "source must recover before model advance"
            );
            let mut outputs = Vec::new();
            if !self.armed && self.reader.caught_up(&self.journal) {
                ensure!(
                    now < self.plan.ready_by_utc,
                    "source missed pre-open readiness"
                );
                ensure!(sequence > 0, "empty bootstrap is not pre-open readiness");
                let witness = ArmedWitness {
                    plan_sha256: hash(&self.plan)?,
                    checked_at_utc: now,
                    through_sequence: sequence,
                    receipt_prefix_sha256: receipt_prefix_hash(&self.journal, sequence)?,
                };
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(self.root.join("ARMED.json"))?;
                serde_json::to_writer(&mut file, &witness)?;
                file.write_all(b"\n")?;
                file.sync_all()?;
                File::open(&self.root)?.sync_all()?;
                self.armed = true;
            }
            let cutoff = self
                .session
                .calendar
                .bounds_utc(self.plan.session.date)
                .unwrap()
                .0;
            if now < cutoff || !self.armed {
                return Ok(outputs);
            }
            if self.session.state.contexts.is_none() {
                if !self.reader.cutoff_poll_ready(&self.journal, cutoff) {
                    return Ok(outputs);
                }
                let barrier = self.reader.seal_cutoff(&mut self.journal, cutoff)?;
                outputs.push(self.session.freeze(&self.journal, barrier, oi_root)?);
            }
            for _ in 0..8 {
                let Some(output) = self.session.observe(&self.journal)? else {
                    break;
                };
                outputs.push(output);
            }
            Ok(outputs)
        })();
        result.map_err(|e| self.session.log.fail(e))
    }
}

#[cfg(test)]
mod tests;
