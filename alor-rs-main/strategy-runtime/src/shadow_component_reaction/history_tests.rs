//! Explicit private final-history replay, never an operational admission test.
use super::*;
use std::io::Read;

#[derive(Deserialize)]
struct Header {
    record_type: String,
    schema_version: String,
    evidence_kind: String,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    definition: Definition,
    model_sha256: String,
    source_hashes: BTreeMap<String, String>,
    input_pins: BTreeMap<String, String>,
    history_columns: Vec<String>,
    bars: Vec<[f64; 6]>,
    state_columns: Vec<String>,
    expected_case_ids: Vec<String>,
}

#[derive(Deserialize)]
struct HistoryCase {
    record_type: String,
    id: String,
    date: chrono::NaiveDate,
    arm: Arm,
    gate: Option<bool>,
    history_range: [usize; 2],
    current_range: [usize; 2],
    current_accepted: bool,
    dependency_ready: bool,
    mr_previous: Option<String>,
    mr_previous2: Option<String>,
    bo_previous: Option<String>,
    bo_previous2: Option<String>,
    mr_inputs: Option<Vec<f64>>,
    bo_inputs: Option<Vec<f64>>,
    expected_state_trace: Vec<Vec<Value>>,
    events: Vec<Value>,
    trades: Vec<Value>,
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq)]
struct Counts {
    cases: usize,
    admitted: usize,
    current_grid_excluded: usize,
    context_hold: usize,
    states: usize,
    trades: usize,
    events: usize,
}
impl Counts {
    fn record(&mut self, c: &HistoryCase) {
        self.cases += 1;
        self.admitted += usize::from(c.current_accepted && c.dependency_ready);
        self.current_grid_excluded += usize::from(!c.current_accepted);
        self.context_hold += usize::from(c.current_accepted && !c.dependency_ready);
        self.states += c.expected_state_trace.len();
        self.trades += c.trades.len();
        self.events += c.events.len();
    }
}

