mod common;

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use alor_protocol::{AckStatus, CommandAck, Envelope, MessageType, OrderCommand};
use strategy_runtime::runtime::StrategyRuntime;
use strategy_runtime::strategy_limit_cancel::LimitCancelConfig;
use strategy_runtime::{deterministic_request_id, BarEvent, DataOrigin, RuntimeConfig, StreamNames};

use crate::common::{extract_payload, redis_flushdb, xadd_json, xlen};

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

fn build_config(redis_url: String, prefix: &str, consumer_name: &str) -> RuntimeConfig {
    let portfolio = "demo".to_string();
    let strategy_id = "limit_cancel".to_string();
    let symbol = "SBER".to_string();
    RuntimeConfig {
        redis_url,
        source: "test-runtime".to_string(),
        strategy_id: strategy_id.clone(),
        portfolio: portfolio.clone(),
        exchange: "alor".to_string(),
        streams: StreamNames {
            bars: format!("{prefix}:bars"),
            orders: format!("{prefix}:orders"),
            positions: format!("{prefix}:positions"),
            commands: format!("{prefix}:commands"),
            acks: format!("{prefix}:acks"),
            health: None,
            dlq_prefix: format!("{prefix}:dlq"),
        },
        runtime_state_stream: format!("{prefix}:runtime-state"),
        trim_maxlen_runtime_state: 1_000,
        consumer_group: format!("{prefix}:group"),
        consumer_name: consumer_name.to_string(),
        block_ms: 100,
        claim_idle_ms: 200,
        claim_batch: 10,
        poll_interval_ms: 50,
        trim_maxlen_bars: 1_000,
        trim_maxlen_orders: 1_000,
        trim_maxlen_positions: 1_000,
        trim_maxlen_commands: 1_000,
        trim_maxlen_acks: 1_000,
        trim_maxlen_health: 1_000,
        limit_cancel: LimitCancelConfig {
            symbol,
            tick_size: 0.01,
            offset_ticks: 50,
            qty: 1.0,
            side: alor_protocol::Side::Buy,
            max_wait_bars_for_ack: 3,
        },
        reset_state_on_start: false,
    }
}

async fn spawn_runtime(config: RuntimeConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut runtime = StrategyRuntime::new(config).await.expect("runtime init");
        let _ = runtime.run().await;
    })
}

async fn publish_bar(redis_url: &str, stream: &str, symbol: &str, ts: i64, close: f64) -> Result<()> {
    let bar = BarEvent {
        symbol: symbol.to_string(),
        close_time_utc: ts,
        close,
        o: close,
        h: close,
        l: close,
        v: 0.0,
        origin: DataOrigin::Live,
    };
    let envelope = Envelope::new(Utc::now().timestamp(), "test", MessageType::Bar, bar);
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
    let message_id = match entry.get(0) {
        Some(redis::Value::Data(data)) => String::from_utf8_lossy(data).to_string(),
        _ => return Err(anyhow::anyhow!("missing message id")),
    };
    let payload = entry
        .get(1)
        .and_then(extract_payload)
        .ok_or_else(|| anyhow::anyhow!("missing payload"))?;
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
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or_else(|| anyhow::anyhow!("timeout waiting for command"))?;
        let result = timeout(
            remaining,
            read_next_command(redis_url, stream, &last_id),
        )
        .await??;
        if predicate(&result.1) {
            return Ok(result);
        }
        last_id = result.0;
    }
}

async fn publish_ack(redis_url: &str, stream: &str, ack: CommandAck) -> Result<()> {
    let envelope = Envelope::new(Utc::now().timestamp(), "test", MessageType::CommandAck, ack);
    let json = serde_json::to_string(&envelope)?;
    xadd_json(redis_url, stream, &json).await?;
    Ok(())
}

async fn list_commands(redis_url: &str, stream: &str) -> Result<Vec<OrderCommand>> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XRANGE")
        .arg(stream)
        .arg("-")
        .arg("+")
        .query_async(&mut conn)
        .await?;
    let entries = match reply {
        redis::Value::Bulk(values) => values,
        _ => return Ok(Vec::new()),
    };
    let mut commands = Vec::new();
    for entry in entries {
        if let redis::Value::Bulk(values) = entry {
            if values.len() < 2 {
                continue;
            }
            if let Some(payload) = extract_payload(&values[1]) {
                let envelope: Envelope<OrderCommand> = serde_json::from_str(&payload)?;
                commands.push(envelope.payload);
            }
        }
    }
    Ok(commands)
}

