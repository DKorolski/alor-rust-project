use super::model::*;
use super::trace;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde_json::json;

fn cfg() -> Config {
    Config {
        model_id: "fixture".into(),
        family: Family::PreviousDay,
        context_hour: 7,
        clock_hour: 7,
        mr_horizon: 3,
        bo_wait: 4,
        mr_start_delay: 0,
    }
}

fn ts(value: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap()
}

fn fixture() -> Vec<Bar> {
    let mut result = Vec::new();
    for date in ["2026-07-10", "2026-07-13", "2026-07-14"] {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        let mut label = date.and_time(time(opening(date, 7), 0));
        let mut i = 0;
        while label.time() <= time(23, 40) {
            let mut b = Bar {
                ts_local: label,
                open: 75.0,
                high: 76.0,
                low: 74.0,
                close: 75.0,
                volume: 100.0,
            };
            if date.day() == 14 {
                b.high = 75.05;
                b.low = 74.95;
                if [0, 4, 8].contains(&i) {
                    b.open = 75.02;
                    b.close = 75.02;
                }
                if [2, 6, 10].contains(&i) {
                    b.low = 74.6;
                }
                if i == 6 {
                    b.high = 76.0;
                }
                if i == 24 {
                    b.open = 76.2;
                    b.close = 76.2;
                    b.high = 76.3;
                }
            }
            result.push(b);
            label += Duration::minutes(10);
            i += 1;
        }
    }
    result
}

use chrono::Datelike;

fn run(raw: &[Bar], config: Config) -> Engine {
    validate_raw(raw).unwrap();
    let bars: Vec<_> = raw.iter().filter(|b| config.eligible(b)).copied().collect();
    let mut engine = Engine::new(config);
    engine.replay(&bars, |_, _, _| Ok(())).unwrap();
    engine
}

#[test]
fn uncapped_mr_stop_priority_and_bo_slot() {
    let e = run(&fixture(), cfg());
    let mr: Vec<_> = e
        .trades
        .iter()
        .filter(|t| t.component == Component::MR)
        .collect();
    assert_eq!(mr.len(), 3);
    assert_eq!(
        mr.iter()
            .map(|t| t.exit_reason.as_str())
            .collect::<Vec<_>>(),
        ["mr_take", "mr_stop", "mr_take"]
    );
    assert!(mr[1].ambiguous_mr_bar);
    assert_eq!(e.mr_scheduled, 3);
    assert_eq!(e.trades.len(), 4);
    assert!(e.used_long);
    assert!(!e.used_short);
    assert!(e.position.is_none() && e.pending.is_none());
}

#[test]
fn pending_is_causal_and_scale_is_captured_at_signal() {
    let mut raw = fixture();
    raw.iter_mut()
        .find(|b| b.ts_local == ts("2026-07-14 07:00"))
        .unwrap()
        .high = 76.0;
    let mut e = Engine::new(cfg());
    let mut saw_signal = false;
    let mut saw_fill = false;
    e.replay(&raw, |phase, engine, bar| {
        if bar.ts_local == ts("2026-07-14 07:00") && phase == "bar_end" {
            let Some(Pending::Entry(p)) = &engine.pending else {
                panic!("entry must be pending");
            };
            assert!(engine.position.is_none());
            assert_eq!(p.action, ts("2026-07-14 07:10"));
            assert_eq!(p.scale, 2.0);
            assert!(!engine.used_long);
            saw_signal = true;
        }
        if bar.ts_local == ts("2026-07-14 07:10") && phase == "pending_applied" {
            let p = engine.position.as_ref().unwrap();
            assert_eq!(p.price, 75.0);
            assert_eq!(p.stop, Some(round_tick(75.86)));
            assert_eq!(p.take, Some(round_tick(74.68)));
            saw_fill = true;
        }
        Ok(())
    })
    .unwrap();
    assert!(saw_signal && saw_fill);
}

#[test]
fn vwap_zero_and_negative_volume_falls_back_to_close() {
    let mut raw = fixture();
    for b in &mut raw {
        b.volume = -1.0;
    }
    let mut config = cfg();
    config.family = Family::Vwap;
    let mut e = Engine::new(config);
    e.replay(&raw, |_, engine, bar| {
        assert_eq!(engine.metrics.as_ref().unwrap().volume.sum, 0.0);
        assert_eq!(engine.features().0, Some(bar.close));
        Ok(())
    })
    .unwrap();
}

