use super::*;
use crate::ri_context::ReceiptBarrier;

mod history;
mod reference;

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}
fn generation() -> ShadowGeneration {
    ShadowGeneration {
        id: "paired-generation".into(),
        first_decision_session: date("2026-10-13"),
    }
}
fn epoch(s: &str) -> i64 {
    (NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap() - chrono::Duration::hours(3))
        .and_utc()
        .timestamp()
}
fn config() -> RiAuthor4142LiveConfig {
    let mut cfg = super::super::tests::canonical07_transition_config();
    cfg.symbol = "RIZ6".into();
    cfg.mode = RiAuthor4142RuntimeMode::ProspectiveShadow;
    cfg.model_feed_policy = Some(RiAuthor4142ModelFeedPolicy::MicroRegularV1);
    cfg
}
fn input(sequence: u64, label: i64, origin: DataOrigin, close: f64) -> ContextInput {
    ContextInput {
        sequence,
        bar: BarEvent {
            symbol: "RIZ6".into(),
            close_time_utc: label,
            close,
            o: close,
            h: close + 10.,
            l: close - 10.,
            v: 10.,
            origin,
        },
        received_at_utc: Some(label + 600),
        durable_at_utc: Some(label + 600),
    }
}
fn add(store: &mut RiContextStore, inputs: &mut Vec<ContextInput>, mut input: ContextInput) {
    input.sequence = store.last_sequence() + 1;
    store
        .ingest(input.clone(), input.durable_at_utc.unwrap())
        .unwrap();
    inputs.push(input);
}
fn fixture(policy: ContextPolicy) -> (RiContextStore, Vec<ContextInput>) {
    let mut store = RiContextStore::new(
        context_identity(&config(), "paired.receipts.v1".into(), policy).unwrap(),
    )
    .unwrap();
    let mut inputs = vec![];
    for day in ["2026-10-09", "2026-10-12"] {
        for label in (epoch(&format!("{day} 07:00"))..=epoch(&format!("{day} 23:30"))).step_by(600)
        {
            let mut i = input(0, label, DataOrigin::History, 90000.);
            i.bar.h = 91000.;
            i.bar.l = 89000.;
            add(&mut store, &mut inputs, i);
        }
    }
    let mut gap = input(0, epoch("2026-10-12 23:40"), DataOrigin::HistoryGap, 90500.);
    gap.received_at_utc = Some(epoch("2026-10-13 03:00"));
    gap.durable_at_utc = gap.received_at_utc;
    gap.bar.h = 91000.;
    gap.bar.l = 89000.;
    add(&mut store, &mut inputs, gap);
    (store, inputs)
}
fn freeze(store: &mut RiContextStore, day: &str, now: i64) {
    let barrier = ReceiptBarrier {
        receipt_stream: store.identity().receipt_stream.clone(),
        through_utc: store.identity().cutoff(date(day)),
        last_sequence: store.last_sequence(),
        ingress_sha256: store.ingress_sha256().unwrap(),
        attested_at_utc: now,
    };
    let dir = tempfile::tempdir().unwrap();
    store
        .freeze_and_save_new(date(day), now, &barrier, &dir.path().join("context.json"))
        .unwrap();
}
fn adapter(store: &RiContextStore, inputs: &[ContextInput]) -> RiFrozenShadowAdapter {
    let mut a =
        RiFrozenShadowAdapter::new(config(), store.identity().clone(), generation()).unwrap();
    for i in inputs {
        assert!(a.process(store, i).unwrap().records.is_empty());
    }
    a
}

