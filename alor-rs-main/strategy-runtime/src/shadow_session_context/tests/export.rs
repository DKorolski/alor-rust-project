//! Explicit synthetic provenance fixture export, not a production receipt importer.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: String,
    evidence_kind: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    identity: Identity,
    operations: Vec<Input>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Receipt {
        value: Receipt,
        processed_at_utc: i64,
    },
    Freeze {
        day: NaiveDate,
        now: i64,
    },
    Restart {
        now: i64,
    },
}

fn projection(s: &Store) -> serde_json::Value {
    let frozen: Vec<_> = s
        .frozen
        .values()
        .map(|v| {
            let mut json = serde_json::to_value(v).unwrap();
            json.as_object_mut().unwrap().remove("identity_sha256");
            json["barrier"]
                .as_object_mut()
                .unwrap()
                .remove("ingress_sha256");
            for key in ["previous", "previous2"] {
                if let Some(a) = json[key].as_object_mut() {
                    a.remove("source_sha256");
                }
            }
            json["ready"] = v.ready().into();
            json["previous_return"] = serde_json::to_value(v.previous_return()).unwrap();
            json
        })
        .collect();
    serde_json::json!({"last_sequence": s.last_sequence(), "accepted_count": s.accepted_count(), "frozen": frozen})
}

#[test]
#[ignore = "explicit synthetic weekend context export; no network, model or broker"]
fn weekend_context_export_actual() {
    let bytes = fs::read(std::env::var("WEEKEND_FIXTURE_INPUT").expect("explicit input")).unwrap();
    assert!(bytes.len() <= 24 * 1024 * 1024);
    let f: Fixture = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(f.schema_version, "weekend_context_fixture_v1");
    assert_eq!(
        f.evidence_kind,
        "synthetic_original_receipt_fixture_not_production"
    );
    assert!(!f.cases.is_empty() && f.cases.len() <= 128);
    let mut ids = BTreeSet::new();
    let mut rows = Vec::new();
    for case in &f.cases {
        assert!(ids.insert(&case.id));
        assert!(!case.operations.is_empty() && case.operations.len() < 2000);
        let mut store = Store::new(case.identity.clone()).unwrap();
        for (index, op) in case.operations.iter().enumerate() {
            let mut route = None;
            match op {
                Input::Receipt {
                    value,
                    processed_at_utc,
                } => {
                    let result = store.ingest(value.clone(), *processed_at_utc).unwrap();
                    assert!(!result.order_emission_allowed());
                    route = Some(result);
                }
                Input::Freeze { day, now } => {
                    freeze(&mut store, &day.to_string(), *now);
                }
                Input::Restart { now } => {
                    let before = projection(&store);
                    let tmp = tempfile::tempdir().unwrap();
                    let path = tmp.path().join("checkpoint.json");
                    store.save_new_checkpoint(&path).unwrap();
                    store =
                        Store::restore(&case.identity, &fs::read(&path).unwrap(), *now).unwrap();
                    assert_eq!(projection(&store), before);
                }
            }
            rows.push(serde_json::json!({"case": case.id, "index": index, "route": route, "state": projection(&store)}));
        }
    }
    let result = serde_json::json!({"schema_version": "weekend_context_actual_v1", "evidence_kind": f.evidence_kind,
        "version": VERSION, "input_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "binary_sha256": format!("{:x}", Sha256::digest(fs::read(std::env::current_exe().unwrap()).unwrap())), "rows": rows});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var("WEEKEND_FIXTURE_ACTUAL").expect("NEW output"))
        .unwrap();
    serde_json::to_writer(&mut file, &result).unwrap();
    writeln!(&mut file).unwrap();
    file.sync_all().unwrap();
}
