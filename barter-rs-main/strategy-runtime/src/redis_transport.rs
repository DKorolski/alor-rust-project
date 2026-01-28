use std::collections::HashSet;

use anyhow::Result;
use chrono::Utc;
use redis::RedisResult;
use serde::de::DeserializeOwned;
use tracing::warn;
use uuid::Uuid;

use alor_protocol::{CommandAck, Envelope, MessageType, OrderCommand, SCHEMA_VERSION};

use crate::{BarEvent, OrderEvent, PositionEvent, RuntimeConfig};

const PAYLOAD_FIELD: &str = "payload";

#[derive(Debug, Clone)]
pub struct RuntimeMessage<T> {
    pub stream: String,
    pub message_id: String,
    pub payload: T,
}

#[derive(Debug, Clone)]
pub struct RedisStreamMessage {
    pub stream: String,
    pub id: String,
    pub payload: String,
}

#[derive(Debug, Clone)]
pub struct DlqPayload {
    pub reason: String,
    pub raw: String,
    pub ts_utc: i64,
    pub original_stream: String,
    pub original_id: String,
}

pub struct RedisRuntimeTransport {
    client: redis::Client,
    config: RuntimeConfig,
    group_initialized: HashSet<String>,
}

impl RedisRuntimeTransport {
    pub fn new(mut config: RuntimeConfig) -> Result<Self> {
        if config.consumer_name.trim().is_empty() || config.consumer_name == "auto" {
            config.consumer_name = format!("runtime-{}", Uuid::new_v4());
        }
        let client = redis::Client::open(config.redis_url.clone())?;
        Ok(Self {
            client,
            config,
            group_initialized: HashSet::new(),
        })
    }

    pub async fn ensure_group(&mut self, stream: &str) -> Result<()> {
        if self.group_initialized.contains(stream) {
            return Ok(());
        }
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let result: RedisResult<redis::Value> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(&self.config.consumer_group)
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
        match result {
            Ok(_) => {
                self.group_initialized.insert(stream.to_string());
                Ok(())
            }
            Err(err) => {
                if err.to_string().contains("BUSYGROUP") {
                    self.group_initialized.insert(stream.to_string());
                    Ok(())
                } else {
                    Err(err.into())
                }
            }
        }
    }

    pub async fn ensure_groups(&mut self, streams: &[&str]) -> Result<()> {
        for stream in streams {
            self.ensure_group(stream).await?;
        }
        Ok(())
    }

    pub async fn xadd_state(&self, stream: &str, payload: &str, maxlen: usize) -> Result<String> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let id: redis::Value = redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(maxlen)
            .arg("*")
            .arg(PAYLOAD_FIELD)
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        Ok(match id {
            redis::Value::Data(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            _ => "".to_string(),
        })
    }

