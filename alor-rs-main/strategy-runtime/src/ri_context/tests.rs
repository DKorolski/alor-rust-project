use super::*;

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}
fn epoch(text: &str) -> i64 {
    (NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap() - Duration::hours(3))
        .and_utc()
        .timestamp()
}
fn identity(policy: ContextPolicy) -> ContextIdentity {
    ContextIdentity {
        profile_id: "ri_context_pair_v1".into(),
        symbol: "RIZ6".into(),
        numerical_config_sha256: "a".repeat(64),
        model_feed_policy: "micro_regular_v1".into(),
        receipt_stream: "test.durable_ri_ingress".into(),
        policy,
        model_open_hour: 7,
        excluded_dates: BTreeSet::from([date("2026-11-04")]),
    }
}
fn store() -> RiContextStore {
    RiContextStore::new(identity(ContextPolicy::RiConfirmedClosingContextV1)).unwrap()
}
fn input(sequence: u64, label: i64, origin: DataOrigin, received: i64) -> ContextInput {
    ContextInput {
        sequence,
        bar: BarEvent {
            symbol: "RIZ6".into(),
            close_time_utc: label,
            o: 90000.,
            h: 91000.,
            l: 89000.,
            close: 90000.,
            v: 10.,
            origin,
        },
        received_at_utc: Some(received),
        durable_at_utc: Some(received),
    }
}
fn recovery(s: &RiContextStore, received: &str) -> ContextInput {
    input(
        s.last_sequence() + 1,
        epoch("2026-10-12 23:40"),
        DataOrigin::HistoryGap,
        epoch(received),
    )
}
fn ingest_day(s: &mut RiContextStore, day: &str, include_close: bool) {
    let d = date(day);
    let start = s.state.identity.cutoff(d);
    let end = epoch(&format!(
        "{day} {}",
        if include_close { "23:40" } else { "23:30" }
    ));
    for ts in (start..=end).step_by(600) {
        let i = input(s.last_sequence() + 1, ts, DataOrigin::Live, ts + 600);
        assert_eq!(s.ingest(i, ts + 600).unwrap(), ContextRoute::ContextOnly);
    }
}
// Fixtures act as a trusted durable collector; production needs a real ingress attestation.
fn barrier(s: &RiContextStore, cutoff: i64, now: i64) -> ReceiptBarrier {
    ReceiptBarrier {
        receipt_stream: s.state.identity.receipt_stream.clone(),
        through_utc: cutoff,
        last_sequence: s.last_sequence(),
        ingress_sha256: s.ingress_sha256().unwrap(),
        attested_at_utc: now,
    }
}
fn freeze(s: &mut RiContextStore, day: &str, now: &str) -> FrozenContext {
    let d = date(day);
    let b = barrier(s, s.state.identity.cutoff(d), epoch(now));
    let tmp = tempfile::tempdir().unwrap();
    s.freeze_and_save_new(d, epoch(now), &b, &tmp.path().join("context.json"))
        .unwrap()
        .clone()
}

#[test]
fn recovery_is_context_only_and_original_provenance_is_preserved() {
    let mut s = store();
    let i = recovery(&s, "2026-10-13 03:00");
    let route = s.ingest(i.clone(), epoch("2026-10-13 06:30")).unwrap();
    assert!(route.context_update_allowed());
    assert!(!route.order_emission_allowed());
    assert_eq!(s.state.canonical[&i.bar.close_time_utc], i);
    assert_eq!(
        s.state.canonical[&i.bar.close_time_utc].bar.origin,
        DataOrigin::HistoryGap
    );
}

