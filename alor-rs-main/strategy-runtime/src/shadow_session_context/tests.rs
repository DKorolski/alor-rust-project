use super::*;

#[path = "tests/export.rs"]
mod export;

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}
fn epoch(s: &str) -> i64 {
    (NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap() - Duration::hours(3))
        .and_utc()
        .timestamp()
}
fn identity(profile: Profile, policy: Policy) -> Identity {
    Identity {
        profile,
        symbol: if profile == Profile::ImoexfReaction07 {
            "IMOEXF"
        } else {
            "USDRUBF"
        }
        .into(),
        model_config_sha256: "a".repeat(64),
        receipt_stream: "fixture-only".into(),
        policy,
        excluded_dates: BTreeSet::new(),
    }
}
fn receipt(s: &Store, label: i64) -> Receipt {
    let sequence = s.last_sequence() + 1;
    Receipt {
        sequence,
        source_id: format!("source-{sequence}"),
        bar: BarEvent {
            symbol: s.identity.symbol.clone(),
            close_time_utc: label,
            o: 100.,
            h: 110.,
            l: 90.,
            close: 105.,
            v: 1.,
            origin: DataOrigin::Live,
        },
        received_at_utc: Some(label + 600),
        durable_at_utc: Some(label + 600),
    }
}
fn day(s: &mut Store, d: &str, full: bool) {
    let weekend = date(d).weekday().num_days_from_monday() >= 5;
    let start = if weekend {
        epoch(&format!("{d} 09:50"))
    } else {
        s.identity.cutoff(date(d))
    };
    let end = epoch(&format!("{d} {}", if weekend { "18:50" } else { "23:40" }));
    for ts in (start..=end).step_by(600) {
        if !full && ts == end {
            continue;
        }
        let r = receipt(s, ts);
        assert!(!s.ingest(r, ts + 600).unwrap().order_emission_allowed());
    }
}
fn barrier(s: &Store, d: &str, now: i64) -> Barrier {
    Barrier {
        receipt_stream: s.identity.receipt_stream.clone(),
        through_utc: s.identity.cutoff(date(d)),
        last_sequence: s.last_sequence(),
        ingress_sha256: s.ingress_sha256().unwrap(),
        attested_at_utc: now,
    }
}
fn freeze(s: &mut Store, d: &str, now: i64) -> Frozen {
    let tmp = tempfile::tempdir().unwrap();
    s.freeze_and_save_new(
        date(d),
        now,
        &barrier(s, d, now),
        &tmp.path().join("frozen.json"),
    )
    .unwrap()
    .clone()
}
fn store() -> Store {
    Store::new(identity(
        Profile::ImoexfReaction07,
        Policy::WeekendContextNoTrades,
    ))
    .unwrap()
}

#[test]
fn checkpoint_preserves_decimal_quote_fingerprints_exactly() {
    let mut s = store();
    let start = epoch("2026-10-23 07:00");
    for slot in 0..101 {
        let ts = start + slot * 600;
        let mut r = receipt(&s, ts);
        r.bar.o = 100.12;
        r.bar.close = 99.6;
        r.bar.v = 0.00006575119054451348;
        s.ingest(r, ts + 600).unwrap();
    }
    let now = epoch("2026-10-26 07:00");
    freeze(&mut s, "2026-10-26", now);
    let bytes = s.checkpoint_bytes().unwrap();
    let restored = Store::restore(s.identity(), &bytes, now).unwrap();
    assert_eq!(
        s.frozen(date("2026-10-26")),
        restored.frozen(date("2026-10-26"))
    );
    assert_eq!(
        s.ingress_sha256().unwrap(),
        restored.ingress_sha256().unwrap()
    );
    assert_eq!(bytes, restored.checkpoint_bytes().unwrap());
}

