use crate::{
    Identifier,
    books::{Level, OrderBook},
    event::{MarketEvent, MarketIter},
    subscription::book::OrderBookEvent,
};
use barter_instrument::exchange::ExchangeId;
use barter_integration::subscription::SubscriptionId;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, PartialOrd, Debug, Deserialize, Serialize)]
pub struct AlorLevel {
    pub price: f64,
    #[serde(alias = "volume", alias = "qty")]
    pub amount: f64,
}

#[derive(Clone, PartialEq, PartialOrd, Debug, Deserialize, Serialize)]
pub struct AlorOrderBookMessage {
    #[serde(default)]
    pub snapshot: bool,
    #[serde(default)]
    pub bids: Vec<AlorLevel>,
    #[serde(default)]
    pub asks: Vec<AlorLevel>,
    pub timestamp: Option<u64>,
    #[serde(rename = "ms_timestamp")]
    pub ms_timestamp: Option<u64>,
}

impl Identifier<Option<SubscriptionId>> for AlorOrderBookMessage {
    fn id(&self) -> Option<SubscriptionId> {
        None
    }
}

impl<InstrumentKey>
    From<(
        ExchangeId,
        InstrumentKey,
        super::AlorPayload<AlorOrderBookMessage>,
    )> for MarketIter<InstrumentKey, OrderBookEvent>
{
    fn from(
        (exchange, instrument, payload): (
            ExchangeId,
            InstrumentKey,
            super::AlorPayload<AlorOrderBookMessage>,
        ),
    ) -> Self {
        let Some(book) = payload.data else {
            return Self(Vec::new());
        };
        let time_exchange = book
            .ms_timestamp
            .or(book.timestamp)
            .and_then(|ms| DateTime::from_timestamp_millis(ms as i64))
            .unwrap_or_else(Utc::now);

        let to_decimal = |value: f64| Decimal::from_f64(value).unwrap_or_default();

        let order_book = OrderBook::new(
            book.ms_timestamp
                .unwrap_or_else(|| book.timestamp.unwrap_or_default()),
            Some(time_exchange),
            book.bids
                .into_iter()
                .map(|level| Level::new(to_decimal(level.price), to_decimal(level.amount))),
            book.asks
                .into_iter()
                .map(|level| Level::new(to_decimal(level.price), to_decimal(level.amount))),
        );

        let kind = if book.snapshot {
            OrderBookEvent::Snapshot(order_book)
        } else {
            OrderBookEvent::Update(order_book)
        };

        Self(vec![Ok(MarketEvent {
            time_exchange,
            time_received: Utc::now(),
            exchange,
            instrument,
            kind,
        })])
    }
}