#[test]
fn controls_have_same_input_digest_but_only_candidate_accepts_closing_gap() {
    let mut candidate = store();
    let mut control = RiContextStore::new(identity(ContextPolicy::RuntimeAcceptedOnly)).unwrap();
    for s in [&mut candidate, &mut control] {
        ingest_day(s, "2026-10-09", true);
        ingest_day(s, "2026-10-12", false);
    }
    let mut i = recovery(&candidate, "2026-10-13 03:00");
    i.bar.close = 90500.;
    assert_eq!(
        candidate
            .ingest(i.clone(), epoch("2026-10-13 03:00"))
            .unwrap(),
        ContextRoute::ContextOnly
    );
    assert_eq!(
        control.ingest(i, epoch("2026-10-13 03:00")).unwrap(),
        ContextRoute::Quarantined(QuarantineReason::RecoveryDisabled)
    );
    assert_eq!(
        candidate.ingress_sha256().unwrap(),
        control.ingress_sha256().unwrap()
    );
    let a = freeze(&mut candidate, "2026-10-13", "2026-10-13 07:10");
    let b = freeze(&mut control, "2026-10-13", "2026-10-13 07:10");
    assert!(a.ready() && b.ready());
    assert_eq!(a.previous.as_ref().unwrap().close, 90500.);
    assert_eq!(b.previous.as_ref().unwrap().close, 90000.);
    assert_eq!(
        a.previous2.as_ref().unwrap().close,
        b.previous2.as_ref().unwrap().close
    );
    assert_eq!(a.previous_return(), Some(90500. / 90000. - 1.));
    let frozen_inputs = candidate.frozen_anchor_inputs(date("2026-10-13")).unwrap();
    assert_eq!(frozen_inputs.len(), 202);
    assert_eq!(
        frozen_inputs.last().unwrap().bar.origin,
        DataOrigin::HistoryGap
    );
    assert_eq!(frozen_inputs.last().unwrap().bar.close, 90500.);
}

#[test]
fn missing_receipt_or_durability_never_invents_availability() {
    for (received, durable, reason) in [
        (
            None,
            Some(epoch("2026-10-13 03:00")),
            QuarantineReason::MissingReceipt,
        ),
        (
            Some(epoch("2026-10-13 03:00")),
            None,
            QuarantineReason::MissingDurability,
        ),
    ] {
        let mut s = store();
        let mut i = recovery(&s, "2026-10-13 03:00");
        i.received_at_utc = received;
        i.durable_at_utc = durable;
        assert_eq!(
            s.ingest(i, epoch("2026-10-13 07:10")).unwrap(),
            ContextRoute::Quarantined(reason)
        );
        assert!(s.state.canonical.is_empty());
    }
}

#[test]
fn receipt_exactly_at_cutoff_or_late_before_first_event_stays_quarantined() {
    for hour in [7, 9] {
        for minute in [0, 5] {
            let mut cfg = identity(ContextPolicy::RiConfirmedClosingContextV1);
            cfg.model_open_hour = hour;
            let mut s = RiContextStore::new(cfg).unwrap();
            let received = format!("2026-10-13 {hour:02}:{minute:02}");
            let i = recovery(&s, &received);
            assert_eq!(
                s.ingest(i, epoch(&format!("2026-10-13 {hour:02}:10")))
                    .unwrap(),
                ContextRoute::Quarantined(QuarantineReason::AtOrAfterCutoff)
            );
            let f = freeze(&mut s, "2026-10-13", &format!("2026-10-13 {hour:02}:10"));
            assert!(f.previous.is_none());
            let next = freeze(&mut s, "2026-10-14", &format!("2026-10-14 {hour:02}:10"));
            assert!(next.previous.is_none()); // No silent reuse on later days.
        }
    }
}

#[test]
fn durable_after_cutoff_is_late_even_if_received_before_cutoff() {
    let mut s = store();
    let mut i = recovery(&s, "2026-10-13 06:59");
    i.durable_at_utc = Some(epoch("2026-10-13 07:00"));
    assert_eq!(
        s.ingest(i, epoch("2026-10-13 07:10")).unwrap(),
        ContextRoute::Quarantined(QuarantineReason::AtOrAfterCutoff)
    );
}

