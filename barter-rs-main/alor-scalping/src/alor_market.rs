use crate::engine::{EngineEvent, TradeSide};
use barter_data::streams::reconnect::{Event, stream::ReconnectingStream};
use barter_data::{
    event::{DataKind, MarketEvent},
    exchange::alor::Alor,
    instrument::MarketInstrumentData,
    streams::{Streams, consumer::MarketStreamResult},
    subscription::{book::OrderBooksL2, trade::PublicTrades},
};
use barter_instrument::instrument::market_data::kind::MarketDataInstrumentKind;
use futures_util::{StreamExt, stream::BoxStream};
use num_traits::ToPrimitive;
use serde_json::Value;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

const OAUTH_URL: &str = "https://oauth.alor.ru/refresh";
const DEFAULT_TEST_REFRESH_TOKEN: &str = "";
const DEFAULT_DEPTH: usize = 10;

pub async fn alor_sih6_stream() -> BoxStream<'static, EngineEvent> {
    let access_token = load_access_token().await.expect("failed to load token");
    unsafe {
        std::env::set_var("ALOR_ACCESS_TOKEN", &access_token);
    }

    let instrument = MarketInstrumentData {
        key: "sih6".to_string(),
        name_exchange: barter_instrument::instrument::name::InstrumentNameExchange::new("SIH6"),
        kind: MarketDataInstrumentKind::Spot,
    };

    let streams = Streams::<MarketStreamResult<String, DataKind>>::builder_multi()
        .add(Streams::<PublicTrades>::builder().subscribe([(
            Alor::default(),
            instrument.clone(),
            PublicTrades,
        )]))
        .add(Streams::<OrderBooksL2>::builder().subscribe([(
            Alor::default(),
            instrument,
            OrderBooksL2,
        )]))
        .init()
        .await
        .expect("failed to initialise streams");

    let stream = streams
        .select_all()
        .with_error_handler(|error| warn!(?error, "MarketStream generated error"))
        .filter_map(|event| async move {
            match event {
                Event::Item(market_event) => map_event(market_event),
                Event::Reconnecting(exchange) => {
                    warn!(?exchange, "MarketStream reconnecting");
                    None
                }
            }
        });

    Box::pin(stream)
}

fn map_event(event: MarketEvent<String, DataKind>) -> Option<EngineEvent> {
    match event.kind {
        DataKind::Trade(trade) => Some(EngineEvent::Trade {
            ts: event.time_exchange,
            price: trade.price,
            qty: trade.amount,
            side: match trade.side {
                barter_instrument::Side::Buy => TradeSide::Buy,
                barter_instrument::Side::Sell => TradeSide::Sell,
            },
        }),
        DataKind::OrderBook(orderbook_event) => {
            use barter_data::subscription::book::OrderBookEvent;
            let snapshot = match orderbook_event {
                OrderBookEvent::Snapshot(snapshot) => snapshot,
                OrderBookEvent::Update(update) => update,
            };

            let bids = snapshot
                .bids()
                .levels()
                .iter()
                .take(DEFAULT_DEPTH)
                .filter_map(|level| Some((level.price.to_f64()?, level.amount.to_f64()?)))
                .collect::<Vec<_>>();

            let asks = snapshot
                .asks()
                .levels()
                .iter()
                .take(DEFAULT_DEPTH)
                .filter_map(|level| Some((level.price.to_f64()?, level.amount.to_f64()?)))
                .collect::<Vec<_>>();

            if bids.is_empty() || asks.is_empty() {
                debug!("dropping orderbook with empty sides");
                return None;
            }

            Some(EngineEvent::OrderBookL2 {
                ts: event.time_exchange,
                bids,
                asks,
            })
        }
        unexpected => {
            warn!(?unexpected, "unexpected data kind in market stream");
            None
        }
    }
}

async fn fetch_access_token(refresh_token: &str) -> Result<String, Box<dyn std::error::Error>> {
    let refresh_token = refresh_token.trim_matches('"').trim();

    let response = reqwest::Client::new()
        .post(OAUTH_URL)
        .query(&[("token", refresh_token)])
        .send()
        .await?
        .error_for_status()?;

    let payload: Value = response.json().await?;

    let access_token = payload
        .get("AccessToken")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("AccessToken not in response: {payload}"),
            )
        })?;

    Ok(access_token.to_string())
}

pub async fn load_access_token() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(token) = std::env::var("ALOR_ACCESS_TOKEN") {
        info!("Using ALOR_ACCESS_TOKEN from environment");
        return Ok(token.trim_matches('"').trim().to_string());
    }

    let refresh_token = std::env::var("ALOR_REFRESH_TOKEN")
        .unwrap_or_else(|_| DEFAULT_TEST_REFRESH_TOKEN.to_string());

    let refresh_token = refresh_token.trim_matches('"').trim();

    info!("Exchanging ALOR_REFRESH_TOKEN for ALOR_ACCESS_TOKEN");
    fetch_access_token(&refresh_token).await
}

pub fn load_local_env() {
    if let Ok(env) = std::fs::read_to_string(".env") {
        for line in env
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        {
            if let Some((key, value)) = line.split_once('=') {
                let trimmed_value = value.trim_matches('"').trim();
                let trimmed_key = key.trim();
                if std::env::var_os(trimmed_key).is_none() {
                    unsafe {
                        std::env::set_var(trimmed_key, trimmed_value);
                    }
                }
            }
        }
    }
}

pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_ansi(cfg!(debug_assertions))
        .init();

    debug!("logging initialised");
}