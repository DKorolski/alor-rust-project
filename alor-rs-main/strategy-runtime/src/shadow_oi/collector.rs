//! Manually invoked, feature-gated ISS collector. This is not a strategy worker.
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{
    digest, publish_once, read_bounded, ArchivedCapture, DailyDecision, Plan, Reason, Receipt,
    Store, HISTORY_URL, MAX_BYTES,
};
use crate::shadow_calendar::{ArtifactPin, Calendar};

pub mod diagnostic;
pub mod io;
pub mod snapshot;
use io::{Clock, ClockReading, Disk, DiskUsage, EvidenceKind, FetchFailure, Transport};

const COMMIT_RESERVE_SECONDS: i64 = 10;
const ATTEMPT_RESERVE_BYTES: u64 = 3 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub max_attempts: u8,
    pub http_timeout_seconds: u8,
    pub retry_delay_seconds: u8,
    pub max_store_bytes: u64,
    pub max_store_files: u64,
    pub min_free_bytes: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            http_timeout_seconds: 30,
            retry_delay_seconds: 30,
            max_store_bytes: 64 * 1024 * 1024,
            max_store_files: 4096,
            min_free_bytes: 256 * 1024 * 1024,
        }
    }
}

impl Policy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=6).contains(&self.max_attempts)
                && (1..=30).contains(&self.http_timeout_seconds)
                && (1..=60).contains(&self.retry_delay_seconds)
                && (ATTEMPT_RESERVE_BYTES..=256 * 1024 * 1024).contains(&self.max_store_bytes)
                && (32..=4096).contains(&self.max_store_files)
                && self.min_free_bytes >= 16 * 1024 * 1024,
            "collector budget bounds"
        );
        Ok(())
    }
    fn permits(&self, usage: &DiskUsage) -> bool {
        usage
            .bytes
            .checked_add(ATTEMPT_RESERVE_BYTES)
            .is_some_and(|n| n <= self.max_store_bytes)
            && usage
                .files
                .checked_add(12)
                .is_some_and(|n| n <= self.max_store_files)
            && usage.free_bytes >= self.min_free_bytes.saturating_add(ATTEMPT_RESERVE_BYTES)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub calendar_path: PathBuf,
    pub calendar_pin: ArtifactPin,
    pub decision_date: NaiveDate,
    pub model_sha256: String,
    pub store_root: PathBuf,
    pub policy: Policy,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&read_bounded(path)?)?;
        config.policy.validate()?;
        ensure!(
            config.calendar_path.is_absolute() && config.store_root.is_absolute(),
            "absolute collector paths required"
        );
        ensure!(
            config
                .store_root
                .components()
                .all(|p| !matches!(p, std::path::Component::ParentDir)),
            "store parent traversal"
        );
        Ok(config)
    }
    pub fn plan(&self) -> Result<Plan> {
        Plan::new(
            &Calendar::load(&self.calendar_path, &self.calendar_pin)?,
            self.decision_date,
            &self.model_sha256,
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    AcceptedSnapshot,
    SourceUnavailable,
    InvalidClock,
    OutsideWindow,
    LateCommit,
    BudgetExhausted,
    UnqualifiedExistingDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    plan: Plan,
    policy: Policy,
    reserved_at: ClockReading,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    plan: Plan,
    number: u8,
    url: String,
    params: std::collections::BTreeMap<String, String>,
    http_status: Option<u16>,
    server_date: Option<String>,
    response_complete: bool,
    failure: Option<FetchFailure>,
    response_sha256: String,
    response_bytes: usize,
    validation: Option<Reason>,
    capture_id: Option<uuid::Uuid>,
    clocks: Vec<ClockReading>,
    clocks_msk: Vec<String>,
}

/// Audit certificate of the already-fsynced data decision. It is not a new OI
/// vintage. Consumers must use load_qualified(), never the offline Store alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qualification {
    schema_version: String,
    plan: Plan,
    evidence_kind: EvidenceKind,
    pub status: Status,
    allow_new_bo: bool,
    decision_sha256: Option<String>,
    selected_attempt: Option<u8>,
    selected_attempt_sha256: Option<String>,
    freeze_started_at: Option<ClockReading>,
    data_durable_at: Option<ClockReading>,
    pub attempts_used: u8,
    pub last_validation: Option<Reason>,
    pub disk: Option<DiskUsage>,
}

