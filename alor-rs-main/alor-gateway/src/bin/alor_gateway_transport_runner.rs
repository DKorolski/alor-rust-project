use std::env;
use std::sync::Arc;

use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};
use uuid::Uuid;

use alor_gateway::config::{AlorGatewayConfig, detect_config_path, log_resolved_config};
use alor_gateway::health_server;
use alor_gateway::services::command_consumer::{CommandConsumerConfig, InMemoryIdempotency};
use alor_gateway::supervisor::{CommandTransport, Supervisor};
use alor_gateway::transport::{StreamNames, StreamTrimLimits, TransportConfig};
use alor_gateway::transport_redis::{RedisCommandSink, RedisCommandSource, RedisEventSink};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let args: Vec<String> = env::args().collect();
    let config_path = detect_config_path(&args);
    let data_report_path =
        detect_data_report_path(&args).or_else(|| env::var("DATA_REPORT_PATH").ok());
    let bar_dump_path = detect_bar_dump_path(&args).or_else(|| env::var("BAR_DUMP_PATH").ok());
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
    cfg.bar_dump_path = bar_dump_path.clone();

    let transport = build_transport_config(&args, &cfg);

    let supervisor = Supervisor::new(cfg.clone());
    let health_state = supervisor.health_state();
    let health_addr = cfg.health_listen_addr.clone();
    tokio::spawn(async move {
        if let Err(error) = health_server::serve(health_state, health_addr).await {
            warn!(?error, "health server stopped");
        }
    });

    info!(
        mode = "transport-only",
        source = transport.source,
        consumer_group = transport.consumer_group,
        consumer_name = transport.consumer_name,
        bars = transport.streams.bars,
        orders = transport.streams.orders,
        positions = transport.streams.positions,
        snapshots = transport.streams.snapshots,
        commands = transport.streams.commands,
        acks = transport.streams.acks,
        health = transport.streams.health,
        dlq_prefix = transport.streams.dlq_prefix,
        trim_bars = transport.trim.bars,
        trim_orders = transport.trim.orders,
        trim_trades = transport.trim.trades,
        trim_positions = transport.trim.positions,
        trim_snapshots = transport.trim.snapshots,
        trim_commands = transport.trim.commands,
        trim_acks = transport.trim.acks,
        trim_health = transport.trim.health,
        "gateway mode: transport-only"
    );

    let event_sink = Arc::new(RedisEventSink::new(transport.clone())?);
    let command_source = RedisCommandSource::new(transport.clone())?;
    let command_sink = Arc::new(RedisCommandSink::new(transport.clone())?);
    let idempotency_config = CommandConsumerConfig::default();
    let idempotency = Arc::new(InMemoryIdempotency::new(
        idempotency_config.idempotency_ttl,
        idempotency_config.idempotency_max,
    ));
    let command_transport = CommandTransport {
        source: Box::new(command_source),
        sink: command_sink,
        idempotency,
        info: alor_gateway::services::command_consumer::CommandConsumerInfo {
            consumer_group: transport.consumer_group.clone(),
            consumer_name: transport.consumer_name.clone(),
            stream: transport.streams.commands.clone(),
            block_ms: transport.block_ms,
        },
    };

    info!("starting alor gateway transport-only runner");
    if let Err(error) = supervisor
        .run_transport_only(Some(event_sink), Some(command_transport))
        .await
    {
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

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == key {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&format!("{key}=")) {
            return Some(value.to_string());
        }
    }
    None
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

fn detect_bar_dump_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--bar-dump" {
            return iter.next().cloned();
        }
        if let Some(path) = arg.strip_prefix("--bar-dump=") {
            return Some(path.to_string());
        }
    }
    None
}

