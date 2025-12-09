use crate::{
    ExchangeWsStream, NoInitialSnapshots,
    exchange::{Connector, ExchangeServer, StreamSelector},
    instrument::InstrumentData,
    subscriber::{WebSocketSubscriber, validator::WebSocketSubValidator},
    subscription::{Map, book::OrderBooksL2, trade::PublicTrades},
    transformer::stateless::StatelessTransformer,
};
use barter_instrument::exchange::ExchangeId;
use barter_integration::{error::SocketError, protocol::websocket::WsMessage};
use serde::de::{Error, Unexpected};
use std::{fmt::Debug, marker::PhantomData};
use url::Url;

/// WebSocket payload wrapper returned by Alor.
///
/// Responses carry data in the `data` field and echo the subscription `guid`.
/// Handshake acks use `requestGuid` instead of `guid` and may omit `data`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AlorPayload<T> {
    #[serde(default = "default_none")]
    pub data: Option<T>,
    #[serde(default, rename = "guid")]
    pub guid: Option<String>,
    #[serde(default, rename = "requestGuid")]
    pub request_guid: Option<String>,
}

fn default_none<T>() -> Option<T> {
    None
}

impl<T> crate::Identifier<Option<barter_integration::subscription::SubscriptionId>>
    for AlorPayload<T>
{
    fn id(&self) -> Option<barter_integration::subscription::SubscriptionId> {
        self.guid
            .as_ref()
            .or(self.request_guid.as_ref())
            .map(|guid| guid.clone().into())
    }
}

pub mod book;
pub mod channel;
pub mod market;
pub mod subscription;
pub mod trade;

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default)]
pub struct Alor<Server = AlorServer> {
    server: PhantomData<Server>,
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default)]
pub struct AlorServer;

impl ExchangeServer for AlorServer {
    const ID: ExchangeId = ExchangeId::Other;

    fn websocket_url() -> &'static str {
        "wss://api.alor.ru/ws"
    }
}

impl<Server> Connector for Alor<Server>
where
    Server: ExchangeServer,
{
    const ID: ExchangeId = Server::ID;
    type Channel = channel::AlorChannel;
    type Market = market::AlorMarket;
    type Subscriber = WebSocketSubscriber;
    type SubValidator = WebSocketSubValidator;
    type SubResponse = subscription::AlorResponse;

    fn url() -> Result<Url, SocketError> {
        Url::parse(Server::websocket_url()).map_err(SocketError::UrlParse)
    }

    fn requests(
        exchange_subs: Vec<crate::exchange::subscription::ExchangeSub<Self::Channel, Self::Market>>,
    ) -> Vec<WsMessage> {
        let token = std::env::var("ALOR_ACCESS_TOKEN").unwrap_or_default();

        exchange_subs
            .into_iter()
            .map(|sub| {
                let guid = format!("{}|{}", sub.channel.as_ref(), sub.market.as_ref());

                WsMessage::text(
                    serde_json::json!({
                        "opcode": sub.channel.as_ref(),
                        "token": token,
                        "exchange": sub.market.exchange,
                        "code": sub.market.code,
                        "depth": 10,
                        "format": "Simple",
                        "frequency": 0,
                        "guid": guid,
                    })
                    .to_string(),
                )
            })
            .collect()
    }

    fn expected_responses<InstrumentKey>(_: &Map<InstrumentKey>) -> usize {
        0
    }
}

impl<Instrument, Server> StreamSelector<Instrument, PublicTrades> for Alor<Server>
where
    Instrument: InstrumentData,
    Server: ExchangeServer + Debug + Send + Sync,
{
    type SnapFetcher = NoInitialSnapshots;
    type Stream = ExchangeWsStream<
        StatelessTransformer<Self, Instrument::Key, PublicTrades, AlorPayload<trade::AlorTrade>>,
    >;
}

impl<Instrument, Server> StreamSelector<Instrument, OrderBooksL2> for Alor<Server>
where
    Instrument: InstrumentData,
    Server: ExchangeServer + Debug + Send + Sync,
{
    type SnapFetcher = NoInitialSnapshots;
    type Stream = ExchangeWsStream<
        StatelessTransformer<
            Self,
            Instrument::Key,
            OrderBooksL2,
            AlorPayload<book::AlorOrderBookMessage>,
        >,
    >;
}

impl<'de, Server> serde::Deserialize<'de> for Alor<Server>
where
    Server: ExchangeServer,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let input = <&str as serde::Deserialize>::deserialize(deserializer)?;

        if input == Self::ID.as_str() {
            Ok(Self::default())
        } else {
            Err(Error::invalid_value(
                Unexpected::Str(input),
                &Self::ID.as_str(),
            ))
        }
    }
}

impl<Server> serde::Serialize for Alor<Server>
where
    Server: ExchangeServer,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(Self::ID.as_str())
    }
}