impl Qualification {
    /// An offline fixture can exercise calculations but cannot authorize a model.
    pub fn allow_new_bo(&self) -> bool {
        self.evidence_kind == EvidenceKind::LinuxKernelClock && self.allow_new_bo
    }
}

fn qualification_path(root: &Path, plan: &Plan) -> PathBuf {
    root.join("qualified")
        .join(format!("{}.json", plan.decision_date))
}
fn decision_path(root: &Path, plan: &Plan) -> PathBuf {
    root.join("decisions")
        .join(format!("{}.json", plan.decision_date))
}
fn attempt_directory(root: &Path, plan: &Plan) -> PathBuf {
    root.join("attempts").join(plan.decision_date.to_string())
}

fn load_inner(root: &Path, plan: &Plan, fixture: bool) -> Result<Option<Qualification>> {
    load_with_reader(root, plan, fixture, &read_bounded)
}

fn load_with_reader(
    root: &Path,
    plan: &Plan,
    fixture: bool,
    read: &impl Fn(&Path) -> Result<Vec<u8>>,
) -> Result<Option<Qualification>> {
    let path = qualification_path(root, plan);
    let bytes = match read(&path) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let q: Qualification = serde_json::from_slice(&bytes)?;
    ensure!(
        q.schema_version == "imoexf_oi_durable_qualification_v1" && q.plan == *plan,
        "qualification identity mismatch"
    );
    ensure!(
        fixture || q.evidence_kind == EvidenceKind::LinuxKernelClock,
        "non-production OI evidence"
    );
    if let Some(expected) = &q.decision_sha256 {
        ensure!(
            digest(&read(&decision_path(root, plan))?) == *expected,
            "qualified decision changed"
        );
    } else {
        ensure!(
            !q.allow_new_bo && q.status != Status::AcceptedSnapshot,
            "unbound enabled decision"
        );
    }
    if q.status == Status::AcceptedSnapshot {
        let decision = Store {
            root: root.to_path_buf(),
        }
        .load_with_reader(plan, read)?
        .context("qualified decision absent")?;
        let durable = q
            .data_durable_at
            .as_ref()
            .context("missing data durability clock")?;
        let freeze = q
            .freeze_started_at
            .as_ref()
            .context("missing freeze clock")?;
        ensure!(
            durable.before_deadline(plan.at(6, 55)),
            "late or uncertain decision durability"
        );
        let n = q
            .selected_attempt
            .context("missing selected HTTP attempt")?;
        ensure!((1..=6).contains(&n), "attempt bound");
        let directory = attempt_directory(root, plan);
        let bytes = read(&directory.join(format!("{n}.json")))?;
        ensure!(
            Some(digest(&bytes)) == q.selected_attempt_sha256,
            "selected attempt changed"
        );
        let a: Attempt = serde_json::from_slice(&bytes)?;
        ensure!(
            a.plan == *plan
                && a.number == n
                && a.url == HISTORY_URL
                && a.params == plan.request_params()
                && a.failure.is_none()
                && a.response_complete
                && a.capture_id == decision.capture_id
                && a.clocks.len() == 4
                && a.clocks_msk
                    == a.clocks
                        .iter()
                        .map(|c| super::msk(c.utc))
                        .collect::<Vec<_>>()
                && a.clocks.windows(2).all(|p| p[1].follows(&p[0]))
                && freeze.follows(a.clocks.last().unwrap())
                && durable.follows(freeze)
                && freeze.utc == decision.frozen_at_utc,
            "selected receipt/clock chain mismatch"
        );
        let raw = read(&directory.join(format!("{n}.body")))?;
        ensure!(
            raw.len() == a.response_bytes && digest(&raw) == a.response_sha256,
            "HTTP response changed"
        );
        let capture = decision.capture_id.context("missing source capture")?;
        let stored_raw = read(
            &root
                .join("captures")
                .join(capture.to_string())
                .join("history.json"),
        )?;
        ensure!(
            raw == stored_raw && q.allow_new_bo == decision.allow_new_bo(),
            "qualified gate mismatch"
        );
        ensure!(
            matches!(
                decision.reason(),
                Reason::Decreasing | Reason::NonDecreasing
            ),
            "invalid accepted snapshot"
        );
    } else {
        ensure!(!q.allow_new_bo, "failed qualification enabled BO");
    }
    Ok(Some(q))
}

