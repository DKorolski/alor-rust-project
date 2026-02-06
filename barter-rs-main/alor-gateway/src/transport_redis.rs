use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use redis::RedisResult;
use serde::de::DeserializeOwned;
use tracing::warn;
use uuid::Uuid;

use alor_protocol::{CommandAck, Envelope, MessageType, OrderCommand, SCHEMA_VERSION};

use crate::health::HealthState;
use crate::models::{
    BarEvent, OrderEvent, OrdersSnapshot, PositionEvent, PositionsSnapshot, TradeEvent,
};
use crate::transport::{CommandEnvelope, CommandSink, CommandSource, EventSink, TransportConfig};

pub struct RedisEventSink {
    client: redis::Client,
    config: TransportConfig,
}

impl RedisEventSink {
    pub fn new(config: TransportConfig) -> Result<Self> {
        let client = redis::Client::open(config.redis_url.clone())?;
        Ok(Self { client, config })
    }

    async fn publish_event<T: serde::Serialize>(
        &self,
        stream: &str,
        msg_type: MessageType,
        payload: &T,
    ) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let envelope = Envelope::new(Utc::now().timestamp(), &self.config.source, msg_type, payload);
        let json = serde_json::to_string(&envelope)?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg(self.config.trim_maxlen)
            .arg("*")
            .arg("payload")
            .arg(json)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl EventSink for RedisEventSink {
    async fn publish_bar(&self, event: BarEvent) -> Result<()> {
        self.publish_event(&self.config.streams.bars, MessageType::Bar, &event)
            .await
    }

    async fn publish_order(&self, event: OrderEvent) -> Result<()> {
        self.publish_event(&self.config.streams.orders, MessageType::Order, &event)
            .await
    }

    async fn publish_trade(&self, event: TradeEvent) -> Result<()> {
        self.publish_event(&self.config.streams.trades, MessageType::Trade, &event)
            .await
    }

    async fn publish_position(&self, event: PositionEvent) -> Result<()> {
        self.publish_event(
            &self.config.streams.positions,
            MessageType::Position,
            &event,
        )
        .await
    }

    async fn publish_health(&self, health: HealthState) -> Result<()> {
        self.publish_event(&self.config.streams.health, MessageType::Health, &health)
            .await
    }

    async fn publish_snapshot_orders(&self, snapshot: OrdersSnapshot) -> Result<()> {
        self.publish_event(
            &self.config.streams.snapshots,
            MessageType::SnapshotOrders,
            &snapshot,
        )
        .await
    }

    async fn publish_snapshot_positions(&self, snapshot: PositionsSnapshot) -> Result<()> {
        self.publish_event(
            &self.config.streams.snapshots,
            MessageType::SnapshotPositions,
            &snapshot,
        )
        .await
    }
}

pub struct RedisCommandSource {
    client: redis::Client,
    config: TransportConfig,
    group_initialized: bool,
    dlq_key: String,
}

struct StreamEntry {
    message_id: String,
    payload: Option<String>,
}

impl RedisCommandSource {
    pub fn new(config: TransportConfig) -> Result<Self> {
        let mut config = config;
        if config.consumer_name.trim().is_empty() || config.consumer_name == "auto" {
            config.consumer_name = format!("consumer-{}", Uuid::new_v4());
        }
        let client = redis::Client::open(config.redis_url.clone())?;
        // tests ожидают dlq_stream = "{dlq_prefix}.{commands_stream}"
        let dlq_key = format!("{}.{}", config.streams.dlq_prefix, config.streams.commands);

        Ok(Self {
            client,
            config,
            group_initialized: false,
            dlq_key,
        })
    }

    // P0.1: гарантируем, что группа создана (иначе XREADGROUP даст NOGROUP)
    async fn ensure_group(&mut self) -> Result<()> {
        if self.group_initialized {
            return Ok(());
        }
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let result: RedisResult<redis::Value> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&self.config.streams.commands)
            .arg(&self.config.consumer_group)
            // Важно: "0" (а не "$"), чтобы уже существующие записи стали доставляемыми.
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;