fn build_transport_config(args: &[String], cfg: &AlorGatewayConfig) -> TransportConfig {
    let redis_url = arg_value(args, "--redis-url")
        .or_else(|| env::var("REDIS_URL").ok())
        .unwrap_or_else(|| "redis://127.0.0.1/".to_string());
    let source = arg_value(args, "--transport-source")
        .or_else(|| env::var("TRANSPORT_SOURCE").ok())
        .unwrap_or_else(|| format!("gateway-{}-{}", std::process::id(), Uuid::new_v4()));
    let consumer_group = arg_value(args, "--consumer-group")
        .or_else(|| env::var("CONSUMER_GROUP").ok())
        .unwrap_or_else(|| "gateway-commands".to_string());
    let consumer_name = arg_value(args, "--consumer-name")
        .or_else(|| env::var("CONSUMER_NAME").ok())
        .unwrap_or_else(|| "auto".to_string());
    let trim_maxlen = arg_value(args, "--trim-maxlen")
        .or_else(|| env::var("TRIM_MAXLEN").ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(100_000);
    let stream_trim = |arg: &str, env_key: &str| {
        arg_value(args, arg)
            .or_else(|| env::var(env_key).ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or(trim_maxlen)
    };
    let trim = StreamTrimLimits {
        bars: stream_trim("--trim-maxlen-bars", "TRIM_MAXLEN_BARS"),
        orders: stream_trim("--trim-maxlen-orders", "TRIM_MAXLEN_ORDERS"),
        trades: stream_trim("--trim-maxlen-trades", "TRIM_MAXLEN_TRADES"),
        positions: stream_trim("--trim-maxlen-positions", "TRIM_MAXLEN_POSITIONS"),
        snapshots: stream_trim("--trim-maxlen-snapshots", "TRIM_MAXLEN_SNAPSHOTS"),
        commands: stream_trim("--trim-maxlen-commands", "TRIM_MAXLEN_COMMANDS"),
        acks: stream_trim("--trim-maxlen-acks", "TRIM_MAXLEN_ACKS"),
        health: stream_trim("--trim-maxlen-health", "TRIM_MAXLEN_HEALTH"),
    };
    let block_ms = arg_value(args, "--block-ms")
        .or_else(|| env::var("BLOCK_MS").ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(500);
    let claim_idle_ms = arg_value(args, "--claim-idle-ms")
        .or_else(|| env::var("CLAIM_IDLE_MS").ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(5_000);

    let portfolio = cfg.portfolio.clone();
    let tf_label = if cfg.tf_sec % 60 == 0 {
        format!("{}m", cfg.tf_sec / 60)
    } else {
        format!("{}s", cfg.tf_sec)
    };

    let default_bars = format!("md.bars.{portfolio}.{tf_label}");
    let default_orders = format!("broker.orders.{portfolio}");
    let default_positions = format!("broker.positions.{portfolio}");
    let default_trades = format!("broker.trades.{portfolio}");
    let default_snapshots = format!("broker.snapshots.{portfolio}");
    let default_commands = format!("cmd.orders.{portfolio}");
    let default_acks = format!("cmd.acks.{portfolio}");
    let default_health = "events.health".to_string();
    let default_dlq_prefix = "dlq".to_string();

    let streams = StreamNames {
        bars: arg_value(args, "--stream-bars")
            .or_else(|| env::var("STREAM_BARS").ok())
            .unwrap_or(default_bars),
        orders: arg_value(args, "--stream-orders")
            .or_else(|| env::var("STREAM_ORDERS").ok())
            .unwrap_or(default_orders),
        trades: arg_value(args, "--stream-trades")
            .or_else(|| env::var("STREAM_TRADES").ok())
            .unwrap_or(default_trades),
        positions: arg_value(args, "--stream-positions")
            .or_else(|| env::var("STREAM_POSITIONS").ok())
            .unwrap_or(default_positions),
        snapshots: arg_value(args, "--stream-snapshots")
            .or_else(|| env::var("STREAM_SNAPSHOTS").ok())
            .unwrap_or(default_snapshots),
        commands: arg_value(args, "--stream-commands")
            .or_else(|| env::var("STREAM_COMMANDS").ok())
            .unwrap_or(default_commands),
        acks: arg_value(args, "--stream-acks")
            .or_else(|| env::var("STREAM_ACKS").ok())
            .unwrap_or(default_acks),
        health: arg_value(args, "--stream-health")
            .or_else(|| env::var("STREAM_HEALTH").ok())
            .unwrap_or(default_health),
        dlq_prefix: arg_value(args, "--dlq-prefix")
            .or_else(|| env::var("DLQ_PREFIX").ok())
            .unwrap_or(default_dlq_prefix),
    };

    TransportConfig {
        redis_url,
        source,
        streams,
        consumer_group,
        consumer_name,
        trim_maxlen,
        trim,
        block_ms,
        claim_idle_ms,
    }
}
