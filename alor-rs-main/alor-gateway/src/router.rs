use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::models::{BarEvent, DataOrigin, OrderEvent, PositionEvent, TradeEvent};

pub struct Router;

#[derive(Debug)]
pub enum RouterCommand {
    UpdateSubscribeWallclock {
        wallclock_ts: i64,
        history_origin: DataOrigin,
    },
}

#[derive(Debug)]
pub enum RouterControl {
    AuthError(i64),
}

pub struct RouterStreams {
    pub bars_rx: mpsc::Receiver<BarEvent>,
    pub positions_rx: mpsc::Receiver<PositionEvent>,
    pub orders_rx: mpsc::Receiver<OrderEvent>,
    pub trades_rx: mpsc::Receiver<TradeEvent>,
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
        let (trades_tx, trades_rx) = mpsc::channel(1024);
        let (control_tx, control_rx) = mpsc::channel(32);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let mut live_buffers: HashMap<String, BarEvent> = HashMap::new();
            let mut live_cutoff_ts: Option<i64> = None;
            let mut history_origin = DataOrigin::History;
            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(RouterCommand::UpdateSubscribeWallclock { wallclock_ts, history_origin: origin }) => {
                                live_cutoff_ts = Some(wallclock_ts - (2 * tf_sec));
                                history_origin = origin;
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

                        let bars = parse_bars(&value, live_cutoff_ts, history_origin);
                        if !bars.is_empty() {
                            for bar in bars {
                                match bar.origin {
                                    DataOrigin::History | DataOrigin::HistoryGap => {
                                        let _ = bars_tx.send(bar).await;
                                    }
                                    DataOrigin::Live => {
                                        let symbol = bar.symbol.clone();
                                        match live_buffers.get(&symbol) {
                                            None => {
                                                live_buffers.insert(symbol, bar);
                                            }
                                            Some(prev) if bar.close_time_utc == prev.close_time_utc => {
                                                trace!(
                                                    symbol = %symbol,
                                                    close_time_utc = bar.close_time_utc,
                                                    "live bar update buffered"
                                                );
                                                live_buffers.insert(symbol, bar);
                                            }
                                            Some(prev) if bar.close_time_utc > prev.close_time_utc => {
                                                let _ = bars_tx.send(prev.clone()).await;
                                                live_buffers.insert(symbol, bar);
                                            }
                                            Some(prev) => {
                                                debug!(
                                                    symbol = %symbol,
                                                    close_time_prev = prev.close_time_utc,
                                                    close_time_new = bar.close_time_utc,
                                                    "live bar out of order dropped"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            continue;
                        }

                        if let Some(trade) = parse_trade(&value) {
                            let _ = trades_tx.send(trade).await;
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
                trades_rx,
                control_rx,
            },
        )
    }
}

fn parse_bars(
    value: &Value,
    live_cutoff_ts: Option<i64>,
    history_origin: DataOrigin,
) -> Vec<BarEvent> {
    let Some(data) = value.get("data") else {
        return Vec::new();
    };

    if let Some(items) = data.as_array() {
        return items
            .iter()
            .filter_map(|item| parse_bar_item(item, live_cutoff_ts, history_origin))
            .collect();
    }

    parse_bar_item(data, live_cutoff_ts, history_origin)
        .into_iter()
        .collect()
}

fn parse_bar_item(
    data: &Value,
    live_cutoff_ts: Option<i64>,
    history_origin: DataOrigin,
) -> Option<BarEvent> {
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    let close_time = data.get("time").or_else(|| data.get("timestamp"))?;
    let close_time_utc = to_i64(close_time)?;
    let origin = match live_cutoff_ts {
        Some(cutoff) if close_time_utc <= cutoff => history_origin,
        Some(_) => DataOrigin::Live,
        None => DataOrigin::Live,
    };
    Some(BarEvent {
        symbol,
        close_time_utc,
        o: data.get("open").and_then(Value::as_f64).unwrap_or_default(),
        h: data.get("high").and_then(Value::as_f64).unwrap_or_default(),
        l: data.get("low").and_then(Value::as_f64).unwrap_or_default(),
        c: data
            .get("close")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        v: data
            .get("volume")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        origin,
    })
}

fn parse_position(value: &Value) -> Option<PositionEvent> {
    let data = value.get("data")?;
    if data.get("status").is_some()
        || data.get("orderId").is_some()
        || data.get("id").is_some()
        || data.get("price").is_some()
        || data.get("side").is_some()
        || data.get("type").is_some()
    {
        return None;
    }
    let avg_price = data
        .get("avgPrice")
        .or_else(|| data.get("avg_price"))
        .and_then(Value::as_f64)?;
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    Some(PositionEvent {
        symbol,
        qty: data.get("qty").and_then(Value::as_f64).unwrap_or_default(),
        avg_price,
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
        .and_then(to_i64)?;
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    Some(OrderEvent {
        order_id,
        request_id: None,
        symbol,
        status: data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_lowercase(),
        side: data
            .get("side")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        order_type: data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        qty: data
            .get("qty")
            .or_else(|| data.get("qtyUnits"))
            .or_else(|| data.get("qtyBatch"))
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        filled: data
            .get("filled")
            .or_else(|| data.get("filledQty"))
            .or_else(|| data.get("filledQtyUnits"))
            .or_else(|| data.get("filledQtyBatch"))
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        price: data
            .get("price")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        existing: data
            .get("existing")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ts_utc: data
            .get("updateTime")
            .or_else(|| data.get("transTime"))
            .or_else(|| data.get("timestamp"))
            .and_then(to_i64)
            .unwrap_or_else(|| Utc::now().timestamp()),
    })
}

fn parse_trade(value: &Value) -> Option<TradeEvent> {
    let data = value.get("data")?;
    let order_id = data
        .get("orderno")
        .or_else(|| data.get("orderNo"))
        .and_then(to_i64)?;
    let trade_id =
        data.get("id")
            .or_else(|| data.get("tradeId"))
            .and_then(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })?;
    let symbol = data
        .get("symbol")
        .or_else(|| data.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    let side = data
        .get("side")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_lowercase();
    Some(TradeEvent {
        trade_id,
        order_id,
        symbol,
        side,
        qty: data
            .get("qty")
            .or_else(|| data.get("qtyUnits"))
            .or_else(|| data.get("qtyBatch"))
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        price: data
            .get("price")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        commission: data
            .get("commission")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        existing: data
            .get("existing")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ts_utc: data
            .get("date")
            .or_else(|| data.get("timestamp"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_bars_respects_origin_cutoff() {
        let value = json!({
            "data": {
                "symbol": "IMOEXF",
                "time": 900,
                "open": 1.0,
                "high": 2.0,
                "low": 0.5,
                "close": 1.5,
                "volume": 10.0
            }
        });
        let bars = parse_bars(&value, Some(1000), DataOrigin::History);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].origin, DataOrigin::History);

        let value = json!({
            "data": {
                "symbol": "IMOEXF",
                "time": 1100,
                "open": 1.0,
                "high": 2.0,
                "low": 0.5,
                "close": 1.5,
                "volume": 10.0
            }
        });
        let bars = parse_bars(&value, Some(1000), DataOrigin::History);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].origin, DataOrigin::Live);
    }

    #[test]
    fn parse_position_event() {
        let value = json!({
            "data": {
                "symbol": "IMOEXF",
                "qty": 2.0,
                "avgPrice": 100.0,
                "timestamp": 1700000000
            }
        });
        let position = parse_position(&value).expect("position");
        assert_eq!(position.symbol, "IMOEXF");
        assert_eq!(position.qty, 2.0);
        assert_eq!(position.avg_price, 100.0);
        assert_eq!(position.ts_utc, 1700000000);
    }

    #[test]
    fn parse_order_event_lowercases_status() {
        let value = json!({
            "data": {
                "orderId": 42,
                "symbol": "IMOEXF",
                "status": "WORKING",
                "side": "buy",
                "type": "limit",
                "qty": 1.0,
                "filled": 0.0,
                "price": 100.0,
                "existing": false,
                "updateTime": 1700000001
            }
        });
        let order = parse_order(&value).expect("order");
        assert_eq!(order.status, "working");
    }

    #[test]
    fn parse_position_ignores_orders() {
        let value = json!({
            "data": {
                "id": "2033125883136551700",
                "symbol": "IMOEXF",
                "type": "limit",
                "side": "buy",
                "status": "working",
                "qty": 1.0,
                "filled": 0.0,
                "price": 2738.0
            }
        });
        assert!(parse_position(&value).is_none());
    }

    #[test]
    fn parse_order_event() {
        let value = json!({
            "data": {
                "orderId": 42,
                "symbol": "IMOEXF",
                "status": "working",
                "side": "buy",
                "type": "limit",
                "qty": 1.0,
                "filled": 1.0,
                "price": 99.5,
                "existing": true,
                "timestamp": 1700000001
            }
        });
        let order = parse_order(&value).expect("order");
        assert_eq!(order.order_id, 42);
        assert_eq!(order.symbol, "IMOEXF");
        assert_eq!(order.status, "working");
        assert_eq!(order.side, "buy");
        assert_eq!(order.order_type, "limit");
        assert_eq!(order.qty, 1.0);
        assert_eq!(order.filled, 1.0);
        assert_eq!(order.price, 99.5);
        assert!(order.existing);
        assert_eq!(order.ts_utc, 1700000001);
    }

    #[test]
    fn parse_trade_event() {
        let value = json!({
            "data": {
                "id": "trade-1",
                "orderno": 42,
                "symbol": "IMOEXF",
                "side": "buy",
                "qty": 1.5,
                "price": 2738.5,
                "commission": 0.5,
                "existing": true,
                "date": "2024-03-01T12:00:00"
            }
        });
        let trade = parse_trade(&value).expect("trade");
        assert_eq!(trade.trade_id, "trade-1");
        assert_eq!(trade.order_id, 42);
        assert_eq!(trade.symbol, "IMOEXF");
        assert_eq!(trade.side, "buy");
        assert_eq!(trade.qty, 1.5);
        assert_eq!(trade.price, 2738.5);
        assert_eq!(trade.commission, 0.5);
        assert!(trade.existing);
        assert_eq!(trade.ts_utc, 1709294400);
    }
}