#[test]
fn branch_trace_is_opt_in_and_context_only_event_clears_previous_branches() {
    assert!(RiAuthor4142LiveStrategy::new(config())
        .unwrap()
        .shadow_branches
        .is_none());
    let (mut s, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    freeze(&mut s, "2026-10-13", epoch("2026-10-13 07:00"));
    let mut a = adapter(&s, &inputs);
    add(
        &mut s,
        &mut inputs,
        input(0, epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
    );
    let out = a.process(&s, inputs.last().unwrap()).unwrap();
    assert_eq!(
        out.state_trace["branches"],
        serde_json::json!(["mr:entry", "bo:observed", "bo:pending_entry"])
    );
    add(
        &mut s,
        &mut inputs,
        input(0, epoch("2026-10-13 07:10"), DataOrigin::HistoryGap, 89900.),
    );
    let context_only = a.process(&s, inputs.last().unwrap()).unwrap();
    assert_eq!(context_only.state_trace["branches"], serde_json::json!([]));
    assert_eq!(context_only.state_trace["bo_observation_index"], 1);
    assert!(context_only.records.is_empty());
}

#[test]
fn old_untraced_witness_is_not_silently_resumed_with_new_trace_contract() {
    let (s, inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    let a = adapter(&s, &inputs);
    let mut witness: serde_json::Value =
        serde_json::from_slice(&a.recovery_witness(&s).unwrap()).unwrap();
    witness["version"] = "ri_frozen_prospective_shadow_v1".into();
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        generation(),
        &s,
        &inputs,
        &serde_json::to_vec(&witness).unwrap()
    )
    .is_err());
}

#[test]
fn mr_bo_and_diagnostic_replay_share_the_exact_frozen_closing_recovery() {
    for (policy, expected_close) in [
        (ContextPolicy::RuntimeAcceptedOnly, 90000.),
        (ContextPolicy::RiConfirmedClosingContextV1, 90500.),
    ] {
        let (mut s, mut inputs) = fixture(policy);
        freeze(&mut s, "2026-10-13", epoch("2026-10-13 07:00"));
        let mut a = adapter(&s, &inputs);
        add(
            &mut s,
            &mut inputs,
            input(0, epoch("2026-10-13 07:00"), DataOrigin::Live, 90000.),
        );
        let out = a.process(&s, inputs.last().unwrap()).unwrap();
        assert_eq!(out.disposition, "prospective_observed");
        let mr = a.strategy.mr_anchor_for_date(date("2026-10-13")).unwrap();
        let bo = a.strategy.bo_context_for_date(date("2026-10-13")).unwrap();
        assert_eq!(mr.prev_close, expected_close);
        assert_eq!(bo.prev_close, expected_close);
        assert_eq!(bo.prev2_close, 90000.);
        let view = a.strategy.shadow_context.as_ref().unwrap();
        let bars = view.diagnostic_bars(date("2026-10-13"), &a.strategy.model_bars);
        assert_eq!(
            bars.iter()
                .rfind(|b| b.ts_local.date() == date("2026-10-12"))
                .unwrap()
                .close,
            expected_close
        );
        assert!(out
            .records
            .iter()
            .all(|r| !r.model_signal_ts_local.starts_with("2026-10-12")));
        // Mutable history must not silently replace a frozen anchor in ANY path.
        a.strategy.model_bars.push(ModelBar {
            ts_local: local(epoch("2026-10-12 23:40")).unwrap(),
            open: 1.,
            high: 2.,
            low: 1.,
            close: 1.,
            volume: 0.,
        });
        assert_eq!(
            a.strategy
                .mr_anchor_for_date(date("2026-10-13"))
                .unwrap()
                .prev_close,
            expected_close
        );
        assert_eq!(
            a.strategy
                .bo_context_for_date(date("2026-10-13"))
                .unwrap()
                .prev_close,
            expected_close
        );
        assert_eq!(
            a.strategy
                .shadow_context
                .as_ref()
                .unwrap()
                .diagnostic_bars(date("2026-10-13"), &a.strategy.model_bars),
            bars
        );
    }
}

#[test]
fn missing_or_incomplete_frozen_context_never_falls_back_to_mutable_history() {
    let (mut s, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    let mut a = adapter(&s, &inputs);
    add(
        &mut s,
        &mut inputs,
        input(0, epoch("2026-10-13 07:00"), DataOrigin::Live, 89500.),
    );
    let before = a.state_trace();
    assert!(a.process(&s, inputs.last().unwrap()).is_err());
    assert_eq!(a.state_trace(), before);
    let mut empty = RiContextStore::new(s.identity().clone()).unwrap();
    freeze(&mut empty, "2026-10-13", epoch("2026-10-13 07:00"));
    let i = input(1, epoch("2026-10-13 07:00"), DataOrigin::Live, 89500.);
    empty.ingest(i.clone(), i.durable_at_utc.unwrap()).unwrap();
    let mut a = adapter(&empty, &[]);
    let out = a.process(&empty, &i).unwrap();
    assert_eq!(out.disposition, "context_not_ready");
    assert!(out.records.is_empty());
    assert!(a.strategy.live_mr.position.is_none());
    assert!(a.strategy.live_bo.context.is_none());
}

#[test]
fn restart_reconstructs_open_mr_counters_bo_observation_and_decision_dedup() {
    let (mut s, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    freeze(&mut s, "2026-10-13", epoch("2026-10-13 07:00"));
    let mut a = adapter(&s, &inputs);
    for minute in [0, 10, 20, 30, 40, 50] {
        add(
            &mut s,
            &mut inputs,
            input(
                0,
                epoch(&format!("2026-10-13 07:{minute:02}")),
                DataOrigin::Live,
                89900.,
            ),
        );
        let result = a.process(&s, inputs.last().unwrap()).unwrap();
        assert!(result
            .records
            .iter()
            .all(|r| r.adapter_decision != RiAuthor4142JournalDecision::IntentEmitted));
    }
    assert_eq!(a.strategy.live_mr.entries_today, 1);
    assert!(a.strategy.live_mr.position.is_some());
    assert_eq!(a.strategy.live_bo.bar_index, 6);
    assert!(a.strategy.live_bo.long_level.is_some());
    let bytes = a.recovery_witness(&s).unwrap();
    let mut restored = RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        generation(),
        &s,
        &inputs,
        &bytes,
    )
    .unwrap();
    assert_eq!(a.state_trace(), restored.state_trace());
    assert_eq!(a.strategy.live_mr, restored.strategy.live_mr);
    assert_eq!(a.strategy.live_bo, restored.strategy.live_bo);
    assert_eq!(
        a.strategy.emitted_decision_keys,
        restored.strategy.emitted_decision_keys
    );
    add(
        &mut s,
        &mut inputs,
        input(0, epoch("2026-10-13 08:00"), DataOrigin::Live, 90100.),
    );
    assert_eq!(
        a.process(&s, inputs.last().unwrap()).unwrap(),
        restored.process(&s, inputs.last().unwrap()).unwrap()
    );
    assert!(restored.strategy.live_mr.position.is_none());
    assert_eq!(
        restored.recovery_witness(&s).unwrap(),
        a.recovery_witness(&s).unwrap()
    );
    assert!(restored.process(&s, inputs.last().unwrap()).is_err());
}

#[test]
fn mr_total_quota_stays_two_and_bo_observer_continues_during_ownership() {
    let (mut s, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    freeze(&mut s, "2026-10-13", epoch("2026-10-13 07:00"));
    let mut a = adapter(&s, &inputs);
    for (minute, close) in [
        (0, 89900.),
        (10, 90100.),
        (20, 90200.),
        (30, 89500.),
        (40, 89900.),
        (50, 89900.),
    ] {
        add(
            &mut s,
            &mut inputs,
            input(
                0,
                epoch(&format!("2026-10-13 07:{minute:02}")),
                DataOrigin::Live,
                close,
            ),
        );
        a.process(&s, inputs.last().unwrap()).unwrap();
    }
    assert_eq!(a.strategy.live_mr.entries_today, 2);
    assert!(a.strategy.live_mr.position.is_none());
    assert_eq!(a.strategy.live_bo.bar_index, 6);
}

#[test]
fn changed_generation_inputs_config_or_witness_cannot_restore_as_flat() {
    let (s, inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    let a = adapter(&s, &inputs);
    let bytes = a.recovery_witness(&s).unwrap();
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        ShadowGeneration {
            id: "wrong".into(),
            ..generation()
        },
        &s,
        &inputs,
        &bytes
    )
    .is_err());
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        generation(),
        &s,
        &inputs[..inputs.len() - 1],
        &bytes
    )
    .is_err());
    let mut changed = inputs.clone();
    changed[0].bar.close += 1.;
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        generation(),
        &s,
        &changed,
        &bytes
    )
    .is_err());
    let mut cfg = config();
    cfg.qty = 2.;
    assert!(RiFrozenShadowAdapter::new(cfg, s.identity().clone(), generation()).is_err());
    let mut witness: RecoveryWitness = serde_json::from_slice(&bytes).unwrap();
    witness.state_sha256 = "0".repeat(64);
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        generation(),
        &s,
        &inputs,
        &serde_json::to_vec(&witness).unwrap()
    )
    .is_err());
}

