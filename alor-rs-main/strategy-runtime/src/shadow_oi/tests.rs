use super::*;
use crate::shadow_calendar::{Definition, Session, SCHEMA as CALENDAR_SCHEMA};
use serde_json::json;

fn day(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

fn calendar(symbol: &str) -> Calendar {
    Calendar::new(Definition {
        schema_version: CALENDAR_SCHEMA.into(),
        calendar_version: "synthetic-test-only-v1".into(),
        reference_sha256: "a".repeat(64),
        symbol: symbol.into(),
        first_date: day("2026-11-12"),
        last_date: day("2026-11-16"),
        sessions: (12..=16)
            .map(|n| {
                let dsvd = n == 14 || n == 15;
                Session {
                    date: day(&format!("2026-11-{n}")),
                    exchange_trade_date: None,
                    kind: if dsvd {
                        SessionKind::Dsvd
                    } else {
                        SessionKind::Regular
                    },
                    context_open_minute: Some(if dsvd { 590 } else { 420 }),
                    close_minute: Some(if dsvd { 1140 } else { 1430 }),
                    main_open_minute: Some(600),
                    main_close_minute: Some(1140),
                    breaks: vec![],
                    note: "synthetic OI fixture, not historical PIT evidence".into(),
                }
            })
            .collect(),
    })
    .unwrap()
}

fn plan() -> Plan {
    Plan::new(&calendar("IMOEXF"), day("2026-11-16"), &"b".repeat(64)).unwrap()
}

fn receipt(p: &Plan) -> Receipt {
    Receipt {
        url: HISTORY_URL.into(),
        params: p.request_params(),
        http_status: 200,
        server_date: Some(p.at(6, 50).to_rfc2822()),
        requested_at_utc: p.at(6, 50),
        received_at_utc: p.at(6, 50) + Duration::seconds(1),
        durable_at_utc: p.at(6, 50) + Duration::seconds(2),
        clock_offset_ms: Some(0),
    }
}

fn payload(prior: f64, prior2: f64) -> Value {
    json!({"history": {
        "columns": ["BOARDID", "SECID", "TRADEDATE", "OPENPOSITION"],
        "data": [["RFUD", "IMOEXF", "2026-11-12", prior2], ["RFUD", "IMOEXF", "2026-11-13", prior]]
    }, "history.cursor": {"columns": ["INDEX", "TOTAL", "PAGESIZE"], "data": [[0, 2, 100]]}})
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

#[test]
fn plan_uses_exact_regular_source_dates_not_weekend_price_context() {
    let p = plan();
    assert_eq!(p.previous, day("2026-11-13"));
    assert_eq!(p.previous2, day("2026-11-12"));
    assert!(Plan::new(&calendar("USDRUBF"), p.decision_date, &p.model_sha256).is_err());
    assert!(Plan::new(&calendar("IMOEXF"), day("2026-11-14"), &p.model_sha256).is_err());
    assert!(Plan::new(&calendar("IMOEXF"), day("2026-11-17"), &p.model_sha256).is_err());
    assert!(Plan::new(&calendar("IMOEXF"), p.decision_date, "not-a-pin").is_err());
}

#[test]
fn only_strictly_negative_change_allows_new_bo_and_zero_numerator_is_valid() {
    let p = plan();
    for (prior, expected) in [
        (90., Reason::Decreasing),
        (0., Reason::Decreasing),
        (100., Reason::NonDecreasing),
        (110., Reason::NonDecreasing),
    ] {
        let actual = evaluate(&p, &receipt(&p), &bytes(&payload(prior, 100.)));
        assert_eq!(actual.reason, expected);
        assert_eq!(actual.allow_new_bo(), prior < 100.);
        assert_eq!(actual.delta_oi, Some(prior / 100. - 1.));
    }
}

#[test]
fn missing_or_diagnostic_current_oi_never_substitutes_a_dated_source() {
    let p = plan();
    let value = json!({"securities": {"columns": ["PREVOPENPOSITION"], "data": [[90]]},
        "marketdata": {"columns": ["OPENPOSITION"], "data": [[90]]}});
    assert_eq!(
        evaluate(&p, &receipt(&p), &bytes(&value)).reason,
        Reason::InvalidSchema
    );
    let mut value = payload(90., 100.);
    value["history"]["data"].as_array_mut().unwrap().pop();
    value.as_object_mut().unwrap().remove("history.cursor");
    value["marketdata"] = json!({"OPENPOSITION": 90});
    assert_eq!(
        evaluate(&p, &receipt(&p), &bytes(&value)).reason,
        Reason::MissingSourceDate
    );
}

#[test]
fn duplicate_required_or_other_dates_are_rejected_not_collapsed() {
    let p = plan();
    let mut value = payload(90., 100.);
    value.as_object_mut().unwrap().remove("history.cursor");
    let duplicate = value["history"]["data"][0].clone();
    value["history"]["data"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert_eq!(
        evaluate(&p, &receipt(&p), &bytes(&value)).reason,
        Reason::DuplicateDate
    );
}

#[test]
fn wrong_symbol_board_future_old_or_noncanonical_date_is_rejected() {
    let p = plan();
    for (column, invalid) in [
        (0, "OTHER"),
        (1, "USDRUBF"),
        (2, "2026-11-16"),
        (2, "2026-11-11"),
        (2, "2026-11-13T00:00:00"),
        (2, "2026-1-13"),
    ] {
        let mut value = payload(90., 100.);
        value["history"]["data"][1][column] = json!(invalid);
        assert_eq!(
            evaluate(&p, &receipt(&p), &bytes(&value)).reason,
            Reason::WrongInstrumentOrDate
        );
    }
}

#[test]
fn invalid_values_and_zero_denominator_fail_closed() {
    let p = plan();
    for value in [
        json!(-1),
        Value::Null,
        json!("90"),
        json!(true),
        json!(1e100),
    ] {
        let mut data = payload(90., 100.);
        data["history"]["data"][1][3] = value;
        assert_eq!(
            evaluate(&p, &receipt(&p), &bytes(&data)).reason,
            Reason::InvalidValue
        );
    }
    assert_eq!(
        evaluate(&p, &receipt(&p), &bytes(&payload(90., 0.))).reason,
        Reason::ZeroDenominator
    );
    for raw in [
        br#"{"history":{"columns":[],"data":[NaN]}}"#.as_slice(),
        b"not json",
    ] {
        assert_eq!(
            evaluate(&p, &receipt(&p), raw).reason,
            Reason::InvalidSchema
        );
    }
}

#[test]
fn ragged_rows_duplicate_columns_and_duplicate_top_level_table_are_rejected() {
    let p = plan();
    for mutate in 0..3 {
        let mut value = payload(90., 100.);
        match mutate {
            0 => {
                value["history"]["data"][0].as_array_mut().unwrap().pop();
            }
            1 => value["history"]["columns"][3] = json!("SECID"),
            _ => value["history"]["columns"][3] = json!("NO_OPENPOSITION"),
        }
        assert_eq!(
            evaluate(&p, &receipt(&p), &bytes(&value)).reason,
            Reason::InvalidSchema
        );
    }
    let raw = br#"{"history":{"columns":[],"data":[]},"history":{"columns":[],"data":[]}}"#;
    assert_eq!(
        evaluate(&p, &receipt(&p), raw).reason,
        Reason::InvalidSchema
    );
}

#[test]
fn pagination_must_cover_the_entire_requested_result() {
    let p = plan();
    for row in [
        json!([1, 2, 100]),
        json!([0, 3, 100]),
        json!([0, 2, 1]),
        json!([0, "2", 100]),
    ] {
        let mut value = payload(90., 100.);
        value["history.cursor"]["data"][0] = row;
        assert_eq!(
            evaluate(&p, &receipt(&p), &bytes(&value)).reason,
            Reason::IncompletePage
        );
    }
}

#[test]
fn public_request_identity_is_exact_and_cannot_include_credentials_or_extra_dates() {
    let p = plan();
    let raw = bytes(&payload(90., 100.));
    for variant in 0..3 {
        let mut r = receipt(&p);
        match variant {
            0 => r.url.push_str("?token=must-not-be-saved"),
            1 => {
                r.params.insert("from".into(), "2026-11-11".into());
            }
            _ => {
                r.params.insert("token".into(), "must-not-be-saved".into());
            }
        }
        assert_eq!(evaluate(&p, &r, &raw).reason, Reason::WrongRequest);
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_path_buf()).unwrap();
        assert!(store.archive(&p, r, &raw).is_err());
        assert_eq!(
            std::fs::read_dir(root.path().join("captures"))
                .unwrap()
                .count(),
            0
        );
    }
    let mut r = receipt(&p);
    r.http_status = 503;
    assert_eq!(evaluate(&p, &r, &raw).reason, Reason::HttpFailure);
}

#[test]
fn clock_server_date_and_request_order_are_validated() {
    let p = plan();
    let raw = bytes(&payload(90., 100.));
    for variant in 0..8 {
        let mut r = receipt(&p);
        match variant {
            0 => r.clock_offset_ms = None,
            1 => r.clock_offset_ms = Some(i64::MIN),
            2 => r.received_at_utc = r.requested_at_utc - Duration::seconds(1),
            3 => r.durable_at_utc = r.received_at_utc - Duration::seconds(1),
            4 => r.requested_at_utc -= Duration::seconds(31),
            5 => r.server_date = None,
            6 => r.server_date = Some("invalid".into()),
            _ => r.server_date = Some((p.at(6, 50) - Duration::minutes(10)).to_rfc2822()),
        }
        assert_eq!(
            evaluate(&p, &r, &raw).reason,
            Reason::InvalidClock,
            "variant {variant}"
        );
    }
}

#[test]
fn receipt_and_durability_deadline_are_inclusive_but_never_backdated() {
    let p = plan();
    let raw = bytes(&payload(90., 100.));
    let mut r = receipt(&p);
    r.durable_at_utc = p.at(6, 55);
    assert!(evaluate(&p, &r, &raw).allow_new_bo());
    r.durable_at_utc += Duration::nanoseconds(1);
    assert_eq!(evaluate(&p, &r, &raw).reason, Reason::LateReceipt);
    let mut r = receipt(&p);
    r.requested_at_utc = p.at(6, 45) - Duration::seconds(1);
    r.received_at_utc = r.requested_at_utc + Duration::seconds(1);
    r.durable_at_utc = r.received_at_utc;
    r.server_date = Some(r.received_at_utc.to_rfc2822());
    assert_eq!(evaluate(&p, &r, &raw).reason, Reason::OutsideCaptureWindow);
}

#[test]
fn payload_size_is_bounded_even_with_a_timely_receipt() {
    let p = plan();
    assert_eq!(
        evaluate(&p, &receipt(&p), &vec![b' '; MAX_BYTES + 1]).reason,
        Reason::InvalidSchema
    );
}

#[test]
fn frozen_decision_survives_restart_and_new_revision_cannot_replace_it() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let capture = store
        .archive(&p, receipt(&p), &bytes(&payload(90., 100.)))
        .unwrap();
    let frozen = store.freeze(&p, Some(&capture), p.at(6, 51)).unwrap();
    assert!(frozen.allow_new_bo());
    let before = std::fs::read(root.path().join("decisions/2026-11-16.json")).unwrap();
    let restarted = Store::new(root.path().to_path_buf()).unwrap();
    let revision = restarted
        .archive(&p, receipt(&p), &bytes(&payload(110., 100.)))
        .unwrap();
    assert_eq!(
        restarted.freeze(&p, Some(&revision), p.at(7, 30)).unwrap(),
        frozen
    );
    assert_eq!(
        std::fs::read(root.path().join("decisions/2026-11-16.json")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("captures"))
            .unwrap()
            .count(),
        2
    );
    assert_eq!(restarted.load(&p).unwrap(), Some(frozen));
}

#[test]
fn missing_is_frozen_only_at_deadline_and_cannot_enable_later() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    assert!(store.freeze(&p, None, p.at(6, 44)).is_err());
    assert!(store.freeze(&p, None, p.at(6, 54)).is_err());
    let frozen = store.freeze(&p, None, p.at(6, 55)).unwrap();
    assert!(!frozen.allow_new_bo());
    assert_eq!(frozen.reason(), Reason::Missing);
    let capture = store
        .archive(&p, receipt(&p), &bytes(&payload(90., 100.)))
        .unwrap();
    assert_eq!(
        store.freeze(&p, Some(&capture), p.at(7, 1)).unwrap(),
        frozen
    );
}

