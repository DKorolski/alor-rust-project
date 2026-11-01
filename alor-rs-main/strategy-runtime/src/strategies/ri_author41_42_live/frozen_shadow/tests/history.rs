//! Test-only use of a DECLARED numerical availability clock, not an ingress import.
use super::*;
use std::{fs, io::Write};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    schema_version: String,
    evidence_kind: String,
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stream {
    symbol: String,
    first_decision_session: NaiveDate,
    source_sha256: String,
    inputs: Vec<NumericalInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NumericalInput {
    bar: BarEvent,
    available_epoch: i64,
}

fn run(stream: &Stream, policy: ContextPolicy) -> Vec<serde_json::Value> {
    assert!(matches!(stream.symbol.as_str(), "RIU6" | "RIZ6"));
    assert!(!stream.inputs.is_empty() && stream.inputs.len() <= 9900);
    assert_eq!(stream.source_sha256.len(), 64);
    let mut cfg = config();
    cfg.symbol = stream.symbol.clone();
    let identity = context_identity(&cfg, "NUMERICAL_ONLY_NOT_RECEIPTS".into(), policy).unwrap();
    let mut store = RiContextStore::new(identity.clone()).unwrap();
    let generation = ShadowGeneration {
        id: format!("numerical-history-{}", stream.symbol),
        first_decision_session: stream.first_decision_session,
    };
    let mut a =
        RiFrozenShadowAdapter::new(cfg.clone(), identity.clone(), generation.clone()).unwrap();
    let mut inputs = Vec::new();
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut previous = None;
    let mut observed_once = false;
    for (index, item) in stream.inputs.iter().enumerate() {
        let b = &item.bar;
        let key = (item.available_epoch, b.close_time_utc);
        assert!(previous.is_none_or(|p| p < key));
        assert!(seen.insert(b.close_time_utc));
        assert_eq!(b.symbol, stream.symbol);
        assert!(item.available_epoch >= b.close_time_utc + 600);
        previous = Some(key);
        let dt = local(b.close_time_utc).unwrap();
        if dt.date() >= stream.first_decision_session
            && dt.weekday().num_days_from_monday() < 5
            && b.origin == DataOrigin::Live
            && item.available_epoch >= identity.cutoff(dt.date())
            && store.frozen(dt.date()).is_none()
        {
            // Synthetic numerical barrier ONLY in cfg(test). It attests no fsync.
            freeze(&mut store, &dt.date().to_string(), item.available_epoch);
        }
        add(
            &mut store,
            &mut inputs,
            ContextInput {
                sequence: 0,
                bar: b.clone(),
                received_at_utc: Some(item.available_epoch),
                durable_at_utc: Some(item.available_epoch),
            },
        );
        let route = store.verified_route(inputs.last().unwrap()).unwrap();
        let route_json = serde_json::to_value(route).unwrap();
        let routing = if route_json["kind"] == "quarantined" {
            format!("quarantined:{}", route_json["reason"].as_str().unwrap())
        } else {
            route_json["kind"].as_str().unwrap().to_string()
        };
        let out = a.process(&store, inputs.last().unwrap()).unwrap();
        let observed = matches!(
            out.disposition.as_str(),
            "prospective_observed" | "context_not_ready"
        );
        observed_once |= observed;
        assert!(out
            .records
            .iter()
            .all(|r| r.adapter_decision != RiAuthor4142JournalDecision::IntentEmitted));
        rows.push(
            serde_json::json!({"symbol": stream.symbol, "policy": policy, "index": index,
            "bar_ts_utc": b.close_time_utc, "available_epoch": item.available_epoch,
            "origin": b.origin, "route": routing, "observed": observed,
            "state": observed_once.then(|| super::reference::project(&out))}),
        );
    }
    assert!(observed_once);
    assert!(
        a.strategy.live_mr.position.is_none()
            && a.strategy.live_bo.position.is_none()
            && a.strategy.live_bo.pending.is_none(),
        "unresolved final history tail"
    );
    let witness = a.recovery_witness(&store).unwrap();
    let restored =
        RiFrozenShadowAdapter::restore(cfg, identity, generation, &store, &inputs, &witness)
            .unwrap();
    assert_eq!(a.state_trace(), restored.state_trace());
    rows
}

#[test]
#[ignore = "explicit historical numerical export, NOT original receipts; no Redis/network"]
fn ri_export_independent_history_actual() {
    let bytes =
        fs::read(std::env::var("RI_HISTORY_INPUT").expect("explicit numerical input")).unwrap();
    assert!(bytes.len() < 16 * 1024 * 1024);
    let f: History = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(f.schema_version, "ri_numerical_history_v1");
    assert_eq!(
        f.evidence_kind,
        "legacy_mixed_availability_clock_not_durability"
    );
    assert!(!f.streams.is_empty() && f.streams.len() <= 2);
    let mut seen = std::collections::HashSet::new();
    let rows: Vec<_> = f
        .streams
        .iter()
        .flat_map(|s| {
            assert!(seen.insert(&s.symbol));
            [
                ContextPolicy::RuntimeAcceptedOnly,
                ContextPolicy::RiConfirmedClosingContextV1,
            ]
            .into_iter()
            .flat_map(|p| run(s, p))
            .collect::<Vec<_>>()
        })
        .collect();
    let result = serde_json::json!({"schema_version": "ri_numerical_history_actual_v1",
        "evidence_kind": f.evidence_kind, "adapter_version": VERSION,
        "input_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "binary_sha256": format!("{:x}", Sha256::digest(fs::read(std::env::current_exe().unwrap()).unwrap())),
        "recovery_verified": true, "rows": rows});
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var("RI_HISTORY_ACTUAL").expect("NEW output"))
        .unwrap();
    serde_json::to_writer(&mut file, &result).unwrap();
    writeln!(&mut file).unwrap();
    file.sync_all().unwrap();
}
