//! Debug utility: combine Alor WS data-subscriptions + CWS order placement.
//!
//! Goal: after a (marketable) limit order executes, print raw payloads for:
//! - orders updates,
//! - positions updates,
//! - all-trades by instrument updates,
//! - and also show quotes used to compute a marketable price.
//!
//! SAFETY:
//! - Use an isolated portfolio/account.
//! - Default qty is 1.
//! - Uses IOC (bookorcancel) marketable limit orders.
//!
//! Required env:
//! - REFRESH_TOKEN  (OAuth refresh token)
//! Optional env:
//! - OAUTH_URL      (default: https://oauth.alor.ru/refresh)
//! - WS_URL         (default: wss://api.alor.ru/ws)
//! - CWS_URL        (default: wss://api.alor.ru/cws)
//! - EXCHANGE       (default: MOEX)
//! - INSTRUMENT_GROUP (default: RFUD)
//! - PORTFOLIO      (default: 7502T0U)
//! - SYMBOL         (default: IMOEXF)
//! - QTY            (default: 1)
//! - TICK_SIZE      (default: 0.5)
//! - MARKETABLE_TICKS (default: 10)
//! - OPEN_SIDE      (buy|sell, default: buy)
//! - TIMEOUT_SEC    (default: 30)
//!
//! Run (example):
//!   REFRESH_TOKEN=... PORTFOLIO=7502T0U SYMBOL=IMOEXF cargo run --bin alor_ws_cws_alltrades_debug

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const DEFAULT_OAUTH_URL: &str = "https://oauth.alor.ru/refresh";
const DEFAULT_WS_URL: &str = "wss://api.alor.ru/ws";
const DEFAULT_CWS_URL: &str = "wss://api.alor.ru/cws";

#[derive(Clone, Debug)]
struct Config {
    oauth_url: String,
    ws_url: String,
    cws_url: String,
    refresh_token: String,
    exchange: String,
    instrument_group: String,
    portfolio: String,
    symbol: String,
    qty: i64,
    tick_size: f64,
    marketable_ticks: i64,
    open_side: String, // "buy" or "sell"
    timeout_sec: u64,
}

impl Config {
    fn from_env() -> Result<Self> {
        let refresh_token = std::env::var("REFRESH_TOKEN")
            .context("Missing env REFRESH_TOKEN")?;

        Ok(Self {
            oauth_url: std::env::var("OAUTH_URL").unwrap_or_else(|_| DEFAULT_OAUTH_URL.to_string()),
            ws_url: std::env::var("WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string()),
            cws_url: std::env::var("CWS_URL").unwrap_or_else(|_| DEFAULT_CWS_URL.to_string()),
            refresh_token,
            exchange: std::env::var("EXCHANGE").unwrap_or_else(|_| "MOEX".to_string()),
            instrument_group: std::env::var("INSTRUMENT_GROUP").unwrap_or_else(|_| "RFUD".to_string()),
            portfolio: std::env::var("PORTFOLIO").unwrap_or_else(|_| "7502T0U".to_string()),
            symbol: std::env::var("SYMBOL").unwrap_or_else(|_| "IMOEXF".to_string()),
            qty: std::env::var("QTY").ok().and_then(|v| v.parse().ok()).unwrap_or(1),
            tick_size: std::env::var("TICK_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5),
            marketable_ticks: std::env::var("MARKETABLE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(10),
            open_side: std::env::var("OPEN_SIDE").unwrap_or_else(|_| "buy".to_string()).to_lowercase(),
            timeout_sec: std::env::var("TIMEOUT_SEC").ok().and_then(|v| v.parse().ok()).unwrap_or(30),
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env()?;

    if cfg.open_side != "buy" && cfg.open_side != "sell" {
        bail!("OPEN_SIDE must be 'buy' or 'sell'");
    }

    println!("== config ==\n{cfg:#?}\n");

    let access_token = get_access_token(&cfg.oauth_url, &cfg.refresh_token).await?;
    println!("got access token: {}...", &access_token.chars().take(12).collect::<String>());

    // --- WS (data subscriptions) ---
    let (ws_stream, _) = tokio_tungstenite::connect_async(&cfg.ws_url)
        .await
        .context("connect ws")?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Broadcast all WS json payloads (debug bus)
    let (ws_tx, _) = broadcast::channel::<Value>(4096);

    // Reader task
    let ws_tx_clone = ws_tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(Message::Text(txt)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                        let _ = ws_tx_clone.send(v);
                    } else {
                        eprintln!("WS non-json text: {txt}");
                    }
                }
                Ok(Message::Binary(bin)) => {
                    if let Ok(txt) = String::from_utf8(bin) {
                        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                            let _ = ws_tx_clone.send(v);
                        } else {
                            eprintln!("WS binary->text non-json: {txt}");
                        }
                    } else {
                        eprintln!("WS binary (non-utf8), len={}", bin.len());
                    }
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(frame)) => {
                    eprintln!("WS closed: {frame:?}");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("WS read error: {e:?}");
                    break;
                }
            }
        }
    });

