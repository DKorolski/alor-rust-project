use std::collections::{BTreeMap, VecDeque};

use super::*;
use crate::shadow_calendar::{Definition, Session, SessionKind, SCHEMA};
use io::{IssTransport, Response, SystemDisk};
use serde_json::json;

fn plan() -> Plan {
    let calendar = Calendar::new(Definition {
        schema_version: SCHEMA.into(),
        calendar_version: "synthetic-fixture-v1".into(),
        reference_sha256: "a".repeat(64),
        symbol: "IMOEXF".into(),
        first_date: "2026-11-12".parse().unwrap(),
        last_date: "2026-11-16".parse().unwrap(),
        sessions: (12..=16)
            .map(|n| {
                let dsvd = n == 14 || n == 15;
                Session {
                    date: format!("2026-11-{n}").parse().unwrap(),
                    exchange_trade_date: None,
                    kind: if dsvd {
                        SessionKind::Dsvd
                    } else {
                        SessionKind::Regular
                    },
                    context_open_minute: Some(if dsvd { 590 } else { 420 }),
                    close_minute: Some(if dsvd { 1140 } else { 1430 }),
                    main_open_minute: Some(600),
                    main_close_minute: Some(1140),
                    breaks: vec![],
                    note: "offline collector fixture, not PIT proof".into(),
                }
            })
            .collect(),
    })
    .unwrap();
    Plan::new(&calendar, "2026-11-16".parse().unwrap(), &"b".repeat(64)).unwrap()
}

struct FakeClock {
    utc: chrono::DateTime<chrono::Utc>,
    mono_ns: u64,
    reads: usize,
    slow: BTreeMap<usize, i64>,
    jumps: BTreeMap<usize, i64>,
    bad: Option<usize>,
    error: Option<usize>,
}
impl FakeClock {
    fn new() -> Self {
        Self {
            utc: plan().at(6, 50),
            mono_ns: 0,
            reads: 0,
            slow: BTreeMap::new(),
            jumps: BTreeMap::new(),
            bad: None,
            error: None,
        }
    }
}
impl Clock for FakeClock {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::OfflineFixture
    }
    fn read(&mut self) -> Result<ClockReading> {
        self.reads += 1;
        ensure!(
            self.error != Some(self.reads),
            "injected clock read failure"
        );
        let delay = 1 + self.slow.get(&self.reads).copied().unwrap_or(0);
        self.utc += chrono::Duration::milliseconds(
            delay + self.jumps.get(&self.reads).copied().unwrap_or(0),
        );
        self.mono_ns += u64::try_from(delay)? * 1_000_000;
        Ok(ClockReading {
            utc: self.utc,
            monotonic_ns: self.mono_ns,
            synchronized: self.bad != Some(self.reads),
            offset_ms: Some(0),
            max_error_ms: Some(1),
        })
    }
    async fn sleep(&mut self, duration: Duration) {
        self.utc += chrono::Duration::from_std(duration).unwrap();
        self.mono_ns += u64::try_from(duration.as_nanos()).unwrap();
    }
}

struct FakeTransport {
    replies: VecDeque<Response>,
    calls: usize,
}
impl FakeTransport {
    fn new(replies: Vec<Response>) -> Self {
        Self {
            replies: replies.into(),
            calls: 0,
        }
    }
}
impl Transport for FakeTransport {
    async fn get(&mut self, _: &Plan, timeout: Duration) -> Response {
        assert!(timeout <= Duration::from_secs(30));
        self.calls += 1;
        self.replies
            .pop_front()
            .expect("unexpected additional network request")
    }
}
struct FakeDisk {
    free_bytes: u64,
    bytes: u64,
}
impl Default for FakeDisk {
    fn default() -> Self {
        Self {
            free_bytes: 1_000_000_000,
            bytes: 0,
        }
    }
}
impl Disk for FakeDisk {
    fn usage(&mut self, _: &Path) -> Result<DiskUsage> {
        Ok(DiskUsage {
            bytes: self.bytes,
            files: 0,
            free_bytes: self.free_bytes,
        })
    }
}

fn response(prior: u64) -> Response {
    Response {
        status: Some(200),
        server_date: Some(plan().at(6, 50).to_rfc2822()),
        failure: None,
        body: serde_json::to_vec(&json!({"history": {
            "columns": ["BOARDID", "SECID", "TRADEDATE", "OPENPOSITION"],
            "data": [["RFUD", "IMOEXF", "2026-11-12", 100], ["RFUD", "IMOEXF", "2026-11-13", prior]]
        }}))
        .unwrap(),
    }
}
fn network_failure() -> Response {
    Response {
        status: None,
        server_date: None,
        body: vec![],
        failure: Some(FetchFailure::Network),
    }
}