#[test]
fn mr_force_windows_h3_h4_and_09() {
    for (hour, horizon, exit) in [
        (7, 3, "2026-07-14 10:00"),
        (7, 4, "2026-07-14 11:00"),
        (9, 3, "2026-07-14 12:00"),
    ] {
        let mut raw = fixture();
        let mut config = cfg();
        config.context_hour = hour;
        config.clock_hour = hour;
        config.mr_horizon = horizon;
        config.bo_wait = if hour == 9 { 2 } else { 4 };
        for b in raw.iter_mut().filter(|b| b.ts_local.date().day() == 14) {
            b.open = 75.0;
            b.close = 75.0;
            b.high = 75.05;
            b.low = 74.95;
            if b.ts_local.time() == time(hour, 0) {
                b.close = 75.02;
            }
        }
        let e = run(&raw, config);
        let mr: Vec<_> = e
            .trades
            .iter()
            .filter(|t| t.component == Component::MR)
            .collect();
        assert_eq!(mr.len(), 1);
        assert_eq!(mr[0].exit_ts, ts(exit));
        assert_eq!(mr[0].exit_reason, "mr_time_cutoff");
    }
}

#[test]
fn bo_wait_is_clock_based_and_eod_is_next_open() {
    for wait in [2, 4] {
        let mut raw = fixture();
        let mut config = cfg();
        config.bo_wait = wait;
        for b in raw.iter_mut().filter(|b| b.ts_local.date().day() == 14) {
            b.open = 76.2;
            b.close = 76.2;
            b.high = 76.3;
            b.low = 76.1;
        }
        let e = run(&raw, config);
        assert_eq!(e.trades.len(), 1);
        assert_eq!(e.trades[0].entry_ts.time(), time(7 + wait, 10));
        assert_eq!(e.trades[0].exit_ts.time(), time(23, 40));
        assert_eq!(e.trades[0].exit_reason, "bo_eod_exit");
    }
}

#[test]
fn terminal_entry_cannot_create_overnight_position() {
    let mut raw = fixture();
    for b in raw.iter_mut().filter(|b| b.ts_local.date().day() == 14) {
        b.open = 75.0;
        b.close = 75.0;
        b.high = 75.05;
        b.low = 74.95;
        if b.ts_local.time() == time(23, 20) {
            b.open = 76.2;
            b.close = 76.2;
            b.high = 76.3;
        }
    }
    let e = run(&raw, cfg());
    assert!(e.trades.is_empty());
    assert_eq!(e.events.len(), 1);
    assert_eq!(e.events[0].role, "entry_suppressed_terminal");
    assert!(e.events[0].price.is_none());
}

#[test]
fn missing_opening_middle_eod_duplicate_and_nonfinite_fail() {
    for missing in ["2026-07-14 07:00", "2026-07-14 12:00", "2026-07-14 23:40"] {
        let bars: Vec<_> = fixture()
            .into_iter()
            .filter(|b| b.ts_local != ts(missing))
            .collect();
        assert!(Engine::new(cfg()).replay(&bars, |_, _, _| Ok(())).is_err());
    }
    let mut raw = fixture();
    raw[1].ts_local = raw[0].ts_local;
    assert!(validate_raw(&raw).is_err());
    raw = fixture();
    raw[0].volume = f64::NAN;
    assert!(validate_raw(&raw).is_err());
}

#[test]
fn weekend_service_and_09_early_bars_never_update_state() {
    let mut raw = fixture();
    for label in ["2026-07-12 12:00", "2026-07-13 08:50", "2026-07-14 06:50"] {
        raw.push(Bar {
            ts_local: ts(label),
            open: 150.0,
            high: 200.0,
            low: 100.0,
            close: 150.0,
            volume: 99999.0,
        });
    }
    raw.sort_by_key(|b| b.ts_local);
    let baseline = run(&fixture(), cfg());
    let changed = run(&raw, cfg());
    assert_eq!(
        serde_json::to_value(&baseline.trades).unwrap(),
        serde_json::to_value(&changed.trades).unwrap()
    );
    assert_eq!(changed.previous.as_ref().unwrap().high, 76.0);
    let mut config = cfg();
    config.context_hour = 9;
    config.clock_hour = 9;
    assert!(!config.eligible(
        raw.iter()
            .find(|b| b.ts_local == ts("2026-07-14 08:00"))
            .unwrap()
    ));
}

#[test]
fn future_bar_cannot_change_prior_trace_or_captured_features() {
    let raw = fixture();
    let mut changed = raw.clone();
    changed
        .iter_mut()
        .find(|b| b.ts_local == ts("2026-07-14 20:00"))
        .unwrap()
        .high = 90.0;
    for family in [Family::PreviousDay, Family::Vwap] {
        let collect = |bars: &[Bar]| {
            let mut config = cfg();
            config.family = family;
            let mut e = Engine::new(config);
            let mut rows = Vec::new();
            e.replay(bars, |phase, e, b| {
                if b.ts_local < ts("2026-07-14 20:00") {
                    rows.push(format!(
                        "{} {} {:?} {:?} {:?}",
                        b.ts_local,
                        phase,
                        e.features(),
                        e.position,
                        e.pending
                    ));
                }
                Ok(())
            })
            .unwrap();
            rows
        };
        assert_eq!(collect(&raw), collect(&changed));
    }
}

