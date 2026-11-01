use super::*;
use crate::shadow_reaction::{imoexf, usdrubf, Sleeve};
use std::cell::Cell;

mod bootstrap_tests;
mod continuous;
mod service;

thread_local! { static NOW: Cell<i64> = const { Cell::new(0) }; }
fn clock() -> i64 {
    NOW.with(Cell::get)
}
fn now(ts: i64) {
    NOW.with(|v| v.set(ts));
}
fn epoch(s: &str) -> i64 {
    (NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap() - Duration::hours(3))
        .and_utc()
        .timestamp()
}
fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}
fn spec(symbol: &str) -> RunnerSpec {
    RunnerSpec {
        ingress: IngressIdentity {
            generation: "paired-reaction-fixture-v1".into(),
            symbol: symbol.into(),
            source_stream: "fixture:reaction:bars".into(),
            receipt_stream: "fixture:reaction:receipts".into(),
        },
        first_decision_session: date("2026-10-26"),
        excluded_dates: BTreeSet::new(),
        bootstrap: None,
        config: if symbol == "IMOEXF" {
            Config::Imoexf {
                sleeve: Sleeve::Combo,
                costs: imoexf::Costs {
                    point_value_rub: 10.,
                    tick_value_rub: 5.,
                    rate_per_component_per_side: 0.,
                    components: 2,
                },
            }
        } else {
            Config::Usdrubf {
                settings: usdrubf::Config {
                    model_id: "fixture_pd7_h3_w4".into(),
                    family: usdrubf::Family::PreviousDay,
                    context_hour: 7,
                    clock_hour: 7,
                    mr_horizon: 3,
                    bo_wait: 4,
                    mr_start_delay: 0,
                },
            }
        },
    }
}
fn bar(symbol: &str, ts: i64, price: f64) -> BarEvent {
    BarEvent {
        symbol: symbol.into(),
        close_time_utc: ts,
        o: price,
        h: price,
        l: price,
        close: price,
        v: 1.,
        origin: DataOrigin::Live,
    }
}
fn warm_bars(symbol: &str) -> Vec<BarEvent> {
    let mut out = vec![];
    for (day, weekend) in [
        ("2026-10-22", false),
        ("2026-10-23", false),
        ("2026-10-24", true),
        ("2026-10-25", true),
    ] {
        let first = epoch(&format!(
            "{day} {}",
            if weekend { "09:50" } else { "07:00" }
        ));
        let last = epoch(&format!(
            "{day} {}",
            if weekend { "18:50" } else { "23:40" }
        ));
        for ts in (first..=last).step_by(600) {
            let mut b = bar(symbol, ts, if weekend { 103. } else { 100. });
            b.h += 2.;
            b.l -= 2.;
            out.push(b);
        }
    }
    out
}
fn create(symbol: &str) -> (tempfile::TempDir, ReactionShadowRunner) {
    now(epoch("2026-10-22 06:00"));
    let temp = tempfile::tempdir().unwrap();
    let runner =
        ReactionShadowRunner::create_with_clock(&temp.path().join("run"), spec(symbol), clock)
            .unwrap();
    (temp, runner)
}
fn append(r: &mut ReactionShadowRunner, b: &BarEvent) {
    let n = r.collector.journal.last_sequence() + 1;
    r.collector
        .journal
        .append(&format!("{n}-0"), &serde_json::to_string(b).unwrap())
        .unwrap()
        .input
        .unwrap();
}
fn ingest(r: &mut ReactionShadowRunner) {
    let through = r.collector.journal.last_sequence();
    r.commit(Operation::Ingest { through }, clock()).unwrap();
}
fn observe(r: &mut ReactionShadowRunner) -> Result<()> {
    while r.state.observed < r.state.stores[0].last_sequence() {
        let through = (r.state.observed + OBSERVATION_BATCH).min(r.state.stores[0].last_sequence());
        r.commit(Operation::Observe { through }, clock())?;
    }
    Ok(())
}
fn freeze(r: &mut ReactionShadowRunner, day: &str) -> Result<()> {
    // Unit fixture attests using the real journal; the ignored Redis test
    // separately exercises public caught-up polling and read-only permissions.
    let raw = r
        .collector
        .journal
        .seal_cutoff(epoch(&format!("{day} 07:00")))?;
    let cutoff = r.collector.project_cutoff(&raw)?;
    r.commit(
        Operation::Freeze {
            date: date(day),
            cutoff: Box::new(cutoff),
        },
        clock(),
    )
}
fn warm(r: &mut ReactionShadowRunner) {
    for b in warm_bars(&r.spec.ingress.symbol) {
        now(b.close_time_utc + 600);
        append(r, &b);
    }
    ingest(r);
    observe(r).unwrap();
    now(epoch("2026-10-26 07:00"));
    freeze(r, "2026-10-26").unwrap();
}
fn live_step(r: &mut ReactionShadowRunner, slot: usize, price: f64) -> Result<()> {
    let b = bar(
        &r.spec.ingress.symbol,
        epoch("2026-10-26 07:00") + slot as i64 * 600,
        price,
    );
    now(b.close_time_utc + 600);
    append(r, &b);
    ingest(r);
    observe(r)
}
fn restart(temp: &tempfile::TempDir, r: ReactionShadowRunner) -> ReactionShadowRunner {
    let spec = r.spec.clone();
    let before = r.state.model_hash().unwrap();
    let count = r.log.count();
    let outputs: Vec<_> = (1..=count)
        .map(|i| r.committed_output(i).unwrap())
        .collect();
    drop(r);
    let r = ReactionShadowRunner::open_with_clock(&temp.path().join("run"), spec, clock).unwrap();
    assert_eq!(before, r.state.model_hash().unwrap());
    assert_eq!(count, r.log.count());
    assert_eq!(
        outputs,
        (1..=count)
            .map(|i| r.committed_output(i).unwrap())
            .collect::<Vec<_>>()
    );
    assert!(!r.status().source_caught_up);
    r
}

