mod common;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use chrono::{FixedOffset, TimeZone, Utc};
use serde::Serialize;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

use alor_protocol::{CommandAck, Envelope, MessageType, OrderCommand, Side};
use strategy_runtime::live_guard::{GatewayPhase, HealthEvent};
use strategy_runtime::runtime::StrategyRuntime;
use strategy_runtime::{
    BacktestConfig, BarEvent, DataOrigin, OrderEvent, PaperConfig, PaperOutput, PositionEvent,
    HybridIntradaySettings, ReadConfig, ReplayConfig, RuntimeConfig, StrategyConfig, StreamNames,
    TradeMode, TrimConfig,
};

use crate::common::{extract_payload, redis_flushdb, xadd_json, xlen};

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

const COMMAND_DEADLINE: Duration = Duration::from_secs(60);
const STARTUP_SETTLE: Duration = Duration::from_millis(600);

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
        strategy: StrategyConfig {
            strategy_id,
            strategy_kind: strategy_runtime::StrategyKind::SessionGapStandalone,
            symbol,
            qty: 1.0,
            side: Side::Buy,
            place_offset_ticks: 50,
            tick_size: 0.01,
            max_wait_bars_for_ack: 3,
            close_trigger: strategy_runtime::CloseTrigger::NextBar,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
            session_open_hour: 16,
            session_open_minute: 0,
            session_close_hour: 23,
            session_close_minute: 49,
            entry_after_open_min: 59,
            exit_before_close_min: 20,
            timezone_offset_hours: 3,
            trading_periods: None,
            max_silence_bars_sec: 0,
            session_gap_k_long: 0.5,
            session_gap_k_short: 0.46,
            session_gap_wait_hours: 2,
            session_gap_k_tp_short: 0.28,
            session_gap_k_sl_short: 0.65,
            session_gap_long_ex_pct: 2.2,
            session_gap_short_ex_pct: 2.2,
            session_gap_close_hour: 23,
            session_gap_close_minute: 49,
            session_gap_min: 60.0,
            session_gap_exit_offset_min: 20,
            session_gap_work_weekends: false,
            session_gap_k_tp_long: 0.28,
            session_gap_k_sl_long: 0.68,
            session_gap_start_cash: 30_000.0,
            session_gap_cash_factor: 0.9,
            session_gap_max_entry_hour: 19,
            hybrid_intraday: HybridIntradaySettings::default(),
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
    publish_snapshots_with_positions(redis_url, stream, HashMap::new()).await
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

#[allow(clippy::too_many_arguments)]
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

fn ts_utc_from_msk(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
    let msk = FixedOffset::east_opt(3 * 3600).expect("valid MSK offset");
    msk.with_ymd_and_hms(y, m, d, h, min, 0)
        .single()
        .expect("valid MSK datetime")
        .with_timezone(&Utc)
        .timestamp()
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

async fn read_next_command_deadline(
    redis_url: &str,
    stream: &str,
    last_id: &str,
    deadline: Duration,
) -> Result<(String, OrderCommand)> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let start = Instant::now();

    loop {
        if start.elapsed() > deadline {
            anyhow::bail!("deadline elapsed while waiting for command (stream={stream})");
        }

        let reply: redis::Value = redis::cmd("XREAD")
            .arg("BLOCK")
            .arg(1_000)
            .arg("COUNT")
            .arg(1)
            .arg("STREAMS")
            .arg(stream)
            .arg(last_id)
            .query_async(&mut conn)
            .await?;

        match reply {
            redis::Value::Nil => continue,
            redis::Value::Bulk(values) if !values.is_empty() => {
                let stream_reply = values
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("missing stream reply"))?;
                let items = match stream_reply {
                    redis::Value::Bulk(values) => values,
                    _ => return Err(anyhow::anyhow!("invalid stream reply")),
                };
                let entries = match items.get(1) {
                    Some(redis::Value::Bulk(entries)) if !entries.is_empty() => entries,
                    _ => continue,
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
                let Some(payload) = extract_payload(&entry[1]) else {
                    return Err(anyhow::anyhow!("missing payload"));
                };
                let envelope: Envelope<OrderCommand> = serde_json::from_str(&payload)?;
                return Ok((message_id, envelope.payload));
            }
            _ => continue,
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

async fn clear_key(redis_url: &str, key: &str) -> Result<()> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    redis::cmd("DEL")
        .arg(key)
        .query_async::<_, ()>(&mut conn)
        .await?;
    Ok(())
}

async fn wait_for_consumer_group(redis_url: &str, stream: &str, group: &str) -> Result<()> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;

    for _ in 0..60 {
        let reply = redis::cmd("XINFO")
            .arg("GROUPS")
            .arg(stream)
            .query_async::<_, redis::Value>(&mut conn)
            .await;

        if let Ok(redis::Value::Bulk(groups)) = reply {
            for group_entry in groups {
                let redis::Value::Bulk(fields) = group_entry else {
                    continue;
                };
                for pair in fields.chunks(2) {
                    let [k, v] = pair else { continue };
                    let redis::Value::Data(k) = k else { continue };
                    let redis::Value::Data(v) = v else { continue };
                    if k == b"name" && v == group.as_bytes() {
                        return Ok(());
                    }
                }
            }
        }

        sleep(Duration::from_millis(100)).await;
    }

    anyhow::bail!("consumer group {group} not ready for stream {stream}")
}

async fn wait_xlen_at_least(
    redis_url: &str,
    stream: &str,
    min_len: i64,
    deadline: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        if start.elapsed() > deadline {
            anyhow::bail!("deadline elapsed while waiting xlen({stream}) >= {min_len}");
        }
        let len = xlen(redis_url, stream).await?;
        if len >= min_len {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn publish_entry_bars(redis_url: &str, config: &RuntimeConfig) -> Result<()> {
    let stream = &config.streams.bars;
    let symbol = &config.strategy.symbol;

    publish_bar_ohlc(
        redis_url,
        stream,
        symbol,
        ts_utc_from_msk(2025, 1, 10, 16, 0),
        120.0,
        121.0,
        119.0,
        120.0,
        DataOrigin::Live,
    )
    .await?;
    publish_bar_ohlc(
        redis_url,
        stream,
        symbol,
        ts_utc_from_msk(2025, 1, 10, 17, 0),
        125.0,
        126.0,
        124.0,
        125.0,
        DataOrigin::Live,
    )
    .await?;
    // важный момент: session_gap_min=60min => если сделать дырку >60мин внутри сессии,
    // стратегия посчитает это новой сессией и сигнал не сгенерируется. Поэтому добавляем 18:00.
    publish_bar_ohlc(
        redis_url,
        stream,
        symbol,
        ts_utc_from_msk(2025, 1, 10, 18, 0),
        130.0,
        131.0,
        129.0,
        130.0,
        DataOrigin::Live,
    )
    .await?;

    publish_bar_ohlc(
        redis_url,
        stream,
        symbol,
        ts_utc_from_msk(2025, 1, 10, 18, 59),
        135.0,
        136.0,
        134.0,
        135.0,
        DataOrigin::Live,
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
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-session-gap-restart-{}", Uuid::new_v4());
    let config_a = build_config(redis_url.clone(), &prefix, "runtime-session-gap-a");
    let config_b = build_config(redis_url.clone(), &prefix, "runtime-session-gap-b");

    if let Some(stream) = &config_a.streams.snapshots {
        publish_snapshots(&redis_url, stream).await?;
    }

    let handle_a = spawn_runtime(config_a.clone()).await;

    wait_for_consumer_group(&redis_url, &config_a.streams.bars, &config_a.consumer_group).await?;
    if let Some(health_stream) = &config_a.streams.health {
        // runtime does not create XREADGROUP consumer groups for health stream;
        // health is polled directly in refresh_health_if_due().
        // So we only publish health events and must not wait for a group here.
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    sleep(STARTUP_SETTLE).await;

    if let Some(health_stream) = &config_a.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    publish_entry_bars(&redis_url, &config_a).await?;

    let (buy_id, buy_cmd) = read_next_command_deadline(
        &redis_url,
        &config_a.streams.commands,
        "0-0",
        COMMAND_DEADLINE,
    )
    .await?;

    let buy_qty = match buy_cmd.action {
        alor_protocol::CommandAction::Market(market) => {
            assert_eq!(market.side, Side::Buy);
            market.qty
        }
        other => panic!("expected market buy, got {other:?}"),
    };

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
        buy_qty,
        ts_utc_from_msk(2025, 1, 10, 18, 59) + 1,
    )
    .await?;

    wait_xlen_at_least(
        &redis_url,
        &config_a.streams.runtime_state,
        1,
        Duration::from_secs(3),
    )
    .await?;

    handle_a.abort();

    if snapshot_only {
        clear_key(&redis_url, &config_b.streams.runtime_state).await?;
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
                        qty: buy_qty,
                        existing: true,
                        avg_price: 100.0,
                        ts_utc: ts_utc_from_msk(2025, 1, 10, 18, 59),
                    },
                )]),
            )
            .await?;
        } else {
            publish_snapshots(&redis_url, stream).await?;
        }
    }

    let handle_b = spawn_runtime(config_b.clone()).await;

    wait_for_consumer_group(&redis_url, &config_b.streams.bars, &config_b.consumer_group).await?;
    if let Some(health_stream) = &config_b.streams.health {
        // runtime does not create XREADGROUP consumer groups for health stream;
        // health is polled directly in refresh_health_if_due().
        // So we only publish health events and must not wait for a group here.
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    sleep(STARTUP_SETTLE).await;
    if let Some(health_stream) = &config_b.streams.health {
        publish_health(&redis_url, health_stream, GatewayPhase::LiveReady, true).await?;
    }

    publish_bar_ohlc(
        &redis_url,
        &config_b.streams.bars,
        &config_b.strategy.symbol,
        ts_utc_from_msk(2025, 1, 10, 23, 30),
        134.0,
        134.0,
        133.0,
        133.5,
        DataOrigin::Live,
    )
    .await?;

    let (_close_id, close_cmd) = read_next_command_deadline(
        &redis_url,
        &config_b.streams.commands,
        &buy_id,
        COMMAND_DEADLINE,
    )
    .await?;

    match close_cmd.action {
        alor_protocol::CommandAction::Market(market) => {
            assert_eq!(market.side, Side::Sell);
            assert_eq!(market.qty, buy_qty);
        }
        other => panic!("expected market sell close after restart, got {other:?}"),
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
        ts_utc_from_msk(2025, 1, 10, 23, 30) + 1,
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