    // --- CWS (orders) ---
    let (cws_stream, _) = tokio_tungstenite::connect_async(&cfg.cws_url)
        .await
        .context("connect cws")?;
    let (mut cws_write, mut cws_read) = cws_stream.split();

    let (cws_tx, _) = broadcast::channel::<Value>(1024);
    let cws_tx_clone = cws_tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = cws_read.next().await {
            match msg {
                Ok(Message::Text(txt)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                        let _ = cws_tx_clone.send(v);
                    } else {
                        eprintln!("CWS non-json text: {txt}");
                    }
                }
                Ok(Message::Binary(bin)) => {
                    if let Ok(txt) = String::from_utf8(bin) {
                        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                            let _ = cws_tx_clone.send(v);
                        } else {
                            eprintln!("CWS binary->text non-json: {txt}");
                        }
                    } else {
                        eprintln!("CWS binary (non-utf8), len={}", bin.len());
                    }
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(frame)) => {
                    eprintln!("CWS closed: {frame:?}");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("CWS read error: {e:?}");
                    break;
                }
            }
        }
    });

    // --- Subscriptions (WS) ---
    let quotes_guid = Uuid::new_v4().to_string();
    let orders_guid = Uuid::new_v4().to_string();
    let positions_guid = Uuid::new_v4().to_string();
    let alltrades_guid = Uuid::new_v4().to_string();
    let trades_portfolio_guid = Uuid::new_v4().to_string();

    // 1) quotes (bid/ask)
    ws_send(
        &mut ws_write,
        json!({
            "opcode": "QuotesSubscribe",
            "exchange": cfg.exchange,
            "instrumentGroup": cfg.instrument_group,
            "code": cfg.symbol,
            "format": "Simple",
            "token": access_token,
            "guid": quotes_guid,
        }),
    )
    .await?;

    // 2) portfolio orders
    ws_send(
        &mut ws_write,
        json!({
            "opcode": "OrdersGetAndSubscribeV2",
            "exchange": cfg.exchange,
            "portfolio": cfg.portfolio,
            "format": "Simple",
            "token": access_token,
            "guid": orders_guid,
        }),
    )
    .await?;

    // 3) portfolio positions
    ws_send(
        &mut ws_write,
        json!({
            "opcode": "PositionsGetAndSubscribeV2",
            "exchange": cfg.exchange,
            "portfolio": cfg.portfolio,
            "format": "Simple",
            "token": access_token,
            "guid": positions_guid,
        }),
    )
    .await?;

    // 4) all trades by instrument (depth=1 => at most 1 existing snapshot item)
    ws_send(
        &mut ws_write,
        json!({
            "opcode": "AllTradesGetAndSubscribe",
            "exchange": cfg.exchange,
            "instrumentGroup": cfg.instrument_group,
            "code": cfg.symbol,
            "depth": 1,
            "includeVirtualTrades": true,
            "format": "Simple",
            "token": access_token,
            "guid": alltrades_guid,
        }),
    )
    .await?;

    // 5) trades by portfolio (our executions)
    ws_send(
        &mut ws_write,
        json!({
            "opcode": "TradesGetAndSubscribeV2",
            "exchange": cfg.exchange,
            "portfolio": cfg.portfolio,
            "skipHistory": true,
            "format": "Simple",
            "token": access_token,
            "guid": trades_portfolio_guid,
        }),
    )
    .await?;

    println!("WS subscribed: quotes={quotes_guid} orders={orders_guid} positions={positions_guid} alltrades={alltrades_guid} trades={trades_portfolio_guid}");

    // Start lightweight debug printers (do not block main flow)
    spawn_guid_printer("WS.quotes", ws_tx.subscribe(), quotes_guid.clone());
    spawn_guid_printer("WS.positions", ws_tx.subscribe(), positions_guid.clone());
    spawn_guid_printer("WS.orders", ws_tx.subscribe(), orders_guid.clone());
    spawn_guid_printer("WS.trades_portfolio", ws_tx.subscribe(), trades_portfolio_guid.clone());

    // For AllTrades we only print non-existing later, and filter by our order id when known.

    // --- Authorize CWS ---
    let auth_guid = Uuid::new_v4().to_string();
    ws_send(
        &mut cws_write,
        json!({
            "opcode": "authorize",
            "token": access_token,
            "guid": auth_guid,
        }),
    )
    .await?;

    let auth_ok = wait_cws_authorized(cws_tx.subscribe(), &auth_guid, Duration::from_secs(cfg.timeout_sec)).await?;
    if !auth_ok {
        bail!("CWS authorize failed");
    }
    println!("CWS authorized");

    // --- Get first quote (bid/ask) ---
    let (bid, ask) = wait_first_quote(
        ws_tx.subscribe(),
        &quotes_guid,
        Duration::from_secs(cfg.timeout_sec),
    )
    .await?;

    println!("best quote: bid={bid} ask={ask}");

    // --- Place OPEN (marketable limit IOC) ---
    let open_side = cfg.open_side.clone();
    let open_price = marketable_price(&open_side, bid, ask, cfg.tick_size, cfg.marketable_ticks);

    println!("placing OPEN order: side={open_side} qty={} price={open_price}", cfg.qty);

    let open_ext_id = format!("dbg-open-{}", Uuid::new_v4());
    let open_guid = Uuid::new_v4().to_string();

    ws_send(
        &mut cws_write,
        json!({
            "opcode": "create:limit",
            "exchange": cfg.exchange,
            "portfolio": cfg.portfolio,
            "side": open_side,
            "symbol": cfg.symbol,
            "quantity": cfg.qty,
            "price": open_price,
            "timeInForce": "bookorcancel",
            "ext_id": open_ext_id,
            "format": "Simple",
            "instrumentGroup": cfg.instrument_group,
            "guid": open_guid,
        }),
    )
    .await?;

    let open_order_id = wait_cws_create_order_id(cws_tx.subscribe(), &open_guid, Duration::from_secs(cfg.timeout_sec)).await?;
    println!("OPEN created, broker_order_id={open_order_id}");

    // Watch AllTrades but only print trades that match our order id (best effort)
    spawn_alltrades_printer_filtered(
        ws_tx.subscribe(),
        alltrades_guid.clone(),
        Some(open_order_id.clone()),
    );

    // Wait until OPEN is filled (from WS.orders stream)
    wait_order_filled(
        ws_tx.subscribe(),
        &orders_guid,
        &open_order_id,
        Duration::from_secs(cfg.timeout_sec),
    )
    .await
    .context("wait OPEN filled")?;

    println!("OPEN filled (observed via WS.orders)");

    // --- Place CLOSE (opposite side) ---
    // Refresh quote again to compute a sane marketable close price.
    let (bid2, ask2) = wait_first_quote(
        ws_tx.subscribe(),
        &quotes_guid,
        Duration::from_secs(cfg.timeout_sec),
    )
    .await
    .unwrap_or((bid, ask));

    let close_side = if cfg.open_side == "buy" { "sell" } else { "buy" };
    let close_price = marketable_price(close_side, bid2, ask2, cfg.tick_size, cfg.marketable_ticks);

    println!("placing CLOSE order: side={close_side} qty={} price={close_price}", cfg.qty);

    let close_ext_id = format!("dbg-close-{}", Uuid::new_v4());
    let close_guid = Uuid::new_v4().to_string();

    ws_send(
        &mut cws_write,
        json!({
            "opcode": "create:limit",
            "exchange": cfg.exchange,
            "portfolio": cfg.portfolio,
            "side": close_side,
            "symbol": cfg.symbol,
            "quantity": cfg.qty,
            "price": close_price,
            "timeInForce": "bookorcancel",
            "ext_id": close_ext_id,
            "format": "Simple",
            "instrumentGroup": cfg.instrument_group,
            "guid": close_guid,
        }),
    )
    .await?;

    let close_order_id = wait_cws_create_order_id(cws_tx.subscribe(), &close_guid, Duration::from_secs(cfg.timeout_sec)).await?;
    println!("CLOSE created, broker_order_id={close_order_id}");

    spawn_alltrades_printer_filtered(
        ws_tx.subscribe(),
        alltrades_guid.clone(),
        Some(close_order_id.clone()),
    );

    wait_order_filled(
        ws_tx.subscribe(),
        &orders_guid,
        &close_order_id,
        Duration::from_secs(cfg.timeout_sec),
    )
    .await
    .context("wait CLOSE filled")?;

    println!("CLOSE filled (observed via WS.orders)");

    println!("DONE. Tip: keep terminal open 3-5s to see remaining WS payload prints.");
    tokio::time::sleep(Duration::from_secs(5)).await;

    Ok(())
}