#[test]
fn reaction_pair_preserves_pending_fill_and_outbox_across_restart() {
    for symbol in ["IMOEXF", "USDRUBF"] {
        let (temp, mut r) = create(symbol);
        warm(&mut r);
        assert!(r.status().pair_ready);
        for (slot, price) in [100.1, 100.12, 99.4, 99.6].into_iter().enumerate() {
            live_step(&mut r, slot, price).unwrap();
            let state = r.state.states().unwrap().unwrap();
            let model = &state[0]["model"]["state"];
            match slot {
                0 => assert!(model["pending"].is_object()),
                1 => assert!(model["position"].is_object()),
                2 => assert!(model["pending"].is_object() && model["position"].is_object()),
                _ => {
                    assert!(model["position"].is_null());
                    assert_eq!(model["trades"].as_array().unwrap().len(), 1);
                }
            }
            assert_ne!(state[0]["model"], state[1]["model"]);
            r = restart(&temp, r);
        }
        live_step(&mut r, 4, 99.6).unwrap();
        let trades: Vec<_> = (1..=r.log.count())
            .flat_map(|index| {
                r.committed_output(index)
                    .unwrap()
                    .steps
                    .into_iter()
                    .filter_map(|s| s.deltas)
                    .flat_map(|delta| delta[0].trades.clone())
            })
            .collect();
        assert_eq!(
            trades.len(),
            1,
            "cumulative state must not duplicate closed rounds"
        );
        assert_eq!(trades[0].id.len(), 64);
        r = restart(&temp, r);
        assert!(!r.status().order_emission_allowed);
        assert!(r.committed_output(r.log.count() + 1).is_err());
    }
}

#[test]
fn reaction_pair_rolls_only_after_complete_eod_and_preserves_next_anchors() {
    for symbol in ["IMOEXF", "USDRUBF"] {
        let (temp, mut r) = create(symbol);
        warm(&mut r);
        let wait = if symbol == "IMOEXF" { 18 } else { 24 };
        for slot in 0..=100 {
            let b = bar(
                symbol,
                epoch("2026-10-26 07:00") + slot * 600,
                if slot < wait { 100. } else { 104. },
            );
            now(b.close_time_utc + 600);
            append(&mut r, &b);
        }
        ingest(&mut r);
        observe(&mut r).unwrap();
        assert!(r.status().session_complete);
        let states = r.state.states().unwrap().unwrap();
        assert!(!states[0]["model"]["state"]["trades"]
            .as_array()
            .unwrap()
            .is_empty());
        r = restart(&temp, r);
        now(epoch("2026-10-27 07:00"));
        freeze(&mut r, "2026-10-27").unwrap();
        for s in &r.state.stores {
            assert_eq!(
                s.frozen(date("2026-10-27"))
                    .unwrap()
                    .previous
                    .as_ref()
                    .unwrap()
                    .session_date,
                date("2026-10-26")
            );
        }
        assert_eq!(r.state.states().unwrap().unwrap()[0]["bars_processed"], 0);
        restart(&temp, r);
    }
}