#[test]
fn legacy_live_modes_and_bo_filter_inheritance_are_rejected() {
    for mode in [
        RiAuthor4142RuntimeMode::MicroLive,
        RiAuthor4142RuntimeMode::Shadow,
        RiAuthor4142RuntimeMode::DryRun,
    ] {
        let mut c = config();
        c.mode = mode;
        c.allow_order_emission = mode.can_emit_orders();
        assert!(context_identity(&c, "input".into(), ContextPolicy::RuntimeAcceptedOnly).is_err());
    }
    let mut c = config();
    c.author42_max_entries_per_day = Some(2);
    assert!(context_identity(&c, "input".into(), ContextPolicy::RuntimeAcceptedOnly).is_err());
    let mut c = config();
    c.model_feed_policy = None;
    assert!(context_identity(&c, "input".into(), ContextPolicy::RuntimeAcceptedOnly).is_err());
}

#[test]
fn unresolved_position_at_next_session_is_an_error_not_overnight_reset() {
    let (mut s, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    freeze(&mut s, "2026-10-13", epoch("2026-10-13 07:00"));
    let mut a = adapter(&s, &inputs);
    add(
        &mut s,
        &mut inputs,
        input(0, epoch("2026-10-13 07:00"), DataOrigin::Live, 89900.),
    );
    a.process(&s, inputs.last().unwrap()).unwrap();
    assert!(a.strategy.live_mr.position.is_some());
    freeze(&mut s, "2026-10-14", epoch("2026-10-14 07:00"));
    add(
        &mut s,
        &mut inputs,
        input(0, epoch("2026-10-14 07:00"), DataOrigin::Live, 90100.),
    );
    let before = a.state_trace();
    assert!(a
        .process(&s, inputs.last().unwrap())
        .unwrap_err()
        .to_string()
        .contains("session boundary"));
    assert_eq!(a.state_trace(), before);
}

#[test]
fn restart_preserves_pending_bo_then_entry_and_eod_without_outbound_intents() {
    let (mut s, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    freeze(&mut s, "2026-10-13", epoch("2026-10-13 07:00"));
    let mut a = adapter(&s, &inputs);
    for ts in (epoch("2026-10-13 07:00")..=epoch("2026-10-13 08:50")).step_by(600) {
        let close = if ts == epoch("2026-10-13 08:50") {
            91000.
        } else {
            90000.
        };
        add(&mut s, &mut inputs, input(0, ts, DataOrigin::Live, close));
        a.process(&s, inputs.last().unwrap()).unwrap();
    }
    assert_eq!(
        a.strategy.live_bo.pending,
        Some(RiAuthor4142LiveBoPending::Entry(RiAuthor4142Side::Long))
    );
    let witness = a.recovery_witness(&s).unwrap();
    let mut b = RiFrozenShadowAdapter::restore(
        config(),
        s.identity().clone(),
        generation(),
        &s,
        &inputs,
        &witness,
    )
    .unwrap();
    assert_eq!(a.strategy.live_bo, b.strategy.live_bo);
    for (label, close) in [("2026-10-13 09:00", 91000.), ("2026-10-13 23:00", 91100.)] {
        add(
            &mut s,
            &mut inputs,
            input(0, epoch(label), DataOrigin::Live, close),
        );
        let left = a.process(&s, inputs.last().unwrap()).unwrap();
        let right = b.process(&s, inputs.last().unwrap()).unwrap();
        assert_eq!(left, right);
        assert!(left
            .records
            .iter()
            .all(|r| r.adapter_decision != RiAuthor4142JournalDecision::IntentEmitted));
    }
    assert!(a.strategy.live_bo.position.is_none());
    assert!(a.strategy.live_bo.pending.is_none());
    assert_eq!(a.strategy.live_bo.entries_today, 1);
}

#[test]
fn recovered_closing_gap_never_becomes_a_trade_and_wrong_payload_is_atomic_error() {
    let (s, inputs) = fixture(ContextPolicy::RiConfirmedClosingContextV1);
    let mut a = adapter(&s, &inputs[..inputs.len() - 1]);
    let before = a.state_trace();
    let mut wrong = inputs.last().unwrap().clone();
    wrong.bar.close += 1.;
    assert!(a.process(&s, &wrong).is_err());
    assert_eq!(a.state_trace(), before);
    let result = a.process(&s, inputs.last().unwrap()).unwrap();
    assert!(result.disposition.starts_with("context_only"));
    assert_eq!(result.origin, DataOrigin::HistoryGap);
    assert!(result.records.is_empty());
    assert_eq!(a.last_model_ts, None);
    assert!(a.strategy.live_mr.position.is_none());
    assert!(a.strategy.live_bo.position.is_none());
}

#[test]
fn explicit_first_decision_session_keeps_warmup_live_records_context_only() {
    let (original, mut inputs) = fixture(ContextPolicy::RuntimeAcceptedOnly);
    let mut store = RiContextStore::new(original.identity().clone()).unwrap();
    for input in &mut inputs {
        if input.bar.origin == DataOrigin::History {
            input.bar.origin = DataOrigin::Live;
        }
        store
            .ingest(input.clone(), input.durable_at_utc.unwrap())
            .unwrap();
    }
    let a = adapter(&store, &inputs);
    assert_eq!(a.last_model_ts, None);
    assert_eq!(a.strategy.live_mr.entries_today, 0);
    assert_eq!(a.strategy.live_bo.bar_index, 0);
    let bytes = a.recovery_witness(&store).unwrap();
    let later = ShadowGeneration {
        first_decision_session: date("2026-10-14"),
        ..generation()
    };
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        store.identity().clone(),
        later,
        &store,
        &inputs,
        &bytes
    )
    .is_err());
    assert!(RiFrozenShadowAdapter::restore(
        config(),
        store.identity().clone(),
        generation(),
        &store,
        &inputs,
        &bytes
    )
    .is_ok());
}
