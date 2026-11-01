//! Durable ingress bridge for matched weekday/weekend reaction contexts.
//! Reuses the RI raw journal and read-only Redis transport, NOT its model policy.
//! The optional runner submodule adds paired numerical execution/outbox; the
//! collector alone has no model execution, Strategy or broker route.
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use redis::aio::MultiplexedConnection;

use super::*;
use crate::ri_context::ingress::{
    redis_source::RiRedisSource, IngressIdentity, IngressUsage, RiIngressJournal,
};
use crate::ri_context::ReceiptBarrier;

pub mod runner;

const PAIR_VERSION: &str = "weekend_collected_context_pair_v1";
const MAX_PAIR_BYTES: usize = 80 * 1024 * 1024;

/// The original journal attestation and its independently recomputed context
/// projection. A rehashed context checkpoint alone is not receipt proof.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CollectedCutoff {
    collector: IngressIdentity,
    original: ReceiptBarrier,
    projected: Barrier,
}
impl CollectedCutoff {
    pub fn barrier(&self) -> &Barrier {
        &self.projected
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairPayload {
    version: String,
    cutoff: CollectedCutoff,
    session_date: NaiveDate,
    checkpoints: [String; 2],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairFile {
    sha256: String,
    payload: PairPayload,
}

#[derive(Debug, Serialize)]
pub struct CollectorStatus {
    pub usage: IngressUsage,
    pub caught_up: bool,
    pub failed: bool,
    pub order_emission_allowed: bool,
}

/// One collector per paired generation. The inherited journal lock protects both
/// arms. Fresh history receives the current clock, never its bar/publication time.
pub struct Collector {
    journal: RiIngressJournal,
    source: RiRedisSource,
    root: PathBuf,
    failed: bool,
    // Derived only from the verified, append-only journal owned by this instance.
    // Never persisted or restored from a caller-provided context snapshot.
    projected_cutoffs: Mutex<Vec<CollectedCutoff>>,
}
impl Collector {
    pub fn create(root: &Path, identity: IngressIdentity) -> Result<Self> {
        Self::create_with_clock(root, identity, || Utc::now().timestamp())
    }

    fn create_with_clock(
        root: &Path,
        identity: IngressIdentity,
        clock: fn() -> i64,
    ) -> Result<Self> {
        Self::validate_identity(&identity)?;
        let source = RiRedisSource::new(identity.clone())?;
        let journal = RiIngressJournal::create_with_clock(root, identity, clock)?;
        Ok(Self {
            journal,
            source,
            root: root.into(),
            failed: false,
            projected_cutoffs: Mutex::new(Vec::new()),
        })
    }

    pub fn open(root: &Path, expected: IngressIdentity) -> Result<Self> {
        Self::open_with_clock(root, expected, || Utc::now().timestamp())
    }

    fn open_with_clock(root: &Path, expected: IngressIdentity, clock: fn() -> i64) -> Result<Self> {
        Self::validate_identity(&expected)?;
        let journal = RiIngressJournal::open_with_clock(root, &expected, clock)?;
        let source = RiRedisSource::new(expected)?;
        Ok(Self {
            journal,
            source,
            root: root.into(),
            failed: false,
            projected_cutoffs: Mutex::new(Vec::new()),
        })
    }

    fn validate_identity(identity: &IngressIdentity) -> Result<()> {
        ensure!(
            matches!(identity.symbol.as_str(), "IMOEXF" | "USDRUBF"),
            "unsupported reaction instrument"
        );
        ensure!(
            !identity.receipt_stream.trim().is_empty() && identity.receipt_stream.len() <= 256,
            "invalid reaction receipt stream"
        );
        Ok(())
    }

    fn healthy(&self) -> Result<()> {
        ensure!(
            !self.failed,
            "reaction collector failed; operator audit required"
        );
        self.journal.now()?;
        Ok(())
    }

    fn fail(&mut self, error: anyhow::Error) -> anyhow::Error {
        self.failed = true;
        // Unknown files already block the shared journal's normal open path.
        // If even this marker fails, a supervisor must prevent automatic restart.
        let saved = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.root.join("FAILED_REACTION_CONTEXT.json"))?;
            file.write_all(
                serde_json::to_string(&serde_json::json!({
                    "error_sha256": hash(&format!("{error:#}"))?,
                    "durable_receipts": self.journal.last_sequence(),
                }))?
                .as_bytes(),
            )?;
            file.sync_all()?;
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if saved.is_ok() {
            error.context("reaction generation marked FAILED; automatic restart forbidden")
        } else {
            error.context("reaction failure marker unavailable; supervisor must block restart")
        }
    }

    pub fn status(&self) -> CollectorStatus {
        let failed = self.healthy().is_err();
        CollectorStatus {
            usage: self.journal.usage(),
            caught_up: !failed && self.source.caught_up(&self.journal),
            failed,
            order_emission_allowed: false,
        }
    }

    /// Never acknowledges, trims or writes Redis. Any unknown source interval
    /// stops this generation; restart is not authorization to skip a cursor.
    pub async fn poll_once(
        &mut self,
        connection: &mut MultiplexedConnection,
    ) -> Result<CollectorStatus> {
        self.healthy()?;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.source.poll(connection, &mut self.journal),
        )
        .await
        .context("reaction source poll timed out")
        .and_then(|r| r);
        if let Err(error) = result {
            return Err(self.fail(error));
        }
        Ok(self.status())
    }

    fn prefix(&self, sequence: u64) -> Result<Vec<Receipt>> {
        self.healthy()?;
        self.journal
            .identified_input_prefix(sequence)?
            .into_iter()
            .map(|(source_id, i)| {
                ensure!(
                    !source_id.is_empty() && source_id.len() <= 256,
                    "unsupported source ID length"
                );
                Ok(Receipt {
                    sequence: i.sequence,
                    source_id,
                    bar: i.bar,
                    received_at_utc: i.received_at_utc,
                    durable_at_utc: i.durable_at_utc,
                })
            })
            .collect()
    }

    fn project_cutoff(&self, original: &ReceiptBarrier) -> Result<CollectedCutoff> {
        self.healthy()?;
        ensure!(
            self.journal.sealed_barriers()?.contains(original),
            "cutoff not attested in original journal"
        );
        let mut cache = self
            .projected_cutoffs
            .lock()
            .map_err(|_| anyhow::anyhow!("cutoff projection cache poisoned"))?;
        if let Some(projected) = cache.iter().find(|c| c.original == *original) {
            return Ok(projected.clone());
        }
        let projected = Barrier {
            receipt_stream: original.receipt_stream.clone(),
            through_utc: original.through_utc,
            last_sequence: original.last_sequence,
            ingress_sha256: hash(&self.prefix(original.last_sequence)?)?,
            attested_at_utc: original.attested_at_utc,
        };
        let cutoff = CollectedCutoff {
            collector: self.journal.identity().clone(),
            original: original.clone(),
            projected,
        };
        // Membership above bounds this cache by the journal's sealed-barrier cap.
        cache.push(cutoff.clone());
        Ok(cutoff)
    }

    fn verify_cutoff(&self, cutoff: &CollectedCutoff) -> Result<()> {
        ensure!(
            self.project_cutoff(&cutoff.original)? == *cutoff,
            "collector/cutoff projection mismatch"
        );
        Ok(())
    }

    /// Requires a fresh, caught-up Redis poll after the calendar cutoff, not just
    /// a maximum bar label. This certifies the local collector prefix only.
    pub fn seal_cutoff(&mut self, through_utc: i64) -> Result<CollectedCutoff> {
        self.healthy()?;
        let result = self
            .source
            .seal_cutoff(&mut self.journal, through_utc)
            .and_then(|b| self.project_cutoff(&b));
        result.map_err(|e| self.fail(e))
    }

    fn validate_pair(&self, pair: &[Store; 2]) -> Result<()> {
        self.healthy()?;
        let a = pair[0].identity();
        let mut b = pair[1].identity().clone();
        ensure!(
            a.policy == Policy::WeekdayOnly && b.policy == Policy::WeekendContextNoTrades,
            "pair order must be weekday control, weekend candidate"
        );
        b.policy = Policy::WeekdayOnly;
        ensure!(
            a == &b
                && a.symbol == self.journal.identity().symbol
                && a.receipt_stream == self.journal.identity().receipt_stream,
            "model/calendar/collector pair identity mismatch"
        );
        ensure!(
            pair[0].last_sequence() == pair[1].last_sequence(),
            "pair ingress is not aligned"
        );
        ensure!(
            pair[0].frozen.keys().eq(pair[1].frozen.keys()),
            "pair freeze dates are not aligned"
        );
        for (a, b) in pair[0].frozen.values().zip(pair[1].frozen.values()) {
            ensure!(
                a.barrier == b.barrier && a.frozen_at_utc == b.frozen_at_utc,
                "pair freeze provenance differs"
            );
        }
        let digest = hash(&self.prefix(pair[0].last_sequence())?)?;
        let attestations = self
            .journal
            .sealed_barriers()?
            .iter()
            .map(|b| self.project_cutoff(b).map(|c| c.projected))
            .collect::<Result<Vec<_>>>()?;
        for store in pair {
            ensure!(
                store.ingress_sha256()? == digest,
                "context diverges from original receipt prefix"
            );
            for f in store.frozen.values() {
                ensure!(
                    f.barrier.through_utc == store.identity().cutoff(f.session_date)
                        && attestations.contains(&f.barrier),
                    "context contains unattested freeze"
                );
            }
        }
        Ok(())
    }

    /// Both views advance or neither does. No caller-supplied receipt/durability
    /// timestamps and no reconstructed history can enter this path.
    pub fn replay_pair(&mut self, pair: &mut [Store; 2], through: u64) -> Result<()> {
        let result = (|| -> Result<[Store; 2]> {
            self.validate_pair(pair)?;
            let now = self.journal.now()?;
            ensure!(
                through >= pair[0].last_sequence(),
                "context sequence regression"
            );
            let inputs = self.prefix(through)?;
            let mut next = pair.clone();
            for store in &mut next {
                store.check_clock(now)?;
                for input in &inputs[store.last_sequence() as usize..] {
                    store.ingest(input.clone(), now)?;
                }
            }
            Ok(next)
        })();
        match result {
            Ok(next) => {
                *pair = next;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    /// One immutable file is the release boundary for BOTH frozen contexts.
    /// This is not a model-state checkpoint or a decision outbox.
    pub fn freeze_pair_new(
        &mut self,
        pair: &mut [Store; 2],
        day: NaiveDate,
        cutoff: &CollectedCutoff,
        path: &Path,
    ) -> Result<()> {
        let result = (|| -> Result<[Store; 2]> {
            self.validate_pair(pair)?;
            self.verify_cutoff(cutoff)?;
            ensure!(
                self.journal.sealed_barriers()?.last() == Some(&cutoff.original),
                "superseded cutoff"
            );
            ensure!(
                cutoff.projected.through_utc == pair[0].identity().cutoff(day),
                "wrong profile cutoff"
            );
            let now = self.journal.now()?;
            ensure!(
                local(now)?.date() == day,
                "cannot retroactively activate an old session"
            );
            let mut next = pair.clone();
            for store in &mut next {
                store.freeze(day, now, &cutoff.projected)?;
            }
            let payload = PairPayload {
                version: PAIR_VERSION.into(),
                cutoff: cutoff.clone(),
                session_date: day,
                checkpoints: [
                    String::from_utf8(next[0].checkpoint_bytes()?)?,
                    String::from_utf8(next[1].checkpoint_bytes()?)?,
                ],
            };
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            ensure!(
                !parent
                    .canonicalize()?
                    .starts_with(self.root.canonicalize()?),
                "paired checkpoint must be outside immutable ingress directory"
            );
            publish_pair(path, &payload)?;
            Ok(next)
        })();
        match result {
            Ok(next) => {
                *pair = next;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    pub fn restore_pair(&mut self, expected: &[Identity; 2], path: &Path) -> Result<[Store; 2]> {
        self.restore_pair_inner(expected, path)
            .map_err(|e| self.fail(e))
    }

    fn restore_pair_inner(&self, expected: &[Identity; 2], path: &Path) -> Result<[Store; 2]> {
        self.healthy()?;
        ensure!(
            fs::symlink_metadata(path)?.file_type().is_file(),
            "non-regular paired checkpoint"
        );
        let mut bytes = vec![];
        File::open(path)?
            .take(MAX_PAIR_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_PAIR_BYTES,
            "paired checkpoint size limit"
        );
        let saved: PairFile = serde_json::from_slice(&bytes)?;
        let p = saved.payload;
        ensure!(
            p.version == PAIR_VERSION && hash(&p)? == saved.sha256,
            "paired checkpoint checksum/version mismatch"
        );
        self.verify_cutoff(&p.cutoff)?;
        ensure!(
            self.journal.sealed_barriers()?.last() == Some(&p.cutoff.original),
            "older paired checkpoint; later cutoff requires explicit recovery"
        );
        let now = self.journal.now()?;
        let pair = [
            Store::restore(&expected[0], p.checkpoints[0].as_bytes(), now)?,
            Store::restore(&expected[1], p.checkpoints[1].as_bytes(), now)?,
        ];
        self.validate_pair(&pair)?;
        for store in &pair {
            ensure!(
                store.last_sequence() == p.cutoff.projected.last_sequence
                    && store.frozen.keys().next_back() == Some(&p.session_date)
                    && store
                        .frozen(p.session_date)
                        .is_some_and(|f| f.barrier == p.cutoff.projected),
                "incomplete or wrong paired freeze"
            );
        }
        Ok(pair)
    }
}

fn publish_pair(path: &Path, payload: &PairPayload) -> Result<()> {
    let bytes = serde_json::to_vec(&PairFile {
        sha256: hash(payload)?,
        payload: payload.clone(),
    })?;
    ensure!(
        bytes.len() <= MAX_PAIR_BYTES,
        "paired checkpoint size limit"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = parent.join(format!(".paired-context-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::hard_link(&tmp, path)?;
    File::open(parent)?.sync_all()?;
    fs::remove_file(tmp)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