#[test]
fn reaction_pair_never_discards_missing_bar_or_unfinished_eod() {
    let (_temp, mut r) = create("IMOEXF");
    warm(&mut r);
    live_step(&mut r, 0, 100.1).unwrap();
    let before = r.state.states().unwrap();
    assert!(live_step(&mut r, 2, 100.1).is_err());
    assert_eq!(before, r.state.states().unwrap());
    now(epoch("2026-10-27 07:00"));
    assert!(freeze(&mut r, "2026-10-27").is_err());
    assert_eq!(before, r.state.states().unwrap());
    assert_eq!(r.state.day, Some(date("2026-10-26")));
}

#[test]
fn reaction_pair_history_is_context_only_and_duplicate_live_has_no_second_fill() {
    let (_temp, mut r) = create("USDRUBF");
    warm(&mut r);
    live_step(&mut r, 0, 100.1).unwrap();
    let before = r.state.states().unwrap();
    let b = bar("USDRUBF", epoch("2026-10-26 07:00"), 100.1);
    append(&mut r, &b);
    ingest(&mut r);
    observe(&mut r).unwrap();
    assert_eq!(before, r.state.states().unwrap());
    assert_eq!(
        r.committed_output(r.log.count()).unwrap().steps[0].disposition,
        Disposition::Duplicate
    );
    let mut history = bar("USDRUBF", epoch("2026-10-26 07:10"), 100.12);
    history.origin = DataOrigin::HistoryGap;
    now(history.close_time_utc + 600);
    append(&mut r, &history);
    ingest(&mut r);
    observe(&mut r).unwrap();
    assert_eq!(before, r.state.states().unwrap());
    assert_eq!(
        r.committed_output(r.log.count()).unwrap().steps[0].disposition,
        Disposition::ContextOnly
    );
    history.origin = DataOrigin::Live;
    append(&mut r, &history);
    ingest(&mut r);
    assert!(
        observe(&mut r).is_err(),
        "history-owned quote must not become a prospective fill"
    );
}

#[test]
fn reaction_pair_second_arm_failure_cannot_release_first_arm() {
    let (_temp, mut r) = create("IMOEXF");
    warm(&mut r);
    let mut b = bar("IMOEXF", epoch("2026-10-26 07:00"), 100.1);
    b.origin = DataOrigin::Replay;
    r.state.sessions.as_mut().unwrap()[1]
        .observe(b.clone(), b.close_time_utc + 600)
        .unwrap();
    let before = r.state.states().unwrap();
    assert!(live_step(&mut r, 0, 100.1).is_err());
    assert_eq!(before, r.state.states().unwrap());
    assert!(r.committed_output(r.log.count()).unwrap().steps.is_empty());
}

#[test]
fn reaction_pair_write_failure_keeps_state_and_blocks_restart() {
    let (temp, mut r) = create("IMOEXF");
    warm(&mut r);
    let b = bar("IMOEXF", epoch("2026-10-26 07:00"), 100.1);
    now(b.close_time_utc + 600);
    append(&mut r, &b);
    ingest(&mut r);
    let before = r.state.model_hash().unwrap();
    let index = r.log.count() + 1;
    fs::write(
        temp.path()
            .join(format!("run/transactions/{index:08}.json")),
        b"collision",
    )
    .unwrap();
    let error = observe(&mut r).unwrap_err();
    r.log.fail(error);
    assert_eq!(before, r.state.model_hash().unwrap());
    assert!(r.committed_output(index).is_err());
    let spec = r.spec.clone();
    drop(r);
    assert!(ReactionShadowRunner::open_with_clock(&temp.path().join("run"), spec, clock).is_err());
}

