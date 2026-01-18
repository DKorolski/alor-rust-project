use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use alor_scalping::strategy::{Bar, StrategyConfig, StrategyState, TradeLogger};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tracing::{debug, info, warn};

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
const DEFAULT_TRADE_LOG: &str = "paper_trades_1.csv";
const DEFAULT_START_CASH: f64 = 30_000.0;
const DEFAULT_FROM_DATE: &str = "2025-12-31";
const DEFAULT_HISTORY_BATCH_LIMIT: usize = 4999;
const DEFAULT_HISTORY_ONLY: bool = true;
const DEFAULT_HISTORY_MAX_GAP_MIN: i64 = 10_080;
const MOSCOW_OFFSET_HOURS: i32 = 3;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
    let trade_log = get_env_or_default("ALOR_TRADE_LOG", DEFAULT_TRADE_LOG);
    let history_batch_limit = std::env::var("ALOR_HISTORY_BATCH_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_HISTORY_BATCH_LIMIT);
    let history_only = std::env::var("ALOR_HISTORY_ONLY")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_HISTORY_ONLY);
    let history_max_gap_min = std::env::var("ALOR_HISTORY_MAX_GAP_MIN")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_HISTORY_MAX_GAP_MIN);

    let from_start = start_of_utc_day();
    let from_ts = std::env::var("ALOR_FROM_TS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| parse_date_env("ALOR_FROM_DATE"))
        .unwrap_or_else(|| parse_date(DEFAULT_FROM_DATE).unwrap_or_else(|| from_start.timestamp()));
    let to_ts = std::env::var("ALOR_TO_TS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| parse_date_env("ALOR_TO_DATE"));

    info!(
        "start portfolio={portfolio} symbol={symbol} exchange={exchange} group={instrument_group} tf={timeframe_sec} format={format} frequency={frequency_ms} skip_history={skip_history} split_adjust={split_adjust} from_ts={from_ts} to_ts={to_ts:?} history_batch_limit={history_batch_limit} history_only={history_only} history_max_gap_min={history_max_gap_min}"
    );

    let mut strategy = StrategyState::new(StrategyConfig::default(), DEFAULT_START_CASH);
    let mut trade_log = TradeLogger::new(&trade_log)?;

    let last_history_ts = fetch_history_batches(
        &access_token,
        &symbol,
        &exchange,
        &instrument_group,
        timeframe_sec,
        from_ts,
        false,
        split_adjust,
        &format,
        frequency_ms,
        &mut strategy,
        &mut trade_log,
        to_ts,
        history_batch_limit,
        history_max_gap_min,
    )
    .await?;

    if history_only || to_ts.is_some() {
        return Ok(());
    }

    let live_from_ts = last_history_ts.unwrap_or(from_ts);
    let mut reconnect_delay = Duration::from_secs(1);
    loop {
        match run_live_stream(
            &access_token,
            &symbol,
            &exchange,
            &instrument_group,
            timeframe_sec,
            live_from_ts,
            true,
            split_adjust,
            &format,
            frequency_ms,
            &mut strategy,
            &mut trade_log,
            None,
        )
        .await
        {
            Ok(()) => {
                warn!("stream ended; reconnecting");
            }
            Err(error) => {
                warn!(?error, "stream error; reconnecting");
            }
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(30));
    }
}

async fn run_live_stream(
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
    strategy: &mut StrategyState,
    trade_log: &mut TradeLogger,
    to_ts: Option<i64>,
) -> anyhow::Result<()> {
    let (ws_data, _) = connect_async(WS_URL).await?;
    let (mut ws_sink, mut ws_stream) = ws_data.split();

    let (first_msg, subscribe_rt_ms) = subscribe_bars(
        &mut ws_sink,
        &mut ws_stream,
        access_token,
        symbol,
        exchange,
        instrument_group,
        timeframe_sec,
        from_ts,
        skip_history,
        split_adjust,
        format,
        frequency_ms,
    )
    .await?;

    info!(
        "SUBSCRIBE first message in {:.2} ms: {}",
        subscribe_rt_ms, first_msg
    );

    loop {
        let msg = match ws_stream.next().await {
            Some(msg) => msg?,
            None => return Ok(()),
        };

        match msg {
            Message::Text(txt) => {
                if let Ok(payload) = serde_json::from_str::<Value>(&txt) {
                    let bars = extract_bars(&payload);
                    for bar in bars {
                        if let Some(to_ts) = to_ts {
                            if bar.time.timestamp() >= to_ts {
                                info!("reached to_ts={}, stopping stream", to_ts);
                                return Ok(());
                            }
                        }
                        strategy.on_bar(bar, trade_log)?;
                    }
                }
            }
            Message::Ping(payload) => {
                ws_sink.send(Message::Pong(payload)).await?;
            }
            Message::Close(frame) => {
                info!(?frame, "ws close received");
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn fetch_history_batches(
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
    strategy: &mut StrategyState,
    trade_log: &mut TradeLogger,
    to_ts: Option<i64>,
    history_batch_limit: usize,
    history_max_gap_min: i64,
) -> anyhow::Result<Option<i64>> {
    let mut current_from_ts = from_ts;
    let mut last_seen_ts = None;

    loop {
        let (ws_data, _) = connect_async(WS_URL).await?;
        let (mut ws_sink, mut ws_stream) = ws_data.split();

        let (first_msg, subscribe_rt_ms) = subscribe_bars(
            &mut ws_sink,
            &mut ws_stream,
            access_token,
            symbol,
            exchange,
            instrument_group,
            timeframe_sec,
            current_from_ts,
            skip_history,
            split_adjust,
            format,
            frequency_ms,
        )
        .await?;

        let bars = read_history_batch(
            &mut ws_stream,
            first_msg,
            history_max_gap_min,
        )
        .await?;
        info!(
            "HISTORY batch from_ts={} bars={} rt_ms={:.2}",
            current_from_ts,
            bars.len(),
            subscribe_rt_ms
        );

        if bars.is_empty() {
            warn!("HISTORY batch empty, stopping history fetch");
            return Ok(last_seen_ts);
        }

        for bar in &bars {
            let bar = *bar;
            if let Some(to_ts) = to_ts {
                if bar.time.timestamp() >= to_ts {
                    return Ok(Some(bar.time.timestamp()));
                }
            }
            strategy.on_bar(bar, trade_log)?;
            last_seen_ts = Some(bar.time.timestamp());
        }

        if bars.len() < history_batch_limit {
            return Ok(last_seen_ts);
        }

        let next_from_ts = last_seen_ts.unwrap_or(current_from_ts) + timeframe_sec;
        current_from_ts = next_from_ts;
    }
}

async fn read_history_batch(
    ws_stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    first_msg: Value,
    history_max_gap_min: i64,
) -> anyhow::Result<Vec<Bar>> {
    let mut bars = extract_bars(&first_msg);
    let mut idle_rounds = 0;
    let mut last_ts = bars.last().map(|bar| bar.time.timestamp());

    loop {
        match timeout(Duration::from_millis(500), ws_stream.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                if let Ok(payload) = serde_json::from_str::<Value>(&txt) {
                    let incoming = extract_bars(&payload);
                    for bar in incoming {
                        if let Some(prev_ts) = last_ts {
                            let diff_min = (bar.time.timestamp() - prev_ts) / 60;
                            if diff_min > history_max_gap_min {
                                info!(
                                    "HISTORY gap {} min exceeds max {}, stopping batch at {}",
                                    diff_min, history_max_gap_min, bar.time
                                );
                                return Ok(bars);
                            }
                        }
                        last_ts = Some(bar.time.timestamp());
                        bars.push(bar);
                    }
                }
                idle_rounds = 0;
            }
            Ok(Some(Ok(Message::Ping(_)))) => {
                idle_rounds = 0;
            }
            Ok(Some(Ok(Message::Close(_)))) => {
                break;
            }
            Ok(Some(Ok(_))) => {
                idle_rounds = 0;
            }
            Ok(Some(Err(error))) => return Err(error.into()),
            Ok(None) => break,
            Err(_) => {
                idle_rounds += 1;
                if idle_rounds >= 2 {
                    break;
                }
            }
        }
    }

    Ok(bars)
}

fn extract_bars(payload: &Value) -> Vec<Bar> {
    let mut bars = if let Some(data) = payload.get("data") {
        extract_bars_from_value(data)
    } else {
        extract_bars_from_value(payload)
    };

    bars.sort_by_key(|bar| bar.time.timestamp());
    if let (Some(first), Some(last)) = (bars.first(), bars.last()) {
        debug!(
            "bars batch size={} first_ts={} last_ts={}",
            bars.len(),
            first.time,
            last.time
        );
    }
    bars
}

fn extract_bars_from_value(value: &Value) -> Vec<Bar> {
    match value {
        Value::Array(items) => items.iter().filter_map(parse_bar).collect(),
        Value::Object(map) => {
            if let Some(bars_value) = map.get("bars") {
                return extract_bars_from_value(bars_value);
            }
            parse_bar(value).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn parse_bar(value: &Value) -> Option<Bar> {
    match value {
        Value::Array(items) => parse_bar_from_array(items),
        Value::Object(_) => parse_bar_from_object(value),
        _ => None,
    }
}

fn parse_bar_from_array(items: &[Value]) -> Option<Bar> {
    if items.len() < 5 {
        return None;
    }

    let time = parse_time(&items[0])?;
    let open = parse_f64(&items[1])?;
    let high = parse_f64(&items[2])?;
    let low = parse_f64(&items[3])?;
    let close = parse_f64(&items[4])?;

    Some(Bar {
        time,
        open,
        high,
        low,
        close,
    })
}

fn parse_bar_from_object(value: &Value) -> Option<Bar> {
    let time = value
        .get("time")
        .or_else(|| value.get("timestamp"))
        .or_else(|| value.get("t"))
        .and_then(parse_time)?;
    let open = value
        .get("open")
        .or_else(|| value.get("o"))
        .and_then(parse_f64)?;
    let high = value
        .get("high")
        .or_else(|| value.get("h"))
        .and_then(parse_f64)?;
    let low = value
        .get("low")
        .or_else(|| value.get("l"))
        .and_then(parse_f64)?;
    let close = value
        .get("close")
        .or_else(|| value.get("c"))
        .and_then(parse_f64)?;

    Some(Bar {
        time,
        open,
        high,
        low,
        close,
    })
}

fn parse_time(value: &Value) -> Option<DateTime<FixedOffset>> {
    match value {
        Value::Number(num) => num.as_i64().and_then(ts_to_datetime),
        Value::String(s) => {
            if let Ok(ts) = s.parse::<i64>() {
                return ts_to_datetime(ts);
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&moscow_offset()));
            }
            None
        }
        _ => None,
    }
}

fn ts_to_datetime(ts: i64) -> Option<DateTime<FixedOffset>> {
    let ts = if ts > 1_000_000_000_000 {
        ts / 1000
    } else {
        ts
    };
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.with_timezone(&moscow_offset()))
}

fn parse_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(num) => num.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
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

async fn get_access_token() -> anyhow::Result<String> {
    let refresh = std::env::var("ALOR_REFRESH_TOKEN")
        .map(|v| v.trim_matches('"').trim().to_string())
        .unwrap_or_default();

    if refresh.is_empty() {
        anyhow::bail!("Нужно задать ALOR_REFRESH_TOKEN в окружении");
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

    info!("get_access_token_ms={dt:.2}");

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
) -> anyhow::Result<(Value, f64)> {
    let guid = new_guid();
    let msg = serde_json::json!({
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

    anyhow::bail!("BarsGetAndSubscribe failed: {resp}");
}

async fn read_until_guid(
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    guid: &str,
    timeout_dur: Duration,
) -> anyhow::Result<Value> {
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
        anyhow::bail!("WS stream ended before response");
    };

    match timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => anyhow::bail!("WS subscribe timeout"),
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

fn moscow_offset() -> FixedOffset {
    FixedOffset::east_opt(MOSCOW_OFFSET_HOURS * 3600)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())
}

fn parse_date_env(key: &str) -> Option<i64> {
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(parse_date)
}

fn parse_date(value: &str) -> Option<i64> {
    let value = value.trim();
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    let dt = Utc
        .with_ymd_and_hms(parsed.year(), parsed.month(), parsed.day(), 0, 0, 0)
        .single()?;
    Some(dt.timestamp())
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