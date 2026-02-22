use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use parking_lot::RwLock;
use serde::Serialize;
use tracing::info;

use crate::health::{GatewayPhase, HealthState, ResyncMode};
use alor_types::MarketState;

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    readiness: bool,
    gateway_phase: GatewayPhase,
    last_resync_mode: ResyncMode,
    ws_connected: bool,
    cws_authorized: bool,
    last_bar_age_sec: u64,
    ws_last_rx_age_sec: u64,
    last_positions_ts: i64,
    last_orders_ts: i64,
    backpressure_lagged: bool,
    active_subscriptions_count: u32,
    desired_subscriptions_count: u32,
    scheduler_state: Option<MarketState>,
    last_gap_backfill_sec: u64,
    last_gap_backfill_bars: u64,
    ws_reconnects_total: u64,
}

pub async fn serve(health: Arc<RwLock<HealthState>>, addr: String) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/liveness", get(liveness))
        .route("/readiness", get(readiness))
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
            ws_connected: guard.ws_connected,
            cws_authorized: guard.cws_authorized,
            last_bar_age_sec: guard.last_bar_age_sec,
            ws_last_rx_age_sec: guard.ws_last_rx_age_sec,
            last_positions_ts: guard.last_positions_ts,
            last_orders_ts: guard.last_orders_ts,
            backpressure_lagged: guard.backpressure_lagged,
            active_subscriptions_count: guard.active_subscriptions_count,
            desired_subscriptions_count: guard.desired_subscriptions_count,
            scheduler_state: guard.scheduler_state,
            last_gap_backfill_sec: guard.last_gap_backfill_sec,
            last_gap_backfill_bars: guard.last_gap_backfill_bars,
            ws_reconnects_total: guard.ws_reconnects_total,
        }),
    )
}
