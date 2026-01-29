use std::env;

use anyhow::Result;
use tokio::signal;
use tracing::{info, Level};
use tracing_subscriber::EnvFilter;

use strategy_runtime::runtime::StrategyRuntime;
use strategy_runtime::{RuntimeConfig, StreamNames};

fn env_or_default(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    let redis_url = env_or_default("REDIS_URL", "redis://127.0.0.1/");
    let strategy_id = env_or_default("STRATEGY_ID", "limit_cancel");
    let portfolio = env_or_default("PORTFOLIO", "demo");
    let exchange = env_or_default("EXCHANGE", "alor");
    let symbol = env_or_default("SYMBOL", "SBER");
    let side = env_or_default("SIDE", "buy").to_lowercase();
    let side = match side.as_str() {
        "sell" => alor_protocol::Side::Sell,
        _ => alor_protocol::Side::Buy,
    };
    let offset_ticks: i64 = env_or_default("PLACE_OFFSET_TICKS", "1")
        .parse()
        .unwrap_or(1);
    let qty: f64 = env_or_default("QTY", "1.0").parse().unwrap_or(1.0);
    let tick_size: f64 = env_or_default("TICK_SIZE", "0.01").parse().unwrap_or(0.01);
    let reset_state_on_start = env_or_default("RESET_STATE_ON_START", "0") == "1";

    let config = RuntimeConfig {
        redis_url,
        source: env_or_default("SOURCE", "strategy-runtime"),
        strategy_id: strategy_id.clone(),
        portfolio: portfolio.clone(),
        exchange: exchange.clone(),
        streams: StreamNames {
            bars: env_or_default("STREAM_BARS", &format!("md.bars.{portfolio}.1m")),
            orders: env_or_default("STREAM_ORDERS", &format!("broker.orders.{portfolio}")),
            positions: env_or_default("STREAM_POSITIONS", &format!("broker.positions.{portfolio}")),
            commands: env_or_default("STREAM_COMMANDS", &format!("cmd.orders.{portfolio}")),
            acks: env_or_default("STREAM_ACKS", &format!("cmd.acks.{portfolio}")),
            health: None,
            dlq_prefix: env_or_default("STREAM_DLQ_PREFIX", "dlq"),
        },
        runtime_state_stream: env_or_default(
            "RUNTIME_STATE_STREAM",
            &format!("runtime.state.{strategy_id}.{portfolio}"),
        ),
        trim_maxlen_runtime_state: env_or_default("TRIM_MAXLEN_RUNTIME_STATE", "2000")
            .parse()
            .unwrap_or(2000),
        consumer_group: env_or_default("CONSUMER_GROUP", "strategy-runtime"),
        consumer_name: env_or_default("CONSUMER_NAME", "auto"),
        block_ms: env_or_default("BLOCK_MS", "500").parse().unwrap_or(500),
        claim_idle_ms: env_or_default("CLAIM_IDLE_MS", "5000")
            .parse()
            .unwrap_or(5000),
        claim_batch: env_or_default("CLAIM_BATCH", "50").parse().unwrap_or(50),
        poll_interval_ms: env_or_default("POLL_INTERVAL_MS", "100")
            .parse()
            .unwrap_or(100),
        trim_maxlen_bars: env_or_default("TRIM_MAXLEN_BARS", "200000")
            .parse()
            .unwrap_or(200000),
        trim_maxlen_orders: env_or_default("TRIM_MAXLEN_ORDERS", "100000")
            .parse()
            .unwrap_or(100000),
        trim_maxlen_positions: env_or_default("TRIM_MAXLEN_POSITIONS", "50000")
            .parse()
            .unwrap_or(50000),
        trim_maxlen_commands: env_or_default("TRIM_MAXLEN_COMMANDS", "50000")
            .parse()
            .unwrap_or(50000),
        trim_maxlen_acks: env_or_default("TRIM_MAXLEN_ACKS", "100000")
            .parse()
            .unwrap_or(100000),
        trim_maxlen_health: env_or_default("TRIM_MAXLEN_HEALTH", "10000")
            .parse()
            .unwrap_or(10000),
        limit_cancel: strategy_runtime::strategy_limit_cancel::LimitCancelConfig {
            symbol,
            tick_size,
            offset_ticks,
            qty,
            side,
            max_wait_bars_for_ack: env_or_default("MAX_WAIT_BARS_FOR_ACK", "3")
                .parse()
                .unwrap_or(3),
        },
        reset_state_on_start,
    };

    info!(
        strategy_id = config.strategy_id,
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