#[test]
fn reaction_pair_uncommitted_cutoff_and_torn_tail_block_recovery() {
    for cutoff in [true, false] {
        let (temp, mut r) = create("IMOEXF");
        warm(&mut r);
        if cutoff {
            now(epoch("2026-10-27 07:00"));
            r.collector.journal.seal_cutoff(clock()).unwrap();
        } else {
            fs::write(
                temp.path().join("run/transactions/.transaction-torn.tmp"),
                b"partial",
            )
            .unwrap();
        }
        let spec = r.spec.clone();
        drop(r);
        assert!(
            ReactionShadowRunner::open_with_clock(&temp.path().join("run"), spec, clock).is_err()
        );
    }
}

#[test]
fn reaction_pair_rehashed_false_state_fails_reexecution() {
    let (temp, mut r) = create("IMOEXF");
    warm(&mut r);
    live_step(&mut r, 0, 100.1).unwrap();
    let last = r.log.count();
    let spec = r.spec.clone();
    drop(r);
    let path = temp.path().join(format!("run/transactions/{last:08}.json"));
    let mut checked: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut tx: Transaction = serde_json::from_value(checked["payload"].clone()).unwrap();
    tx.model_sha256 = "0".repeat(64);
    checked["payload"] = serde_json::to_value(&tx).unwrap();
    checked["sha256"] = hash(&tx).unwrap().into();
    fs::write(path, serde_json::to_vec(&checked).unwrap()).unwrap();
    let head = serde_json::json!({"index":last,"sha256":hash(&tx).unwrap()});
    fs::write(
        temp.path().join("run/transactions/HEAD.json"),
        serde_json::to_vec(&serde_json::json!({"payload":head,"sha256":hash(&head).unwrap()}))
            .unwrap(),
    )
    .unwrap();
    assert!(ReactionShadowRunner::open_with_clock(&temp.path().join("run"), spec, clock).is_err());
    assert!(temp.path().join("run/transactions/FAILED.json").exists());
}

#[test]
fn reaction_pair_changed_config_activation_and_parallel_writer_rejected() {
    let (temp, r) = create("IMOEXF");
    assert!(
        ReactionShadowRunner::open_with_clock(&temp.path().join("run"), r.spec.clone(), clock)
            .is_err()
    );
    let mut changed = r.spec.clone();
    changed.excluded_dates.insert(date("2026-10-27"));
    drop(r);
    assert!(
        ReactionShadowRunner::open_with_clock(&temp.path().join("run"), changed, clock).is_err()
    );
    now(epoch("2026-10-26 07:00"));
    assert!(ReactionShadowRunner::create_with_clock(
        &temp.path().join("late"),
        spec("IMOEXF"),
        clock
    )
    .is_err());
    let mut unsupported = spec("USDRUBF");
    if let Config::Usdrubf { settings } = &mut unsupported.config {
        settings.bo_wait = 2;
    }
    assert!(unsupported.header().is_err());
}

#[test]
fn reaction_pair_not_ready_is_common_suppression_not_asymmetric_activation() {
    let (_temp, mut r) = create("IMOEXF");
    for b in warm_bars("IMOEXF").into_iter().filter(|b| {
        local(b.close_time_utc)
            .unwrap()
            .weekday()
            .num_days_from_monday()
            >= 5
    }) {
        now(b.close_time_utc + 600);
        append(&mut r, &b);
    }
    ingest(&mut r);
    observe(&mut r).unwrap();
    now(epoch("2026-10-26 07:00"));
    freeze(&mut r, "2026-10-26").unwrap();
    assert!(!r.state.stores[0]
        .frozen(date("2026-10-26"))
        .unwrap()
        .ready());
    assert!(r.state.stores[1]
        .frozen(date("2026-10-26"))
        .unwrap()
        .ready());
    live_step(&mut r, 0, 100.1).unwrap();
    assert!(!r.status().pair_ready);
    let output = r.committed_output(r.log.count()).unwrap();
    assert_eq!(output.steps[0].disposition, Disposition::ContextNotReady);
    assert!(output.steps[0].states.is_none());
}

