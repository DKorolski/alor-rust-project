use super::Alor;
use crate::{
    Identifier,
    subscription::{Subscription, book::OrderBooksL2, trade::PublicTrades},
};
use serde::Serialize;

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize)]
pub enum AlorChannel {
    OrderBook,
    Trades,
}

impl AlorChannel {
    pub const ORDERBOOK: Self = Self::OrderBook;
    pub const TRADES: Self = Self::Trades;
}

impl AsRef<str> for AlorChannel {
    fn as_ref(&self) -> &str {
        match self {
            Self::OrderBook => "OrderBookGetAndSubscribe",
            Self::Trades => "AllTradesGetAndSubscribe",
        }
    }
}

impl<Instrument> Identifier<AlorChannel> for Subscription<Alor, Instrument, OrderBooksL2> {
    fn id(&self) -> AlorChannel {
        AlorChannel::ORDERBOOK
    }
}

impl<Instrument> Identifier<AlorChannel> for Subscription<Alor, Instrument, PublicTrades> {
    fn id(&self) -> AlorChannel {
        AlorChannel::TRADES
    }
}