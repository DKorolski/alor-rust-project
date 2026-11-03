use super::*;

fn context_buffer(label: &str, received: &str) -> TerminalFinalizer {
    let mut f = TerminalFinalizer::new_with_context(policy(true), vec![]).unwrap();
    healthy(&mut f, received);
    f.observe_received(bar(label), ts(received), clock(received))
        .unwrap();
    f
}

#[test]
fn sparse_context_interval_releases_only_observed_quote_and_preserves_holes() {
    let mut f = context_buffer("2026-11-07T09:50:00+03:00", "2026-11-07T09:51:00+03:00");
    for s in ["2026-11-07T10:00:00+03:00", "2026-11-07T10:00:09+03:00"] {
        healthy(&mut f, s);
        assert!(f.tick(clock(s), true).unwrap().is_empty());
    }
    let cutoff = "2026-11-07T10:00:10+03:00";
    healthy(&mut f, cutoff);
    assert!(f.tick(clock(cutoff), false).unwrap().is_empty());
    let release = f.tick(clock(cutoff), true).unwrap().pop().unwrap();
    assert_eq!(release.proof.reason, ReleaseReason::ContextIntervalWs);
    assert_eq!(release.proof.next_label_utc, None);
    assert_eq!(
        release.proof.first_received_at_utc,
        ts("2026-11-07T09:51:00+03:00")
    );
    assert_eq!(release.bar.v, 589.);
    assert!(f.tick(clock(cutoff), true).unwrap().is_empty());
    assert!(
        f.observe(
            bar("2026-11-07T10:20:00+03:00"),
            clock("2026-11-07T10:20:00+03:00")
        )
        .is_err()
    );
    f.commit(&release, clock(cutoff)).unwrap();
    assert!(f.commit(&release, clock(cutoff)).is_err());
    healthy(&mut f, "2026-11-07T10:20:00+03:00");
    assert!(matches!(
        f.observe(
            bar("2026-11-07T10:20:00+03:00"),
            clock("2026-11-07T10:20:00+03:00")
        )
        .unwrap(),
        Observation::Buffered
    ));
    assert_eq!(
        f.last_committed_label("IMOEXF"),
        Some(release.bar.close_time_utc)
    );
    assert_eq!(f.buffers.len(), 1);
    assert_eq!(f.committed["IMOEXF"].len(), 1);
}

#[test]
fn context_timer_keeps_missing_terminal_and_feed_deadlines_strict() {
    for (received, cutoff, drained, stale) in [
        (
            "2026-11-07T09:51:00+03:00",
            "2026-11-07T10:01:31+03:00",
            true,
            false,
        ),
        (
            "2026-11-07T09:51:00+03:00",
            "2026-11-07T10:01:31+03:00",
            false,
            false,
        ),
        (
            "2026-11-07T09:51:00+03:00",
            "2026-11-07T10:00:10+03:00",
            true,
            true,
        ),
        (
            "2026-11-07T18:41:00+03:00",
            "2026-11-07T19:01:31+03:00",
            true,
            false,
        ),
    ] {
        let label = if received.contains("18:41") {
            "2026-11-07T18:40:00+03:00"
        } else {
            "2026-11-07T09:50:00+03:00"
        };
        let mut f = context_buffer(label, received);
        if !stale {
            healthy(&mut f, cutoff);
        }
        assert!(f.tick(clock(cutoff), drained).is_err());
        assert!(f.pending_releases().is_empty());
        assert_eq!(f.last_committed_label("IMOEXF"), None);
    }
    let mut f = context_buffer("2026-11-07T18:50:00+03:00", "2026-11-07T18:59:59+03:00");
    healthy(&mut f, "2026-11-07T19:00:10+03:00");
    let r = f
        .tick(clock("2026-11-07T19:00:10+03:00"), true)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(r.proof.reason, ReleaseReason::SessionBoundaryWs);
}

