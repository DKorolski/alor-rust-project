use std::collections::HashSet;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use futures_util::{sink::Sink, stream::Stream};
use rand::{Rng, distr::Alphanumeric};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tracing::error;

const OAUTH_URL: &str = "https://oauth.alor.ru/refresh";
const CWS_URL: &str = "wss://api.alor.ru/cws";
const WS_URL: &str = "wss://api.alor.ru/ws";

const DEFAULT_PORTFOLIO: &str = "7502T0U";
const DEFAULT_SYMBOL: &str = "IMOEXF";
const DEFAULT_EXCHANGE: &str = "MOEX";
const DEFAULT_INSTRUMENT_GROUP: &str = "MOEX";
const DEFAULT_PRICE: f64 = 2700.0;
const DEFAULT_QTY: i32 = 1;
const DEFAULT_STOP_OFFSET: f64 = 50.0;
const UPDATE_DELTA: f64 = 10.0;
const TIME_IN_FORCE: &str = "BookOrCancel";
const DEFAULT_STOP_CONDITION: &str = "More";
const ALLOW_MARGIN: bool = true;
const DEFAULT_ACTIVATE: bool = true;
const DEFAULT_STOP_ORDER_STATUSES: &[&str] =
    &["working", "canceled", "rejected", "filled"];

