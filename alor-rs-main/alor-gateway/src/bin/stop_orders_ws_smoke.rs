use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::{Duration, Instant, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use alor_gateway::auth::TokenProvider;
use alor_gateway::config::{AlorGatewayConfig, detect_config_path, log_resolved_config};
use alor_gateway::cws_client::CwsClient;
use alor_gateway::health::HealthState;
use alor_gateway::ws_subscriptions::{build_stop_orders_subscribe, build_unsubscribe};
use alor_protocol::StopLimitCondition;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let config_path = detect_config_path(&args);
    let resolved = if let Some(path) = config_path.clone() {
        AlorGatewayConfig::from_file_with_sources(path)?
    } else {
        AlorGatewayConfig::from_env_with_sources()?
    };
    log_resolved_config(&resolved, config_path.as_deref());
    let cfg = resolved.config;

    let dry_run = has_flag(&args, "--dry-run");
    let live_confirm = has_flag(&args, "--live-confirm");
    if dry_run == live_confirm {
        bail!("specify exactly one of --dry-run or --live-confirm");
    }

    let symbol = arg_value(&args, "--symbol")
        .or_else(|| cfg.symbols.first().cloned())
        .context("symbol is required (set --symbol or symbols in config)")?;
    let side = arg_value(&args, "--side").unwrap_or_else(|| "buy".to_string());
    if !matches!(side.as_str(), "buy" | "sell") {
        bail!("--side must be buy or sell");
    }
    let qty = parse_f64(&args, "--qty", 1.0)?;
    let trigger_price = parse_f64_opt(&args, "--trigger-price")?.unwrap_or(1.0);
    let limit_price = parse_f64_opt(&args, "--limit-price")?.unwrap_or(trigger_price);
    let condition = parse_condition(arg_value(&args, "--condition").as_deref())?;
    let ws_timeout_sec = parse_u64(&args, "--ws-timeout-sec", 20)?;
    let stop_end_after_sec = parse_i64(&args, "--stop-end-after-sec", 600)?;
    let stop_end_unix_time = now_unix_ts().saturating_add(stop_end_after_sec);
    let comment_prefix =
        arg_value(&args, "--comment-prefix").unwrap_or_else(|| "smoke_stoporders_ws".to_string());
    let comment = format!("{comment_prefix}_{}", now_unix_ts());
    let instrument_group =
        arg_value(&args, "--instrument-group").or_else(|| Some(cfg.instrument_group.clone()));

    println!(
        "stop-orders ws smoke config: mode={}, symbol={}, side={}, qty={}, trigger={}, limit={}, condition={}, ws_timeout_sec={}, stop_end_unix_time={}, instrument_group={:?}, comment={}",
        if dry_run { "dry-run" } else { "live-confirm" },
        symbol,
        side,
        qty,
        trigger_price,
        limit_price,
        condition.as_canonical_str(),
        ws_timeout_sec,
        stop_end_unix_time,
        instrument_group,
        comment
    );

    if dry_run {
        println!("dry-run: no requests sent");
        return Ok(());
    }

    let token_provider = TokenProvider::new(cfg.oauth_url.clone(), cfg.refresh_token.clone());
    let access_token = token_provider.access_token().await?;

    let (mut ws_stream, _) = connect_async(&cfg.ws_url)
        .await
        .with_context(|| format!("failed to connect ws_url={}", cfg.ws_url))?;

    let (stop_guid, stop_subscribe_msg) = build_stop_orders_subscribe(&cfg, &access_token, false);
    ws_stream
        .send(Message::Text(stop_subscribe_msg.into()))
        .await
        .context("failed to send StopOrdersGetAndSubscribeV2")?;
    wait_for_subscription_ack(
        &mut ws_stream,
        &stop_guid,
        Duration::from_secs(ws_timeout_sec),
    )
    .await
    .context("stop-orders subscribe ack timeout")?;
    println!("stop-orders subscribe ack: PASS guid={stop_guid}");

    let health = Arc::new(parking_lot::RwLock::new(HealthState::default()));
    let cws = CwsClient::start(cfg.clone(), token_provider, health);

    let create_resp = cws
        .create_stop_limit(
            &cfg.portfolio,
            &cfg.exchange,
            &symbol,
            &side,
            qty,
            trigger_price,
            limit_price,
            condition,
            stop_end_unix_time,
            Some(&comment),
            instrument_group.as_deref(),
            true,
            None,
        )
        .await
        .context("create:stopLimit failed")?;
    println!("create response: {}", create_resp);

    let stop_order_id = extract_order_id_str(&create_resp)
        .or_else(|| extract_order_id_num(&create_resp).map(|v| v.to_string()))
        .context("create response does not contain orderNumber/orderId")?;

    let created_status = wait_for_stop_order_status(
        &mut ws_stream,
        &stop_order_id,
        &["working", "filled", "rejected", "expired"],
        Duration::from_secs(ws_timeout_sec),
    )
    .await
    .with_context(|| format!("no ws status after create for stop_order_id={stop_order_id}"))?;
    println!(
        "ws event after create: PASS stop_order_id={} status={}",
        stop_order_id, created_status
    );

    let delete_resp = cws
        .delete_stop_limit(
            &cfg.portfolio,
            &cfg.exchange,
            &stop_order_id,
            Some(side.as_str()),
            true,
            None,
        )
        .await
        .context("delete:stopLimit failed")?;
    println!("delete response: {}", delete_resp);

    let deleted_status = wait_for_stop_order_status(
        &mut ws_stream,
        &stop_order_id,
        &["canceled", "cancelled", "rejected", "expired", "filled"],
        Duration::from_secs(ws_timeout_sec),
    )
    .await
    .with_context(|| {
        format!("no ws terminal status after delete for stop_order_id={stop_order_id}")
    })?;
    println!(
        "ws event after delete: PASS stop_order_id={} status={}",
        stop_order_id, deleted_status
    );

    let unsubscribe_msg = build_unsubscribe(&stop_guid, &access_token);
    let _ = ws_stream.send(Message::Text(unsubscribe_msg.into())).await;
    let _ = ws_stream.close(None).await;

    println!(
        "smoke result: PASS stop_order_id={} create_status={} delete_status={}",
        stop_order_id, created_status, deleted_status
    );
    Ok(())
}

