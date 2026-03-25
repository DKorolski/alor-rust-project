use std::time::Instant;
use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use parking_lot::RwLock;
use serde::Serialize;
use tracing::info;

use crate::health::{CwsFrameTraceEntry, GatewayPhase, HealthState, ResyncMode};
use alor_types::MarketState;

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    readiness: bool,
    gateway_phase: GatewayPhase,
    last_resync_mode: ResyncMode,
    stack_name: Option<String>,
    gateway_instance_id: Option<String>,
    auth_principal_fingerprint: Option<String>,
    access_token_fingerprint: Option<String>,
    access_token_last_source: Option<String>,
    access_token_last_consumer: Option<String>,
    access_token_obtained_ts_utc: Option<i64>,
    access_token_last_used_ts_utc: Option<i64>,
    access_token_age_ms: Option<u64>,
    access_token_ttl_remaining_ms: Option<u64>,
    ws_connected: bool,
    cws_authorized: bool,
    cws_connection_instance_id: Option<String>,
    cws_connect_seq: u64,
    cws_reconnect_seq: u64,
    cws_connected_ts_utc: Option<i64>,
    cws_last_connect_ts_utc: Option<i64>,
    cws_last_transport_failure_ts_utc: Option<i64>,
    cws_last_rx_ts_utc: Option<i64>,
    cws_last_rx_age_ms: Option<u64>,
    cws_last_tx_ts_utc: Option<i64>,
    cws_last_tx_age_ms: Option<u64>,
    cws_last_limit_send_ts_utc: Option<i64>,
    cws_last_limit_error_ts_utc: Option<i64>,
    cws_last_successful_send_ts_utc: Option<i64>,
    cws_last_successful_ack_ts_utc: Option<i64>,
    cws_last_control_success_ts_utc: Option<i64>,
    cws_last_control_success_age_ms: Option<u64>,
    cws_last_control_failure_ts_utc: Option<i64>,
    cws_last_control_failure_age_ms: Option<u64>,
    cws_last_ping_ts_utc: Option<i64>,
    cws_last_pong_ts_utc: Option<i64>,
    cws_last_ping_pong_age_ms: Option<u64>,
    cws_connect_total: u64,
    cws_reconnect_total: u64,
    cws_protocol_reset_total: u64,
    cws_limit_send_total: u64,
    cws_limit_error_total: u64,
    cws_pending_failed_total: u64,
    cws_pending_count: u64,
    cws_pending_guids: Vec<String>,
    cws_oldest_pending_age_ms: Option<u64>,
    request_map_size: u64,
    cws_create_limit_send_total: u64,
    cws_create_limit_success_total: u64,
    cws_create_limit_failure_total: u64,
    cws_delete_limit_send_total: u64,
    cws_delete_limit_success_total: u64,
    cws_delete_limit_failure_total: u64,
    cws_replace_limit_send_total: u64,
    cws_replace_limit_success_total: u64,
    cws_replace_limit_failure_total: u64,
    reconnect_count: u64,
    token_refresh_count: u64,
    last_bar_age_sec: u64,
    ws_last_rx_age_sec: u64,
    last_positions_ts: i64,
    last_orders_ts: i64,
    backpressure_lagged: bool,
    event_backpressure_lagged: bool,
    event_sink_degraded: bool,
    event_publish_retries_total: u64,
    event_publish_timeout_total: u64,
    event_publish_fail_total: u64,
    event_queue_full_drops_total: u64,
    last_event_publish_ts: i64,
    active_subscriptions_count: u32,
    desired_subscriptions_count: u32,
    commands_received_total: u64,
    commands_accepted_total: u64,
    commands_rejected_total: u64,
    commands_duplicate_total: u64,
    command_duplicate_total: u64,
    command_expired_total: u64,
    command_validation_failed_total: u64,
    command_processed_total: u64,
    command_consumer_alive: bool,
    command_consumer_last_poll_ts_utc: i64,
    command_consumer_last_message_id: Option<String>,
    command_consumer_errors_total: u64,
    command_consumer_redis_timeouts_total: u64,
    cws_errors_total: u64,
    orders_ws_events_total: u64,
    scheduler_state: Option<MarketState>,
    last_gap_backfill_sec: u64,
    last_gap_backfill_bars: u64,
    ws_reconnects_total: u64,
}