    pub async fn publish_command(&self, command: &OrderCommand) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let envelope = Envelope::new(
            Utc::now().timestamp(),
            &self.config.source,
            MessageType::Command,
            command,
        );
        let json = serde_json::to_string(&envelope)?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(&self.config.streams.commands)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen_commands)
            .arg("*")
            .arg(PAYLOAD_FIELD)
            .arg(json)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn publish_command_and_state(
        &self,
        command: &OrderCommand,
        state_payload: &str,
    ) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let envelope = Envelope::new(
            Utc::now().timestamp(),
            &self.config.source,
            MessageType::Command,
            command,
        );
        let command_json = serde_json::to_string(&envelope)?;
        redis::pipe()
            .atomic()
            .cmd("XADD")
            .arg(&self.config.streams.commands)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen_commands)
            .arg("*")
            .arg(PAYLOAD_FIELD)
            .arg(command_json)
            .cmd("XADD")
            .arg(&self.config.runtime_state_stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.trim_maxlen_runtime_state)
            .arg("*")
            .arg(PAYLOAD_FIELD)
            .arg(state_payload)
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn hgetall(&self, key: &str) -> Result<Vec<(String, String)>> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let values: Vec<(String, String)> = redis::cmd("HGETALL")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        Ok(values)
    }

    pub async fn xrevrange_last(&self, stream: &str) -> Result<Option<String>> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let reply: redis::Value = redis::cmd("XREVRANGE")
            .arg(stream)
            .arg("+")
            .arg("-")
            .arg("COUNT")
            .arg(1)
            .query_async(&mut conn)
            .await?;
        let entries = match reply {
            redis::Value::Bulk(entries) => entries,
            _ => return Ok(None),
        };
        let entry = match entries.first() {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let entry = match entry {
            redis::Value::Bulk(values) => values,
            _ => return Ok(None),
        };
        if entry.len() < 2 {
            return Ok(None);
        }
        Ok(self.extract_payload(&entry[1]))
    }

    pub async fn xack(&self, stream: &str, message_id: &str) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: redis::Value = redis::cmd("XACK")
            .arg(stream)
            .arg(&self.config.consumer_group)
            .arg(message_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn read_group(&self, stream: &str, count: usize) -> RedisResult<redis::Value> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&self.config.consumer_group)
            .arg(&self.config.consumer_name)
            .arg("BLOCK")
            .arg(self.config.block_ms)
            .arg("COUNT")
            .arg(count)
            .arg("STREAMS")
            .arg(stream)
            .arg(">")
            .query_async(&mut conn)
            .await
    }

    pub async fn claim_idle(
        &self,
        stream: &str,
        start: &str,
        count: usize,
    ) -> RedisResult<redis::Value> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        redis::cmd("XAUTOCLAIM")
            .arg(stream)
            .arg(&self.config.consumer_group)
            .arg(&self.config.consumer_name)
            .arg(self.config.claim_idle_ms)
            .arg(start)
            .arg("COUNT")
            .arg(count)
            .query_async(&mut conn)
            .await
    }

    pub async fn write_dlq(
        &self,
        original_stream: &str,
        message_id: &str,
        payload: &str,
        reason: &str,
        trim_maxlen: usize,
    ) -> Result<()> {
        let dlq_stream = format!("{}.{}", self.config.streams.dlq_prefix, original_stream);
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: redis::Value = redis::cmd("XADD")
            .arg(&dlq_stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(trim_maxlen)
            .arg("*")
            .arg("original_stream")
            .arg(original_stream)
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

    pub async fn next_bar(&self) -> Option<RuntimeMessage<BarEvent>> {
        self.next_message(&self.config.streams.bars, MessageType::Bar, self.config.trim_maxlen_bars)
            .await
    }

    pub async fn next_ack(&self) -> Option<RuntimeMessage<CommandAck>> {
        self.next_message(&self.config.streams.acks, MessageType::CommandAck, self.config.trim_maxlen_acks)
            .await
    }

    pub async fn next_order(&self) -> Option<RuntimeMessage<OrderEvent>> {
        self.next_message(&self.config.streams.orders, MessageType::Order, self.config.trim_maxlen_orders)
            .await
    }

    pub async fn next_position(&self) -> Option<RuntimeMessage<PositionEvent>> {
        self.next_message(
            &self.config.streams.positions,
            MessageType::Position,
            self.config.trim_maxlen_positions,
        )
        .await
    }

    async fn next_message<T: DeserializeOwned>(
        &self,
        stream: &str,
        msg_type: MessageType,
        trim_maxlen: usize,
    ) -> Option<RuntimeMessage<T>> {
        let reply = self.read_group(stream, 1).await.ok()?;
        let mut entries = self.parse_read_group_entries(stream, reply);
        let entry = entries.pop()?;
        self.decode_entry(stream, msg_type, trim_maxlen, entry).await
    }

    pub fn parse_autoclaim_entries(
        &self,
        stream: &str,
        reply: redis::Value,
    ) -> (String, Vec<RedisStreamMessage>) {
        let parts = match reply {
            redis::Value::Bulk(values) => values,
            _ => return ("0-0".to_string(), Vec::new()),
        };
        if parts.len() < 2 {
            return ("0-0".to_string(), Vec::new());
        }
        let next_start_id = match &parts[0] {
            redis::Value::Data(data) => String::from_utf8_lossy(data).to_string(),
            _ => "0-0".to_string(),
        };
        let entries = match &parts[1] {
            redis::Value::Bulk(values) => values,
            _ => return (next_start_id, Vec::new()),
        };
        let parsed = entries
            .iter()
            .filter_map(|entry| self.parse_entry(stream, entry))
            .collect();
        (next_start_id, parsed)
    }

    pub fn parse_read_group_entries(
        &self,
        stream_name: &str,
        reply: redis::Value,
    ) -> Vec<RedisStreamMessage> {
        let streams = match reply {
            redis::Value::Bulk(streams) => streams,
            _ => return Vec::new(),
        };
        let stream = match streams.first() {
            Some(stream) => stream,
            None => return Vec::new(),
        };
        let items = match stream {
            redis::Value::Bulk(values) => values,
            _ => return Vec::new(),
        };
        if items.len() < 2 {
            return Vec::new();
        }
        let entries = match &items[1] {
            redis::Value::Bulk(entries) => entries,
            _ => return Vec::new(),
        };
        entries
            .iter()
            .filter_map(|entry| self.parse_entry(stream_name, entry))
            .collect()
    }

    fn parse_entry(&self, stream_name: &str, entry: &redis::Value) -> Option<RedisStreamMessage> {
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
        let payload = self.extract_payload(&entry[1]).unwrap_or_default();
        Some(RedisStreamMessage {
            stream: stream_name.to_string(),
            id: message_id,
            payload,
        })
    }

    fn extract_payload(&self, fields: &redis::Value) -> Option<String> {
        let fields = match fields {
            redis::Value::Bulk(values) => values,
            _ => return None,
        };
        for chunk in fields.chunks(2) {
            if let [key, value] = chunk {
                if let redis::Value::Data(key) = key {
                    if key == PAYLOAD_FIELD.as_bytes() {
                        if let redis::Value::Data(value) = value {
                            return Some(String::from_utf8_lossy(value).to_string());
                        }
                    }
                }
            }
        }
        None
    }

    pub async fn decode_entry<T: DeserializeOwned>(
        &self,
        stream: &str,
        msg_type: MessageType,
        trim_maxlen: usize,
        entry: RedisStreamMessage,
    ) -> Option<RuntimeMessage<T>> {
        let message_id = entry.id;
        let payload = entry.payload;
        if payload.is_empty() {
            if let Err(error) = self
                .write_dlq(stream, &message_id, &payload, "missing_payload", trim_maxlen)
                .await
            {
                warn!(?error, stream, message_id, "dlq write failed");
            }
            let _ = self.xack(stream, &message_id).await;
            return None;
        }
        match self.parse_message::<T>(&payload, msg_type) {
            Ok(envelope) => Some(RuntimeMessage {
                stream: stream.to_string(),
                message_id,
                payload: envelope.payload,
            }),
            Err(reason) => {
                warn!(?reason, stream, message_id, "runtime message decode failed");
                if let Err(error) = self
                    .write_dlq(stream, &message_id, &payload, &reason, trim_maxlen)
                    .await
                {
                    warn!(?error, stream, message_id, "dlq write failed");
                }
                let _ = self.xack(stream, &message_id).await;
                None
            }
        }
    }

    fn parse_message<T: DeserializeOwned>(
        &self,
        payload: &str,
        expected: MessageType,
    ) -> Result<Envelope<T>, String> {
        let envelope: Envelope<T> = serde_json::from_str(payload)
            .map_err(|error| format!("parse_error: {error}"))?;
        if envelope.schema_version > SCHEMA_VERSION {
            return Err("unsupported_schema".to_string());
        }
        if envelope.msg_type != expected {
            return Err("unexpected_msg_type".to_string());
        }
        Ok(envelope)
    }
}