#[test]
fn pre_cutoff_evidence_processed_late_is_used_only_before_freeze() {
    let mut s = store();
    ingest_day(&mut s, "2026-10-09", true);
    ingest_day(&mut s, "2026-10-12", false);
    let i = recovery(&s, "2026-10-13 06:50");
    assert_eq!(
        s.ingest(i, epoch("2026-10-13 07:10")).unwrap(),
        ContextRoute::ContextOnly
    );
    let f = freeze(&mut s, "2026-10-13", "2026-10-13 07:10");
    assert_eq!(f.cutoff_utc, epoch("2026-10-13 07:00"));
    assert_eq!(f.previous.as_ref().unwrap().distinct_bars, 101);
    assert_eq!(
        f.previous.as_ref().unwrap().latest_durable_at_utc,
        epoch("2026-10-13 06:50")
    );
}

#[test]
fn late_processing_cannot_rewrite_a_frozen_anchor_even_with_old_receipt() {
    let mut s = store();
    ingest_day(&mut s, "2026-10-09", true);
    ingest_day(&mut s, "2026-10-12", false);
    let before = freeze(&mut s, "2026-10-13", "2026-10-13 07:00");
    let i = recovery(&s, "2026-10-13 06:50");
    assert_eq!(
        s.ingest(i, epoch("2026-10-13 07:05")).unwrap(),
        ContextRoute::Quarantined(QuarantineReason::BehindFrozenBarrier)
    );
    assert_eq!(s.frozen(date("2026-10-13")), Some(&before));
    assert_eq!(
        freeze(&mut s, "2026-10-14", "2026-10-14 07:00")
            .previous
            .unwrap()
            .distinct_bars,
        100
    );
}

#[test]
fn barrier_requires_complete_consumption_hash_stream_and_cutoff() {
    let mut s = store();
    let i = recovery(&s, "2026-10-13 03:00");
    s.ingest(i, epoch("2026-10-13 03:00")).unwrap();
    let cutoff = epoch("2026-10-13 07:00");
    let good = barrier(&s, cutoff, cutoff);
    for kind in 0..5 {
        let mut bad = good.clone();
        match kind {
            0 => bad.last_sequence = 0,
            1 => bad.ingress_sha256 = "bad".into(),
            2 => bad.receipt_stream = "other".into(),
            3 => bad.through_utc = cutoff - 1,
            _ => bad.attested_at_utc = cutoff - 1,
        }
        assert!(s.freeze(date("2026-10-13"), cutoff, &bad).is_err());
        assert!(s.frozen(date("2026-10-13")).is_none());
    }
}

#[test]
fn only_closing_weekday_same_contract_recoveries_are_admitted() {
    for (label, symbol, reason) in [
        (
            "2026-10-12 23:30",
            "RIZ6",
            QuarantineReason::NonClosingRecovery,
        ),
        ("2026-10-12 06:50", "RIZ6", QuarantineReason::NonModelBar),
        ("2026-10-11 23:40", "RIZ6", QuarantineReason::NonModelBar),
        ("2026-11-04 23:40", "RIZ6", QuarantineReason::NonModelBar),
        ("2026-10-12 23:40", "RIU6", QuarantineReason::WrongContract),
    ] {
        let mut s = store();
        let mut i = input(1, epoch(label), DataOrigin::HistoryGap, epoch(label) + 600);
        i.bar.symbol = symbol.into();
        assert_eq!(
            s.ingest(i, epoch(label) + 600).unwrap(),
            ContextRoute::Quarantined(reason)
        );
    }
}

#[test]
fn next_open_skips_excluded_days_and_weekend_but_uses_transition_clock() {
    let mut s = store();
    let label = epoch("2026-11-03 23:40");
    assert_eq!(
        s.ingest(
            input(1, label, DataOrigin::HistoryGap, epoch("2026-11-04 08:00")),
            epoch("2026-11-04 08:00")
        )
        .unwrap(),
        ContextRoute::ContextOnly
    );
    let mut s = store();
    let label = epoch("2026-07-10 23:40");
    assert_eq!(
        s.ingest(
            input(1, label, DataOrigin::HistoryGap, epoch("2026-07-13 08:00")),
            epoch("2026-07-13 08:00")
        )
        .unwrap(),
        ContextRoute::ContextOnly
    );
    let mut s = store();
    let label = epoch("2026-07-13 23:40");
    assert_eq!(
        s.ingest(
            input(1, label, DataOrigin::HistoryGap, epoch("2026-07-14 08:00")),
            epoch("2026-07-14 08:00")
        )
        .unwrap(),
        ContextRoute::Quarantined(QuarantineReason::AtOrAfterCutoff)
    );
}

