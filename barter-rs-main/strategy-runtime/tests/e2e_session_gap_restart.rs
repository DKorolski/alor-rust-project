mod common;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use chrono::{FixedOffset, TimeZone, Utc};
use serde::Serialize;
use uuid::Uuid;

use alor_protocol::{CommandAck, Envelope, MessageType, OrderCommand};
use strategy_runtime::live_guard::{GatewayPhase, HealthEvent};
use strategy_runtime::runtime::StrategyRuntime;
use strategy_runtime::{
    BacktestConfig, BarEvent, DataOrigin, OrderEvent, PaperConfig, PaperOutput, PositionEvent,
    ReadConfig, ReplayConfig, RuntimeConfig, StrategyConfig, StreamNames, TradeMode, TrimConfig,
};

use crate::common::{extract_payload, xadd_json, xlen};

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

fn ts_utc_from_msk(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    let msk = FixedOffset::east_opt(3 * 3600).expect("valid msk");
    msk.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("valid datetime")
        .with_timezone(&Utc)
        .timestamp()
}

fn build_config(redis_url: String, prefix: &str, consumer_name: &str) -> RuntimeConfig {
    let portfolio = "demo".to_string();
    let strategy_id = "session_gap_standalone".to_string();
    let symbol = "IMOEXF".to_string();
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
        guard_log_interval_ms: 500,
        bootstrap_dump: false,
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
        strategy: StrategyConfig {
            strategy_id,
            strategy_kind: strategy_runtime::StrategyKind::SessionGapStandalone,
            symbol,
            qty: 1.0,
            side: alor_protocol::Side::Buy,
            place_offset_ticks: 50,
            tick_size: 0.01,
            max_wait_bars_for_ack: 3,
            close_trigger: strategy_runtime::CloseTrigger::NextBar,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
            session_open_hour: 10,
            session_open_minute: 0,
            session_close_hour: 23,
            session_close_minute: 49,
            entry_after_open_min: 59,
            exit_before_close_min: 20,
            timezone_offset_hours: 3,
        },
        paper: PaperConfig {
            enabled: false,
            output: PaperOutput::Stdout,
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

async fn publish_bar_ohlc(
    redis_url: &str,
    stream: &str,
    symbol: &str,
    ts_utc: i64,
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    origin: DataOrigin,
) -> Result<()> {
    let bar = BarEvent {
        symbol: symbol.to_string(),
        close_time_utc: ts_utc,
        close: c,
        o,
        h,
        l,
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
        cws_authorized: true,
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

async fn wait_for_command<F>(
    redis_url: &str,
    stream: &str,
    mut last_id: String,
    timeout_duration: Duration,
    predicate: F,
) -> Result<(String, OrderCommand)>
where
    F: Fn(&OrderCommand) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout_duration;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow::anyhow!("timeout waiting for command"));
        }
        match read_next_command(redis_url, stream, &last_id).await {
            Ok((next_id, cmd)) => {
                if predicate(&cmd) {
                    return Ok((next_id, cmd));
                }
                last_id = next_id;
            }
            Err(error) if error.to_string().contains("empty xread reply") => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error),
        }
    }
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

async fn clear_runtime_state_stream(redis_url: &str, runtime_state_stream: &str) -> Result<()> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let _: i64 = redis::cmd("DEL")
        .arg(runtime_state_stream)
        .query_async(&mut conn)
        .await?;
    Ok(())
}

