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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionEvent {
    pub symbol: String,
    pub qty: f64,
    pub avg_price: f64,
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
    pub ts_utc: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PositionsSnapshot {
    pub positions: HashMap<String, PositionEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrdersSnapshot {
    pub orders: HashMap<i64, OrderEvent>,
}
