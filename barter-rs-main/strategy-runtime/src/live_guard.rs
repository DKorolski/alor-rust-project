use serde::{Deserialize, Serialize};

use crate::TradeMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayPhase {
    #[serde(alias = "SyncingHistory", alias = "syncingHistory")]
    SyncingHistory,
    #[serde(alias = "Reconnecting", alias = "reconnecting")]
    Reconnecting,
    #[serde(alias = "SyncingGap", alias = "syncingGap")]
    SyncingGap,
    #[serde(alias = "LiveReady", alias = "liveReady")]
    LiveReady,
}

impl Default for GatewayPhase {
    fn default() -> Self {
        Self::SyncingHistory
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEvent {
    #[serde(default)]
    pub gateway_phase: GatewayPhase,
    #[serde(default)]
    pub readiness: bool,
    #[serde(default)]
    pub cws_authorized: bool,
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

pub fn evaluate_live_guard(
    trade_mode: TradeMode,
    allow_live_orders: bool,
    state: &LiveGuardState,
    has_bars: bool,
) -> LiveGuardDecision {
    let mut reasons = Vec::new();
    if trade_mode != TradeMode::Live {
        reasons.push(format!("trade_mode={trade_mode:?}"));
    }
    if !allow_live_orders {
        reasons.push("allow_live_orders=false".to_string());
    }
    if trade_mode == TradeMode::Live && !has_bars {
        reasons.push("bars_read_total=0".to_string());
    }
    let phase = state
        .health
        .as_ref()
        .map(|health| health.gateway_phase)
        .unwrap_or(GatewayPhase::SyncingHistory);
    if phase != GatewayPhase::LiveReady {
        reasons.push(format!("phase={phase:?}"));
    }
    let allowed = reasons.is_empty();
    LiveGuardDecision { allowed, reasons }
}
