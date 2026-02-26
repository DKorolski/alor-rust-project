use strategy_runtime::live_guard::{
    evaluate_live_guard, GatewayPhase, HealthEvent, LiveGuardState,
};
use strategy_runtime::TradeMode;

fn state(health: Option<HealthEvent>) -> LiveGuardState {
    LiveGuardState { health }
}

#[test]
fn readiness_false_when_live_guard_blocked() {
    let decision = evaluate_live_guard(
        TradeMode::Paper,
        false,
        &state(None),
        false,
        false,
        10,
        20,
        true,
    );
    assert!(!decision.allowed);
}

#[test]
fn readiness_false_when_gateway_health_stale() {
    let decision = evaluate_live_guard(
        TradeMode::Live,
        true,
        &state(Some(HealthEvent {
            gateway_phase: GatewayPhase::LiveReady,
            readiness: true,
            ws_connected: true,
            cws_authorized: true,
            scheduler_state: Some("Open".to_string()),
            last_event_ts: 1,
        })),
        true,
        true,
        100,
        20,
        true,
    );
    assert!(!decision.allowed);
    assert!(decision.reasons.iter().any(|r| r == "gateway_health_stale"));
}

#[test]
fn readiness_false_when_require_gateway_ready_and_gateway_ready_false() {
    let decision = evaluate_live_guard(
        TradeMode::Live,
        true,
        &state(Some(HealthEvent {
            gateway_phase: GatewayPhase::LiveReady,
            readiness: false,
            ws_connected: true,
            cws_authorized: true,
            scheduler_state: Some("Open".to_string()),
            last_event_ts: 100,
        })),
        true,
        true,
        101,
        20,
        true,
    );
    assert!(!decision.allowed);
    assert!(decision.reasons.iter().any(|r| r == "gateway_ready=false"));
}

#[test]
fn readiness_true_when_allowed_and_gateway_ready() {
    let decision = evaluate_live_guard(
        TradeMode::Live,
        true,
        &state(Some(HealthEvent {
            gateway_phase: GatewayPhase::LiveReady,
            readiness: true,
            ws_connected: true,
            cws_authorized: true,
            scheduler_state: Some("Open".to_string()),
            last_event_ts: 100,
        })),
        true,
        true,
        101,
        20,
        true,
    );
    assert!(decision.allowed, "reasons={:?}", decision.reasons);
}