async fn ws_send<W>(write: &mut W, v: Value) -> Result<()>
where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let txt = serde_json::to_string(&v).context("serialize ws json")?;
    write.send(Message::Text(txt)).await.context("ws send")?;
    Ok(())
}

async fn get_access_token(oauth_url: &str, refresh_token: &str) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct TokenResponse {
        #[serde(rename = "AccessToken")]
        access_token: String,
    }

    let url = format!("{oauth_url}?token={refresh_token}");
    let resp = reqwest::Client::new()
        .post(&url)
        .send()
        .await
        .context("oauth request")?;

    if !resp.status().is_success() {
        let txt = resp.text().await.unwrap_or_default();
        bail!("oauth failed: status={} body={txt}", resp.status());
    }

    let tr: TokenResponse = resp.json().await.context("oauth json")?;
    Ok(tr.access_token)
}

fn marketable_price(side: &str, bid: f64, ask: f64, tick: f64, ticks: i64) -> f64 {
    let ticks_f = ticks as f64;
    // buy: price ABOVE ask to cross
    // sell: price BELOW bid to cross
    let raw = if side == "buy" {
        ask + ticks_f * tick
    } else {
        bid - ticks_f * tick
    };

    // Align to tick grid.
    (raw / tick).round() * tick
}

fn spawn_guid_printer(name: &'static str, mut rx: broadcast::Receiver<Value>, guid: String) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(v) => {
                    let g = v.get("guid").and_then(|x| x.as_str()).unwrap_or("");
                    if g == guid {
                        println!("\n== {name} ==\n{}\n", serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("{name} lagged by {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn spawn_alltrades_printer_filtered(
    mut rx: broadcast::Receiver<Value>,
    guid: String,
    order_id_opt: Option<String>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(v) => {
                    let g = v.get("guid").and_then(|x| x.as_str()).unwrap_or("");
                    if g != guid {
                        continue;
                    }
                    // data is an object in Simple format
                    let data = match v.get("data") {
                        Some(d) => d,
                        None => continue,
                    };

                    // Skip snapshot/old history if present.
                    if data.get("existing").and_then(|x| x.as_bool()) == Some(true) {
                        continue;
                    }

                    if let Some(order_id) = order_id_opt.as_ref() {
                        // Best effort: match by `orderno` (doc field; can be string or number)
                        let orderno_v = data.get("orderno");
                        let orderno = orderno_v
                            .and_then(|x| x.as_str().map(|s| s.to_string()).or_else(|| x.as_i64().map(|n| n.to_string())))
                            .unwrap_or_default();
                        if orderno != *order_id {
                            continue;
                        }
                    }

                    println!("\n== WS.alltrades (filtered) ==\n{}\n", serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("WS.alltrades lagged by {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

async fn wait_first_quote(
    mut rx: broadcast::Receiver<Value>,
    guid: &str,
    max_wait: Duration,
) -> Result<(f64, f64)> {
    let fut = async {
        loop {
            let v = rx.recv().await.map_err(|e| anyhow!("quote recv: {e:?}"))?;
            if v.get("guid").and_then(|x| x.as_str()) != Some(guid) {
                continue;
            }
            let data = v
                .get("data")
                .ok_or_else(|| anyhow!("quote: missing data"))?;
            let bid = data
                .get("bid")
                .and_then(|x| x.as_f64())
                .ok_or_else(|| anyhow!("quote: missing bid"))?;
            let ask = data
                .get("ask")
                .and_then(|x| x.as_f64())
                .ok_or_else(|| anyhow!("quote: missing ask"))?;
            return Ok((bid, ask));
        }
    };

    timeout(max_wait, fut)
        .await
        .context("quote timeout")?
}

async fn wait_cws_authorized(
    mut rx: broadcast::Receiver<Value>,
    auth_guid: &str,
    max_wait: Duration,
) -> Result<bool> {
    let fut = async {
        loop {
            let v = rx.recv().await.map_err(|e| anyhow!("cws recv: {e:?}"))?;
            // CWS uses requestGuid, but we also send guid
            let rg = v
                .get("requestGuid")
                .or_else(|| v.get("guid"))
                .and_then(|x| x.as_str())
                .unwrap_or("");

            if rg != auth_guid {
                continue;
            }

            let code = v.get("httpCode").and_then(|x| x.as_i64()).unwrap_or(-1);
            return Ok(code == 200);
        }
    };

    timeout(max_wait, fut)
        .await
        .context("cws auth timeout")?
}

async fn wait_cws_create_order_id(
    mut rx: broadcast::Receiver<Value>,
    guid: &str,
    max_wait: Duration,
) -> Result<String> {
    let fut = async {
        loop {
            let v = rx.recv().await.map_err(|e| anyhow!("cws recv: {e:?}"))?;
            let rg = v
                .get("requestGuid")
                .or_else(|| v.get("guid"))
                .and_then(|x| x.as_str())
                .unwrap_or("");

            if rg != guid {
                continue;
            }

            // CWS create responses typically have orderNumber (string or number)
            if let Some(order_no) = v.get("orderNumber") {
                if let Some(s) = order_no.as_str() {
                    return Ok(s.to_string());
                }
                if let Some(n) = order_no.as_i64() {
                    return Ok(n.to_string());
                }
            }

            // Sometimes it is nested
            if let Some(order_no) = v.get("data").and_then(|d| d.get("orderNumber")) {
                if let Some(s) = order_no.as_str() {
                    return Ok(s.to_string());
                }
                if let Some(n) = order_no.as_i64() {
                    return Ok(n.to_string());
                }
            }

            // If error
            if let Some(msg) = v.get("message").and_then(|x| x.as_str()) {
                bail!("CWS create failed: {msg}");
            }

            bail!("CWS create: can't extract orderNumber: {v}");
        }
    };

    timeout(max_wait, fut)
        .await
        .context("cws create timeout")?
}

async fn wait_order_filled(
    mut rx: broadcast::Receiver<Value>,
    orders_guid: &str,
    order_id: &str,
    max_wait: Duration,
) -> Result<()> {
    let fut = async {
        loop {
            let v = rx.recv().await.map_err(|e| anyhow!("ws recv: {e:?}"))?;
            if v.get("guid").and_then(|x| x.as_str()) != Some(orders_guid) {
                continue;
            }

            let data = match v.get("data") {
                Some(d) => d,
                None => continue,
            };

            // Orders can come as an array or as a single object depending on format/phase.
            if let Some(arr) = data.as_array() {
                for item in arr {
                    if is_filled_order(item, order_id) {
                        println!("\n== WS.orders (filled match) ==\n{}\n", serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
                        return Ok(());
                    }
                }
            } else {
                if is_filled_order(data, order_id) {
                    println!("\n== WS.orders (filled match) ==\n{}\n", serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
                    return Ok(());
                }
            }
        }
    };

    timeout(max_wait, fut)
        .await
        .context("order fill timeout")??;

    Ok(())
}

fn is_filled_order(order: &Value, order_id: &str) -> bool {
    let id = match order.get("id") {
        Some(v) => v
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_i64().map(|n| n.to_string()))
            .unwrap_or_default(),
        None => String::new(),
    };
    if id != order_id {
        return false;
    }
    let status = order.get("status").and_then(|x| x.as_str()).unwrap_or("");
    status.eq_ignore_ascii_case("filled")
}
