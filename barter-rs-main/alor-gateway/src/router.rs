use std::collections::HashSet;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::models::{BarEvent, DataOrigin, OrderEvent, PositionEvent};

pub struct Router;

#[derive(Debug)]
pub enum RouterCommand {
    UpdateSubscribeWallclock(i64),
}

#[derive(Debug)]
pub enum RouterControl {
    AuthError(i64),
}

pub struct RouterStreams {
    pub bars_rx: mpsc::Receiver<BarEvent>,
    pub positions_rx: mpsc::Receiver<PositionEvent>,
    pub orders_rx: mpsc::Receiver<OrderEvent>,
    pub control_rx: mpsc::Receiver<RouterControl>,
}

impl Router {
    pub fn start(
        mut raw_rx: mpsc::Receiver<Value>,
        tf_sec: i64,
    ) -> (mpsc::Sender<RouterCommand>, RouterStreams) {
        let (bars_tx, bars_rx) = mpsc::channel(1024);
        let (positions_tx, positions_rx) = mpsc::channel(1024);
        let (orders_tx, orders_rx) = mpsc::channel(1024);
        let (control_tx, control_rx) = mpsc::channel(32);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let mut bar_dedup = HashSet::new();
            let mut live_cutoff_ts: Option<i64> = None;
            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(RouterCommand::UpdateSubscribeWallclock(ts)) => {
                                live_cutoff_ts = Some(ts - (2 * tf_sec));
                            }
                            None => break,
                        }
                    }
                    value = raw_rx.recv() => {
                        let Some(value) = value else {
                            break;
                        };
                        if let Some(code) = value.get("httpCode").and_then(Value::as_i64) {
                            if matches!(code, 401 | 403) {
                                warn!(http_code = code, "auth error from ws");
                                let _ = control_tx.send(RouterControl::AuthError(code)).await;
                            }
                            continue;
                        }

                        let bars = parse_bars(&value, live_cutoff_ts);
                        if !bars.is_empty() {
                            for bar in bars {
                                let key = (bar.symbol.clone(), bar.close_time_utc);
                                if bar_dedup.insert(key) {
                                    let _ = bars_tx.send(bar).await;
                                } else {
                                    debug!("duplicate bar dropped");
                                }
                            }
                            continue;
                        }

                        if let Some(position) = parse_position(&value) {
                            let _ = positions_tx.send(position).await;
                            continue;
                        }

                        if let Some(order) = parse_order(&value) {
                            let _ = orders_tx.send(order).await;
                        }
                    }
                }
            }
        });

        (
            cmd_tx,
            RouterStreams {
                bars_rx,
                positions_rx,
                orders_rx,
                control_rx,
            },
        )
    }
}

fn parse_bars(value: &Value, live_cutoff_ts: Option<i64>) -> Vec<BarEvent> {
    let Some(data) = value.get("data") else {
        return Vec::new();
    };

    if let Some(items) = data.as_array() {
        return items
            .iter()
            .filter_map(|item| parse_bar_item(item, live_cutoff_ts))
            .collect();
    }

    parse_bar_item(data, live_cutoff_ts).into_iter().collect()
}

fn parse_bar_item(data: &Value, live_cutoff_ts: Option<i64>) -> Option<BarEvent> {
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    let close_time = data.get("time").or_else(|| data.get("timestamp"))?;
    let close_time_utc = to_i64(close_time)?;
    let origin = match live_cutoff_ts {
        Some(cutoff) if close_time_utc <= cutoff => DataOrigin::History,
        Some(_) => DataOrigin::Live,
        None => DataOrigin::Live,
    };
    Some(BarEvent {
        symbol,
        close_time_utc,
        o: data.get("open").and_then(Value::as_f64).unwrap_or_default(),
        h: data.get("high").and_then(Value::as_f64).unwrap_or_default(),
        l: data.get("low").and_then(Value::as_f64).unwrap_or_default(),
        c: data.get("close").and_then(Value::as_f64).unwrap_or_default(),
        v: data.get("volume").and_then(Value::as_f64).unwrap_or_default(),
        origin,
    })
}

fn parse_position(value: &Value) -> Option<PositionEvent> {
    let data = value.get("data")?;
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    Some(PositionEvent {
        symbol,
        qty: data.get("qty").and_then(Value::as_f64).unwrap_or_default(),
        avg_price: data
            .get("avgPrice")
            .or_else(|| data.get("avg_price"))
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        ts_utc: data
            .get("timestamp")
            .and_then(to_i64)
            .unwrap_or_else(|| Utc::now().timestamp()),
    })
}

fn parse_order(value: &Value) -> Option<OrderEvent> {
    let data = value.get("data")?;
    let order_id = data
        .get("orderId")
        .or_else(|| data.get("id"))
        .and_then(Value::as_i64)?;
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    Some(OrderEvent {
        order_id,
        symbol,
        status: data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        filled: data
            .get("filled")
            .or_else(|| data.get("filledQty"))
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        price: data.get("price").and_then(Value::as_f64).unwrap_or_default(),
        ts_utc: data
            .get("timestamp")
            .and_then(to_i64)
            .unwrap_or_else(|| Utc::now().timestamp()),
    })
}

fn to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return Some(v as i64);
    }
    if let Some(s) = value.as_str() {
        if let Ok(v) = s.parse::<i64>() {
            return Some(v);
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Some(dt.timestamp());
        }
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
            return Some(dt.and_utc().timestamp());
        }
    }
    None
}
