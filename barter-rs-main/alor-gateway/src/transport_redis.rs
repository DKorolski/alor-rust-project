use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use redis::RedisResult;
use serde::de::DeserializeOwned;

use alor_protocol::{CommandAck, Envelope, MessageType, OrderCommand, SCHEMA_VERSION};

use crate::health::HealthState;
use crate::models::{BarEvent, OrderEvent, OrdersSnapshot, PositionEvent, PositionsSnapshot};
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
        let envelope = Envelope::new(
            Utc::now().timestamp(),
            self.config.source.clone(),
            msg_type,
            payload,
        );
        let payload = serde_json::to_string(&envelope)?;
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen)
            .arg("*")
            .arg("payload")
            .arg(payload)
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
}

impl RedisCommandSource {
    pub fn new(config: TransportConfig) -> Result<Self> {
        let client = redis::Client::open(config.redis_url.clone())?;
        Ok(Self { client, config })
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

    async fn write_dlq(&self, stream: &str, payload: &str, reason: &str) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let dlq_stream = format!("{}.{}", self.config.streams.dlq_prefix, stream);
        let _: redis::Value = redis::cmd("XADD")
            .arg(dlq_stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen)
            .arg("*")
            .arg("reason")
            .arg(reason)
            .arg("payload")
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    async fn parse_message<T: DeserializeOwned>(
        &self,
        payload: &str,
    ) -> Result<Envelope<T>> {
        let envelope: Envelope<T> = serde_json::from_str(payload)?;
        Ok(envelope)
    }
}

#[async_trait]
impl CommandSource for RedisCommandSource {
    async fn next_command(&mut self) -> Option<CommandEnvelope> {
        let reply = self.read_group().await.ok()?;
        let streams = match reply {
            redis::Value::Bulk(streams) => streams,
            _ => return None,
        };
        let stream = streams.first()?;
        let entries = match stream {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        if entries.len() < 2 {
            return None;
        }
        let entries = match &entries[1] {
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
            redis::Value::Data(bytes) => String::from_utf8_lossy(bytes).to_string(),
            _ => return None,
        };
        let fields = match &entry[1] {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        let mut payload = None;
        for chunk in fields.chunks(2) {
            if let [key, value] = chunk {
                if let redis::Value::Data(key) = key {
                    if key == b"payload" {
                        if let redis::Value::Data(value) = value {
                            payload = Some(String::from_utf8_lossy(value).to_string());
                        }
                    }
                }
            }
        }
        let payload = payload?;
        match self.parse_message::<OrderCommand>(&payload).await {
            Ok(envelope) => {
                if envelope.schema_version > SCHEMA_VERSION {
                    if let Err(error) = self.write_dlq(&self.config.streams.commands, &payload, "unsupported_schema").await {
                        tracing::warn!(?error, "dlq write failed");
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
                tracing::warn!(?error, "command decode failed");
                None
            }
        }
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

    async fn publish_envelope<T: serde::Serialize>(
        &self,
        stream: &str,
        msg_type: MessageType,
        payload: &T,
    ) -> Result<()> {
        let envelope = Envelope::new(
            Utc::now().timestamp(),
            self.config.source.clone(),
            msg_type,
            payload,
        );
        let payload = serde_json::to_string(&envelope)?;
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen)
            .arg("*")
            .arg("payload")
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl CommandSink for RedisCommandSink {
    async fn publish_command(&self, command: OrderCommand) -> Result<()> {
        self.publish_envelope(&self.config.streams.commands, MessageType::Command, &command)
            .await
    }

    async fn publish_ack(&self, ack: CommandAck) -> Result<()> {
        self.publish_envelope(&self.config.streams.acks, MessageType::CommandAck, &ack)
            .await
    }
}

pub async fn claim_pending(
    config: &TransportConfig,
    idle_ms: u64,
) -> Result<Vec<String>> {
    let client = redis::Client::open(config.redis_url.clone())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XAUTOCLAIM")
        .arg(&config.streams.commands)
        .arg(&config.consumer_group)
        .arg(&config.consumer_name)
        .arg(idle_ms)
        .arg("0-0")
        .arg("COUNT")
        .arg(10)
        .query_async(&mut conn)
        .await?;
    let mut ids = Vec::new();
    if let redis::Value::Bulk(values) = reply {
        if let Some(redis::Value::Bulk(entries)) = values.get(1) {
            for entry in entries {
                if let redis::Value::Bulk(entry) = entry {
                    if let Some(redis::Value::Data(id)) = entry.first() {
                        ids.push(String::from_utf8_lossy(id).to_string());
                    }
                }
            }
        }
    }
    Ok(ids)
}
