use super::*;
use crate::shadow_session_context::{Barrier, Policy, Receipt};
use chrono::Datelike;
use std::collections::BTreeSet;

#[path = "tests/export.rs"]
mod export;

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}
fn monday() -> NaiveDate {
    date("2026-10-26")
}
fn configs() -> Vec<Config> {
    let mut configs = vec![Config::Imoexf {
        sleeve: Sleeve::Combo,
        costs: imoexf::Costs {
            point_value_rub: 10.0,
            tick_value_rub: 5.0,
            rate_per_component_per_side: 0.00006575119054451348,
            components: 2,
        },
    }];
    for (hour, horizon, wait) in [(7, 3, 4), (7, 4, 4), (7, 3, 2), (9, 3, 2)] {
        configs.push(Config::Usdrubf {
            settings: usdrubf::Config {
                model_id: format!("fixture_pd{hour}_h{horizon}_w{wait}"),
                family: usdrubf::Family::PreviousDay,
                context_hour: hour,
                clock_hour: hour,
                mr_horizon: horizon,
                bo_wait: wait,
                mr_start_delay: 0,
            },
        });
    }
    configs
}
fn identity(cfg: &Config, policy: Policy) -> Identity {
    let profile = cfg.profile().unwrap();
    Identity {
        profile,
        symbol: if profile == Profile::ImoexfReaction07 {
            "IMOEXF"
        } else {
            "USDRUBF"
        }
        .into(),
        model_config_sha256: cfg.fingerprint().unwrap(),
        receipt_stream: "synthetic-reaction-only".into(),
        policy,
        excluded_dates: BTreeSet::new(),
    }
}
fn bar(identity: &Identity, day: NaiveDate, slot: usize, price: f64) -> BarEvent {
    BarEvent {
        symbol: identity.symbol.clone(),
        close_time_utc: identity.cutoff(day) + slot as i64 * 600,
        o: price,
        h: price,
        l: price,
        close: price,
        v: 1.0,
        origin: DataOrigin::Replay,
    }
}
fn store(cfg: &Config, policy: Policy) -> Store {
    let id = identity(cfg, policy);
    let mut s = Store::new(id.clone()).unwrap();
    for d in ["2026-10-22", "2026-10-23", "2026-10-24", "2026-10-25"] {
        let day = date(d);
        let weekend = day.weekday().num_days_from_monday() >= 5;
        let start = if weekend {
            (day.and_hms_opt(9, 50, 0).unwrap() - Duration::hours(3))
                .and_utc()
                .timestamp()
        } else {
            id.cutoff(day)
        };
        let end = (day
            .and_hms_opt(
                if weekend { 18 } else { 23 },
                if weekend { 50 } else { 40 },
                0,
            )
            .unwrap()
            - Duration::hours(3))
        .and_utc()
        .timestamp();
        for ts in (start..=end).step_by(600) {
            let price = if weekend { 103.0 } else { 100.0 };
            let mut b = bar(&id, day, 0, price);
            b.close_time_utc = ts;
            b.h += 2.0;
            b.l -= 2.0;
            b.origin = DataOrigin::History;
            let sequence = s.last_sequence() + 1;
            let receipt = Receipt {
                sequence,
                source_id: format!("fixture-{sequence}"),
                bar: b,
                received_at_utc: Some(ts + 600),
                durable_at_utc: Some(ts + 600),
            };
            assert!(!s
                .ingest(receipt, ts + 600)
                .unwrap()
                .order_emission_allowed());
        }
    }
    let now = id.cutoff(monday());
    let barrier = Barrier {
        receipt_stream: id.receipt_stream,
        through_utc: now,
        last_sequence: s.last_sequence(),
        ingress_sha256: s.ingress_sha256().unwrap(),
        attested_at_utc: now,
    };
    let temp = tempfile::tempdir().unwrap();
    s.freeze_and_save_new(monday(), now, &barrier, &temp.path().join("context.json"))
        .unwrap();
    s
}
fn step(session: &mut NumericalSession, slot: usize, price: f64) {
    let b = bar(&session.identity, monday(), slot, price);
    let now = b.close_time_utc + 600;
    session.observe(b, now).unwrap();
}
fn model(session: &NumericalSession) -> Value {
    session.state().unwrap()["model"]["state"].clone()
}
fn restart(s: NumericalSession) -> NumericalSession {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("numerical-prefix.json");
    // Test artifact persistence, not a production atomic write protocol.
    std::fs::write(&path, s.checkpoint_bytes().unwrap()).unwrap();
    let now = s
        .inputs
        .last()
        .map_or(s.frozen.frozen_at_utc, |i| i.observed_at_utc);
    let restored =
        NumericalSession::restore(&s.identity, &s.config, &std::fs::read(path).unwrap(), now)
            .unwrap();
    assert_eq!(s.state().unwrap(), restored.state().unwrap());
    restored
}

