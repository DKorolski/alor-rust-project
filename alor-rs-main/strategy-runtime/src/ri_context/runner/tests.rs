use std::cell::Cell;

use super::*;
use crate::strategies::ri_author41_42_live::{
    RiAuthor4142ExecutionPath, RiAuthor4142ModelFeedPolicy, RiAuthor4142RuntimeMode,
};
use crate::strategy_host::BarEvent;

mod benchmark;

thread_local! { static CLOCK: Cell<i64> = const { Cell::new(0) }; }
fn clock() -> i64 {
    CLOCK.with(Cell::get)
}
fn set_clock(now: i64) {
    CLOCK.with(|c| c.set(now));
}
fn epoch(raw: &str) -> i64 {
    (chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M").unwrap()
        - chrono::Duration::hours(3))
    .and_utc()
    .timestamp()
}
fn date(raw: &str) -> NaiveDate {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
}
fn spec() -> RunnerSpec {
    RunnerSpec {
        generation: ShadowGeneration {
            id: "paired-test-v1".into(),
            first_decision_session: date("2026-10-13"),
        },
        source_stream: "fixture:bars".into(),
        receipt_stream: "fixture:receipts".into(),
        strategy: RiAuthor4142LiveConfig {
            symbol: "RIZ6".into(),
            profile_id: "ri_author41_42_primary_combo_cost2".into(),
            timeframe: "10m".into(),
            mode: RiAuthor4142RuntimeMode::ProspectiveShadow,
            model_feed_policy: Some(RiAuthor4142ModelFeedPolicy::MicroRegularV1),
            allow_order_emission: false,
            execution_path: RiAuthor4142ExecutionPath::ActionScopedOnly,
            order_symbol: None,
            session_start_time: "07:00:00".into(),
            session_end_time: "23:49:59".into(),
            author41_entry_end_time: "10:00:00".into(),
            author41_time_exit: "20:00:00".into(),
            author42_exit_time: "23:00:00".into(),
            author42_last_entry_time: None,
            author42_max_entries_per_day: None,
            excluded_model_dates: vec![],
            min_anchor_bars: 92,
            anchor_first_bar_at_or_before: "07:10:00".into(),
            anchor_last_bar_at_or_after: "23:30:00".into(),
            anchor_transition_date: Some("2026-07-14".into()),
            pre_transition_min_anchor_bars: Some(80),
            pre_transition_anchor_first_bar_at_or_before: Some("09:10:00".into()),
            pre_transition_anchor_last_bar_at_or_after: Some("23:30:00".into()),
            actual_expiry_date: None,
            roll_target_sessions_before: 1,
            roll_fallback_sessions_before: 2,
            qty: 1.,
            timezone_offset_hours: 3,
        },
    }
}

