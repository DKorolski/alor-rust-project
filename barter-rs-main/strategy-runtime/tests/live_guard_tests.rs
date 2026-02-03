use serde_json::from_str;
use strategy_runtime::live_guard::{
    evaluate_live_guard, GatewayPhase, HealthEvent, LiveGuardState,
};
use strategy_runtime::TradeMode;

#[test]
fn live_guard_requires_live_phase() {
    let mut state = LiveGuardState::default();
    state.update_health(HealthEvent {
        gateway_phase: GatewayPhase::LiveReady,
        readiness: true,
        cws_authorized: true,
        last_event_ts: 1,
    });

    let decision = evaluate_live_guard(TradeMode::Live, true, &state, true);
    assert!(decision.allowed);
}

#[test]
fn live_guard_blocks_when_mode_not_live() {
    let mut state = LiveGuardState::default();
    state.update_health(HealthEvent {
        gateway_phase: GatewayPhase::LiveReady,
        readiness: true,
        cws_authorized: true,
        last_event_ts: 2,
    });

    let decision = evaluate_live_guard(TradeMode::Paper, true, &state, true);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .iter()
        .any(|reason| reason.contains("trade_mode")));
}

#[test]
fn gateway_phase_accepts_pascal_case_alias() {
    let payload = r#"
{
  "schema_version": 1,
  "ts_utc": 1,
  "source": "test",
  "msg_type": "health",
  "payload": {
    "gateway_phase": "LiveReady",
    "readiness": true,
    "cws_authorized": true,
    "last_event_ts": 1
  }
}
"#;
    let parsed: alor_protocol::Envelope<HealthEvent> = from_str(payload).expect("deserialize");
    assert_eq!(parsed.payload.gateway_phase, GatewayPhase::LiveReady);
}
