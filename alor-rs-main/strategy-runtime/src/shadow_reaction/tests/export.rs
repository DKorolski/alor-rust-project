//! Input-only export for the independent Python oracle. No expected states,
//! credentials, network or runtime order path are available to this harness.
use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: String,
    evidence_kind: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoricalInput {
    value: Receipt,
    processed_at_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    identity: Identity,
    config: Config,
    history: Vec<HistoricalInput>,
    day: NaiveDate,
    freeze_at_utc: i64,
    operations: Vec<Op>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Op {
    Bar {
        value: BarEvent,
        observed_at_utc: i64,
    },
    Restart {
        now: i64,
    },
    Receipt {
        value: Receipt,
        processed_at_utc: i64,
    },
}

#[test]
#[ignore = "explicit synthetic frozen reaction comparison; requires new output"]
fn weekend_reaction_export_actual() {
    export_actual(
        "weekend_reaction_fixture_v1",
        "synthetic_numerical_reaction_not_production",
    );
}

#[test]
#[ignore = "explicit historical numerical replay; not original receipt evidence"]
fn weekend_reaction_historical_export_actual() {
    export_actual(
        "weekend_reaction_history_v1",
        "historical_prices_declared_close_clock_not_production_receipts",
    );
}

fn export_actual(schema: &str, evidence: &str) {
    let input = std::env::var("REACTION_FIXTURE_INPUT").expect("explicit input");
    let bytes = fs::read(input).unwrap();
    assert!(bytes.len() <= 32 * 1024 * 1024);
    let fixture: Fixture = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fixture.schema_version, schema);
    assert_eq!(fixture.evidence_kind, evidence);
    assert!(!fixture.cases.is_empty() && fixture.cases.len() <= 128);
    let mut cases = vec![];
    let mut ids = BTreeSet::new();
    for case in fixture.cases {
        assert!(ids.insert(case.id.clone()));
        assert!(case.history.len() <= 1000 && case.operations.len() <= 160);
        let mut context = Store::new(case.identity.clone()).unwrap();
        let mut routes = vec![];
        for input in case.history {
            routes.push(context.ingest(input.value, input.processed_at_utc).unwrap());
        }
        let dir = tempfile::tempdir().unwrap();
        let barrier = Barrier {
            receipt_stream: case.identity.receipt_stream.clone(),
            through_utc: case.identity.cutoff(case.day),
            last_sequence: context.last_sequence(),
            ingress_sha256: context.ingress_sha256().unwrap(),
            attested_at_utc: case.freeze_at_utc,
        };
        let frozen = context
            .freeze_and_save_new(
                case.day,
                case.freeze_at_utc,
                &barrier,
                &dir.path().join("initial-context.json"),
            )
            .unwrap()
            .clone();
        let mut session = NumericalSession::new(&context, case.day, case.config.clone()).unwrap();
        let mut rows = vec![];
        for (index, op) in case.operations.into_iter().enumerate() {
            let mut route = None;
            let kind = match op {
                Op::Bar {
                    value,
                    observed_at_utc,
                } => {
                    session.observe(value, observed_at_utc).unwrap();
                    "bar"
                }
                Op::Receipt {
                    value,
                    processed_at_utc,
                } => {
                    let before = session.state().unwrap();
                    route = Some(context.ingest(value, processed_at_utc).unwrap());
                    assert_eq!(session.state().unwrap(), before);
                    "receipt"
                }
                Op::Restart { now } => {
                    let before = session.state().unwrap();
                    let file = dir.path().join(format!("model-{index}.json"));
                    fs::write(&file, session.checkpoint_bytes().unwrap()).unwrap();
                    session = NumericalSession::restore(
                        &case.identity,
                        &case.config,
                        &fs::read(file).unwrap(),
                        now,
                    )
                    .unwrap();
                    assert_eq!(session.state().unwrap(), before);
                    let file = dir.path().join(format!("context-{index}.json"));
                    context.save_new_checkpoint(&file).unwrap();
                    context =
                        Store::restore(&case.identity, &fs::read(file).unwrap(), now).unwrap();
                    assert_eq!(context.frozen(case.day), Some(&frozen));
                    "restart"
                }
            };
            let state = session.state().unwrap();
            assert_eq!(state["order_emission_allowed"], false);
            assert_eq!(state["frozen"], serde_json::to_value(&frozen).unwrap());
            rows.push(json!({"index":index,"kind":kind,"route":route,
                "context_sequence":context.last_sequence(),"context_accepted":context.accepted_count(),
                "bars_processed":state["bars_processed"],"model":state["model"]}));
        }
        session.complete().unwrap();
        cases.push(json!({"id":case.id,"identity":case.identity,"config":case.config,
            "routes":routes,"frozen":frozen,"rows":rows,"complete":true,"order_emission_allowed":false}));
    }
    let result = json!({"schema_version":"weekend_reaction_actual_v1",
        "evidence_kind":fixture.evidence_kind,"input_sha256":format!("{:x}",Sha256::digest(&bytes)),
        "binary_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe().unwrap()).unwrap())),
        "cases":cases});
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var("REACTION_FIXTURE_ACTUAL").expect("new output"))
        .unwrap();
    serde_json::to_writer(&mut out, &result).unwrap();
    writeln!(&mut out).unwrap();
    out.sync_all().unwrap();
}
