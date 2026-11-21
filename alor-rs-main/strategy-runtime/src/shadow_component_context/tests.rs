use super::*;
use crate::shadow_calendar::{Definition, Session, SCHEMA};
use crate::BarEvent;

fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}
fn calendar() -> Calendar {
    Calendar::new(Definition {
        schema_version: SCHEMA.into(),
        calendar_version: "synthetic-component-test-v1".into(),
        reference_sha256: "a".repeat(64),
        symbol: "IMOEXF".into(),
        first_date: date("2026-11-12"),
        last_date: date("2026-11-16"),
        sessions: (12..=16)
            .map(|n| {
                let weekend = n == 14 || n == 15;
                Session {
                    date: date(&format!("2026-11-{n}")),
                    exchange_trade_date: None,
                    kind: if weekend {
                        SessionKind::Dsvd
                    } else {
                        SessionKind::Regular
                    },
                    context_open_minute: Some(if weekend { 590 } else { 420 }),
                    close_minute: Some(if weekend { 1140 } else { 1430 }),
                    main_open_minute: Some(600),
                    main_close_minute: Some(1140),
                    breaks: vec![],
                    note: "synthetic features, no real receipt/admission proof".into(),
                }
            })
            .collect(),
    })
    .unwrap()
}
fn request(arm: Arm) -> Request {
    Request {
        arm,
        model_sha256: "b".repeat(64),
        receipt_stream: "synthetic-prefix-v1".into(),
        evidence: Evidence::NumericalFixture,
    }
}
fn rows(c: &Calendar) -> Vec<Receipt> {
    let mut result = vec![];
    for (day, close, spread) in [
        ("2026-11-12", 100., 2.),
        ("2026-11-13", 110., 4.),
        ("2026-11-14", 120., 3.),
        ("2026-11-15", 90., 5.),
    ] {
        for ts in c.expected_labels_utc(date(day)) {
            result.push(Receipt {
                sequence: result.len() as u64 + 1,
                source_id: format!("synthetic-{ts}"),
                bar: BarEvent {
                    symbol: "IMOEXF".into(),
                    close_time_utc: ts,
                    close,
                    o: close,
                    h: close + spread,
                    l: close - spread,
                    v: 1.,
                    origin: DataOrigin::Replay,
                },
                received_at_utc: Some(ts + 600),
                durable_at_utc: Some(ts + 600),
            });
        }
    }
    result
}
fn resequence(rows: &mut [Receipt]) {
    rows.sort_by_key(|r| r.bar.close_time_utc);
    for (i, row) in rows.iter_mut().enumerate() {
        row.sequence = i as u64 + 1;
    }
}
fn barrier(c: &Calendar, rows: &[Receipt], now: i64) -> Barrier {
    Barrier {
        receipt_stream: "synthetic-prefix-v1".into(),
        through_utc: c.bounds_utc(date("2026-11-16")).unwrap().0,
        last_sequence: rows.len() as u64,
        ingress_sha256: prefix_sha256(rows).unwrap(),
        attested_at_utc: now,
    }
}
fn snapshot(c: &Calendar, arm: Arm, rows: &[Receipt]) -> FrozenComponents {
    let day = date("2026-11-16");
    let now = c.bounds_utc(day).unwrap().0 + 600;
    freeze(c, request(arm), day, now, rows, &barrier(c, rows, now)).unwrap()
}

#[test]
fn a_a_and_a_b_keep_independent_close_range_return_and_source_dates() {
    let c = calendar();
    let rows = rows(&c);
    let aa = snapshot(&c, Arm::ImoAa, &rows);
    let ab = snapshot(&c, Arm::ImoAb, &rows);
    assert!(aa.dependency_ready && ab.dependency_ready);
    assert_eq!(aa.mr, ab.mr);
    let mr = ab.mr.inputs.as_ref().unwrap();
    let bo = ab.bo.inputs.as_ref().unwrap();
    assert_eq!(
        (mr.previous_close, mr.previous_range, mr.previous2_close),
        (110., 8., 100.)
    );
    assert_eq!(
        (bo.previous_close, bo.previous_range, bo.previous2_close),
        (90., 10., 120.)
    );
    assert_eq!(mr.previous_return, 110. / 100. - 1.);
    assert_eq!(bo.previous_return, -0.25);
    assert_eq!(bo.entry_scale, bo.stop_scale);
    assert_eq!(ab.mr.selection.previous, Some(date("2026-11-13")));
    assert_eq!(ab.bo.selection.previous, Some(date("2026-11-15")));
    assert_eq!(ab.bo.selection.previous2, Some(date("2026-11-14")));
}

