use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataOrigin {
    History,
    HistoryGap,
    Live,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarEvent {
    pub symbol: String,
    pub close_time_utc: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
    pub origin: DataOrigin,
}

pub type PositionEvent = alor_types::PositionSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEvent {
    pub trade_id: String,
    pub order_id: i64,
    pub symbol: String,
    pub side: String,
    pub qty: f64,
    pub price: f64,
    pub commission: f64,
    pub existing: bool,
    pub ts_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderEvent {
    pub order_id: i64,
    pub request_id: Option<Uuid>,
    pub symbol: String,
    pub status: String,
    pub side: String,
    pub order_type: String,
    pub qty: f64,
    pub filled: f64,
    pub price: f64,
    pub existing: bool,
    pub comment: Option<String>,
    pub ts_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopOrderEvent {
    pub stop_order_id: String,
    pub exchange_order_id: Option<i64>,
    pub symbol: String,
    pub status: String,
    pub side: String,
    pub qty: f64,
    pub filled: f64,
    pub stop_price: f64,
    pub price: f64,
    pub existing: bool,
    pub comment: Option<String>,
    pub end_time: Option<i64>,
    pub ts_utc: i64,
}

pub type PositionsSnapshot = alor_types::PositionsSnapshot;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrdersSnapshot {
    pub orders: HashMap<i64, OrderEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StopOrdersSnapshot {
    pub stop_orders: HashMap<String, StopOrderEvent>,
}