#[tokio::test]
async fn complete_response_has_measured_raw_and_decision_durability() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let mut c = FakeClock::new();
    let mut t = FakeTransport::new(vec![response(90)]);
    let q = collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut c,
        &mut t,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::AcceptedSnapshot);
    assert_eq!(q.last_validation, Some(Reason::Decreasing));
    assert!(q.allow_new_bo);
    assert!(
        !q.allow_new_bo(),
        "offline evidence must not enable a production model"
    );
    assert_eq!(t.calls, 1);
    let data = Store::new(root.path().to_path_buf())
        .unwrap()
        .load(&p)
        .unwrap()
        .unwrap();
    let captured = data.capture_id.unwrap();
    let manifest = read_bounded(
        &root
            .path()
            .join("captures")
            .join(captured.to_string())
            .join("manifest.json"),
    )
    .unwrap();
    let capture: super::super::Capture = serde_json::from_slice(&manifest).unwrap();
    assert!(capture.receipt.durable_at_utc > capture.receipt.received_at_utc);
    assert!(q.data_durable_at.unwrap().utc > data.frozen_at_utc);
    assert!(
        load_qualified(root.path(), &p).is_err(),
        "fixture quarantine"
    );
}

#[tokio::test]
async fn zero_or_positive_oi_is_valid_but_disallows_bo() {
    for prior in [100, 110] {
        let root = tempfile::tempdir().unwrap();
        let q = collect(
            root.path(),
            &plan(),
            &Policy::default(),
            &mut FakeClock::new(),
            &mut FakeTransport::new(vec![response(prior)]),
            &mut FakeDisk::default(),
        )
        .await
        .unwrap();
        assert_eq!(q.status, Status::AcceptedSnapshot);
        assert_eq!(q.last_validation, Some(Reason::NonDecreasing));
        assert!(!q.allow_new_bo);
    }
}

#[tokio::test]
async fn temporary_http_failure_retries_and_preserves_exact_responses() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let mut failed = response(90);
    failed.status = Some(503);
    failed.body = b"ISS unavailable".to_vec();
    let mut t = FakeTransport::new(vec![failed, response(90)]);
    let q = collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut FakeClock::new(),
        &mut t,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::AcceptedSnapshot);
    assert_eq!(q.attempts_used, 2);
    assert_eq!(t.calls, 2);
    assert_eq!(
        std::fs::read(attempt_directory(root.path(), &p).join("1.body")).unwrap(),
        b"ISS unavailable"
    );
}

#[tokio::test]
async fn partial_even_valid_json_can_never_qualify() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let mut partial = response(90);
    partial.failure = Some(FetchFailure::IncompleteBody);
    let policy = Policy {
        max_attempts: 1,
        ..Policy::default()
    };
    let q = collect(
        root.path(),
        &p,
        &policy,
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![partial]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::SourceUnavailable);
    assert!(!q.allow_new_bo);
    let attempt: Attempt = serde_json::from_slice(
        &read_bounded(&attempt_directory(root.path(), &p).join("1.json")).unwrap(),
    )
    .unwrap();
    assert!(!attempt.response_complete);
    assert!(attempt.capture_id.is_none());
}

#[tokio::test]
async fn query_count_and_response_bytes_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let policy = Policy {
        max_attempts: 2,
        ..Policy::default()
    };
    let mut t = FakeTransport::new(vec![network_failure(), network_failure()]);
    let q = collect(
        root.path(),
        &p,
        &policy,
        &mut FakeClock::new(),
        &mut t,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::SourceUnavailable);
    assert_eq!(t.calls, 2);
    let mut no_calls = FakeTransport::new(vec![]);
    let same = collect(
        root.path(),
        &p,
        &policy,
        &mut FakeClock::new(),
        &mut no_calls,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(same.status, q.status);
    assert_eq!(no_calls.calls, 0);
}

