use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataOrigin {
    History,
    HistoryGap,
    Live,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct PositionEvent {
    pub symbol: String,
    pub qty: f64,
    pub avg_price: f64,
    pub ts_utc: i64,
}

#[derive(Debug, Clone)]
pub struct OrderEvent {
    pub order_id: i64,
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

#[derive(Debug, Clone, Default)]
pub struct PositionsSnapshot {
    pub positions: HashMap<String, PositionEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct OrdersSnapshot {
    pub orders: HashMap<i64, OrderEvent>,
}
