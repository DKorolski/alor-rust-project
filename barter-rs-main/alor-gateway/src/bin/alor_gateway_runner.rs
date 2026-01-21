use std::env;

use alor_gateway::config::{
    AlorGatewayConfig, detect_config_path, log_resolved_config,
};
use alor_gateway::health_server;
use alor_gateway::supervisor::Supervisor;
use alor_scalping::strategy::{StrategyConfig, StrategyState};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let args: Vec<String> = env::args().collect();
    let config_path = detect_config_path(&args);
    let data_report_path = detect_data_report_path(&args)
        .or_else(|| env::var("DATA_REPORT_PATH").ok());
    let resolved = if let Some(path) = config_path.clone() {
        AlorGatewayConfig::from_file_with_sources(path)?
    } else {
        AlorGatewayConfig::from_env_with_sources()?
    };
    log_resolved_config(&resolved, config_path.as_deref());
    let mut cfg = resolved.config.clone();
    cfg.from_ts = resolved.derived.computed_from_ts;
    cfg.skip_history_bars = resolved.derived.skip_history_effective;
    cfg.data_report_path = data_report_path.clone();
    let supervisor = Supervisor::new(cfg.clone());
    let health_state = supervisor.health_state();
    let health_addr = cfg.health_listen_addr.clone();
    tokio::spawn(async move {
        if let Err(error) = health_server::serve(health_state, health_addr).await {
            warn!(?error, "health server stopped");
        }
    });
    let strategy = StrategyState::new(StrategyConfig::default(), 30_000.0);

    info!("starting alor gateway runner");
    if let Err(error) = supervisor.run(strategy).await {
        warn!(?error, "gateway stopped with error");
    }

    Ok(())
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_ansi(cfg!(debug_assertions))
        .init();
}

fn detect_data_report_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--data-report" {
            return iter.next().cloned();
        }
        if let Some(path) = arg.strip_prefix("--data-report=") {
            return Some(path.to_string());
        }
    }
    None
}
