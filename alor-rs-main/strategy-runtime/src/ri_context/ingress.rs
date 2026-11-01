//! Local, single-writer receipt journal for an explicitly selected shadow generation.
//!
//! Receipt means first observation INSIDE this serialized collector, not broker
//! arrival or gateway publication. Old Redis/history payloads received now are late.
//! This module is not a Redis consumer and never acknowledges upstream messages.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{hash, local, ContextInput, ReceiptBarrier, RiContextStore};
use crate::strategy_host::BarEvent;

pub mod redis_source;

const VERSION: &str = "ri_shadow_receipt_journal_v1";
const MAX_RECORDS: usize = 10_000;
const MAX_BARRIERS: usize = 128;
const MAX_RAW_BYTES: usize = 32 * 1024;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IngressIdentity {
    /// A fresh, operator-selected generation. Never infer the latest directory.
    pub generation: String,
    pub receipt_stream: String,
    pub source_stream: String,
    pub symbol: String,
}

impl IngressIdentity {
    fn validate(&self) -> Result<()> {
        for value in [
            &self.generation,
            &self.receipt_stream,
            &self.source_stream,
            &self.symbol,
        ] {
            ensure!(
                !value.trim().is_empty() && value.len() <= 512,
                "invalid ingress identity"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: String,
    identity: IngressIdentity,
    created_at_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReceipt {
    sequence: u64,
    source_id: String,
    raw_bar_json: String,
    received_at_utc: i64,
    previous_commit_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Commit {
    sequence: u64,
    raw_sha256: String,
    /// Sampled AFTER raw payload and its directory entry are fsynced.
    durable_at_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBarrier {
    barrier: ReceiptBarrier,
    last_commit_sha256: String,
}

#[derive(Serialize)]
struct IngressFault {
    source_id_sha256: String,
    raw_bytes: usize,
    raw_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checked<T> {
    payload: T,
    sha256: String,
}

#[derive(Debug, Clone)]
struct Row {
    raw: RawReceipt,
    commit: Commit,
}

impl Row {
    fn input(&self) -> Result<ContextInput> {
        let bar: BarEvent = serde_json::from_str(&self.raw.raw_bar_json)
            .context("invalid bar retained in ingress audit; context consumption blocked")?;
        Ok(ContextInput {
            sequence: self.raw.sequence,
            bar,
            received_at_utc: Some(self.raw.received_at_utc),
            durable_at_utc: Some(self.commit.durable_at_utc),
        })
    }
}

/// An immutable record was committed. Parsing may still fail, in which case no
/// cutoff barrier can be issued. The raw bytes remain available for investigation.
#[derive(Debug)]
pub struct AppendReceipt {
    pub sequence: u64,
    pub redelivery: bool,
    pub input: Result<ContextInput>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IngressUsage {
    pub records: usize,
    pub barriers: usize,
    pub committed_bytes: u64,
    pub max_records: usize,
    pub max_barriers: usize,
    pub max_bytes: u64,
    pub writer_failed: bool,
}

/// Bounded generation, exclusively locked for its lifetime. A torn tail is NOT
/// silently truncated. There is no skip/reset, broker client or Intent interface.
pub struct RiIngressJournal {
    root: PathBuf,
    _writer: File,
    manifest: Manifest,
    rows: Vec<Row>,
    by_source: BTreeMap<String, usize>,
    barriers: Vec<StoredBarrier>,
    total_bytes: u64,
    clock_floor: i64,
    poisoned: bool,
    clock: fn() -> i64,
}

fn wall_clock() -> i64 {
    Utc::now().timestamp()
}

impl RiIngressJournal {
    pub fn create(root: &Path, identity: IngressIdentity) -> Result<Self> {
        Self::create_with_clock(root, identity, wall_clock)
    }

    pub(crate) fn create_with_clock(
        root: &Path,
        identity: IngressIdentity,
        clock: fn() -> i64,
    ) -> Result<Self> {
        identity.validate()?;
        let now = clock();
        local(now)?;
        fs::create_dir(root).context("ingress generation must be a NEW directory")?;
        let parent = root
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()?;
        let writer = lock_writer(root)?;
        let manifest = Manifest {
            version: VERSION.into(),
            identity,
            created_at_utc: now,
        };
        let total_bytes = publish_new(root, "manifest.json", &manifest)?;
        Ok(Self {
            root: root.to_path_buf(),
            _writer: writer,
            manifest,
            rows: vec![],
            by_source: BTreeMap::new(),
            barriers: vec![],
            total_bytes,
            clock_floor: now,
            poisoned: false,
            clock,
        })
    }

    pub fn open(root: &Path, expected: &IngressIdentity) -> Result<Self> {
        Self::open_with_clock(root, expected, wall_clock)
    }

    pub(crate) fn open_with_clock(
        root: &Path,
        expected: &IngressIdentity,
        clock: fn() -> i64,
    ) -> Result<Self> {
        expected.validate()?;
        ensure!(
            fs::symlink_metadata(root)?.file_type().is_dir(),
            "invalid journal directory"
        );
        let writer = lock_writer(root)?;
        let mut names = BTreeMap::new();
        let mut total_bytes = 0_u64;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            ensure!(entry.file_type()?.is_file(), "non-regular ingress file");
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid filename"))?;
            let size = entry.metadata()?.len();
            ensure!(size <= MAX_FILE_BYTES as u64, "oversized ingress file");
            total_bytes = total_bytes
                .checked_add(size)
                .context("ingress size overflow")?;
            ensure!(total_bytes <= MAX_TOTAL_BYTES, "ingress generation full");
            names.insert(name, size);
            ensure!(
                names.len() <= MAX_RECORDS * 2 + MAX_BARRIERS + 2,
                "too many ingress files"
            );
        }
        names.remove("writer.lock");
        ensure!(
            names.remove("manifest.json").is_some(),
            "missing ingress manifest"
        );
        let manifest: Manifest = read_checked(root.join("manifest.json"))?;
        ensure!(
            manifest.version == VERSION && manifest.identity == *expected,
            "ingress identity mismatch"
        );
        let mut journal = Self {
            root: root.to_path_buf(),
            _writer: writer,
            clock_floor: manifest.created_at_utc,
            manifest,
            rows: vec![],
            by_source: BTreeMap::new(),
            barriers: vec![],
            total_bytes,
            poisoned: false,
            clock,
        };
        local(journal.clock_floor)?;
        for sequence in 1..=MAX_RECORDS as u64 {
            let raw_name = format!("{sequence:08}.raw.json");
            let commit_name = format!("{sequence:08}.commit.json");
            if !names.contains_key(&raw_name) && !names.contains_key(&commit_name) {
                break;
            }
            ensure!(
                names.remove(&raw_name).is_some() && names.remove(&commit_name).is_some(),
                "uncommitted ingress tail; explicit recovery required"
            );
            let raw: RawReceipt = read_checked(root.join(raw_name))?;
            let commit: Commit = read_checked(root.join(commit_name))?;
            validate_source(&raw.source_id, &raw.raw_bar_json)?;
            ensure!(
                raw.sequence == sequence && commit.sequence == sequence,
                "ingress sequence mismatch"
            );
            ensure!(
                commit.raw_sha256 == hash(&raw)?
                    && raw.previous_commit_sha256 == journal.last_commit_hash()?,
                "ingress chain mismatch"
            );
            local(raw.received_at_utc)?;
            local(commit.durable_at_utc)?;
            ensure!(
                raw.received_at_utc >= journal.clock_floor
                    && commit.durable_at_utc >= raw.received_at_utc,
                "ingress clock regression"
            );
            ensure!(
                !journal.by_source.contains_key(&raw.source_id),
                "duplicate committed source id"
            );
            journal.clock_floor = commit.durable_at_utc;
            journal
                .by_source
                .insert(raw.source_id.clone(), journal.rows.len());
            journal.rows.push(Row { raw, commit });
        }
        for ordinal in 1..=MAX_BARRIERS {
            let name = format!("barrier-{ordinal:04}.json");
            if names.remove(&name).is_none() {
                break;
            }
            let saved: StoredBarrier = read_checked(root.join(name))?;
            journal.validate_barrier(&saved)?;
            journal.clock_floor = journal.clock_floor.max(saved.barrier.attested_at_utc);
            journal.barriers.push(saved);
        }
        ensure!(
            names.is_empty(),
            "unknown, missing or incomplete ingress files; explicit recovery required"
        );
        let now = journal.now()?;
        journal.clock_floor = now;
        Ok(journal)
    }

    pub fn identity(&self) -> &IngressIdentity {
        &self.manifest.identity
    }

    pub fn last_sequence(&self) -> u64 {
        self.rows.len() as u64
    }

    /// Exact committed inputs for deterministic shadow recovery, never a fresh
    /// timestamped history download. Malformed records cannot be skipped.
    pub fn input_prefix(&self, sequence: u64) -> Result<Vec<ContextInput>> {
        self.now()?;
        ensure!(sequence <= self.last_sequence(), "missing ingress sequence");
        self.rows[..sequence as usize]
            .iter()
            .map(Row::input)
            .collect()
    }

    /// Read-only bridge for other shadow context policies. Original source IDs
    /// and collector timestamps stay attached; no disk-format conversion occurs.
    pub(crate) fn identified_input_prefix(
        &self,
        sequence: u64,
    ) -> Result<Vec<(String, ContextInput)>> {
        let inputs = self.input_prefix(sequence)?;
        Ok(self
            .rows
            .iter()
            .zip(inputs)
            .map(|(row, input)| (row.raw.source_id.clone(), input))
            .collect())
    }

    pub(crate) fn sealed_barriers(&self) -> Result<Vec<ReceiptBarrier>> {
        self.now()?;
        Ok(self.barriers.iter().map(|b| b.barrier.clone()).collect())
    }

    /// Last durably accepted source cursor, not proof of upstream completeness.
    pub fn last_source_id(&self) -> Option<&str> {
        self.rows.last().map(|row| row.raw.source_id.as_str())
    }

    pub(super) fn source_id_at(&self, sequence: u64) -> Result<Option<String>> {
        ensure!(sequence <= self.last_sequence(), "missing ingress sequence");
        Ok(sequence
            .checked_sub(1)
            .map(|n| self.rows[n as usize].raw.source_id.clone()))
    }

    pub(super) fn contains_barrier(&self, barrier: &ReceiptBarrier) -> bool {
        self.barriers.iter().any(|saved| &saved.barrier == barrier)
    }

    /// Failed-write temporary files are not counted in committed_bytes. A failed
    /// writer requires a directory audit, not automatic deletion of the tail.
    pub fn usage(&self) -> IngressUsage {
        IngressUsage {
            records: self.rows.len(),
            barriers: self.barriers.len(),
            committed_bytes: self.total_bytes,
            max_records: MAX_RECORDS,
            max_barriers: MAX_BARRIERS,
            max_bytes: MAX_TOTAL_BYTES,
            writer_failed: self.poisoned,
        }
    }

    pub(crate) fn now(&self) -> Result<i64> {
        ensure!(
            !self.poisoned,
            "ingress writer failed; reopen and audit before continuing"
        );
        let now = (self.clock)();
        local(now)?;
        ensure!(now >= self.clock_floor, "ingress clock regression");
        Ok(now)
    }

    fn commit_hash_at(&self, sequence: u64) -> Result<String> {
        ensure!(sequence <= self.last_sequence(), "missing ingress sequence");
        if sequence == 0 {
            hash(&self.manifest)
        } else {
            hash(&self.rows[sequence as usize - 1].commit)
        }
    }

    fn last_commit_hash(&self) -> Result<String> {
        self.commit_hash_at(self.last_sequence())
    }

    fn inputs_hash_through(&self, sequence: u64) -> Result<String> {
        ensure!(sequence <= self.last_sequence(), "missing ingress sequence");
        let digests = self.rows[..sequence as usize]
            .iter()
            .map(|row| Ok((row.raw.sequence, row.input()?.digest()?)))
            .collect::<Result<Vec<_>>>()?;
        hash(&digests)
    }

    /// Call at the serialized ingress boundary. No externally supplied receipt
    /// timestamp is accepted, including for replay/redelivery after restart.
    pub fn append(&mut self, source_id: &str, raw_bar_json: &str) -> Result<AppendReceipt> {
        ensure!(
            !self.poisoned,
            "ingress writer failed; explicit recovery required"
        );
        match self.append_inner(source_id, raw_bar_json) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                self.poisoned = true;
                // Do not let a failed observation disappear across a normal restart.
                // If even this small marker cannot be persisted, the supervisor must
                // keep the generation stopped and retain the unacknowledged source.
                let fault = IngressFault {
                    source_id_sha256: format!("{:x}", Sha256::digest(source_id.as_bytes())),
                    raw_bytes: raw_bar_json.len(),
                    raw_sha256: format!("{:x}", Sha256::digest(raw_bar_json.as_bytes())),
                };
                match publish_new(&self.root, "FAILED.json", &fault) {
                    Ok(_) => Err(error.context("ingress generation marked FAILED; explicit recovery required")),
                    Err(fault_error) => Err(error.context(format!("ingress fault marker could not be persisted: {fault_error}; supervisor must prevent restart"))),
                }
            }
        }
    }

    fn append_inner(&mut self, source_id: &str, raw_bar_json: &str) -> Result<AppendReceipt> {
        let received = self.now()?;
        validate_source(source_id, raw_bar_json)?;
        if let Some(index) = self.by_source.get(source_id) {
            let row = &self.rows[*index];
            ensure!(
                row.raw.raw_bar_json == raw_bar_json,
                "conflicting upstream redelivery"
            );
            self.clock_floor = received;
            return Ok(AppendReceipt {
                sequence: row.raw.sequence,
                redelivery: true,
                input: row.input(),
            });
        }
        ensure!(
            self.rows.len() < MAX_RECORDS,
            "ingress record limit; explicit rotation required"
        );
        // Reserve worst-case space before publishing either half of a record.
        ensure!(
            self.total_bytes + 2 * MAX_FILE_BYTES as u64 <= MAX_TOTAL_BYTES,
            "ingress byte limit; explicit rotation required"
        );
        let sequence = self.last_sequence() + 1;
        let raw = RawReceipt {
            sequence,
            source_id: source_id.into(),
            raw_bar_json: raw_bar_json.into(),
            received_at_utc: received,
            previous_commit_sha256: self.last_commit_hash()?,
        };
        let result = (|| -> Result<(Commit, u64, i64)> {
            let bytes = publish_new(&self.root, &format!("{sequence:08}.raw.json"), &raw)?;
            let durable = self.now()?;
            ensure!(
                durable >= received,
                "ingress clock regressed during payload sync"
            );
            let commit = Commit {
                sequence,
                raw_sha256: hash(&raw)?,
                durable_at_utc: durable,
            };
            let bytes =
                bytes + publish_new(&self.root, &format!("{sequence:08}.commit.json"), &commit)?;
            let completed = self.now()?;
            ensure!(
                completed >= durable,
                "ingress clock regressed during commit sync"
            );
            Ok((commit, bytes, completed))
        })();
        let (commit, bytes, completed) = match result {
            Ok(result) => result,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        let row = Row { raw, commit };
        let receipt = AppendReceipt {
            sequence,
            redelivery: false,
            input: row.input(),
        };
        self.by_source.insert(source_id.into(), self.rows.len());
        self.rows.push(row);
        self.total_bytes += bytes;
        self.clock_floor = completed;
        Ok(receipt)
    }

    fn validate_barrier(&self, saved: &StoredBarrier) -> Result<()> {
        let b = &saved.barrier;
        local(b.through_utc)?;
        local(b.attested_at_utc)?;
        ensure!(
            b.receipt_stream == self.identity().receipt_stream
                && b.last_sequence <= self.last_sequence(),
            "wrong ingress barrier identity/sequence"
        );
        ensure!(
            b.through_utc >= self.manifest.created_at_utc && b.attested_at_utc >= b.through_utc,
            "invalid ingress barrier clock"
        );
        ensure!(
            b.ingress_sha256 == self.inputs_hash_through(b.last_sequence)?
                && saved.last_commit_sha256 == self.commit_hash_at(b.last_sequence)?,
            "ingress barrier checksum mismatch"
        );
        if b.last_sequence > 0 {
            ensure!(
                self.rows[b.last_sequence as usize - 1]
                    .commit
                    .durable_at_utc
                    <= b.attested_at_utc,
                "barrier before included commit"
            );
        }
        if let Some(next) = self.rows.get(b.last_sequence as usize) {
            ensure!(
                next.raw.received_at_utc >= b.attested_at_utc,
                "backdated receipt behind sealed barrier"
            );
        }
        if let Some(previous) = self.barriers.last() {
            ensure!(
                b.through_utc > previous.barrier.through_utc
                    && b.attested_at_utc >= previous.barrier.attested_at_utc
                    && b.last_sequence >= previous.barrier.last_sequence,
                "ingress barrier regression"
            );
        }
        Ok(())
    }

    /// Attests ONLY this collector's serialized, durably recorded input prefix.
    /// It does NOT attest an empty Redis backlog or complete exchange history.
    pub fn seal_cutoff(&mut self, through_utc: i64) -> Result<ReceiptBarrier> {
        let now = self.now()?;
        local(through_utc)?;
        ensure!(
            through_utc <= now && through_utc >= self.manifest.created_at_utc,
            "cutoff outside collector lifetime"
        );
        if let Some(saved) = self
            .barriers
            .iter()
            .find(|b| b.barrier.through_utc == through_utc)
        {
            return Ok(saved.barrier.clone());
        }
        ensure!(
            self.barriers.len() < MAX_BARRIERS,
            "ingress barrier limit; explicit rotation required"
        );
        ensure!(
            self.total_bytes + MAX_FILE_BYTES as u64 <= MAX_TOTAL_BYTES,
            "ingress byte limit"
        );
        let saved = StoredBarrier {
            barrier: ReceiptBarrier {
                receipt_stream: self.identity().receipt_stream.clone(),
                through_utc,
                last_sequence: self.last_sequence(),
                ingress_sha256: self.inputs_hash_through(self.last_sequence())?,
                attested_at_utc: now,
            },
            last_commit_sha256: self.last_commit_hash()?,
        };
        self.validate_barrier(&saved)?;
        let bytes = match publish_new(
            &self.root,
            &format!("barrier-{:04}.json", self.barriers.len() + 1),
            &saved,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        self.clock_floor = now;
        self.total_bytes += bytes;
        self.barriers.push(saved.clone());
        Ok(saved.barrier)
    }

    /// Replay an exact prefix, with original receipt/durability, atomically into
    /// either paired context policy. Does not restore strategy positions/counters.
    pub fn replay_through(
        &self,
        store: &mut RiContextStore,
        sequence: u64,
        processed_at_utc: i64,
    ) -> Result<()> {
        let now = self.now()?;
        ensure!(
            processed_at_utc <= now,
            "future context processing timestamp"
        );
        ensure!(
            store.state.identity.receipt_stream == self.identity().receipt_stream
                && store.state.identity.symbol == self.identity().symbol,
            "context/collector identity mismatch"
        );
        ensure!(
            store.last_sequence() <= sequence && sequence <= self.last_sequence(),
            "invalid context replay prefix"
        );
        ensure!(
            store.ingress_sha256()? == self.inputs_hash_through(store.last_sequence())?,
            "context checkpoint diverges from ingress prefix"
        );
        let mut candidate = RiContextStore {
            state: store.state.clone(),
        };
        candidate.check_clock(processed_at_utc)?;
        for row in &self.rows[store.last_sequence() as usize..sequence as usize] {
            ensure!(
                row.commit.durable_at_utc <= processed_at_utc,
                "processing before durable receipt"
            );
            candidate.ingest(row.input()?, processed_at_utc)?;
        }
        *store = candidate;
        Ok(())
    }
}

fn validate_source(source_id: &str, raw: &str) -> Result<()> {
    ensure!(
        !source_id.trim().is_empty() && source_id.len() <= 512,
        "invalid ingress source id"
    );
    ensure!(raw.len() <= MAX_RAW_BYTES, "oversized ingress payload");
    Ok(())
}

fn lock_writer(root: &Path) -> Result<File> {
    let path = root.join("writer.lock");
    if path.try_exists()? {
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_file(),
            "invalid writer lock file"
        );
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .context("ingress writer already active or filesystem locking unsupported")?;
    Ok(file)
}

fn publish_new<T: Serialize>(root: &Path, name: &str, payload: &T) -> Result<u64> {
    let bytes = serde_json::to_vec(&Checked {
        sha256: hash(payload)?,
        payload,
    })?;
    ensure!(bytes.len() <= MAX_FILE_BYTES, "oversized ingress record");
    let temporary = root.join(format!(".ingress-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::hard_link(&temporary, root.join(name))?;
    File::open(root)?.sync_all()?;
    fs::remove_file(temporary)?;
    File::open(root)?.sync_all()?;
    Ok(bytes.len() as u64)
}

fn read_checked<T: serde::de::DeserializeOwned + Serialize>(path: PathBuf) -> Result<T> {
    ensure!(
        fs::symlink_metadata(&path)?.file_type().is_file(),
        "non-regular ingress file"
    );
    let mut bytes = vec![];
    File::open(path)?
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_FILE_BYTES, "oversized ingress file");
    let checked: Checked<T> = serde_json::from_slice(&bytes)?;
    ensure!(
        hash(&checked.payload)? == checked.sha256,
        "ingress file checksum mismatch"
    );
    Ok(checked.payload)
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod tests;
