use chrono::Utc;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

pub const SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Bar,
    Order,
    StopOrder,
    Trade,
    Position,
    Health,
    Command,
    CommandAck,
    SnapshotOrders,
    SnapshotStopOrders,
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
    Market(MarketOrder),
    Cancel(CancelOrder),
    Replace(ReplaceOrder),
    CreateStopLimit(CreateStopLimitOrder),
    DeleteStopLimit(DeleteStopLimitOrder),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntentClass {
    Entry,
    Exit,
    CancelCleanup,
    ProtectiveRepair,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderCommand {
    pub request_id: Uuid,
    #[serde(default = "default_created_ts_utc")]
    pub created_ts_utc: i64,
    pub strategy_id: String,
    pub portfolio: String,
    pub exchange: String,
    pub symbol: String,
    pub action: CommandAction,
    #[serde(default)]
    pub intent_class: Option<IntentClass>,
    pub ttl_ms: Option<u64>,
}

fn default_created_ts_utc() -> i64 {
    0
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
            created_ts_utc: Utc::now().timestamp(),
            strategy_id: strategy_id.into(),
            portfolio: portfolio.into(),
            exchange: exchange.into(),
            symbol: symbol.into(),
            action,
            intent_class: None,
            ttl_ms: None,
        }
    }

    pub fn with_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = Some(ttl_ms);
        self
    }

    pub fn effective_intent_class(&self) -> IntentClass {
        infer_intent_class(self.intent_class, &self.action)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus {
    Accepted,
    Confirmed,
    Rejected,
    Duplicate,
    Expired,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandAck {
    pub request_id: Uuid,
    pub status: AckStatus,
    pub broker_order_id: Option<i64>,
    #[serde(default)]
    pub broker_order_id_str: Option<String>,
    pub error_code: Option<String>,
    pub error_msg: Option<String>,
    #[serde(default)]
    pub cws_http_code: Option<i64>,
    #[serde(default)]
    pub cws_message: Option<String>,
    #[serde(default)]
    pub cws_request_guid: Option<String>,
    #[serde(default = "default_processed_ts_utc")]
    pub processed_ts_utc: i64,
}

impl CommandAck {
    pub fn confirmed(request_id: Uuid, broker_order_id: Option<i64>) -> Self {
        Self {
            request_id,
            status: AckStatus::Confirmed,
            broker_order_id,
            broker_order_id_str: broker_order_id.map(|v| v.to_string()),
            error_code: None,
            error_msg: None,
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: None,
            processed_ts_utc: Utc::now().timestamp(),
        }
    }

    pub fn duplicate(request_id: Uuid) -> Self {
        Self {
            request_id,
            status: AckStatus::Duplicate,
            broker_order_id: None,
            broker_order_id_str: None,
            error_code: None,
            error_msg: None,
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: None,
            processed_ts_utc: Utc::now().timestamp(),
        }
    }

    pub fn rejected(
        request_id: Uuid,
        error_code: impl Into<String>,
        error_msg: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            status: AckStatus::Rejected,
            broker_order_id: None,
            broker_order_id_str: None,
            error_code: Some(error_code.into()),
            error_msg: Some(error_msg.into()),
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: None,
            processed_ts_utc: Utc::now().timestamp(),
        }
    }

    pub fn accepted(request_id: Uuid) -> Self {
        Self {
            request_id,
            status: AckStatus::Accepted,
            broker_order_id: None,
            broker_order_id_str: None,
            error_code: None,
            error_msg: None,
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: None,
            processed_ts_utc: Utc::now().timestamp(),
        }
    }

    pub fn expired(request_id: Uuid, error_msg: impl Into<String>) -> Self {
        Self {
            request_id,
            status: AckStatus::Expired,
            broker_order_id: None,
            broker_order_id_str: None,
            error_code: Some("expired".to_string()),
            error_msg: Some(error_msg.into()),
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: None,
            processed_ts_utc: Utc::now().timestamp(),
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
            broker_order_id_str: None,
            error_code: Some(error_code.into()),
            error_msg: Some(error_msg.into()),
            cws_http_code: None,
            cws_message: None,
            cws_request_guid: None,
            processed_ts_utc: Utc::now().timestamp(),
        }
    }
}

fn default_processed_ts_utc() -> i64 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaceOrder {
    pub price: f64,
    pub qty: f64,
    pub side: Side,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketOrder {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateStopLimitOrder {
    pub side: Side,
    pub qty: f64,
    pub trigger_price: f64,
    pub price: f64,
    pub condition: StopLimitCondition,
    pub stop_end_unix_time: i64,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub instrument_group: Option<String>,
    #[serde(default)]
    pub check_duplicates: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeleteStopLimitOrder {
    pub order_id: String,
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default)]
    pub check_duplicates: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopLimitCondition {
    More,
    Less,
    MoreOrEqual,
    LessOrEqual,
}

impl StopLimitCondition {
    pub fn as_canonical_str(self) -> &'static str {
        match self {
            StopLimitCondition::More => "more",
            StopLimitCondition::Less => "less",
            StopLimitCondition::MoreOrEqual => "moreorequal",
            StopLimitCondition::LessOrEqual => "lessorequal",
        }
    }

    fn parse_tolerant(input: &str) -> Option<Self> {
        let normalized = input
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        match normalized.as_str() {
            "more" => Some(Self::More),
            "less" => Some(Self::Less),
            "moreorequal" | "moreequal" => Some(Self::MoreOrEqual),
            "lessorequal" | "lessequal" => Some(Self::LessOrEqual),
            _ => None,
        }
    }
}

impl Serialize for StopLimitCondition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_canonical_str())
    }
}

