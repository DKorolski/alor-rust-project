//! One-session, three-arm local-receipt bridge. No scheduler, broker/Intent,
//! Redis writes or admission authority. Reuses the existing journal/HEAD WAL.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ri_context::ingress::{IngressIdentity, RiIngressJournal};
use crate::ri_context::runner::storage::{LogRecord, TransactionLog};
use crate::ri_context::ReceiptBarrier;
use crate::shadow_calendar::{ArtifactPin, Calendar};
use crate::shadow_component_context::{self as context, Arm, Evidence, FrozenComponents, Request};
use crate::shadow_component_reaction::{BoGate, Engine};
use crate::shadow_session_context::{Barrier, Receipt};
use crate::{BarEvent, DataOrigin};

const VERSION: &str = "imoexf_component_session_v1";
const MODEL: &str = "a469365d54844879ed63108bc99d45be18ace95d261627b57a94ceb1a4c18135";
const ARMS: [Arm; 3] = [Arm::ImoAa, Arm::ImoAb, Arm::ImoAbOi];

pub mod coordinator;

fn hash(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn local(ts: i64) -> Result<chrono::NaiveDateTime> {
    DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(Duration::hours(3)))
        .context("invalid session timestamp")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub ingress: IngressIdentity,
    pub date: NaiveDate,
    /// Copied reviewed artifact, not a mutable path reloaded on restart.
    pub calendar_json: String,
    pub calendar_pin: ArtifactPin,
    pub model_sha256: String,
    /// Declared reviewed M10 producer identity; not exchange/source readiness proof.
    pub source_manifest_sha256: String,
    pub max_execution_lag_seconds: u16,
}
impl Spec {
    fn calendar(&self) -> Result<Calendar> {
        ensure!(
            [
                &self.ingress.generation,
                &self.ingress.receipt_stream,
                &self.ingress.source_stream
            ]
            .iter()
            .all(|s| !s.trim().is_empty() && s.len() <= 512)
                && self.ingress.symbol == "IMOEXF"
                && self.ingress.receipt_stream.len() <= 220
                && !self.ingress.generation.is_empty()
                && self.model_sha256 == MODEL
                && self.source_manifest_sha256.len() == 64
                && self
                    .source_manifest_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                && (1..=300).contains(&self.max_execution_lag_seconds),
            "component session identity/budget"
        );
        let cal = Calendar::from_json(self.calendar_json.as_bytes(), &self.calendar_pin)?;
        crate::shadow_oi::Plan::new(&cal, self.date, &self.model_sha256)?;
        Ok(cal)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum OiDecision {
    Disabled {
        reason: String,
        detail_sha256: Option<String>,
    },
    #[cfg(feature = "shadow-oi-http")]
    Qualified {
        snapshot: Box<crate::shadow_oi::collector::snapshot::Snapshot>,
    },
}
impl OiDecision {
    fn capture(spec: &Spec, cal: &Calendar, root: Option<&Path>) -> Self {
        #[cfg(feature = "shadow-oi-http")]
        {
            let result = (|| {
                ensure!(
                    root.is_none_or(Path::is_absolute),
                    "absolute qualified OI root required"
                );
                let plan = crate::shadow_oi::Plan::new(cal, spec.date, &spec.model_sha256)?;
                root.map(|p| crate::shadow_oi::collector::snapshot::Snapshot::capture(p, &plan))
                    .transpose()
                    .map(Option::flatten)
            })();
            match result {
                Ok(Some(snapshot)) => Self::Qualified {
                    snapshot: Box::new(snapshot),
                },
                Ok(None) => Self::Disabled {
                    reason: "qualified_oi_missing".into(),
                    detail_sha256: None,
                },
                Err(error) => Self::Disabled {
                    reason: "qualified_oi_invalid".into(),
                    detail_sha256: hash(&format!("{error:#}")).ok(),
                },
            }
        }
        #[cfg(not(feature = "shadow-oi-http"))]
        {
            let _ = (spec, cal, root);
            Self::Disabled {
                reason: "oi_feature_not_compiled".into(),
                detail_sha256: None,
            }
        }
    }
    fn evaluate(&self, spec: &Spec, cal: &Calendar) -> Result<(bool, bool, String)> {
        #[cfg(not(feature = "shadow-oi-http"))]
        let _ = (spec, cal);
        match self {
            Self::Disabled { reason, .. } => Ok((false, false, reason.clone())),
            #[cfg(feature = "shadow-oi-http")]
            Self::Qualified { snapshot } => {
                let q = snapshot.verify(&crate::shadow_oi::Plan::new(
                    cal,
                    spec.date,
                    &spec.model_sha256,
                )?)?;
                Ok((
                    q.allow_new_bo(),
                    q.status == crate::shadow_oi::collector::Status::AcceptedSnapshot,
                    serde_json::to_value(q.status)?
                        .as_str()
                        .context("OI status")?
                        .to_string(),
                ))
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Freeze {
        barrier: ReceiptBarrier,
        oi: OiDecision,
    },
    Observe {
        through: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Audit {
    pub label_utc: i64,
    pub first_sequence: u64,
    pub repeated_sequence: u64,
    /// o,h,l,c,v bit mask. Context selection depends on h/l/c only.
    pub changed_fields: u8,
}
fn changed(a: &BarEvent, b: &BarEvent) -> u8 {
    [
        a.o != b.o,
        a.h != b.h,
        a.l != b.l,
        a.close != b.close,
        a.v != b.v,
    ]
    .iter()
    .enumerate()
    .fold(0, |mask, (i, different)| mask | (u8::from(*different) << i))
}
fn prefix(journal: &RiIngressJournal, sequence: u64) -> Result<Vec<Receipt>> {
    journal
        .identified_input_prefix(sequence)?
        .into_iter()
        .map(|(source_id, i)| {
            let r = Receipt {
                sequence: i.sequence,
                source_id,
                bar: i.bar,
                received_at_utc: i.received_at_utc,
                durable_at_utc: i.durable_at_utc,
            };
            ensure!(
                matches!(r.bar.origin, DataOrigin::Live | DataOrigin::History)
                    && r.bar.symbol == "IMOEXF"
                    && r.bar
                        .close_time_utc
                        .checked_add(600)
                        .is_some_and(|end| Some(end) <= r.received_at_utc)
                    && r.received_at_utc <= r.durable_at_utc
                    && r.durable_at_utc.is_some(),
                "unusable original source receipt"
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
                "invalid source OHLCV"
            );
            Ok(r)
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelRecord {
    pub id: String,
    pub arm: Arm,
    pub kind: String,
    pub payload: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Output {
    pub transaction: u64,
    pub outbox_id: String,
    pub observed_sequence: u64,
    pub frozen: Option<[Value; 3]>,
    pub audits: Vec<Audit>,
    pub records: Vec<ModelRecord>,
    pub ready: bool,
    pub complete: bool,
    pub oi_allows_new_bo: bool,
    pub oi_input_valid: bool,
    pub oi_status: String,
    pub hold_reason: Option<String>,
    pub order_emission_allowed: bool,
    pub operational_admission_required: bool,
}
#[derive(Serialize, Deserialize)]
struct Transaction {
    index: u64,
    previous_sha256: String,
    processed_at_utc: i64,
    operation: Operation,
    output: Output,
    checkpoints: Option<[String; 3]>,
}
impl LogRecord for Transaction {
    fn index(&self) -> u64 {
        self.index
    }
    fn previous_sha256(&self) -> &str {
        &self.previous_sha256
    }
}

#[derive(Clone, Default)]
struct State {
    contexts: Option<[FrozenComponents; 3]>,
    engines: Option<[Engine; 3]>,
    gates: Option<[BoGate; 3]>,
    cursor: u64,
    processed_at: Option<i64>,
    canonical: BTreeMap<i64, Receipt>,
    oi_allows: bool,
    oi_input_valid: bool,
    oi_status: String,
    hold_reason: Option<String>,
}
impl State {
    fn checkpoints(&self) -> Result<Option<[String; 3]>> {
        self.engines
            .as_ref()
            .map(|e| {
                Ok([
                    String::from_utf8(e[0].checkpoint_bytes()?)?,
                    String::from_utf8(e[1].checkpoint_bytes()?)?,
                    String::from_utf8(e[2].checkpoint_bytes()?)?,
                ])
            })
            .transpose()
    }
    fn apply(
        &mut self,
        spec: &Spec,
        cal: &Calendar,
        journal: &RiIngressJournal,
        op: &Operation,
        now: i64,
        index: u64,
    ) -> Result<Output> {
        ensure!(
            journal.identity() == &spec.ingress
                && now <= journal.now()?
                && self.processed_at.is_none_or(|old| now >= old),
            "source/processing clock changed"
        );
        let mut audits = Vec::new();
        let mut records = Vec::new();
        let mut frozen = None;
        let cutoff = cal.bounds_utc(spec.date).context("session bounds")?.0;
        match op {
            Operation::Freeze { barrier, oi } => {
                ensure!(
                    self.contexts.is_none()
                        && journal.sealed_barriers()?.contains(barrier)
                        && barrier.through_utc == cutoff
                        && barrier.attested_at_utc <= now
                        && now >= cutoff
                        && now < cutoff + 600,
                    "wrong, late or unattested session freeze"
                );
                for r in prefix(journal, barrier.last_sequence)? {
                    if local(r.bar.close_time_utc)?.date() >= spec.date
                        || r.durable_at_utc.unwrap() >= cutoff
                    {
                        continue;
                    }
                    if let Some(first) = self.canonical.get(&r.bar.close_time_utc) {
                        audits.push(Audit {
                            label_utc: r.bar.close_time_utc,
                            first_sequence: first.sequence,
                            repeated_sequence: r.sequence,
                            changed_fields: changed(&first.bar, &r.bar),
                        });
                    } else {
                        self.canonical.insert(r.bar.close_time_utc, r);
                    }
                }
                let projected: Vec<_> = self
                    .canonical
                    .values()
                    .cloned()
                    .enumerate()
                    .map(|(i, mut r)| {
                        r.sequence = i as u64 + 1;
                        r
                    })
                    .collect();
                let stream = format!("{}/canonical-m10-v1", spec.ingress.receipt_stream);
                let projection = Barrier {
                    receipt_stream: stream.clone(),
                    through_utc: cutoff,
                    last_sequence: projected.len() as u64,
                    ingress_sha256: context::prefix_sha256(&projected)?,
                    attested_at_utc: barrier.attested_at_utc,
                };
                let build = |arm| {
                    context::freeze(
                        cal,
                        Request {
                            arm,
                            model_sha256: spec.model_sha256.clone(),
                            receipt_stream: stream.clone(),
                            evidence: Evidence::DeclaredAsObserved,
                        },
                        spec.date,
                        now,
                        &projected,
                        &projection,
                    )
                };
                let contexts = [build(ARMS[0])?, build(ARMS[1])?, build(ARMS[2])?];
                let mut used_fields: BTreeMap<i64, u8> = BTreeMap::new();
                for component in contexts.iter().flat_map(|f| [&f.mr, &f.bo]) {
                    if let Some(previous) = &component.previous {
                        for label in cal.expected_labels_utc(previous.anchor.session_date) {
                            *used_fields.entry(label).or_default() |= 0b00110;
                        }
                        *used_fields.entry(previous.anchor.last_bar_utc).or_default() |= 0b01000;
                    }
                    if let Some(previous2) = &component.previous2 {
                        *used_fields
                            .entry(previous2.anchor.last_bar_utc)
                            .or_default() |= 0b01000;
                    }
                }
                let conflict = audits.iter().any(|a| {
                    a.changed_fields & used_fields.get(&a.label_utc).copied().unwrap_or(0) != 0
                });
                self.hold_reason = if conflict {
                    Some("selected_context_hlc_revision".into())
                } else if !contexts.iter().all(|c| c.dependency_ready) {
                    Some("component_context_not_ready".into())
                } else {
                    None
                };
                (self.oi_allows, self.oi_input_valid, self.oi_status) = oi.evaluate(spec, cal)?;
                let gates = [
                    BoGate::Off,
                    BoGate::Off,
                    BoGate::observed(&contexts[2], self.oi_allows, hash(oi)?)?,
                ];
                if !conflict && contexts.iter().all(|c| c.dependency_ready) {
                    self.engines = Some([
                        Engine::new(contexts[0].clone(), gates[0].clone())?,
                        Engine::new(contexts[1].clone(), gates[1].clone())?,
                        Engine::new(contexts[2].clone(), gates[2].clone())?,
                    ]);
                }
                frozen = Some([
                    serde_json::to_value(&contexts[0])?,
                    serde_json::to_value(&contexts[1])?,
                    serde_json::to_value(&contexts[2])?,
                ]);
                self.contexts = Some(contexts);
                self.gates = Some(gates);
                self.cursor = barrier.last_sequence;
            }
            Operation::Observe { through } => {
                ensure!(
                    self.contexts.is_some()
                        && *through > self.cursor
                        && *through - self.cursor <= 16
                        && local(now)?.date() == spec.date,
                    "invalid observation batch/session"
                );
                let rows = prefix(journal, *through)?;
                for r in &rows[self.cursor as usize..] {
                    let label = r.bar.close_time_utc;
                    if let Some(first) = self.canonical.get(&label) {
                        audits.push(Audit {
                            label_utc: label,
                            first_sequence: first.sequence,
                            repeated_sequence: r.sequence,
                            changed_fields: changed(&first.bar, &r.bar),
                        });
                        continue;
                    }
                    if r.bar.origin != DataOrigin::Live
                        || !cal.expected_labels_utc(spec.date).contains(&label)
                    {
                        continue;
                    }
                    ensure!(
                        r.durable_at_utc.unwrap() <= now
                            && now <= label + 600 + i64::from(spec.max_execution_lag_seconds),
                        "stale live model input"
                    );
                    if let Some(engines) = &mut self.engines {
                        let mut b = r.bar.clone();
                        b.origin = DataOrigin::Replay;
                        for (arm, e) in ARMS.iter().zip(engines.iter_mut()) {
                            let old = [e.events().len(), e.trades().len()];
                            e.observe(b.clone(), now)?;
                            for (kind, start, values) in [
                                ("event", old[0], serde_json::to_value(e.events())?),
                                ("trade", old[1], serde_json::to_value(e.trades())?),
                            ] {
                                for (n, payload) in
                                    values.as_array().unwrap().iter().enumerate().skip(start)
                                {
                                    records.push(ModelRecord {
                                        id: hash(&(
                                            &spec.ingress.generation,
                                            spec.date,
                                            arm,
                                            kind,
                                            n,
                                        ))?,
                                        arm: *arm,
                                        kind: kind.into(),
                                        payload: payload.clone(),
                                    });
                                }
                            }
                        }
                    }
                    self.canonical.insert(label, r.clone());
                }
                self.cursor = *through;
            }
        }
        self.processed_at = Some(now);
        Ok(Output {
            transaction: index,
            outbox_id: hash(&(&spec.ingress.generation, spec.date, index))?,
            observed_sequence: self.cursor,
            frozen,
            audits,
            records,
            ready: self.engines.is_some(),
            complete: self
                .engines
                .as_ref()
                .is_some_and(|e| e.iter().all(Engine::is_complete)),
            oi_allows_new_bo: self.oi_allows,
            oi_input_valid: self.oi_input_valid,
            oi_status: self.oi_status.clone(),
            hold_reason: self.hold_reason.clone(),
            order_emission_allowed: false,
            operational_admission_required: true,
        })
    }
}

pub struct Session {
    spec: Spec,
    calendar: Calendar,
    log: TransactionLog<Transaction>,
    state: State,
    clock: fn() -> i64,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub date: NaiveDate,
    pub committed_transactions: u64,
    pub transaction_bytes: u64,
    pub observed_sequence: u64,
    pub ready: bool,
    pub complete: bool,
    pub failed: bool,
    pub hold_reason: Option<String>,
    pub oi_allows_new_bo: bool,
    pub oi_input_valid: bool,
    pub oi_status: String,
    pub order_emission_allowed: bool,
    pub operational_admission_required: bool,
}
impl Session {
    pub fn status(&self) -> Status {
        let failed = self.log.healthy().is_err();
        Status {
            date: self.spec.date,
            committed_transactions: self.log.count(),
            transaction_bytes: self.log.bytes(),
            observed_sequence: self.state.cursor,
            ready: !failed && self.state.engines.is_some(),
            complete: !failed
                && self
                    .state
                    .engines
                    .as_ref()
                    .is_some_and(|e| e.iter().all(Engine::is_complete)),
            failed,
            hold_reason: if failed {
                Some("generation_failed_audit_required".into())
            } else if self.state.contexts.is_none() {
                Some("awaiting_freeze".into())
            } else {
                self.state.hold_reason.clone()
            },
            oi_allows_new_bo: !failed && self.state.oi_allows,
            oi_input_valid: self.state.oi_input_valid,
            oi_status: self.state.oi_status.clone(),
            order_emission_allowed: false,
            operational_admission_required: true,
        }
    }
    pub fn create(root: &Path, spec: Spec) -> Result<Self> {
        Self::create_with_clock(root, spec, || Utc::now().timestamp())
    }
    fn create_with_clock(root: &Path, spec: Spec, clock: fn() -> i64) -> Result<Self> {
        let calendar = spec.calendar()?;
        ensure!(
            clock() < calendar.bounds_utc(spec.date).unwrap().0,
            "create session before opening"
        );
        let header = serde_json::json!({"version":VERSION,"spec":spec});
        let log = TransactionLog::create(root, &header)?;
        Ok(Self {
            spec,
            calendar,
            log,
            state: State::default(),
            clock,
        })
    }
    pub fn open(root: &Path, expected: Spec, source: &RiIngressJournal) -> Result<Self> {
        Self::open_with_clock(root, expected, source, || Utc::now().timestamp())
    }
    fn open_with_clock(
        root: &Path,
        spec: Spec,
        source: &RiIngressJournal,
        clock: fn() -> i64,
    ) -> Result<Self> {
        ensure!(
            source.identity() == &spec.ingress,
            "session source identity changed"
        );
        let calendar = spec.calendar()?;
        let header = serde_json::json!({"version":VERSION,"spec":spec});
        let (log, transactions) = TransactionLog::<Transaction>::open(root, &header)?;
        let mut result = Self {
            spec,
            calendar,
            log,
            state: State::default(),
            clock,
        };
        for tx in transactions {
            ensure!(tx.processed_at_utc <= clock(), "future session transaction");
            let output = result.state.apply(
                &result.spec,
                &result.calendar,
                source,
                &tx.operation,
                tx.processed_at_utc,
                tx.index,
            )?;
            ensure!(
                output == tx.output && result.state.checkpoints()? == tx.checkpoints,
                "session witness differs from original receipt replay"
            );
        }
        if let (Some(contexts), Some(gates), Some(checkpoints)) = (
            &result.state.contexts,
            &result.state.gates,
            result.state.checkpoints()?,
        ) {
            result.state.engines = Some([
                Engine::restore(
                    contexts[0].clone(),
                    gates[0].clone(),
                    checkpoints[0].as_bytes(),
                )?,
                Engine::restore(
                    contexts[1].clone(),
                    gates[1].clone(),
                    checkpoints[1].as_bytes(),
                )?,
                Engine::restore(
                    contexts[2].clone(),
                    gates[2].clone(),
                    checkpoints[2].as_bytes(),
                )?,
            ]);
        }
        Ok(result)
    }
    fn commit(&mut self, source: &RiIngressJournal, op: Operation) -> Result<Output> {
        self.log.healthy()?;
        let now = (self.clock)();
        let mut staged = self.state.clone();
        let result = (|| {
            let output = staged.apply(
                &self.spec,
                &self.calendar,
                source,
                &op,
                now,
                self.log.count() + 1,
            )?;
            let tx = Transaction {
                index: self.log.count() + 1,
                previous_sha256: self.log.head().into(),
                processed_at_utc: now,
                operation: op,
                output: output.clone(),
                checkpoints: staged.checkpoints()?,
            };
            self.log.append(&tx)?;
            Ok(output)
        })();
        match result {
            Ok(output) => {
                self.state = staged;
                Ok(output)
            }
            Err(error) => Err(self.log.fail(error)),
        }
    }
    pub fn freeze(
        &mut self,
        source: &RiIngressJournal,
        barrier: ReceiptBarrier,
        oi_root: Option<&Path>,
    ) -> Result<Output> {
        self.log.healthy()?;
        ensure!(
            self.state.contexts.is_none(),
            "session already frozen; no late OI re-enable"
        );
        let oi = OiDecision::capture(&self.spec, &self.calendar, oi_root);
        self.commit(source, Operation::Freeze { barrier, oi })
    }
    /// Bounded processing of the existing owned journal, never a Redis mutation.
    pub fn observe(&mut self, source: &RiIngressJournal) -> Result<Option<Output>> {
        self.log.healthy()?;
        ensure!(
            source.identity() == &self.spec.ingress,
            "session source identity changed"
        );
        if source.last_sequence() == self.state.cursor {
            return Ok(None);
        }
        let through = source.last_sequence().min(self.state.cursor + 16);
        self.commit(source, Operation::Observe { through })
            .map(Some)
    }
    pub fn committed_output(&self, index: u64) -> Result<Output> {
        Ok(self.log.read(index)?.output)
    }
}

#[cfg(test)]
mod tests;