#[test]
fn duplicates_and_conflicts_never_inflate_bars_or_replace_original_close() {
    let mut s = store();
    let i = recovery(&s, "2026-10-13 03:00");
    s.ingest(i.clone(), epoch("2026-10-13 03:00")).unwrap();
    assert_eq!(
        s.ingest(i.clone(), epoch("2026-10-13 03:01")).unwrap(),
        ContextRoute::DuplicateQuote
    );
    let mut duplicate = i.clone();
    duplicate.sequence = 2;
    assert_eq!(
        s.ingest(duplicate, epoch("2026-10-13 03:02")).unwrap(),
        ContextRoute::DuplicateQuote
    );
    let mut conflict = i.clone();
    conflict.sequence = 3;
    conflict.bar.close += 1.;
    assert_eq!(
        s.ingest(conflict, epoch("2026-10-13 03:03")).unwrap(),
        ContextRoute::Quarantined(QuarantineReason::ConflictingQuote)
    );
    assert_eq!(s.state.canonical.len(), 1);
    assert_eq!(s.state.canonical[&i.bar.close_time_utc], i);
}

#[test]
fn same_sequence_conflict_and_ingress_gap_fail_without_mutation() {
    let mut s = store();
    let i = recovery(&s, "2026-10-13 03:00");
    s.ingest(i.clone(), epoch("2026-10-13 03:00")).unwrap();
    let before = s.checkpoint_bytes().unwrap();
    let mut conflict = i.clone();
    conflict.bar.close += 1.;
    assert!(s.ingest(conflict, epoch("2026-10-13 03:01")).is_err());
    let mut gap = i;
    gap.sequence = 3;
    assert!(s.ingest(gap, epoch("2026-10-13 03:01")).is_err());
    assert_eq!(s.checkpoint_bytes().unwrap(), before);
}

#[test]
fn invalid_quotes_and_receipt_clocks_fail_closed_but_keep_audit_digest() {
    for kind in 0..5 {
        let mut s = store();
        let mut i = recovery(&s, "2026-10-13 03:00");
        let reason = match kind {
            0 => {
                i.bar.close = f64::NAN;
                QuarantineReason::InvalidOhlcv
            }
            1 => {
                i.bar.v = -1.;
                QuarantineReason::InvalidOhlcv
            }
            2 => {
                i.received_at_utc = Some(i.bar.close_time_utc + 599);
                i.durable_at_utc = i.received_at_utc;
                QuarantineReason::BarNotClosed
            }
            3 => {
                i.durable_at_utc = Some(epoch("2026-10-13 02:00"));
                QuarantineReason::InvalidReceiptClock
            }
            _ => {
                i.bar.origin = DataOrigin::Replay;
                QuarantineReason::UnsupportedOrigin
            }
        };
        assert_eq!(
            s.ingest(i, epoch("2026-10-13 03:00")).unwrap(),
            ContextRoute::Quarantined(reason)
        );
        assert!(s.state.canonical.is_empty());
        let restored = RiContextStore::restore(
            &s.state.identity,
            &s.checkpoint_bytes().unwrap(),
            epoch("2026-10-13 04:00"),
        )
        .unwrap();
        assert_eq!(restored.last_sequence(), 1);
    }
}