#[test]
fn frozen_weekend_changes_decisions_but_never_trades_the_weekend() {
    for cfg in configs() {
        let mut states = vec![];
        for policy in [Policy::WeekdayOnly, Policy::WeekendContextNoTrades] {
            let store = store(&cfg, policy);
            let mut s = NumericalSession::new(&store, monday(), cfg.clone()).unwrap();
            let before = s.state().unwrap();
            for d in [date("2026-10-24"), date("2026-10-25")] {
                let b = bar(&s.identity, d, 0, 100.1);
                assert!(s.observe(b.clone(), b.close_time_utc + 600).is_err());
                assert_eq!(before, s.state().unwrap());
            }
            step(&mut s, 0, 100.1);
            states.push(model(&s));
            assert_eq!(s.state().unwrap()["order_emission_allowed"], false);
            assert_eq!(
                s.frozen.previous.as_ref().unwrap().session_date,
                date(if policy == Policy::WeekdayOnly {
                    "2026-10-23"
                } else {
                    "2026-10-25"
                })
            );
        }
        assert_eq!(states[0]["branch"], "mr_selected");
        assert!(states[0]["pending"].is_object());
        assert!(states[1]["pending"].is_null());
        assert_ne!(states[1]["branch"], "mr_selected");
    }
}

#[test]
fn mr_pending_entry_position_pending_exit_and_fill_survive_disk_restart() {
    for cfg in configs() {
        for policy in [Policy::WeekdayOnly, Policy::WeekendContextNoTrades] {
            let context = store(&cfg, policy);
            let anchor = context
                .frozen(monday())
                .unwrap()
                .previous
                .as_ref()
                .unwrap()
                .close;
            let mut continuous = NumericalSession::new(&context, monday(), cfg.clone()).unwrap();
            let mut recovered =
                restart(NumericalSession::new(&context, monday(), cfg.clone()).unwrap());
            for (slot, delta) in [0.10, 0.12, -0.60, -0.40].into_iter().enumerate() {
                step(&mut continuous, slot, anchor + delta);
                step(&mut recovered, slot, anchor + delta);
                recovered = restart(recovered);
                assert_eq!(continuous.state().unwrap(), recovered.state().unwrap());
                let m = model(&recovered);
                match slot {
                    0 => {
                        assert!(m["pending"].is_object());
                        assert!(m["position"].is_null());
                    }
                    1 => {
                        assert!(m["pending"].is_null());
                        assert!(m["position"].is_object());
                    }
                    2 => {
                        assert!(m["pending"].is_object());
                        assert!(m["position"].is_object());
                    }
                    _ => {
                        assert!(m["position"].is_null());
                        assert_eq!(m["trades"].as_array().unwrap().len(), 1);
                    }
                }
            }
            let m = model(&recovered);
            let trade = &m["trades"][0];
            assert!((trade["gross_points"].as_f64().unwrap() - 0.52).abs() < 1e-10);
            assert_eq!(trade["entry_price"], anchor + 0.12);
            assert_eq!(trade["exit_price"], anchor - 0.40);
            assert_eq!(m["events"].as_array().unwrap().len(), 2);
            assert_eq!(m["previous"]["close"], anchor);
        }
    }
}

