use serde::{Deserialize, Serialize};

use crate::TradeMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GatewayPhase {
    #[serde(alias = "SyncingHistory", alias = "syncingHistory")]
    #[default]
    SyncingHistory,
    #[serde(alias = "Reconnecting", alias = "reconnecting")]
    Reconnecting,
    #[serde(alias = "SyncingGap", alias = "syncingGap")]
    SyncingGap,
    #[serde(alias = "LiveReady", alias = "liveReady")]
    LiveReady,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEvent {
    #[serde(default)]
    pub gateway_phase: GatewayPhase,
    #[serde(default)]
    pub readiness: bool,
    #[serde(default)]
    pub ws_connected: bool,
    #[serde(default)]
    pub cws_authorized: bool,
    #[serde(default)]
    pub scheduler_state: Option<String>,
    #[serde(default, alias = "last_event_publish_ts", alias = "last_event_ts")]
    pub last_event_ts: i64,
}

#[derive(Debug, Clone, Default)]
pub struct LiveGuardState {
    pub health: Option<HealthEvent>,
}

impl LiveGuardState {
    pub fn update_health(&mut self, health: HealthEvent) {
        self.health = Some(health);
    }
}

#[derive(Debug, Clone)]
pub struct LiveGuardDecision {
    pub allowed: bool,
    pub reasons: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate_live_guard(
    trade_mode: TradeMode,
    allow_live_orders: bool,
    state: &LiveGuardState,
    has_bars: bool,
    bars_stream_has_data: bool,
    now_ts_utc: i64,
    gateway_health_stale_sec: u64,
    require_gateway_ready: bool,
) -> LiveGuardDecision {
    let mut reasons = Vec::new();
    if trade_mode != TradeMode::Live {
        reasons.push(format!("trade_mode={trade_mode:?}"));
    }
    if !allow_live_orders {
        reasons.push("allow_live_orders=false".to_string());
    }
    if trade_mode == TradeMode::Live && !has_bars {
        if bars_stream_has_data {
            reasons.push("waiting_for_next_bar_after_restart".to_string());
        } else {
            reasons.push("waiting_for_first_bar".to_string());
        }
    }
    let phase = state
        .health
        .as_ref()
        .map(|health| health.gateway_phase)
        .unwrap_or(GatewayPhase::SyncingHistory);
    if phase != GatewayPhase::LiveReady {
        reasons.push(format!("phase={phase:?}"));
    }

    let stale_sec = i64::try_from(gateway_health_stale_sec).unwrap_or(i64::MAX);
    let is_stale = state
        .health
        .as_ref()
        .map(|health| {
            health.last_event_ts <= 0 || now_ts_utc.saturating_sub(health.last_event_ts) > stale_sec
        })
        .unwrap_or(true);
    if is_stale {
        reasons.push("gateway_health_stale".to_string());
    }

    if require_gateway_ready {
        let gateway_ready = state.health.as_ref().map(|h| h.readiness).unwrap_or(false);
        let ws_connected = state
            .health
            .as_ref()
            .map(|h| h.ws_connected)
            .unwrap_or(false);
        let cws_authorized = state
            .health
            .as_ref()
            .map(|h| h.cws_authorized)
            .unwrap_or(false);
        if !gateway_ready {
            reasons.push("gateway_ready=false".to_string());
        }
        if !ws_connected {
            reasons.push("ws_connected=false".to_string());
        }
        if !cws_authorized {
            reasons.push("cws_authorized=false".to_string());
        }
    }
    let allowed = reasons.is_empty();
    LiveGuardDecision { allowed, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_when_gateway_health_is_stale() {
        let state = LiveGuardState {
            health: Some(HealthEvent {
                gateway_phase: GatewayPhase::LiveReady,
                readiness: true,
                ws_connected: true,
                cws_authorized: true,
                scheduler_state: Some("Open".to_string()),
                last_event_ts: 100,
            }),
        };

        let decision =
            evaluate_live_guard(TradeMode::Live, true, &state, true, true, 130, 20, true);

        assert!(!decision.allowed);
        assert!(decision.reasons.iter().any(|r| r == "gateway_health_stale"));
    }

    #[test]
    fn allows_when_gateway_health_is_fresh() {
        let state = LiveGuardState {
            health: Some(HealthEvent {
                gateway_phase: GatewayPhase::LiveReady,
                readiness: true,
                ws_connected: true,
                cws_authorized: true,
                scheduler_state: Some("Open".to_string()),
                last_event_ts: 115,
            }),
        };

        let decision =
            evaluate_live_guard(TradeMode::Live, true, &state, true, true, 130, 20, true);

        assert!(decision.allowed);
    }
}
