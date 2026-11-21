use super::*;
use crate::shadow_calendar::ArtifactPin;
use crate::{BarEvent, DataOrigin};
use sha2::{Digest, Sha256};
use std::cell::Cell;

thread_local! { static NOW: Cell<i64> = const { Cell::new(0) }; }
fn clock() -> i64 {
    NOW.with(Cell::get)
}
fn at(now: i64) {
    NOW.with(|v| v.set(now));
}
fn fixture_process(_: &ProcessPin) -> Result<()> {
    Ok(())
}
fn fixture() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/component_reaction_reference_v1.json"
    ))
    .unwrap()
}
fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn bar(row: &Value, origin: DataOrigin) -> BarEvent {
    BarEvent {
        symbol: "IMOEXF".into(),
        close_time_utc: row[0].as_i64().unwrap(),
        o: row[1].as_f64().unwrap(),
        h: row[2].as_f64().unwrap(),
        l: row[3].as_f64().unwrap(),
        close: row[4].as_f64().unwrap(),
        v: row[5].as_f64().unwrap(),
        origin,
    }
}
struct Rig {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    source: std::path::PathBuf,
    plan: Plan,
    cutoff: i64,
}
impl Rig {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let source = base.join("upstream");
        fs::create_dir(&source).unwrap();
        let f = fixture();
        let calendar_json = serde_json::to_string(&f["definition"]).unwrap();
        let cfg = json!({"symbol":"IMOEXF","stream":"fixture:component:bars","first_live_session":"2026-11-05","context_finalization":true,"late_context_history_policy":"audit_only_v1","ordinary_observation_max_lateness_seconds":180,"terminal":{"enabled":true,"symbols":["IMOEXF"],"max_heartbeat_age_seconds":60,"max_lateness_seconds":120}});
        let bytes = serde_json::to_vec(&json!({"version":"shadow_gateway_ws_closed_m10_v4","config_sha256":digest(&serde_json::to_vec(&cfg).unwrap()),"config":cfg})).unwrap();
        fs::write(source.join("manifest.json"), &bytes).unwrap();
        let spec = Spec {
            ingress: crate::ri_context::ingress::IngressIdentity {
                generation: "component-driver-fixture".into(),
                receipt_stream: "fixture.receipts.component-driver".into(),
                source_stream: "fixture:component:bars".into(),
                symbol: "IMOEXF".into(),
            },
            date: "2026-11-09".parse().unwrap(),
            calendar_pin: ArtifactPin {
                artifact_sha256: digest(calendar_json.as_bytes()),
                reference_sha256: f["definition"]["reference_sha256"].as_str().unwrap().into(),
                calendar_version: f["definition"]["calendar_version"].as_str().unwrap().into(),
                symbol: "IMOEXF".into(),
                first_date: "2026-11-05".parse().unwrap(),
                last_date: "2026-11-09".parse().unwrap(),
            },
            calendar_json,
            model_sha256: super::super::MODEL.into(),
            source_manifest_sha256: digest(&bytes),
            max_execution_lag_seconds: 300,
        };
        let cutoff = spec.calendar().unwrap().bounds_utc(spec.date).unwrap().0;
        let plan = Plan {
            source: SourcePin {
                root: source.clone(),
                manifest_sha256: spec.source_manifest_sha256.clone(),
                process: ProcessPin {
                    pid: std::process::id(),
                    linux_start_ticks: 1,
                    boot_id: "00000000-0000-0000-0000-000000000001".into(),
                },
                reviewed_price_audit_sha256: None,
                reviewed_late_context_sha256: None,
            },
            first_source_id: format!(
                "{}-0",
                f["histories"]["regular"][0][0].as_i64().unwrap() * 1000
            ),
            session: spec,
            ready_by_utc: cutoff - 600,
            max_poll_seconds: 10,
            max_queue_age_seconds: 120,
        };
        at(cutoff - 1200);
        let rig = Self {
            _temp: tmp,
            root: base.join("driver"),
            source,
            plan,
            cutoff,
        };
        rig.health(1, true);
        rig
    }
    fn health(&self, generation: u64, ready: bool) {
        let cfg: Value =
            serde_json::from_slice(&fs::read(self.source.join("manifest.json")).unwrap()).unwrap();
        fs::write(self.source.join("HEALTH.json"),serde_json::to_vec(&json!({"version":"shadow_gateway_ws_closed_m10_v4","config_sha256":cfg["config_sha256"],"order_emission_allowed":false,"generation":generation,"heartbeat_utc":clock(),"last_ws_rx_utc":clock(),"status":if ready {"ready"} else {"reconnecting"},"connected":ready,"subscribed":ready,"history_complete":ready,"queue_drained":ready})).unwrap()).unwrap();
    }
    fn create(&self) -> Driver {
        Driver::create_with_clock(&self.root, self.plan.clone(), clock, fixture_process).unwrap()
    }
    fn open(&self) -> Result<Driver> {
        Driver::open_with_clock(&self.root, self.plan.clone(), clock, fixture_process)
    }
}

