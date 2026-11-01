use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use super::*;
use crate::shadow_reaction::{imoexf, usdrubf, Config, NumericalSession, Sleeve};

thread_local! {
    static NOW: Cell<i64> = const { Cell::new(0) };
    static TICKS: RefCell<VecDeque<i64>> = const { RefCell::new(VecDeque::new()) };
}
fn clock() -> i64 {
    TICKS
        .with(|v| v.borrow_mut().pop_front())
        .unwrap_or_else(|| NOW.with(Cell::get))
}
fn now(ts: i64) {
    NOW.with(|v| v.set(ts));
    TICKS.with(|v| v.borrow_mut().clear());
}
fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}
fn epoch(text: &str) -> i64 {
    (NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap() - Duration::hours(3))
        .and_utc()
        .timestamp()
}
fn cfg(symbol: &str) -> Config {
    if symbol == "USDRUBF" {
        return Config::Usdrubf {
            settings: usdrubf::Config {
                model_id: "fixture_pd7_h3_w4".into(),
                family: usdrubf::Family::PreviousDay,
                context_hour: 7,
                clock_hour: 7,
                mr_horizon: 3,
                bo_wait: 4,
                mr_start_delay: 0,
            },
        };
    }
    Config::Imoexf {
        sleeve: Sleeve::Combo,
        costs: imoexf::Costs {
            point_value_rub: 10.,
            tick_value_rub: 5.,
            rate_per_component_per_side: 0.,
            components: 2,
        },
    }
}
fn identity(symbol: &str) -> IngressIdentity {
    IngressIdentity {
        generation: "reaction-collector-test-v1".into(),
        source_stream: "fixture:bars".into(),
        receipt_stream: "fixture:receipts:reaction-v1".into(),
        symbol: symbol.into(),
    }
}
fn pair(symbol: &str) -> [Store; 2] {
    [Policy::WeekdayOnly, Policy::WeekendContextNoTrades].map(|policy| {
        Store::new(Identity {
            profile: if symbol == "IMOEXF" {
                Profile::ImoexfReaction07
            } else {
                Profile::UsdrubfPreviousDay07
            },
            symbol: symbol.into(),
            model_config_sha256: cfg(symbol).fingerprint().unwrap(),
            receipt_stream: identity(symbol).receipt_stream,
            policy,
            excluded_dates: BTreeSet::new(),
        })
        .unwrap()
    })
}
fn bar(symbol: &str, ts: i64, price: f64, origin: DataOrigin) -> BarEvent {
    BarEvent {
        symbol: symbol.into(),
        close_time_utc: ts,
        o: price,
        h: price + 2.,
        l: price - 2.,
        close: price,
        v: 1.,
        origin,
    }
}
fn create(symbol: &str) -> (tempfile::TempDir, Collector) {
    now(epoch("2026-10-22 06:00"));
    let dir = tempfile::tempdir().unwrap();
    let c =
        Collector::create_with_clock(&dir.path().join("ingress"), identity(symbol), clock).unwrap();
    (dir, c)
}
fn append(c: &mut Collector, b: &BarEvent) {
    c.journal
        .append(
            &format!("{}-0", b.close_time_utc),
            &serde_json::to_string(b).unwrap(),
        )
        .unwrap()
        .input
        .unwrap();
}
fn bars(symbol: &str, omit_sunday_close: bool) -> Vec<BarEvent> {
    let mut out = vec![];
    for (d, price) in [
        ("2026-10-22", 100.),
        ("2026-10-23", 100.),
        ("2026-10-24", 102.),
        ("2026-10-25", 103.),
    ] {
        let weekend = day(d).weekday().num_days_from_monday() >= 5;
        let first = epoch(&format!("{d} {}", if weekend { "09:50" } else { "07:00" }));
        let last = epoch(&format!("{d} {}", if weekend { "18:50" } else { "23:40" }));
        for ts in (first..=last).step_by(600) {
            if omit_sunday_close && d == "2026-10-25" && ts == last {
                continue;
            }
            out.push(bar(symbol, ts, price, DataOrigin::Live));
        }
    }
    out
}
fn warm(c: &mut Collector, omit_sunday_close: bool) {
    for b in bars(&c.journal.identity().symbol, omit_sunday_close) {
        now(b.close_time_utc + 600);
        append(c, &b);
    }
}
fn attest_fixture(c: &mut Collector) -> CollectedCutoff {
    // Unit fixture uses the real durable journal seal. Only the separate Redis
    // integration below certifies the public caught-up polling boundary.
    let raw = c.journal.seal_cutoff(epoch("2026-10-26 07:00")).unwrap();
    c.project_cutoff(&raw).unwrap()
}
fn replay(c: &mut Collector, p: &mut [Store; 2]) {
    c.replay_pair(p, c.journal.last_sequence()).unwrap();
}

