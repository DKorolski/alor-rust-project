use super::*;
use serde_json::json;
use std::cell::Cell;
use std::collections::BTreeSet;

#[cfg(feature = "shadow-oi-http")]
#[path = "oi_tests.rs"]
mod oi;

thread_local! { static NOW: Cell<i64> = const { Cell::new(0) }; }
fn clock() -> i64 {
    NOW.with(Cell::get)
}
fn at(value: i64) {
    NOW.with(|now| now.set(value));
}
fn fixture() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/component_reaction_reference_v1.json"
    ))
    .unwrap()
}
fn spec() -> Spec {
    let f = fixture();
    let calendar_json = serde_json::to_string(&f["definition"]).unwrap();
    Spec {
        ingress: IngressIdentity {
            generation: "component-fixture-one-session".into(),
            receipt_stream: "fixture.component.m10".into(),
            source_stream: "fixture.md.IMOEXF.10m".into(),
            symbol: "IMOEXF".into(),
        },
        date: "2026-11-09".parse().unwrap(),
        calendar_pin: ArtifactPin {
            artifact_sha256: format!("{:x}", Sha256::digest(calendar_json.as_bytes())),
            reference_sha256: f["definition"]["reference_sha256"].as_str().unwrap().into(),
            calendar_version: f["definition"]["calendar_version"].as_str().unwrap().into(),
            symbol: "IMOEXF".into(),
            first_date: "2026-11-05".parse().unwrap(),
            last_date: "2026-11-09".parse().unwrap(),
        },
        calendar_json,
        model_sha256: MODEL.into(),
        source_manifest_sha256: "c".repeat(64),
        max_execution_lag_seconds: 300,
    }
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
fn append(source: &mut RiIngressJournal, b: &BarEvent) {
    let id = format!("1000-{}", source.last_sequence() + 1);
    source
        .append(&id, &serde_json::to_string(b).unwrap())
        .unwrap()
        .input
        .unwrap();
}
fn setup() -> (tempfile::TempDir, Spec, RiIngressJournal, Session) {
    let root = tempfile::tempdir().unwrap();
    let spec = spec();
    let cutoff = spec.calendar().unwrap().bounds_utc(spec.date).unwrap().0;
    at(cutoff - 2400);
    let mut source = RiIngressJournal::create_with_clock(
        &root.path().join("ingress"),
        spec.ingress.clone(),
        clock,
    )
    .unwrap();
    for row in fixture()["histories"]["regular"].as_array().unwrap() {
        append(&mut source, &bar(row, DataOrigin::History));
    }
    let session =
        Session::create_with_clock(&root.path().join("session"), spec.clone(), clock).unwrap();
    (root, spec, source, session)
}
fn freeze(
    source: &mut RiIngressJournal,
    session: &mut Session,
    spec: &Spec,
    oi: Option<&Path>,
) -> Output {
    let cutoff = spec.calendar().unwrap().bounds_utc(spec.date).unwrap().0;
    at(cutoff);
    let barrier = source.seal_cutoff(cutoff).unwrap();
    session.freeze(source, barrier, oi).unwrap()
}

#[test]
fn original_context_pending_fills_outbox_and_terminal_survive_restart_without_oi_reload() {
    let (root, spec, mut source, mut session) = setup();
    let frozen = freeze(&mut source, &mut session, &spec, None);
    assert!(frozen.ready && !frozen.oi_allows_new_bo && !frozen.complete);
    assert!(!frozen.order_emission_allowed && frozen.operational_admission_required);
    let mut all = Vec::new();
    let rows = fixture()["series"]["rounds"].as_array().unwrap().clone();
    for (i, row) in rows.iter().enumerate() {
        let b = bar(row, DataOrigin::Live);
        at(b.close_time_utc + 601);
        append(&mut source, &b);
        let out = session.observe(&source).unwrap().unwrap();
        assert_eq!(out.complete, i + 1 == rows.len());
        all.extend(out.records.clone());
        assert!(session.observe(&source).unwrap().is_none());
        if [0, 1, 5, 18, 36, 60, 99, 100].contains(&i) {
            let number = out.transaction;
            drop(session);
            session = Session::open_with_clock(
                &root.path().join("session"),
                spec.clone(),
                &source,
                clock,
            )
            .unwrap();
            assert_eq!(session.committed_output(number).unwrap(), out);
            assert_eq!(session.committed_output(1).unwrap(), frozen);
        }
    }
    let ids: BTreeSet<_> = all.iter().map(|r| &r.id).collect();
    assert_eq!(ids.len(), all.len());
    for arm in ARMS {
        let actual: Vec<_> = all
            .iter()
            .filter(|r| r.arm == arm && r.kind == "trade")
            .map(|r| r.payload.clone())
            .collect();
        let id = if arm == Arm::ImoAa {
            "regular/rounds/IMO_AA"
        } else if arm == Arm::ImoAb {
            "regular/rounds/IMO_AB"
        } else {
            "regular/rounds/IMO_AB_OI_FALSE"
        };
        let expected = fixture()["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap()["trades"]
            .clone();
        assert_eq!(json!(actual), expected);
    }
}

#[test]
fn nightly_exact_republication_deduplicates_but_selected_hlc_conflict_holds_all_arms() {
    for conflict in [false, true] {
        let (_root, spec, mut source, mut session) = setup();
        let mut repeated = bar(&fixture()["histories"]["regular"][111], DataOrigin::History);
        if conflict {
            repeated.h += 10.;
        }
        append(&mut source, &repeated);
        let out = freeze(&mut source, &mut session, &spec, None);
        assert_eq!(out.ready, !conflict);
        assert_eq!(
            out.hold_reason.as_deref(),
            if conflict {
                Some("selected_context_hlc_revision")
            } else {
                None
            }
        );
        assert_eq!(out.audits.len(), 1);
        assert_eq!(out.audits[0].changed_fields, if conflict { 2 } else { 0 });
    }
}

#[test]
fn context_only_volume_revision_and_late_previous_day_revision_do_not_refreeze_model() {
    let (root, spec, mut source, mut session) = setup();
    let mut changed = bar(&fixture()["histories"]["regular"][10], DataOrigin::History);
    changed.v += 20.;
    append(&mut source, &changed);
    let saved = freeze(&mut source, &mut session, &spec, None);
    assert!(saved.ready);
    at(clock() + 1);
    changed.h += 50.;
    append(&mut source, &changed);
    let out = session.observe(&source).unwrap().unwrap();
    assert!(out.ready && out.records.is_empty() && !out.complete);
    assert_eq!(out.audits.len(), 1);
    assert!(session
        .freeze(&source, source.sealed_barriers().unwrap()[0].clone(), None)
        .is_err());
    drop(session);
    let session =
        Session::open_with_clock(&root.path().join("session"), spec, &source, clock).unwrap();
    assert_eq!(session.committed_output(1).unwrap(), saved);
}

#[test]
fn replay_origin_fake_barrier_gap_and_stale_model_input_never_release_an_outbox() {
    for case in ["fake_barrier", "gap", "stale", "replay"] {
        let (root, spec, mut source, mut session) = setup();
        if case == "fake_barrier" {
            let cutoff = spec.calendar().unwrap().bounds_utc(spec.date).unwrap().0;
            at(cutoff);
            let mut barrier = source.seal_cutoff(cutoff).unwrap();
            barrier.ingress_sha256 = "0".repeat(64);
            assert!(session.freeze(&source, barrier, None).is_err());
            assert!(session.committed_output(1).is_err());
        } else {
            freeze(&mut source, &mut session, &spec, None);
            let b = bar(
                &fixture()["series"]["rounds"][usize::from(case == "gap")],
                if case == "replay" {
                    DataOrigin::Replay
                } else {
                    DataOrigin::Live
                },
            );
            at(b.close_time_utc + if case == "stale" { 901 } else { 601 });
            append(&mut source, &b);
            assert!(session.observe(&source).is_err());
            assert!(session.committed_output(2).is_err());
        }
        assert!(root.path().join("session/FAILED.json").exists());
        assert!(session.observe(&source).is_err());
    }
}

#[test]
fn late_start_manifest_change_parallel_writer_and_uncommitted_tail_are_rejected() {
    let (root, spec, mut source, mut session) = setup();
    assert!(
        Session::open_with_clock(&root.path().join("session"), spec.clone(), &source, clock)
            .is_err()
    );
    freeze(&mut source, &mut session, &spec, None);
    assert!(Session::create_with_clock(&root.path().join("late"), spec.clone(), clock).is_err());
    drop(session);
    let mut wrong = spec.clone();
    wrong.source_manifest_sha256 = "d".repeat(64);
    assert!(Session::open_with_clock(&root.path().join("session"), wrong, &source, clock).is_err());
    std::fs::copy(
        root.path().join("session/00000001.json"),
        root.path().join("session/00000002.json"),
    )
    .unwrap();
    assert!(Session::open_with_clock(&root.path().join("session"), spec, &source, clock).is_err());
}

#[test]
fn rehashed_operator_edit_cannot_replace_an_original_receipt_replay_witness() {
    let (root, spec, mut source, mut session) = setup();
    freeze(&mut source, &mut session, &spec, None);
    drop(session);
    let path = root.path().join("session/00000001.json");
    let checked: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut tx: Transaction = serde_json::from_value(checked["payload"].clone()).unwrap();
    tx.output.oi_allows_new_bo = true;
    let sha = hash(&tx).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&json!({"payload": tx, "sha256":sha})).unwrap(),
    )
    .unwrap();
    let head = json!({"index":1,"sha256":sha});
    std::fs::write(
        root.path().join("session/HEAD.json"),
        serde_json::to_vec(&json!({"sha256":hash(&head).unwrap(),"payload":head})).unwrap(),
    )
    .unwrap();
    assert!(Session::open_with_clock(&root.path().join("session"), spec, &source, clock).is_err());
}