#[test]
fn context_policy_does_not_change_weekday_next_timestamp_path() {
    for opt_in in [false, true] {
        let mut f = if opt_in {
            TerminalFinalizer::new_with_context(policy(true), vec![]).unwrap()
        } else {
            TerminalFinalizer::new(policy(true)).unwrap()
        };
        healthy(&mut f, "2026-11-06T09:51:00+03:00");
        f.observe(
            bar("2026-11-06T09:50:00+03:00"),
            clock("2026-11-06T09:51:00+03:00"),
        )
        .unwrap();
        healthy(&mut f, "2026-11-06T10:00:10+03:00");
        assert!(
            f.tick(clock("2026-11-06T10:00:10+03:00"), true)
                .unwrap()
                .is_empty()
        );
        let Observation::Ready(r) = f
            .observe(
                bar("2026-11-06T10:00:00+03:00"),
                clock("2026-11-06T10:00:10+03:00"),
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(r.proof.reason, ReleaseReason::NextTimestamp);
        assert_eq!(r.bar.c, 103.);
    }
}

#[test]
fn context_policy_scope_restore_and_revisions_are_explicit() {
    let mut f = context_buffer("2026-11-07T09:50:00+03:00", "2026-11-07T09:51:00+03:00");
    let cutoff = "2026-11-07T10:00:10+03:00";
    healthy(&mut f, cutoff);
    let r = f.tick(clock(cutoff), true).unwrap().pop().unwrap();
    let mut restored = TerminalFinalizer::new_with_context(policy(true), vec![]).unwrap();
    restored
        .restore_committed(std::slice::from_ref(&r), clock(cutoff))
        .unwrap();
    assert!(restored.buffers.is_empty());
    assert!(
        TerminalFinalizer::new(policy(true))
            .unwrap()
            .restore_committed(std::slice::from_ref(&r), clock(cutoff))
            .is_err()
    );
    let mut forged = r.clone();
    forged.proof.policy_sha256 = TerminalFinalizer::new(policy(true)).unwrap().policy_sha256;
    forged.id = hash(&(&forged.bar, &forged.proof)).unwrap();
    assert!(
        TerminalFinalizer::new(policy(true))
            .unwrap()
            .restore_committed(&[forged], clock(cutoff))
            .is_err()
    );
    f.commit(&r, clock(cutoff)).unwrap();
    let mut changed = r.bar.clone();
    changed.c += 1.;
    assert!(matches!(
        f.observe(changed, clock(cutoff)).unwrap(),
        Observation::Revision { .. }
    ));
    assert_eq!(f.last_committed_label("IMOEXF"), Some(r.bar.close_time_utc));
    let day = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
    let mut excluded = TerminalFinalizer::new_with_context(policy(true), vec![day]).unwrap();
    healthy(&mut excluded, "2026-11-04T09:51:00+03:00");
    assert!(
        excluded
            .observe(
                bar("2026-11-04T09:40:00+03:00"),
                clock("2026-11-04T09:51:00+03:00")
            )
            .is_err()
    );
    excluded
        .observe(
            bar("2026-11-04T09:50:00+03:00"),
            clock("2026-11-04T09:51:00+03:00"),
        )
        .unwrap();
    healthy(&mut excluded, "2026-11-04T10:00:10+03:00");
    assert_eq!(
        excluded
            .tick(clock("2026-11-04T10:00:10+03:00"), true)
            .unwrap()[0]
            .proof
            .reason,
        ReleaseReason::ContextIntervalWs
    );
}

fn ts(s: &str) -> i64 {
    DateTime::parse_from_rfc3339(s).unwrap().timestamp()
}
fn clock(s: &str) -> Clock {
    let utc = ts(s);
    Clock {
        utc,
        monotonic_ms: ((utc - ts("2026-11-01T00:00:00+03:00")) * 1000) as u64,
    }
}
fn policy(enabled: bool) -> TerminalPolicy {
    TerminalPolicy {
        enabled,
        first_live_session: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        grace_seconds: 10,
        max_lateness_seconds: 90,
        max_heartbeat_age_seconds: 45,
        max_clock_step_seconds: 5,
        symbols: BTreeMap::from([(
            "IMOEXF".into(),
            SessionCalendar {
                weekday_end: NaiveTime::from_hms_opt(23, 50, 0).unwrap(),
                weekend_end: Some(NaiveTime::from_hms_opt(19, 0, 0).unwrap()),
                overrides: BTreeMap::new(),
            },
        )]),
    }
}
fn bar(s: &str) -> BarEvent {
    BarEvent {
        symbol: "IMOEXF".into(),
        close_time_utc: ts(s),
        o: 100.,
        h: 106.,
        l: 98.,
        c: 103.,
        v: 589.,
        origin: DataOrigin::Live,
    }
}
fn healthy(f: &mut TerminalFinalizer, s: &str) {
    let now = clock(s);
    f.update_feed(
        FeedEvidence {
            generation: 1,
            connected: true,
            subscribed: true,
            gap_resolved: true,
            last_rx_utc: now.utc,
        },
        now,
    )
    .unwrap();
}
fn buffered_terminal(enabled: bool) -> TerminalFinalizer {
    let mut f = TerminalFinalizer::new(policy(enabled)).unwrap();
    healthy(&mut f, "2026-11-02T23:49:59+03:00");
    f.observe(
        bar("2026-11-02T23:40:00+03:00"),
        clock("2026-11-02T23:49:59+03:00"),
    )
    .unwrap();
    f
}
fn terminal(f: &mut TerminalFinalizer) -> Release {
    healthy(f, "2026-11-02T23:50:10+03:00");
    f.tick(clock("2026-11-02T23:50:10+03:00"), true)
        .unwrap()
        .pop()
        .unwrap()
}

#[test]
fn queued_ws_receipt_is_not_relabelled_as_processing_time() {
    let mut f = TerminalFinalizer::new(policy(true)).unwrap();
    healthy(&mut f, "2026-11-02T23:50:10+03:00");
    f.observe_received(
        bar("2026-11-02T23:40:00+03:00"),
        ts("2026-11-02T23:49:58+03:00"),
        clock("2026-11-02T23:50:10+03:00"),
    )
    .unwrap();
    let release = terminal(&mut f);
    assert_eq!(
        release.proof.first_received_at_utc,
        ts("2026-11-02T23:49:58+03:00")
    );
    assert_eq!(
        release.proof.observation_cutoff_utc,
        ts("2026-11-02T23:50:10+03:00")
    );
}

#[test]
fn opt_in_is_disabled_when_field_omitted() {
    let mut json = serde_json::to_value(policy(true)).unwrap();
    json.as_object_mut().unwrap().remove("enabled");
    assert!(
        !serde_json::from_value::<TerminalPolicy>(json)
            .unwrap()
            .enabled
    );
    let mut f = buffered_terminal(false);
    healthy(&mut f, "2026-11-02T23:50:10+03:00");
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), true)
            .unwrap()
            .is_empty()
    );
    let next = f
        .observe(
            bar("2026-11-02T23:50:00+03:00"),
            clock("2026-11-02T23:50:10+03:00"),
        )
        .unwrap();
    assert!(
        matches!(next, Observation::Ready(r) if r.proof.reason == ReleaseReason::NextTimestamp)
    );
}

