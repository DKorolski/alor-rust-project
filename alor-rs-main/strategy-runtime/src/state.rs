use std::collections::HashMap;

use alor_protocol::Side;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{OrderEvent, PositionEvent, StopOrderEvent};

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
    EntryRecoveryVerificationPending {
        request_id: Uuid,
        side: Side,
        qty: f64,
        baseline_qty: f64,
        tp: Option<f64>,
        sl: Option<f64>,
        verification_started_ts: i64,
        transport_error_code: Option<String>,
        transport_error_msg: Option<String>,
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

fn default_session_gap_live_phase() -> SessionGapLivePhase {
    SessionGapLivePhase::Flat
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum StrategyState {
    #[default]
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
        #[serde(default)]
        session_date: Option<String>,
        #[serde(default)]
        traded_session: bool,
        #[serde(default)]
        prev_close: Option<f64>,
        #[serde(default)]
        yesterday_range: Option<f64>,
        #[serde(default)]
        pre_prev_close: Option<f64>,
        #[serde(default)]
        first_min_high: Option<f64>,
        #[serde(default)]
        first_min_low: Option<f64>,
        #[serde(default)]
        first_hour_price: Option<f64>,
        #[serde(default)]
        session_start_ts_utc: Option<i64>,
        #[serde(default)]
        session_end_ts_utc: Option<i64>,
        #[serde(default)]
        session_high: Option<f64>,
        #[serde(default)]
        session_low: Option<f64>,
        #[serde(default)]
        session_close: Option<f64>,
        #[serde(default)]
        last_dt_ts_utc: Option<i64>,
        #[serde(default = "default_session_gap_live_phase")]
        phase: SessionGapLivePhase,
        #[serde(default)]
        phase_last_change_ts_utc: Option<i64>,
        #[serde(default)]
        last_bar_ts: Option<i64>,
    },
    HybridIntradayRuntime {
        #[serde(default)]
        active_cycle_id: Option<String>,
        #[serde(default)]
        next_cycle_seq: u32,
        #[serde(default)]
        last_position_qty: f64,
        #[serde(default)]
        current_owner: Option<crate::strategies::hybrid_intraday::Owner>,
        #[serde(default)]
        current_side: Option<crate::strategies::hybrid_intraday::Side>,
        #[serde(default)]
        pending_entry_owner: Option<crate::strategies::hybrid_intraday::Owner>,
        #[serde(default)]
        pending_entry_side: Option<crate::strategies::hybrid_intraday::Side>,
        #[serde(default)]
        pending_entry_cycle_id: Option<String>,
        #[serde(default)]
        pending_entry_request_id: Option<Uuid>,
        #[serde(default)]
        pending_entry_created_ts_utc: Option<i64>,
        #[serde(default)]
        pending_exit_request_id: Option<Uuid>,
        #[serde(default)]
        pending_exit_created_ts_utc: Option<i64>,
        #[serde(default)]
        pending_tp_request_id: Option<Uuid>,
        #[serde(default)]
        pending_tp_created_ts_utc: Option<i64>,
        #[serde(default)]
        pending_sl_request_id: Option<Uuid>,
        #[serde(default)]
        pending_sl_created_ts_utc: Option<i64>,
        #[serde(default)]
        tp_order_id: Option<i64>,
        #[serde(default)]
        sl_stop_order_id: Option<String>,
        #[serde(default)]
        sl_exchange_order_id: Option<i64>,
        #[serde(default)]
        sl_triggered_ts: Option<i64>,
        #[serde(default)]
        mr_take_price: Option<f64>,
        #[serde(default)]
        mr_stop_price: Option<f64>,
        #[serde(default)]
        repair_deadline_ts: Option<i64>,
        #[serde(default)]
        next_repair_at_ts: Option<i64>,
        #[serde(default)]
        repair_backoff_level: u32,
        #[serde(default)]
        repair_attempts: u32,
        #[serde(default)]
        safe_mode_close_only: bool,
        #[serde(default)]
        safe_mode_reason: Option<String>,
        #[serde(default)]
        entry_ready: bool,
        #[serde(default)]
        last_bar_close: Option<f64>,
        #[serde(default)]
        last_day_local: Option<String>,
        #[serde(default)]
        current_day_high: Option<f64>,
        #[serde(default)]
        current_day_low: Option<f64>,
        #[serde(default)]
        prev_day_range: Option<f64>,
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

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RuntimeState {
    pub last_processed_bar_ts: HashMap<String, i64>,
    pub strategy_state: StrategyState,
    pub orders: HashMap<i64, OrderEvent>,
    #[serde(default)]
    pub stop_orders: HashMap<String, StopOrderEvent>,
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

#[cfg(test)]
mod tests {
    use super::{SessionGapLivePhase, StrategyState};

    #[test]
    fn session_gap_standalone_deserializes_with_missing_new_fields() {
        let legacy_json = r#"{
            "SessionGapStandalone": {
                "session_date": "2025-12-05",
                "traded_session": true,
                "prev_close": 100.0,
                "yesterday_range": 2.0,
                "pre_prev_close": 99.0,
                "first_min_high": 101.0,
                "first_min_low": 98.0,
                "first_hour_price": 100.5,
                "session_start_ts_utc": 1,
                "session_end_ts_utc": 2,
                "last_dt_ts_utc": 3,
                "phase": "Flat"
            }
        }"#;
        let state: StrategyState = serde_json::from_str(legacy_json).unwrap();

        match state {
            StrategyState::SessionGapStandalone {
                phase,
                session_high,
                session_low,
                session_close,
                phase_last_change_ts_utc,
                last_bar_ts,
                ..
            } => {
                assert!(matches!(phase, SessionGapLivePhase::Flat));
                assert_eq!(session_high, None);
                assert_eq!(session_low, None);
                assert_eq!(session_close, None);
                assert_eq!(phase_last_change_ts_utc, None);
                assert_eq!(last_bar_ts, None);
            }
            other => panic!("unexpected state: {other:?}"),
        }
    }
}
