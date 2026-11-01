use super::*;
use crate::shadow_session_context::collector::runner::bootstrap::HistoricalBootstrap;

fn plan(bars: &[BarEvent], start: u64) -> HistoricalBootstrap {
    let ids: Vec<_> = (start..start + bars.len() as u64)
        .map(|id| format!("{id}-0"))
        .collect();
    let dates: BTreeSet<_> = bars
        .iter()
        .map(|b| local(b.close_time_utc).unwrap().date())
        .collect();
    HistoricalBootstrap {
        first_source_id: ids.first().unwrap().clone(),
        last_source_id: ids.last().unwrap().clone(),
        rows: bars.len() as u64,
        source_sha256: hash(&ids.iter().zip(bars).collect::<Vec<_>>()).unwrap(),
        first_session: *dates.first().unwrap(),
        last_session: *dates.last().unwrap(),
    }
}

fn setup(bars: &[BarEvent]) -> (tempfile::TempDir, ReactionShadowRunner) {
    now(epoch("2026-10-26 06:00"));
    let mut spec = spec(&bars[0].symbol);
    spec.bootstrap = Some(plan(bars, 1));
    let temp = tempfile::tempdir().unwrap();
    let r = ReactionShadowRunner::create_with_clock(&temp.path().join("run"), spec, clock).unwrap();
    (temp, r)
}

#[test]
fn reaction_bootstrap_context_matches_prospective_reference_without_historical_decisions() {
    for symbol in ["IMOEXF", "USDRUBF"] {
        let (_control_dir, mut control) = create(symbol);
        warm(&mut control);
        let bars = warm_bars(symbol);
        let (temp, mut r) = setup(&bars);
        for b in &bars {
            append(&mut r, b);
        }
        assert!(r
            .commit(
                Operation::Ingest {
                    through: bars.len() as u64
                },
                clock()
            )
            .is_err());
        r.commit(Operation::Bootstrap, clock()).unwrap();
        assert_eq!(r.status().bootstrap_status, "loaded");
        assert_eq!(r.state.observed, bars.len() as u64);
        assert!(r.state.sessions.is_none() && r.state.day.is_none());
        let output = r.committed_output(1).unwrap();
        assert!(output.steps.is_empty() && output.frozen.is_none());
        assert!(output.historical_bootstrap.is_some() && !output.order_emission_allowed);
        assert!(r.commit(Operation::Bootstrap, clock()).is_err());
        for store in &r.state.stores {
            assert!(store
                .inputs
                .iter()
                .all(|i| i.received_at_utc == Some(clock()) && i.durable_at_utc == Some(clock())));
            let restored = Store::restore(
                store.identity(),
                &store.checkpoint_bytes().unwrap(),
                clock(),
            )
            .unwrap();
            assert_eq!(
                restored
                    .anchors_before(r.spec.first_decision_session)
                    .unwrap(),
                store.anchors_before(r.spec.first_decision_session).unwrap()
            );
        }
        r = restart(&temp, r);
        now(epoch("2026-10-26 07:00"));
        freeze(&mut r, "2026-10-26").unwrap();
        for arm in 0..2 {
            let actual = r.state.stores[arm]
                .frozen(r.spec.first_decision_session)
                .unwrap();
            let expected = control.state.stores[arm]
                .frozen(r.spec.first_decision_session)
                .unwrap();
            for (a, e) in [
                (&actual.previous, &expected.previous),
                (&actual.previous2, &expected.previous2),
            ] {
                let (a, e) = (a.as_ref().unwrap(), e.as_ref().unwrap());
                assert_eq!(
                    (
                        a.session_date,
                        a.open,
                        a.high,
                        a.low,
                        a.close,
                        a.range,
                        a.distinct_bars
                    ),
                    (
                        e.session_date,
                        e.open,
                        e.high,
                        e.low,
                        e.close,
                        e.range,
                        e.distinct_bars
                    )
                );
                assert_eq!(a.latest_received_at_utc, epoch("2026-10-26 06:00"));
            }
        }
        // Numerical state, pending ownership, counters, events and trades match;
        // receipt provenance intentionally differs from the prospective fixture.
        for (slot, price) in [100.1, 100.12, 99.4, 99.6, 100.4].into_iter().enumerate() {
            live_step(&mut control, slot, price).unwrap();
            live_step(&mut r, slot, price).unwrap();
            let (actual, expected) = (
                r.state.states().unwrap().unwrap(),
                control.state.states().unwrap().unwrap(),
            );
            for arm in 0..2 {
                assert_eq!(actual[arm]["model"], expected[arm]["model"]);
            }
        }
        restart(&temp, r);
    }
}