#[test]
fn restart_before_and_after_freeze_preserves_context_and_original_receipt() {
    let mut uninterrupted = store();
    ingest_day(&mut uninterrupted, "2026-10-09", true);
    ingest_day(&mut uninterrupted, "2026-10-12", false);
    let i = recovery(&uninterrupted, "2026-10-13 03:00");
    uninterrupted
        .ingest(i.clone(), epoch("2026-10-13 03:00"))
        .unwrap();
    let mut resumed = RiContextStore::restore(
        &uninterrupted.state.identity,
        &uninterrupted.checkpoint_bytes().unwrap(),
        epoch("2026-10-13 06:00"),
    )
    .unwrap();
    let expected = freeze(&mut uninterrupted, "2026-10-13", "2026-10-13 07:10");
    let actual = freeze(&mut resumed, "2026-10-13", "2026-10-13 07:10");
    assert_eq!(actual, expected);
    assert_eq!(actual.sha256().unwrap(), expected.sha256().unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("committed.json");
    resumed.save_new_checkpoint(&path).unwrap();
    let mut restored = RiContextStore::restore(
        &resumed.state.identity,
        &fs::read(path).unwrap(),
        epoch("2026-10-13 08:00"),
    )
    .unwrap();
    assert_eq!(restored.frozen(date("2026-10-13")), Some(&expected));
    assert_eq!(
        restored.frozen_anchor_inputs(date("2026-10-13")).unwrap(),
        resumed.frozen_anchor_inputs(date("2026-10-13")).unwrap()
    );
    assert_eq!(
        restored.ingest(i, epoch("2026-10-13 08:00")).unwrap(),
        ContextRoute::DuplicateQuote
    );
}

#[test]
fn wrong_identity_tampered_truncated_or_missing_provenance_checkpoint_is_rejected() {
    let mut s = store();
    ingest_day(&mut s, "2026-10-12", true);
    freeze(&mut s, "2026-10-13", "2026-10-13 07:00");
    let bytes = s.checkpoint_bytes().unwrap();
    for kind in 0..3 {
        let mut other = s.state.identity.clone();
        match kind {
            0 => other.symbol = "RIU6".into(),
            1 => other.policy = ContextPolicy::RuntimeAcceptedOnly,
            _ => other.numerical_config_sha256 = "b".repeat(64),
        }
        assert!(RiContextStore::restore(&other, &bytes, epoch("2026-10-13 08:00")).is_err());
    }
    assert!(RiContextStore::restore(
        &s.state.identity,
        &bytes[..bytes.len() - 1],
        epoch("2026-10-13 08:00")
    )
    .is_err());
    let mut envelope: CheckpointEnvelope = serde_json::from_slice(&bytes).unwrap();
    envelope.payload.canonical.pop_first();
    envelope.payload_sha256 = hash(&envelope.payload).unwrap();
    assert!(RiContextStore::restore(
        &s.state.identity,
        &serde_json::to_vec(&envelope).unwrap(),
        epoch("2026-10-13 08:00")
    )
    .is_err());
}

#[test]
fn failed_checkpoint_publication_does_not_expose_new_freeze_or_overwrite() {
    let mut s = store();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("existing");
    fs::write(&path, "keep").unwrap();
    let now = epoch("2026-10-13 07:10");
    let b = barrier(&s, epoch("2026-10-13 07:00"), now);
    assert!(s
        .freeze_and_save_new(date("2026-10-13"), now, &b, &path)
        .is_err());
    assert!(s.frozen(date("2026-10-13")).is_none());
    assert_eq!(fs::read_to_string(path).unwrap(), "keep");
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[test]
fn completeness_and_two_prior_sessions_remain_separate_from_receipt_acceptance() {
    let mut s = store();
    ingest_day(&mut s, "2026-10-09", true);
    for ts in [epoch("2026-10-12 07:00"), epoch("2026-10-12 23:40")] {
        let i = input(s.last_sequence() + 1, ts, DataOrigin::Live, ts + 600);
        s.ingest(i, ts + 600).unwrap();
    }
    let f = freeze(&mut s, "2026-10-13", "2026-10-13 07:10");
    assert!(!f.ready());
    assert!(s.frozen_anchor_inputs(date("2026-10-13")).is_err());
    assert_eq!(f.previous.unwrap().session_date, date("2026-10-09"));
    assert_eq!(f.skipped_incomplete[0].date, date("2026-10-12"));
    let mut s = store();
    ingest_day(&mut s, "2026-07-10", true);
    ingest_day(&mut s, "2026-07-13", true);
    let f = freeze(&mut s, "2026-07-14", "2026-07-14 07:10");
    assert!(f.ready());
    assert_eq!(f.previous.unwrap().required_bars, 80);
}

#[test]
fn processing_clock_cannot_rewind_after_duplicate_or_restart() {
    let mut s = store();
    let i = recovery(&s, "2026-10-13 03:00");
    s.ingest(i.clone(), epoch("2026-10-13 03:00")).unwrap();
    s.ingest(i.clone(), epoch("2026-10-13 04:00")).unwrap();
    assert!(s.ingest(i.clone(), epoch("2026-10-13 03:30")).is_err());
    let mut restored = RiContextStore::restore(
        &s.state.identity,
        &s.checkpoint_bytes().unwrap(),
        epoch("2026-10-13 08:00"),
    )
    .unwrap();
    assert!(restored.ingest(i, epoch("2026-10-13 07:00")).is_err());
}

#[test]
fn unsupported_feed_clock_or_missing_identity_fails_before_ingestion() {
    for kind in 0..4 {
        let mut cfg = identity(ContextPolicy::RiConfirmedClosingContextV1);
        match kind {
            0 => cfg.model_feed_policy = "legacy_shadow_breaks_v1".into(),
            1 => cfg.model_open_hour = 6,
            2 => cfg.numerical_config_sha256.clear(),
            _ => cfg.receipt_stream.clear(),
        }
        assert!(RiContextStore::new(cfg).is_err());
    }
}

#[test]
fn frozen_payload_mutation_fails_reconstruction_even_with_recomputed_checksum() {
    let mut s = store();
    ingest_day(&mut s, "2026-10-09", true);
    ingest_day(&mut s, "2026-10-12", true);
    freeze(&mut s, "2026-10-13", "2026-10-13 07:00");
    let bytes = s.checkpoint_bytes().unwrap();
    let mut saved: CheckpointEnvelope = serde_json::from_slice(&bytes).unwrap();
    saved
        .payload
        .frozen
        .get_mut(&date("2026-10-13"))
        .unwrap()
        .previous
        .as_mut()
        .unwrap()
        .close += 1.;
    saved.payload_sha256 = hash(&saved.payload).unwrap();
    let error = RiContextStore::restore(
        &s.state.identity,
        &serde_json::to_vec(&saved).unwrap(),
        epoch("2026-10-13 08:00"),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("reconstruction mismatch"));
}

#[test]
fn bounded_frozen_history_stops_instead_of_silently_evicting_contexts() {
    let mut s = store();
    let mut day = date("2026-10-13");
    for _ in 0..MAX_FROZEN_SESSIONS {
        while !s.state.identity.regular_date(day) {
            day = day.succ_opt().unwrap();
        }
        let now = s.state.identity.cutoff(day);
        let b = barrier(&s, now, now);
        s.freeze(day, now, &b).unwrap();
        day = day.succ_opt().unwrap();
    }
    while !s.state.identity.regular_date(day) {
        day = day.succ_opt().unwrap();
    }
    let now = s.state.identity.cutoff(day);
    let b = barrier(&s, now, now);
    assert!(s.freeze(day, now, &b).is_err());
    assert_eq!(s.state.frozen.len(), MAX_FROZEN_SESSIONS);
    assert!(s.frozen(date("2026-10-13")).is_some());
}

#[test]
fn future_weekend_and_backdated_freezes_are_rejected() {
    let mut s = store();
    let now = epoch("2026-10-13 06:59");
    let b = barrier(&s, epoch("2026-10-13 07:00"), now);
    assert!(s.freeze(date("2026-10-13"), now, &b).is_err());
    freeze(&mut s, "2026-10-13", "2026-10-13 07:00");
    let now = epoch("2026-10-17 07:00");
    let b = barrier(&s, now, now);
    assert!(s.freeze(date("2026-10-17"), now, &b).is_err());
    assert!(s.freeze(date("2026-10-12"), now, &b).is_err());
}