#[test]
fn reaction_pair_weekend_and_service_bars_never_advance_model_state() {
    let (_temp, mut r) = create("IMOEXF");
    warm(&mut r);
    let b = bar("IMOEXF", epoch("2026-10-26 06:50"), 100.1);
    append(&mut r, &b);
    ingest(&mut r);
    observe(&mut r).unwrap();
    assert_eq!(r.state.states().unwrap().unwrap()[0]["bars_processed"], 0);
    live_step(&mut r, 0, 100.1).unwrap();
    let before = r.state.states().unwrap();
    let b = bar("IMOEXF", epoch("2026-10-31 10:00"), 100.1);
    now(b.close_time_utc + 600);
    append(&mut r, &b);
    ingest(&mut r);
    observe(&mut r).unwrap();
    assert_eq!(before, r.state.states().unwrap());
    assert_eq!(
        r.committed_output(r.log.count()).unwrap().steps[0].disposition,
        Disposition::NonTradingBar
    );
}

#[test]
fn reaction_pair_unobserved_previous_day_cannot_be_replayed_as_today() {
    let (_temp, mut r) = create("IMOEXF");
    warm(&mut r);
    let b = bar("IMOEXF", epoch("2026-10-26 07:00"), 100.1);
    now(b.close_time_utc + 600);
    append(&mut r, &b);
    ingest(&mut r);
    now(epoch("2026-10-27 07:00"));
    assert!(observe(&mut r).is_err());
    assert!(freeze(&mut r, "2026-10-27").is_err());
}

#[tokio::test]
#[ignore = "starts owned loopback Redis; explicit local integration only"]
async fn reaction_runner_readonly_redis_restart_integration() {
    use crate::ri_context::ingress::redis_source::tests::TestServer;
    use alor_protocol::{Envelope, MessageType};
    let server = TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("reaction_reader")
        .arg("on")
        .arg(">fixture-password")
        .arg("~fixture:reaction:bars")
        .arg("-@all")
        .arg("+multi")
        .arg("+exec")
        .arg("+xrange")
        .arg("+ping")
        .query_async(&mut admin)
        .await
        .unwrap();
    let client = redis::Client::open(format!(
        "redis://reaction_reader:fixture-password@127.0.0.1:{}/",
        server.port
    ))
    .unwrap();
    let mut reader = client.get_multiplexed_async_connection().await.unwrap();
    let (temp, mut r) = create("USDRUBF");
    let mut inputs = warm_bars("USDRUBF");
    inputs.extend((0..4).map(|slot| {
        bar(
            "USDRUBF",
            epoch("2026-10-26 07:00") + slot * 600,
            [100.1, 100.12, 99.4, 99.6][slot as usize],
        )
    }));
    for group in inputs.chunk_by(|a, b| {
        local(a.close_time_utc).unwrap().date() == local(b.close_time_utc).unwrap().date()
    }) {
        for b in group {
            let _: String = redis::cmd("XADD")
                .arg("fixture:reaction:bars")
                .arg(format!("{}-0", b.close_time_utc))
                .arg("payload")
                .arg(
                    serde_json::to_string(&Envelope::new(
                        1,
                        "fixture-gateway",
                        MessageType::Bar,
                        b,
                    ))
                    .unwrap(),
                )
                .query_async(&mut admin)
                .await
                .unwrap();
        }
        now(group.last().unwrap().close_time_utc + 600);
        assert!(r.poll_once(&mut reader).await.unwrap().source_caught_up);
    }
    assert!(r.status().pair_ready);
    assert_eq!(
        r.state.states().unwrap().unwrap()[0]["model"]["state"]["trades"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    r = restart(&temp, r);
    let count = r.log.count();
    r.poll_once(&mut reader).await.unwrap();
    assert_eq!(count, r.log.count());
    let denied: redis::RedisResult<String> = redis::cmd("XADD")
        .arg("fixture:commands")
        .arg("*")
        .arg("payload")
        .arg("forbidden")
        .query_async(&mut reader)
        .await;
    assert!(denied.is_err());
    let groups: Vec<redis::Value> = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("fixture:reaction:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert!(groups.is_empty());
    let len: usize = redis::cmd("XLEN")
        .arg("fixture:reaction:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert_eq!(len, inputs.len());
    let _: usize = redis::cmd("XDEL")
        .arg("fixture:reaction:bars")
        .arg(r.collector.journal.last_source_id().unwrap())
        .query_async(&mut admin)
        .await
        .unwrap();
    assert!(r.poll_once(&mut reader).await.is_err());
    assert!(r.status().failed);
}