#[test]
fn oi_arm_has_identical_price_features_to_a_b_and_does_not_gate_mr() {
    let c = calendar();
    let rows = rows(&c);
    let ab = snapshot(&c, Arm::ImoAb, &rows);
    let oi = snapshot(&c, Arm::ImoAbOi, &rows);
    assert_eq!(ab.mr, oi.mr);
    assert_eq!(ab.bo, oi.bo);
    assert_eq!(ab.session_trace, oi.session_trace);
    assert_eq!(serde_json::to_value(oi.request.arm).unwrap(), "IMO_AB_OI");
}

#[test]
fn incomplete_latest_dsvd_holds_combo_but_does_not_fall_back_or_erase_mr() {
    let c = calendar();
    let mut rows = rows(&c);
    rows.pop(); // Missing Sunday terminal bin: count alone is not enough.
    let frozen = snapshot(&c, Arm::ImoAb, &rows);
    assert!(!frozen.dependency_ready && frozen.mr.selection.ready && !frozen.bo.selection.ready);
    assert!(frozen.mr.inputs.is_some() && frozen.bo.inputs.is_none());
    assert_eq!(frozen.bo.selection.previous, Some(date("2026-11-15")));
    assert_eq!(
        frozen.bo.selection.unresolved_dates,
        vec![date("2026-11-15")]
    );
    let sunday = frozen.session_trace.last().unwrap();
    assert_eq!(sunday.bo, Disposition::Previous);
    assert_eq!(sunday.mr, Disposition::PolicyExcluded);
}

#[test]
fn absent_scheduled_regular_session_blocks_both_not_an_older_anchor() {
    let c = calendar();
    let mut rows = rows(&c);
    rows.retain(|r| local(r.bar.close_time_utc).unwrap().date() != date("2026-11-13"));
    resequence(&mut rows);
    let frozen = snapshot(&c, Arm::ImoAa, &rows);
    assert!(!frozen.dependency_ready);
    assert_eq!(frozen.mr.selection.previous, Some(date("2026-11-13")));
    assert!(frozen.mr.previous.is_none());
    assert!(frozen.mr.inputs.is_none());
}

#[test]
fn regular_service_extremes_never_change_features_but_trace_counts_ignored_rows() {
    let c = calendar();
    let mut data = rows(&c);
    let expected = snapshot(&c, Arm::ImoAb, &data);
    let mut service = data[0].clone();
    service.source_id = "synthetic-service".into();
    service.bar.close_time_utc -= 600;
    service.received_at_utc = Some(service.bar.close_time_utc + 600);
    service.durable_at_utc = service.received_at_utc;
    service.bar.h = 10_000.;
    data.push(service);
    resequence(&mut data);
    let actual = snapshot(&c, Arm::ImoAb, &data);
    let inputs = |f: &FrozenComponents| (f.mr.inputs.clone(), f.bo.inputs.clone());
    assert_eq!(inputs(&actual), inputs(&expected));
    assert_eq!(actual.session_trace[0].coverage.ignored_labels, 1);
}

#[test]
fn dsvd_auction_label_is_included_for_b_context_not_blanket_service_filtered() {
    let c = calendar();
    let mut data = rows(&c);
    let sunday = c.expected_labels_utc(date("2026-11-15"))[0];
    data.iter_mut()
        .find(|r| r.bar.close_time_utc == sunday)
        .unwrap()
        .bar
        .h = 200.;
    let frozen = snapshot(&c, Arm::ImoAb, &data);
    assert_eq!(frozen.bo.inputs.unwrap().previous_high, 200.);
    assert_eq!(frozen.mr.inputs.unwrap().previous_high, 114.);
}

