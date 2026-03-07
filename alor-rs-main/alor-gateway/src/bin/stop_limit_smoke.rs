use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use alor_gateway::auth::TokenProvider;
use alor_gateway::config::{AlorGatewayConfig, detect_config_path, log_resolved_config};
use alor_gateway::cws_client::CwsClient;
use alor_gateway::health::HealthState;
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
    let stop_end_after_sec = parse_i64(&args, "--stop-end-after-sec", 600)?;
    let stop_end_unix_time = now_unix_ts().saturating_add(stop_end_after_sec);
    let comment_prefix =
        arg_value(&args, "--comment-prefix").unwrap_or_else(|| "smoke_stoplimit".to_string());
    let comment = format!("{comment_prefix}_{}", now_unix_ts());
    let delete_without_side = has_flag(&args, "--delete-without-side");
    let instrument_group = arg_value(&args, "--instrument-group")
        .or_else(|| Some(cfg.instrument_group.clone()));

    println!(
        "stop-limit smoke config: mode={}, symbol={}, side={}, qty={}, trigger={}, limit={}, condition={}, stop_end_unix_time={}, instrument_group={:?}, comment={}",
        if dry_run { "dry-run" } else { "live-confirm" },
        symbol,
        side,
        qty,
        trigger_price,
        limit_price,
        condition.as_canonical_str(),
        stop_end_unix_time,
        instrument_group,
        comment
    );

    if dry_run {
        println!("dry-run: no requests sent");
        return Ok(());
    }

    let health = Arc::new(parking_lot::RwLock::new(HealthState::default()));
    let token_provider = TokenProvider::new(cfg.oauth_url.clone(), cfg.refresh_token.clone());
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
        )
        .await
        .context("create:stopLimit failed")?;
    println!("create response: {}", create_resp);

    let stop_order_id = extract_order_id_str(&create_resp)
        .or_else(|| extract_order_id_num(&create_resp).map(|v| v.to_string()))
        .context("create response does not contain orderNumber/orderId")?;

    let delete_side = if delete_without_side {
        None
    } else {
        Some(side.as_str())
    };
    let delete_resp = cws
        .delete_stop_limit(
            &cfg.portfolio,
            &cfg.exchange,
            &stop_order_id,
            delete_side,
            true,
        )
        .await
        .context("delete:stopLimit failed")?;
    println!("delete response: {}", delete_resp);
    println!("smoke result: PASS stop_order_id={stop_order_id}");
    Ok(())
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