#[test]
fn bo_observation_wait_quota_and_eod_pending_survive_restart_without_carry() {
    for cfg in configs() {
        for policy in [Policy::WeekdayOnly, Policy::WeekendContextNoTrades] {
            let context = store(&cfg, policy);
            let anchor = context
                .frozen(monday())
                .unwrap()
                .previous
                .as_ref()
                .unwrap()
                .close;
            let mut s = NumericalSession::new(&context, monday(), cfg.clone()).unwrap();
            let wait = match &cfg {
                Config::Imoexf { .. } => 3,
                Config::Usdrubf { settings } => settings.bo_wait,
            } as usize
                * 6;
            let end = if s.identity.opening_hour(monday()) == 7 {
                100
            } else {
                88
            };
            for slot in 0..end {
                step(
                    &mut s,
                    slot,
                    if slot < wait { anchor } else { anchor + 4.0 },
                );
                if slot == wait {
                    assert_eq!(model(&s)["branch"], "bo_selected");
                    assert!(model(&s)["pending"].is_object());
                    s = restart(s);
                }
                if slot == wait + 1 {
                    assert_eq!(model(&s)["used_long"], true);
                    assert!(model(&s)["position"].is_object());
                    s = restart(s);
                }
            }
            // 23:30 schedules the exit, but it is NOT already a fill or flat.
            assert!(model(&s)["position"].is_object());
            assert!(model(&s)["pending"].is_object());
            assert!(s.complete().is_err());
            s = restart(s);
            let before = s.state().unwrap();
            let next_day = bar(&s.identity, monday().succ_opt().unwrap(), 0, anchor);
            assert!(s
                .observe(next_day.clone(), next_day.close_time_utc + 600)
                .is_err());
            assert_eq!(before, s.state().unwrap());
            step(&mut s, end, anchor + 3.5);
            s.complete().unwrap();
            assert_eq!(model(&s)["trades"].as_array().unwrap().len(), 1);
            assert_eq!(model(&s)["trades"][0]["gross_points"], -0.5);
            assert!(model(&s)["position"].is_null());
            let count = match &s.engine {
                Engine::Imoexf(e) => e.observation.as_ref().unwrap().count,
                Engine::Usdrubf(e) => e.metrics.as_ref().unwrap().count,
            };
            assert_eq!(count, end + 1);
        }
    }
}

#[test]
fn invalid_execution_inputs_are_transactional_and_cannot_consume_pending() {
    for cfg in configs() {
        let context = store(&cfg, Policy::WeekdayOnly);
        let mut s = NumericalSession::new(&context, monday(), cfg).unwrap();
        step(&mut s, 0, 100.1);
        let before = s.checkpoint_bytes().unwrap();
        let valid = bar(&s.identity, monday(), 1, 100.12);
        let now = valid.close_time_utc + 600;
        let mut bad = vec![];
        let mut b = valid.clone();
        b.close_time_utc += 600;
        bad.push(b);
        let mut b = valid.clone();
        b.close_time_utc -= 600;
        bad.push(b);
        let mut b = valid.clone();
        b.symbol = "RIU6".into();
        bad.push(b);
        let mut b = valid.clone();
        b.origin = DataOrigin::Live;
        bad.push(b);
        let mut b = valid.clone();
        b.h = b.l - 1.0;
        bad.push(b);
        let mut b = valid.clone();
        b.v = -1.0;
        bad.push(b);
        let mut b = valid.clone();
        b.close = f64::NAN;
        bad.push(b);
        for b in bad {
            assert!(s.observe(b, now + 600).is_err());
            assert_eq!(before, s.checkpoint_bytes().unwrap());
        }
        assert!(s.observe(valid.clone(), now - 1).is_err());
        assert_eq!(before, s.checkpoint_bytes().unwrap());
        s.observe(valid, now).unwrap();
        assert!(model(&s)["position"].is_object());
    }
}

#[test]
fn corrupted_or_rehashed_wrong_decision_state_and_wrong_identity_fail_restore() {
    for cfg in configs() {
        let context = store(&cfg, Policy::WeekdayOnly);
        let mut s = NumericalSession::new(&context, monday(), cfg.clone()).unwrap();
        step(&mut s, 0, 100.1);
        let bytes = s.checkpoint_bytes().unwrap();
        let now = s.inputs[0].observed_at_utc;
        assert!(
            NumericalSession::restore(&s.identity, &cfg, &bytes[..bytes.len() - 1], now).is_err()
        );
        assert!(NumericalSession::restore(&s.identity, &cfg, &bytes, now - 1).is_err());
        let mut wrong = s.identity.clone();
        wrong.policy = Policy::WeekendContextNoTrades;
        assert!(NumericalSession::restore(&wrong, &cfg, &bytes, now).is_err());
        let mut saved: Envelope = serde_json::from_slice(&bytes).unwrap();
        saved.payload.witness["model"]["state"]["pending"] = Value::Null;
        saved.sha256 = hash(&saved.payload).unwrap();
        assert!(NumericalSession::restore(
            &s.identity,
            &cfg,
            &serde_json::to_vec(&saved).unwrap(),
            now
        )
        .is_err());
        let mut saved: Envelope = serde_json::from_slice(&bytes).unwrap();
        saved.payload.inputs[0].bar.close = 100.11;
        saved.sha256 = hash(&saved.payload).unwrap();
        assert!(NumericalSession::restore(
            &s.identity,
            &cfg,
            &serde_json::to_vec(&saved).unwrap(),
            now
        )
        .is_err());
    }
}