#[derive(Debug, Serialize)]
struct CwsDebugResponse {
    stack_name: Option<String>,
    gateway_instance_id: Option<String>,
    auth_principal_fingerprint: Option<String>,
    access_token_fingerprint: Option<String>,
    access_token_last_source: Option<String>,
    access_token_last_consumer: Option<String>,
    cws_authorized: bool,
    cws_connection_instance_id: Option<String>,
    cws_connect_seq: u64,
    cws_reconnect_seq: u64,
    cws_connected_ts_utc: Option<i64>,
    cws_last_rx_ts_utc: Option<i64>,
    cws_last_rx_age_ms: Option<u64>,
    cws_last_tx_ts_utc: Option<i64>,
    cws_last_tx_age_ms: Option<u64>,
    cws_last_control_success_ts_utc: Option<i64>,
    cws_last_control_success_age_ms: Option<u64>,
    cws_last_control_failure_ts_utc: Option<i64>,
    cws_last_control_failure_age_ms: Option<u64>,
    cws_last_ping_ts_utc: Option<i64>,
    cws_last_pong_ts_utc: Option<i64>,
    cws_last_ping_pong_age_ms: Option<u64>,
    cws_pending_count: u64,
    cws_pending_guids: Vec<String>,
    cws_oldest_pending_age_ms: Option<u64>,
    request_map_size: u64,
    cws_create_limit_send_total: u64,
    cws_create_limit_success_total: u64,
    cws_create_limit_failure_total: u64,
    cws_delete_limit_send_total: u64,
    cws_delete_limit_success_total: u64,
    cws_delete_limit_failure_total: u64,
    cws_replace_limit_send_total: u64,
    cws_replace_limit_success_total: u64,
    cws_replace_limit_failure_total: u64,
    recent_inbound_frames: Vec<CwsFrameTraceEntry>,
    recent_outbound_frames: Vec<CwsFrameTraceEntry>,
}

pub async fn serve(health: Arc<RwLock<HealthState>>, addr: String) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/liveness", get(liveness))
        .route("/readiness", get(readiness))
        .route("/debug/cws", get(cws_debug))
        .with_state(health);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    info!(%local_addr, "health server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn liveness() -> StatusCode {
    StatusCode::OK
}

