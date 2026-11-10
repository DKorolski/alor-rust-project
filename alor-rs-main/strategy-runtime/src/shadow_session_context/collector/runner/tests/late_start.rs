use super::*;
use crate::shadow_session_context::collector::runner::{
    bootstrap::HistoricalBootstrap,
    late_start::{LateStart, RecoveryPrefix},
    service::ServiceGuard,
};

fn pinned(bars: &[BarEvent]) -> (HistoricalBootstrap, RecoveryPrefix) {
    let n = bars
        .iter()
        .take_while(|b| local(b.close_time_utc).unwrap().date() < date("2026-10-26"))
        .count();
    let digest = |items: &[BarEvent]| {
        hash(
            &items
                .iter()
                .enumerate()
                .map(|(i, b)| (format!("{}-0", i + 1), b))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    (
        HistoricalBootstrap {
            first_source_id: "1-0".into(),
            last_source_id: format!("{n}-0"),
            rows: n as u64,
            source_sha256: digest(&bars[..n]),
            first_session: date("2026-10-22"),
            last_session: date("2026-10-25"),
        },
        RecoveryPrefix {
            last_source_id: format!("{}-0", bars.len()),
            rows: bars.len() as u64,
            source_sha256: digest(bars),
            last_label_utc: bars.last().unwrap().close_time_utc,
        },
    )
}

fn bars(symbol: &str) -> Vec<BarEvent> {
    let mut bars = warm_bars(symbol);
    bars.extend((0..25).map(|slot| {
        bar(
            symbol,
            epoch("2026-10-26 07:00") + slot * 600,
            if slot == 24 { 105. } else { 100. },
        )
    }));
    bars
}

fn setup(symbol: &str) -> (tempfile::TempDir, ReactionShadowRunner) {
    let bars = bars(symbol);
    let (bootstrap, recovery) = pinned(&bars);
    let mut spec = spec(symbol);
    spec.bootstrap = Some(bootstrap);
    spec.late_start = Some(LateStart {
        version: "diagnostic_late_start_v1".into(),
        source_first_live_session: date("2026-10-27"),
        first_clean_session: date("2026-10-27"),
        recovery,
    });
    now(epoch("2026-10-26 11:10"));
    let temp = tempfile::tempdir().unwrap();
    let mut r =
        ReactionShadowRunner::create_with_clock(&temp.path().join("run"), spec, clock).unwrap();
    for b in bars {
        append(&mut r, &b);
    }
    (temp, r)
}

fn advance_owned_prefix(r: &mut ReactionShadowRunner) -> Result<()> {
    let late = r.spec.late_start.as_ref().unwrap();
    late.recovery
        .verify(&r.collector.prefix(late.recovery.rows)?, &r.spec, clock())?;
    r.commit(Operation::Bootstrap, clock())?;
    ingest(r);
    let raw = r.collector.journal.seal_cutoff(clock())?;
    let cutoff = r.collector.project_cutoff(&raw)?;
    r.commit(
        Operation::Freeze {
            date: date("2026-10-26"),
            cutoff: Box::new(cutoff),
        },
        clock(),
    )?;
    observe(r)
}

#[test]
fn late_start_reconstructs_both_models_pending_and_preserves_actual_receipt_clocks() {
    for symbol in ["IMOEXF", "USDRUBF"] {
        let (_control_dir, mut control) = create(symbol);
        warm(&mut control);
        for slot in 0..25 {
            live_step(&mut control, slot, if slot == 24 { 105. } else { 100. }).unwrap();
        }
        let expected = control.state.states().unwrap().unwrap();
        let (dir, mut r) = setup(symbol);
        advance_owned_prefix(&mut r).unwrap();
        let actual = r.state.states().unwrap().unwrap();
        for arm in 0..2 {
            assert_eq!(actual[arm]["model"], expected[arm]["model"]);
            assert!(
                !actual[arm]["model"]["state"]["pending"].is_null(),
                "fixture must retain pending ownership: {symbol}"
            );
            assert_eq!(
                actual[arm]["frozen"]["diagnostic_as_observed_at_utc"],
                clock()
            );
            let restored = Store::restore(
                r.state.stores[arm].identity(),
                &r.state.stores[arm].checkpoint_bytes().unwrap(),
                clock(),
            )
            .unwrap();
            assert_eq!(
                restored.frozen(date("2026-10-26")),
                r.state.stores[arm].frozen(date("2026-10-26"))
            );
        }
        let mut replayed = 0;
        for i in 1..=r.log.count() {
            let output = r.committed_output(i).unwrap();
            assert!(!output.order_emission_allowed);
            assert!(
                !output
                    .diagnostic_late_start
                    .unwrap()
                    .current_session_clean_eligible
            );
            replayed += output
                .steps
                .iter()
                .filter(|s| s.disposition == Disposition::DiagnosticReplayed)
                .count();
        }
        assert_eq!(replayed, 25);
        assert!(r
            .collector
            .prefix(r.collector.journal.last_sequence())
            .unwrap()
            .iter()
            .all(|r| r.received_at_utc == Some(clock()) && r.durable_at_utc == Some(clock())));
        assert!(
            ServiceGuard::new(&r, super::service::policy(), clock() + 600).is_err(),
            "partial-session resume must remain forbidden"
        );
        restart(&dir, r);
    }
}

#[test]
fn late_start_does_not_waive_new_bar_age_or_accept_changed_recovery() {
    let (_dir, mut r) = setup("USDRUBF");
    let guard = ServiceGuard::new(&r, super::service::policy(), clock() + 600).unwrap();
    assert_eq!(
        guard
            .status(&r)
            .unwrap()
            .max_unobserved_model_lateness_seconds,
        0
    );
    let late = r.spec.late_start.as_ref().unwrap();
    let mut changed = r.collector.prefix(late.recovery.rows).unwrap();
    changed.last_mut().unwrap().bar.close += 0.01;
    assert!(late.recovery.verify(&changed, &r.spec, clock()).is_err());
    advance_owned_prefix(&mut r).unwrap();
    let next = bar("USDRUBF", epoch("2026-10-26 11:10"), 105.);
    now(next.close_time_utc + 1021);
    append(&mut r, &next);
    let status = guard.status(&r).unwrap();
    assert!(guard.check_model_ages(&status).is_err());
    assert_eq!(status.clean_sessions_completed, Some(0));
    assert_eq!(status.started_sessions, 0);
}

#[test]
fn late_start_partial_eod_is_not_a_clean_session_and_next_day_is_strict() {
    let (_dir, mut r) = setup("USDRUBF");
    let mut policy = super::service::policy();
    policy.max_sessions = 1;
    let guard = ServiceGuard::new(&r, policy, clock() + 600).unwrap();
    advance_owned_prefix(&mut r).unwrap();
    for slot in 25..100 {
        live_step(&mut r, slot, 105.).unwrap();
    }
    let mut terminal = bar("USDRUBF", epoch("2026-10-26 23:40"), 105.);
    terminal.origin = DataOrigin::History;
    now(terminal.close_time_utc + 610);
    let mut raw = serde_json::to_value(&terminal).unwrap();
    raw["source_proof"] = serde_json::json!({"kind":"as_observed_ws","release_id":"owned-terminal", "ws_sequence_at_cutoff":100,
        "release":{"generation":1,"closed_at_utc":terminal.close_time_utc+600,"reason":"session_boundary_ws", "next_label_utc":null,
            "observation_cutoff_utc":clock(),"first_received_at_utc":terminal.close_time_utc,"last_received_at_utc":terminal.close_time_utc+599}});
    let seq = r.collector.journal.last_sequence() + 1;
    r.collector
        .journal
        .append(&format!("{seq}-0"), &raw.to_string())
        .unwrap();
    ingest(&mut r);
    observe(&mut r).unwrap();
    assert_eq!(
        r.committed_output(r.log.count())
            .unwrap()
            .steps
            .last()
            .unwrap()
            .disposition,
        Disposition::DiagnosticForwardObserved
    );
    let status = guard.status(&r).unwrap();
    assert!(status.runner.session_complete);
    assert!(!status.completed && status.last_complete_session.is_none());
    assert_eq!(status.clean_sessions_completed, Some(0));
    now(epoch("2026-10-27 07:00"));
    freeze(&mut r, "2026-10-27").unwrap();
    for slot in 0..101 {
        let input = bar("USDRUBF", epoch("2026-10-27 07:00") + slot * 600, 105.);
        now(input.close_time_utc + 600);
        append(&mut r, &input);
        ingest(&mut r);
        observe(&mut r).unwrap();
    }
    let status = guard.status(&r).unwrap();
    assert!(
        status.completed
            && status
                .diagnostic_late_start
                .unwrap()
                .current_session_clean_eligible
    );
    assert_eq!(status.started_sessions, 1);
    assert_eq!(status.clean_sessions_completed, Some(1));
    assert!(r.state.stores[0]
        .frozen(date("2026-10-27"))
        .unwrap()
        .diagnostic_as_observed_at_utc
        .is_none());
}

#[test]
fn strict_creation_after_cutoff_and_diagnostic_invalid_clean_date_are_blocked() {
    now(epoch("2026-10-26 11:10"));
    let dir = tempfile::tempdir().unwrap();
    assert!(ReactionShadowRunner::create_with_clock(
        &dir.path().join("strict"),
        spec("USDRUBF"),
        clock
    )
    .is_err());
    let (_dir, mut r) = setup("USDRUBF");
    r.spec.late_start.as_mut().unwrap().first_clean_session = date("2026-10-26");
    assert!(r.spec.header().is_err());
}

#[test]
fn late_start_history_forward_requires_original_ws_proof_and_preserves_age_guard() {
    for (proof, reason) in [
        (false, "next_timestamp"),
        (true, "next_timestamp"),
        (true, "next_timestamp_delayed"),
    ] {
        let (_dir, mut r) = setup("USDRUBF");
        let guard = ServiceGuard::new(&r, super::service::policy(), clock() + 600).unwrap();
        advance_owned_prefix(&mut r).unwrap();
        let mut next = bar("USDRUBF", epoch("2026-10-26 11:10"), 105.);
        next.origin = DataOrigin::History;
        now(next.close_time_utc + 699);
        let mut payload = serde_json::to_value(&next).unwrap();
        if proof {
            payload["source_proof"] = serde_json::json!({"kind":"as_observed_ws","release_id":"owned-test", "ws_sequence_at_cutoff":10,
            "release":{"generation":1,"closed_at_utc":next.close_time_utc+600,"reason":reason, "next_label_utc":next.close_time_utc+600,
                "observation_cutoff_utc":next.close_time_utc+695,"first_received_at_utc":next.close_time_utc,"last_received_at_utc":next.close_time_utc+695}});
        }
        let seq = r.collector.journal.last_sequence() + 1;
        r.collector
            .journal
            .append(&format!("{seq}-0"), &payload.to_string())
            .unwrap();
        guard.check_model_ages(&guard.status(&r).unwrap()).unwrap();
        ingest(&mut r);
        let before = r.state.model_hash().unwrap();
        if proof {
            observe(&mut r).unwrap();
            let output = r.committed_output(r.log.count()).unwrap();
            assert_eq!(
                output.steps.last().unwrap().disposition,
                Disposition::DiagnosticForwardObserved
            );
            assert_eq!(
                output.steps.last().unwrap().receipt.bar.origin,
                DataOrigin::History
            );
            assert!(
                !output
                    .diagnostic_late_start
                    .unwrap()
                    .current_session_clean_eligible
            );
        } else {
            assert!(observe(&mut r).is_err());
            assert_eq!(r.state.model_hash().unwrap(), before);
        }
        // A post-boundary History label cannot evade the ordinary model age guard.
        let mut stale = next.clone();
        stale.close_time_utc += 600;
        now(stale.close_time_utc + 721);
        append(&mut r, &stale);
        assert!(guard.check_model_ages(&guard.status(&r).unwrap()).is_err());
    }
}