#[test]
fn late_first_freeze_disables_bo_even_with_a_timely_raw_capture() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let capture = store
        .archive(&p, receipt(&p), &bytes(&payload(90., 100.)))
        .unwrap();
    assert!(store.freeze(&p, Some(&capture), p.at(6, 50)).is_err());
    let frozen = store
        .freeze(&p, Some(&capture), p.at(6, 55) + Duration::nanoseconds(1))
        .unwrap();
    assert!(!frozen.allow_new_bo());
    assert_eq!(frozen.reason(), Reason::LateFreeze);
}

#[test]
fn invalid_capture_keeps_failure_evidence_and_retry_window() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let capture = store.archive(&p, receipt(&p), b"malformed ISS").unwrap();
    assert!(store.freeze(&p, Some(&capture), p.at(6, 51)).is_err());
    let frozen = store.freeze(&p, Some(&capture), p.at(6, 55)).unwrap();
    assert_eq!(frozen.reason(), Reason::InvalidSchema);
    assert!(!frozen.allow_new_bo());
    assert_eq!(
        read_bounded(
            &root
                .path()
                .join("captures")
                .join(capture.id.to_string())
                .join("history.json")
        )
        .unwrap(),
        b"malformed ISS"
    );
}

#[test]
fn changed_model_cannot_reuse_frozen_day_without_explicit_rebuild() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    store.freeze(&p, None, p.at(6, 55)).unwrap();
    let mut changed = p;
    changed.model_sha256 = "c".repeat(64);
    assert!(store.load(&changed).is_err());
    assert!(store.freeze(&changed, None, changed.at(6, 55)).is_err());
}

