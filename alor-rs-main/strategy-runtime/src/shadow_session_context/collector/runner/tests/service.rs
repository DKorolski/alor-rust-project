use super::*;
use crate::shadow_session_context::collector::runner::service::{
    LaunchSpec, ServiceGuard, ServicePolicy,
};

fn policy() -> ServicePolicy {
    ServicePolicy {
        startup_timeout_seconds: 600,
        poll_timeout_seconds: 60,
        poll_interval_seconds: 5,
        shutdown_timeout_seconds: 15,
        max_queue_age_seconds: 30,
        max_model_lateness_seconds: 120,
        max_sessions: 20,
    }
}

#[test]
fn reaction_service_policy_and_scope_are_explicit() {
    let launch = LaunchSpec {
        runner: spec("IMOEXF"),
        policy: policy(),
    };
    launch.validate().unwrap();
    let encoded = toml::to_string(&launch).unwrap();
    toml::from_str::<LaunchSpec>(&encoded)
        .unwrap()
        .validate()
        .unwrap();
    assert!(toml::from_str::<LaunchSpec>(&format!("live_enabled=true\n{encoded}")).is_err());
    assert!(toml::from_str::<LaunchSpec>(
        &encoded.replace("components = 2", "components = 2\nunknown_cost = 1")
    )
    .is_err());
    for mutate in [
        |p: &mut ServicePolicy| p.max_sessions = 21,
        |p: &mut ServicePolicy| p.startup_timeout_seconds = 0,
        |p: &mut ServicePolicy| p.max_queue_age_seconds = 5,
        |p: &mut ServicePolicy| p.max_model_lateness_seconds = 20,
    ] {
        let mut p = policy();
        mutate(&mut p);
        assert!(p.validate().is_err());
    }
    let mut usd = LaunchSpec {
        runner: spec("USDRUBF"),
        policy: policy(),
    };
    if let Config::Usdrubf { settings } = &mut usd.runner.config {
        settings.bo_wait = 2;
    }
    assert!(usd.validate().is_err());
    for template in [
        include_str!("../../../../../../configs/examples/reaction-shadow.imoexf-combo.toml"),
        include_str!("../../../../../../configs/examples/reaction-shadow.usdrubf-h4-w4.toml"),
    ] {
        toml::from_str::<LaunchSpec>(template)
            .unwrap()
            .validate()
            .unwrap();
    }
}

#[test]
fn reaction_service_imoexf_template_uses_frozen_cost_policy() {
    let launch: LaunchSpec = toml::from_str(include_str!(
        "../../../../../../configs/examples/reaction-shadow.imoexf-combo.toml"
    ))
    .unwrap();
    let Config::Imoexf { costs, .. } = launch.runner.config else {
        panic!("IMOEXF launch template must select the IMOEXF engine");
    };
    assert_eq!(costs.point_value_rub, 10.0);
    assert_eq!(costs.tick_value_rub, 5.0);
    assert_eq!(costs.components, 2);
    assert_eq!(costs.rate_per_component_per_side, 0.00006575119054451348);
    let round_trip_fee = (2500.0 + 2500.0)
        * costs.point_value_rub
        * costs.rate_per_component_per_side
        * costs.components as f64;
    assert!((round_trip_fee - 6.575119054451348).abs() < 1e-12);
}

#[test]
fn reaction_service_deadline_no_intraday_resume_and_diagnostics() {
    let (_temp, mut r) = create("IMOEXF");
    assert!(ServiceGuard::new(&r, policy(), clock()).is_err());
    assert!(ServiceGuard::new(&r, policy(), epoch("2026-10-26 07:01")).is_err());
    let guard = ServiceGuard::new(&r, policy(), epoch("2026-10-26 06:59")).unwrap();
    let before = guard.status(&r).unwrap();
    assert!(!before.armed && !before.completed && !before.order_emission_allowed);
    assert_eq!(before.resource_state, "normal");
    assert_eq!(before.last_complete_session, None);
    warm(&mut r);
    assert!(ServiceGuard::new(&r, policy(), epoch("2026-10-27 06:59")).is_err());
    assert_eq!(guard.status(&r).unwrap().started_sessions, 1);
    assert!(guard.status(&r).unwrap().source_cursor.is_some());
    assert_eq!(guard.status(&r).unwrap().head_sha256, r.log.head());
}

#[test]
fn reaction_service_age_diagnostics_do_not_count_history_or_duplicates() {
    let (_temp, mut r) = create("USDRUBF");
    let guard = ServiceGuard::new(&r, policy(), epoch("2026-10-26 06:59")).unwrap();
    warm(&mut r);
    let mut b = bar("USDRUBF", epoch("2026-10-26 07:00"), 100.);
    now(b.close_time_utc + 610);
    append(&mut r, &b);
    now(b.close_time_utc + 650);
    let status = guard.status(&r).unwrap();
    assert_eq!(status.max_unobserved_queue_age_seconds, 40);
    assert_eq!(status.max_unobserved_model_lateness_seconds, 50);
    ingest(&mut r);
    observe(&mut r).unwrap();
    now(b.close_time_utc + 900);
    append(&mut r, &b);
    b.origin = DataOrigin::History;
    append(&mut r, &b);
    let status = guard.status(&r).unwrap();
    assert_eq!(status.max_unobserved_model_lateness_seconds, 0);
    assert_eq!(status.max_unobserved_queue_age_seconds, 0);
}