#[test]
fn projected_cutoff_cache_keeps_original_proof_and_rebuilds_on_open() {
    let (dir, mut c) = create("IMOEXF");
    warm(&mut c, false);
    now(epoch("2026-10-26 07:00"));
    let token = attest_fixture(&mut c);
    assert_eq!(c.projected_cutoffs.lock().unwrap().len(), 1);
    assert_eq!(
        token.projected.ingress_sha256,
        hash(&c.prefix(token.original.last_sequence).unwrap()).unwrap()
    );
    now(epoch("2026-10-26 07:10"));
    append(
        &mut c,
        &bar("IMOEXF", epoch("2026-10-26 07:00"), 101., DataOrigin::Live),
    );
    assert_eq!(c.project_cutoff(&token.original).unwrap(), token);
    assert_eq!(c.projected_cutoffs.lock().unwrap().len(), 1);
    let mut forged = token.original.clone();
    forged.last_sequence += 1;
    assert!(c.project_cutoff(&forged).is_err());
    c.failed = true;
    assert!(c.project_cutoff(&token.original).is_err());
    c.failed = false;
    drop(c);
    let c =
        Collector::open_with_clock(&dir.path().join("ingress"), identity("IMOEXF"), clock).unwrap();
    assert!(c.projected_cutoffs.lock().unwrap().is_empty());
    assert_eq!(c.project_cutoff(&token.original).unwrap(), token);
    assert_eq!(c.projected_cutoffs.lock().unwrap().len(), 1);
    now(epoch("2026-10-25 07:00"));
    assert!(c.project_cutoff(&token.original).is_err());
}

#[test]
fn shared_collector_preserves_weekend_policy_difference_and_restores_original_sources() {
    for symbol in ["IMOEXF", "USDRUBF"] {
        let (dir, mut c) = create(symbol);
        warm(&mut c, false);
        now(epoch("2026-10-26 07:00"));
        let token = attest_fixture(&mut c);
        let mut p = pair(symbol);
        replay(&mut c, &mut p);
        let path = dir.path().join("pair.json");
        c.freeze_pair_new(&mut p, day("2026-10-26"), &token, &path)
            .unwrap();
        let original = p
            .each_ref()
            .map(|s| s.frozen(day("2026-10-26")).unwrap().clone());
        assert!(original.iter().all(Frozen::ready));
        assert_eq!(
            original[0].previous.as_ref().unwrap().session_date,
            day("2026-10-23")
        );
        assert_eq!(
            original[1].previous.as_ref().unwrap().session_date,
            day("2026-10-25")
        );
        assert_eq!(
            original[1].previous2.as_ref().unwrap().session_date,
            day("2026-10-24")
        );
        let inputs = c.prefix(c.journal.last_sequence()).unwrap();
        assert_eq!(
            p[0].ingress_sha256().unwrap(),
            p[1].ingress_sha256().unwrap()
        );
        assert!(!c.status().order_emission_allowed);
        let identities = p.each_ref().map(|s| s.identity().clone());
        drop(c);
        now(epoch("2026-10-26 07:10"));
        let mut c =
            Collector::open_with_clock(&dir.path().join("ingress"), identity(symbol), clock)
                .unwrap();
        let restored = c.restore_pair(&identities, &path).unwrap();
        assert_eq!(c.prefix(c.journal.last_sequence()).unwrap(), inputs);
        assert_eq!(
            restored
                .each_ref()
                .map(|s| s.frozen(day("2026-10-26")).unwrap().clone()),
            original
        );
        assert!(
            !c.status().caught_up,
            "a restart cannot inherit Redis freshness"
        );
    }
}

