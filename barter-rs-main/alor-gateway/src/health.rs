use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GatewayPhase {
    SyncingHistory,
    Reconnecting,
    SyncingGap,
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

#[derive(Debug, Default, Clone, Serialize)]
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
    pub event_backpressure_lagged: bool,
    pub event_sink_degraded: bool,
    pub event_publish_retries_total: u64,
    pub event_publish_timeout_total: u64,
    pub event_publish_fail_total: u64,
    pub event_queue_full_drops_total: u64,
    pub last_event_publish_ts: i64,
    pub command_processed_total: u64,
    pub command_duplicate_total: u64,
    pub command_expired_total: u64,
    pub command_validation_failed_total: u64,
    pub command_consumer_alive: bool,
    pub command_consumer_last_poll_ts_utc: i64,
    pub command_consumer_last_message_id: Option<String>,
    pub command_consumer_errors_total: u64,
    pub command_consumer_redis_timeouts_total: u64,
    pub ws_last_rx_ts: i64,
    pub ws_last_rx_age_sec: u64,
    pub ws_reconnects_total: u64,
    pub last_resync_mode: ResyncMode,
    pub last_gap_backfill_sec: u64,
    pub last_gap_backfill_bars: u64,
    pub active_subscriptions_count: u32,
    pub desired_subscriptions_count: u32,
}