#[test]
fn paired_weekend_anchors_and_tuesday_return_are_separate_calendar_sessions() {
    for profile in [
        Profile::ImoexfReaction07,
        Profile::UsdrubfPreviousDay07,
        Profile::UsdrubfPreviousDay09,
    ] {
        for policy in [Policy::WeekdayOnly, Policy::WeekendContextNoTrades] {
            let mut s = Store::new(identity(profile, policy)).unwrap();
            for d in ["2026-10-22", "2026-10-23", "2026-10-24", "2026-10-25"] {
                day(&mut s, d, true);
            }
            let monday = s.identity.cutoff(date("2026-10-26"));
            let f = freeze(&mut s, "2026-10-26", monday);
            assert!(f.ready());
            assert_eq!(
                f.previous.as_ref().unwrap().session_date,
                date(if policy == Policy::WeekdayOnly {
                    "2026-10-23"
                } else {
                    "2026-10-25"
                })
            );
            assert_eq!(
                f.previous2.as_ref().unwrap().session_date,
                date(if policy == Policy::WeekdayOnly {
                    "2026-10-22"
                } else {
                    "2026-10-24"
                })
            );
            day(&mut s, "2026-10-26", true);
            let tuesday = s.identity.cutoff(date("2026-10-27"));
            let t = freeze(&mut s, "2026-10-27", tuesday);
            assert_eq!(
                t.previous.as_ref().unwrap().session_date,
                date("2026-10-26")
            );
            assert_eq!(t.previous2, f.previous);
        }
    }
}

#[test]
fn exact_cutoff_and_durability_crossing_are_late_forever() {
    for profile in [Profile::ImoexfReaction07, Profile::UsdrubfPreviousDay09] {
        for delay in [0, 300] {
            let mut s = Store::new(identity(profile, Policy::WeekendContextNoTrades)).unwrap();
            day(&mut s, "2026-10-25", false);
            let cutoff = s.identity.cutoff(date("2026-10-26"));
            let mut r = receipt(&s, epoch("2026-10-25 18:50"));
            r.received_at_utc = Some(cutoff - 1);
            r.durable_at_utc = Some(cutoff + delay);
            assert_eq!(
                s.ingest(r, cutoff + 600).unwrap(),
                Route::Quarantined(Reject::AtOrAfterCutoff)
            );
            let f = freeze(&mut s, "2026-10-26", cutoff + 600);
            assert!(f.previous.is_none());
            let next = s.identity.cutoff(date("2026-10-27"));
            assert!(freeze(&mut s, "2026-10-27", next).previous.is_none());
        }
    }
}

#[test]
fn delayed_processing_before_freeze_and_restart_preserve_original_provenance() {
    let mut s = store();
    day(&mut s, "2026-10-25", false);
    let cutoff = s.identity.cutoff(date("2026-10-26"));
    let mut r = receipt(&s, epoch("2026-10-25 18:50"));
    r.received_at_utc = Some(cutoff - 2);
    r.durable_at_utc = Some(cutoff - 1);
    assert_eq!(
        s.ingest(r.clone(), cutoff + 300).unwrap(),
        Route::ContextOnly
    );
    s = Store::restore(s.identity(), &s.checkpoint_bytes().unwrap(), cutoff + 300).unwrap();
    let f = freeze(&mut s, "2026-10-26", cutoff + 600);
    assert_eq!(
        f.previous.as_ref().unwrap().latest_received_at_utc,
        cutoff - 2
    );
    assert_eq!(
        f.previous.as_ref().unwrap().latest_durable_at_utc,
        cutoff - 1
    );
    let restored =
        Store::restore(s.identity(), &s.checkpoint_bytes().unwrap(), cutoff + 600).unwrap();
    assert_eq!(restored.frozen(date("2026-10-26")), Some(&f));
    assert_eq!(restored.canonical.get(&r.bar.close_time_utc), Some(&r));
}