async fn wait_for_subscription_ack(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    guid: &str,
    timeout_dur: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout_dur;
    loop {
        let now = Instant::now();
        if now >= deadline {
            bail!("timeout waiting subscribe ack guid={guid}");
        }
        let remaining = deadline.saturating_duration_since(now);
        let msg = timeout(remaining, ws_stream.next())
            .await
            .context("ws read timeout waiting subscribe ack")?
            .context("ws stream closed while waiting subscribe ack")??;
        if let Some(text) = message_to_text(msg)? {
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let ack_guid = value
                .get("requestGuid")
                .or_else(|| value.get("guid"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let http_code = value
                .get("httpCode")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if ack_guid == guid && http_code == 200 {
                return Ok(());
            }
        }
    }
}

async fn wait_for_stop_order_status(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    stop_order_id: &str,
    accepted_statuses: &[&str],
    timeout_dur: Duration,
) -> Result<String> {
    let deadline = Instant::now() + timeout_dur;
    loop {
        let now = Instant::now();
        if now >= deadline {
            bail!("timeout waiting stop-order status for id={stop_order_id}");
        }
        let remaining = deadline.saturating_duration_since(now);
        let msg = timeout(remaining, ws_stream.next())
            .await
            .context("ws read timeout waiting stop-order status")?
            .context("ws stream closed while waiting stop-order status")??;
        if let Some(text) = message_to_text(msg)? {
            let Some((event_id, status)) = parse_stop_order_status(&text) else {
                continue;
            };
            if event_id == stop_order_id
                && accepted_statuses
                    .iter()
                    .any(|candidate| status.eq_ignore_ascii_case(candidate))
            {
                return Ok(status);
            }
        }
    }
}

fn message_to_text(msg: Message) -> Result<Option<String>> {
    match msg {
        Message::Text(text) => Ok(Some(text.to_string())),
        Message::Binary(_) => Ok(None),
        Message::Ping(_) => Ok(None),
        Message::Pong(_) => Ok(None),
        Message::Frame(_) => Ok(None),
        Message::Close(_) => bail!("ws closed"),
    }
}

fn parse_stop_order_status(text: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let data = value.get("data")?;
    if data.get("stopPrice").is_none()
        && data.get("triggerPrice").is_none()
        && data.get("stopEndUnixTime").is_none()
    {
        return None;
    }
    let stop_order_id = data
        .get("id")
        .or_else(|| data.get("orderId"))
        .and_then(value_to_string)?;
    let status = data
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_lowercase();
    Some((stop_order_id, status))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == key {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&format!("{key}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn parse_f64_opt(args: &[String], key: &str) -> Result<Option<f64>> {
    match arg_value(args, key) {
        Some(v) => Ok(Some(
            v.parse::<f64>()
                .with_context(|| format!("invalid {key} value: {v}"))?,
        )),
        None => Ok(None),
    }
}

fn parse_f64(args: &[String], key: &str, default: f64) -> Result<f64> {
    Ok(parse_f64_opt(args, key)?.unwrap_or(default))
}

fn parse_i64(args: &[String], key: &str, default: i64) -> Result<i64> {
    match arg_value(args, key) {
        Some(v) => v
            .parse::<i64>()
            .with_context(|| format!("invalid {key} value: {v}")),
        None => Ok(default),
    }
}

fn parse_u64(args: &[String], key: &str, default: u64) -> Result<u64> {
    match arg_value(args, key) {
        Some(v) => v
            .parse::<u64>()
            .with_context(|| format!("invalid {key} value: {v}")),
        None => Ok(default),
    }
}

fn parse_condition(raw: Option<&str>) -> Result<StopLimitCondition> {
    let value = raw.unwrap_or("lessorequal");
    serde_json::from_str::<StopLimitCondition>(&format!("\"{value}\""))
        .with_context(|| format!("invalid --condition value: {value}"))
}

fn now_unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn extract_order_id_str(value: &Value) -> Option<String> {
    value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(Value::as_str)
        .map(|v| v.to_string())
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(Value::as_str)
                    .map(|v| v.to_string())
            })
        })
}

fn extract_order_id_num(value: &Value) -> Option<i64> {
    value
        .get("orderNumber")
        .or_else(|| value.get("orderId"))
        .and_then(to_i64)
        .or_else(|| {
            value.get("data").and_then(|data| {
                data.get("orderNumber")
                    .or_else(|| data.get("orderId"))
                    .and_then(to_i64)
            })
        })
}

fn to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return Some(v as i64);
    }
    value.as_str().and_then(|v| v.parse::<i64>().ok())
}