const TERMINAL_STATUSES: &[&str] = &[
    "canceled",
    "cancelled",
    "rejected",
    "filled",
    "expired",
    "done",
    "completed",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let access_token = get_access_token().await?;
    let portfolio = get_env_or_default("ALOR_PORTFOLIO", DEFAULT_PORTFOLIO);
    let symbol = get_env_or_default("ALOR_SYMBOL", DEFAULT_SYMBOL);
    let exchange = get_env_or_default("ALOR_EXCHANGE", DEFAULT_EXCHANGE);
    let instrument_group = get_env_or_default("ALOR_INSTRUMENT_GROUP", DEFAULT_INSTRUMENT_GROUP);
    let price: f64 = std::env::var("ALOR_PRICE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PRICE);
    let qty: i32 = std::env::var("ALOR_QTY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_QTY);
    let update_delta: f64 = std::env::var("ALOR_UPDATE_DELTA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(UPDATE_DELTA);
    let stop_offset: f64 = std::env::var("ALOR_STOP_OFFSET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_STOP_OFFSET);
    let stop_condition = get_env_or_default("ALOR_STOP_CONDITION", DEFAULT_STOP_CONDITION);
    let activate = std::env::var("ALOR_ACTIVATE")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_ACTIVATE);
    let stop_statuses = get_env_list("ALOR_STOP_ORDER_STATUSES")
        .unwrap_or_else(|| DEFAULT_STOP_ORDER_STATUSES.iter().map(|s| s.to_string()).collect());

    let stop_price = price - stop_offset;

    let tag: String = new_guid().chars().take(10).collect();
    let comment = format!("ws-stoplimit-speed:{tag}");

    println!("Используем:");
    println!("  portfolio: {portfolio}");
    println!("  symbol   : {symbol}");
    println!("  exchange : {exchange}");
    println!("  group    : {instrument_group}");
    println!("  price    : {price}");
    println!("  stop_px  : {stop_price}");
    println!("  qty      : {qty}");
    println!("  comment  : {comment}");
    println!("  tif      : {TIME_IN_FORCE}");
    println!("  cond     : {stop_condition}");
    println!("  activate : {activate}");
    println!("  statuses : {}", stop_statuses.join(","));

    let (ws_data, _) = connect_async(WS_URL).await?;
    let (mut ws_sink, ws_stream) = ws_data.split();
    let ws_router = WsRouter::new(ws_stream);

    let (cws_stream, _) = connect_async(CWS_URL).await?;
    let (mut cws_sink, mut cws_stream) = cws_stream.split();

    let mut subscribe_rx = ws_router.subscribe();
    let subscribe_guid = new_guid();
    let (first_msg, subscribe_rt_ms) = subscribe_orders(
        &mut ws_sink,
        &mut subscribe_rx,
        &subscribe_guid,
        &access_token,
        &portfolio,
        &exchange,
    )
    .await?;
    println!(
        "<< SUBSCRIBE FIRST: {} (dt={subscribe_rt_ms:.2} ms)",
        first_msg
    );

    let mut stop_subscribe_rx = ws_router.subscribe();
    let stop_subscribe_guid = new_guid();
    let (stop_first_msg, stop_subscribe_rt_ms) = subscribe_stop_orders(
        &mut ws_sink,
        &mut stop_subscribe_rx,
        &stop_subscribe_guid,
        &access_token,
        &portfolio,
        &exchange,
        &stop_statuses,
    )
    .await?;
    println!(
        "<< STOP SUBSCRIBE FIRST: {} (dt={stop_subscribe_rt_ms:.2} ms)",
        stop_first_msg
    );

    let (auth_resp, auth_dt_ms) =
        authorize_cws(&mut cws_sink, &mut cws_stream, &access_token).await?;
    println!("<< AUTH RESP: {} (dt={auth_dt_ms:.2} ms)", auth_resp);

    // CREATE
    let mut ws_create_rx = ws_router.subscribe();
    let (create_resp, create_ack_ms) = create_stop_limit_order(
        &mut cws_sink,
        &mut cws_stream,
        &portfolio,
        &symbol,
        &exchange,
        &instrument_group,
        price,
        stop_price,
        &stop_condition,
        qty,
        activate,
        &comment,
    )
    .await?;
    println!("<< CREATE RESP: {} (dt={create_ack_ms:.2} ms)", create_resp);
    if !is_http_ok(&create_resp) {
        println!("CWS error on create: {}", cws_error_message(&create_resp));
        return Ok(());
    }

    let order_number = order_id_from_cws(&create_resp)
        .ok_or_else(|| anyhow::anyhow!("orderNumber missing in create response"))?;
    let rec1 = wait_order_event(
        &mut ws_create_rx,
        WaitOpts {
            order_id: Some(order_number.clone()),
            order_ids: None,
            comment: Some(comment.clone()),
            predicate: None,
            timeout: Duration::from_secs(1),
            first_msg: Some(stop_first_msg.clone()),
        },
    )
    .await;

    let wait_first_ms = rec1
        .as_ref()
        .map(|(_, dt)| *dt)
        .unwrap_or_else(|| duration_ms(Duration::from_secs(1)));
    let create_total_ms = create_ack_ms + wait_first_ms;

    println!("\n========== AFTER CREATE ==========");
    println!("orderNumber (ack): {order_number}");
    println!("[TIMING] create_ack_ms           : {create_ack_ms:.2} ms");
    println!("[TIMING] create_to_first_ws_ms   : {create_total_ms:.2} ms");
    println!("[TIMING] wait_first_ws_ms        : {wait_first_ms:.2} ms");

    if let Some((rec, _)) = &rec1 {
        println!(
            "stream.status: {}",
            status_of(rec).unwrap_or("<none>".to_string())
        );
        println!("stream.rec   : {}", rec);
    } else {
        println!("WS события после create нет (стоп-лимит может быть не активирован).");
    }

    // UPDATE
    let mut ws_update_rx_old = ws_router.subscribe();
    let mut ws_update_rx_new = ws_router.subscribe();
    let new_stop_price = stop_price + update_delta;
    let new_price = new_stop_price;
    let (update_resp, update_ack_ms) = update_stop_limit_order(
        &mut cws_sink,
        &mut cws_stream,
        &portfolio,
        &symbol,
        &exchange,
        &instrument_group,
        &order_number,
        new_price,
        new_stop_price,
        &stop_condition,
        qty,
        activate,
        &comment,
    )
    .await?;
    println!("<< UPDATE RESP: {} (dt={update_ack_ms:.2} ms)", update_resp);
    if !is_http_ok(&update_resp) {
        println!("CWS error on update: {}", cws_error_message(&update_resp));
        return Ok(());
    }

    let updated_order_number = order_id_from_cws(&update_resp)
        .ok_or_else(|| anyhow::anyhow!("orderNumber missing in update response"))?;

    let symbol_filter = symbol.clone();
    let portfolio_filter = portfolio.clone();
    let price_filter = new_price;

    let (rec_old, wait_old_ms) = match wait_order_event(
        &mut ws_update_rx_old,
        WaitOpts {
            order_id: Some(order_number.clone()),
            order_ids: None,
            comment: None,
            predicate: Some(Box::new(|r| status_of(r).as_deref() == Some("canceled"))),
            timeout: Duration::from_secs(2),
            first_msg: None,
        },
    )
    .await
    {
        Some((rec, dt)) => (Some(rec), dt),
        None => (None, duration_ms(Duration::from_secs(2))),
    };

    let (rec_new, wait_new_ms) = match wait_order_event(
        &mut ws_update_rx_new,
        WaitOpts {
            order_id: Some(updated_order_number.clone()),
            order_ids: None,
            comment: None,
            predicate: Some(Box::new(move |r| {
                matches!(
                    (r.get("symbol"), r.get("portfolio"), r.get("stopPrice")),
                    (Some(Value::String(sym)), Some(Value::String(port)), stop_price_val)
                        if sym == &symbol_filter
                            && port == &portfolio_filter
                            && price_matches(stop_price_val, price_filter)
                )
            })),
            timeout: Duration::from_secs(2),
            first_msg: None,
        },
    )
    .await
    {
        Some((rec, dt)) => (Some(rec), dt),
        None => (None, duration_ms(Duration::from_secs(2))),
    };

    println!("\n========== AFTER UPDATE ==========");
    println!("orderNumber (ack/new): {updated_order_number}");
    println!("[TIMING] update_ack_ms          : {update_ack_ms:.2} ms");
    println!("[TIMING] wait_old_cancel_ms     : {wait_old_ms:.2} ms");
    println!("[TIMING] wait_new_evt_ms        : {wait_new_ms:.2} ms");

    println!("\nOld Order (before update):");
    println!("orderNumber: {order_number}");
    if let Some(rec) = &rec_old {
        println!(
            "Old Order status: {}",
            status_of(rec).unwrap_or("<none>".to_string())
        );
        println!("Old Order details: {rec}");
    } else {
        println!("Old Order cancel event: NOT FOUND (timeout)");
    }

    println!("\nNew Order (after update):");
    println!("orderNumber: {updated_order_number}");
    if let Some(rec) = &rec_new {
        let ws_id = order_id_from_ws(rec).unwrap_or_else(|| "<none>".to_string());
        println!(
            "New Order status: {}",
            status_of(rec).unwrap_or("<none>".to_string())
        );
        println!("New Order details: {rec}");
        if ws_id != updated_order_number {
            println!("WARNING: WS id {ws_id} != ack orderNumber {updated_order_number}");
        }
    } else {
        println!("New Order event: NOT FOUND (timeout)");
    }

    // DELETE
    let mut ws_delete_rx = ws_router.subscribe();
    let (delete_resp, delete_ack_ms) = delete_stop_limit_order(
        &mut cws_sink,
        &mut cws_stream,
        &portfolio,
        &exchange,
        &updated_order_number,
    )
    .await?;
    println!("<< DELETE RESP: {} (dt={delete_ack_ms:.2} ms)", delete_resp);
    if !is_http_ok(&delete_resp) {
        println!("CWS error on delete: {}", cws_error_message(&delete_resp));
        return Ok(());
    }
    let rec_d = wait_order_event(
        &mut ws_delete_rx,
        WaitOpts {
            order_id: Some(updated_order_number.clone()),
            order_ids: None,
            comment: None,
            predicate: Some(Box::new(|r| {
                status_of(r)
                    .map(|s| TERMINAL_STATUSES.contains(&s.as_str()))
                    .unwrap_or(false)
            })),
            timeout: Duration::from_secs(2),
            first_msg: None,
        },
    )
    .await;

    let wait_delete_ms = rec_d
        .as_ref()
        .map(|(_, dt)| *dt)
        .unwrap_or_else(|| duration_ms(Duration::from_secs(2)));

    println!("\n========== AFTER DELETE ==========");
    println!("[TIMING] delete_ack_ms          : {delete_ack_ms:.2} ms");
    println!("[TIMING] wait_delete_evt_ms     : {wait_delete_ms:.2} ms");

    if let Some((rec, _)) = &rec_d {
        println!(
            "stream.status: {}",
            status_of(rec).unwrap_or("<none>".to_string())
        );
        println!("stream.rec   : {rec}");
    } else {
        println!("Не дождались события по ордеру после delete (timeout).");
    }

    println!("\n========== DONE ==========");

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

fn get_env_list(key: &str) -> Option<Vec<String>> {
    let raw = std::env::var(key).ok()?;
    let items: Vec<String> = raw
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect();
    if items.is_empty() { None } else { Some(items) }
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

struct WsRouter {
    tx: broadcast::Sender<Value>,
}

impl WsRouter {
    fn new(
        mut stream: impl Stream<Item = Result<Message, WsError>> + Send + Unpin + 'static,
    ) -> Self {
        let (tx, _) = broadcast::channel(64);
        let tx_clone = tx.clone();

        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Text(txt)) => match serde_json::from_str::<Value>(&txt) {
                        Ok(val) => {
                            let _ = tx_clone.send(val);
                        }
                        Err(err) => {
                            error!(?err, "failed to parse WS message");
                        }
                    },
                    Ok(Message::Binary(bin)) => {
                        if let Ok(txt) = String::from_utf8(bin.to_vec()) {
                            if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                                let _ = tx_clone.send(val);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        error!(?err, "WS stream error");
                        break;
                    }
                }
            }
        });

        Self { tx }
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.tx.subscribe()
    }
}

struct WaitOpts {
    order_id: Option<String>,
    order_ids: Option<HashSet<String>>,
    comment: Option<String>,
    predicate: Option<Box<dyn Fn(&Value) -> bool + Send + Sync + 'static>>,
    timeout: Duration,
    first_msg: Option<Value>,
}

async fn wait_order_event(
    rx: &mut broadcast::Receiver<Value>,
    opts: WaitOpts,
) -> Option<(Value, f64)> {
    let t0 = Instant::now();

    if let Some(first) = opts.first_msg.clone() {
        if let Some(hit) = match_event(&first, &opts) {
            return Some((hit, duration_ms(t0.elapsed())));
        }
    }

    let deadline = opts.timeout;

    let wait_future = async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Some(hit) = match_event(&msg, &opts) {
                        return Some((hit, duration_ms(t0.elapsed())));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    error!(skipped, "WS receiver lagged");
                }
                Err(_) => return None,
            }
        }
    };

    timeout(deadline, wait_future).await.ok().flatten()
}