#[test]
fn collected_context_can_seed_numerical_session_but_never_accepts_live_execution_directly() {
    let (dir, mut c) = create("IMOEXF");
    warm(&mut c, false);
    now(epoch("2026-10-26 07:00"));
    let token = attest_fixture(&mut c);
    let mut p = pair("IMOEXF");
    replay(&mut c, &mut p);
    c.freeze_pair_new(
        &mut p,
        day("2026-10-26"),
        &token,
        &dir.path().join("pair.json"),
    )
    .unwrap();
    for store in &p {
        let mut session = NumericalSession::new(store, day("2026-10-26"), cfg("IMOEXF")).unwrap();
        let b = bar("IMOEXF", epoch("2026-10-26 07:00"), 100.1, DataOrigin::Live);
        assert!(session.observe(b.clone(), b.close_time_utc + 600).is_err());
        let mut numerical = b.clone();
        numerical.origin = DataOrigin::Replay;
        session.observe(numerical, b.close_time_utc + 600).unwrap();
        assert_eq!(session.state().unwrap()["order_emission_allowed"], false);
        assert!(session.complete().is_err());
    }
}

#[test]
fn fresh_late_history_does_not_become_a_precutoff_receipt_even_with_delayed_first_event() {
    let (dir, mut c) = create("IMOEXF");
    warm(&mut c, true);
    now(epoch("2026-10-26 07:05"));
    let b = bar(
        "IMOEXF",
        epoch("2026-10-25 18:50"),
        103.,
        DataOrigin::HistoryGap,
    );
    append(&mut c, &b);
    let input = c.prefix(c.journal.last_sequence()).unwrap().pop().unwrap();
    assert_eq!(input.received_at_utc, Some(clock()));
    now(epoch("2026-10-26 07:10"));
    let token = attest_fixture(&mut c);
    let mut p = pair("IMOEXF");
    replay(&mut c, &mut p);
    assert!(matches!(
        p[1].operations.last().unwrap(),
        Operation::Ingest {
            route: Route::Quarantined(Reject::AtOrAfterCutoff),
            ..
        }
    ));
    c.freeze_pair_new(
        &mut p,
        day("2026-10-26"),
        &token,
        &dir.path().join("pair.json"),
    )
    .unwrap();
    assert_eq!(
        p[1].frozen(day("2026-10-26"))
            .unwrap()
            .previous
            .as_ref()
            .unwrap()
            .session_date,
        day("2026-10-24")
    );
}

#[test]
fn durable_boundary_crossing_cutoff_is_quarantined() {
    let (_dir, mut c) = create("IMOEXF");
    let cut = epoch("2026-10-26 07:00");
    now(cut + 1);
    TICKS.with(|v| *v.borrow_mut() = VecDeque::from([cut - 1, cut, cut + 1]));
    append(
        &mut c,
        &bar(
            "IMOEXF",
            epoch("2026-10-25 18:50"),
            103.,
            DataOrigin::HistoryGap,
        ),
    );
    let input = c.prefix(1).unwrap().pop().unwrap();
    assert_eq!(input.received_at_utc, Some(cut - 1));
    assert_eq!(input.durable_at_utc, Some(cut));
    let mut p = pair("IMOEXF");
    replay(&mut c, &mut p);
    assert!(matches!(
        p[1].operations.last().unwrap(),
        Operation::Ingest {
            route: Route::Quarantined(Reject::AtOrAfterCutoff),
            ..
        }
    ));
}

