use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GatewayPhase {
    SyncingHistory,
    LiveReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ResyncMode {
    Cold,
    Warm,
}

impl Default for ResyncMode {
    fn default() -> Self {
        Self::Cold
    }
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
    pub backpressure_lagged: bool,
    pub ws_last_rx_ts: i64,
    pub ws_last_rx_age_sec: u64,
    pub ws_reconnects_total: u64,
    pub last_resync_mode: ResyncMode,
    pub last_gap_backfill_sec: u64,
    pub last_gap_backfill_bars: u64,
    pub active_subscriptions_count: u32,
    pub desired_subscriptions_count: u32,
}