async fn publish_signal_generating_bars(
    config: &RuntimeConfig,
    s2_year: i32,
    s2_month: u32,
    s2_day: u32,
) -> Result<()> {
    let s = &config.streams.bars;
    let symbol = &config.strategy.symbol;
    let redis_url = &config.redis_url;

    async fn push_bar_with_health(
        config: &RuntimeConfig,
        ts: i64,
        o: f64,
        h: f64,
        l: f64,
        c: f64,
    ) -> Result<()> {
        if let Some(health_stream) = &config.streams.health {
            publish_health(
                &config.redis_url,
                health_stream,
                GatewayPhase::LiveReady,
                true,
            )
            .await?;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        publish_bar_ohlc(
            &config.redis_url,
            &config.streams.bars,
            &config.strategy.symbol,
            ts,
            o,
            h,
            l,
            c,
            DataOrigin::Live,
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(220)).await;
        Ok(())
    }

    let (s1_year, s1_month, s1_day) = if s2_day > 1 {
        (s2_year, s2_month, s2_day - 1)
    } else {
        (s2_year, s2_month, s2_day)
    };
    let (s0_year, s0_month, s0_day) = if s1_day > 1 {
        (s1_year, s1_month, s1_day - 1)
    } else {
        (s1_year, s1_month, s1_day)
    };

    let _ = (redis_url, s, symbol);

    push_bar_with_health(
        config,
        ts_utc_from_msk(s0_year, s0_month, s0_day, 23, 49),
        100.0,
        101.0,
        99.0,
        100.0,
    )
    .await?;
    push_bar_with_health(
        config,
        ts_utc_from_msk(s1_year, s1_month, s1_day, 23, 49),
        110.0,
        120.0,
        80.0,
        110.0,
    )
    .await?;
    push_bar_with_health(
        config,
        ts_utc_from_msk(s2_year, s2_month, s2_day, 16, 0),
        120.0,
        121.0,
        119.0,
        120.0,
    )
    .await?;
    push_bar_with_health(
        config,
        ts_utc_from_msk(s2_year, s2_month, s2_day, 17, 0),
        125.0,
        126.0,
        124.0,
        125.0,
    )
    .await?;
    push_bar_with_health(
        config,
        ts_utc_from_msk(s2_year, s2_month, s2_day, 18, 59),
        135.0,
        136.0,
        134.0,
        135.0,
    )
    .await?;

    Ok(())
}

async fn run_restart_mid_cycle_case(
    with_updated_snapshot: bool,
    snapshot_only: bool,
) -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    let prefix = format!("e2e-session-gap-restart-{}", Uuid::new_v4());
    let config_a = build_config(redis_url.clone(), &prefix, "runtime-session-gap-a");
    let config_b = build_config(redis_url.clone(), &prefix, "runtime-session-gap-b");

    if let Some(stream) = &config_a.streams.snapshots {
        publish_snapshots(&redis_url, stream).await?;
    }
    if let Some(health_stream) = &config_a.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let handle_a = spawn_runtime(config_a.clone()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    if let Some(health_stream) = &config_a.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }
    publish_bar_ohlc(
        &redis_url,
        &config_a.streams.bars,
        &config_a.strategy.symbol,
        ts_utc_from_msk(2025, 12, 1, 0, 0),
        100.0,
        100.0,
        100.0,
        100.0,
        DataOrigin::Live,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut buy_result = None;
    for (year, month, day) in [(2025, 12, 5), (2025, 12, 10), (2025, 12, 15)] {
        publish_signal_generating_bars(&config_a, year, month, day).await?;
        match wait_for_command(
            &redis_url,
            &config_a.streams.commands,
            "0-0".to_string(),
            Duration::from_secs(25),
            |cmd| {
                matches!(
                    cmd.action,
                    alor_protocol::CommandAction::Market(alor_protocol::MarketOrder {
                        side: alor_protocol::Side::Buy,
                        ..
                    })
                )
            },
        )
        .await
        {
            Ok(found) => {
                buy_result = Some(found);
                break;
            }
            Err(error) if error.to_string().contains("timeout waiting for command") => {
                if let Some(health_stream) = &config_a.streams.health {
                    publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
                }
            }
            Err(error) => return Err(error),
        }
    }
    let (buy_id, buy_cmd) = buy_result.ok_or_else(|| anyhow::anyhow!("entry command not emitted"))?;

    match buy_cmd.action {
        alor_protocol::CommandAction::Market(market) => {
            assert_eq!(market.side, alor_protocol::Side::Buy);
            assert!(market.qty > 0.0);
        }
        other => panic!("expected market entry, got {other:?}"),
    }

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
        ts_utc_from_msk(2025, 12, 5, 19, 0),
    )
    .await?;

    handle_a.abort();

    if snapshot_only {
        clear_runtime_state_stream(&redis_url, &config_b.streams.runtime_state).await?;
    }

    if let Some(stream) = &config_b.streams.snapshots {
        if with_updated_snapshot || snapshot_only {
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
                        ts_utc: ts_utc_from_msk(2025, 12, 5, 19, 0),
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
    tokio::time::sleep(Duration::from_millis(300)).await;
    if let Some(health_stream) = &config_b.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    publish_bar_ohlc(
        &redis_url,
        &config_b.streams.bars,
        &config_b.strategy.symbol,
        ts_utc_from_msk(2025, 12, 5, 23, 30),
        131.0,
        132.0,
        130.0,
        131.0,
        DataOrigin::Live,
    )
    .await?;
    publish_bar_ohlc(
        &redis_url,
        &config_b.streams.bars,
        &config_b.strategy.symbol,
        ts_utc_from_msk(2025, 12, 5, 23, 31),
        131.5,
        132.5,
        130.5,
        131.5,
        DataOrigin::Live,
    )
    .await?;

    let (_close_id, close_cmd) = wait_for_command(
        &redis_url,
        &config_b.streams.commands,
        buy_id,
        Duration::from_secs(15),
        |cmd| {
            matches!(
                cmd.action,
                alor_protocol::CommandAction::Market(alor_protocol::MarketOrder {
                    side: alor_protocol::Side::Sell,
                    ..
                })
            )
        },
    )
    .await?;
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
        ts_utc_from_msk(2025, 12, 5, 23, 31),
    )
    .await?;

    let total_commands = xlen(&redis_url, &config_b.streams.commands).await?;
    assert_eq!(total_commands, 2, "must not send duplicate entry after restart");

    handle_b.abort();
    Ok(())
}

#[tokio::test]
async fn restart_mid_cycle_uses_runtime_state_without_snapshot_update() -> Result<()> {
    run_restart_mid_cycle_case(false, false).await
}

#[tokio::test]
async fn restart_mid_cycle_works_with_updated_snapshot() -> Result<()> {
    run_restart_mid_cycle_case(true, false).await
}

#[tokio::test]
async fn restart_mid_cycle_snapshot_only_works() -> Result<()> {
    run_restart_mid_cycle_case(false, true).await
}