#[test]
fn reaction_service_complete_generation_is_not_restartable() {
    let (_temp, mut r) = create("IMOEXF");
    let mut limits = policy();
    limits.max_sessions = 1;
    let guard = ServiceGuard::new(&r, limits.clone(), epoch("2026-10-26 06:59")).unwrap();
    warm(&mut r);
    for slot in 0..101 {
        live_step(&mut r, slot, 100.).unwrap();
    }
    let status = guard.status(&r).unwrap();
    assert!(status.completed && status.runner.session_complete);
    assert_eq!(status.last_complete_session, Some(date("2026-10-26")));
    assert!(ServiceGuard::new(&r, limits, epoch("2026-10-27 06:59")).is_err());
}

#[tokio::test]
#[ignore = "starts owned loopback Redis; explicit local integration only"]
async fn reaction_service_readonly_redis_enforces_age_before_model_and_missing_bar() {
    use crate::ri_context::ingress::redis_source::tests::TestServer;
    use alor_protocol::{Envelope, MessageType};
    let server = TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("reaction_service")
        .arg("on")
        .arg(">fixture-password")
        .arg("~fixture:reaction:bars")
        .arg("-@all")
        .arg("+multi")
        .arg("+exec")
        .arg("+xrange")
        .arg("+ping")
        .query_async(&mut admin)
        .await
        .unwrap();
    let mut reader = redis::Client::open(format!(
        "redis://reaction_service:fixture-password@127.0.0.1:{}/",
        server.port
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    for fault in [
        "clean_eod",
        "source_delay",
        "queue_delay",
        "missing_bar",
        "startup_deadline",
    ] {
        let (_temp, mut r) = create("USDRUBF");
        let mut limits = policy();
        if fault == "clean_eod" {
            limits.max_sessions = 1;
        }
        let mut guard = ServiceGuard::new(&r, limits, epoch("2026-10-26 06:59")).unwrap();
        if fault == "startup_deadline" {
            now(epoch("2026-10-26 06:59"));
            assert!(guard.poll_once(&mut r, &mut reader).await.is_err());
            assert_eq!(r.log.count(), 0);
            continue;
        }
        let warmup = warm_bars("USDRUBF");
        for group in warmup.chunk_by(|a, b| {
            local(a.close_time_utc).unwrap().date() == local(b.close_time_utc).unwrap().date()
        }) {
            for b in group {
                let _: String = redis::cmd("XADD")
                    .arg("fixture:reaction:bars")
                    .arg("*")
                    .arg("payload")
                    .arg(
                        serde_json::to_string(&Envelope::new(1, "fixture", MessageType::Bar, b))
                            .unwrap(),
                    )
                    .query_async(&mut admin)
                    .await
                    .unwrap();
            }
            now(group.last().unwrap().close_time_utc + 600);
            assert!(guard.poll_once(&mut r, &mut reader).await.unwrap().armed);
        }
        now(epoch("2026-10-26 07:00"));
        assert!(
            guard
                .poll_once(&mut r, &mut reader)
                .await
                .unwrap()
                .runner
                .pair_ready
        );
        let before = r.log.count();
        let b = bar("USDRUBF", epoch("2026-10-26 07:00"), 100.);
        if fault == "clean_eod" {
            let mut completed = false;
            for slot in 0..101 {
                let price = [100.1, 100.12, 99.4, 99.6]
                    .get(slot)
                    .copied()
                    .unwrap_or(100.);
                let input = bar("USDRUBF", b.close_time_utc + slot as i64 * 600, price);
                let _: String = redis::cmd("XADD")
                    .arg("fixture:reaction:bars")
                    .arg("*")
                    .arg("payload")
                    .arg(
                        serde_json::to_string(&Envelope::new(
                            1,
                            "fixture",
                            MessageType::Bar,
                            &input,
                        ))
                        .unwrap(),
                    )
                    .query_async(&mut admin)
                    .await
                    .unwrap();
                now(input.close_time_utc + 600);
                let status = guard.poll_once(&mut r, &mut reader).await.unwrap();
                assert_eq!(status.completed, slot == 100);
                assert!(!status.order_emission_allowed);
                completed = status.completed;
            }
            assert!(completed && r.status().session_complete);
            assert!(
                !r.state.states().unwrap().unwrap()[0]["model"]["state"]["trades"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let commands: usize = redis::cmd("EXISTS")
                .arg("fixture:commands")
                .query_async(&mut admin)
                .await
                .unwrap();
            assert_eq!(commands, 0);
            let _: usize = redis::cmd("DEL")
                .arg("fixture:reaction:bars")
                .query_async(&mut admin)
                .await
                .unwrap();
            continue;
        }
        match fault {
            "source_delay" => {
                let _: String = redis::cmd("XADD")
                    .arg("fixture:reaction:bars")
                    .arg("*")
                    .arg("payload")
                    .arg(
                        serde_json::to_string(&Envelope::new(1, "fixture", MessageType::Bar, &b))
                            .unwrap(),
                    )
                    .query_async(&mut admin)
                    .await
                    .unwrap();
                // Freshly received does not imply a fresh model bar.
                now(b.close_time_utc + 721);
            }
            "queue_delay" => {
                now(b.close_time_utc + 600);
                append(&mut r, &b);
                now(b.close_time_utc + 631);
            }
            _ => now(b.close_time_utc + 721),
        }
        let error = guard.poll_once(&mut r, &mut reader).await.unwrap_err();
        let expected = match fault {
            "source_delay" => "source lateness exceeded",
            "queue_delay" => "queue age exceeded",
            _ => "expected live M10/EOD bar missing",
        };
        assert!(
            format!("{error:#}").contains(expected),
            "{fault}: {error:#}"
        );
        assert_eq!(r.log.count(), before, "{fault} emitted a transaction");
        assert!(r.status().failed);
        assert!(r.state.model_inputs.is_empty());
        // This is the owned test Redis, never a production connection.
        let _: usize = redis::cmd("DEL")
            .arg("fixture:reaction:bars")
            .query_async(&mut admin)
            .await
            .unwrap();
    }
}