fn bar(label: i64, origin: DataOrigin, close: f64) -> BarEvent {
    BarEvent {
        symbol: "RIZ6".into(),
        close_time_utc: label,
        o: close,
        h: close + 10.,
        l: close - 10.,
        close,
        v: 10.,
        origin,
    }
}
fn append(runner: &mut RiShadowRunner, bar: BarEvent, received: i64) {
    set_clock(received);
    let id = format!("{}-0", runner.journal.last_sequence() + 1);
    runner
        .journal
        .append(&id, &serde_json::to_string(&bar).unwrap())
        .unwrap();
}
fn ingest(runner: &mut RiShadowRunner) {
    runner
        .commit(
            Operation::Ingest {
                through: runner.journal.last_sequence(),
            },
            clock(),
        )
        .unwrap();
}
fn observe(runner: &mut RiShadowRunner) {
    while runner.state.observed < runner.journal.last_sequence() {
        runner
            .commit(
                Operation::Observe {
                    through: (runner.state.observed + OBSERVATION_BATCH)
                        .min(runner.journal.last_sequence()),
                },
                clock(),
            )
            .unwrap();
    }
}
fn fixture() -> (tempfile::TempDir, RiShadowRunner) {
    let dir = tempfile::tempdir().unwrap();
    set_clock(epoch("2026-10-09 06:00"));
    let mut runner =
        RiShadowRunner::create_with_clock(&dir.path().join("runner"), spec(), clock).unwrap();
    for day in ["2026-10-09", "2026-10-12"] {
        for label in (epoch(&format!("{day} 07:00"))..=epoch(&format!("{day} 23:30"))).step_by(600)
        {
            let mut b = bar(label, DataOrigin::History, 90000.);
            b.h = 91000.;
            b.l = 89000.;
            append(&mut runner, b, label + 600);
        }
        ingest(&mut runner);
        observe(&mut runner);
    }
    let mut gap = bar(epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.);
    gap.h = 91000.;
    gap.l = 89000.;
    append(&mut runner, gap, epoch("2026-10-13 03:00"));
    ingest(&mut runner);
    observe(&mut runner);
    set_clock(epoch("2026-10-13 07:00"));
    let barrier = runner.journal.seal_cutoff(clock()).unwrap();
    runner
        .commit(
            Operation::Freeze {
                date: date("2026-10-13"),
                barrier,
            },
            clock(),
        )
        .unwrap();
    (dir, runner)
}
fn enter(runner: &mut RiShadowRunner) {
    append(
        runner,
        bar(epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
        epoch("2026-10-13 07:10"),
    );
    ingest(runner);
    observe(runner);
}

#[test]
fn paired_runner_restores_open_mr_and_distinct_frozen_context_without_republication() {
    let (dir, mut r) = fixture();
    enter(&mut r);
    let state: Vec<_> = r.state.adapters.iter().map(|a| a.state_trace()).collect();
    assert!(!state[0]["mr_position"].is_null());
    assert_eq!(state[0]["mr_entries_today_total"], 1);
    assert_eq!(state[0]["frozen_context"]["previous"]["close"], 90000.);
    assert_eq!(state[1]["frozen_context"]["previous"]["close"], 90500.);
    let count = r.log.count();
    let output = r.committed_output(count).unwrap();
    assert_eq!(output.steps.len(), 1);
    assert!(r.committed_output(count + 1).is_err());
    drop(r);
    let mut reopened =
        RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).unwrap();
    assert_eq!(reopened.log.count(), count);
    assert_eq!(reopened.committed_output(count).unwrap(), output);
    assert_eq!(
        reopened
            .state
            .adapters
            .iter()
            .map(|a| a.state_trace())
            .collect::<Vec<_>>(),
        state
    );
    assert!(!reopened.status().source_caught_up);
    append(
        &mut reopened,
        bar(epoch("2026-10-13 07:10"), DataOrigin::Live, 90200.),
        epoch("2026-10-13 07:20"),
    );
    ingest(&mut reopened);
    observe(&mut reopened);
    let last = reopened.committed_output(reopened.log.count()).unwrap();
    assert_ne!(last.outbox_id, output.outbox_id);
    assert!(reopened.state.adapters[0].state_trace()["mr_position"].is_null());
}

#[test]
fn durable_commit_before_memory_swap_recovers_once() {
    let (dir, mut r) = fixture();
    append(
        &mut r,
        bar(epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
        epoch("2026-10-13 07:10"),
    );
    ingest(&mut r);
    let (next, tx) = RiShadowRunner::prepare(
        &r.header,
        &r.state,
        &r.journal,
        r.log.count() + 1,
        r.log.head().into(),
        Operation::Observe {
            through: r.journal.last_sequence(),
        },
        clock(),
    )
    .unwrap();
    r.log.append(&tx).unwrap();
    // Simulated kill after durable HEAD, before the in-memory swap/return.
    assert!(r.state.observed < next.observed);
    drop(r);
    let r = RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).unwrap();
    assert_eq!(r.committed_output(tx.index).unwrap(), tx.output);
    assert_eq!(
        r.state.adapters[0].state_trace(),
        next.adapters[0].state_trace()
    );
}