#[test]
fn timer_never_releases_before_close_or_grace_or_without_drained_prefix() {
    let mut f = buffered_terminal(true);
    for s in ["2026-11-02T23:50:00+03:00", "2026-11-02T23:50:09+03:00"] {
        healthy(&mut f, s);
        assert!(f.tick(clock(s), true).unwrap().is_empty());
    }
    healthy(&mut f, "2026-11-02T23:50:10+03:00");
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), false)
            .unwrap()
            .is_empty()
    );
    let r = terminal(&mut f);
    assert_eq!(r.proof.reason, ReleaseReason::SessionBoundaryWs);
    assert_eq!(
        r.proof.first_received_at_utc,
        ts("2026-11-02T23:49:59+03:00")
    );
}

#[test]
fn update_before_release_uses_latest_real_ohlcv_without_rewriting_after_proposal() {
    let mut f = buffered_terminal(true);
    healthy(&mut f, "2026-11-02T23:50:05+03:00");
    let mut changed = bar("2026-11-02T23:40:00+03:00");
    changed.c = 104.;
    changed.v = 590.;
    f.observe(changed.clone(), clock("2026-11-02T23:50:05+03:00"))
        .unwrap();
    let r = terminal(&mut f);
    assert_eq!(r.bar.c, 104.);
    assert_eq!(r.bar.v, 590.);
    changed.c = 105.;
    assert!(matches!(
        f.observe(changed, clock("2026-11-02T23:50:10+03:00"))
            .unwrap(),
        Observation::Revision {
            kind: RevisionKind::Price,
            ..
        }
    ));
    assert_eq!(f.pending_releases()[0].id, r.id);
}