#[test]
fn tampered_raw_or_missing_capture_is_not_silently_replaced_after_restart() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let capture = store
        .archive(&p, receipt(&p), &bytes(&payload(90., 100.)))
        .unwrap();
    store.freeze(&p, Some(&capture), p.at(6, 51)).unwrap();
    let path = root
        .path()
        .join("captures")
        .join(capture.id.to_string())
        .join("history.json");
    std::fs::write(&path, bytes(&payload(110., 100.))).unwrap();
    assert!(store.load(&p).is_err());
    assert!(store.freeze(&p, None, p.at(6, 55)).is_err());
    std::fs::remove_file(path).unwrap();
    assert!(store.load(&p).is_err());
}

#[test]
fn incomplete_capture_from_another_store_cannot_publish_a_daily_decision() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let foreign = Store::new(other.path().to_path_buf()).unwrap();
    let capture = foreign
        .archive(&p, receipt(&p), &bytes(&payload(90., 100.)))
        .unwrap();
    assert!(store.freeze(&p, Some(&capture), p.at(6, 51)).is_err());
    assert!(store.load(&p).unwrap().is_none());
}

#[test]
fn racing_freezes_publish_exactly_one_immutable_result() {
    let p = plan();
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_path_buf()).unwrap();
    let a = store
        .archive(&p, receipt(&p), &bytes(&payload(90., 100.)))
        .unwrap();
    let b = store
        .archive(&p, receipt(&p), &bytes(&payload(110., 100.)))
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            barrier.wait();
            store.freeze(&p, Some(&a), p.at(6, 51)).unwrap()
        });
        let right = scope.spawn(|| {
            barrier.wait();
            store.freeze(&p, Some(&b), p.at(6, 51)).unwrap()
        });
        assert_eq!(left.join().unwrap(), right.join().unwrap());
    });
    assert_eq!(
        std::fs::read_dir(root.path().join("decisions"))
            .unwrap()
            .count(),
        1
    );
}
