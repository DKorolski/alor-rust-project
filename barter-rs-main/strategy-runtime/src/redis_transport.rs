use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisStreamMessage {
    pub stream: String,
    pub id: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqPayload {
    pub reason: String,
    pub raw: String,
    pub ts_utc: i64,
    pub original_stream: String,
    pub original_id: String,
}
