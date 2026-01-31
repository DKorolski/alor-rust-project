use std::env;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use tokio::signal;
use tracing::{info, Level};
use tracing_subscriber::EnvFilter;

use strategy_runtime::config::{load_runtime_config, ResolvedRuntimeConfig};
use strategy_runtime::runtime::StrategyRuntime;

const DEFAULT_CONFIG_PATH: &str = "strategy-runtime.toml";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    let (config_path, allow_missing) = parse_config_path()?;
    let ResolvedRuntimeConfig {
        config,
        sources,
        path,
        file_loaded,
    } = load_runtime_config(config_path, allow_missing)?;

    info!(
        config_path = %path.display(),
        config_file_loaded = file_loaded,
        redis_url = config.redis_url,
        redis_url_source = %sources.redis_url,
        portfolio = config.portfolio,
        portfolio_source = %sources.portfolio,
        exchange = config.exchange,
        exchange_source = %sources.exchange,
        source = config.source,
        source_source = %sources.source,
        streams = ?config.streams,
        streams_sources = ?sources.streams,
        consumer_group = config.consumer_group,
        consumer_group_source = %sources.consumer_group,
        consumer_name = config.consumer_name,
        consumer_name_source = %sources.consumer_name,
        strategy = ?config.strategy,
        strategy_sources = ?sources.strategy,
        health_stream = ?config.streams.health,
        "resolved runtime config"
    );

    info!(
        strategy_id = config.strategy.strategy_id,
        portfolio = config.portfolio,
        exchange = config.exchange,
        "starting strategy runtime"
    );

    let mut runtime = StrategyRuntime::new(config).await?;
    tokio::select! {
        result = runtime.run() => {
            if let Err(error) = result {
                return Err(error);
            }
        }
        _ = signal::ctrl_c() => {
            info!("shutdown requested");
        }
    }

    Ok(())
}

fn parse_config_path() -> Result<(PathBuf, bool)> {
    let mut args = env::args().skip(1);
    let mut config_path = None;
    while let Some(arg) = args.next() {
        if arg == "--config" {
            let value = args
                .next()
                .ok_or_else(|| anyhow!("--config requires a path"))?;
            config_path = Some(PathBuf::from(value));
        }
    }

    let allow_missing = config_path.is_none();
    let path = config_path.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));

    Ok((path, allow_missing))
}