fn match_event(msg: &Value, opts: &WaitOpts) -> Option<Value> {
    for rec in extract_order_records(msg) {
        let rid = order_id_from_ws(&rec);

        if let Some(ids) = &opts.order_ids {
            if let Some(id) = &rid {
                if !ids.contains(id) {
                    continue;
                }
            } else {
                continue;
            }
        } else if let Some(target_id) = &opts.order_id {
            if rid.as_deref() != Some(target_id.as_str()) {
                continue;
            }
        }

        if let Some(comment) = &opts.comment {
            if rec.get("comment").and_then(Value::as_str) != Some(comment.as_str()) {
                continue;
            }
        }

        if let Some(pred) = &opts.predicate {
            if !(pred)(&rec) {
                continue;
            }
        }

        return Some(rec);
    }

    None
}

fn extract_order_records(msg: &Value) -> Vec<Value> {
    if let Some(data) = msg.get("data") {
        match data {
            Value::Array(arr) => arr.iter().filter(|v| v.is_object()).cloned().collect(),
            Value::Object(_) => vec![data.clone()],
            _ => Vec::new(),
        }
    } else if msg.is_array() {
        msg.as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .cloned()
            .collect()
    } else if msg.get("orderNumber").is_some()
        || msg.get("orderId").is_some()
        || msg.get("id").is_some()
    {
        vec![msg.clone()]
    } else {
        Vec::new()
    }
}