#[test]
fn missing_freeze_anchors_and_changed_model_identity_are_not_bootstrapped() {
    for cfg in configs() {
        let empty = Store::new(identity(&cfg, Policy::WeekdayOnly)).unwrap();
        assert!(NumericalSession::new(&empty, monday(), cfg.clone()).is_err());
        let context = store(&cfg, Policy::WeekdayOnly);
        assert!(
            NumericalSession::new(&context, monday().succ_opt().unwrap(), cfg.clone()).is_err()
        );
        let mut altered = cfg.clone();
        match &mut altered {
            Config::Imoexf { sleeve, .. } => *sleeve = Sleeve::Mr,
            Config::Usdrubf { settings } => {
                settings.mr_horizon = if settings.mr_horizon == 3 { 4 } else { 3 }
            }
        }
        assert!(NumericalSession::new(&context, monday(), altered).is_err());
    }
}

#[test]
fn imoexf_mr_quota_and_mr_first_no_overlap_do_not_hide_bo_observation() {
    let cfg = configs().remove(0);
    let context = store(&cfg, Policy::WeekdayOnly);
    let mut s = NumericalSession::new(&context, monday(), cfg.clone()).unwrap();
    for slot in 0..18 {
        step(&mut s, slot, 100.1);
    }
    step(&mut s, 18, 104.0); // MR stop and BO setup on 10:00: exit wins.
    assert_eq!(model(&s)["position"]["entry"]["owner"], "MR");
    assert_eq!(model(&s)["pending"]["owner"], "MR");
    assert_eq!(model(&s)["pending"]["exit_reason"], "stop");
    assert_eq!(model(&s)["observation"]["count"], 19);
    s = restart(s);
    step(&mut s, 19, 104.0);
    assert!(model(&s)["position"].is_null());
    assert_eq!(model(&s)["pending"]["owner"], "BO");
    step(&mut s, 20, 104.0);
    assert_eq!(model(&s)["position"]["entry"]["owner"], "BO");
    assert_eq!(model(&s)["mr_entries"], 1);
    assert_eq!(model(&s)["observation"]["count"], 21);

    for sleeve in [Sleeve::Mr, Sleeve::Combo] {
        let mut cfg = cfg.clone();
        if let Config::Imoexf {
            sleeve: current, ..
        } = &mut cfg
        {
            *current = sleeve;
        }
        let context = store(&cfg, Policy::WeekdayOnly);
        let mut s = NumericalSession::new(&context, monday(), cfg).unwrap();
        for (slot, price) in [100.1, 99.4, 99.6, 100.1, 99.4, 99.6, 100.1]
            .into_iter()
            .enumerate()
        {
            step(&mut s, slot, price);
            s = restart(s);
        }
        assert_eq!(model(&s)["mr_entries"], 2);
        assert_eq!(model(&s)["trades"].as_array().unwrap().len(), 2);
        assert!(model(&s)["pending"].is_null());
        assert!(model(&s)["position"].is_null());
    }
}