#[test]
fn post_freeze_backdated_input_cannot_repair_today_or_tomorrow() {
    let mut s = store();
    day(&mut s, "2026-10-25", false);
    let cutoff = s.identity.cutoff(date("2026-10-26"));
    let f = freeze(&mut s, "2026-10-26", cutoff);
    let mut r = receipt(&s, epoch("2026-10-25 18:50"));
    r.received_at_utc = Some(cutoff - 2);
    r.durable_at_utc = Some(cutoff - 1);
    assert_eq!(
        s.ingest(r, cutoff + 60).unwrap(),
        Route::Quarantined(Reject::BehindFrozenBarrier)
    );
    assert_eq!(s.frozen(date("2026-10-26")), Some(&f));
    let next = s.identity.cutoff(date("2026-10-27"));
    assert!(freeze(&mut s, "2026-10-27", next).previous.is_none());
}

#[test]
fn duplicate_conflicting_and_out_of_order_receipts_never_rewrite_quotes() {
    let mut s = store();
    let ts = epoch("2026-10-25 10:00");
    let r = receipt(&s, ts);
    assert_eq!(s.ingest(r.clone(), ts + 600).unwrap(), Route::ContextOnly);
    let mut duplicate = r.clone();
    duplicate.sequence = 2;
    assert_eq!(s.ingest(duplicate, ts + 700).unwrap(), Route::Duplicate);
    let mut conflicting = r.clone();
    conflicting.sequence = 3;
    conflicting.bar.close = 109.;
    assert_eq!(
        s.ingest(conflicting, ts + 800).unwrap(),
        Route::Quarantined(Reject::ConflictingReceipt)
    );
    let mut conflicting = receipt(&s, ts);
    conflicting.bar.close = 109.;
    assert_eq!(
        s.ingest(conflicting, ts + 900).unwrap(),
        Route::Quarantined(Reject::ConflictingQuote)
    );
    let earlier = receipt(&s, ts - 600);
    assert_eq!(s.ingest(earlier, ts + 1000).unwrap(), Route::ContextOnly);
    assert_eq!(s.accepted_count(), 2);
    assert_eq!(s.canonical[&ts], r);
    let before = s.checkpoint_bytes().unwrap();
    assert!(s.ingest(receipt(&s, ts + 600), ts).is_err());
    assert_eq!(s.checkpoint_bytes().unwrap(), before);
}

#[test]
fn weekend_completeness_is_44_of_55_with_endpoints_not_ri_92() {
    for (count, first, end, accepted) in [
        (44, 590, 1130, true),
        (43, 590, 1130, false),
        (44, 610, 1130, false),
        (44, 590, 1120, false),
    ] {
        let mut s = store();
        let midnight = epoch("2026-10-25 00:00");
        let labels = (0..count - 1).map(|n| first + n * 10).chain([end]);
        for m in labels {
            let ts = midnight + m * 60;
            let r = receipt(&s, ts);
            s.ingest(r, ts + 600).unwrap();
        }
        let f = freeze(&mut s, "2026-10-26", epoch("2026-10-26 07:00"));
        assert_eq!(f.previous.is_some(), accepted);
        if accepted {
            assert_eq!(f.previous.unwrap().required_bars, 44);
        } else {
            assert_eq!(f.skipped.len(), 1);
        }
    }
}

#[test]
fn regular_completeness_transition_and_service_guard_are_not_weekend_rules() {
    let mut s = store();
    let service = receipt(&s, epoch("2026-07-13 08:50"));
    assert_eq!(
        s.ingest(service, epoch("2026-07-13 09:00")).unwrap(),
        Route::Quarantined(Reject::NonModelBar)
    );
    day(&mut s, "2026-07-13", true);
    let a = freeze(&mut s, "2026-07-14", epoch("2026-07-14 07:00"));
    assert_eq!(a.previous.unwrap().required_bars, 89);
    day(&mut s, "2026-07-14", false);
    let b = freeze(&mut s, "2026-07-15", epoch("2026-07-15 07:00"));
    assert_eq!(b.previous.unwrap().session_date, date("2026-07-13"));
    assert_eq!(b.skipped[0].required_bars, 101);
}

