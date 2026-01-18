use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GatewayPhase {
    SyncingHistory,
    LiveReady,
}

impl Default for GatewayPhase {
    fn default() -> Self {
        Self::SyncingHistory
    }
}

#[derive(Debug, Default)]
pub struct HealthState {
    pub ws_connected: bool,
    pub cws_authorized: bool,
    pub last_bar_ts: i64,
    pub last_bar_age_sec: u64,
    pub last_positions_ts: i64,
    pub last_orders_ts: i64,
    pub reconnect_count: u64,
    pub token_refresh_count: u64,
    pub gateway_phase: GatewayPhase,
    pub readiness: bool,
}
