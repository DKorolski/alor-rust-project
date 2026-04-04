mod common;

use std::collections::HashMap;
use std::time::Duration;

use alor_types::TradingPeriods;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use tokio::time::timeout;
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
    let strategy_id = "market_buy_and_close".to_string();
    let symbol = "SBER".to_string();
    let mut strategy =
        StrategyConfig::defaults_for_kind(strategy_runtime::StrategyKind::MarketBuyAndClose);
    strategy.strategy_id = strategy_id.clone();
    strategy.symbol = symbol;
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

async fn publish_snapshots_with_positions(
    redis_url: &str,
    stream: &str,
    positions: HashMap<String, PositionEvent>,
) -> Result<()> {
    let orders = OrdersSnapshot {
        orders: HashMap::new(),
    };
    let positions = PositionsSnapshot { positions };
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

async fn read_last_payload(redis_url: &str, stream: &str) -> Result<String> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XREVRANGE")
        .arg(stream)
        .arg("+")
        .arg("-")
        .arg("COUNT")
        .arg(1)
        .query_async(&mut conn)
        .await?;
    let entries = match reply {
        redis::Value::Bulk(values) => values,
        _ => return Err(anyhow::anyhow!("empty xrevrange reply")),
    };
    let entry = entries
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing runtime state entry"))?;
    let entry = match entry {
        redis::Value::Bulk(values) => values,
        _ => return Err(anyhow::anyhow!("invalid runtime state entry")),
    };
    if entry.len() < 2 {
        return Err(anyhow::anyhow!("missing runtime state payload"));
    }
    extract_payload(&entry[1]).ok_or_else(|| anyhow::anyhow!("missing payload"))
}

async fn publish_position(
    redis_url: &str,
    stream: &str,
    symbol: &str,
    qty: f64,
    ts_utc: i64,
) -> Result<()> {
    let position = PositionEvent {
        symbol: symbol.to_string(),
        qty,
        existing: false,
        avg_price: 100.0,
        ts_utc,
    };
    let envelope = Envelope::new(
        Utc::now().timestamp(),
        "test",
        MessageType::Position,
        position,
    );
    let json = serde_json::to_string(&envelope)?;
    xadd_json(redis_url, stream, &json).await?;
    Ok(())
}

async fn publish_ack(redis_url: &str, stream: &str, ack: CommandAck) -> Result<()> {
    let envelope = Envelope::new(Utc::now().timestamp(), "test", MessageType::CommandAck, ack);
    let json = serde_json::to_string(&envelope)?;
    xadd_json(redis_url, stream, &json).await?;
    Ok(())
}

#[tokio::test]
async fn buy_and_close_smoke() -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-buy-close-{}", Uuid::new_v4());
    let config = build_config(redis_url.clone(), &prefix, "runtime-buy-close");

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
        20_000,
        100.0,
        DataOrigin::Live,
    )
    .await?;

    let (buy_id, buy_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config.streams.commands, "0-0"),
    )
    .await??;
    publish_ack(
        &redis_url,
        &config.streams.acks,
        CommandAck::accepted(buy_cmd.request_id),
    )
    .await?;
    publish_position(
        &redis_url,
        &config.streams.positions,
        &config.strategy.symbol,
        1.0,
        20_001,
    )
    .await?;

    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.strategy.symbol,
        21_000,
        101.0,
        DataOrigin::Live,
    )
    .await?;

    let (_close_id, close_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config.streams.commands, &buy_id),
    )
    .await??;
    publish_ack(
        &redis_url,
        &config.streams.acks,
        CommandAck::accepted(close_cmd.request_id),
    )
    .await?;
    publish_position(
        &redis_url,
        &config.streams.positions,
        &config.strategy.symbol,
        0.0,
        21_001,
    )
    .await?;

    let total_commands = xlen(&redis_url, &config.streams.commands).await?;
    assert_eq!(total_commands, 2);

    runtime_handle.abort();
    Ok(())
}

