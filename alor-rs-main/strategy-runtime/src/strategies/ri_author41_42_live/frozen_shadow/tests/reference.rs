//! Test-only numerical export. Never import synthetic receipts into a deployment.
use std::{fs, io::Write};

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixtures {
    schema_version: String,
    evidence_kind: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    day: NaiveDate,
    warmup: Vec<BarEvent>,
    events: Vec<BarEvent>,
}

pub(super) fn project(out: &ShadowStep) -> serde_json::Value {
    let s = &out.state_trace;
    let phase = s["strategy_state"]["RiAuthor41_42Live"]["phase"]
        .as_str()
        .expect("RI strategy phase must be present");
    let anchor = |key: &str| {
        let a = &s["frozen_context"][key];
        if a.is_null() {
            serde_json::Value::Null
        } else {
            serde_json::json!({"date": a["session_date"], "high": a["high"], "low": a["low"],
                "close": a["close"], "range": a["range"], "bars": a["distinct_bars"]})
        }
    };
    let position = |key: &str| {
        let mut p = s[key].clone();
        if let Some(p) = p.as_object_mut() {
            p.remove("decision_key");
        }
        p
    };
    let actions: Vec<_> = out
        .records
        .iter()
        .filter(|r| r.adapter_decision == RiAuthor4142JournalDecision::ProspectiveIntentSuppressed)
        .map(|r| {
            serde_json::json!({"component": r.component, "role": r.role, "side": r.side,
            "reason": r.entry_exit_reason, "scheduled_ts": r.candidate_scheduled_ts_local,
            "pnl": r.shadow_pnl_points})
        })
        .collect();
    serde_json::json!({
        "context": {"previous": anchor("previous"), "previous2": anchor("previous2")},
        "phase": phase, "mr_total": s["mr_entries_today_total"],
        "lifecycle_owner": s["strategy_state"]["RiAuthor41_42Live"]["current_component"],
        "lifecycle_side": s["strategy_state"]["RiAuthor41_42Live"]["current_side"],
        "lifecycle_entry_ts": s["strategy_state"]["RiAuthor41_42Live"]["current_entry_ts_local"],
        "bo_total": s["bo_entries_today"], "mr": position("mr_position"), "bo": position("bo_position"),
        "pending": if s["bo_pending"].is_null() { serde_json::Value::Null } else {
            serde_json::json!({"kind": s["bo_pending"]["kind"], "side": s["bo_pending"]["side"],
                "reason": s["bo_pending"]["reason"]})
        }, "observation_count": s["bo_observation_index"],
        "first_bar": s["bo_first_bar"], "long_level": s["bo_long_level"], "short_level": s["bo_short_level"],
        "day_high": s["bo_day_high"], "day_low": s["bo_day_low"], "trade_allowed": s["bo_trade_allowed"],
        "long_used": s["bo_was_long_today"], "short_used": s["bo_was_short_today"],
        "branches": s["branches"], "actions": actions,
    })
}

fn run(case: &Case, policy: ContextPolicy) -> Vec<serde_json::Value> {
    assert!(!case.events.is_empty() && case.events.len() <= 1000 && case.warmup.len() <= 1000);
    let identity = context_identity(&config(), "synthetic-state-fixtures".into(), policy).unwrap();
    let mut store = RiContextStore::new(identity.clone()).unwrap();
    let generation = ShadowGeneration {
        id: case.id.clone(),
        first_decision_session: case.day,
    };
    let mut a = RiFrozenShadowAdapter::new(config(), identity.clone(), generation.clone()).unwrap();
    let mut inputs = Vec::new();
    for b in &case.warmup {
        assert!(local(b.close_time_utc).unwrap().date() < case.day);
        let i = ContextInput {
            sequence: 0,
            bar: b.clone(),
            received_at_utc: Some(b.close_time_utc + 600),
            durable_at_utc: Some(b.close_time_utc + 600),
        };
        add(&mut store, &mut inputs, i);
        assert!(a
            .process(&store, inputs.last().unwrap())
            .unwrap()
            .records
            .is_empty());
    }
    freeze(
        &mut store,
        &case.day.to_string(),
        epoch(&format!("{} 07:00", case.day)),
    );
    let mut untraced = a.clone();
    untraced.strategy.shadow_branches = None;
    let mut rows = Vec::new();
    for (index, b) in case.events.iter().enumerate() {
        assert_eq!(local(b.close_time_utc).unwrap().date(), case.day);
        let i = ContextInput {
            sequence: 0,
            bar: b.clone(),
            received_at_utc: Some(b.close_time_utc + 600),
            durable_at_utc: Some(b.close_time_utc + 600),
        };
        add(&mut store, &mut inputs, i);
        let out = a.process(&store, inputs.last().unwrap()).unwrap();
        let mut muted = untraced.process(&store, inputs.last().unwrap()).unwrap();
        muted.state_trace["branches"] = out.state_trace["branches"].clone();
        assert_eq!(out, muted, "tracing changed numerical/lifecycle output");
        assert!(out
            .records
            .iter()
            .all(|r| r.adapter_decision != RiAuthor4142JournalDecision::IntentEmitted));
        rows.push(
            serde_json::json!({"case": case.id, "policy": policy, "index": index,
            "bar_ts_utc": b.close_time_utc, "state": project(&out)}),
        );
        // Every subsequent step must match the uninterrupted, untraced control.
        if index == case.events.len() / 2 {
            let witness = a.recovery_witness(&store).unwrap();
            a = RiFrozenShadowAdapter::restore(
                config(),
                identity.clone(),
                generation.clone(),
                &store,
                &inputs,
                &witness,
            )
            .unwrap();
            assert_eq!(a.state_trace(), out.state_trace);
        }
    }
    rows
}

#[test]
#[ignore = "explicit synthetic cross-language fixture export; no Redis/network"]
fn ri_export_independent_state_fixture_actual() {
    let input_path = std::env::var("RI_STATE_FIXTURE_INPUT").expect("explicit fixture input");
    let output_path = std::env::var("RI_STATE_FIXTURE_ACTUAL").expect("explicit NEW output");
    let bytes = fs::read(&input_path).unwrap();
    assert!(bytes.len() < 8 * 1024 * 1024);
    let f: Fixtures = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(f.schema_version, "ri_prospective_fixture_v1");
    assert_eq!(f.evidence_kind, "synthetic_model_clock");
    assert!(!f.cases.is_empty() && f.cases.len() <= 64);
    let mut ids = std::collections::HashSet::new();
    let rows: Vec<_> = f
        .cases
        .iter()
        .flat_map(|case| {
            assert!(ids.insert(&case.id), "duplicate case");
            [
                ContextPolicy::RuntimeAcceptedOnly,
                ContextPolicy::RiConfirmedClosingContextV1,
            ]
            .into_iter()
            .flat_map(|p| run(case, p))
            .collect::<Vec<_>>()
        })
        .collect();
    let result = serde_json::json!({"schema_version": "ri_prospective_fixture_actual_v1",
        "evidence_kind": f.evidence_kind, "adapter_version": VERSION,
        "input_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "binary_sha256": format!("{:x}", Sha256::digest(fs::read(std::env::current_exe().unwrap()).unwrap())),
        "rows": rows});
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, &result).unwrap();
    writeln!(&mut file).unwrap();
    file.sync_all().unwrap();
}