        match result {
            Ok(_) => {
                self.group_initialized = true;
                Ok(())
            }
            Err(err) => {
                if err.to_string().contains("BUSYGROUP") {
                    self.group_initialized = true;
                    Ok(())
                } else {
                    Err(err.into())
                }
            }
        }
    }

    async fn read_group(&self) -> RedisResult<redis::Value> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&self.config.consumer_group)
            .arg(&self.config.consumer_name)
            .arg("BLOCK")
            .arg(self.config.block_ms)
            .arg("COUNT")
            .arg(1)
            .arg("STREAMS")
            .arg(&self.config.streams.commands)
            .arg(">")
            .query_async(&mut conn)
            .await
    }

    // P0.2: подбираем pending (idle) сообщения у других consumer’ов
    async fn claim_idle(&self) -> RedisResult<redis::Value> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        redis::cmd("XAUTOCLAIM")
            .arg(&self.config.streams.commands)
            .arg(&self.config.consumer_group)
            .arg(&self.config.consumer_name)
            .arg(self.config.claim_idle_ms)
            .arg("0-0")
            .arg("COUNT")
            .arg(1)
            .query_async(&mut conn)
            .await
    }

    // P0.3: DLQ + ACK «poison» сообщений
    async fn write_dlq(
        &self,
        stream: &str,
        message_id: &str,
        payload: &str,
        reason: &str,
    ) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(&self.dlq_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen)
            .arg("*")
            .arg("original_stream")
            .arg(stream)
            .arg("original_id")
            .arg(message_id)
            .arg("reason")
            .arg(reason)
            .arg("raw")
            .arg(payload)
            .arg("ts_utc")
            .arg(Utc::now().timestamp())
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    fn parse_message<T: DeserializeOwned>(&self, payload: &str) -> Result<Envelope<T>> {
        let envelope: Envelope<T> = serde_json::from_str(payload)?;
        Ok(envelope)
    }

    fn extract_payload(&self, fields: &redis::Value) -> Option<String> {
        let fields = match fields {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        for chunk in fields.chunks(2) {
            if let [key, value] = chunk {
                if let redis::Value::Data(key) = key {
                    if key == b"payload" {
                        if let redis::Value::Data(value) = value {
                            return Some(String::from_utf8_lossy(value).to_string());
                        }
                    }
                }
            }
        }
        None
    }

    fn parse_read_group_entry(&self, reply: redis::Value) -> Option<StreamEntry> {
        // [[ stream_name, [[ id, [k,v,k,v...] ], ... ] ]]
        let streams = match reply {
            redis::Value::Bulk(streams) => streams,
            _ => return None,
        };
        let stream = streams.first()?;
        let items = match stream {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        if items.len() < 2 {
            return None;
        }
        let entries = match &items[1] {
            redis::Value::Bulk(entries) => entries,
            _ => return None,
        };
        let entry = entries.first()?;
        let entry = match entry {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        if entry.len() < 2 {
            return None;
        }
        let message_id = match &entry[0] {
            redis::Value::Data(data) => String::from_utf8_lossy(data).to_string(),
            _ => return None,
        };
        let payload = self.extract_payload(&entry[1]);
        Some(StreamEntry { message_id, payload })
    }

    fn parse_autoclaim_entry(&self, reply: redis::Value) -> Option<StreamEntry> {
        // [ next_start_id, [[ id, [k,v,...] ], ...], [deleted_ids...] ]
        let parts = match reply {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        if parts.len() < 2 {
            return None;
        }
        let entries = match &parts[1] {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        let entry = entries.first()?;
        let entry = match entry {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        if entry.len() < 2 {
            return None;
        }
        let message_id = match &entry[0] {
            redis::Value::Data(data) => String::from_utf8_lossy(data).to_string(),
            _ => return None,
        };
        let payload = self.extract_payload(&entry[1]);
        Some(StreamEntry { message_id, payload })
    }

    async fn handle_payload(&self, entry: StreamEntry) -> Option<CommandEnvelope> {
        let message_id = entry.message_id;

        let Some(payload) = entry.payload else {
            if let Err(error) = self
                .write_dlq(
                    &self.config.streams.commands,
                    &message_id,
                    "",
                    "missing_payload",
                )
                .await
            {
                warn!(?error, "dlq write failed");
            }
            let _ = self.ack(&message_id).await;
            return None;
        };

        match self.parse_message::<OrderCommand>(&payload) {
            Ok(envelope) => {
                if envelope.schema_version > SCHEMA_VERSION {
                    if let Err(error) = self
                        .write_dlq(
                            &self.config.streams.commands,
                            &message_id,
                            &payload,
                            "unsupported_schema",
                        )
                        .await
                    {
                        warn!(?error, "dlq write failed");
                    }
                    let _ = self.ack(&message_id).await;
                    return None;
                }

                if envelope.msg_type != MessageType::Command {
                    if let Err(error) = self
                        .write_dlq(
                            &self.config.streams.commands,
                            &message_id,
                            &payload,
                            "unexpected_msg_type",
                        )
                        .await
                    {
                        warn!(?error, "dlq write failed");
                    }
                    let _ = self.ack(&message_id).await;
                    return None;
                }

                Some(CommandEnvelope {
                    command: envelope.payload,
                    message_id: Some(message_id),
                })
            }
            Err(error) => {
                warn!(?error, "command decode failed");
                if let Err(error) = self
                    .write_dlq(
                        &self.config.streams.commands,
                        &message_id,
                        &payload,
                        "parse_error",
                    )
                    .await
                {
                    warn!(?error, "dlq write failed");
                }
                let _ = self.ack(&message_id).await;
                None
            }
        }
    }
}

#[async_trait]
impl CommandSource for RedisCommandSource {
    async fn next_command(&mut self) -> Result<Option<CommandEnvelope>> {
        self.ensure_group().await?;

        // P0.2: сначала пытаемся вернуть pending (idle) сообщение
        let reply = self.claim_idle().await?;
        if let Some(entry) = self.parse_autoclaim_entry(reply) {
            return Ok(self.handle_payload(entry).await);
        }

        // затем читаем новые
        let reply = self.read_group().await?;
        if let Some(entry) = self.parse_read_group_entry(reply) {
            return Ok(self.handle_payload(entry).await);
        }

        Ok(None)
    }

    async fn ack(&self, message_id: &str) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: redis::Value = redis::cmd("XACK")
            .arg(&self.config.streams.commands)
            .arg(&self.config.consumer_group)
            .arg(message_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

pub struct RedisCommandSink {
    client: redis::Client,
    config: TransportConfig,
}

impl RedisCommandSink {
    pub fn new(config: TransportConfig) -> Result<Self> {
        let client = redis::Client::open(config.redis_url.clone())?;
        Ok(Self { client, config })
    }

    async fn publish<T: serde::Serialize>(
        &self,
        stream: &str,
        msg_type: MessageType,
        payload: &T,
    ) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let envelope = Envelope::new(Utc::now().timestamp(), &self.config.source, msg_type, payload);
        let json = serde_json::to_string(&envelope)?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen)
            .arg("*")
            .arg("payload")
            .arg(json)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl CommandSink for RedisCommandSink {
    async fn publish_command(&self, command: OrderCommand) -> Result<()> {
        self.publish(
            &self.config.streams.commands,
            MessageType::Command,
            &command,
        )
        .await
    }

    async fn publish_ack(&self, ack: CommandAck) -> Result<()> {
        self.publish(&self.config.streams.acks, MessageType::CommandAck, &ack)
            .await
    }
}