async fn readiness(
    State(health): State<Arc<RwLock<HealthState>>>,
) -> (StatusCode, Json<ReadinessResponse>) {
    let guard = health.read();
    let cws_last_rx_age_ms = age_ms(guard.cws_last_rx_at);
    let cws_last_tx_age_ms = age_ms(guard.cws_last_tx_at);
    let cws_last_control_success_age_ms = age_ms(guard.cws_last_control_success_at);
    let cws_last_control_failure_age_ms = age_ms(guard.cws_last_control_failure_at);
    let cws_last_ping_pong_age_ms = min_age_ms(guard.cws_last_ping_at, guard.cws_last_pong_at);
    let status = if guard.readiness {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(ReadinessResponse {
            readiness: guard.readiness,
            gateway_phase: guard.gateway_phase,
            last_resync_mode: guard.last_resync_mode,
            stack_name: guard.stack_name.clone(),
            gateway_instance_id: guard.gateway_instance_id.clone(),
            auth_principal_fingerprint: guard.auth_principal_fingerprint.clone(),
            access_token_fingerprint: guard.access_token_fingerprint.clone(),
            access_token_last_source: guard.access_token_last_source.clone(),
            access_token_last_consumer: guard.access_token_last_consumer.clone(),
            access_token_obtained_ts_utc: guard.access_token_obtained_ts_utc,
            access_token_last_used_ts_utc: guard.access_token_last_used_ts_utc,
            access_token_age_ms: guard.access_token_age_ms,
            access_token_ttl_remaining_ms: guard.access_token_ttl_remaining_ms,
            ws_connected: guard.ws_connected,
            cws_authorized: guard.cws_authorized,
            cws_connection_instance_id: guard.cws_connection_instance_id.clone(),
            cws_connect_seq: guard.cws_connect_seq,
            cws_reconnect_seq: guard.cws_reconnect_seq,
            cws_connected_ts_utc: guard.cws_connected_ts_utc,
            cws_last_connect_ts_utc: guard.cws_last_connect_ts_utc,
            cws_last_transport_failure_ts_utc: guard.cws_last_transport_failure_ts_utc,
            cws_last_rx_ts_utc: guard.cws_last_rx_ts_utc,
            cws_last_rx_age_ms,
            cws_last_tx_ts_utc: guard.cws_last_tx_ts_utc,
            cws_last_tx_age_ms,
            cws_last_limit_send_ts_utc: guard.cws_last_limit_send_ts_utc,
            cws_last_limit_error_ts_utc: guard.cws_last_limit_error_ts_utc,
            cws_last_successful_send_ts_utc: guard.cws_last_successful_send_ts_utc,
            cws_last_successful_ack_ts_utc: guard.cws_last_successful_ack_ts_utc,
            cws_last_control_success_ts_utc: guard.cws_last_control_success_ts_utc,
            cws_last_control_success_age_ms,
            cws_last_control_failure_ts_utc: guard.cws_last_control_failure_ts_utc,
            cws_last_control_failure_age_ms,
            cws_last_ping_ts_utc: guard.cws_last_ping_ts_utc,
            cws_last_pong_ts_utc: guard.cws_last_pong_ts_utc,
            cws_last_ping_pong_age_ms,
            cws_connect_total: guard.cws_connect_total,
            cws_reconnect_total: guard.cws_reconnect_total,
            cws_protocol_reset_total: guard.cws_protocol_reset_total,
            cws_limit_send_total: guard.cws_limit_send_total,
            cws_limit_error_total: guard.cws_limit_error_total,
            cws_pending_failed_total: guard.cws_pending_failed_total,
            cws_pending_count: guard.cws_pending_count,
            cws_pending_guids: guard.cws_pending_guids.clone(),
            cws_oldest_pending_age_ms: guard.cws_oldest_pending_age_ms,
            request_map_size: guard.request_map_size,
            cws_create_limit_send_total: guard.cws_create_limit_send_total,
            cws_create_limit_success_total: guard.cws_create_limit_success_total,
            cws_create_limit_failure_total: guard.cws_create_limit_failure_total,
            cws_delete_limit_send_total: guard.cws_delete_limit_send_total,
            cws_delete_limit_success_total: guard.cws_delete_limit_success_total,
            cws_delete_limit_failure_total: guard.cws_delete_limit_failure_total,
            cws_replace_limit_send_total: guard.cws_replace_limit_send_total,
            cws_replace_limit_success_total: guard.cws_replace_limit_success_total,
            cws_replace_limit_failure_total: guard.cws_replace_limit_failure_total,
            reconnect_count: guard.reconnect_count,
            token_refresh_count: guard.token_refresh_count,
            last_bar_age_sec: guard.last_bar_age_sec,
            ws_last_rx_age_sec: guard.ws_last_rx_age_sec,
            last_positions_ts: guard.last_positions_ts,
            last_orders_ts: guard.last_orders_ts,
            backpressure_lagged: guard.backpressure_lagged,
            event_backpressure_lagged: guard.event_backpressure_lagged,
            event_sink_degraded: guard.event_sink_degraded,
            event_publish_retries_total: guard.event_publish_retries_total,
            event_publish_timeout_total: guard.event_publish_timeout_total,
            event_publish_fail_total: guard.event_publish_fail_total,
            event_queue_full_drops_total: guard.event_queue_full_drops_total,
            last_event_publish_ts: guard.last_event_publish_ts,
            active_subscriptions_count: guard.active_subscriptions_count,
            desired_subscriptions_count: guard.desired_subscriptions_count,
            commands_received_total: guard.commands_received_total,
            commands_accepted_total: guard.commands_accepted_total,
            commands_rejected_total: guard.commands_rejected_total,
            commands_duplicate_total: guard.commands_duplicate_total,
            command_duplicate_total: guard.command_duplicate_total,
            command_expired_total: guard.command_expired_total,
            command_validation_failed_total: guard.command_validation_failed_total,
            command_processed_total: guard.command_processed_total,
            command_consumer_alive: guard.command_consumer_alive,
            command_consumer_last_poll_ts_utc: guard.command_consumer_last_poll_ts_utc,
            command_consumer_last_message_id: guard.command_consumer_last_message_id.clone(),
            command_consumer_errors_total: guard.command_consumer_errors_total,
            command_consumer_redis_timeouts_total: guard.command_consumer_redis_timeouts_total,
            cws_errors_total: guard.cws_errors_total,
            orders_ws_events_total: guard.orders_ws_events_total,
            scheduler_state: guard.scheduler_state,
            last_gap_backfill_sec: guard.last_gap_backfill_sec,
            last_gap_backfill_bars: guard.last_gap_backfill_bars,
            ws_reconnects_total: guard.ws_reconnects_total,
        }),
    )
}

