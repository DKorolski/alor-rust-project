use alor_gateway::config::AlorGatewayConfig;
use alor_gateway::supervisor::Supervisor;
use alor_scalping::strategy::{StrategyConfig, StrategyState};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let cfg = AlorGatewayConfig::from_env()?;
    let supervisor = Supervisor::new(cfg);
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
