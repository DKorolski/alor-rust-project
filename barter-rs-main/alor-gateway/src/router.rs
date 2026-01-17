use std::collections::HashSet;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::models::{BarEvent, OrderEvent, PositionEvent};

pub struct Router;

pub struct RouterStreams {
    pub bars_rx: mpsc::Receiver<BarEvent>,
    pub positions_rx: mpsc::Receiver<PositionEvent>,
    pub orders_rx: mpsc::Receiver<OrderEvent>,
}

impl Router {
    pub fn start(mut raw_rx: mpsc::Receiver<Value>) -> RouterStreams {
        let (bars_tx, bars_rx) = mpsc::channel(1024);
        let (positions_tx, positions_rx) = mpsc::channel(1024);
        let (orders_tx, orders_rx) = mpsc::channel(1024);

        tokio::spawn(async move {
            let mut bar_dedup = HashSet::new();
            while let Some(value) = raw_rx.recv().await {
                if let Some(code) = value.get("httpCode").and_then(Value::as_i64) {
                    if matches!(code, 401 | 403) {
                        warn!(http_code = code, "auth error from ws");
                    }
                    continue;
                }

                let bars = parse_bars(&value);
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
        });

        RouterStreams {
            bars_rx,
            positions_rx,
            orders_rx,
        }
    }
}

fn parse_bars(value: &Value) -> Vec<BarEvent> {
    let Some(data) = value.get("data") else {
        return Vec::new();
    };

    if let Some(items) = data.as_array() {
        return items.iter().filter_map(parse_bar_item).collect();
    }

    parse_bar_item(data).into_iter().collect()
}

fn parse_bar_item(data: &Value) -> Option<BarEvent> {
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    let close_time = data.get("time").or_else(|| data.get("timestamp"))?;
    let close_time_utc = to_i64(close_time)?;
    Some(BarEvent {
        symbol,
        close_time_utc,
        o: data.get("open").and_then(Value::as_f64).unwrap_or_default(),
        h: data.get("high").and_then(Value::as_f64).unwrap_or_default(),
        l: data.get("low").and_then(Value::as_f64).unwrap_or_default(),
        c: data.get("close").and_then(Value::as_f64).unwrap_or_default(),
        v: data.get("volume").and_then(Value::as_f64).unwrap_or_default(),
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