#[test]
fn history_received_after_pair_freeze_cannot_rewrite_the_anchors_on_recovery() {
    let (dir, mut c) = create("IMOEXF");
    warm(&mut c, true);
    now(epoch("2026-10-26 07:00"));
    let cutoff = attest_fixture(&mut c);
    let mut p = pair("IMOEXF");
    replay(&mut c, &mut p);
    let path = dir.path().join("pair.json");
    c.freeze_pair_new(&mut p, day("2026-10-26"), &cutoff, &path)
        .unwrap();
    let frozen = p
        .each_ref()
        .map(|s| s.frozen(day("2026-10-26")).unwrap().clone());
    now(epoch("2026-10-26 07:10"));
    append(
        &mut c,
        &bar(
            "IMOEXF",
            epoch("2026-10-25 18:50"),
            103.,
            DataOrigin::HistoryGap,
        ),
    );
    replay(&mut c, &mut p);
    assert_eq!(
        p.each_ref()
            .map(|s| s.frozen(day("2026-10-26")).unwrap().clone()),
        frozen
    );
    let identities = p.each_ref().map(|s| s.identity().clone());
    drop(c);
    let mut c =
        Collector::open_with_clock(&dir.path().join("ingress"), identity("IMOEXF"), clock).unwrap();
    let mut restored = c.restore_pair(&identities, &path).unwrap();
    replay(&mut c, &mut restored);
    assert_eq!(
        restored
            .each_ref()
            .map(|s| s.frozen(day("2026-10-26")).unwrap().clone()),
        frozen
    );
    assert!(matches!(
        restored[1].operations.last().unwrap(),
        Operation::Ingest {
            route: Route::Quarantined(Reject::AtOrAfterCutoff),
            ..
        }
    ));
}

#[test]
fn second_arm_replay_failure_does_not_partially_advance_the_control() {
    let (_dir, mut c) = create("IMOEXF");
    now(epoch("2026-10-22 07:10"));
    append(
        &mut c,
        &bar("IMOEXF", epoch("2026-10-22 07:00"), 100., DataOrigin::Live),
    );
    let mut p = pair("IMOEXF");
    p[1].clock = Some(clock() + 600);
    let before = p.each_ref().map(|s| s.checkpoint_bytes().unwrap());
    assert!(c.replay_pair(&mut p, 1).is_err());
    assert_eq!(p.each_ref().map(|s| s.checkpoint_bytes().unwrap()), before);
    assert_eq!(p[0].last_sequence(), 0);
    assert!(c.status().failed);
}

#[test]
fn paired_write_failure_does_not_release_one_arm_and_blocks_normal_reopen() {
    let (dir, mut c) = create("IMOEXF");
    now(epoch("2026-10-26 07:00"));
    let token = attest_fixture(&mut c);
    let mut p = pair("IMOEXF");
    let before = p.each_ref().map(|s| s.checkpoint_bytes().unwrap());
    let path = dir.path().join("existing.json");
    fs::write(&path, b"preserve-me").unwrap();
    assert!(c
        .freeze_pair_new(&mut p, day("2026-10-26"), &token, &path)
        .is_err());
    assert_eq!(p.each_ref().map(|s| s.checkpoint_bytes().unwrap()), before);
    assert_eq!(fs::read(path).unwrap(), b"preserve-me");
    assert!(c.status().failed);
    drop(c);
    assert!(
        Collector::open_with_clock(&dir.path().join("ingress"), identity("IMOEXF"), clock).is_err()
    );
}

#[test]
fn forged_projection_and_mismatched_pair_fail_before_mutation() {
    for wrong_pair in [false, true] {
        let (_dir, mut c) = create("IMOEXF");
        now(epoch("2026-10-26 07:00"));
        let mut token = attest_fixture(&mut c);
        let mut p = pair("IMOEXF");
        if wrong_pair {
            p[1].identity.excluded_dates.insert(day("2026-10-30"));
        } else {
            token.projected.ingress_sha256 = "0".repeat(64);
        }
        let before = p.each_ref().map(|s| s.checkpoint_bytes().unwrap());
        let dir = tempfile::tempdir().unwrap();
        assert!(c
            .freeze_pair_new(
                &mut p,
                day("2026-10-26"),
                &token,
                &dir.path().join("pair.json")
            )
            .is_err());
        assert_eq!(p.each_ref().map(|s| s.checkpoint_bytes().unwrap()), before);
        assert!(!dir.path().join("pair.json").exists());
    }
}

