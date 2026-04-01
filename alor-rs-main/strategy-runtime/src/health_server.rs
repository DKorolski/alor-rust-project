use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use parking_lot::RwLock;
use serde::Serialize;

use crate::RuntimeHealthSnapshot;

pub type RuntimeSharedState = Arc<RwLock<RuntimeHealthSnapshot>>;

#[derive(Debug, Clone)]
pub struct HealthCfg {
    pub enabled: bool,
    pub listen_addr: String,
    pub expose_metrics: bool,
}

#[derive(Debug, Serialize)]
struct BuildInfo {
    version: &'static str,
    git_sha: &'static str,
}

#[derive(Debug, Serialize)]
struct LivenessResponse {
    liveness: bool,
    build: BuildInfo,
    uptime_sec: u64,
}

#[derive(Debug, Serialize)]
struct GatewayReadinessResponse {
    health_age_sec: Option<i64>,
    require_gateway_ready: bool,
    gateway_ready: Option<bool>,
    ws_connected: Option<bool>,
    cws_authorized: Option<bool>,
    scheduler_state: Option<String>,
}

#[derive(Debug, Serialize)]
struct SchedulerReadinessResponse {
    state: String,
    now_local: String,
    note: Option<String>,
    timezone_offset_hours: i32,
}

#[derive(Debug, Serialize)]
struct StreamsReadinessResponse {
    last_bar_ts_utc: Option<i64>,
    last_ack_ts_utc: Option<i64>,
    last_intent_ts_utc: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    readiness: bool,
    runtime_phase: String,
    live_guard: String,
    live_guard_reasons: Vec<String>,
    exit_recovery_active: bool,
    close_only_degraded: bool,
    operator_intervention_required: bool,
    open_risk_position_unflattened: bool,
    orders_mode: String,
    allow_live_orders: bool,
    allow_paper_orders: bool,
    gateway: GatewayReadinessResponse,
    scheduler: SchedulerReadinessResponse,
    streams: StreamsReadinessResponse,
}

pub async fn spawn_health_server(shared: RuntimeSharedState, cfg: HealthCfg) -> anyhow::Result<()> {
    if !cfg.enabled {
        tracing::info!("health server disabled");
        return Ok(());
    }

    let app = Router::new()
        .route("/liveness", get(liveness))
        .route("/readiness", get(readiness))
        .route("/healthz", get(readiness))
        .route("/metrics", get(metrics))
        .with_state((shared, cfg.expose_metrics));

    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await?;
    tracing::info!(addr = %listener.local_addr()?, "health server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn liveness(State((state, _)): State<(RuntimeSharedState, bool)>) -> Json<LivenessResponse> {
    let guard = state.read();
    Json(LivenessResponse {
        liveness: true,
        build: BuildInfo {
            version: env!("CARGO_PKG_VERSION"),
            git_sha: option_env!("GIT_SHA").unwrap_or("unknown"),
        },
        uptime_sec: guard.uptime_start.elapsed().as_secs(),
    })
}

async fn readiness(
    State((state, _)): State<(RuntimeSharedState, bool)>,
) -> (StatusCode, Json<ReadinessResponse>) {
    let guard = state.read().clone();
    let readiness = guard.readiness;

    let payload = ReadinessResponse {
        readiness,
        runtime_phase: guard.runtime_phase,
        live_guard: guard.live_guard_status,
        live_guard_reasons: guard.live_guard_reasons,
        exit_recovery_active: guard.exit_recovery_active,
        close_only_degraded: guard.close_only_degraded,
        operator_intervention_required: guard.operator_intervention_required,
        open_risk_position_unflattened: guard.open_risk_position_unflattened,
        orders_mode: guard.orders_mode,
        allow_live_orders: guard.allow_live_orders,
        allow_paper_orders: guard.allow_paper_orders,
        gateway: GatewayReadinessResponse {
            health_age_sec: guard.gateway_health_age_sec,
            require_gateway_ready: guard.require_gateway_ready,
            gateway_ready: guard.gateway_ready,
            ws_connected: guard.ws_connected,
            cws_authorized: guard.cws_authorized,
            scheduler_state: guard.gateway_scheduler_state,
        },
        scheduler: SchedulerReadinessResponse {
            state: guard.scheduler_state,
            now_local: guard.now_local,
            note: guard.scheduler_note,
            timezone_offset_hours: guard.timezone_offset_hours,
        },
        streams: StreamsReadinessResponse {
            last_bar_ts_utc: guard.last_bar_ts_utc,
            last_ack_ts_utc: guard.last_ack_ts_utc,
            last_intent_ts_utc: guard.last_intent_ts_utc,
        },
    };

    let status = if readiness {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(payload))
}

async fn metrics(
    State((_, expose_metrics)): State<(RuntimeSharedState, bool)>,
) -> impl IntoResponse {
    if !expose_metrics {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::OK.into_response()
}
