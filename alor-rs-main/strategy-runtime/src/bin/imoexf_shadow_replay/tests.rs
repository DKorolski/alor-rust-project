use super::{model::*, trace};
use chrono::Duration;
use serde_json::{json, Value};

fn bar(day: u32, minute: i64, price: f64) -> Bar {
    Bar {
        ts_local: transition().and_time(time(7, 0))
            + Duration::days(i64::from(day))
            + Duration::minutes(minute),
        open: price,
        high: price,
        low: price,
        close: price,
        volume: 1.0,
    }
}

fn data() -> Vec<Bar> {
    let mut prior = bar(0, 0, 104.0);
    prior.high = 106.0;
    prior.low = 100.0;
    let mut bars = vec![prior];
    bars.extend((0..=100).map(|i| bar(1, i * 10, 104.0)));
    bars
}

fn price(b: &mut Bar, value: f64) {
    b.open = value;
    b.high = value;
    b.low = value;
    b.close = value;
}

fn engine(sleeve: &'static str) -> Engine {
    Engine::new(
        sleeve,
        Costs {
            point_value_rub: 10.0,
            tick_value_rub: 5.0,
            rate_per_component_per_side: 0.00006575119054451348,
            components: 2,
        },
    )
    .unwrap()
}

fn replay(sleeve: &'static str, bars: &[Bar]) -> Engine {
    let mut e = engine(sleeve);
    e.replay(bars, |_, _, _| Ok(())).unwrap();
    e
}