#[tokio::test]
async fn reservation_survives_interruption_and_restart_does_not_reset_query_budget() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let policy = Policy {
        max_attempts: 2,
        ..Policy::default()
    };
    let mut c = FakeClock::new();
    c.error = Some(4); // Response arrived, but its receipt clock was not recorded.
    let mut t = FakeTransport::new(vec![response(90)]);
    assert!(collect(
        root.path(),
        &p,
        &policy,
        &mut c,
        &mut t,
        &mut FakeDisk::default()
    )
    .await
    .is_err());
    let mut resumed = FakeTransport::new(vec![response(90)]);
    let q = collect(
        root.path(),
        &p,
        &policy,
        &mut FakeClock::new(),
        &mut resumed,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.attempts_used, 2);
    assert_eq!(t.calls + resumed.calls, 2);
}

#[tokio::test]
async fn changed_budget_after_interrupted_attempt_is_not_silently_accepted() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let mut c = FakeClock::new();
    c.error = Some(4);
    assert!(collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut c,
        &mut FakeTransport::new(vec![response(90)]),
        &mut FakeDisk::default()
    )
    .await
    .is_err());
    let changed = Policy {
        max_attempts: 2,
        ..Policy::default()
    };
    assert!(collect(
        root.path(),
        &p,
        &changed,
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![]),
        &mut FakeDisk::default()
    )
    .await
    .is_err());
}

#[tokio::test]
async fn late_data_fsync_cannot_enable_bo_even_when_receipt_and_freeze_started_timely() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let mut c = FakeClock::new();
    c.utc = p.at(6, 54) + chrono::Duration::seconds(40);
    c.slow.insert(8, 20_000);
    let q = collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut c,
        &mut FakeTransport::new(vec![response(90)]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::LateCommit);
    assert!(!q.allow_new_bo);
    assert!(q.data_durable_at.unwrap().utc > p.at(6, 55));
}

#[tokio::test]
async fn clock_step_or_lost_sync_before_raw_or_data_commit_disables_bo() {
    for read in [4, 5, 7, 8] {
        let root = tempfile::tempdir().unwrap();
        let mut c = FakeClock::new();
        c.jumps.insert(read, 1000);
        let q = collect(
            root.path(),
            &plan(),
            &Policy::default(),
            &mut c,
            &mut FakeTransport::new(vec![response(90)]),
            &mut FakeDisk::default(),
        )
        .await
        .unwrap();
        assert_eq!(q.status, Status::InvalidClock, "read {read}");
        assert!(!q.allow_new_bo);
    }
}

#[tokio::test]
async fn unhealthy_clock_at_start_performs_no_request() {
    let root = tempfile::tempdir().unwrap();
    let mut c = FakeClock::new();
    c.bad = Some(1);
    let mut t = FakeTransport::new(vec![]);
    let q = collect(
        root.path(),
        &plan(),
        &Policy::default(),
        &mut c,
        &mut t,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::InvalidClock);
    assert_eq!(t.calls, 0);
}

#[tokio::test]
async fn premature_start_does_not_create_a_frozen_future_failure() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let mut c = FakeClock::new();
    c.utc = p.at(6, 44);
    assert!(collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut c,
        &mut FakeTransport::new(vec![]),
        &mut FakeDisk::default()
    )
    .await
    .is_err());
    assert!(!qualification_path(root.path(), &p).exists());
}

#[tokio::test]
async fn no_request_is_started_without_commit_time_remaining() {
    let root = tempfile::tempdir().unwrap();
    let mut c = FakeClock::new();
    c.utc = plan().at(6, 54) + chrono::Duration::seconds(59);
    let q = collect(
        root.path(),
        &plan(),
        &Policy::default(),
        &mut c,
        &mut FakeTransport::new(vec![]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::SourceUnavailable);
    assert_eq!(q.attempts_used, 0);
}

#[tokio::test]
async fn storage_pressure_disables_bo_without_queries_or_deleting_evidence() {
    for usage in [
        FakeDisk {
            free_bytes: 0,
            ..FakeDisk::default()
        },
        FakeDisk {
            bytes: 64 * 1024 * 1024,
            ..FakeDisk::default()
        },
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut usage = usage;
        let q = collect(
            root.path(),
            &plan(),
            &Policy::default(),
            &mut FakeClock::new(),
            &mut FakeTransport::new(vec![]),
            &mut usage,
        )
        .await
        .unwrap();
        assert_eq!(q.status, Status::BudgetExhausted);
        assert!(!q.allow_new_bo);
    }
}

#[tokio::test]
async fn restored_frozen_result_ignores_a_new_vintage_and_uses_no_http() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![response(90)]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    let before = std::fs::read(qualification_path(root.path(), &p)).unwrap();
    let mut late_clock = FakeClock::new();
    late_clock.utc = p.at(7, 30);
    let mut no_calls = FakeTransport::new(vec![]);
    let q = collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut late_clock,
        &mut no_calls,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::AcceptedSnapshot);
    assert_eq!(
        std::fs::read(qualification_path(root.path(), &p)).unwrap(),
        before
    );
    assert_eq!(no_calls.calls, 0);
}