#[test]
fn reaction_bootstrap_does_not_relax_normal_late_receipt_or_frozen_context_guards() {
    let bars = warm_bars("IMOEXF");
    let (_temp, mut r) = setup(&bars);
    for b in &bars {
        append(&mut r, b);
    }
    r.commit(Operation::Bootstrap, clock()).unwrap();
    let old = r.state.stores[0]
        .anchors_before(r.spec.first_decision_session)
        .unwrap();
    let mut changed = bars[0].clone();
    changed.h += 100.;
    append(&mut r, &changed);
    ingest(&mut r);
    assert!(matches!(
        r.state.stores[0].operations.last(),
        Some(crate::shadow_session_context::Operation::Ingest {
            route: Route::Quarantined(Reject::AtOrAfterCutoff),
            ..
        })
    ));
    assert_eq!(
        old,
        r.state.stores[0]
            .anchors_before(r.spec.first_decision_session)
            .unwrap()
    );
    let mut ordinary = Store::new(r.header.identities[0].clone()).unwrap();
    let first = r.collector.prefix(1).unwrap().remove(0);
    assert_eq!(
        ordinary.ingest(first, clock()).unwrap(),
        Route::Quarantined(Reject::AtOrAfterCutoff)
    );
}

#[test]
fn reaction_bootstrap_rejects_incomplete_conflicting_or_wrong_contract_history_atomically() {
    for fault in [
        "missing_day",
        "missing_bar",
        "conflict",
        "symbol",
        "ohlc",
        "replay",
        "future",
        "clock",
    ] {
        let mut bars = warm_bars("IMOEXF");
        match fault {
            "missing_day" => {
                bars.retain(|b| local(b.close_time_utc).unwrap().date() != date("2026-10-23"))
            }
            "missing_bar" => {
                bars.remove(2);
            }
            "conflict" => {
                let mut b = bars[1].clone();
                b.h += 10.;
                bars.push(b);
            }
            "symbol" => bars[1].symbol = "USDRUBF".into(),
            "ohlc" => bars[1].h = 1.,
            "replay" => bars[1].origin = DataOrigin::Replay,
            "future" => bars[1].close_time_utc = epoch("2026-10-26 07:00"),
            _ => {}
        }
        if fault == "future" {
            let b = plan(&bars, 1);
            assert!(b.validate(date("2026-10-26")).is_err());
            continue;
        }
        let (_temp, mut r) = setup(&bars);
        for b in &bars {
            append(&mut r, b);
        }
        if fault == "clock" {
            now(epoch("2026-10-26 07:00"));
        }
        assert!(r.commit(Operation::Bootstrap, clock()).is_err(), "{fault}");
        assert_eq!(r.log.count(), 0, "{fault}");
        assert_eq!(r.state.stores[0].last_sequence(), 0, "{fault}");
    }
}

#[test]
fn reaction_bootstrap_manifest_tamper_and_partial_receipt_restart() {
    let bars = warm_bars("USDRUBF");
    let (temp, mut r) = setup(&bars);
    for b in &bars[..128] {
        append(&mut r, b);
    }
    r.advance().unwrap();
    assert_eq!(r.log.count(), 0);
    r = restart(&temp, r);
    for b in &bars[128..] {
        append(&mut r, b);
    }
    let inputs = r.collector.prefix(bars.len() as u64).unwrap();
    for fault in ["first", "last", "hash", "count", "dates"] {
        let mut changed = r.spec.bootstrap.clone().unwrap();
        match fault {
            "first" => changed.first_source_id = "2-0".into(),
            "last" => changed.last_source_id = "9999-0".into(),
            "hash" => changed.source_sha256 = "0".repeat(64),
            "count" => changed.rows -= 1,
            _ => changed.first_session = date("2026-10-21"),
        }
        assert!(
            changed
                .verify(&inputs, r.spec.first_decision_session)
                .is_err(),
            "{fault}"
        );
    }
    r.commit(Operation::Bootstrap, clock()).unwrap();
    r = restart(&temp, r);
    assert_eq!(r.log.count(), 1);
    assert_eq!(r.collector.journal.last_sequence(), bars.len() as u64);
    let mut changed = r.spec.clone();
    changed.bootstrap.as_mut().unwrap().rows -= 1;
    drop(r);
    assert!(
        ReactionShadowRunner::open_with_clock(&temp.path().join("run"), changed, clock).is_err()
    );
}

