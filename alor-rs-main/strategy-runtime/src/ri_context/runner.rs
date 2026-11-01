//! Opt-in paired RI shadow runner. The local transaction log is both the recovery
//! WAL and the canonical decision outbox. No broker transport or Redis writes.
use std::fs;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use chrono::{NaiveDate, Utc};
use redis::aio::MultiplexedConnection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ingress::{redis_source::RiRedisSource, IngressIdentity, RiIngressJournal};
use super::{
    hash, local, ContextIdentity, ContextPolicy, FrozenContext, ReceiptBarrier, RiContextStore,
};
use crate::strategies::ri_author41_42_live::{
    frozen_shadow::{context_identity, RiFrozenShadowAdapter, ShadowGeneration, ShadowStep},
    RiAuthor4142LiveConfig,
};
use crate::strategy_host::DataOrigin;

pub(crate) mod storage;
use storage::TransactionLog;

const VERSION: &str = "ri_paired_shadow_runner_v1";
const OBSERVATION_BATCH: u64 = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunnerSpec {
    pub generation: ShadowGeneration,
    pub source_stream: String,
    pub receipt_stream: String,
    pub strategy: RiAuthor4142LiveConfig,
}

impl RunnerSpec {
    fn ingress_identity(&self) -> IngressIdentity {
        IngressIdentity {
            generation: self.generation.id.clone(),
            receipt_stream: self.receipt_stream.clone(),
            source_stream: self.source_stream.clone(),
            symbol: self.strategy.symbol.clone(),
        }
    }