#[tokio::test]
async fn orphan_offline_draft_after_crash_is_not_a_timing_attestation() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let r = Receipt {
        url: HISTORY_URL.into(),
        params: p.request_params(),
        http_status: 200,
        server_date: response(90).server_date,
        requested_at_utc: p.at(6, 50),
        received_at_utc: p.at(6, 50),
        durable_at_utc: p.at(6, 50),
        clock_offset_ms: Some(0),
    };
    let archived = store.archive(&p, r, &response(90).body).unwrap();
    store.freeze(&p, Some(&archived), p.at(6, 51)).unwrap();
    let q = collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::UnqualifiedExistingDecision);
    assert!(!q.allow_new_bo);
}

#[tokio::test]
async fn changed_response_or_qualification_identity_is_detected_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![response(90)]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    std::fs::write(
        attempt_directory(root.path(), &p).join("1.body"),
        b"changed",
    )
    .unwrap();
    assert!(load_inner(root.path(), &p, true).is_err());
    assert!(collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![]),
        &mut FakeDisk::default()
    )
    .await
    .is_err());
}

#[test]
fn lock_is_exclusive_and_released_after_process_scope() {
    let root = tempfile::tempdir().unwrap();
    let first = OwnedLock::acquire(root.path()).unwrap();
    assert!(OwnedLock::acquire(root.path()).is_err());
    drop(first);
    assert!(OwnedLock::acquire(root.path()).is_ok());
}

#[test]
fn fixed_iss_request_is_get_only_has_exact_dates_and_no_auth_headers() {
    let p = plan();
    let transport = IssTransport::new().unwrap();
    let request = transport.request(&p, Duration::from_secs(5)).unwrap();
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(request.url().scheme(), "https");
    assert_eq!(request.url().host_str(), Some("iss.moex.com"));
    let params: BTreeMap<String, String> = request.url().query_pairs().into_owned().collect();
    assert_eq!(params, p.request_params());
    assert!(request
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .is_none());
    assert!(request.headers().get(reqwest::header::COOKIE).is_none());
    assert!(transport.request(&p, Duration::ZERO).is_err());
    assert!(transport.request(&p, Duration::from_secs(31)).is_err());
}

#[test]
fn kernel_clock_samples_and_monotonic_boundaries_are_conservative() {
    let mut clock = FakeClock::new();
    let first = clock.read().unwrap();
    let mut next = clock.read().unwrap();
    assert!(next.follows(&first));
    next.max_error_ms = Some(1001);
    assert!(!next.follows(&first));
    next.max_error_ms = Some(1);
    next.utc += chrono::Duration::milliseconds(251);
    assert!(!next.follows(&first));
    next.offset_ms = Some(i64::MIN);
    assert!(!next.healthy());
}

#[test]
fn local_disk_scanner_is_bounded_and_rejects_symlink_evidence() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("raw"), b"abc").unwrap();
    let usage = SystemDisk.usage(root.path()).unwrap();
    assert_eq!(usage.bytes, 3);
    assert!(usage.free_bytes > 0);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path().join("raw"), root.path().join("alias")).unwrap();
        assert!(SystemDisk.usage(root.path()).is_err());
    }
}

#[test]
fn ntp_micro_nano_units_round_away_from_zero_and_invalid_health_is_not_fabricated() {
    assert_eq!(
        io::interpret_ntp(true, false, 1001, 1001),
        (true, Some(2), Some(2))
    );
    assert_eq!(
        io::interpret_ntp(true, true, -1_000_001, 1000),
        (true, Some(-2), Some(1))
    );
    assert_eq!(io::interpret_ntp(false, false, 0, 0), (false, None, None));
    assert_eq!(io::interpret_ntp(true, false, 0, -1), (false, None, None));
}

#[test]
fn real_clock_query_never_claims_supported_sync_on_non_linux() {
    let mut clock = io::SystemClock::default();
    let sample = clock.read().unwrap();
    if cfg!(target_os = "linux") {
        assert_eq!(clock.kind(), EvidenceKind::LinuxKernelClock);
    } else {
        assert_eq!(clock.kind(), EvidenceKind::UnsupportedClock);
        assert!(!sample.healthy());
        assert!(sample.offset_ms.is_none());
    }
}