#[test]
fn frozen_weekday_execution_matches_existing_batch_model_on_same_complete_feed() {
    for cfg in configs() {
        let context = store(&cfg, Policy::WeekdayOnly);
        let mut s = NumericalSession::new(&context, monday(), cfg.clone()).unwrap();
        let end = if s.identity.opening_hour(monday()) == 7 {
            100
        } else {
            88
        };
        let mut raw = vec![];
        for day in [date("2026-10-22"), date("2026-10-23")] {
            for slot in 0..=end {
                let mut b = bar(&s.identity, day, slot, 100.0);
                b.h = 102.0;
                b.l = 98.0;
                raw.push(b);
            }
        }
        for slot in 0..=end {
            let price = match slot {
                0 => 100.1,
                1 => 100.12,
                2 => 99.4,
                3 => 99.6,
                4..=23 => 100.0,
                _ => 104.0,
            };
            let b = bar(&s.identity, monday(), slot, price);
            s.observe(b.clone(), b.close_time_utc + 600).unwrap();
            raw.push(b);
        }
        s.complete().unwrap();
        let (actual, expected) = match &cfg {
            Config::Imoexf { costs, .. } => {
                let bars: Vec<_> = raw
                    .iter()
                    .map(|b| imoexf::Bar {
                        ts_local: local(b.close_time_utc).unwrap(),
                        open: b.o,
                        high: b.h,
                        low: b.l,
                        close: b.close,
                        volume: b.v,
                    })
                    .collect();
                let mut old = imoexf::Engine::new("combo", costs.clone()).unwrap();
                old.replay(&bars, |_, _, _| Ok(())).unwrap();
                (model(&s), serde_json::to_value(old).unwrap())
            }
            Config::Usdrubf { settings } => {
                let bars: Vec<_> = raw
                    .iter()
                    .map(|b| usdrubf::Bar {
                        ts_local: local(b.close_time_utc).unwrap(),
                        open: b.o,
                        high: b.h,
                        low: b.l,
                        close: b.close,
                        volume: b.v,
                    })
                    .collect();
                let mut old = usdrubf::Engine::new(settings.clone());
                old.replay(&bars, |_, _, _| Ok(())).unwrap();
                (model(&s), serde_json::to_value(old).unwrap())
            }
        };
        for key in [
            "position",
            "pending",
            "branch",
            "trades",
            "events",
            "used_long",
            "used_short",
            "mr_entries",
            "mr_scheduled",
            "metrics",
            "observation",
        ] {
            assert_eq!(
                actual[key], expected[key],
                "baseline mismatch: {key}, {cfg:?}"
            );
        }
    }
}

#[test]
fn restart_clock_floor_is_not_lost_on_repeated_checkpoint() {
    let cfg = configs().remove(0);
    let context = store(&cfg, Policy::WeekdayOnly);
    let mut s = NumericalSession::new(&context, monday(), cfg.clone()).unwrap();
    step(&mut s, 0, 100.1);
    let now = s.clock_floor + 2_000;
    let mut recovered =
        NumericalSession::restore(&s.identity, &cfg, &s.checkpoint_bytes().unwrap(), now).unwrap();
    let saved = recovered.checkpoint_bytes().unwrap();
    assert!(NumericalSession::restore(&s.identity, &cfg, &saved, now - 1).is_err());
    let b = bar(&s.identity, monday(), 1, 100.12);
    assert!(recovered
        .observe(b.clone(), b.close_time_utc + 600)
        .is_err());
    assert_eq!(saved, recovered.checkpoint_bytes().unwrap());
    // Delayed numerical delivery is explicit, not a missing-bar forward fill.
    recovered.observe(b, now).unwrap();
}

#[test]
fn bo_only_has_distinct_identity_and_does_not_consume_mr_quota() {
    let combo = configs().remove(0);
    let mut cfg = combo.clone();
    if let Config::Imoexf { sleeve, .. } = &mut cfg {
        *sleeve = Sleeve::Bo;
    }
    assert_ne!(cfg.fingerprint().unwrap(), combo.fingerprint().unwrap());
    let context = store(&cfg, Policy::WeekdayOnly);
    let mut s = NumericalSession::new(&context, monday(), cfg).unwrap();
    for slot in 0..=100 {
        step(&mut s, slot, if slot < 18 { 100.1 } else { 104.0 });
        if slot == 0 {
            assert!(model(&s)["position"].is_null());
            assert!(model(&s)["pending"].is_null());
            assert_eq!(model(&s)["branch"], "bo_wait");
        }
    }
    s.complete().unwrap();
    assert_eq!(model(&s)["mr_entries"], 0);
    assert_eq!(model(&s)["trades"][0]["component"], "BO");
    assert_eq!(
        model(&s)["trades"][0]["model_id"],
        "bo053_reaction07_control"
    );
}