#[test]
fn explicit_plan_preopen_create_and_incomplete_generation_are_not_repaired() {
    let rig = Rig::new();
    let driver = rig.create();
    assert!(
        driver.status().operational_admission_required && !driver.status().order_emission_allowed
    );
    assert!(!driver.status().clean_price_session && !driver.status().clean_oi_comparison_session);
    drop(driver);
    assert!(rig.open().is_ok());
    let mut changed = rig.plan.clone();
    changed.max_poll_seconds += 1;
    assert!(Driver::open_with_clock(&rig.root, changed, clock, fixture_process).is_err());
    assert!(
        Driver::create_with_clock(&rig.root, rig.plan.clone(), clock, fixture_process).is_err()
    );
    at(rig.plan.ready_by_utc);
    assert!(rig.open().is_err());
    assert!(rig.root.join("session/FAILED.json").exists());
}

#[test]
fn source_guard_recovers_generation_but_rejects_identity_or_pre_freeze_audit_drift() {
    let rig = Rig::new();
    let mut guard = SourceGuard::new(
        rig.plan.source.clone(),
        &rig.plan.session,
        120,
        fixture_process,
    )
    .unwrap();
    assert!(matches!(
        guard.check(clock(), true).unwrap(),
        SourceState::Ready
    ));
    rig.health(2, false);
    assert!(matches!(
        guard.check(clock(), true).unwrap(),
        SourceState::Recovering
    ));
    rig.health(2, true);
    assert!(matches!(
        guard.check(clock(), true).unwrap(),
        SourceState::Ready
    ));
    fs::write(rig.source.join("revisions.jsonl"), "{\"fixture\":true}\n").unwrap();
    assert!(guard.check(clock(), true).is_err());
    // A frozen as-observed session is not rerun for a later historical audit.
    assert!(matches!(
        guard.check(clock(), false).unwrap(),
        SourceState::Ready
    ));
    rig.health(1, true);
    assert!(guard.check(clock(), false).is_err());
}

#[test]
fn advance_without_checked_poll_and_expired_readiness_never_grant_or_emit() {
    let rig = Rig::new();
    let mut driver = rig.create();
    assert!(driver.advance(None).is_err());
    assert!(driver.status().session.failed && !driver.status().clean_price_session);
    assert!(driver.committed_output(1).is_err());
}