fn status_of(rec: &Value) -> Option<String> {
    rec.get("status")
        .and_then(|s| s.as_str())
        .map(|s| s.to_lowercase())
}

fn order_id_from_ws(event: &Value) -> Option<String> {
    if let Some(id) = event.get("id") {
        return id
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| id.as_i64().map(|i| i.to_string()));
    }

    if let Some(id) = event.get("orderNumber") {
        return id
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| id.as_i64().map(|i| i.to_string()));
    }

    if let Some(id) = event.get("orderId") {
        return id
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| id.as_i64().map(|i| i.to_string()));
    }

    None
}

fn order_id_from_cws(event: &Value) -> Option<String> {
    event.get("orderNumber").and_then(|v| {
        v.as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_i64().map(|i| i.to_string()))
    })
}

fn guid_of(event: &Value) -> Option<String> {
    event
        .get("requestGuid")
        .or_else(|| event.get("guid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

async fn subscribe_orders(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    rx: &mut broadcast::Receiver<Value>,
    guid: &str,
    access_token: &str,
    portfolio: &str,
    exchange: &str,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let msg = json!({
        "opcode": "OrdersGetAndSubscribeV2",
        "exchange": exchange,
        "portfolio": portfolio,
        "skipHistory": true,
        "frequency": 0,
        "format": "Simple",
        "guid": guid,
        "token": access_token,
    });

    let payload = serde_json::to_string(&msg)?;
    let t0 = Instant::now();
    sink.send(Message::Text(payload.into())).await?;

    let wait_future = async move {
        loop {
            match rx.recv().await {
                Ok(val) => {
                    if let Some(code) = val.get("httpCode").and_then(Value::as_i64) {
                        if code == 200 {
                            return Ok((val, duration_ms(t0.elapsed())));
                        }
                        return Err(format!("OrdersGetAndSubscribeV2 failed: {val}"));
                    }

                    if val.get("guid").and_then(Value::as_str) == Some(guid) {
                        return Ok((val, duration_ms(t0.elapsed())));
                    }
                }
                Err(_) => return Err("WS channel closed".to_string()),
            }
        }
    };

    timeout(Duration::from_secs(5), wait_future)
        .await
        .map_err(|_| "OrdersGetAndSubscribeV2 timeout".into())
        .and_then(|res| res.map_err(Into::into))
}

async fn subscribe_stop_orders(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    rx: &mut broadcast::Receiver<Value>,
    guid: &str,
    access_token: &str,
    portfolio: &str,
    exchange: &str,
    order_statuses: &[String],
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let mut msg = serde_json::Map::new();
    msg.insert("opcode".into(), Value::String("StopOrdersGetAndSubscribeV2".to_string()));
    msg.insert("exchange".into(), Value::String(exchange.to_string()));
    msg.insert("portfolio".into(), Value::String(portfolio.to_string()));
    msg.insert("skipHistory".into(), Value::Bool(true));
    msg.insert("format".into(), Value::String("Simple".to_string()));
    msg.insert("guid".into(), Value::String(guid.to_string()));
    msg.insert("token".into(), Value::String(access_token.to_string()));

    if !order_statuses.is_empty() {
        msg.insert(
            "orderStatuses".into(),
            Value::Array(order_statuses.iter().cloned().map(Value::String).collect()),
        );
    }

    let msg = Value::Object(msg);

    let payload = serde_json::to_string(&msg)?;
    println!(">> STOP SUBSCRIBE REQ: {payload}");
    let t0 = Instant::now();
    sink.send(Message::Text(payload.into())).await?;

    let wait_future = async move {
        loop {
            match rx.recv().await {
                Ok(val) => {
                    if let Some(code) = val.get("httpCode").and_then(Value::as_i64) {
                        if code == 200 {
                            return Ok((val, duration_ms(t0.elapsed())));
                        }
                        return Err(format!("StopOrdersGetAndSubscribeV2 failed: {val}"));
                    }

                    if val.get("guid").and_then(Value::as_str) == Some(guid) {
                        return Ok((val, duration_ms(t0.elapsed())));
                    }
                }
                Err(_) => return Err("WS channel closed".to_string()),
            }
        }
    };

    timeout(Duration::from_secs(5), wait_future)
        .await
        .map_err(|_| "StopOrdersGetAndSubscribeV2 timeout".into())
        .and_then(|res| res.map_err(Into::into))
}

async fn authorize_cws(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl Stream<Item = Result<Message, WsError>> + Unpin),
    access_token: &str,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let guid = new_guid();
    let msg = json!({
        "opcode": "authorize",
        "guid": guid,
        "token": access_token,
    });

    let payload = serde_json::to_string(&msg)?;
    let t0 = Instant::now();
    sink.send(Message::Text(payload.into())).await?;

    let resp = read_until_guid(stream, &guid, Duration::from_secs(2)).await?;
    let dt = duration_ms(t0.elapsed());
    if resp.get("httpCode").and_then(Value::as_i64) != Some(200) {
        return Err(format!("WS authorize failed: {resp}").into());
    }
    Ok((resp, dt))
}

async fn create_stop_limit_order(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl Stream<Item = Result<Message, WsError>> + Unpin),
    portfolio: &str,
    symbol: &str,
    exchange: &str,
    instrument_group: &str,
    price: f64,
    trigger_price: f64,
    condition: &str,
    qty: i32,
    activate: bool,
    comment: &str,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let guid = new_guid();
    let msg = json!({
        "opcode": "create:stopLimit",
        "guid": guid,
        "side": "buy",
        "quantity": qty,
        "price": price,
        "condition": condition,
        "triggerPrice": trigger_price,
        "instrument": {"symbol": symbol, "exchange": exchange, "instrumentGroup": instrument_group},
        "comment": comment,
        "user": {"portfolio": portfolio},
        "timeInForce": TIME_IN_FORCE,
        "allowMargin": ALLOW_MARGIN,
        "checkDuplicates": true,
        "activate": activate,
    });

    send_with_ack(sink, stream, msg, &guid, Duration::from_secs(2)).await
}

async fn update_stop_limit_order(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl Stream<Item = Result<Message, WsError>> + Unpin),
    portfolio: &str,
    symbol: &str,
    exchange: &str,
    instrument_group: &str,
    order_number: &str,
    new_price: f64,
    new_trigger_price: f64,
    condition: &str,
    qty: i32,
    activate: bool,
    comment: &str,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let guid = new_guid();
    let msg = json!({
        "opcode": "update:stopLimit",
        "guid": guid,
        "orderId": order_number,
        "exchange": exchange,
        "instrument": {"symbol": symbol, "exchange": exchange, "instrumentGroup": instrument_group},
        "user": {"portfolio": portfolio},
        "price": new_price,
        "condition": condition,
        "triggerPrice": new_trigger_price,
        "quantity": qty,
        "side": "buy",
        "allowMargin": ALLOW_MARGIN,
        "timeInForce": TIME_IN_FORCE,
        "checkDuplicates": true,
        "comment": comment,
        "activate": activate,
    });

    send_with_ack(sink, stream, msg, &guid, Duration::from_secs(2)).await
}