impl<'de> Deserialize<'de> for StopLimitCondition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse_tolerant(&raw)
            .ok_or_else(|| D::Error::custom(format!("invalid stop limit condition: {raw}")))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

pub fn infer_intent_class(explicit: Option<IntentClass>, action: &CommandAction) -> IntentClass {
    explicit.unwrap_or_else(|| match action {
        CommandAction::Cancel(_) | CommandAction::DeleteStopLimit(_) => IntentClass::CancelCleanup,
        CommandAction::Market(_) => IntentClass::Entry,
        CommandAction::Place(_) => IntentClass::Entry,
        CommandAction::Replace(_) => IntentClass::ProtectiveRepair,
        CommandAction::CreateStopLimit(_) => IntentClass::ProtectiveRepair,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn infer_intent_class_from_delete_stop_limit() {
        let action = CommandAction::DeleteStopLimit(DeleteStopLimitOrder {
            order_id: "12345".to_string(),
            side: None,
            check_duplicates: None,
        });
        assert_eq!(
            infer_intent_class(None, &action),
            IntentClass::CancelCleanup
        );
    }

    #[test]
    fn infer_intent_class_respects_explicit_value() {
        let action = CommandAction::Market(MarketOrder {
            qty: 1.0,
            side: Side::Sell,
        });
        assert_eq!(
            infer_intent_class(Some(IntentClass::Exit), &action),
            IntentClass::Exit
        );
    }

    #[test]
    fn deserialize_order_command_without_intent_class_is_supported() {
        let payload = json!({
            "request_id": Uuid::new_v4(),
            "created_ts_utc": 1,
            "strategy_id": "s",
            "portfolio": "p",
            "exchange": "MOEX",
            "symbol": "SBER",
            "action": {
                "market": {
                    "qty": 1.0,
                    "side": "buy"
                }
            },
            "ttl_ms": null
        });
        let cmd: OrderCommand = serde_json::from_value(payload).expect("deserialize order command");
        assert_eq!(cmd.intent_class, None);
        assert_eq!(cmd.effective_intent_class(), IntentClass::Entry);
    }

    #[test]
    fn stop_limit_condition_serializes_to_canonical_form() {
        let s = serde_json::to_string(&StopLimitCondition::LessOrEqual).expect("serialize");
        assert_eq!(s, "\"lessorequal\"");
    }

    #[test]
    fn stop_limit_condition_parses_aliases() {
        let aliases = [
            "\"lessorequal\"",
            "\"LessOrEqual\"",
            "\"less_or_equal\"",
            "\"LESS_OR_EQUAL\"",
        ];
        for alias in aliases {
            let parsed: StopLimitCondition = serde_json::from_str(alias).expect("parse alias");
            assert_eq!(parsed, StopLimitCondition::LessOrEqual);
        }
    }
}