#[test]
fn context_cannot_replace_original_receipt_metadata() {
    let (_dir, mut c) = create("IMOEXF");
    now(epoch("2026-10-26 07:05"));
    append(
        &mut c,
        &bar(
            "IMOEXF",
            epoch("2026-10-25 18:50"),
            103.,
            DataOrigin::HistoryGap,
        ),
    );
    let mut p = pair("IMOEXF");
    let mut forged = c.prefix(1).unwrap().pop().unwrap();
    forged.received_at_utc = Some(epoch("2026-10-25 19:00"));
    forged.durable_at_utc = forged.received_at_utc;
    for store in &mut p {
        store.ingest(forged.clone(), clock()).unwrap();
    }
    let before = p.each_ref().map(|s| s.checkpoint_bytes().unwrap());
    assert!(c.replay_pair(&mut p, 1).is_err());
    assert_eq!(p.each_ref().map(|s| s.checkpoint_bytes().unwrap()), before);
    assert!(c.status().failed);
}

#[test]
fn rehashed_bad_pair_and_changed_generation_cannot_restore_or_fall_back_flat() {
    for mutation in ["generation", "checkpoint", "truncated"] {
        let (dir, mut c) = create("IMOEXF");
        now(epoch("2026-10-26 07:00"));
        let token = attest_fixture(&mut c);
        let mut p = pair("IMOEXF");
        let path = dir.path().join("pair.json");
        c.freeze_pair_new(&mut p, day("2026-10-26"), &token, &path)
            .unwrap();
        let identities = p.each_ref().map(|s| s.identity().clone());
        let mut saved: PairFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match mutation {
            "generation" => saved.payload.cutoff.collector.generation.push_str("-other"),
            "checkpoint" => saved.payload.checkpoints[1] = "{}".into(),
            _ => (),
        }
        saved.sha256 = hash(&saved.payload).unwrap();
        let mut bytes = serde_json::to_vec(&saved).unwrap();
        if mutation == "truncated" {
            bytes.truncate(bytes.len() / 2);
        }
        fs::write(&path, bytes).unwrap();
        assert!(c.restore_pair(&identities, &path).is_err());
        assert!(c.status().failed);
        assert!(c.replay_pair(&mut pair("IMOEXF"), 0).is_err());
    }
}

#[test]
fn earlier_paired_checkpoint_is_not_implicitly_selected_after_later_cutoff() {
    let (dir, mut c) = create("IMOEXF");
    now(epoch("2026-10-26 07:00"));
    let token = attest_fixture(&mut c);
    let mut p = pair("IMOEXF");
    let path = dir.path().join("pair.json");
    c.freeze_pair_new(&mut p, day("2026-10-26"), &token, &path)
        .unwrap();
    now(epoch("2026-10-27 07:00"));
    c.journal.seal_cutoff(clock()).unwrap();
    assert!(c
        .restore_pair(&p.each_ref().map(|s| s.identity().clone()), &path)
        .is_err());
    assert!(c.status().failed);
}

#[test]
fn writer_lock_redelivery_and_unsupported_contract_are_not_bypassed() {
    let (dir, mut c) = create("IMOEXF");
    assert!(
        Collector::open_with_clock(&dir.path().join("ingress"), identity("IMOEXF"), clock).is_err()
    );
    assert!(Collector::create_with_clock(&dir.path().join("ri"), identity("RIZ6"), clock).is_err());
    now(epoch("2026-10-22 07:10"));
    let b = bar("IMOEXF", epoch("2026-10-22 07:00"), 100., DataOrigin::Live);
    append(&mut c, &b);
    let original = c.prefix(1).unwrap();
    now(epoch("2026-10-22 08:00"));
    append(&mut c, &b);
    assert_eq!(c.prefix(1).unwrap(), original);
    assert_eq!(c.journal.last_sequence(), 1);
    let mut changed = b;
    changed.close = 101.;
    assert!(c
        .journal
        .append(
            &format!("{}-0", changed.close_time_utc),
            &serde_json::to_string(&changed).unwrap()
        )
        .is_err());
    assert!(c.status().failed);
}

#[test]
fn no_cutoff_without_caught_up_redis_poll_and_no_checkpoint_inside_ingress() {
    let (_dir, mut c) = create("IMOEXF");
    now(epoch("2026-10-26 07:00"));
    assert!(c.seal_cutoff(clock()).is_err());
    assert!(c.status().failed);
    let (_dir, mut c) = create("IMOEXF");
    now(epoch("2026-10-26 07:00"));
    let token = attest_fixture(&mut c);
    let path = c.root.join("not-an-ingress-record.json");
    assert!(c
        .freeze_pair_new(&mut pair("IMOEXF"), day("2026-10-26"), &token, &path)
        .is_err());
    assert!(!path.exists());
}