#[tokio::test]
async fn e2e_limit_cancel_happy_path() -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-{}", Uuid::new_v4());
    let config = build_config(redis_url.clone(), &prefix, "runtime-a");

    let runtime_handle = spawn_runtime(config.clone()).await;

    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.limit_cancel.symbol,
        1_000,
        100.0,
    )
    .await?;

    let (place_id, place_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config.streams.commands, "0-0"),
    )
    .await??;

    let expected_request_id = deterministic_request_id(
        &config.strategy_id,
        &config.portfolio,
        &config.limit_cancel.symbol,
        "place",
        1_000,
        0,
    );
    assert_eq!(place_cmd.request_id, expected_request_id);

    publish_ack(
        &redis_url,
        &config.streams.acks,
        CommandAck::success(place_cmd.request_id, Some(123)),
    )
    .await?;

    publish_bar(
        &redis_url,
        &config.streams.bars,
        &config.limit_cancel.symbol,
        2_000,
        101.0,
    )
    .await?;

    let (_cancel_id, cancel_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config.streams.commands, &place_id),
    )
    .await??;

    match cancel_cmd.action {
        alor_protocol::CommandAction::Cancel(cancel) => {
            assert_eq!(cancel.order_id, 123);
        }
        _ => panic!("expected cancel"),
    }

    publish_ack(
        &redis_url,
        &config.streams.acks,
        CommandAck::accepted(cancel_cmd.request_id),
    )
    .await?;

    sleep(Duration::from_millis(200)).await;

    let total_commands = xlen(&redis_url, &config.streams.commands).await?;
    assert_eq!(total_commands, 2);

    runtime_handle.abort();
    Ok(())
}

#[tokio::test]
async fn e2e_restart_without_duplicate_place() -> Result<()> {
    let redis_url = match redis_url() {
        Some(url) => url,
        None => {
            eprintln!("REDIS_URL not set; skipping e2e test");
            return Ok(());
        }
    };
    redis_flushdb(&redis_url).await?;

    let prefix = format!("e2e-restart-{}", Uuid::new_v4());
    let config_a = build_config(redis_url.clone(), &prefix, "runtime-a");
    let config_b = build_config(redis_url.clone(), &prefix, "runtime-b");

    let handle_a = spawn_runtime(config_a.clone()).await;

    publish_bar(
        &redis_url,
        &config_a.streams.bars,
        &config_a.limit_cancel.symbol,
        3_000,
        100.0,
    )
    .await?;

    let (place_id, place_cmd) = timeout(
        Duration::from_secs(5),
        read_next_command(&redis_url, &config_a.streams.commands, "0-0"),
    )
    .await??;
    let place_request_id = place_cmd.request_id;

    handle_a.abort();
    sleep(Duration::from_millis(200)).await;

    let handle_b = spawn_runtime(config_b.clone()).await;
    sleep(Duration::from_millis(500)).await;

    publish_ack(
        &redis_url,
        &config_b.streams.acks,
        CommandAck {
            request_id: place_request_id,
            status: AckStatus::Success,
            broker_order_id: Some(456),
            error_code: None,
            error_msg: None,
            processed_ts_utc: Utc::now().timestamp(),
        },
    )
    .await?;

    publish_bar(
        &redis_url,
        &config_b.streams.bars,
        &config_b.limit_cancel.symbol,
        4_000,
        101.0,
    )
    .await?;

    let (_cancel_id, cancel_cmd) = wait_for_command(
        &redis_url,
        &config_b.streams.commands,
        place_id,
        Duration::from_secs(5),
        |cmd| matches!(cmd.action, alor_protocol::CommandAction::Cancel(_)),
    )
    .await?;

    match cancel_cmd.action {
        alor_protocol::CommandAction::Cancel(cancel) => {
            assert_eq!(cancel.order_id, 456);
        }
        _ => panic!("expected cancel"),
    }

    let commands = list_commands(&redis_url, &config_b.streams.commands).await?;
    let place_requests: Vec<_> = commands
        .iter()
        .filter(|cmd| matches!(cmd.action, alor_protocol::CommandAction::Place(_)))
        .collect();
    assert_eq!(place_requests.len(), 1);
    assert_eq!(place_requests[0].request_id, place_request_id);

    handle_b.abort();
    Ok(())
}
