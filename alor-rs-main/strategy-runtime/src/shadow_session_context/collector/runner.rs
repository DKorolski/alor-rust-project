//! Opt-in weekday/weekend reaction pair. Original receipts are the input WAL;
//! one transaction HEAD releases both contexts, model states and the outbox.
//! No Strategy/Intent, Redis writes, stdout decisions or broker transport.
use super::*;
use crate::shadow_reaction::{Config, NumericalSession};
use serde_json::Value;

// Shared fsync/HEAD protocol, separately typed schemas and generation directories.
// Generic storage does not alter the existing RI serialization or file version.
use crate::ri_context::runner::storage;
use storage::TransactionLog;

const VERSION: &str = "weekend_reaction_runner_v1";
const OBSERVATION_BATCH: u64 = 16;

pub mod bootstrap;
pub mod service;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerSpec {
    pub ingress: IngressIdentity,
    pub config: Config,
    pub first_decision_session: NaiveDate,
    pub excluded_dates: BTreeSet<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<bootstrap::HistoricalBootstrap>,
}
impl RunnerSpec {
    fn header(&self) -> Result<Header> {
        Collector::validate_identity(&self.ingress)?;
        let identity = Identity {
            profile: self.config.profile()?,
            symbol: self.ingress.symbol.clone(),
            model_config_sha256: self.config.fingerprint()?,
            receipt_stream: self.ingress.receipt_stream.clone(),
            policy: Policy::WeekdayOnly,
            excluded_dates: self.excluded_dates.clone(),
        };
        identity.validate()?;
        ensure!(
            identity.trading_date(self.first_decision_session),
            "non-trading activation date"
        );
        if let Some(bootstrap) = &self.bootstrap {
            bootstrap.validate(self.first_decision_session)?;
        }
        ensure!(
            identity.profile != Profile::UsdrubfPreviousDay09,
            "USD09 weekend pair remains offline-only"
        );
        if let Config::Usdrubf { settings } = &self.config {
            ensure!(
                settings.bo_wait == 4,
                "only H3/W4 and H4/W4 collector pairs approved"
            );
        }
        let mut candidate = identity.clone();
        candidate.policy = Policy::WeekendContextNoTrades;
        Ok(Header {
            version: VERSION.into(),
            spec: serde_json::to_value(self)?,
            identities: [identity, candidate],
        })
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Header {
    version: String,
    spec: Value,
    identities: [Identity; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Bootstrap,
    Ingest {
        through: u64,
    },
    Freeze {
        date: NaiveDate,
        cutoff: Box<CollectedCutoff>,
    },
    Observe {
        through: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    BeforeActivation,
    NonTradingBar,
    ContextOnly,
    Duplicate,
    ContextNotReady,
    ModelObserved,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PairedStep {
    pub receipt: Receipt,
    pub disposition: Disposition,
    /// Exact numerical states, including pending owner, counters, events/trades.
    /// These are simulated next-open reactions, never broker fills.
    pub states: Option<[Value; 2]>,
    /// Only newly added events/rounds. Do not sum cumulative state.trades.
    pub deltas: Option<[ModelDelta; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelRecord {
    pub id: String,
    pub payload: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelDelta {
    pub events: Vec<ModelRecord>,
    pub trades: Vec<ModelRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommittedOutput {
    pub generation: String,
    pub transaction: u64,
    pub outbox_id: String,
    pub ingested_sequence: u64,
    pub observed_sequence: u64,
    pub frozen: Option<[Frozen; 2]>,
    pub steps: Vec<PairedStep>,
    pub order_emission_allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_bootstrap: Option<bootstrap::HistoricalBootstrap>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Transaction {
    index: u64,
    previous_sha256: String,
    processed_at_utc: i64,
    operation: Operation,
    output: CommittedOutput,
    context_sha256: [String; 2],
    model_sha256: String,
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
    stores: [Store; 2],
    sessions: Option<[NumericalSession; 2]>,
    day: Option<NaiveDate>,
    observed: u64,
    model_inputs: BTreeMap<i64, Receipt>,
    processed_at: Option<i64>,
}
impl PairState {
    fn new(header: &Header) -> Result<Self> {
        Ok(Self {
            stores: [
                Store::new(header.identities[0].clone())?,
                Store::new(header.identities[1].clone())?,
            ],
            sessions: None,
            day: None,
            observed: 0,
            model_inputs: BTreeMap::new(),
            processed_at: None,
        })
    }
    fn states(&self) -> Result<Option<[Value; 2]>> {
        self.sessions
            .as_ref()
            .map(|s| Ok([s[0].state()?, s[1].state()?]))
            .transpose()
    }
    fn model_hash(&self) -> Result<String> {
        hash(&(
            self.day,
            self.observed,
            &self.model_inputs,
            self.states()?,
            self.processed_at,
        ))
    }
    fn deltas(
        &self,
        spec: &RunnerSpec,
        before: &[Value; 2],
        after: &[Value; 2],
    ) -> Result<[ModelDelta; 2]> {
        let records = |arm: usize, kind: &str| -> Result<Vec<ModelRecord>> {
            let old = before[arm]["model"]["state"][kind]
                .as_array()
                .context("missing old model records")?;
            let new = after[arm]["model"]["state"][kind]
                .as_array()
                .context("missing new model records")?;
            ensure!(
                new.starts_with(old),
                "model event/trade history was rewritten"
            );
            new.iter()
                .enumerate()
                .skip(old.len())
                .map(|(index, value)| {
                    Ok(ModelRecord {
                        id: hash(&(
                            &spec.ingress.generation,
                            self.stores[arm].identity(),
                            self.day,
                            kind,
                            index,
                        ))?,
                        payload: value.clone(),
                    })
                })
                .collect()
        };
        Ok([
            ModelDelta {
                events: records(0, "events")?,
                trades: records(0, "trades")?,
            },
            ModelDelta {
                events: records(1, "events")?,
                trades: records(1, "trades")?,
            },
        ])
    }
    fn freeze(
        &mut self,
        spec: &RunnerSpec,
        collector: &Collector,
        date: NaiveDate,
        cutoff: &CollectedCutoff,
        now: i64,
    ) -> Result<[Frozen; 2]> {
        collector.verify_cutoff(cutoff)?;
        let identity = self.stores[0].identity();
        let expected = if let Some(previous) = self.day {
            local(identity.next_cutoff(previous)?)?.date()
        } else {
            spec.first_decision_session
        };
        ensure!(
            date == expected && local(now)?.date() == date,
            "skipped/backdated decision session; explicit recovery required"
        );
        ensure!(
            cutoff.barrier().through_utc == identity.cutoff(date),
            "wrong calendar cutoff"
        );
        if let Some(sessions) = &self.sessions {
            for s in sessions {
                s.complete()?;
            }
        }
        // Never discard an unobserved previous-session live tail at rollover.
        for input in &collector.prefix(self.stores[0].last_sequence())?[self.observed as usize..] {
            let day = local(input.bar.close_time_utc)?.date();
            ensure!(
                input.bar.origin != DataOrigin::Live
                    || day < spec.first_decision_session
                    || day >= date
                    || !identity.trading_date(day),
                "unobserved live tail before freeze"
            );
        }
        let frozen = [
            self.stores[0].freeze(date, now, cutoff.barrier())?,
            self.stores[1].freeze(date, now, cutoff.barrier())?,
        ];
        self.sessions = if frozen.iter().all(Frozen::ready) {
            Some([
                NumericalSession::new(&self.stores[0], date, spec.config.clone())?,
                NumericalSession::new(&self.stores[1], date, spec.config.clone())?,
            ])
        } else {
            None
        };
        self.day = Some(date);
        self.model_inputs.clear();
        Ok(frozen)
    }
    fn observe(&mut self, spec: &RunnerSpec, input: &Receipt, now: i64) -> Result<Disposition> {
        let dt = local(input.bar.close_time_utc)?;
        let date = dt.date();
        let identity = self.stores[0].identity();
        if date < spec.first_decision_session {
            return Ok(Disposition::BeforeActivation);
        }
        if !identity.trading_date(date)
            || dt.time()
                < chrono::NaiveTime::from_hms_opt(identity.opening_hour(date), 0, 0).unwrap()
            || dt.time() > chrono::NaiveTime::from_hms_opt(23, 40, 0).unwrap()
        {
            return Ok(Disposition::NonTradingBar);
        }
        if input.bar.origin != DataOrigin::Live {
            return Ok(Disposition::ContextOnly);
        }
        ensure!(
            dt.minute().is_multiple_of(10) && dt.second() == 0,
            "non-M10 live model bar"
        );
        ensure!(
            date == local(now)?.date() && self.day == Some(date),
            "live receipt outside active session; no retrospective decisions"
        );
        let received = input.received_at_utc.context("missing receipt clock")?;
        let durable = input.durable_at_utc.context("missing durable clock")?;
        ensure!(
            received >= input.bar.close_time_utc + 600
                && received <= durable
                && durable <= now
                && local(received)?.date() == date,
            "invalid prospective receipt clocks"
        );
        if let Some(old) = self.model_inputs.get(&input.bar.close_time_utc) {
            ensure!(
                old.bar == input.bar,
                "conflicting already-observed live quote"
            );
            return Ok(Disposition::Duplicate);
        }
        // Only the original canonical receipt can drive the model. A history
        // replacement or a quarantined quote never becomes a prospective fill.
        for store in &self.stores {
            ensure!(
                store.canonical.get(&input.bar.close_time_utc) == Some(input),
                "live receipt not canonical in both arms"
            );
        }
        let Some(sessions) = &mut self.sessions else {
            return Ok(Disposition::ContextNotReady);
        };
        let mut numerical = input.bar.clone();
        numerical.origin = DataOrigin::Replay;
        // The conversion is private and follows original receipt checks above;
        // NumericalSession's public Live-origin prohibition remains unchanged.
        for s in sessions {
            s.observe(numerical.clone(), now)?;
        }
        self.model_inputs
            .insert(input.bar.close_time_utc, input.clone());
        Ok(Disposition::ModelObserved)
    }
    fn apply(
        &mut self,
        spec: &RunnerSpec,
        collector: &Collector,
        operation: &Operation,
        now: i64,
    ) -> Result<(Vec<PairedStep>, Option<[Frozen; 2]>)> {
        ensure!(
            now <= collector.journal.now()? && self.processed_at.is_none_or(|old| now >= old),
            "runner clock regression/future operation"
        );
        let mut steps = vec![];
        let mut frozen = None;
        match operation {
            Operation::Bootstrap => {
                let plan = spec
                    .bootstrap
                    .as_ref()
                    .context("bootstrap not configured")?;
                let inputs = collector.prefix(plan.rows)?;
                plan.verify(&inputs, spec.first_decision_session)?;
                ensure!(
                    self.observed == 0 && self.day.is_none(),
                    "bootstrap after observation"
                );
                for store in &mut self.stores {
                    store.bootstrap(&inputs, spec.first_decision_session, now)?;
                }
                self.observed = plan.rows;
            }
            Operation::Ingest { through } => {
                ensure!(
                    spec.bootstrap
                        .as_ref()
                        .is_none_or(|b| self.stores[0].last_sequence() >= b.rows),
                    "historical bootstrap must commit before normal ingestion"
                );
                ensure!(
                    *through > self.stores[0].last_sequence(),
                    "non-advancing ingestion"
                );
                let inputs = collector.prefix(*through)?;
                for store in &mut self.stores {
                    for input in &inputs[store.last_sequence() as usize..] {
                        store.ingest(input.clone(), now)?;
                    }
                }
            }
            Operation::Freeze { date, cutoff } => {
                frozen = Some(self.freeze(spec, collector, *date, cutoff, now)?);
            }
            Operation::Observe { through } => {
                ensure!(
                    *through > self.observed
                        && *through - self.observed <= OBSERVATION_BATCH
                        && *through <= self.stores[0].last_sequence(),
                    "invalid observation prefix"
                );
                for input in &collector.prefix(*through)?[self.observed as usize..] {
                    let before = self.states()?;
                    let disposition = self.observe(spec, input, now)?;
                    let states = if disposition == Disposition::ModelObserved {
                        self.states()?
                    } else {
                        None
                    };
                    let deltas = states
                        .as_ref()
                        .map(|after| {
                            self.deltas(
                                spec,
                                before
                                    .as_ref()
                                    .context("model missing before observation")?,
                                after,
                            )
                        })
                        .transpose()?;
                    steps.push(PairedStep {
                        receipt: input.clone(),
                        disposition,
                        states,
                        deltas,
                    });
                }
                self.observed = *through;
            }
        }
        self.processed_at = Some(now);
        collector.validate_pair(&self.stores)?;
        Ok((steps, frozen))
    }
}

#[derive(Debug, Serialize)]
pub struct RunnerStatus {
    pub committed_transactions: u64,
    pub durable_receipts: u64,
    pub ingested_sequence: u64,
    pub observed_sequence: u64,
    pub transaction_bytes: u64,
    pub source_caught_up: bool,
    pub failed: bool,
    pub session_date: Option<NaiveDate>,
    pub pair_ready: bool,
    pub session_complete: bool,
    pub order_emission_allowed: bool,
    pub bootstrap_status: &'static str,
    pub bootstrap_rows: u64,
}

pub struct ReactionShadowRunner {
    header: Header,
    spec: RunnerSpec,
    collector: Collector,
    log: TransactionLog<Transaction>,
    state: PairState,
}
impl ReactionShadowRunner {
    pub fn create(root: &Path, spec: RunnerSpec) -> Result<Self> {
        Self::create_with_clock(root, spec, || Utc::now().timestamp())
    }
    fn create_with_clock(root: &Path, spec: RunnerSpec, clock: fn() -> i64) -> Result<Self> {
        let header = spec.header()?;
        ensure!(
            clock() < header.identities[0].cutoff(spec.first_decision_session),
            "create before first decision cutoff"
        );
        fs::create_dir(root).context("new generation directory required")?;
        storage::sync_parent(root)?;
        let log = TransactionLog::create(&root.join("transactions"), &header)?;
        let mut collector =
            Collector::create_with_clock(&root.join("ingress"), spec.ingress.clone(), clock)?;
        if let Some(b) = &spec.bootstrap {
            collector.source.start_at(&b.first_source_id)?;
        }
        let state = PairState::new(&header)?;
        Ok(Self {
            header,
            spec,
            collector,
            log,
            state,
        })
    }
    pub fn open(root: &Path, spec: RunnerSpec) -> Result<Self> {
        Self::open_with_clock(root, spec, || Utc::now().timestamp())
    }
    fn open_with_clock(root: &Path, spec: RunnerSpec, clock: fn() -> i64) -> Result<Self> {
        ensure!(
            fs::symlink_metadata(root)?.file_type().is_dir(),
            "invalid runner root"
        );
        let mut names = fs::read_dir(root)?
            .map(|e| Ok(e?.file_name()))
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        ensure!(
            names == ["ingress", "transactions"].map(std::ffi::OsString::from),
            "unknown runner files; audit required"
        );
        let header = spec.header()?;
        let (log, transactions) = TransactionLog::open(&root.join("transactions"), &header)?;
        let mut collector =
            Collector::open_with_clock(&root.join("ingress"), spec.ingress.clone(), clock)?;
        if let Some(b) = &spec.bootstrap {
            collector.source.start_at(&b.first_source_id)?;
        }
        let state = PairState::new(&header)?;
        let mut runner = Self {
            header,
            spec,
            collector,
            log,
            state,
        };
        let restored = (|| -> Result<()> {
            let mut barriers = vec![];
            for tx in transactions {
                let (next, actual) = runner.prepare(
                    tx.index,
                    tx.previous_sha256.clone(),
                    tx.operation.clone(),
                    tx.processed_at_utc,
                )?;
                ensure!(
                    actual == tx,
                    "reconstructed transaction differs; no flat fallback"
                );
                if let Operation::Freeze { cutoff, .. } = &tx.operation {
                    barriers.push(cutoff.original.clone());
                }
                runner.state = next;
            }
            ensure!(
                barriers == runner.collector.journal.sealed_barriers()?,
                "uncommitted cutoff; explicit audit required"
            );
            Ok(())
        })();
        restored.map_err(|e| runner.log.fail(e))?;
        Ok(runner)
    }
    fn prepare(
        &self,
        index: u64,
        previous_sha256: String,
        operation: Operation,
        now: i64,
    ) -> Result<(PairState, Transaction)> {
        let mut next = self.state.clone();
        let (steps, frozen) = tracing::subscriber::with_default(
            tracing::subscriber::NoSubscriber::default(),
            || next.apply(&self.spec, &self.collector, &operation, now),
        )?;
        let historical_bootstrap = if matches!(operation, Operation::Bootstrap) {
            self.spec.bootstrap.clone()
        } else {
            None
        };
        let tx = Transaction {
            index,
            previous_sha256,
            processed_at_utc: now,
            operation,
            output: CommittedOutput {
                generation: self.spec.ingress.generation.clone(),
                transaction: index,
                outbox_id: format!("{}:{index}", hash(&self.header)?),
                ingested_sequence: next.stores[0].last_sequence(),
                observed_sequence: next.observed,
                frozen,
                steps,
                order_emission_allowed: false,
                historical_bootstrap,
            },
            context_sha256: [
                hash(&next.stores[0].checkpoint_bytes()?)?,
                hash(&next.stores[1].checkpoint_bytes()?)?,
            ],
            model_sha256: next.model_hash()?,
        };
        Ok((next, tx))
    }
    fn commit(&mut self, operation: Operation, now: i64) -> Result<()> {
        self.log.healthy()?;
        let (next, tx) =
            self.prepare(self.log.count() + 1, self.log.head().into(), operation, now)?;
        self.log.append(&tx)?;
        self.state = next;
        Ok(())
    }
    fn advance(&mut self) -> Result<()> {
        self.log.healthy()?;
        if let Some(b) = &self.spec.bootstrap {
            if self.state.stores[0].last_sequence() == 0 {
                if self.collector.journal.last_sequence() < b.rows {
                    ensure!(
                        !self.collector.status().caught_up,
                        "bootstrap range incomplete/trimmed"
                    );
                    return Ok(());
                }
                self.commit(Operation::Bootstrap, self.collector.journal.now()?)?;
            }
        }
        if self.collector.journal.last_sequence() > self.state.stores[0].last_sequence() {
            self.commit(
                Operation::Ingest {
                    through: self.collector.journal.last_sequence(),
                },
                self.collector.journal.now()?,
            )?;
        }
        if !self.collector.status().caught_up {
            return Ok(());
        }
        let now = self.collector.journal.now()?;
        let date = local(now)?.date();
        let identity = &self.header.identities[0];
        if date >= self.spec.first_decision_session
            && identity.trading_date(date)
            && now >= identity.cutoff(date)
            && self.state.day != Some(date)
        {
            let cutoff = self.collector.seal_cutoff(identity.cutoff(date))?;
            self.commit(
                Operation::Freeze {
                    date,
                    cutoff: Box::new(cutoff),
                },
                self.collector.journal.now()?,
            )?;
        }
        while self.state.observed < self.state.stores[0].last_sequence() {
            let through =
                (self.state.observed + OBSERVATION_BATCH).min(self.state.stores[0].last_sequence());
            self.commit(
                Operation::Observe { through },
                self.collector.journal.now()?,
            )?;
        }
        Ok(())
    }
    pub async fn poll_once(
        &mut self,
        connection: &mut MultiplexedConnection,
    ) -> Result<RunnerStatus> {
        self.log.healthy()?;
        let result = match self.collector.poll_once(connection).await {
            Ok(_) => self.advance(),
            Err(error) => Err(error),
        };
        result.map_err(|e| self.log.fail(e))?;
        Ok(self.status())
    }
    pub fn status(&self) -> RunnerStatus {
        let collector = self.collector.status();
        RunnerStatus {
            committed_transactions: self.log.count(),
            durable_receipts: collector.usage.records as u64,
            ingested_sequence: self.state.stores[0].last_sequence(),
            observed_sequence: self.state.observed,
            transaction_bytes: self.log.bytes(),
            source_caught_up: collector.caught_up,
            failed: collector.failed || self.log.healthy().is_err(),
            session_date: self.state.day,
            pair_ready: self.state.sessions.is_some(),
            session_complete: self
                .state
                .sessions
                .as_ref()
                .is_some_and(|s| s.iter().all(|s| s.complete().is_ok())),
            order_emission_allowed: false,
            bootstrap_status: match &self.spec.bootstrap {
                None => "disabled",
                Some(b) if self.state.stores[0].last_sequence() >= b.rows => "loaded",
                Some(_) => "pending",
            },
            bootstrap_rows: self.spec.bootstrap.as_ref().map_or(0, |b| b.rows),
        }
    }
    /// Outbox delivery is at-least-once. Consumers must deduplicate by outbox_id;
    /// restart reconstructs state but does not publish it a second time.
    pub fn committed_output(&self, index: u64) -> Result<CommittedOutput> {
        self.log.read(index).map(|tx| tx.output)
    }
}

#[cfg(test)]
mod tests;