#[test]
fn one_arm_rejecting_a_bar_cannot_advance_or_release_the_other_arms() {
    let (_root, spec, mut source, mut session) = setup();
    freeze(&mut source, &mut session, &spec, None);
    let b = bar(&fixture()["series"]["rounds"][0], DataOrigin::Live);
    at(b.close_time_utc + 601);
    append(&mut source, &b);
    // Inject a divergent in-memory arm so A/A steps first and A/B then fails.
    let mut simulated = b.clone();
    simulated.origin = DataOrigin::Replay;
    session.state.engines.as_mut().unwrap()[1]
        .observe(simulated, clock())
        .unwrap();
    assert!(session.observe(&source).is_err());
    assert_eq!(
        session.state.engines.as_ref().unwrap()[0]
            .state()
            .observed_bars,
        0
    );
    assert_eq!(
        session.state.engines.as_ref().unwrap()[2]
            .state()
            .observed_bars,
        0
    );
    assert!(session.committed_output(2).is_err());
    let status = session.status();
    assert!(status.failed && !status.ready && !status.order_emission_allowed);
    assert_eq!(status.committed_transactions, 1);
}

#[test]
fn service_quote_revisions_stay_audit_only_and_repeated_model_bars_never_duplicate_fills() {
    let (_root, spec, mut source, mut session) = setup();
    let mut service = bar(&fixture()["histories"]["regular"][0], DataOrigin::History);
    service.close_time_utc = (spec
        .date
        .pred_opt()
        .unwrap()
        .pred_opt()
        .unwrap()
        .pred_opt()
        .unwrap()
        .and_hms_opt(6, 50, 0)
        .unwrap()
        - Duration::hours(3))
    .and_utc()
    .timestamp();
    append(&mut source, &service);
    service.h += 10.;
    append(&mut source, &service);
    let frozen = freeze(&mut source, &mut session, &spec, None);
    assert!(frozen.ready);
    let b = bar(&fixture()["series"]["rounds"][0], DataOrigin::Live);
    at(b.close_time_utc + 601);
    append(&mut source, &b);
    session.observe(&source).unwrap().unwrap();
    let b = bar(&fixture()["series"]["rounds"][1], DataOrigin::Live);
    at(b.close_time_utc + 601);
    append(&mut source, &b);
    let original = session.observe(&source).unwrap().unwrap();
    assert!(original.records.iter().any(|r| r.kind == "event"));
    at(clock() + 1);
    append(&mut source, &b);
    let repeated = session.observe(&source).unwrap().unwrap();
    assert!(repeated.records.is_empty() && repeated.audits.len() == 1);
    assert_eq!(
        session.state.engines.as_ref().unwrap()[0]
            .state()
            .observed_bars,
        2
    );
}

#[test]
fn previous2_uses_only_terminal_close_not_intraday_close_or_high_low() {
    for terminal in [false, true] {
        let (_root, spec, mut source, mut session) = setup();
        let rows = fixture()["histories"]["regular"]
            .as_array()
            .unwrap()
            .clone();
        let previous2: Vec<_> = rows
            .iter()
            .filter(|r| local(r[0].as_i64().unwrap()).unwrap().date().to_string() == "2026-11-05")
            .collect();
        let row = if terminal {
            *previous2.last().unwrap()
        } else {
            previous2[10]
        };
        let mut revised = bar(row, DataOrigin::History);
        revised.h += 10.;
        revised.close += 1.;
        append(&mut source, &revised);
        let out = freeze(&mut source, &mut session, &spec, None);
        assert_eq!(out.ready, !terminal);
        assert_eq!(out.audits.len(), 1);
    }
}
