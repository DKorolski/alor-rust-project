use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use parking_lot::RwLock;
use serde::Serialize;

use crate::health::{GatewayPhase, HealthState};

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    readiness: bool,
    gateway_phase: GatewayPhase,
    ws_connected: bool,
    cws_authorized: bool,
    last_bar_age_sec: u64,
    last_positions_ts: i64,
    last_orders_ts: i64,
    backpressure_lagged: bool,
}

pub async fn serve(health: Arc<RwLock<HealthState>>, addr: String) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/liveness", get(liveness))
        .route("/readiness", get(readiness))
        .with_state(health);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn liveness() -> StatusCode {
    StatusCode::OK
}

async fn readiness(State(health): State<Arc<RwLock<HealthState>>>) -> Json<ReadinessResponse> {
    let guard = health.read();
    Json(ReadinessResponse {
        readiness: guard.readiness,
        gateway_phase: guard.gateway_phase,
        ws_connected: guard.ws_connected,
        cws_authorized: guard.cws_authorized,
        last_bar_age_sec: guard.last_bar_age_sec,
        last_positions_ts: guard.last_positions_ts,
        last_orders_ts: guard.last_orders_ts,
        backpressure_lagged: guard.backpressure_lagged,
    })
}