#[tokio::test]
#[ignore = "starts owned loopback Redis; explicit bootstrap integration only"]
async fn reaction_bootstrap_readonly_redis_skips_10k_backlog_and_checks_boundaries() {
    use crate::ri_context::ingress::redis_source::tests::TestServer;
    use crate::shadow_session_context::collector::runner::service::{ServiceGuard, ServicePolicy};
    use alor_protocol::{Envelope, MessageType};
    let server = TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("bootstrap")
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
    let mut reader = redis::Client::open(format!(
        "redis://bootstrap:fixture-password@127.0.0.1:{}/",
        server.port
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    let bars = warm_bars("IMOEXF");
    let old = 10_000 - bars.len() as u64;
    let mut pipe = redis::pipe();
    for id in 1..=old {
        // Invalid old payload proves it is neither imported nor parsed.
        pipe.cmd("XADD")
            .arg("fixture:reaction:bars")
            .arg(format!("{id}-0"))
            .arg("payload")
            .arg("obsolete-not-a-bar")
            .ignore();
    }
    for (index, b) in bars.iter().enumerate() {
        pipe.cmd("XADD")
            .arg("fixture:reaction:bars")
            .arg(format!("{}-0", old + index as u64 + 1))
            .arg("payload")
            .arg(serde_json::to_string(&Envelope::new(1, "fixture", MessageType::Bar, b)).unwrap())
            .ignore();
    }
    let _: () = pipe.query_async(&mut admin).await.unwrap();
    now(epoch("2026-10-26 06:00"));
    let mut spec = spec("IMOEXF");
    spec.bootstrap = Some(plan(&bars, old + 1));
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("run");
    let mut r = ReactionShadowRunner::create_with_clock(&root, spec.clone(), clock).unwrap();
    let policy = ServicePolicy {
        startup_timeout_seconds: 600,
        poll_timeout_seconds: 60,
        poll_interval_seconds: 5,
        shutdown_timeout_seconds: 15,
        max_queue_age_seconds: 30,
        max_model_lateness_seconds: 120,
        max_sessions: 20,
    };
    let mut guard = ServiceGuard::new(&r, policy, epoch("2026-10-26 06:59")).unwrap();
    for _ in 0..3 {
        guard.poll_once(&mut r, &mut reader).await.unwrap();
    }
    assert!(guard.status(&r).unwrap().armed);
    assert_eq!(r.status().bootstrap_status, "loaded");
    assert_eq!(r.status().durable_receipts, bars.len() as u64);
    assert_eq!(r.log.count(), 1);
    assert_eq!(r.collector.prefix(1).unwrap()[0].bar, bars[0]);
    let len: u64 = redis::cmd("XLEN")
        .arg("fixture:reaction:bars")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert_eq!(len, 10_000);
    // A first-boundary trim must not affect durable-cursor resume, but it must
    // reject a new generation. A durable-cursor trim blocks both paths.
    let _: u64 = redis::cmd("XDEL")
        .arg("fixture:reaction:bars")
        .arg(&spec.bootstrap.as_ref().unwrap().first_source_id)
        .query_async(&mut admin)
        .await
        .unwrap();
    drop(r);
    let mut r = ReactionShadowRunner::open_with_clock(&root, spec.clone(), clock).unwrap();
    r.poll_once(&mut reader).await.unwrap();
    assert_eq!(r.log.count(), 1);
    let mut fresh =
        ReactionShadowRunner::create_with_clock(&temp.path().join("fresh"), spec.clone(), clock)
            .unwrap();
    assert!(fresh.poll_once(&mut reader).await.is_err());
    assert!(fresh.status().failed);
    let _: u64 = redis::cmd("XDEL")
        .arg("fixture:reaction:bars")
        .arg("10000-0")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert!(r.poll_once(&mut reader).await.is_err());
    assert!(r.status().failed);
    for fault in [
        "changed_payload",
        "missing_middle",
        "malformed",
        "wrong_symbol",
    ] {
        // Only this fixture's PID-verified, ephemeral Redis is mutated by admin.
        let _: u64 = redis::cmd("DEL")
            .arg("fixture:reaction:bars")
            .query_async(&mut admin)
            .await
            .unwrap();
        let mut pipe = redis::pipe();
        for (index, b) in bars.iter().enumerate() {
            if index == 5 && fault == "missing_middle" {
                continue;
            }
            let mut b = b.clone();
            if index == 5 && fault == "changed_payload" {
                b.h += 10.;
            }
            if index == 5 && fault == "wrong_symbol" {
                b.symbol = "USDRUBF".into();
            }
            let raw = if index == 5 && fault == "malformed" {
                "not-an-envelope".into()
            } else {
                serde_json::to_string(&Envelope::new(1, "fixture", MessageType::Bar, b)).unwrap()
            };
            pipe.cmd("XADD")
                .arg("fixture:reaction:bars")
                .arg(format!("{}-0", old + index as u64 + 1))
                .arg("payload")
                .arg(raw)
                .ignore();
        }
        let _: () = pipe.query_async(&mut admin).await.unwrap();
        let mut faulted =
            ReactionShadowRunner::create_with_clock(&temp.path().join(fault), spec.clone(), clock)
                .unwrap();
        for _ in 0..3 {
            if faulted.poll_once(&mut reader).await.is_err() {
                break;
            }
        }
        assert!(faulted.status().failed, "{fault}");
        assert_eq!(faulted.log.count(), 0, "{fault}");
        assert_eq!(faulted.state.stores[0].last_sequence(), 0, "{fault}");
    }
}
