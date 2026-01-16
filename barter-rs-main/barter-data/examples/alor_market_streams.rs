use barter_data::{
    event::DataKind,
    exchange::alor::Alor,
    instrument::MarketInstrumentData,
    streams::{Streams, consumer::MarketStreamResult, reconnect::stream::ReconnectingStream},
    subscription::{book::OrderBooksL2, trade::PublicTrades},
};
use barter_instrument::{
    instrument::market_data::kind::MarketDataInstrumentKind,
    instrument::name::InstrumentNameExchange,
};
use futures_util::StreamExt;
use tracing::{info, warn};

const OAUTH_URL: &str = "https://oauth.alor.ru/refresh";
// Sandbox refresh token supplied for quick-start testing. Override it in your environment for
// production or personal credentials.
const DEFAULT_TEST_REFRESH_TOKEN: &str = "";

#[rustfmt::skip]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load local environment variables (eg/ from .env)
    load_local_env();
    init_logging();

    // Acquire an access token using either a provided ALOR_ACCESS_TOKEN or by exchanging a
    // long-lived ALOR_REFRESH_TOKEN.
    let access_token = load_access_token().await?;
    unsafe {
        std::env::set_var("ALOR_ACCESS_TOKEN", &access_token);
    }

    // Alor subscriptions use the exchange symbol directly
    let instrument = MarketInstrumentData {
        key: "sih6".to_string(),
        name_exchange: InstrumentNameExchange::new("SIH6"),
        // MarketDataInstrumentKind::Spot is accepted by the subscription validator for ExchangeId::Other
        kind: MarketDataInstrumentKind::Spot,
    };

    // Initialise combined market data streams for Alor OrderBooksL2 and PublicTrades
    let streams = Streams::<MarketStreamResult<String, DataKind>>::builder_multi()
        .add(Streams::<PublicTrades>::builder()
            .subscribe([(Alor::default(), instrument.clone(), PublicTrades)]))
        .add(Streams::<OrderBooksL2>::builder()
            .subscribe([(Alor::default(), instrument, OrderBooksL2)]))
        .init()
        .await?;

    let mut joined_stream = streams
        .select_all()
        .with_error_handler(|error| warn!(?error, "MarketStream generated error"));

    while let Some(event) = joined_stream.next().await {
        info!("{event:?}");
    }

    Ok(())
}

async fn fetch_access_token(refresh_token: &str) -> Result<String, Box<dyn std::error::Error>> {
    let refresh_token = refresh_token.trim_matches('"').trim();

    let response = reqwest::Client::new()
        .post(OAUTH_URL)
        .query(&[("token", refresh_token)])
        .send()
        .await?
        .error_for_status()?;

    let payload: serde_json::Value = response.json().await?;

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

async fn load_access_token() -> Result<String, Box<dyn std::error::Error>> {
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

/// Minimal .env loader to avoid the `dotenvy` dependency in restricted environments.
fn load_local_env() {
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

// Initialise an INFO `Subscriber` for `Tracing` Json logs and install it as the global default.
fn init_logging() {
    tracing_subscriber::fmt()
        // Filter messages based on the INFO
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        // Disable colours on release builds
        .with_ansi(cfg!(debug_assertions))
        // Enable Json formatting
        .json()
        // Install this Tracing subscriber as global default
        .init();
}