/// Missing, corrupt or unqualified evidence must be mapped to BO-disabled by
/// the future model adapter, never to a whole-engine pause or a late re-enable.
pub fn load_qualified(root: &Path, plan: &Plan) -> Result<Option<Qualification>> {
    load_inner(root, plan, false)
}

struct OwnedLock(std::fs::File);
impl OwnedLock {
    fn acquire(root: &Path) -> Result<Self> {
        let path = root.join("collector.lock");
        if path.exists() {
            ensure!(
                !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
                "symlink lock"
            );
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: valid owned descriptor; this lock never adjusts system state.
            ensure!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
                "another OI collector owns this store"
            );
        }
        #[cfg(not(unix))]
        anyhow::bail!("collector requires Unix locking");
        Ok(Self(file))
    }
}
impl Drop for OwnedLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: descriptor remains owned until this guard is dropped.
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

fn prepare_root(root: &Path) -> Result<()> {
    ensure!(root.is_absolute(), "absolute store root required");
    std::fs::create_dir_all(root)?;
    ensure!(
        !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
        "symlink store root"
    );
    for name in ["qualified", "attempts"] {
        std::fs::create_dir_all(root.join(name))?;
    }
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

fn used_attempts(root: &Path, plan: &Plan, policy: &Policy) -> Result<u8> {
    let directory = attempt_directory(root, plan);
    std::fs::create_dir_all(&directory)?;
    let mut used = 0;
    for n in 1..=6 {
        let path = directory.join(format!("{n}.reserved.json"));
        if !path.exists() {
            continue;
        }
        let reservation: Reservation = serde_json::from_slice(&read_bounded(&path)?)?;
        ensure!(
            reservation.plan == *plan && reservation.policy == *policy,
            "attempt identity/budget changed across restart"
        );
        ensure!(n <= policy.max_attempts, "attempt budget already consumed");
        used = n;
    }
    Ok(used)
}

fn finish(root: &Path, q: Qualification) -> Result<Qualification> {
    publish_once(&qualification_path(root, &q.plan), &serde_json::to_vec(&q)?)?;
    // Certificate logs an earlier observed data fsync, not its own publication
    // timestamp. No caller may substitute certificate time for data durability.
    load_inner(
        root,
        &q.plan,
        q.evidence_kind == EvidenceKind::OfflineFixture,
    )?
    .context("qualification missing after publication")
}

fn failed(
    plan: &Plan,
    kind: EvidenceKind,
    status: Status,
    used: u8,
    validation: Option<Reason>,
    disk: Option<DiskUsage>,
) -> Qualification {
    Qualification {
        schema_version: "imoexf_oi_durable_qualification_v1".into(),
        plan: plan.clone(),
        evidence_kind: kind,
        status,
        allow_new_bo: false,
        decision_sha256: None,
        selected_attempt: None,
        selected_attempt_sha256: None,
        data_durable_at: None,
        freeze_started_at: None,
        attempts_used: used,
        last_validation: validation,
        disk,
    }
}

/// Only public ISS data is requested. Persistence errors are fatal: absence of
/// a qualification is a closed BO gate, not permission to use an offline draft.
pub async fn collect<C: Clock, T: Transport, D: Disk>(
    root: &Path,
    plan: &Plan,
    policy: &Policy,
    clock: &mut C,
    transport: &mut T,
    disk: &mut D,
) -> Result<Qualification> {
    policy.validate()?;
    prepare_root(root)?;
    let _owner = OwnedLock::acquire(root)?;
    if let Some(existing) = load_inner(root, plan, clock.kind() == EvidenceKind::OfflineFixture)? {
        return Ok(existing);
    }
    let store = Store::new(root.to_path_buf())?;
    let mut used = used_attempts(root, plan, policy)?;
    if store.load(plan)?.is_some() {
        return finish(
            root,
            failed(
                plan,
                clock.kind(),
                Status::UnqualifiedExistingDecision,
                used,
                None,
                None,
            ),
        );
    }
    let mut previous = clock.read()?;
    if previous.utc < plan.at(6, 45) || previous.utc > plan.at(6, 55) {
        // A premature invocation must not freeze a future day as failed.
        anyhow::bail!("outside dated 06:45..06:55 MSK capture window");
    }
    if !previous.healthy() {
        return finish(
            root,
            failed(plan, clock.kind(), Status::InvalidClock, used, None, None),
        );
    }
    let mut last_validation = None;
    let mut last_disk = None;
    while used < policy.max_attempts {
        let now = clock.read()?;
        if !now.follows(&previous) {
            return finish(
                root,
                failed(
                    plan,
                    clock.kind(),
                    Status::InvalidClock,
                    used,
                    last_validation,
                    last_disk,
                ),
            );
        }
        let remaining =
            (plan.at(6, 55) - now.utc).num_milliseconds() - COMMIT_RESERVE_SECONDS * 1000;
        if remaining < 1000 {
            previous = now;
            break;
        }
        let usage = disk.usage(root)?;
        if !policy.permits(&usage) {
            return finish(
                root,
                failed(
                    plan,
                    clock.kind(),
                    Status::BudgetExhausted,
                    used,
                    last_validation,
                    Some(usage),
                ),
            );
        }
        last_disk = Some(usage);
        used += 1;
        let directory = attempt_directory(root, plan);
        publish_once(
            &directory.join(format!("{used}.reserved.json")),
            &serde_json::to_vec(&Reservation {
                plan: plan.clone(),
                policy: policy.clone(),
                reserved_at: now.clone(),
            })?,
        )?;
        std::fs::File::open(root.join("attempts"))?.sync_all()?;
        let start = clock.read()?;
        if !start.follows(&now) {
            return finish(
                root,
                failed(
                    plan,
                    clock.kind(),
                    Status::InvalidClock,
                    used,
                    last_validation,
                    last_disk,
                ),
            );
        }
        if start.utc >= plan.at(6, 55) - chrono::Duration::seconds(COMMIT_RESERVE_SECONDS) {
            return finish(
                root,
                failed(
                    plan,
                    clock.kind(),
                    Status::LateCommit,
                    used,
                    last_validation,
                    last_disk,
                ),
            );
        }
        let remaining =
            (plan.at(6, 55) - start.utc).num_milliseconds() - COMMIT_RESERVE_SECONDS * 1000;
        let timeout = Duration::from_millis(
            u64::try_from(remaining)?.min(u64::from(policy.http_timeout_seconds) * 1000),
        );
        let response = match tokio::time::timeout(timeout, transport.get(plan, timeout)).await {
            Ok(response) => response,
            Err(_) => io::Response {
                status: None,
                server_date: None,
                body: vec![],
                failure: Some(FetchFailure::Timeout),
            },
        };
        ensure!(
            response.body.len() <= MAX_BYTES,
            "transport violated body budget"
        );
        let received = clock.read()?;
        publish_once(&directory.join(format!("{used}.body")), &response.body)?;
        let complete = response.failure.is_none() && response.status.is_some();
        let mut clocks = vec![start.clone(), received.clone()];
        let mut captured: Option<ArchivedCapture> = None;
        if complete {
            let receipt = Receipt {
                url: HISTORY_URL.into(),
                params: plan.request_params(),
                http_status: response.status.unwrap(),
                server_date: response.server_date.clone(),
                requested_at_utc: start.utc,
                received_at_utc: received.utc,
                durable_at_utc: received.utc,
                clock_offset_ms: if received.follows(&start) {
                    received.offset_ms
                } else {
                    None
                },
            };
            let archived = store.archive_measured(plan, receipt, &response.body, || {
                let durable = clock.read()?;
                let utc = durable.utc;
                clocks.push(durable);
                Ok(utc)
            })?;
            last_validation = Some(archived.capture.evaluation.reason);
            captured = Some(archived);
        }
        let after_archive = clock.read()?;
        clocks.push(after_archive.clone());
        let a = Attempt {
            plan: plan.clone(),
            number: used,
            url: HISTORY_URL.into(),
            params: plan.request_params(),
            http_status: response.status,
            server_date: response.server_date,
            response_complete: complete,
            failure: response.failure,
            response_sha256: digest(&response.body),
            response_bytes: response.body.len(),
            validation: captured.as_ref().map(|c| c.capture.evaluation.reason),
            capture_id: captured.as_ref().map(|c| c.id),
            clocks: clocks.clone(),
            clocks_msk: clocks.iter().map(|c| super::msk(c.utc)).collect(),
        };
        let attempt_bytes = serde_json::to_vec(&a)?;
        publish_once(&directory.join(format!("{used}.json")), &attempt_bytes)?;
        if !clocks.windows(2).all(|p| p[1].follows(&p[0])) {
            return finish(
                root,
                failed(
                    plan,
                    clock.kind(),
                    Status::InvalidClock,
                    used,
                    last_validation,
                    last_disk,
                ),
            );
        }
        if let Some(capture) = &captured {
            if capture.capture.evaluation.valid_snapshot() {
                let freeze = clock.read()?;
                if !freeze.follows(&after_archive) {
                    return finish(
                        root,
                        failed(
                            plan,
                            clock.kind(),
                            Status::InvalidClock,
                            used,
                            last_validation,
                            last_disk,
                        ),
                    );
                }
                let decision: DailyDecision = store.freeze(plan, Some(capture), freeze.utc)?;
                let durable = clock.read()?;
                let status = if !durable.follows(&freeze) {
                    Status::InvalidClock
                } else if !durable.before_deadline(plan.at(6, 55)) {
                    Status::LateCommit
                } else {
                    Status::AcceptedSnapshot
                };
                let mut q = failed(plan, clock.kind(), status, used, last_validation, last_disk);
                q.allow_new_bo = status == Status::AcceptedSnapshot && decision.allow_new_bo();
                q.decision_sha256 = Some(digest(&read_bounded(&decision_path(root, plan))?));
                q.selected_attempt = Some(used);
                q.selected_attempt_sha256 = Some(digest(&attempt_bytes));
                q.freeze_started_at = Some(freeze);
                q.data_durable_at = Some(durable);
                return finish(root, q);
            }
        }
        previous = after_archive;
        let remaining = (plan.at(6, 55) - previous.utc).num_milliseconds().max(0);
        let sleep = u64::try_from(remaining)?.min(u64::from(policy.retry_delay_seconds) * 1000);
        clock.sleep(Duration::from_millis(sleep)).await;
    }
    let now = clock.read()?;
    if !now.follows(&previous) {
        return finish(
            root,
            failed(
                plan,
                clock.kind(),
                Status::InvalidClock,
                used,
                last_validation,
                last_disk,
            ),
        );
    }
    // Queries are exhausted; wait without querying or enabling the model later.
    let wait = (plan.at(6, 55) - now.utc).num_milliseconds().max(0);
    clock
        .sleep(Duration::from_millis(u64::try_from(wait)?))
        .await;
    let final_clock = clock.read()?;
    let status = if final_clock.follows(&now) {
        Status::SourceUnavailable
    } else {
        Status::InvalidClock
    };
    finish(
        root,
        failed(plan, clock.kind(), status, used, last_validation, last_disk),
    )
}

#[cfg(test)]
mod tests;