async fn delete_stop_limit_order(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl Stream<Item = Result<Message, WsError>> + Unpin),
    portfolio: &str,
    exchange: &str,
    order_number: &str,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let guid = new_guid();
    let msg = json!({
        "opcode": "delete:stopLimit",
        "guid": guid,
        "orderId": order_number,
        "exchange": exchange,
        "user": {"portfolio": portfolio},
        "checkDuplicates": true,
    });

    send_with_ack(sink, stream, msg, &guid, Duration::from_secs(2)).await
}

async fn send_with_ack(
    sink: &mut (impl Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl Stream<Item = Result<Message, WsError>> + Unpin),
    payload: Value,
    guid: &str,
    timeout_dur: Duration,
) -> Result<(Value, f64), Box<dyn std::error::Error>> {
    let payload_str = serde_json::to_string(&payload)?;
    let t0 = Instant::now();
    sink.send(Message::Text(payload_str.into())).await?;

    let resp = read_until_guid(stream, guid, timeout_dur).await?;
    let dt = duration_ms(t0.elapsed());

    Ok((resp, dt))
}

async fn read_until_guid(
    stream: &mut (impl Stream<Item = Result<Message, WsError>> + Unpin),
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
                }
            }
        }
        Err("CWS stream ended before ack".into())
    };

    match timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => Err("CWS ack timeout".into()),
    }
}

fn price_matches(price_val: Option<&Value>, expected: f64) -> bool {
    match price_val {
        Some(Value::Number(num)) => num
            .as_f64()
            .map(|p| (p - expected).abs() < 1e-9)
            .unwrap_or(false),
        Some(Value::String(s)) => s
            .parse::<f64>()
            .map(|p| (p - expected).abs() < 1e-9)
            .unwrap_or(false),
        _ => true,
    }
}

fn is_http_ok(resp: &Value) -> bool {
    resp.get("httpCode").and_then(Value::as_i64) == Some(200)
}

fn cws_error_message(resp: &Value) -> String {
    let code = resp
        .get("httpCode")
        .and_then(Value::as_i64)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let message = resp
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("<no message>");
    format!("httpCode={code} message={message}")
}

fn duration_ms(dur: Duration) -> f64 {
    dur.as_secs_f64() * 1000.0
}

fn new_guid() -> String {
    let rng = rand::rng();
    rng.sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}