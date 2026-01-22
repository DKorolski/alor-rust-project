use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Bar,
    Order,
    Position,
    Health,
    Command,
    CommandAck,
    SnapshotOrders,
    SnapshotPositions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope<T> {
    pub schema_version: u16,
    pub ts_utc: i64,
    pub source: String,
    pub msg_type: MessageType,
    pub payload: T,
}

impl<T> Envelope<T> {
    pub fn new(ts_utc: i64, source: impl Into<String>, msg_type: MessageType, payload: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ts_utc,
            source: source.into(),
            msg_type,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CommandAction {
    Place(PlaceOrder),
    Cancel(CancelOrder),
    Replace(ReplaceOrder),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderCommand {
    pub request_id: Uuid,
    pub strategy_id: String,
    pub portfolio: String,
    pub exchange: String,
    pub symbol: String,
    pub action: CommandAction,
    pub ttl_ms: Option<u64>,
}

impl OrderCommand {
    pub fn new(
        strategy_id: impl Into<String>,
        portfolio: impl Into<String>,
        exchange: impl Into<String>,
        symbol: impl Into<String>,
        action: CommandAction,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            strategy_id: strategy_id.into(),
            portfolio: portfolio.into(),
            exchange: exchange.into(),
            symbol: symbol.into(),
            action,
            ttl_ms: None,
        }
    }

    pub fn with_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = Some(ttl_ms);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus {
    Success,
    Error,
    Duplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandAck {
    pub request_id: Uuid,
    pub status: AckStatus,
    pub broker_order_id: Option<i64>,
    pub error_code: Option<String>,
    pub error_msg: Option<String>,
}

impl CommandAck {
    pub fn success(request_id: Uuid, broker_order_id: Option<i64>) -> Self {
        Self {
            request_id,
            status: AckStatus::Success,
            broker_order_id,
            error_code: None,
            error_msg: None,
        }
    }

    pub fn duplicate(request_id: Uuid) -> Self {
        Self {
            request_id,
            status: AckStatus::Duplicate,
            broker_order_id: None,
            error_code: None,
            error_msg: None,
        }
    }

    pub fn error(
        request_id: Uuid,
        error_code: impl Into<String>,
        error_msg: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            status: AckStatus::Error,
            broker_order_id: None,
            error_code: Some(error_code.into()),
            error_msg: Some(error_msg.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaceOrder {
    pub price: f64,
    pub qty: f64,
    pub side: Side,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CancelOrder {
    pub order_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplaceOrder {
    pub order_id: i64,
    pub new_price: f64,
    pub new_qty: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

