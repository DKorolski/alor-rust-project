use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

use alor_types::MarketState;

#[derive(Debug, Default, Clone, Serialize)]
pub struct CwsFrameTraceEntry {
    pub ts_utc: i64,
    pub seq: u64,
    pub direction: String,
    pub conn_instance_id: Option<String>,
    pub opcode: Option<String>,
    pub guid: Option<String>,
    pub request_guid: Option<String>,
    pub correlation_guid: Option<String>,
    pub order_id: Option<String>,
    pub symbol: Option<String>,
    pub message_class: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum GatewayPhase {
    #[default]
    SyncingHistory,
    Reconnecting,
    SyncingGap,
    LiveReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum ResyncMode {
    #[default]
    Cold,
    Warm,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct HealthState {
    pub stack_name: Option<String>,
    pub gateway_instance_id: Option<String>,
    pub auth_principal_fingerprint: Option<String>,
    pub access_token_fingerprint: Option<String>,
    pub access_token_last_source: Option<String>,
    pub access_token_last_consumer: Option<String>,
    pub access_token_obtained_ts_utc: Option<i64>,
    pub access_token_last_used_ts_utc: Option<i64>,
    pub access_token_age_ms: Option<u64>,
    pub access_token_ttl_remaining_ms: Option<u64>,
    pub ws_connected: bool,
    pub cws_authorized: bool,
    pub cws_connection_instance_id: Option<String>,
    pub cws_connect_seq: u64,
    pub cws_reconnect_seq: u64,
    pub cws_connected_ts_utc: Option<i64>,
    pub cws_last_connect_ts_utc: Option<i64>,
    pub cws_last_transport_failure_ts_utc: Option<i64>,
    pub cws_last_rx_ts_utc: Option<i64>,
    pub cws_last_tx_ts_utc: Option<i64>,
    pub cws_last_limit_send_ts_utc: Option<i64>,
    pub cws_last_limit_error_ts_utc: Option<i64>,
    pub cws_last_successful_send_ts_utc: Option<i64>,
    pub cws_last_successful_ack_ts_utc: Option<i64>,
    pub cws_last_control_success_ts_utc: Option<i64>,
    pub cws_last_control_failure_ts_utc: Option<i64>,
    pub cws_last_ping_ts_utc: Option<i64>,
    pub cws_last_pong_ts_utc: Option<i64>,
    pub cws_pending_count: u64,
    pub cws_pending_guids: Vec<String>,
    pub cws_oldest_pending_age_ms: Option<u64>,
    pub request_map_size: u64,
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
    pub commands_received_total: u64,
    pub commands_accepted_total: u64,
    pub commands_rejected_total: u64,
    pub commands_duplicate_total: u64,
    pub command_duplicate_total: u64,
    pub command_expired_total: u64,
    pub command_validation_failed_total: u64,
    pub commands_rejected_http_code_total: HashMap<i64, u64>,
    pub cws_errors_total: u64,
    pub cws_reconnects_total: u64,
    pub cws_connect_total: u64,
    pub cws_reconnect_total: u64,
    pub cws_protocol_reset_total: u64,
    pub cws_limit_send_total: u64,
    pub cws_limit_error_total: u64,
    pub cws_pending_failed_total: u64,
    pub cws_create_limit_send_total: u64,
    pub cws_create_limit_success_total: u64,
    pub cws_create_limit_failure_total: u64,
    pub cws_delete_limit_send_total: u64,
    pub cws_delete_limit_success_total: u64,
    pub cws_delete_limit_failure_total: u64,
    pub cws_replace_limit_send_total: u64,
    pub cws_replace_limit_success_total: u64,
    pub cws_replace_limit_failure_total: u64,
    pub orders_ws_events_total: u64,
    pub orders_ws_status_total: HashMap<String, u64>,
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
    pub scheduler_state: Option<MarketState>,
    pub cws_recent_inbound_frames: Vec<CwsFrameTraceEntry>,
    pub cws_recent_outbound_frames: Vec<CwsFrameTraceEntry>,
    #[serde(skip)]
    pub cws_connected_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_rx_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_tx_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_control_success_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_control_failure_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_ping_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_pong_at: Option<Instant>,
    #[serde(skip)]
    pub cws_last_reconnect_at: Option<Instant>,
}
