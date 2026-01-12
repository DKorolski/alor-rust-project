use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, TimeZone, Utc};
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
const DEFAULT_INSTRUMENT_GROUP: &str = "RFUD";
const DEFAULT_TIMEFRAME_SEC: i64 = 60;
const DEFAULT_SKIP_HISTORY: bool = false;
const DEFAULT_SPLIT_ADJUST: bool = true;
const DEFAULT_FORMAT: &str = "Simple";
const DEFAULT_FREQUENCY_MS: i64 = 250;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let access_token = get_access_token().await?;
    let portfolio = get_env_or_default("ALOR_PORTFOLIO", DEFAULT_PORTFOLIO);
    let symbol = get_env_or_default("ALOR_SYMBOL", DEFAULT_SYMBOL);
    let exchange = get_env_or_default("ALOR_EXCHANGE", DEFAULT_EXCHANGE);
    let instrument_group = get_env_or_default("ALOR_INSTRUMENT_GROUP", DEFAULT_INSTRUMENT_GROUP);
    let timeframe_sec = std::env::var("ALOR_TF")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEFRAME_SEC);
    let format = get_env_or_default("ALOR_FORMAT", DEFAULT_FORMAT);
    let frequency_ms = std::env::var("ALOR_FREQUENCY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_FREQUENCY_MS);
    let skip_history = std::env::var("ALOR_SKIP_HISTORY")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_SKIP_HISTORY);
    let split_adjust = std::env::var("ALOR_SPLIT_ADJUST")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_SPLIT_ADJUST);

    let from_start = start_of_utc_day();
    let from_ts = from_start.timestamp();

    println!("Используем:");
    println!("  portfolio: {portfolio}");
    println!("  symbol   : {symbol}");
    println!("  exchange : {exchange}");
    println!("  group    : {instrument_group}");
    println!("  tf_sec   : {timeframe_sec}");
    println!("  format   : {format}");
    println!("  freq_ms  : {frequency_ms}");
    println!("  skipHist : {skip_history}");
    println!("  splitAdj : {split_adjust}");
    println!("  from_utc : {from_start} (ts={from_ts})");

    let (ws_data, _) = connect_async(WS_URL).await?;
    let (mut ws_sink, mut ws_stream) = ws_data.split();

    let (first_msg, subscribe_rt_ms) = subscribe_bars(
        &mut ws_sink,
        &mut ws_stream,
        &access_token,
        &symbol,
        &exchange,
        &instrument_group,
        timeframe_sec,
        from_ts,
        skip_history,
        split_adjust,
        &format,
        frequency_ms,
    )
    .await?;

    println!(
        "<< SUBSCRIBE FIRST: {} (dt={subscribe_rt_ms:.2} ms)",
        first_msg
    );

    if let Some(data) = first_msg.get("data") {
        println!("first.bar: {data}");
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

async fn subscribe_bars(
    sink: &mut (impl futures_util::sink::Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    access_token: &str,
    symbol: &str,
    exchange: &str,
    instrument_group: &str,
    timeframe_sec: i64,
    from_ts: i64,
    skip_history: bool,
    split_adjust: bool,
    format: &str,
    frequency_ms: i64,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let guid = new_guid();
    let msg = json!({
        "opcode": "BarsGetAndSubscribe",
        "exchange": exchange,
        "code": symbol,
        "instrumentGroup": instrument_group,
        "tf": timeframe_sec,
        "from": from_ts,
        "skipHistory": skip_history,
        "splitAdjust": split_adjust,
        "format": format,
        "frequency": frequency_ms,
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

    Err(format!("BarsGetAndSubscribe failed: {resp}").into())
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

fn start_of_utc_day() -> DateTime<Utc> {
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" => Some(true),
        "0" | "false" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn new_guid() -> String {
    use rand::{Rng, distr::Alphanumeric};

    let rng = rand::rng();
    rng.sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}