#[tokio::test]
#[ignore = "isolated local redis-server/ACL test; never REDIS_URL or VPS"]
async fn reaction_collector_readonly_redis_pair_integration() {
    use crate::ri_context::ingress::redis_source::tests::TestServer;
    use alor_protocol::{Envelope, MessageType};

    let server = TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("reactionreader")
        .arg("on")
        .arg(">local-test-only")
        .arg("~fixture:*")
        .arg("+multi")
        .arg("+exec")
        .arg("+xrange")
        .arg("+ping")
        .query_async(&mut admin)
        .await
        .unwrap();
    let mut reader = redis::Client::open(format!(
        "redis://reactionreader:local-test-only@127.0.0.1:{}/",
        server.port
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    let (dir, mut c) = create("IMOEXF");
    let inputs = bars("IMOEXF", false);
    let mut p = pair("IMOEXF");
    for group in inputs.chunk_by(|a, b| {
        local(a.close_time_utc).unwrap().date() == local(b.close_time_utc).unwrap().date()
    }) {
        for b in group {
            let _: String = redis::cmd("XADD")
                .arg("fixture:bars")
                .arg(format!("{}-0", b.close_time_utc))
                .arg("payload")
                .arg(
                    serde_json::to_string(&Envelope::new(1, "test-gateway", MessageType::Bar, b))
                        .unwrap(),
                )
                .query_async(&mut admin)
                .await
                .unwrap();
        }
        now(group.last().unwrap().close_time_utc + 600);
        assert!(c.poll_once(&mut reader).await.unwrap().caught_up);
        replay(&mut c, &mut p);
    }
    now(epoch("2026-10-26 07:00"));
    c.poll_once(&mut reader).await.unwrap();
    let token = c.seal_cutoff(clock()).unwrap();
    let path = dir.path().join("pair.json");
    c.freeze_pair_new(&mut p, day("2026-10-26"), &token, &path)
        .unwrap();
    assert_eq!(
        p[1].frozen(day("2026-10-26"))
            .unwrap()
            .previous
            .as_ref()
            .unwrap()
            .session_date,
        day("2026-10-25")
    );
    let prefixes = c.prefix(c.journal.last_sequence()).unwrap();
    let identities = p.each_ref().map(|s| s.identity().clone());
    drop(c);
    let mut c =
        Collector::open_with_clock(&dir.path().join("ingress"), identity("IMOEXF"), clock).unwrap();
    c.restore_pair(&identities, &path).unwrap();
    assert!(!c.status().caught_up);
    c.poll_once(&mut reader).await.unwrap();
    assert_eq!(c.seal_cutoff(clock()).unwrap(), token);
    assert_eq!(c.prefix(c.journal.last_sequence()).unwrap(), prefixes);
    let denied: redis::RedisResult<String> = redis::cmd("XADD")
        .arg("fixture:commands")
        .arg("*")
        .arg("payload")
        .arg("forbidden")
        .query_async(&mut reader)
        .await;
    assert!(denied.is_err());
    let length: usize = redis::cmd("XLEN")
        .arg("fixture:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    let commands: usize = redis::cmd("EXISTS")
        .arg("fixture:commands")
        .query_async(&mut admin)
        .await
        .unwrap();
    let groups: Vec<redis::Value> = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("fixture:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert_eq!(length, inputs.len());
    assert_eq!(commands, 0);
    assert!(groups.is_empty());
    let cursor = c.journal.last_source_id().unwrap().to_owned();
    let _: usize = redis::cmd("XDEL")
        .arg("fixture:bars")
        .arg(cursor)
        .query_async(&mut admin)
        .await
        .unwrap();
    assert!(c.poll_once(&mut reader).await.is_err());
    assert!(c.status().failed);
    drop(c);
    assert!(
        Collector::open_with_clock(&dir.path().join("ingress"), identity("IMOEXF"), clock).is_err()
    );
}