#[test]
fn durable_receipt_tail_is_not_lost_when_no_transaction_was_written() {
    let (dir, mut r) = fixture();
    append(
        &mut r,
        bar(epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
        epoch("2026-10-13 07:10"),
    );
    let tail = r.journal.last_sequence();
    drop(r);
    let mut r = RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).unwrap();
    assert_eq!(r.journal.last_sequence(), tail);
    assert_eq!(r.state.observed, tail - 1);
    ingest(&mut r);
    observe(&mut r);
    assert_eq!(r.state.observed, tail);
    assert!(!r.state.adapters[0].state_trace()["mr_position"].is_null());
}

#[test]
fn paired_runner_recovers_pending_bo_then_closes_at_eod() {
    let (dir, mut r) = fixture();
    for ts in (epoch("2026-10-13 07:00")..=epoch("2026-10-13 08:50")).step_by(600) {
        let close = if ts == epoch("2026-10-13 08:50") {
            91000.
        } else {
            90000.
        };
        append(&mut r, bar(ts, DataOrigin::Live, close), ts + 600);
    }
    ingest(&mut r);
    observe(&mut r);
    let before = r.state.adapters[0].state_trace();
    assert_eq!(before["bo_pending"]["kind"], "entry");
    assert_eq!(before["bo_observation_index"], 12);
    drop(r);
    let mut r = RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).unwrap();
    assert_eq!(r.state.adapters[0].state_trace(), before);
    for (label, close) in [("2026-10-13 09:00", 91000.), ("2026-10-13 23:00", 91100.)] {
        append(
            &mut r,
            bar(epoch(label), DataOrigin::Live, close),
            epoch(label) + 600,
        );
        ingest(&mut r);
        observe(&mut r);
    }
    let state = r.state.adapters[0].state_trace();
    assert!(state["bo_position"].is_null());
    assert!(state["bo_pending"].is_null());
    assert_eq!(state["bo_entries_today"], 1);
    let count = r.log.count();
    drop(r);
    let r = RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).unwrap();
    assert_eq!(r.state.adapters[0].state_trace(), state);
    assert_eq!(r.log.count(), count);
}

#[test]
fn torn_tail_or_deleted_committed_suffix_cannot_silently_replay() {
    for scenario in ["temp", "beyond_head", "deleted", "corrupt"] {
        let (dir, r) = fixture();
        let count = r.log.count();
        drop(r);
        let path = dir.path().join("runner/transactions");
        match scenario {
            "temp" => fs::write(path.join(".transaction-crash.tmp"), b"partial").unwrap(),
            "beyond_head" => {
                fs::copy(
                    path.join(format!("{count:08}.json")),
                    path.join(format!("{:08}.json", count + 1)),
                )
                .unwrap();
            }
            "deleted" => fs::remove_file(path.join(format!("{count:08}.json"))).unwrap(),
            _ => fs::write(path.join(format!("{count:08}.json")), b"{}").unwrap(),
        }
        assert!(
            RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).is_err(),
            "{scenario}"
        );
    }
}

#[test]
fn runner_rejects_live_config_wrong_generation_and_second_writer() {
    let (dir, r) = fixture();
    assert!(RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).is_err());
    drop(r);
    for change in ["live", "generation", "bo_max2", "source"] {
        let mut cfg = spec();
        match change {
            "live" => cfg.strategy.mode = RiAuthor4142RuntimeMode::MicroLive,
            "generation" => cfg.generation.id.push_str("-different"),
            "bo_max2" => cfg.strategy.author42_max_entries_per_day = Some(2),
            _ => cfg.source_stream.push_str("-different"),
        }
        assert!(RiShadowRunner::open_with_clock(&dir.path().join("runner"), cfg, clock).is_err());
    }
    assert!(RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).is_ok());
}