#[test]
fn warmup_fallback_is_distinct_from_two_session_readiness() {
    let mut e = Engine::new(cfg());
    e.replay(&fixture(), |phase, e, b| {
        if phase == "observed" && b.ts_local == ts("2026-07-13 09:00") {
            assert!(e.previous.is_some());
            assert!(e.previous2.is_none());
            assert_eq!(e.prior_return(), None);
            assert_eq!(e.features().3, 0.0);
        }
        if phase == "observed" && b.ts_local == ts("2026-07-14 07:00") {
            assert!(e.previous2.is_some());
            assert_eq!(e.prior_return(), Some(0.0));
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn manifest_selection_is_not_trusted_without_actual_raw_filter() {
    let raw = fixture();
    let config = cfg();
    let inputs: Vec<_> = raw
        .iter()
        .map(|b| trace::Input {
            input_id: format!("USDRUBF|10m|{}", trace::stamp(b.ts_local)),
            bar_ts_local: trace::stamp(b.ts_local),
            input_sha256: "a".repeat(64),
        })
        .collect();
    let hash = trace::hash_json(&serde_json::to_value(&inputs).unwrap());
    let mut manifest = json!({"bundle_version":"shadow_state_bundle_v1","trace_schema":trace::SCHEMA,
        "frozen_contract_sha256":trace::CONTRACT,"inputs":inputs,"inputs_sha256":hash,
        "excluded_inputs":[],"excluded_inputs_sha256":trace::hash_json(&json!([])),
        "raw_input_count":raw.len(),"raw_inputs_sha256":hash});
    assert!(trace::verify_selection(&manifest, &raw, &config).is_ok());
    manifest["inputs"][0]["bar_ts_local"] = json!("2026-07-10T08:50:00");
    manifest["inputs_sha256"] = json!(trace::hash_json(&manifest["inputs"]));
    assert!(trace::verify_selection(&manifest, &raw, &config).is_err());
}

#[test]
fn raw_csv_uses_moscow_naive_labels_not_utc_or_offsets() {
    let good = "ts_local,open,high,low,close,volume\n2026-07-14 07:00:00,75,76,74,75,100\n";
    let bar: Bar = csv::Reader::from_reader(good.as_bytes())
        .deserialize()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(bar.ts_local, ts("2026-07-14 07:00"));
    for label in ["2026-07-14 07:00:00Z", "2026-07-14 07:00:00+03:00"] {
        let bad = good.replace("2026-07-14 07:00:00", label);
        assert!(csv::Reader::from_reader(bad.as_bytes())
            .deserialize::<Bar>()
            .next()
            .unwrap()
            .is_err());
    }
}

#[test]
fn bo_quota_is_one_fill_per_side_not_one_total() {
    let mut raw = fixture();
    for (label, price) in [
        ("2026-07-14 12:10", 76.2),
        ("2026-07-14 13:00", 73.8),
        ("2026-07-14 14:10", 73.8),
    ] {
        let b = raw.iter_mut().find(|b| b.ts_local == ts(label)).unwrap();
        b.open = price;
        b.close = price;
        b.high = price + 0.05;
        b.low = price - 0.05;
    }
    let e = run(&raw, cfg());
    let bo: Vec<_> = e
        .trades
        .iter()
        .filter(|t| t.component == Component::BO)
        .collect();
    assert_eq!(bo.len(), 2);
    assert_eq!(bo[0].entry_ts, ts("2026-07-14 11:10"));
    assert_eq!(bo[1].entry_ts, ts("2026-07-14 13:10"));
    assert!(e.used_long && e.used_short);
    for pair in e.trades.windows(2) {
        assert!(pair[0].exit_ts < pair[1].entry_ts);
    }
}

#[test]
fn exit_signal_cannot_also_emit_entry_but_next_completed_bar_can() {
    let mut raw = fixture();
    let b = raw
        .iter_mut()
        .find(|b| b.ts_local == ts("2026-07-14 07:30"))
        .unwrap();
    b.close = 75.02;
    let e = run(&raw, cfg());
    assert_eq!(e.trades[0].exit_signal_ts, ts("2026-07-14 07:20"));
    assert_eq!(e.trades[0].exit_ts, ts("2026-07-14 07:30"));
    assert_eq!(e.trades[1].entry_signal_ts, ts("2026-07-14 07:30"));
    assert_eq!(e.trades[1].entry_ts, ts("2026-07-14 07:40"));
}

#[test]
fn exporter_refuses_output_inside_immutable_inputs() {
    let root = tempfile::tempdir().unwrap();
    let handoff = root.path().join("handoff");
    let reference = root.path().join("reference");
    std::fs::create_dir(&handoff).unwrap();
    std::fs::create_dir(&reference).unwrap();
    for parent in [&handoff, &reference] {
        let output = parent.join("new-output");
        let error = super::run(&handoff, &reference, &output).unwrap_err();
        assert!(error.to_string().contains("outside immutable"));
        assert!(!output.exists());
    }
}
