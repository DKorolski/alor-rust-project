mod common;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use alor_protocol::{CommandAck, Envelope, MessageType, OrderCommand};
use strategy_runtime::live_guard::{GatewayPhase, HealthEvent};
use strategy_runtime::runtime::StrategyRuntime;
use strategy_runtime::{
    BacktestConfig, BarEvent, DataOrigin, OrderEvent, PaperConfig, PaperExecutionMode,
    PaperOutput, PositionEvent, ReadConfig, ReplayConfig, RuntimeConfig, StrategyConfig,
    StreamNames, TradeMode, TrimConfig,
};

use crate::common::{extract_payload, redis_flushdb, xadd_json, xlen};

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

fn build_config(redis_url: String, prefix: &str, consumer_name: &str) -> RuntimeConfig {
    let portfolio = "demo".to_string();
    let strategy_id = "limit_cancel".to_string();
    let symbol = "SBER".to_string();
    let mut strategy = StrategyConfig::defaults_for_kind(strategy_runtime::StrategyKind::LimitCancel);
    strategy.strategy_id = strategy_id.clone();
    strategy.symbol = symbol;
    if let Some(settings) = strategy.limit_cancel_mut() {
        settings.place_offset_ticks = 50;
    }
    RuntimeConfig {
        redis_url,
        source: "test-runtime".to_string(),
        portfolio: portfolio.clone(),
        exchange: "alor".to_string(),
        streams: StreamNames {
            bars: format!("{prefix}:bars"),
            orders: format!("{prefix}:orders"),
            trades: format!("{prefix}:trades"),
            positions: format!("{prefix}:positions"),
            commands: format!("{prefix}:commands"),
            acks: format!("{prefix}:acks"),
            snapshots: Some(format!("{prefix}:snapshots")),
            health: Some(format!("{prefix}:health")),
            dlq_prefix: format!("{prefix}:dlq"),
            runtime_state: format!("{prefix}:runtime-state"),
        },
        consumer_group: format!("{prefix}:group"),
        consumer_name: consumer_name.to_string(),
        trade_mode: TradeMode::Live,
        allow_live_orders: true,
        allow_paper_orders: true,
        guard_log_interval_ms: 500,
        still_blocked_log_period_sec: 60,
        gateway_health_stale_sec: 20,
        require_gateway_ready: true,
        bootstrap_dump: false,
        health: strategy_runtime::HealthServerConfig {
            enabled: false,
            listen_addr: "127.0.0.1:0".to_string(),
            expose_metrics: false,
        },
        read: ReadConfig {
            block_ms: 100,
            claim_idle_ms: 200,
            claim_batch: 10,
            poll_interval_ms: 50,
        },
        trim: TrimConfig {
            bars: 1_000,
            orders: 1_000,
            trades: 1_000,
            positions: 1_000,
            commands: 1_000,
            acks: 1_000,
            health: 1_000,
            runtime_state: 1_000,
        },
        strategy,
        paper: PaperConfig {
            enabled: false,
            output: PaperOutput::Stdout,
            execution_mode: PaperExecutionMode::LiveOnly,
            file_path: format!("{prefix}-paper.jsonl"),
            trades_csv: format!("{prefix}-trades.csv"),
            summary_json: format!("{prefix}-summary.json"),
            append: false,
        },
        backtest: BacktestConfig {
            enabled: false,
            trade_log: format!("{prefix}-backtest.log"),
            trades_csv: format!("{prefix}-trades.csv"),
            summary_json: format!("{prefix}-summary.json"),
            append: false,
        },
        replay: ReplayConfig {
            enabled: false,
            bars_csv_path: None,
            reference_trades_csv_path: None,
            output_dir: "replay_out".to_string(),
            price_tolerance: 1e-8,
            strict_dedup: true,
        },
        reset_state_on_start: false,
    }
}

#[derive(Debug, Serialize)]
struct OrdersSnapshot {
    orders: HashMap<i64, OrderEvent>,
}

#[derive(Debug, Serialize)]
struct PositionsSnapshot {
    positions: HashMap<String, PositionEvent>,
}

async fn spawn_runtime(config: RuntimeConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut runtime = StrategyRuntime::new(config).await.expect("runtime init");
        let _ = runtime.run().await;
    })
}

async fn publish_snapshots(redis_url: &str, stream: &str) -> Result<()> {
    let orders = OrdersSnapshot {
        orders: HashMap::new(),
    };
    let positions = PositionsSnapshot {
        positions: HashMap::new(),
    };
    let orders_envelope = Envelope::new(
        Utc::now().timestamp(),
        "test",
        MessageType::SnapshotOrders,
        orders,
    );
    let positions_envelope = Envelope::new(
        Utc::now().timestamp(),
        "test",
        MessageType::SnapshotPositions,
        positions,
    );
    let orders_json = serde_json::to_string(&orders_envelope)?;
    let positions_json = serde_json::to_string(&positions_envelope)?;
    xadd_json(redis_url, stream, &orders_json).await?;
    xadd_json(redis_url, stream, &positions_json).await?;
    Ok(())
}

