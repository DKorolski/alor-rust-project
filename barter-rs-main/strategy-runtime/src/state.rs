use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{OrderEvent, PositionEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyState {
    Idle,
    Placed {
        place_request_id: Uuid,
        order_id: Option<i64>,
        cancel_due: bool,
        cancel_bar_ts: Option<i64>,
        placed_bar_ts: i64,
        last_bar_ts: i64,
        bars_waited: u32,
    },
    CancelSent {
        cancel_request_id: Uuid,
        order_id: i64,
        last_bar_ts: i64,
    },
    Done {
        last_bar_ts: i64,
    },
}

impl Default for StrategyState {
    fn default() -> Self {
        StrategyState::Idle
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RuntimeState {
    pub last_processed_bar_ts: HashMap<String, i64>,
    pub strategy_state: StrategyState,
    pub orders: HashMap<i64, OrderEvent>,
    pub positions: HashMap<String, PositionEvent>,
}

impl RuntimeState {
    pub fn is_duplicate_bar(&self, symbol: &str, bar_ts: i64) -> bool {
        self.last_processed_bar_ts
            .get(symbol)
            .is_some_and(|last_ts| bar_ts <= *last_ts)
    }

    pub fn update_last_bar_ts(&mut self, symbol: &str, bar_ts: i64) {
        self.last_processed_bar_ts
            .insert(symbol.to_string(), bar_ts);
    }
}
