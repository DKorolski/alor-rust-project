use crate::{
    Identifier,
    event::{MarketEvent, MarketIter},
    subscription::trade::PublicTrade,
};
use barter_instrument::{Side, exchange::ExchangeId};
use barter_integration::subscription::SubscriptionId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, PartialEq, PartialOrd, Debug, Serialize)]
pub struct AlorTrade {
    pub id: String,
    pub symbol: String,
    pub price: f64,
    pub qty: f64,
    pub timestamp: Option<u64>,
    pub side: Side,
}

#[derive(Deserialize)]
struct RawAlorTrade {
    #[serde(deserialize_with = "de_trade_id")]
    id: String,
    symbol: String,
    price: f64,
    #[serde(alias = "qty", alias = "volume")]
    qty: f64,
    #[serde(default)]
    timestamp: Option<Value>,
    #[serde(default)]
    time: Option<Value>,
    #[serde(deserialize_with = "de_alor_side")]
    side: Side,
}

impl<'de> Deserialize<'de> for AlorTrade {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let raw = RawAlorTrade::deserialize(deserializer)?;

        let timestamp =
            de_optional_timestamp(raw.timestamp.or(raw.time)).map_err(serde::de::Error::custom)?;

        Ok(Self {
            id: raw.id,
            symbol: raw.symbol,
            price: raw.price,
            qty: raw.qty,
            timestamp,
            side: raw.side,
        })
    }
}

impl Identifier<Option<SubscriptionId>> for AlorTrade {
    fn id(&self) -> Option<SubscriptionId> {
        None
    }
}

impl<InstrumentKey> From<(ExchangeId, InstrumentKey, super::AlorPayload<AlorTrade>)>
    for MarketIter<InstrumentKey, PublicTrade>
{
    fn from(
        (exchange, instrument, payload): (ExchangeId, InstrumentKey, super::AlorPayload<AlorTrade>),
    ) -> Self {
        let Some(trade) = payload.data else {
            return Self(Vec::new());
        };

        let time_exchange = trade
            .timestamp
            .and_then(|ms| DateTime::from_timestamp_millis(ms as i64))
            .unwrap_or_else(Utc::now);

        Self(vec![Ok(MarketEvent {
            time_exchange,
            time_received: Utc::now(),
            exchange,
            instrument,
            kind: PublicTrade {
                id: trade.id,
                price: trade.price,
                amount: trade.qty,
                side: trade.side,
            },
        })])
    }
}

fn de_alor_side<'de, D>(deserializer: D) -> Result<Side, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let value = <&str as Deserialize>::deserialize(deserializer)?;
    match value.to_lowercase().as_str() {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        other => Err(serde::de::Error::custom(format!("unknown side: {other}"))),
    }
}

fn de_trade_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!("unexpected id: {other}"))),
    }
}

fn de_optional_timestamp(value: Option<Value>) -> Result<Option<u64>, String> {
    match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| "timestamp not u64".to_string())
            .map(Some),
        Some(Value::String(s)) => chrono::DateTime::parse_from_rfc3339(&s)
            .map(|dt| Some(dt.timestamp_millis() as u64))
            .map_err(|err| format!("invalid time: {err}")),
        Some(other) => Err(format!("unexpected time: {other}")),
        None => Ok(None),
    }
}