#[test]
fn pending_append_does_not_move_watermark_and_cannot_be_committed_twice() {
    let mut f = buffered_terminal(true);
    let r = terminal(&mut f);
    assert_eq!(f.last_committed_label("IMOEXF"), None);
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), true)
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.pending_releases().len(), 1);
    let mut bad = r.clone();
    bad.bar.c = 105.;
    assert!(f.commit(&bad, clock("2026-11-02T23:50:10+03:00")).is_err());
    assert_eq!(f.last_committed_label("IMOEXF"), None);
    f.commit(&r, clock("2026-11-02T23:50:10+03:00")).unwrap();
    assert_eq!(f.last_committed_label("IMOEXF"), Some(r.bar.close_time_utc));
    assert!(f.commit(&r, clock("2026-11-02T23:50:10+03:00")).is_err());
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn next_timestamp_wins_timer_race_and_next_buffer_survives_commit() {
    let mut f = buffered_terminal(true);
    healthy(&mut f, "2026-11-02T23:50:00+03:00");
    let Observation::Ready(r) = f
        .observe(
            bar("2026-11-02T23:50:00+03:00"),
            clock("2026-11-02T23:50:00+03:00"),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(r.proof.reason, ReleaseReason::NextTimestamp);
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), true)
            .unwrap()
            .is_empty()
    );
    f.commit(&r, clock("2026-11-02T23:50:10+03:00")).unwrap();
    assert_eq!(
        f.buffers["IMOEXF"].current.bar.close_time_utc,
        ts("2026-11-02T23:50:00+03:00")
    );
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn timer_wins_next_timestamp_race_without_duplicate_release() {
    let mut f = buffered_terminal(true);
    let r = terminal(&mut f);
    assert!(
        f.observe(
            bar("2026-11-02T23:50:00+03:00"),
            clock("2026-11-02T23:50:10+03:00")
        )
        .is_err()
    );
    f.commit(&r, clock("2026-11-02T23:50:10+03:00")).unwrap();
    assert!(matches!(
        f.observe(
            bar("2026-11-02T23:50:00+03:00"),
            clock("2026-11-02T23:50:10+03:00")
        )
        .unwrap(),
        Observation::Buffered
    ));
    assert!(
        f.tick(clock("2026-11-02T23:50:10+03:00"), true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stale_heartbeat_does_not_release_even_when_process_is_running() {
    let mut f = buffered_terminal(true);
    assert!(f.tick(clock("2026-11-02T23:51:00+03:00"), true).is_err());
    assert!(f.pending_releases().is_empty());
}

#[test]
fn reconnect_or_unconfirmed_subscription_clears_only_unpublished_buffers() {
    for (connected, subscribed, gap_resolved, generation) in [
        (false, true, true, 1),
        (true, false, true, 1),
        (true, true, false, 1),
        (true, true, true, 2),
    ] {
        let mut f = buffered_terminal(true);
        let now = clock("2026-11-02T23:50:10+03:00");
        assert_eq!(
            f.update_feed(
                FeedEvidence {
                    connected,
                    subscribed,
                    gap_resolved,
                    generation,
                    last_rx_utc: now.utc
                },
                now
            )
            .unwrap(),
            1
        );
        assert!(f.tick(now, true).unwrap().is_empty());
        assert!(f.tick(clock("2026-11-02T23:51:31+03:00"), true).is_err());
    }
}

#[test]
fn reconnect_cannot_discard_uncertain_publication() {
    let mut f = buffered_terminal(true);
    let r = terminal(&mut f);
    let now = clock("2026-11-02T23:50:10+03:00");
    assert!(
        f.update_feed(
            FeedEvidence {
                generation: 2,
                connected: true,
                subscribed: true,
                gap_resolved: true,
                last_rx_utc: now.utc
            },
            now
        )
        .is_err()
    );
    assert_eq!(f.pending_releases()[0].id, r.id);
    assert_eq!(f.feed.unwrap().generation, 1);
}

#[test]
fn invalid_current_data_and_clock_steps_are_transactional() {
    let f = buffered_terminal(true);
    for field in 0..5 {
        let mut f = f.clone();
        let mut b = bar("2026-11-02T23:40:00+03:00");
        match field {
            0 => b.c = f64::NAN,
            1 => b.v = -1.,
            2 => b.origin = DataOrigin::History,
            3 => b.symbol = "OTHER".into(),
            _ => b.h = 90.,
        }
        assert!(f.observe(b, clock("2026-11-02T23:49:59+03:00")).is_err());
        assert_eq!(f.buffers["IMOEXF"].current.bar.c, 103.);
    }
    let mut f = f;
    let mut now = clock("2026-11-02T23:50:10+03:00");
    now.monotonic_ms -= 11_000;
    assert!(f.tick(now, true).is_err());
    assert!(f.pending_releases().is_empty());
}

#[test]
fn weekend_short_day_and_explicit_no_session_calendars_are_distinct() {
    let mut p = policy(true);
    p.symbols.get_mut("IMOEXF").unwrap().overrides.insert(
        NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
        Some(NaiveTime::from_hms_opt(16, 0, 0).unwrap()),
    );
    p.symbols
        .get_mut("IMOEXF")
        .unwrap()
        .overrides
        .insert(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(), None);
    for (label, observed) in [
        ("2026-11-03T15:50:00+03:00", "2026-11-03T16:00:10+03:00"),
        ("2026-11-07T18:50:00+03:00", "2026-11-07T19:00:10+03:00"),
    ] {
        let mut f = TerminalFinalizer::new(p.clone()).unwrap();
        healthy(&mut f, observed);
        f.observe(bar(label), clock(observed)).unwrap();
        assert_eq!(f.tick(clock(observed), true).unwrap().len(), 1);
    }
    let mut f = TerminalFinalizer::new(p).unwrap();
    healthy(&mut f, "2026-11-04T23:59:00+03:00");
    assert!(
        f.tick(clock("2026-11-04T23:59:00+03:00"), true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn missing_terminal_is_not_a_synthetic_candle_or_a_clean_session() {
    let mut f = TerminalFinalizer::new(policy(true)).unwrap();
    healthy(&mut f, "2026-11-02T23:51:31+03:00");
    assert!(f.tick(clock("2026-11-02T23:51:31+03:00"), true).is_err());
    assert!(f.pending_releases().is_empty());
    assert_eq!(f.last_committed_label("IMOEXF"), None);
}

#[test]
fn blocked_queue_or_publication_cannot_hide_terminal_deadline_forever() {
    let mut f = buffered_terminal(true);
    let now = clock("2026-11-02T23:51:31+03:00");
    assert!(f.tick(now, false).is_err());
    assert!(f.pending_releases().is_empty());
    let mut f = buffered_terminal(true);
    let r = terminal(&mut f);
    assert!(f.tick(now, true).is_err());
    assert_eq!(f.pending_releases()[0].id, r.id);
    assert_eq!(f.last_committed_label("IMOEXF"), None);
}

#[test]
fn pre_activation_context_cannot_require_a_terminal_live_candle() {
    let mut p = policy(true);
    p.first_live_session = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
    let mut f = TerminalFinalizer::new(p).unwrap();
    assert!(
        f.tick(clock("2026-11-02T23:59:00+03:00"), true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn late_revisions_are_audit_outcomes_not_canonical_replacement_or_source_failure() {
    let mut f = buffered_terminal(true);
    let r = terminal(&mut f);
    let now = clock("2026-11-02T23:50:10+03:00");
    f.commit(&r, now).unwrap();
    assert!(matches!(
        f.observe(r.bar.clone(), now).unwrap(),
        Observation::Duplicate
    ));
    let mut revised = r.bar.clone();
    revised.v += 1.;
    assert!(matches!(
        f.observe(revised, now).unwrap(),
        Observation::Revision {
            kind: RevisionKind::VolumeOnly,
            ..
        }
    ));
    let mut revised = r.bar.clone();
    revised.c += 0.5;
    assert!(matches!(
        f.observe(revised, now).unwrap(),
        Observation::Revision {
            kind: RevisionKind::Price,
            ..
        }
    ));
    assert_eq!(f.committed["IMOEXF"][0].bar.c, 103.);
    assert!(f.pending_releases().is_empty());
}

#[test]
fn restore_never_replays_a_committed_terminal_or_restores_old_buffers() {
    let mut old = buffered_terminal(true);
    let r = terminal(&mut old);
    old.commit(&r, clock("2026-11-02T23:50:10+03:00")).unwrap();
    let mut f = TerminalFinalizer::new(policy(true)).unwrap();
    f.restore_committed(std::slice::from_ref(&r), clock("2026-11-03T06:00:00+03:00"))
        .unwrap();
    assert!(f.buffers.is_empty() && f.feed.is_none());
    healthy(&mut f, "2026-11-03T06:00:00+03:00");
    assert!(matches!(
        f.observe(r.bar.clone(), clock("2026-11-03T06:00:00+03:00"))
            .unwrap(),
        Observation::Duplicate
    ));
    assert!(
        f.restore_committed(&[r], clock("2026-11-03T06:00:00+03:00"))
            .is_err()
    );
}

#[test]
fn tampered_or_rehashed_wrong_proofs_and_duplicate_restore_are_rejected() {
    let mut old = buffered_terminal(true);
    let r = terminal(&mut old);
    for kind in 0..5 {
        let mut row = r.clone();
        match kind {
            0 => row.bar.c += 1.,
            1 => row.proof.closed_at_utc += 1,
            2 => row.proof.policy_sha256 = "wrong".into(),
            3 => row.proof.first_received_at_utc = row.proof.observation_cutoff_utc + 1,
            _ => row.proof.next_label_utc = Some(row.bar.close_time_utc + 600),
        }
        if kind != 0 {
            row.id = hash(&(&row.bar, &row.proof)).unwrap();
        }
        let mut f = TerminalFinalizer::new(policy(true)).unwrap();
        assert!(
            f.restore_committed(&[row], clock("2026-11-03T06:00:00+03:00"))
                .is_err()
        );
        assert!(f.committed.is_empty());
    }
    let mut f = TerminalFinalizer::new(policy(true)).unwrap();
    assert!(
        f.restore_committed(&[r.clone(), r], clock("2026-11-03T06:00:00+03:00"))
            .is_err()
    );
}

#[test]
fn invalid_policy_is_rejected_before_any_input() {
    for kind in 0..5 {
        let mut p = policy(true);
        match kind {
            0 => p.grace_seconds = 0,
            1 => p.max_lateness_seconds = 1000,
            2 => p.max_heartbeat_age_seconds = 0,
            3 => p.symbols.clear(),
            _ => {
                p.symbols.get_mut("IMOEXF").unwrap().weekday_end =
                    NaiveTime::from_hms_opt(23, 49, 0).unwrap()
            }
        }
        assert!(TerminalFinalizer::new(p).is_err());
    }
}

#[test]
fn ordinary_bar_sequence_matches_existing_buffer_semantics() {
    let mut f = TerminalFinalizer::new(policy(true)).unwrap();
    let mut input = bar("2026-11-02T09:00:00+03:00");
    healthy(&mut f, "2026-11-02T09:00:01+03:00");
    f.observe(input.clone(), clock("2026-11-02T09:00:01+03:00"))
        .unwrap();
    input.c = 104.;
    input.v = 590.;
    healthy(&mut f, "2026-11-02T09:09:59+03:00");
    f.observe(input.clone(), clock("2026-11-02T09:09:59+03:00"))
        .unwrap();
    healthy(&mut f, "2026-11-02T09:10:01+03:00");
    let Observation::Ready(r) = f
        .observe(
            bar("2026-11-02T09:10:00+03:00"),
            clock("2026-11-02T09:10:01+03:00"),
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(same_bar(&r.bar, &input));
    f.commit(&r, clock("2026-11-02T09:10:01+03:00")).unwrap();
    assert_eq!(
        f.buffers["IMOEXF"].current.bar.close_time_utc,
        ts("2026-11-02T09:10:00+03:00")
    );
}
