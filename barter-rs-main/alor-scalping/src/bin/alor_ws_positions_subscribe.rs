use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

const OAUTH_URL: &str = "https://oauth.alor.ru/refresh";
const WS_URL: &str = "wss://api.alor.ru/ws";

const DEFAULT_PORTFOLIO: &str = "7502T0U";
const DEFAULT_SYMBOL: &str = "IMOEXF";
const DEFAULT_EXCHANGE: &str = "MOEX";
const DEFAULT_SKIP_HISTORY: bool = false;
const DEFAULT_FORMAT: &str = "Simple";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let access_token = get_access_token().await?;
    let portfolio = get_env_or_default("ALOR_PORTFOLIO", DEFAULT_PORTFOLIO);
    let symbol_filter = get_env_or_default("ALOR_SYMBOL", DEFAULT_SYMBOL);
    let exchange = get_env_or_default("ALOR_EXCHANGE", DEFAULT_EXCHANGE);
    let skip_history = std::env::var("ALOR_SKIP_HISTORY")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_SKIP_HISTORY);
    let format = get_env_or_default("ALOR_FORMAT", DEFAULT_FORMAT);

    println!("Используем:");
    println!("  portfolio: {portfolio}");
    println!("  symbol   : {symbol_filter}");
    println!("  exchange : {exchange}");
    println!("  format   : {format}");
    println!("  skipHist : {skip_history}");

    let (ws_data, _) = connect_async(WS_URL).await?;
    let (mut ws_sink, mut ws_stream) = ws_data.split();

    let (first_msg, subscribe_rt_ms) = subscribe_positions(
        &mut ws_sink,
        &mut ws_stream,
        &access_token,
        &portfolio,
        &exchange,
        skip_history,
        &format,
    )
    .await?;

    println!(
        "<< SUBSCRIBE FIRST: {} (dt={subscribe_rt_ms:.2} ms)",
        first_msg
    );

    if let Some(data) = first_msg.get("data") {
        println!("first.position: {data}");
        if data.get("symbol").and_then(Value::as_str) == Some(symbol_filter.as_str()) {
            let qty = data.get("qty").and_then(Value::as_f64).unwrap_or(0.0);
            let open = data.get("open").and_then(Value::as_f64).unwrap_or(0.0);
            let volume = data.get("volume").and_then(Value::as_f64).unwrap_or(0.0);
            println!("symbol.qty: {symbol_filter} qty={qty} open={open} volume={volume}");
        }
    }

    Ok(())
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with_ansi(cfg!(debug_assertions))
        .init();
}

fn get_env_or_default(key: &str, default: &str) -> String {
    std::env::var(key)
        .map(|v| v.trim_matches('"').trim().to_string())
        .unwrap_or_else(|_| default.to_string())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" => Some(true),
        "0" | "false" | "no" | "n" => Some(false),
        _ => None,
    }
}

async fn get_access_token() -> Result<String, Box<dyn std::error::Error>> {
    let refresh = std::env::var("ALOR_REFRESH_TOKEN")
        .map(|v| v.trim_matches('"').trim().to_string())
        .unwrap_or_default();

    if refresh.is_empty() {
        return Err("Нужно задать ALOR_REFRESH_TOKEN в окружении".into());
    }

    let t0 = Instant::now();
    let response = reqwest::Client::new()
        .post(OAUTH_URL)
        .query(&[("token", refresh)])
        .send()
        .await?;
    let dt = duration_ms(t0.elapsed());

    let payload: Value = response.error_for_status()?.json().await?;
    let access = payload
        .get("AccessToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("AccessToken not in response: {payload}"))?;

    println!("[TIMING] get_access_token_ms     : {dt:.2} ms (можно не учитывать)");

    Ok(access.trim().to_string())
}

async fn subscribe_positions(
    sink: &mut (impl futures_util::sink::Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    access_token: &str,
    portfolio: &str,
    exchange: &str,
    skip_history: bool,
    format: &str,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let guid = new_guid();
    let msg = json!({
        "opcode": "PositionsGetAndSubscribeV2",
        "exchange": exchange,
        "portfolio": portfolio,
        "skipHistory": skip_history,
        "format": format,
        "guid": guid,
        "token": access_token,
    });

    let payload = serde_json::to_string(&msg)?;
    let t0 = Instant::now();
    sink.send(Message::Text(payload.into())).await?;

    let resp = read_until_guid(stream, &guid, Duration::from_secs(5)).await?;
    let dt = duration_ms(t0.elapsed());

    if resp.get("httpCode").and_then(Value::as_i64) == Some(200) {
        return Ok((resp, dt));
    }

    if resp.get("data").is_some() {
        return Ok((resp, dt));
    }

    Err(format!("PositionsGetAndSubscribeV2 failed: {resp}").into())
}

async fn read_until_guid(
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    guid: &str,
    timeout_dur: Duration,
) -> Result<Value, Box<dyn std::error::Error>> {
    let fut = async move {
        while let Some(msg) = stream.next().await {
            let msg = msg?;
            if let Message::Text(txt) = msg {
                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                    if guid_of(&val).as_deref() == Some(guid) {
                        return Ok(val);
                    }
                    if val.get("httpCode").is_some() {
                        return Ok(val);
                    }
                }
            }
        }
        Err("WS stream ended before response".into())
    };

    match timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => Err("WS subscribe timeout".into()),
    }
}

fn guid_of(event: &Value) -> Option<String> {
    event
        .get("requestGuid")
        .or_else(|| event.get("guid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn duration_ms(dur: Duration) -> f64 {
    dur.as_secs_f64() * 1000.0
}

fn new_guid() -> String {
    use rand::{Rng, distr::Alphanumeric};

    let rng = rand::rng();
    rng.sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}