#[test]
fn late_first_event_does_not_relax_cutoff_and_equality_is_not_timely() {
    let c = calendar();
    let mut data = rows(&c);
    let cutoff = c.bounds_utc(date("2026-11-16")).unwrap().0;
    data.last_mut().unwrap().received_at_utc = Some(cutoff);
    data.last_mut().unwrap().durable_at_utc = Some(cutoff);
    let frozen = snapshot(&c, Arm::ImoAb, &data);
    assert_eq!(frozen.cutoff_utc, cutoff);
    assert_eq!(frozen.frozen_at_utc, cutoff + 600);
    assert!(!frozen.dependency_ready && frozen.bo.inputs.is_none());
    data.last_mut().unwrap().received_at_utc = Some(cutoff - 1);
    data.last_mut().unwrap().durable_at_utc = Some(cutoff - 1);
    assert!(snapshot(&c, Arm::ImoAb, &data).dependency_ready);
}

#[test]
fn future_receipts_cannot_change_pre_open_features() {
    let c = calendar();
    let mut data = rows(&c);
    let expected = snapshot(&c, Arm::ImoAb, &data);
    let cutoff = expected.cutoff_utc;
    let mut future = data.last().unwrap().clone();
    future.bar.close_time_utc = cutoff;
    future.bar.h = 9999.;
    future.source_id = "synthetic-future".into();
    future.sequence += 1;
    future.received_at_utc = Some(cutoff + 600);
    future.durable_at_utc = future.received_at_utc;
    data.push(future);
    let actual = snapshot(&c, Arm::ImoAb, &data);
    assert_eq!(actual.mr, expected.mr);
    assert_eq!(actual.bo, expected.bo);
    assert_eq!(actual.session_trace, expected.session_trace);
}

#[test]
fn wrong_barrier_or_missing_receipt_is_never_a_ready_context() {
    let c = calendar();
    let day = date("2026-11-16");
    let data = rows(&c);
    let now = c.bounds_utc(day).unwrap().0;
    for variant in 0..4 {
        let mut b = barrier(&c, &data, now);
        match variant {
            0 => b.ingress_sha256 = "0".repeat(64),
            1 => b.last_sequence -= 1,
            2 => b.through_utc -= 1,
            _ => b.receipt_stream.push('2'),
        }
        assert!(freeze(&c, request(Arm::ImoAb), day, now, &data, &b).is_err());
    }
    let mut data = data;
    data[0].received_at_utc = None;
    assert!(freeze(
        &c,
        request(Arm::ImoAb),
        day,
        now,
        &data,
        &barrier(&c, &data, now)
    )
    .is_err());
}

#[test]
fn conflicting_duplicate_labels_and_replay_disguised_as_observed_fail() {
    let c = calendar();
    let day = date("2026-11-16");
    let mut data = rows(&c);
    let now = c.bounds_utc(day).unwrap().0;
    let mut req = request(Arm::ImoAb);
    req.evidence = Evidence::DeclaredAsObserved;
    assert!(freeze(&c, req, day, now, &data, &barrier(&c, &data, now)).is_err());
    let mut repeated = data[0].clone();
    repeated.source_id = "synthetic-conflict".into();
    repeated.bar.close += 1.;
    data.push(repeated);
    resequence(&mut data);
    assert!(freeze(
        &c,
        request(Arm::ImoAb),
        day,
        now,
        &data,
        &barrier(&c, &data, now)
    )
    .is_err());
}

#[test]
fn flat_but_complete_context_is_data_ready_not_a_positive_range_signal() {
    let c = calendar();
    let mut data = rows(&c);
    for r in &mut data {
        r.bar.o = 100.;
        r.bar.h = 100.;
        r.bar.l = 100.;
        r.bar.close = 100.;
    }
    let frozen = snapshot(&c, Arm::ImoAb, &data);
    assert!(frozen.dependency_ready);
    assert!(!frozen.mr.inputs.unwrap().signal_context_valid);
    assert!(!frozen.bo.inputs.unwrap().signal_context_valid);
}