async fn publish_bar(
    redis_url: &str,
    stream: &str,
    symbol: &str,
    ts: i64,
    close: f64,
    origin: DataOrigin,
) -> Result<()> {
    let bar = BarEvent {
        symbol: symbol.to_string(),
        close_time_utc: ts,
        close,
        o: close,
        h: close,
        l: close,
        v: 0.0,
        origin,
    };
    let envelope = Envelope::new(Utc::now().timestamp(), "test", MessageType::Bar, bar);
    let json = serde_json::to_string(&envelope)?;
    xadd_json(redis_url, stream, &json).await?;
    Ok(())
}

async fn publish_health(
    redis_url: &str,
    stream: &str,
    phase: GatewayPhase,
    readiness: bool,
) -> Result<()> {
    let health = HealthEvent {
        gateway_phase: phase,
        readiness,
        ws_connected: true,
        cws_authorized: true,
        scheduler_state: Some("Open".to_string()),
        last_event_ts: Utc::now().timestamp(),
    };
    let envelope = Envelope::new(Utc::now().timestamp(), "test", MessageType::Health, health);
    let json = serde_json::to_string(&envelope)?;
    xadd_json(redis_url, stream, &json).await?;
    Ok(())
}

async fn read_next_command(
    redis_url: &str,
    stream: &str,
    last_id: &str,
) -> Result<(String, OrderCommand)> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XREAD")
        .arg("BLOCK")
        .arg(2_000)
        .arg("COUNT")
        .arg(1)
        .arg("STREAMS")
        .arg(stream)
        .arg(last_id)
        .query_async(&mut conn)
        .await?;
    let streams = match reply {
        redis::Value::Bulk(values) => values,
        _ => return Err(anyhow::anyhow!("empty xread reply")),
    };
    let stream_reply = streams
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing stream reply"))?;
    let items = match stream_reply {
        redis::Value::Bulk(values) => values,
        _ => return Err(anyhow::anyhow!("invalid stream reply")),
    };
    let entries = match items.get(1) {
        Some(redis::Value::Bulk(entries)) => entries,
        _ => return Err(anyhow::anyhow!("missing entries")),
    };
    let entry = entries
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing entry"))?;
    let entry = match entry {
        redis::Value::Bulk(values) => values,
        _ => return Err(anyhow::anyhow!("invalid entry")),
    };
    if entry.len() < 2 {
        return Err(anyhow::anyhow!("missing entry values"));
    }
    let message_id = match &entry[0] {
        redis::Value::Data(data) => String::from_utf8_lossy(data).to_string(),
        _ => return Err(anyhow::anyhow!("invalid entry id")),
    };
    let payload = extract_payload(&entry[1]).ok_or_else(|| anyhow::anyhow!("missing payload"))?;
    let envelope: Envelope<OrderCommand> = serde_json::from_str(&payload)?;
    Ok((message_id, envelope.payload))
}

async fn publish_ack(redis_url: &str, stream: &str, ack: CommandAck) -> Result<()> {
    let envelope = Envelope::new(Utc::now().timestamp(), "test", MessageType::CommandAck, ack);
    let json = serde_json::to_string(&envelope)?;
    xadd_json(redis_url, stream, &json).await?;
    Ok(())
}

#[tokio::test]
async fn reconnect_blocks_intents() -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-reconnect-{}", Uuid::new_v4());
    let config = build_config(redis_url.clone(), &prefix, "runtime-reconnect");

    if let Some(stream) = &config.streams.snapshots {
        publish_snapshots(&redis_url, stream).await?;
    }
    if let Some(health_stream) = &config.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    let runtime_handle = spawn_runtime(config.clone()).await;
    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.strategy.symbol,
        10_000,
        100.0,
        DataOrigin::Live,
    )
    .await?;

    let (place_id, place_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config.streams.commands, "0-0"),
    )
    .await??;
    publish_ack(
        &redis_url,
        &config.streams.acks,
        CommandAck::confirmed(place_cmd.request_id, Some(123)),
    )
    .await?;

    if let Some(health_stream) = &config.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::Reconnecting, false).await?;
    }
    sleep(Duration::from_secs(3)).await;

    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.strategy.symbol,
        11_000,
        101.0,
        DataOrigin::Live,
    )
    .await?;
    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.strategy.symbol,
        12_000,
        102.0,
        DataOrigin::Live,
    )
    .await?;

    sleep(Duration::from_millis(500)).await;
    let total_commands = xlen(&redis_url, &config.streams.commands).await?;
    assert_eq!(total_commands, 1);

    if let Some(health_stream) = &config.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }
    sleep(Duration::from_secs(3)).await;
    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.strategy.symbol,
        13_000,
        103.0,
        DataOrigin::Live,
    )
    .await?;

    let _ = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config.streams.commands, &place_id),
    )
    .await??;

    runtime_handle.abort();
    Ok(())
}