async fn cws_debug(
    State(health): State<Arc<RwLock<HealthState>>>,
) -> (StatusCode, Json<CwsDebugResponse>) {
    let guard = health.read();
    let status = if guard.readiness {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(CwsDebugResponse {
            stack_name: guard.stack_name.clone(),
            gateway_instance_id: guard.gateway_instance_id.clone(),
            auth_principal_fingerprint: guard.auth_principal_fingerprint.clone(),
            access_token_fingerprint: guard.access_token_fingerprint.clone(),
            access_token_last_source: guard.access_token_last_source.clone(),
            access_token_last_consumer: guard.access_token_last_consumer.clone(),
            cws_authorized: guard.cws_authorized,
            cws_connection_instance_id: guard.cws_connection_instance_id.clone(),
            cws_connect_seq: guard.cws_connect_seq,
            cws_reconnect_seq: guard.cws_reconnect_seq,
            cws_connected_ts_utc: guard.cws_connected_ts_utc,
            cws_last_rx_ts_utc: guard.cws_last_rx_ts_utc,
            cws_last_rx_age_ms: age_ms(guard.cws_last_rx_at),
            cws_last_tx_ts_utc: guard.cws_last_tx_ts_utc,
            cws_last_tx_age_ms: age_ms(guard.cws_last_tx_at),
            cws_last_control_success_ts_utc: guard.cws_last_control_success_ts_utc,
            cws_last_control_success_age_ms: age_ms(guard.cws_last_control_success_at),
            cws_last_control_failure_ts_utc: guard.cws_last_control_failure_ts_utc,
            cws_last_control_failure_age_ms: age_ms(guard.cws_last_control_failure_at),
            cws_last_ping_ts_utc: guard.cws_last_ping_ts_utc,
            cws_last_pong_ts_utc: guard.cws_last_pong_ts_utc,
            cws_last_ping_pong_age_ms: min_age_ms(guard.cws_last_ping_at, guard.cws_last_pong_at),
            cws_pending_count: guard.cws_pending_count,
            cws_pending_guids: guard.cws_pending_guids.clone(),
            cws_oldest_pending_age_ms: guard.cws_oldest_pending_age_ms,
            request_map_size: guard.request_map_size,
            cws_create_limit_send_total: guard.cws_create_limit_send_total,
            cws_create_limit_success_total: guard.cws_create_limit_success_total,
            cws_create_limit_failure_total: guard.cws_create_limit_failure_total,
            cws_delete_limit_send_total: guard.cws_delete_limit_send_total,
            cws_delete_limit_success_total: guard.cws_delete_limit_success_total,
            cws_delete_limit_failure_total: guard.cws_delete_limit_failure_total,
            cws_replace_limit_send_total: guard.cws_replace_limit_send_total,
            cws_replace_limit_success_total: guard.cws_replace_limit_success_total,
            cws_replace_limit_failure_total: guard.cws_replace_limit_failure_total,
            recent_inbound_frames: guard.cws_recent_inbound_frames.clone(),
            recent_outbound_frames: guard.cws_recent_outbound_frames.clone(),
        }),
    )
}

fn age_ms(value: Option<Instant>) -> Option<u64> {
    value.map(|instant| instant.elapsed().as_millis() as u64)
}

fn min_age_ms(first: Option<Instant>, second: Option<Instant>) -> Option<u64> {
    match (age_ms(first), age_ms(second)) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}
