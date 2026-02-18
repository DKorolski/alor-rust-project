use std::collections::HashMap;

use alor_protocol::Side;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{OrderEvent, PositionEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionGapLivePhase {
    Flat,
    PendingEntry {
        request_id: Uuid,
        side: Side,
        qty: f64,
        baseline_qty: f64,
        tp: Option<f64>,
        sl: Option<f64>,
        sent_ts: i64,
        acked: bool,
    },
    InPosition {
        side: Side,
        qty: f64,
        avg_price: f64,
        baseline_qty: f64,
        tp: Option<f64>,
        sl: Option<f64>,
        opened_ts: i64,
    },
    PendingExit {
        request_id: Uuid,
        side: Side,
        qty: f64,
        baseline_qty: f64,
        reason: String,
        sent_ts: i64,
        acked: bool,
    },
    Blocked {
        reason: String,
        ts_utc: i64,
    },
}

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
    MarketBuyPending {
        buy_request_id: Uuid,
        baseline_qty: Option<f64>,
        close_trigger: crate::CloseTrigger,
        pending_bar_ts: i64,
        last_bar_ts: i64,
    },
    MarketBuySent {
        buy_request_id: Uuid,
        baseline_qty: Option<f64>,
        close_trigger: crate::CloseTrigger,
        buy_bar_ts: i64,
        last_bar_ts: i64,
    },
    MarketCloseSent {
        close_request_id: Uuid,
        baseline_qty: Option<f64>,
        last_bar_ts: i64,
    },
    MarketLivePendingEntry {
        request_guid: Uuid,
        side: alor_protocol::Side,
        qty: f64,
        baseline_qty: f64,
        close_trigger: crate::CloseTrigger,
        sent_ts: i64,
        acked: bool,
        entry_confirmed_ts: Option<i64>,
        last_bar_ts: i64,
    },
    MarketLiveInPosition {
        side: alor_protocol::Side,
        qty: f64,
        avg_price: f64,
        baseline_qty: f64,
        close_trigger: crate::CloseTrigger,
        opened_ts: i64,
        last_bar_ts: i64,
    },
    MarketLivePendingExit {
        request_guid: Uuid,
        reason: String,
        side: alor_protocol::Side,
        qty: f64,
        baseline_qty: f64,
        sent_ts: i64,
        acked: bool,
        last_bar_ts: i64,
    },
    Blocked {
        reason: String,
        last_bar_ts: i64,
    },
    SessionGapStandalone {
        session_date: Option<String>,
        traded_session: bool,
        prev_close: Option<f64>,
        yesterday_range: Option<f64>,
        pre_prev_close: Option<f64>,
        first_min_high: Option<f64>,
        first_min_low: Option<f64>,
        first_hour_price: Option<f64>,
        session_start_ts_utc: Option<i64>,
        session_end_ts_utc: Option<i64>,
        last_dt_ts_utc: Option<i64>,
        phase: SessionGapLivePhase,
        last_bar_ts: Option<i64>,
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
    pub last_trade_ts: Option<i64>,
    pub last_trade_id: Option<String>,
    pub seen_trade_ids: Vec<String>,
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