#[tokio::test]
async fn outer_timeout_bounds_a_transport_that_never_completes() {
    struct Never;
    impl Transport for Never {
        async fn get(&mut self, _: &Plan, _: Duration) -> Response {
            std::future::pending().await
        }
    }
    let root = tempfile::tempdir().unwrap();
    let policy = Policy {
        max_attempts: 1,
        http_timeout_seconds: 1,
        ..Policy::default()
    };
    let start = std::time::Instant::now();
    let q = collect(
        root.path(),
        &plan(),
        &policy,
        &mut FakeClock::new(),
        &mut Never,
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert_eq!(q.status, Status::SourceUnavailable);
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn config_rejects_trading_keys_and_relative_paths_without_io() {
    let root = tempfile::tempdir().unwrap();
    let d: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/calendar_reference_v1.json"
    ))
    .unwrap();
    let fixture = &d["cases"][0];
    let mut config = json!({
        "calendar_path": root.path().join("not-read-by-config-load.json"),
        "calendar_pin": fixture["pin"], "decision_date": "2026-11-16",
        "model_sha256": "b".repeat(64), "store_root": root.path().join("store"),
        "policy": Policy::default()
    });
    let path = root.path().join("config.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    assert!(Config::load(&path).is_ok());
    config["refresh_token"] = json!("must-not-be-used");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    assert!(Config::load(&path).is_err());
    config.as_object_mut().unwrap().remove("refresh_token");
    config["store_root"] = json!("relative/path");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    assert!(Config::load(&path).is_err());
    assert!(!root.path().join("store").exists());
}

#[test]
fn deadline_includes_clock_uncertainty_not_just_displayed_wall_time() {
    let p = plan();
    let mut clock = FakeClock::new();
    let mut reading = clock.read().unwrap();
    reading.utc = p.at(6, 55) - chrono::Duration::microseconds(500);
    assert!(!reading.before_deadline(p.at(6, 55)));
    reading.utc = p.at(6, 55) - chrono::Duration::milliseconds(1);
    assert!(reading.before_deadline(p.at(6, 55)));
}

// A test-only kernel-shaped clock exercises the certificate verifier. It is NOT
// evidence about this host's synchronization or a real prospective capture.
struct KernelFixtureClock(FakeClock);
impl Clock for KernelFixtureClock {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::LinuxKernelClock
    }
    fn read(&mut self) -> Result<ClockReading> {
        self.0.read()
    }
    async fn sleep(&mut self, duration: Duration) {
        self.0.sleep(duration).await;
    }
}

#[tokio::test]
async fn frozen_qualification_bundle_survives_external_deletion_and_rejects_tamper() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut KernelFixtureClock(FakeClock::new()),
        &mut FakeTransport::new(vec![response(90)]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    let saved = snapshot::Snapshot::capture(root.path(), &p)
        .unwrap()
        .unwrap();
    assert!(saved.verify(&p).unwrap().allow_new_bo());
    std::fs::remove_file(root.path().join("qualified/2026-11-16.json")).unwrap();
    assert!(snapshot::Snapshot::capture(root.path(), &p)
        .unwrap()
        .is_none());
    let bytes = serde_json::to_vec(&saved).unwrap();
    let restored: snapshot::Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert!(restored.verify(&p).unwrap().allow_new_bo());
    let mut wrong = p.clone();
    wrong.model_sha256 = "c".repeat(64);
    assert!(restored.verify(&wrong).is_err());
    let mut value = serde_json::to_value(&saved).unwrap();
    value["files"]["attempts/2026-11-16/1.body"][0] = json!(32);
    let bad: snapshot::Snapshot = serde_json::from_value(value).unwrap();
    assert!(bad.verify(&p).is_err());
    let mut value = serde_json::to_value(&saved).unwrap();
    value["files"]["unbound.json"] = json!([0]);
    let bad: snapshot::Snapshot = serde_json::from_value(value).unwrap();
    assert!(bad.verify(&p).is_err());
}

#[tokio::test]
async fn offline_fixture_qualification_cannot_be_minted_into_an_observed_bundle() {
    let root = tempfile::tempdir().unwrap();
    let p = plan();
    collect(
        root.path(),
        &p,
        &Policy::default(),
        &mut FakeClock::new(),
        &mut FakeTransport::new(vec![response(90)]),
        &mut FakeDisk::default(),
    )
    .await
    .unwrap();
    assert!(snapshot::Snapshot::capture(root.path(), &p).is_err());
}