async fn run_restart_mid_cycle_case(with_updated_snapshot: bool) -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-buy-close-restart-{}", Uuid::new_v4());
    let config_a = build_config(redis_url.clone(), &prefix, "runtime-buy-close-a");
    let config_b = build_config(redis_url.clone(), &prefix, "runtime-buy-close-b");

    if let Some(stream) = &config_a.streams.snapshots {
        publish_snapshots(&redis_url, stream).await?;
    }
    if let Some(health_stream) = &config_a.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    let handle_a = spawn_runtime(config_a.clone()).await;

    publish_bar(
        &redis_url,
        &config_a.streams.bars,
        &config_a.strategy.symbol,
        30_000,
        100.0,
        DataOrigin::Live,
    )
    .await?;

    let (buy_id, buy_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config_a.streams.commands, "0-0"),
    )
    .await??;

    publish_ack(
        &redis_url,
        &config_a.streams.acks,
        CommandAck::accepted(buy_cmd.request_id),
    )
    .await?;
    publish_position(
        &redis_url,
        &config_a.streams.positions,
        &config_a.strategy.symbol,
        1.0,
        30_001,
    )
    .await?;

    handle_a.abort();

    if let Some(stream) = &config_b.streams.snapshots {
        if with_updated_snapshot {
            publish_snapshots_with_positions(
                &redis_url,
                stream,
                HashMap::from([(
                    config_b.strategy.symbol.clone(),
                    PositionEvent {
                        symbol: config_b.strategy.symbol.clone(),
                        qty: 1.0,
                        existing: true,
                        avg_price: 100.0,
                        ts_utc: 30_010,
                    },
                )]),
            )
            .await?;
        } else {
            publish_snapshots(&redis_url, stream).await?;
        }
    }
    if let Some(health_stream) = &config_b.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    let handle_b = spawn_runtime(config_b.clone()).await;

    publish_bar(
        &redis_url,
        &config_b.streams.bars,
        &config_b.strategy.symbol,
        31_000,
        101.0,
        DataOrigin::Live,
    )
    .await?;

    let (_close_id, close_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config_b.streams.commands, &buy_id),
    )
    .await??;
    match close_cmd.action {
        alor_protocol::CommandAction::Market(market) => {
            assert_eq!(market.side, alor_protocol::Side::Sell);
            assert_eq!(market.qty, 1.0);
        }
        other => panic!("expected market close after restart, got {other:?}"),
    }

    publish_ack(
        &redis_url,
        &config_b.streams.acks,
        CommandAck::accepted(close_cmd.request_id),
    )
    .await?;
    publish_position(
        &redis_url,
        &config_b.streams.positions,
        &config_b.strategy.symbol,
        0.0,
        31_001,
    )
    .await?;

    let total_commands = xlen(&redis_url, &config_b.streams.commands).await?;
    assert_eq!(
        total_commands, 2,
        "must not send duplicate entry after restart"
    );

    handle_b.abort();
    Ok(())
}

#[tokio::test]
async fn restart_mid_cycle_uses_runtime_state_without_snapshot_update() -> Result<()> {
    run_restart_mid_cycle_case(false).await
}

#[tokio::test]
async fn restart_mid_cycle_works_with_updated_snapshot() -> Result<()> {
    run_restart_mid_cycle_case(true).await
}

#[tokio::test]
async fn marketable_limit_dropped_entry_restores_previous_state() -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-buy-close-guard-{}", Uuid::new_v4());
    let mut config = build_config(redis_url.clone(), &prefix, "runtime-buy-close-guard");
    config
        .strategy
        .market_buy_and_close_mut()
        .expect("market buy and close settings")
        .live_order_style = strategy_runtime::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
    config.strategy.trading_periods = Some(TradingPeriods {
        session_start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        session_end: chrono::NaiveTime::from_hms_opt(9, 5, 0).unwrap(),
        break_start_1: chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        break_end_1: chrono::NaiveTime::from_hms_opt(12, 5, 0).unwrap(),
        break_start_2: chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        break_end_2: chrono::NaiveTime::from_hms_opt(15, 5, 0).unwrap(),
        weekends_off: true,
        timezone_offset_hours: 0,
    });

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
        20_000,
        100.0,
        DataOrigin::Live,
    )
    .await?;
    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.strategy.symbol,
        21_000,
        101.0,
        DataOrigin::Live,
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    let total_commands = xlen(&redis_url, &config.streams.commands).await?;
    assert_eq!(
        total_commands, 0,
        "entry must not be emitted outside trading window"
    );

    let last_state_payload = read_last_payload(&redis_url, &config.streams.runtime_state).await?;
    let last_state: serde_json::Value = serde_json::from_str(&last_state_payload)?;
    assert_eq!(
        last_state
            .get("strategy_state")
            .and_then(|state| state.as_str()),
        Some("Idle"),
        "runtime must restore pre-intent state when entry is dropped before publish"
    );

    runtime_handle.abort();
    Ok(())
}