fn expected_history_ids(
    cal: &Calendar,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Vec<String> {
    let oi_start = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
    let mut ids = Vec::new();
    let mut day = start;
    while day <= end {
        if cal.session(day).kind == crate::shadow_calendar::SessionKind::Regular
            && day.weekday().num_days_from_monday() < 5
        {
            ids.push(format!("{day}/IMO_AA"));
            ids.push(format!("{day}/IMO_AB"));
            if day >= oi_start {
                ids.push(format!("{day}/IMO_AB_OI_LAG1"));
            }
            ids.push(format!("{day}/IMO_AB_OI_OFF"));
        }
        day = day.succ_opt().unwrap();
    }
    ids
}

#[test]
fn history_case_inventory_requires_all_controls_and_never_trades_dsvd() {
    let f = fixture();
    let cal = Calendar::new(f.definition.clone()).unwrap();
    let ids = expected_history_ids(&cal, f.definition.first_date, f.definition.last_date);
    assert_eq!(ids.len(), 12);
    assert!(!ids
        .iter()
        .any(|id| id.starts_with("2026-11-07/") || id.starts_with("2026-11-08/")));
    for date in ["2026-11-05", "2026-11-06", "2026-11-09"] {
        assert!(ids.contains(&format!("{date}/IMO_AB_OI_OFF")));
        assert!(ids.contains(&format!("{date}/IMO_AB_OI_LAG1")));
    }
}

fn frozen_context(h: &Header, cal: &Calendar, c: &HistoryCase) -> FrozenComponents {
    let [first, last] = c.history_range;
    let receipts: Vec<_> = h.bars[first..last]
        .iter()
        .enumerate()
        .map(|(i, data)| {
            let b = bar(data);
            Receipt {
                sequence: i as u64 + 1,
                source_id: format!("final-history-not-pit-{}", b.close_time_utc),
                received_at_utc: Some(b.close_time_utc + 600),
                durable_at_utc: Some(b.close_time_utc + 600),
                bar: b,
            }
        })
        .collect();
    let cutoff = cal.bounds_utc(c.date).unwrap().0;
    assert!(receipts.iter().all(|r| r.durable_at_utc.unwrap() < cutoff));
    let request = Request {
        arm: c.arm,
        model_sha256: h.model_sha256.clone(),
        receipt_stream: format!("final-history-not-pit/{}", c.date),
        evidence: Evidence::NumericalFixture,
    };
    let barrier = Barrier {
        receipt_stream: request.receipt_stream.clone(),
        through_utc: cutoff,
        last_sequence: receipts.len() as u64,
        ingress_sha256: context::prefix_sha256(&receipts).unwrap(),
        attested_at_utc: cutoff,
    };
    context::freeze(cal, request, c.date, cutoff, &receipts, &barrier).unwrap()
}

fn check_case(h: &Header, cal: &Calendar, c: &HistoryCase) -> Option<Engine> {
    assert_eq!(c.record_type, "case");
    assert!(h.start <= c.date && c.date <= h.end);
    let frozen = frozen_context(h, cal, c);
    assert_eq!(
        frozen.dependency_ready, c.dependency_ready,
        "{} readiness",
        c.id
    );
    for (actual, expected) in [
        (frozen.mr.selection.previous, &c.mr_previous),
        (frozen.mr.selection.previous2, &c.mr_previous2),
        (frozen.bo.selection.previous, &c.bo_previous),
        (frozen.bo.selection.previous2, &c.bo_previous2),
    ] {
        assert_eq!(
            actual.map(|d| d.to_string()),
            *expected,
            "{} source dates",
            c.id
        );
    }
    for (component, expected) in [(&frozen.mr, &c.mr_inputs), (&frozen.bo, &c.bo_inputs)] {
        let actual = component.inputs.as_ref().map(|i| {
            vec![
                i.previous_close,
                i.previous_high,
                i.previous_low,
                i.previous_range,
                i.previous2_close,
                i.previous_return,
                i.entry_scale,
                i.stop_scale,
            ]
        });
        close(
            &json!(actual),
            &json!(expected),
            &format!("{} features/scales", c.id),
        );
    }
    let [first, last] = c.current_range;
    let rows = &h.bars[first..last];
    let labels: Vec<_> = rows.iter().map(|r| r[0] as i64).collect();
    assert_eq!(
        labels == cal.expected_labels_utc(c.date),
        c.current_accepted,
        "{} reference current-day coverage mask",
        c.id
    );
    let gate = c
        .gate
        .map_or(BoGate::Off, |allow_new_bo| BoGate::FrozenNumerical {
            allow_new_bo,
        });
    if !c.current_accepted || !c.dependency_ready {
        assert!(c.expected_state_trace.is_empty() && c.trades.is_empty() && c.events.is_empty());
        if !c.dependency_ready {
            assert!(Engine::new(frozen, gate).is_err());
        }
        // Missing current-day data is an ex-post reference mask, not a fake
        // pre-open receipt/readiness failure or a flat, clean session.
        return None;
    }
    assert_eq!(rows.len(), c.expected_state_trace.len());
    let mut e = Engine::new(frozen.clone(), gate.clone()).unwrap();
    let mut restored_pending = [false, false];
    for (i, (data, expected)) in rows.iter().zip(&c.expected_state_trace).enumerate() {
        let b = bar(data);
        let now = b.close_time_utc + 600;
        e.observe(b, now)
            .unwrap_or_else(|error| panic!("{}/{i}: {error:#}", c.id));
        let actual = row(e.state());
        assert_eq!(actual.len(), h.state_columns.len());
        assert_eq!(expected.len(), h.state_columns.len());
        for (column, (a, b)) in h.state_columns.iter().zip(actual.iter().zip(expected)) {
            close(a, b, &format!("{}/{i}/{column}", c.id));
        }
        let pending_kind = e
            .state()
            .pending
            .as_ref()
            .map(|a| usize::from(a.exit_reason.is_some()));
        let restore_pending = pending_kind.is_some_and(|kind| !restored_pending[kind]);
        if i == 5 || restore_pending || i + 1 == rows.len() {
            if let Some(kind) = pending_kind {
                restored_pending[kind] = true;
            }
            e = Engine::restore(frozen.clone(), gate.clone(), &e.checkpoint_bytes().unwrap())
                .unwrap();
        }
        assert_eq!(e.is_complete(), i + 1 == rows.len());
    }
    close(
        &json!(e.events()),
        &json!(c.events),
        &format!("{} events", c.id),
    );
    close(
        &json!(e.trades()),
        &json!(c.trades),
        &format!("{} trades/costs", c.id),
    );
    assert!(e.is_complete(), "{} terminal flat", c.id);
    Some(e)
}

#[test]
#[ignore = "explicit private pinned final-history export; no HTTP, Redis or VPS"]
fn historical_component_reference_event_state_parity() {
    let path =
        std::env::var_os("ALOR_COMPONENT_HISTORY_REFERENCE").expect("explicit reference path");
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .unwrap()
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() <= 128 * 1024 * 1024 && bytes.ends_with(b"\n"));
    let lines: Vec<_> = bytes.split_inclusive(|b| *b == b'\n').collect();
    let h: Header = serde_json::from_slice(lines[0]).unwrap();
    assert_eq!(h.record_type, "header");
    assert_eq!(h.schema_version, "imoexf_component_history_v1");
    assert_eq!(h.evidence_kind, "frozen_final_history_not_pit");
    assert_eq!(h.start.to_string(), "2026-03-23");
    assert_eq!(h.end.to_string(), "2026-11-13");
    for (key, expected) in fixture().source_hashes {
        assert_eq!(h.source_hashes[&key], expected, "pinned {key}");
    }
    assert_eq!(
        h.source_hashes["oi_gates"],
        "d536de8141b0c04939a5337ece6565e755d15b7227a74034bc957b82e8636bf3"
    );
    assert_eq!(h.model_sha256, h.source_hashes["model"]);
    assert_eq!(h.input_pins.len(), 6);
    assert_eq!(h.history_columns, ["ts_utc", "o", "h", "l", "close", "v"]);
    assert_eq!(h.state_columns, fixture().state_columns);
    assert_eq!(
        lines.len(),
        h.expected_case_ids.len() + 2,
        "missing/truncated case/footer"
    );
    assert!(h.bars.windows(2).all(|w| w[0][0] < w[1][0]));
    assert!(h
        .bars
        .iter()
        .all(|r| r[0].is_finite() && r[0].fract() == 0.));
    let cal = Calendar::new(h.definition.clone()).unwrap();
    assert_eq!(
        h.expected_case_ids,
        expected_history_ids(&cal, h.start, h.end)
    );
    let mut prefix = Sha256::new();
    prefix.update(lines[0]);
    let mut counts = Counts::default();
    let mut profiles: BTreeMap<String, Counts> = BTreeMap::new();
    let mut control: Option<(chrono::NaiveDate, Engine)> = None;
    for (id, line) in h.expected_case_ids.iter().zip(&lines[1..lines.len() - 1]) {
        prefix.update(line);
        let c: HistoryCase = serde_json::from_slice(line).unwrap();
        assert_eq!(c.id, *id, "duplicate/reordered/missing case");
        let profile = c.id.split('/').nth(1).unwrap();
        let (arm, gated) = match profile {
            "IMO_AA" => (Arm::ImoAa, false),
            "IMO_AB" => (Arm::ImoAb, false),
            "IMO_AB_OI_OFF" => (Arm::ImoAbOi, false),
            "IMO_AB_OI_LAG1" => (Arm::ImoAbOi, true),
            _ => panic!("unreviewed history profile"),
        };
        assert_eq!(c.arm, arm);
        assert_eq!(c.gate.is_some(), gated);
        let e = check_case(&h, &cal, &c);
        if profile == "IMO_AB" {
            control = e.clone().map(|e| (c.date, e));
        } else if let Some(e) = &e {
            if profile == "IMO_AB_OI_OFF" {
                let (day, a) = control.as_ref().expect("admitted A/B control");
                assert_eq!(*day, c.date);
                assert_eq!(a.state(), e.state(), "OI-off state control");
                assert_eq!(a.events(), e.events(), "OI-off event control");
                assert_eq!(a.trades(), e.trades(), "OI-off cost control");
            } else if profile == "IMO_AB_OI_LAG1" {
                let (day, a) = control.as_ref().expect("admitted A/B control");
                assert_eq!(*day, c.date);
                let mr = |engine: &Engine| {
                    engine
                        .trades()
                        .iter()
                        .filter(|t| t.component == Owner::Mr)
                        .cloned()
                        .collect::<Vec<_>>()
                };
                assert_eq!(mr(a), mr(e), "OI must never alter MR trades/costs");
                assert_eq!(a.state().bo_long_level, e.state().bo_long_level);
                assert_eq!(a.state().bo_short_level, e.state().bo_short_level);
            }
        }
        counts.record(&c);
        profiles.entry(profile.into()).or_default().record(&c);
        if counts.cases % 40 == 0 {
            println!(
                "history progress: {} cases, {} states, last={}",
                counts.cases, counts.states, c.id
            );
        }
    }
    let footer: Value = serde_json::from_slice(lines.last().unwrap()).unwrap();
    assert_eq!(footer["record_type"], "complete");
    assert_eq!(footer["prefix_sha256"], format!("{:x}", prefix.finalize()));
    let summary = &footer["summary"];
    close(
        &serde_json::to_value(&counts).unwrap(),
        &json!({
            "cases": summary["cases"], "admitted": summary["admitted"],
            "current_grid_excluded": summary["current_grid_excluded"], "context_hold": summary["context_hold"],
            "states": summary["states"], "trades": summary["trades"], "events": summary["events"]
        }),
        "complete replay counts",
    );
    assert_eq!(
        serde_json::to_value(&profiles).unwrap(),
        summary["by_profile"]
    );
    println!(
        "{}",
        json!({"status":"HISTORICAL_NUMERICAL_PARITY_PASS_NOT_PIT",
        "counts": counts, "by_profile": profiles})
    );
}