#[test]
fn reaction_uses_next_open_and_counts_only_filled_entries() {
    let mut bars = data();
    price(&mut bars[1], 104.5);
    price(&mut bars[2], 103.5);
    bars[2].open = 105.0;
    bars[2].high = 105.0;
    price(&mut bars[3], 103.8);
    let mut e = engine("combo");
    let mut states = vec![];
    e.replay(&bars, |phase, e, b| {
        if b.ts_local.date() > transition() && b.ts_local.time() < time(7, 30) {
            states.push((
                b.ts_local.time(),
                phase.to_owned(),
                e.mr_entries,
                e.observation.as_ref().unwrap().count,
                e.position.as_ref().map(|p| p.bars_held),
                e.branch,
            ));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(e.trades.len(), 1);
    let t = &e.trades[0];
    assert_eq!(
        (t.entry_price, t.exit_price, t.gross_points),
        (105.0, 103.8, 105.0 - 103.8)
    );
    assert_eq!(t.exit_reason, "take_author_close");
    assert_eq!(t.bars_held, 1);
    assert!(states.contains(&(time(7, 0), "bar_end".into(), 0, 1, None, "mr_selected")));
    assert!(states.contains(&(
        time(7, 10),
        "pending_applied".into(),
        1,
        1,
        Some(0),
        "not_evaluated"
    )));
    assert!(states.contains(&(
        time(7, 10),
        "observed".into(),
        1,
        2,
        Some(0),
        "not_evaluated"
    )));
    assert!(states.contains(&(
        time(7, 10),
        "bar_end".into(),
        1,
        2,
        Some(1),
        "pending_active"
    )));
    assert!(states.contains(&(
        time(7, 20),
        "pending_applied".into(),
        1,
        2,
        None,
        "not_evaluated"
    )));
}

#[test]
fn mr_uses_previous_low_normalizer_and_strict_price_boundaries() {
    let mut bars = data();
    bars[0].high = 107.5;
    bars[0].close = 107.0;
    for b in &mut bars[1..] {
        price(b, 107.5);
    }
    assert!(replay("mr", &bars).trades.is_empty()); // 7.5/100 == upper bound, not 7.5/107.
    let mut bars = data();
    price(&mut bars[1], 104.0 + 0.16 * 6.0);
    assert!(replay("mr", &bars).trades.is_empty());
}

#[test]
fn mr_window_is_inclusive_at_1000_but_not_a_forced_exit() {
    let mut bars = data();
    for b in &mut bars[19..] {
        price(b, 104.5);
    }
    bars[20].open = 104.0;
    bars[20].low = 104.0;
    let e = replay("mr", &bars);
    assert_eq!(e.trades.len(), 1);
    assert_eq!(e.trades[0].entry_signal_ts.time(), time(10, 0));
    assert_eq!(e.trades[0].exit_signal_ts.time(), time(20, 0));
    assert_eq!(e.trades[0].exit_reason, "time_exit");
    price(&mut bars[19], 104.0);
    assert!(replay("mr", &bars).trades.is_empty());
}

#[test]
fn mr_fill_cap_is_two_and_no_reversal_is_emitted_at_exit_signal() {
    let mut bars = data();
    for i in [1, 3, 5] {
        price(&mut bars[i], 104.5);
    }
    for i in [2, 4, 6] {
        price(&mut bars[i], 103.5);
    }
    let e = replay("combo", &bars);
    assert_eq!(e.trades.len(), 2);
    assert!(e.trades.iter().all(|t| t.component == Owner::MR));
    assert_eq!(e.trades[1].entry_signal_ts, e.trades[0].exit_ts);
    assert!(e.trades[1].entry_ts > e.trades[0].exit_ts);
    assert_eq!(e.mr_entries, 2);
}

#[test]
fn mr_stop_has_priority_over_take_and_executes_at_next_open() {
    let mut bars = data();
    price(&mut bars[1], 104.5);
    price(&mut bars[2], 103.0);
    bars[2].high = 107.48;
    let e = replay("mr", &bars);
    assert_eq!(e.trades[0].exit_reason, "stop");
    assert_eq!(e.trades[0].exit_price, bars[3].open);
    assert_ne!(e.trades[0].exit_price, 107.48);
}

#[test]
fn mr_breakeven_requires_strictly_more_than_twenty_bars() {
    let mut bars = data();
    for b in &mut bars[1..23] {
        price(b, 104.4);
        b.low = 104.0;
    }
    bars[2].open = 104.0;
    let e = replay("mr", &bars);
    assert_eq!(e.trades[0].bars_held, 21);
    assert_eq!(e.trades[0].exit_reason, "breakeven_limit");
    assert_eq!(e.trades[0].exit_ts, bars[23].ts_local);
}

#[test]
fn bo_observations_continue_through_mr_and_bo_wait_uses_first_bar() {
    let mut bars = data();
    price(&mut bars[1], 104.5);
    for b in &mut bars[2..] {
        price(b, 108.0);
    }
    let e = replay("combo", &bars);
    assert_eq!(e.trades.len(), 2);
    assert_eq!(e.trades[0].exit_reason, "stop");
    assert_eq!(e.trades[1].entry_signal_ts.time(), time(10, 0));
    assert_eq!(e.observation.as_ref().unwrap().count, 101);
    // Delayed first input shifts BO wait; there is no synthetic 07:00 bar.
    let mut delayed = data();
    delayed.drain(1..7);
    for b in &mut delayed[1..] {
        price(b, 108.0);
    }
    let e = replay("combo", &delayed);
    assert_eq!(e.trades[0].entry_signal_ts.time(), time(11, 0));
}

#[test]
fn bo_stops_are_close_based_and_eod_precedes_stop_checks() {
    let mut bars = data();
    for b in &mut bars[19..] {
        price(b, 108.0);
        b.low = 100.0;
    }
    price(&mut bars[100], 100.0); // 23:30 also crosses stop2.
    let e = replay("combo", &bars);
    assert_eq!(e.trades.len(), 1);
    assert_eq!(e.trades[0].exit_reason, "bo_eod");
    assert_eq!(e.trades[0].exit_signal_ts.time(), time(23, 30));
    assert_eq!(e.trades[0].exit_ts.time(), time(23, 40));
}

#[test]
fn bo_cannot_enter_on_eod_execution_bar() {
    let mut bars = data();
    for b in &mut bars[99..] {
        price(b, 108.0);
    }
    let e = replay("combo", &bars);
    assert!(e.trades.is_empty());
    assert_eq!(e.events.len(), 1);
    assert_eq!(e.events[0].role, "entry_suppressed_eod");
    assert!(e.events[0].price.is_none() && e.events[0].signal_ts.is_none());
}

#[test]
fn missing_execution_or_eod_bar_fails_closed() {
    let mut bars = data();
    price(&mut bars[1], 104.5);
    bars.remove(2);
    assert!(engine("mr").replay(&bars, |_, _, _| Ok(())).is_err());
    let mut bars = data();
    for b in &mut bars[19..] {
        price(b, 108.0);
    }
    bars.remove(100); // Lost EOD signal must not disappear at session rollover.
    assert!(engine("combo").replay(&bars, |_, _, _| Ok(())).is_err());
    let mut bars = data();
    for b in &mut bars[19..] {
        price(b, 108.0);
    }
    bars.pop();
    assert!(engine("combo").replay(&bars, |_, _, _| Ok(())).is_err());
}

#[test]
fn feed_validation_rejects_service_weekend_duplicates_and_bad_ohlcv() {
    let base = data()[0];
    let mut b = base;
    b.ts_local = base.ts_local - Duration::days(1); // 13 July 07:00.
    assert!(validate_bars(&[b]).is_err());
    b = bar(4, 0, 104.0);
    assert!(validate_bars(&[b]).is_err()); // Saturday.
    b = base;
    b.volume = -1.0;
    assert!(validate_bars(&[b]).is_err());
    b = base;
    b.close = b.high + 1.0;
    assert!(validate_bars(&[b]).is_err());
    b = base;
    b.volume = f64::NAN;
    assert!(validate_bars(&[b]).is_err());
    assert!(validate_bars(&[base, base]).is_err());
    b = base;
    b.ts_local += Duration::minutes(1);
    assert!(validate_bars(&[b]).is_err());
}

#[test]
fn trace_is_causal_and_input_provenance_cannot_be_substituted() {
    let mut bars = data();
    price(&mut bars[1], 104.5);
    price(&mut bars[2], 103.0);
    let inputs: Vec<_> = bars
        .iter()
        .map(|b| trace::Input {
            input_id: format!("IMOEXF|10m|{}", trace::stamp(b.ts_local)),
            input_sha256: "a".repeat(64),
            bar_ts_local: trace::stamp(b.ts_local),
        })
        .collect();
    let mut manifest = json!({"bundle_version":"shadow_state_bundle_v1","trace_schema":trace::SCHEMA,
        "identity":trace::identity("combo"),"model_input_file_sha256":"raw", "reference_manifest_sha256":"ref",
        "inputs":inputs,"inputs_sha256":trace::hash_json(&serde_json::to_value(&inputs).unwrap())});
    assert!(trace::verify_inputs(&manifest, &bars, "raw", "combo", "ref").is_ok());
    assert!(trace::verify_inputs(&manifest, &bars, "other", "combo", "ref").is_err());
    manifest["inputs"][0]["input_id"] = json!("wrong");
    assert!(trace::verify_inputs(&manifest, &bars, "raw", "combo", "ref").is_err());
    let mut bytes = vec![];
    let mut writer = trace::Trace::new(&mut bytes, "combo", inputs);
    let mut e = engine("combo");
    e.replay(&bars, |phase, e, b| writer.emit(phase, e, b))
        .unwrap();
    writer.finish().unwrap();
    drop(writer);
    let rows: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let row = rows
        .iter()
        .find(|r| r["phase"] == "bar_end" && r["signals"]["branch"] == "mr_selected")
        .unwrap();
    assert_eq!(row["pending"]["proxy_known"], false);
    assert!(row["position"].is_null());
    assert!(row["pending"].get("price").is_none());
    assert!(row["timing"]["received_ts_utc"].is_null());
    assert!(rows[0]["bo_observation"]["day_high"].is_null());
    assert_eq!(rows[0]["bo_observation"]["count"], 0);
}

#[test]
fn previous2_return_is_causal_and_missing_context_is_explicit() {
    let mut bars = data();
    price(&mut bars[1], 104.5);
    price(&mut bars[2], 103.5);
    bars.push(bar(2, 0, 104.0));
    let mut first = None;
    let mut second = None;
    let mut third = None;
    engine("mr")
        .replay(&bars, |phase, e, b| {
            if phase == "bar_start" && b.ts_local.time() == time(7, 0) {
                let value = (e.valid(), e.context(), e.previous.as_ref().map(|p| p.count));
                match (b.ts_local.date() - transition()).num_days() {
                    0 => first = Some(value),
                    1 => second = Some(value),
                    2 => third = Some(value),
                    _ => unreachable!(),
                }
            }
            Ok(())
        })
        .unwrap();
    assert!(!first.unwrap().0);
    assert!(second.unwrap().1 .3.is_nan());
    assert_eq!(second.unwrap().2, Some(1));
    assert_eq!(third.unwrap().1 .3, 0.0);
    assert_eq!(third.unwrap().2, Some(101));
    // A future day's extreme prices cannot change already completed trades.
    let baseline = replay("mr", &bars);
    assert_eq!(baseline.trades.len(), 1);
    bars.last_mut().unwrap().high = 10000.0;
    let changed = replay("mr", &bars);
    assert_eq!(
        serde_json::to_value(baseline.trades).unwrap(),
        serde_json::to_value(changed.trades).unwrap()
    );
}

#[test]
fn bo_used_side_is_counted_on_fill_not_signal_and_return_gate_blocks_long() {
    let mut bars = data();
    for b in &mut bars[19..] {
        price(b, 108.0);
    }
    let mut saw_signal = false;
    let mut saw_fill = false;
    engine("combo")
        .replay(&bars, |phase, e, b| {
            if b.ts_local.date() > transition()
                && b.ts_local.time() == time(10, 0)
                && phase == "bar_end"
            {
                assert!(!e.used_long);
                assert_eq!(e.branch, "bo_selected");
                saw_signal = true;
            }
            if b.ts_local.date() > transition()
                && b.ts_local.time() == time(10, 10)
                && phase == "pending_applied"
            {
                assert!(e.used_long);
                saw_fill = true;
            }
            Ok(())
        })
        .unwrap();
    assert!(saw_signal && saw_fill);
    // New prior2 establishes a -5.45% previous return without a future override.
    let mut earlier = bar(0, 0, 110.0);
    earlier.ts_local -= Duration::days(1);
    earlier.ts_local += Duration::hours(2); // pre-transition 09:00.
    let mut gated = vec![earlier];
    gated.extend(bars);
    assert!(replay("combo", &gated).trades.is_empty());
}

#[test]
fn csv_requires_naive_moscow_labels() {
    for label in [
        "2026-07-14 07:00:00Z",
        "2026-07-14 07:00:00+03:00",
        "not a timestamp",
    ] {
        let csv = format!("ts_local,open,high,low,close,volume\n{label},104,106,100,104,1\n");
        assert!(csv::Reader::from_reader(csv.as_bytes())
            .deserialize::<Bar>()
            .next()
            .unwrap()
            .is_err());
    }
    let csv = "ts_local,open,high,low,close,volume\n2026-07-14 07:00:00,104,106,100,104,1\n";
    let b = csv::Reader::from_reader(csv.as_bytes())
        .deserialize::<Bar>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(b.ts_local, bar(0, 0, 104.0).ts_local);
}

#[test]
fn immutable_output_directories_are_rejected_before_loading_data() {
    let root = tempfile::tempdir().unwrap();
    let handoff = root.path().join("handoff");
    let reference = root.path().join("reference");
    std::fs::create_dir(&handoff).unwrap();
    std::fs::create_dir(&reference).unwrap();
    for output in [handoff.join("candidate"), reference.join("candidate")] {
        let err = super::run(&handoff, &reference, &output).unwrap_err();
        assert!(err.to_string().contains("immutable"));
        assert!(!output.exists());
    }
}