#[test]
fn uncommitted_previous_session_live_bar_blocks_instead_of_fabricating_next_day_trade() {
    let (_dir, mut r) = fixture();
    append(
        &mut r,
        bar(epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
        epoch("2026-10-13 07:10"),
    );
    ingest(&mut r);
    set_clock(epoch("2026-10-14 07:10"));
    let before = r.state.adapters[0].state_trace();
    let result = r.commit(
        Operation::Observe {
            through: r.journal.last_sequence(),
        },
        clock(),
    );
    assert!(result.unwrap_err().to_string().contains("crossed session"));
    assert_eq!(r.state.adapters[0].state_trace(), before);
}

#[test]
fn forged_cutoff_and_failed_write_do_not_release_any_decision() {
    let (dir, mut r) = fixture();
    let count = r.log.count();
    let mut fake = r.state.stores[0]
        .frozen(date("2026-10-13"))
        .unwrap()
        .barrier
        .clone();
    fake.attested_at_utc += 1;
    assert!(r
        .commit(
            Operation::Freeze {
                date: date("2026-10-13"),
                barrier: fake
            },
            clock()
        )
        .is_err());
    assert_eq!(r.log.count(), count);
    append(
        &mut r,
        bar(epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
        epoch("2026-10-13 07:10"),
    );
    ingest(&mut r);
    let before = r.state.adapters[0].state_trace();
    let head = dir.path().join("runner/transactions/HEAD.json");
    fs::remove_file(&head).unwrap();
    fs::create_dir(&head).unwrap();
    let error = r
        .commit(
            Operation::Observe {
                through: r.journal.last_sequence(),
            },
            clock(),
        )
        .unwrap_err();
    let _ = r.log.fail(error);
    assert_eq!(r.state.adapters[0].state_trace(), before);
    assert!(r.status().failed);
    assert!(r.committed_output(r.log.count()).is_err());
    drop(r);
    assert!(RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).is_err());
}

#[test]
fn structurally_valid_transaction_with_wrong_witness_fails_deterministic_recovery() {
    let (dir, r) = fixture();
    let count = r.log.count();
    let mut tx = r.log.read(count).unwrap();
    drop(r);
    tx.witnesses[0] = "{}".into();
    let path = dir.path().join("runner/transactions");
    fs::write(
        path.join(format!("{count:08}.json")),
        serde_json::to_vec(&serde_json::json!({"payload": tx, "sha256": hash(&tx).unwrap()}))
            .unwrap(),
    )
    .unwrap();
    let head = serde_json::json!({"index": count, "sha256": hash(&tx).unwrap()});
    fs::write(
        path.join("HEAD.json"),
        serde_json::to_vec(&serde_json::json!({"payload": head, "sha256": hash(&head).unwrap()}))
            .unwrap(),
    )
    .unwrap();
    assert!(RiShadowRunner::open_with_clock(&dir.path().join("runner"), spec(), clock).is_err());
}

#[test]
fn new_generation_cannot_claim_a_cutoff_that_already_passed() {
    let dir = tempfile::tempdir().unwrap();
    set_clock(epoch("2026-10-13 07:01"));
    assert!(RiShadowRunner::create_with_clock(&dir.path().join("runner"), spec(), clock).is_err());
    assert!(!dir.path().join("runner").exists());
    let roundtrip: RunnerSpec = toml::from_str(&toml::to_string(&spec()).unwrap()).unwrap();
    assert_eq!(roundtrip, spec());
}

#[tokio::test]
#[ignore = "requires isolated local redis-server; never uses REDIS_URL"]
async fn ri_paired_runner_readonly_redis_restart_integration() {
    use crate::ri_context::ingress::redis_source::tests::TestServer;
    use alor_protocol::{Envelope, MessageType};
    let server = TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("pairedreader")
        .arg("on")
        .arg(">test-local-only")
        .arg("~fixture:bars")
        .arg("+multi")
        .arg("+exec")
        .arg("+xrange")
        .arg("+ping")
        .query_async(&mut admin)
        .await
        .unwrap();
    let mut reader = redis::Client::open(format!(
        "redis://pairedreader:test-local-only@127.0.0.1:{}/",
        server.port
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    async fn send(admin: &mut MultiplexedConnection, seq: &mut usize, b: BarEvent) {
        *seq += 1;
        let raw = serde_json::to_string(&Envelope::new(1, "fixture", MessageType::Bar, b)).unwrap();
        let _: String = redis::cmd("XADD")
            .arg("fixture:bars")
            .arg(format!("{seq}-0"))
            .arg("payload")
            .arg(raw)
            .query_async(admin)
            .await
            .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("runner");
    set_clock(epoch("2026-10-09 06:00"));
    let mut r = RiShadowRunner::create_with_clock(&root, spec(), clock).unwrap();
    let mut seq = 0;
    for day in ["2026-10-09", "2026-10-12"] {
        for label in (epoch(&format!("{day} 07:00"))..=epoch(&format!("{day} 23:30"))).step_by(600)
        {
            let mut b = bar(label, DataOrigin::History, 90000.);
            b.h = 91000.;
            b.l = 89000.;
            send(&mut admin, &mut seq, b).await;
        }
        set_clock(epoch(&format!("{day} 23:40")));
        assert!(r.poll_once(&mut reader).await.unwrap().source_caught_up);
    }
    let mut closing = bar(epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.);
    closing.h = 91000.;
    closing.l = 89000.;
    send(&mut admin, &mut seq, closing).await;
    set_clock(epoch("2026-10-13 03:00"));
    r.poll_once(&mut reader).await.unwrap();
    set_clock(epoch("2026-10-13 07:00"));
    r.poll_once(&mut reader).await.unwrap();
    send(
        &mut admin,
        &mut seq,
        bar(epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
    )
    .await;
    set_clock(epoch("2026-10-13 07:10"));
    let status = r.poll_once(&mut reader).await.unwrap();
    assert_eq!(status.observed_sequence as usize, seq);
    let state: Vec<_> = r.state.adapters.iter().map(|a| a.state_trace()).collect();
    assert!(!state[0]["mr_position"].is_null());
    assert_eq!(state[1]["frozen_context"]["previous"]["close"], 90500.);
    let last = r.committed_output(status.committed_transactions).unwrap();
    assert_eq!(
        r.poll_once(&mut reader)
            .await
            .unwrap()
            .committed_transactions,
        status.committed_transactions
    );
    drop(r);
    let mut r = RiShadowRunner::open_with_clock(&root, spec(), clock).unwrap();
    assert_eq!(
        r.poll_once(&mut reader)
            .await
            .unwrap()
            .committed_transactions,
        status.committed_transactions
    );
    assert_eq!(
        r.committed_output(status.committed_transactions).unwrap(),
        last
    );
    assert_eq!(
        r.state
            .adapters
            .iter()
            .map(|a| a.state_trace())
            .collect::<Vec<_>>(),
        state
    );
    let denied: redis::RedisResult<String> = redis::cmd("XADD")
        .arg("fixture:commands")
        .arg("*")
        .arg("payload")
        .arg("forbidden")
        .query_async(&mut reader)
        .await;
    assert!(denied.is_err());
    let keys: usize = redis::cmd("DBSIZE").query_async(&mut admin).await.unwrap();
    let length: usize = redis::cmd("XLEN")
        .arg("fixture:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert_eq!(keys, 1);
    assert_eq!(length, seq);
    // Missing resume cursor stops both arms durably; a normal restart cannot
    // clear the incident or continue from whatever happens to be in Redis now.
    send(
        &mut admin,
        &mut seq,
        bar(epoch("2026-10-13 07:10"), DataOrigin::Live, 90200.),
    )
    .await;
    let _: usize = redis::cmd("XTRIM")
        .arg("fixture:bars")
        .arg("MAXLEN")
        .arg(1)
        .query_async(&mut admin)
        .await
        .unwrap();
    assert!(r.poll_once(&mut reader).await.is_err());
    assert!(r.status().failed);
    assert_eq!(r.log.count(), status.committed_transactions);
    drop(r);
    assert!(RiShadowRunner::open_with_clock(&root, spec(), clock).is_err());
}