    fn identities(&self) -> Result<[ContextIdentity; 2]> {
        ensure!(
            !self.source_stream.trim().is_empty() && self.source_stream.len() <= 512,
            "invalid source stream"
        );
        Ok([
            context_identity(
                &self.strategy,
                self.receipt_stream.clone(),
                ContextPolicy::RuntimeAcceptedOnly,
            )?,
            context_identity(
                &self.strategy,
                self.receipt_stream.clone(),
                ContextPolicy::RiConfirmedClosingContextV1,
            )?,
        ])
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Header {
    version: String,
    spec: RunnerSpec,
    identities: [ContextIdentity; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Ingest {
        through: u64,
    },
    Freeze {
        date: NaiveDate,
        barrier: ReceiptBarrier,
    },
    Observe {
        through: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PairedStep {
    pub sequence: u64,
    /// Fixed order: runtime_accepted_only, ri_confirmed_closing_context_v1.
    pub arms: [ShadowStep; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommittedOutput {
    pub generation: String,
    pub transaction: u64,
    pub outbox_id: String,
    pub ingested_sequence: u64,
    pub observed_sequence: u64,
    pub source_id: Option<String>,
    pub frozen: Option<[FrozenContext; 2]>,
    pub steps: Vec<PairedStep>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Transaction {
    index: u64,
    previous_sha256: String,
    processed_at_utc: i64,
    operation: Operation,
    output: CommittedOutput,
    ingress_sha256: String,
    context_sha256: [String; 2],
    witnesses: [String; 2],
}
impl storage::LogRecord for Transaction {
    fn index(&self) -> u64 {
        self.index
    }
    fn previous_sha256(&self) -> &str {
        &self.previous_sha256
    }
}

#[derive(Clone)]
struct PairState {
    stores: [RiContextStore; 2],
    adapters: [RiFrozenShadowAdapter; 2],
    observed: u64,
    processed_at_utc: Option<i64>,
}

impl PairState {
    fn new(header: &Header) -> Result<Self> {
        let make = |identity: &ContextIdentity| -> Result<_> {
            Ok((
                RiContextStore::new(identity.clone())?,
                RiFrozenShadowAdapter::new(
                    header.spec.strategy.clone(),
                    identity.clone(),
                    header.spec.generation.clone(),
                )?,
            ))
        };
        let (s0, a0) = make(&header.identities[0])?;
        let (s1, a1) = make(&header.identities[1])?;
        Ok(Self {
            stores: [s0, s1],
            adapters: [a0, a1],
            observed: 0,
            processed_at_utc: None,
        })
    }

    fn apply(
        &mut self,
        spec: &RunnerSpec,
        journal: &RiIngressJournal,
        operation: &Operation,
        now: i64,
    ) -> Result<(Vec<PairedStep>, Option<[FrozenContext; 2]>)> {
        ensure!(now <= journal.now()?, "future runner operation");
        ensure!(
            self.processed_at_utc.is_none_or(|old| now >= old),
            "runner clock regression"
        );
        let mut steps = vec![];
        let mut frozen = None;
        match operation {
            Operation::Ingest { through } => {
                ensure!(
                    *through > self.stores[0].last_sequence(),
                    "non-advancing ingress operation"
                );
                for store in &mut self.stores {
                    journal.replay_through(store, *through, now)?;
                }
            }
            Operation::Freeze { date, barrier } => {
                ensure!(
                    journal.contains_barrier(barrier),
                    "cutoff has no original durable ingress attestation"
                );
                ensure!(
                    *date >= spec.generation.first_decision_session,
                    "freeze before activation session"
                );
                ensure!(
                    local(now)?.date() == *date,
                    "freeze must belong to current session"
                );
                ensure!(
                    barrier.through_utc == self.stores[0].identity().cutoff(*date),
                    "wrong session cutoff"
                );
                for store in &mut self.stores {
                    ensure!(
                        store.frozen(*date).is_none(),
                        "duplicate freeze transaction"
                    );
                    store.freeze(*date, now, barrier)?;
                }
                frozen = Some([
                    self.stores[0]
                        .frozen(*date)
                        .context("missing control freeze")?
                        .clone(),
                    self.stores[1]
                        .frozen(*date)
                        .context("missing candidate freeze")?
                        .clone(),
                ]);
            }
            Operation::Observe { through } => {
                ensure!(
                    *through > self.observed && *through - self.observed <= OBSERVATION_BATCH,
                    "invalid observation batch"
                );
                ensure!(
                    *through <= self.stores[0].last_sequence(),
                    "observation ahead of context ingestion"
                );
                let inputs = journal.input_prefix(*through)?;
                for input in &inputs[self.observed as usize..] {
                    if input.bar.origin == DataOrigin::Live {
                        let day = local(input.bar.close_time_utc)?.date();
                        if day >= spec.generation.first_decision_session
                            && input.received_at_utc.is_some_and(|ts| {
                                local(ts).map(|t| t.date() == day).unwrap_or(false)
                            })
                            && self.stores.iter().any(|s| {
                                s.verified_route(input)
                                    .is_ok_and(|r| r.context_update_allowed())
                            })
                        {
                            ensure!(day == local(now)?.date(), "uncommitted live receipt crossed session; operator recovery required");
                        }
                    }
                    steps.push(PairedStep {
                        sequence: input.sequence,
                        arms: [
                            self.adapters[0].process(&self.stores[0], input)?,
                            self.adapters[1].process(&self.stores[1], input)?,
                        ],
                    });
                }
                self.observed = *through;
            }
        }
        self.processed_at_utc = Some(now);
        Ok((steps, frozen))
    }
}

#[derive(Debug, Serialize)]
pub struct RunnerStatus {
    pub committed_transactions: u64,
    pub durable_receipts: u64,
    pub ingested_sequence: u64,
    pub observed_sequence: u64,
    pub source_caught_up: bool,
    pub failed: bool,
    pub transaction_bytes: u64,
    pub last_frozen_session: Option<NaiveDate>,
    pub context_ready: [bool; 2],
}

/// Local-only orchestration. Callers read committed outbox records by index;
/// there is deliberately no stdout decision stream or outbound order publisher.
pub struct RiShadowRunner {
    header: Header,
    journal: RiIngressJournal,
    source: RiRedisSource,
    log: TransactionLog<Transaction>,
    state: PairState,
}

impl RiShadowRunner {
    pub fn create(root: &Path, spec: RunnerSpec) -> Result<Self> {
        Self::create_with_clock(root, spec, || Utc::now().timestamp())
    }

    fn create_with_clock(root: &Path, spec: RunnerSpec, clock: fn() -> i64) -> Result<Self> {
        let header = Header {
            version: VERSION.into(),
            identities: spec.identities()?,
            spec,
        };
        let state = PairState::new(&header)?;
        ensure!(
            clock() < header.identities[0].cutoff(header.spec.generation.first_decision_session),
            "new generation must start before first decision cutoff"
        );
        fs::create_dir(root).context("runner requires a NEW generation directory")?;
        storage::sync_parent(root)?;
        let log = TransactionLog::create(&root.join("transactions"), &header)?;
        let journal = RiIngressJournal::create_with_clock(
            &root.join("ingress"),
            header.spec.ingress_identity(),
            clock,
        )?;
        let source = RiRedisSource::new(journal.identity().clone())?;
        Ok(Self {
            header,
            journal,
            source,
            log,
            state,
        })
    }

    pub fn open(root: &Path, expected: RunnerSpec) -> Result<Self> {
        Self::open_with_clock(root, expected, || Utc::now().timestamp())
    }

    fn open_with_clock(root: &Path, expected: RunnerSpec, clock: fn() -> i64) -> Result<Self> {
        ensure!(
            fs::symlink_metadata(root)?.file_type().is_dir(),
            "non-regular runner root"
        );
        let mut names = fs::read_dir(root)?
            .map(|e| Ok(e?.file_name()))
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        ensure!(
            names == ["ingress", "transactions"].map(std::ffi::OsString::from),
            "unexpected runner files; audit required"
        );
        let header = Header {
            version: VERSION.into(),
            identities: expected.identities()?,
            spec: expected,
        };
        let (log, transactions) =
            TransactionLog::<Transaction>::open(&root.join("transactions"), &header)?;
        let journal = RiIngressJournal::open_with_clock(
            &root.join("ingress"),
            &header.spec.ingress_identity(),
            clock,
        )?;
        let mut state = PairState::new(&header)?;
        for tx in transactions {
            let (next, actual) = Self::prepare(
                &header,
                &state,
                &journal,
                tx.index,
                tx.previous_sha256.clone(),
                tx.operation.clone(),
                tx.processed_at_utc,
            )?;
            ensure!(
                actual == tx,
                "paired runner recovery mismatch; no flat fallback"
            );
            state = next;
        }
        let source = RiRedisSource::new(journal.identity().clone())?;
        Ok(Self {
            header,
            journal,
            source,
            log,
            state,
        })
    }

    fn prepare(
        header: &Header,
        current: &PairState,
        journal: &RiIngressJournal,
        index: u64,
        previous_sha256: String,
        operation: Operation,
        now: i64,
    ) -> Result<(PairState, Transaction)> {
        let mut next = current.clone();
        // Speculative calculations/recovery must not leak duplicate decision logs.
        let (steps, frozen) = tracing::subscriber::with_default(
            tracing::subscriber::NoSubscriber::default(),
            || next.apply(&header.spec, journal, &operation, now),
        )?;
        let ingested = next.stores[0].last_sequence();
        let output = CommittedOutput {
            generation: header.spec.generation.id.clone(),
            transaction: index,
            outbox_id: format!("{}:{index}", hash(header)?),
            ingested_sequence: ingested,
            observed_sequence: next.observed,
            source_id: journal.source_id_at(ingested)?,
            frozen,
            steps,
        };
        let tx = Transaction {
            index,
            previous_sha256,
            processed_at_utc: now,
            operation,
            output,
            ingress_sha256: next.stores[0].ingress_sha256()?,
            context_sha256: [
                format!("{:x}", Sha256::digest(next.stores[0].checkpoint_bytes()?)),
                format!("{:x}", Sha256::digest(next.stores[1].checkpoint_bytes()?)),
            ],
            witnesses: [
                String::from_utf8(next.adapters[0].recovery_witness(&next.stores[0])?)?,
                String::from_utf8(next.adapters[1].recovery_witness(&next.stores[1])?)?,
            ],
        };
        Ok((next, tx))
    }

    fn commit(&mut self, operation: Operation, now: i64) -> Result<()> {
        self.log.healthy()?;
        let (next, tx) = Self::prepare(
            &self.header,
            &self.state,
            &self.journal,
            self.log.count() + 1,
            self.log.head().into(),
            operation,
            now,
        )?;
        self.log.append(&tx)?;
        self.state = next;
        Ok(())
    }

    async fn poll_inner(&mut self, connection: &mut MultiplexedConnection) -> Result<()> {
        self.log.healthy()?;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.source.poll(connection, &mut self.journal),
        )
        .await
        .context("shadow Redis poll timed out")??;
        let now = self.journal.now()?;
        if self.journal.last_sequence() > self.state.stores[0].last_sequence() {
            self.commit(
                Operation::Ingest {
                    through: self.journal.last_sequence(),
                },
                now,
            )?;
        }
        if !self.source.caught_up(&self.journal) {
            return Ok(());
        }
        let date = local(now)?.date();
        let identity = &self.header.identities[0];
        if date >= self.header.spec.generation.first_decision_session
            && identity.regular_date(date)
            && now >= identity.cutoff(date)
            && self.state.stores[0].frozen(date).is_none()
        {
            let barrier = self
                .source
                .seal_cutoff(&mut self.journal, identity.cutoff(date))?;
            self.commit(Operation::Freeze { date, barrier }, self.journal.now()?)?;
        }
        while self.state.observed < self.state.stores[0].last_sequence() {
            let through =
                (self.state.observed + OBSERVATION_BATCH).min(self.state.stores[0].last_sequence());
            self.commit(Operation::Observe { through }, self.journal.now()?)?;
        }
        Ok(())
    }

    /// Any source/storage/model failure stops this generation durably. An ordinary
    /// service restart is not authorization to skip an unknown interval.
    pub async fn poll_once(
        &mut self,
        connection: &mut MultiplexedConnection,
    ) -> Result<RunnerStatus> {
        if let Err(error) = self.poll_inner(connection).await {
            return Err(self.log.fail(error));
        }
        Ok(self.status())
    }

    pub fn status(&self) -> RunnerStatus {
        RunnerStatus {
            committed_transactions: self.log.count(),
            durable_receipts: self.journal.last_sequence(),
            ingested_sequence: self.state.stores[0].last_sequence(),
            observed_sequence: self.state.observed,
            source_caught_up: self.source.caught_up(&self.journal),
            failed: self.log.healthy().is_err(),
            transaction_bytes: self.log.bytes(),
            last_frozen_session: self.state.stores[0]
                .state
                .frozen
                .keys()
                .next_back()
                .copied(),
            context_ready: self.state.stores.each_ref().map(|s| {
                s.state
                    .frozen
                    .values()
                    .next_back()
                    .is_some_and(FrozenContext::ready)
            }),
        }
    }

    pub fn committed_output(&self, index: u64) -> Result<CommittedOutput> {
        self.log.read(index).map(|tx| tx.output)
    }
}

#[cfg(test)]
mod tests;
