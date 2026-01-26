use std::time::Duration;

use alor_protocol::{CommandAck, CommandAction, Envelope, MessageType, OrderCommand, PlaceOrder, SCHEMA_VERSION};
use alor_gateway::models::{BarEvent, DataOrigin};
use alor_gateway::transport::{CommandSink, CommandSource, EventSink, StreamNames, TransportConfig};
use alor_gateway::transport_redis::{RedisCommandSink, RedisCommandSource, RedisEventSink};
use redis::AsyncCommands;
use testcontainers::{clients::Cli, core::WaitFor, GenericImage};
use tokio::time::sleep;

fn redis_image() -> GenericImage {
    GenericImage::new("redis", "7-alpine")
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
}

fn base_config(redis_url: String, prefix: &str, consumer_name: &str) -> TransportConfig {
    TransportConfig {
        redis_url,
        source: "test".to_string(),
        streams: StreamNames {
            bars: format!("{prefix}:bars"),
            orders: format!("{prefix}:orders"),
            positions: format!("{prefix}:positions"),
            snapshots: format!("{prefix}:snapshots"),
            commands: format!("{prefix}:commands"),
            acks: format!("{prefix}:acks"),
            health: format!("{prefix}:health"),
            dlq_prefix: format!("{prefix}:dlq"),
        },
        consumer_group: format!("{prefix}-group"),
        consumer_name: consumer_name.to_string(),
        trim_maxlen: 5,
        block_ms: 10,
        claim_idle_ms: 5,
    }
}

async fn pending_count(config: &TransportConfig) -> anyhow::Result<i64> {
    let client = redis::Client::open(config.redis_url.clone())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XPENDING")
        .arg(&config.streams.commands)
        .arg(&config.consumer_group)
        .query_async(&mut conn)
        .await?;
    match reply {
        redis::Value::Bulk(values) => {
            if let Some(redis::Value::Int(count)) = values.first() {
                Ok(*count)
            } else {
                Ok(0)
            }
        }
        _ => Ok(0),
    }
}

#[tokio::test]
async fn command_path_acknowledges_and_publishes_ack() {
    let docker = Cli::default();
    let node = docker.run(redis_image());
    let port = node.get_host_port_ipv4(6379);
    let redis_url = format!("redis://127.0.0.1:{port}");
    let prefix = "command-path";
    let config = base_config(redis_url, prefix, "consumer-1");

    let sink = RedisCommandSink::new(config.clone()).unwrap();
    let mut source = RedisCommandSource::new(config.clone()).unwrap();

    let command = OrderCommand::new(
        "strategy",
        "portfolio",
        "exchange",
        "symbol",
        CommandAction::Place(PlaceOrder {
            price: 1.0,
            qty: 1.0,
            side: alor_protocol::Side::Buy,
        }),
    );
    sink.publish_command(command.clone()).await.unwrap();

    let envelope = source.next_command().await.expect("command missing");
    assert_eq!(envelope.command.request_id, command.request_id);
    source.ack(envelope.message_id.as_ref().unwrap()).await.unwrap();
    sink.publish_ack(CommandAck::success(command.request_id, None))
        .await
        .unwrap();

    let client = redis::Client::open(config.redis_url.clone()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let ack_len: i64 = conn.xlen(&config.streams.acks).await.unwrap();
    assert!(ack_len >= 1);
    assert_eq!(pending_count(&config).await.unwrap(), 0);
}

#[tokio::test]
async fn pending_recovery_claims_idle_message() {
    let docker = Cli::default();
    let node = docker.run(redis_image());
    let port = node.get_host_port_ipv4(6379);
    let redis_url = format!("redis://127.0.0.1:{port}");
    let prefix = "pending-recovery";
    let config_a = base_config(redis_url.clone(), prefix, "consumer-a");
    let config_b = base_config(redis_url, prefix, "consumer-b");

    let sink = RedisCommandSink::new(config_a.clone()).unwrap();
    let mut source_a = RedisCommandSource::new(config_a.clone()).unwrap();
    let mut source_b = RedisCommandSource::new(config_b.clone()).unwrap();

    let command = OrderCommand::new(
        "strategy",
        "portfolio",
        "exchange",
        "symbol",
        CommandAction::Place(PlaceOrder {
            price: 1.0,
            qty: 1.0,
            side: alor_protocol::Side::Buy,
        }),
    );
    sink.publish_command(command.clone()).await.unwrap();

    let envelope = source_a.next_command().await.expect("command missing");
    assert_eq!(envelope.command.request_id, command.request_id);

    sleep(Duration::from_millis(10)).await;
    let reclaimed = source_b
        .next_command()
        .await
        .expect("reclaimed command missing");
    assert_eq!(reclaimed.command.request_id, command.request_id);
    source_b
        .ack(reclaimed.message_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(pending_count(&config_a).await.unwrap(), 0);
}

#[tokio::test]
async fn poison_messages_go_to_dlq() {
    let docker = Cli::default();
    let node = docker.run(redis_image());
    let port = node.get_host_port_ipv4(6379);
    let redis_url = format!("redis://127.0.0.1:{port}");
    let prefix = "poison";
    let config = base_config(redis_url.clone(), prefix, "consumer-x");
    let mut source = RedisCommandSource::new(config.clone()).unwrap();

    let client = redis::Client::open(redis_url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let _: redis::Value = redis::cmd("XADD")
        .arg(&config.streams.commands)
        .arg("*")
        .arg("payload")
        .arg("{broken")
        .query_async(&mut conn)
        .await
        .unwrap();
    let _ = source.next_command().await;

    let bad_envelope = Envelope {
        schema_version: SCHEMA_VERSION + 1,
        ts_utc: chrono::Utc::now().timestamp(),
        source: "test".to_string(),
        msg_type: MessageType::Command,
        payload: OrderCommand::new(
            "strategy",
            "portfolio",
            "exchange",
            "symbol",
            CommandAction::Place(PlaceOrder {
                price: 1.0,
                qty: 1.0,
                side: alor_protocol::Side::Buy,
            }),
        ),
    };
    let payload = serde_json::to_string(&bad_envelope).unwrap();
    let _: redis::Value = redis::cmd("XADD")
        .arg(&config.streams.commands)
        .arg("*")
        .arg("payload")
        .arg(payload)
        .query_async(&mut conn)
        .await
        .unwrap();
    let _ = source.next_command().await;

    let dlq_stream = format!("{}.{}", config.streams.dlq_prefix, config.streams.commands);
    let dlq_len: i64 = conn.xlen(dlq_stream).await.unwrap();
    assert!(dlq_len >= 2);
    assert_eq!(pending_count(&config).await.unwrap(), 0);
}

#[tokio::test]
async fn stream_trim_keeps_length_bounded() {
    let docker = Cli::default();
    let node = docker.run(redis_image());
    let port = node.get_host_port_ipv4(6379);
    let redis_url = format!("redis://127.0.0.1:{port}");
    let prefix = "trim";
    let config = base_config(redis_url.clone(), prefix, "consumer-trim");
    let sink = RedisEventSink::new(config.clone()).unwrap();

    for i in 0..20 {
        sink.publish_bar(BarEvent {
            symbol: "SYM".to_string(),
            close_time_utc: i,
            o: 1.0,
            h: 1.0,
            l: 1.0,
            c: 1.0,
            v: 1.0,
            origin: DataOrigin::Live,
        })
        .await
        .unwrap();
    }

    let client = redis::Client::open(redis_url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let len: i64 = conn.xlen(&config.streams.bars).await.unwrap();
    assert!(len <= config.trim_maxlen as i64 + 2);
}