#[test]
fn provenance_contract_origin_and_calendar_fail_closed() {
    for (kind, expected) in [
        (0, Reject::MissingReceipt),
        (1, Reject::MissingDurability),
        (2, Reject::WrongContract),
        (3, Reject::UnsupportedOrigin),
        (4, Reject::BarNotClosed),
        (5, Reject::InvalidClock),
        (6, Reject::InvalidOhlcv),
        (7, Reject::ExcludedDate),
    ] {
        let mut s = store();
        let ts = epoch("2026-10-25 18:50");
        let mut r = receipt(&s, ts);
        match kind {
            0 => r.received_at_utc = None,
            1 => r.durable_at_utc = None,
            2 => r.bar.symbol = "USDRUBF".into(),
            3 => r.bar.origin = DataOrigin::Replay,
            4 => r.received_at_utc = Some(ts),
            5 => r.durable_at_utc = Some(ts + 700),
            6 => r.bar.l = 120.,
            _ => {
                s.identity.excluded_dates.insert(date("2026-10-25"));
            }
        }
        assert_eq!(s.ingest(r, ts + 600).unwrap(), Route::Quarantined(expected));
        assert_eq!(s.accepted_count(), 0);
    }
}

#[test]
fn invalid_barrier_and_storage_failure_publish_no_frozen_context() {
    let mut s = store();
    day(&mut s, "2026-10-23", true);
    let now = epoch("2026-10-26 07:00");
    let tmp = tempfile::tempdir().unwrap();
    let bytes = s.checkpoint_bytes().unwrap();
    for mutation in 0..4 {
        let mut b = barrier(&s, "2026-10-26", now);
        match mutation {
            0 => b.last_sequence -= 1,
            1 => b.ingress_sha256 = "bad".into(),
            2 => b.through_utc -= 1,
            _ => b.receipt_stream = "other".into(),
        }
        assert!(s
            .freeze_and_save_new(date("2026-10-26"), now, &b, &tmp.path().join("frozen.json"))
            .is_err());
        assert_eq!(s.checkpoint_bytes().unwrap(), bytes);
    }
    let b = barrier(&s, "2026-10-26", now);
    assert!(s
        .freeze_and_save_new(
            date("2026-10-26"),
            now,
            &b,
            &tmp.path().join("missing/frozen.json")
        )
        .is_err());
    assert_eq!(s.checkpoint_bytes().unwrap(), bytes);
    s.save_new_checkpoint(&tmp.path().join("existing.json"))
        .unwrap();
    assert!(s
        .freeze_and_save_new(
            date("2026-10-26"),
            now,
            &b,
            &tmp.path().join("existing.json")
        )
        .is_err());
    assert_eq!(fs::read(tmp.path().join("existing.json")).unwrap(), bytes);
    assert!(s.frozen(date("2026-10-26")).is_none());
}

#[test]
fn restore_rejects_identity_tampering_and_inconsistent_rehashed_snapshot() {
    let mut s = store();
    day(&mut s, "2026-10-25", true);
    let now = epoch("2026-10-26 07:00");
    freeze(&mut s, "2026-10-26", now);
    let bytes = s.checkpoint_bytes().unwrap();
    let mut wrong = s.identity.clone();
    wrong.policy = Policy::WeekdayOnly;
    assert!(Store::restore(&wrong, &bytes, now).is_err());
    assert!(Store::restore(s.identity(), &bytes[..bytes.len() - 1], now).is_err());
    let mut envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    let Operation::Freeze { value } = envelope.payload.operations.last_mut().unwrap() else {
        panic!()
    };
    value.previous.as_mut().unwrap().close += 1.;
    envelope.sha256 = hash(&envelope.payload).unwrap();
    assert!(Store::restore(s.identity(), &serde_json::to_vec(&envelope).unwrap(), now).is_err());
    assert!(Store::restore(s.identity(), &bytes, now - 1).is_err());
}