#[test]
fn source_manifest_budget_and_process_errors_fail_before_driver_creation() {
    let rig = Rig::new();
    let mut invalid = rig.plan.clone();
    invalid.source.manifest_sha256 = "d".repeat(64);
    assert!(Driver::create_with_clock(&rig.root, invalid, clock, fixture_process).is_err());
    fn replaced(_: &ProcessPin) -> Result<()> {
        anyhow::bail!("controlled process replacement")
    }
    assert!(Driver::create_with_clock(&rig.root, rig.plan.clone(), clock, replaced).is_err());
    assert!(!rig.root.exists());
    let mut invalid = rig.plan.clone();
    invalid.max_queue_age_seconds = 121;
    assert!(invalid.validate().is_err());
    let mut invalid = rig.plan.clone();
    invalid.first_source_id = "$".into();
    assert!(invalid.validate().is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_pin_checks_actual_boot_and_start_ticks_not_pid_only() {
    let pin = ProcessPin::inspect(std::process::id()).unwrap();
    source::check_process(&pin).unwrap();
    let mut changed = pin.clone();
    changed.linux_start_ticks += 1;
    assert!(source::check_process(&changed).is_err());
    let mut changed = pin;
    changed.boot_id = "00000000-0000-0000-0000-000000000000".into();
    assert!(source::check_process(&changed).is_err());
}

async fn add(admin: &mut MultiplexedConnection, b: BarEvent) {
    add_with_publication(admin, b, clock()).await;
}
async fn add_with_publication(admin: &mut MultiplexedConnection, b: BarEvent, published: i64) {
    let payload = json!({"schema_version":1,"source":"shadow_gateway_ws_closed_m10_v4","msg_type":"bar","ts_utc":published,"payload":b});
    let _: String = redis::cmd("XADD")
        .arg("fixture:component:bars")
        .arg(format!("{}-0", b.close_time_utc * 1000))
        .arg("payload")
        .arg(serde_json::to_string(&payload).unwrap())
        .query_async(admin)
        .await
        .unwrap();
}
async fn server() -> (
    crate::ri_context::ingress::redis_source::tests::TestServer,
    MultiplexedConnection,
    MultiplexedConnection,
) {
    let server = crate::ri_context::ingress::redis_source::tests::TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("componentreader")
        .arg("on")
        .arg(">fixture-only")
        .arg("~fixture:component:bars")
        .arg("+multi")
        .arg("+exec")
        .arg("+xrange")
        .arg("+ping")
        .query_async(&mut admin)
        .await
        .unwrap();
    let reader = redis::Client::open(format!(
        "redis://componentreader:fixture-only@127.0.0.1:{}/",
        server.port
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    (server, admin, reader)
}
async fn warmup(
    rig: &Rig,
    driver: &mut Driver,
    admin: &mut MultiplexedConnection,
    reader: &mut MultiplexedConnection,
) {
    for row in fixture()["histories"]["regular"].as_array().unwrap() {
        add(admin, bar(row, DataOrigin::History)).await;
    }
    for _ in 0..8 {
        driver.collect(reader).await.unwrap();
        assert!(driver.advance(None).unwrap().is_empty());
        if driver.status().armed_source_only {
            break;
        }
    }
    assert!(driver.status().armed_source_only && driver.status().source_caught_up);
    at(rig.cutoff);
    rig.health(1, true);
    driver.collect(reader).await.unwrap();
    let output = driver.advance(None).unwrap();
    assert_eq!(output.len(), 1);
    assert!(output[0].ready);
    assert!(!output[0].order_emission_allowed && output[0].operational_admission_required);
}

#[tokio::test]
#[ignore = "starts owned local redis-server; never REDIS_URL or a VPS"]
async fn readonly_acl_full_day_restart_and_stable_outbox_with_missing_oi() {
    let (_server, mut admin, mut reader) = server().await;
    let rig = Rig::new();
    let mut driver = rig.create();
    warmup(&rig, &mut driver, &mut admin, &mut reader).await;
    for command in [
        vec![
            "XADD",
            "fixture:component:bars",
            "*",
            "payload",
            "forbidden",
        ],
        vec!["XACK", "fixture:component:bars", "g", "1-0"],
        vec!["XTRIM", "fixture:component:bars", "MAXLEN", "0"],
        vec!["XRANGE", "other:stream", "-", "+"],
    ] {
        let mut c = redis::cmd(command[0]);
        for arg in &command[1..] {
            c.arg(*arg);
        }
        assert!(c.query_async::<_, redis::Value>(&mut reader).await.is_err());
    }
    let mut ids = std::collections::BTreeSet::new();
    let rows = fixture()["series"]["rounds"].as_array().unwrap().clone();
    for (i, row) in rows.iter().enumerate() {
        let b = bar(row, DataOrigin::Live);
        at(b.close_time_utc + 601);
        rig.health(1, true);
        add(&mut admin, b).await;
        driver.collect(&mut reader).await.unwrap();
        for output in driver.advance(None).unwrap() {
            for r in output.records {
                assert!(ids.insert(r.id));
            }
        }
        if [1, 15, 55, 100].contains(&i) {
            let n = driver.status().session.committed_transactions;
            let out = driver.committed_output(n).unwrap();
            drop(driver);
            driver = rig.open().unwrap();
            assert_eq!(driver.committed_output(n).unwrap(), out);
        }
    }
    let s = driver.status();
    assert!(s.session.complete && s.clean_price_session);
    assert!(
        !s.clean_oi_comparison_session
            && !s.order_emission_allowed
            && s.operational_admission_required
    );
    assert!(!ids.is_empty());
    let transactions = s.session.committed_transactions;
    for now in [clock() + 1, rig.cutoff + 86400] {
        at(now);
        assert!(driver.collect(&mut reader).await.unwrap().session.complete);
        assert!(driver.advance(None).unwrap().is_empty());
        assert!(driver.status().session.complete && !driver.status().session.failed);
        assert_eq!(driver.status().session.committed_transactions, transactions);
    }
}

#[tokio::test]
#[ignore = "starts owned local redis-server; never REDIS_URL or a VPS"]
async fn reconnect_same_process_resumes_but_an_earlier_mutated_prefix_fails() {
    let (_server, mut admin, mut reader) = server().await;
    let rig = Rig::new();
    let mut driver = rig.create();
    warmup(&rig, &mut driver, &mut admin, &mut reader).await;
    rig.health(2, false);
    assert_eq!(
        driver
            .collect(&mut reader)
            .await
            .unwrap()
            .waiting_reason
            .as_deref(),
        Some("upstream_recovering")
    );
    let before = driver.status().session.committed_transactions;
    rig.health(2, true);
    driver.collect(&mut reader).await.unwrap();
    assert!(driver.advance(None).unwrap().is_empty());
    assert_eq!(driver.status().session.committed_transactions, before);
    // Recreate identical IDs/tail but mutate an EARLY row, not the resume cursor.
    let _: usize = redis::cmd("DEL")
        .arg("fixture:component:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    for (i, row) in fixture()["histories"]["regular"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut b = bar(row, DataOrigin::History);
        if i == 1 {
            b.h += 0.5;
        }
        add(&mut admin, b).await;
    }
    assert!(driver.collect(&mut reader).await.is_err());
    assert!(driver.status().session.failed);
    assert_eq!(driver.status().session.committed_transactions, before);
}

#[tokio::test]
#[ignore = "starts owned local redis-server; never REDIS_URL or a VPS"]
async fn missing_bar_times_out_even_with_empty_healthy_stream() {
    let (_server, mut admin, mut reader) = server().await;
    let rig = Rig::new();
    let mut driver = rig.create();
    warmup(&rig, &mut driver, &mut admin, &mut reader).await;
    at(rig.cutoff + 900);
    rig.health(1, true);
    assert!(driver.collect(&mut reader).await.is_err());
    assert!(driver.status().session.failed && !driver.status().clean_price_session);
    assert_eq!(driver.status().session.committed_transactions, 1);
}

#[test]
fn collected_prefix_queue_expiry_cannot_arm_or_freeze() {
    let rig = Rig::new();
    let mut driver = rig.create();
    driver.collected = Some((0, clock() - i64::from(rig.plan.max_queue_age_seconds)));
    assert!(driver.advance(None).is_err());
    assert!(!driver.status().armed_source_only && driver.status().session.failed);
    assert!(driver.committed_output(1).is_err());
}

#[tokio::test]
#[ignore = "starts owned local redis-server; never REDIS_URL or a VPS"]
async fn envelope_only_mutation_is_detected_after_driver_restart() {
    let (_server, mut admin, mut reader) = server().await;
    let rig = Rig::new();
    let mut driver = rig.create();
    warmup(&rig, &mut driver, &mut admin, &mut reader).await;
    drop(driver);
    let mut driver = rig.open().unwrap();
    let _: usize = redis::cmd("DEL")
        .arg("fixture:component:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    // All bars and the last cursor are identical. Only an old envelope's
    // publication time changes; payload-only/cursor-only checks miss this.
    for (i, row) in fixture()["histories"]["regular"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        add_with_publication(
            &mut admin,
            bar(row, DataOrigin::History),
            rig.cutoff - 1200 + i64::from(i == 1),
        )
        .await;
    }
    assert!(driver.collect(&mut reader).await.is_err());
    assert!(driver.status().session.failed);
    assert_eq!(driver.status().session.committed_transactions, 1);
}

#[tokio::test]
#[ignore = "starts owned local redis-server; never REDIS_URL or a VPS"]
async fn preopen_readiness_survives_restart_without_granting_a_stale_poll() {
    let (_server, mut admin, mut reader) = server().await;
    let rig = Rig::new();
    let mut driver = rig.create();
    for row in fixture()["histories"]["regular"].as_array().unwrap() {
        add(&mut admin, bar(row, DataOrigin::History)).await;
    }
    for _ in 0..8 {
        driver.collect(&mut reader).await.unwrap();
        assert!(driver.advance(None).unwrap().is_empty());
        if driver.status().armed_source_only {
            break;
        }
    }
    assert!(rig.root.join("ARMED.json").is_file());
    drop(driver);
    at(rig.plan.ready_by_utc + 30);
    rig.health(2, true);
    let mut driver = rig.open().unwrap();
    assert!(driver.status().armed_source_only && !driver.status().source_ready);
    assert!(!driver.status().source_caught_up && !driver.status().session.ready);
    at(rig.cutoff);
    rig.health(2, true);
    driver.collect(&mut reader).await.unwrap();
    let output = driver.advance(None).unwrap();
    assert_eq!(output.len(), 1);
    assert!(output[0].ready && output[0].operational_admission_required);
    assert!(!output[0].order_emission_allowed);
    drop(driver);
    let path = rig.root.join("ARMED.json");
    let mut witness: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    witness["receipt_prefix_sha256"] = json!("0".repeat(64));
    fs::write(path, serde_json::to_vec(&witness).unwrap()).unwrap();
    assert!(rig.open().is_err());
}
