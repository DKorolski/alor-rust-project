use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrategyBar {
    pub symbol: String,
    pub time: DateTime<FixedOffset>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PositionSnapshot {
    pub symbol: String,
    pub qty: f64,
    pub avg_price: f64,
    pub ts_utc: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PositionsSnapshot {
    pub positions: HashMap<String, PositionSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OrderSnapshot {
    pub order_id: i64,
    pub symbol: String,
    pub status: String,
    pub filled: f64,
    pub price: f64,
    pub ts_utc: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OrdersSnapshot {
    pub orders: HashMap<i64, OrderSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrategyContext {
    pub positions: PositionsSnapshot,
    pub orders: OrdersSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Action {
    PlaceLimit {
        symbol: String,
        price: f64,
        qty: f64,
        side: Side,
    },
    Cancel {
        order_id: i64,
    },
    Replace {
        order_id: i64,
        new_price: f64,
        new_qty: f64,
    },
    Noop,
}

pub trait StrategyCore {
    fn on_bar(&mut self, bar: StrategyBar, ctx: StrategyContext) -> Vec<Action>;
}
