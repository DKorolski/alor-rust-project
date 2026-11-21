use super::*;
use std::collections::BTreeMap;

use crate::shadow_calendar::{Calendar, Definition};
use crate::shadow_component_context::{self as context, Request};
use crate::shadow_session_context::{Barrier, Receipt};
use serde_json::json;

#[path = "history_tests.rs"]
mod history;

#[derive(Deserialize)]
struct Fixture {
    schema_version: String,
    evidence_kind: String,
    source_hashes: BTreeMap<String, String>,
    definition: Definition,
    histories: BTreeMap<String, Vec<[f64; 6]>>,
    series: BTreeMap<String, Vec<[f64; 6]>>,
    state_columns: Vec<String>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    id: String,
    context: String,
    series: String,
    arm: Arm,
    gate: Option<bool>,
    ready: bool,
    mr_previous: String,
    mr_previous2: String,
    bo_previous: String,
    bo_previous2: String,
    mr_inputs: Option<Vec<f64>>,
    bo_inputs: Option<Vec<f64>>,
    expected_state_trace: Vec<Vec<Value>>,
    events: Vec<Value>,
    trades: Vec<Value>,
}
fn fixture() -> Fixture {
    serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/component_reaction_reference_v1.json"
    ))
    .unwrap()
}
fn bar(row: &[f64; 6]) -> BarEvent {
    BarEvent {
        symbol: "IMOEXF".into(),
        close_time_utc: row[0] as i64,
        o: row[1],
        h: row[2],
        l: row[3],
        close: row[4],
        v: row[5],
        origin: DataOrigin::Replay,
    }
}
fn prepare(f: &Fixture, c: &Case) -> FrozenComponents {
    let cal = Calendar::new(f.definition.clone()).unwrap();
    let cutoff = cal.bounds_utc(f.definition.last_date).unwrap().0;
    let receipts: Vec<_> = f.histories[&c.context]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let b = bar(row);
            Receipt {
                sequence: i as u64 + 1,
                source_id: format!("fixture-{}", b.close_time_utc),
                received_at_utc: Some(b.close_time_utc + 600),
                durable_at_utc: Some(b.close_time_utc + 600),
                bar: b,
            }
        })
        .collect();
    let request = Request {
        arm: c.arm,
        model_sha256: f.source_hashes["model"].clone(),
        receipt_stream: "synthetic-component-reaction-reference-v1".into(),
        evidence: Evidence::NumericalFixture,
    };
    let barrier = Barrier {
        receipt_stream: request.receipt_stream.clone(),
        through_utc: cutoff,
        last_sequence: receipts.len() as u64,
        ingress_sha256: context::prefix_sha256(&receipts).unwrap(),
        attested_at_utc: cutoff,
    };
    context::freeze(
        &cal,
        request,
        f.definition.last_date,
        cutoff,
        &receipts,
        &barrier,
    )
    .unwrap()
}
fn gate(c: &Case) -> BoGate {
    c.gate
        .map_or(BoGate::Off, |allow_new_bo| BoGate::FrozenNumerical {
            allow_new_bo,
        })
}
fn row(s: &State) -> Vec<Value> {
    let p = s.position.as_ref();
    let a = s.pending.as_ref();
    vec![
        json!(s.last_label_utc),
        json!(s.observed_bars),
        json!(s.mr_entries),
        json!(s.used_long),
        json!(s.used_short),
        json!(s.day_high),
        json!(s.day_low),
        json!(s.first_high),
        json!(s.first_low),
        json!(s.bo_long_level),
        json!(s.bo_short_level),
        json!(s.author_allowed),
        json!(p.map(|p| p.entry.owner)),
        json!(p.map(|p| p.entry.side)),
        json!(p.map(|p| p.entry.signal_ts_utc)),
        json!(p.map(|p| p.entry.at_utc)),
        json!(p.map(|p| p.price)),
        json!(p.map(|p| p.bars_held)),
        json!(a.map(|a| if a.exit_reason.is_some() {
            "exit"
        } else {
            "entry"
        })),
        json!(a.map(|a| a.owner)),
        json!(a.map(|a| a.side)),
        json!(a.map(|a| a.signal_ts_utc)),
        json!(a.map(|a| a.at_utc)),
        json!(a.and_then(|a| a.exit_reason)),
        json!(s.decision_site),
        json!(s.mr_predicate),
        json!(s.bo_long_predicate),
        json!(s.bo_short_predicate),
    ]
}
fn close(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            if a.is_i64() && b.is_i64() {
                assert_eq!(a, b, "{path}");
            } else {
                assert!(
                    (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() <= 1e-9,
                    "{path}: {a} vs {b}"
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                close(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{path}"
            );
            for (k, a) in a {
                close(a, &b[k], &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

#[test]
fn pinned_reference_matches_independent_features_all_events_trades_and_each_bar_state() {
    let f = fixture();
    assert_eq!(f.schema_version, "imoexf_component_reaction_fixture_v1");
    assert_eq!(
        f.evidence_kind,
        "synthetic_pinned_reference_not_prospective_pit"
    );
    assert_eq!(
        f.source_hashes["model"],
        "a469365d54844879ed63108bc99d45be18ace95d261627b57a94ceb1a4c18135"
    );
    assert_eq!(
        f.source_hashes["compiler"],
        "c68fe4b11bcf19eda6a8b38469470ad5594101647435c3ecf96b7a0730fec4f4"
    );
    assert_eq!(f.cases.len(), 50);
    assert_eq!(f.state_columns.len(), 28);
    let mut states = 0;
    for c in &f.cases {
        let frozen = prepare(&f, c);
        assert_eq!(frozen.dependency_ready, c.ready, "{} readiness", c.id);
        for (actual, expected) in [
            (frozen.mr.selection.previous, &c.mr_previous),
            (frozen.mr.selection.previous2, &c.mr_previous2),
            (frozen.bo.selection.previous, &c.bo_previous),
            (frozen.bo.selection.previous2, &c.bo_previous2),
        ] {
            assert_eq!(actual.unwrap().to_string(), *expected, "{} dates", c.id);
        }
        for (component, expected) in [(&frozen.mr, &c.mr_inputs), (&frozen.bo, &c.bo_inputs)] {
            let inputs = component.inputs.as_ref().map(|i| {
                vec![
                    i.previous_close,
                    i.previous_high,
                    i.previous_low,
                    i.previous_range,
                    i.previous2_close,
                    i.previous_return,
                ]
            });
            close(
                &json!(inputs),
                &json!(expected),
                &format!("{} features", c.id),
            );
        }
        if !c.ready {
            assert!(Engine::new(frozen, gate(c)).is_err());
            continue;
        }
        let mut e = Engine::new(frozen, gate(c)).unwrap();
        assert!(!e.is_complete());
        assert_eq!(f.series[&c.series].len(), c.expected_state_trace.len());
        for (i, (b, expected)) in f.series[&c.series]
            .iter()
            .zip(&c.expected_state_trace)
            .enumerate()
        {
            let b = bar(b);
            let now = b.close_time_utc + 600;
            e.observe(b, now).unwrap();
            let actual = row(e.state());
            assert_eq!(actual.len(), f.state_columns.len());
            assert_eq!(expected.len(), f.state_columns.len());
            assert_eq!(e.is_complete(), i + 1 == f.series[&c.series].len());
            for (column, (a, b)) in f.state_columns.iter().zip(actual.iter().zip(expected)) {
                close(a, b, &format!("{}/{i}/{column}", c.id));
            }
            states += 1;
        }
        close(
            &json!(e.events()),
            &json!(c.events),
            &format!("{} events", c.id),
        );
        close(
            &json!(e.trades()),
            &json!(c.trades),
            &format!("{} trades", c.id),
        );
        assert!(e.is_complete(), "{} no overnight", c.id);
    }
    assert_eq!(states, 4646);
}

#[test]
fn restart_reexecutes_original_context_and_gate_without_duplicate_fill_or_event() {
    let f = fixture();
    let c = f
        .cases
        .iter()
        .find(|c| c.id == "regular/rounds/IMO_AB")
        .unwrap();
    let frozen = prepare(&f, c);
    let mut uninterrupted = Engine::new(frozen.clone(), gate(c)).unwrap();
    let mut resumed = uninterrupted.clone();
    for (i, b) in f.series[&c.series].iter().enumerate() {
        let b = bar(b);
        let now = b.close_time_utc + 600;
        uninterrupted.observe(b.clone(), now).unwrap();
        resumed.observe(b, now).unwrap();
        if [0, 1, 2, 5, 6, 18, 36, 48, 60, 99, 100].contains(&i) {
            resumed = Engine::restore(
                frozen.clone(),
                gate(c),
                &resumed.checkpoint_bytes().unwrap(),
            )
            .unwrap();
        }
        assert_eq!(resumed.state(), uninterrupted.state());
        assert_eq!(resumed.events(), uninterrupted.events());
        assert_eq!(resumed.trades(), uninterrupted.trades());
    }
}

#[test]
fn missing_duplicate_or_invalid_bar_fails_atomically_with_pending_entry_retained() {
    let f = fixture();
    let c = &f.cases[0];
    let mut e = Engine::new(prepare(&f, c), gate(c)).unwrap();
    let rows = &f.series[&c.series];
    let first = bar(&rows[0]);
    e.observe(first.clone(), first.close_time_utc + 600)
        .unwrap();
    let before = e.checkpoint_bytes().unwrap();
    let gap = bar(&rows[2]);
    assert!(e.observe(gap.clone(), gap.close_time_utc + 600).is_err());
    assert!(e
        .observe(first.clone(), first.close_time_utc + 600)
        .is_err());
    let mut invalid = bar(&rows[1]);
    invalid.h = invalid.l - 1.;
    assert!(e
        .observe(invalid.clone(), invalid.close_time_utc + 600)
        .is_err());
    assert_eq!(e.checkpoint_bytes().unwrap(), before);
}

#[test]
fn valid_hash_does_not_hide_forged_witness_or_changed_gate() {
    let f = fixture();
    let c = f
        .cases
        .iter()
        .find(|c| c.id == "regular/rounds/IMO_AB_OI_FALSE")
        .unwrap();
    let frozen = prepare(&f, c);
    let mut e = Engine::new(frozen.clone(), gate(c)).unwrap();
    let b = bar(&f.series[&c.series][0]);
    e.observe(b.clone(), b.close_time_utc + 600).unwrap();
    let bytes = e.checkpoint_bytes().unwrap();
    assert!(Engine::restore(frozen.clone(), BoGate::Off, &bytes).is_err());
    let mut envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    envelope.payload.witness[0]["mr_entries"] = json!(99);
    envelope.sha256 = hash(&envelope.payload).unwrap();
    assert!(Engine::restore(frozen, gate(c), &serde_json::to_vec(&envelope).unwrap()).is_err());
}

#[test]
fn closed_numerical_oi_gate_keeps_mr_and_sixth_bar_observation_identical() {
    let f = fixture();
    let find = |id: &str| f.cases.iter().find(|c| c.id == id).unwrap();
    let control = find("regular/mr_hold/IMO_AB");
    let closed = find("regular/mr_hold/IMO_AB_OI_FALSE");
    let mut a = Engine::new(prepare(&f, control), gate(control)).unwrap();
    let mut b = Engine::new(prepare(&f, closed), gate(closed)).unwrap();
    for (i, data) in f.series["mr_hold"].iter().enumerate() {
        let bar = bar(data);
        let now = bar.close_time_utc + 600;
        a.observe(bar.clone(), now).unwrap();
        b.observe(bar, now).unwrap();
        assert_eq!(a.state.bo_long_level, b.state.bo_long_level);
        assert_eq!(a.state.bo_short_level, b.state.bo_short_level);
        if i == 5 {
            assert!(a.state.bo_long_level.is_some() && a.state.position.is_some());
        }
    }
    let mr = |e: &Engine| {
        e.trades
            .iter()
            .filter(|t| t.component == Owner::Mr)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(mr(&a), mr(&b));
    assert_eq!(mr(&a)[0].exit_reason, "time_exit");
    assert_eq!(
        (
            local(mr(&a)[0].exit_ts_utc).unwrap().hour(),
            local(mr(&a)[0].exit_ts_utc).unwrap().minute()
        ),
        (20, 10)
    );
}

#[test]
fn shortened_or_break_calendar_and_arm_policy_cannot_silently_change_execution() {
    let f = fixture();
    let c = &f.cases[0];
    let original = prepare(&f, c);
    let mut shortened = original.clone();
    shortened.session_close_utc -= 3600;
    assert!(Engine::new(shortened, gate(c)).is_err());
    let mut gap = original.clone();
    gap.expected_execution_labels_utc.remove(20);
    assert!(Engine::new(gap, gate(c)).is_err());
    let mut wrong = original;
    wrong.mr.selection.policy = ContextPolicy::RegularAndDsvd;
    assert!(Engine::new(wrong, gate(c)).is_err());
}

#[test]
fn ambiguous_mr_bar_keeps_stop_priority_and_breakeven_is_a_next_open_exit() {
    let f = fixture();
    for (series, reason) in [("mr_ambiguous", "stop"), ("breakeven", "breakeven_limit")] {
        let c = f
            .cases
            .iter()
            .find(|c| c.id == format!("regular/{series}/IMO_AA"))
            .unwrap();
        let mut e = Engine::new(prepare(&f, c), gate(c)).unwrap();
        for row in &f.series[series] {
            let b = bar(row);
            let now = b.close_time_utc + 600;
            e.observe(b, now).unwrap();
        }
        let t = &e.trades()[0];
        assert_eq!(t.exit_reason, reason);
        assert_eq!(t.exit_ts_utc, t.exit_signal_ts_utc + 600);
    }
}

#[test]
fn observed_oi_requires_a_bound_gate_and_checkpoint_cannot_mint_permission() {
    let f = fixture();
    let c = f
        .cases
        .iter()
        .find(|c| c.id == "regular/rounds/IMO_AB_OI_FALSE")
        .unwrap();
    let mut frozen = prepare(&f, c);
    frozen.request.evidence = Evidence::DeclaredAsObserved;
    assert!(Engine::new(frozen.clone(), BoGate::Off).is_err());
    assert!(Engine::new(frozen.clone(), gate(c)).is_err());
    let observed = BoGate::observed(&frozen, false, "e".repeat(64)).unwrap();
    let e = Engine::new(frozen.clone(), observed.clone()).unwrap();
    let bytes = e.checkpoint_bytes().unwrap();
    let mut envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    envelope.payload.gate["binding"]["allow_new_bo"] = json!(true);
    envelope.sha256 = hash(&envelope.payload).unwrap();
    assert!(Engine::restore(
        frozen.clone(),
        observed.clone(),
        &serde_json::to_vec(&envelope).unwrap()
    )
    .is_err());
    let mut envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    envelope.payload.version = "imoexf_component_reaction_v1".into();
    envelope.sha256 = hash(&envelope.payload).unwrap();
    assert!(Engine::restore(
        frozen.clone(),
        observed.clone(),
        &serde_json::to_vec(&envelope).unwrap()
    )
    .is_err());
    frozen.frozen_at_utc += 1;
    assert!(Engine::new(frozen, observed).is_err());
}
