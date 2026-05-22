use std::collections::HashMap;

use alor_protocol::Side;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::strategy_host::{OrderEvent, PositionEvent, StopOrderEvent};

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
    EntryDeferredWindowClosed {
        side: Side,
        qty: f64,
        deferred_ts_utc: i64,
        original_request_id: Uuid,
        last_error_code: Option<String>,
        last_error_msg: Option<String>,
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
        price: f64,
        baseline_qty: f64,
        reason: String,
        sent_ts: i64,
        acked: bool,
    },
    ExitDeferredWindowClosed {
        side: Side,
        qty: f64,
        baseline_qty: f64,
        reason: String,
        deferred_ts_utc: i64,
        original_request_id: Uuid,
        last_error_code: Option<String>,
        last_error_msg: Option<String>,
    },
    ExitRecoveryPending {
        request_id: Uuid,
        side: Side,
        qty: f64,
        price: f64,
        baseline_qty: f64,
        reason: String,
        sent_ts: i64,
        acked: bool,
        retry_attempt: u32,
        last_error_code: Option<String>,
        last_error_msg: Option<String>,
    },
    CloseOnlyDegraded {
        side: Side,
        qty: f64,
        baseline_qty: f64,
        reason: String,
        entered_ts_utc: i64,
        retry_attempts_exhausted: u32,
        last_error_code: Option<String>,
        last_error_msg: Option<String>,
        operator_intervention_required: bool,
    },
    Blocked {
        reason: String,
        ts_utc: i64,
    },
}

fn default_session_gap_live_phase() -> SessionGapLivePhase {
    SessionGapLivePhase::Flat
}

fn default_ri_author4142_phase() -> String {
    "flat".to_string()
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
        deferred_entry_owner: Option<crate::strategies::hybrid_intraday::Owner>,
        #[serde(default)]
        deferred_entry_side: Option<crate::strategies::hybrid_intraday::Side>,
        #[serde(default)]
        deferred_entry_cycle_id: Option<String>,
        #[serde(default)]
        deferred_entry_entry_style: Option<crate::strategies::hybrid_intraday::EntryStyle>,
        #[serde(default)]
        deferred_entry_reason: Option<crate::strategies::hybrid_intraday::ReasonCode>,
        #[serde(default)]
        deferred_entry_stop_price: Option<f64>,
        #[serde(default)]
        deferred_entry_take_price: Option<f64>,
        #[serde(default)]
        deferred_entry_ts_utc: Option<i64>,
        #[serde(default)]
        deferred_entry_request_id: Option<Uuid>,
        #[serde(default)]
        pending_exit_request_id: Option<Uuid>,
        #[serde(default)]
        pending_exit_created_ts_utc: Option<i64>,
        #[serde(default)]
        deferred_exit_owner: Option<crate::strategies::hybrid_intraday::Owner>,
        #[serde(default)]
        deferred_exit_reason: Option<crate::strategies::hybrid_intraday::ReasonCode>,
        #[serde(default)]
        deferred_exit_cycle_id: Option<String>,
        #[serde(default)]
        deferred_exit_ts_utc: Option<i64>,
        #[serde(default)]
        deferred_exit_request_id: Option<Uuid>,
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
        prev_day_close: Option<f64>,
        #[serde(default)]
        last_day_local: Option<String>,
        #[serde(default)]
        current_day_high: Option<f64>,
        #[serde(default)]
        current_day_low: Option<f64>,
        #[serde(default)]
        current_day_close: Option<f64>,
        #[serde(default)]
        prev_day_range: Option<f64>,
        #[serde(default)]
        prev_day_return: Option<f64>,
        #[serde(default)]
        day_before_close: Option<f64>,
        #[serde(default)]
        today_start_local: Option<String>,
        #[serde(default)]
        was_long_today: bool,
        #[serde(default)]
        was_short_today: bool,
        #[serde(default)]
        overnight_exit_armed_date: Option<String>,
        #[serde(default)]
        risk_gate_shadow_session_date: Option<String>,
        #[serde(default)]
        risk_gate_shadow_pnl_points: f64,
        #[serde(default)]
        risk_gate_shadow_trade_count: u32,
        #[serde(default)]
        risk_gate_shadow_entry_ts_utc: Option<i64>,
        #[serde(default)]
        risk_gate_shadow_entry_price: Option<f64>,
        #[serde(default)]
        risk_gate_shadow_side: Option<crate::strategies::hybrid_intraday::Side>,
        #[serde(default)]
        risk_gate_shadow_target_price: Option<f64>,
        #[serde(default)]
        risk_gate_shadow_stop_price: Option<f64>,
        #[serde(default)]
        risk_gate_pending_session_date: Option<String>,
        #[serde(default)]
        risk_gate_pending_shadow_pnl_points: f64,
        #[serde(default)]
        risk_gate_pending_shadow_trade_count: u32,
        #[serde(default)]
        risk_gate_mr_enabled_current_session: Option<bool>,
        #[serde(default)]
        risk_gate_rolling_sum_lb120: Option<f64>,
        #[serde(default)]
        risk_gate_last_finalized_session_date: Option<String>,
        #[serde(default)]
        risk_gate_ledger_rows_count: usize,
    },
    AlorUsdrubfHybrid {
        #[serde(default)]
        lifecycle_stage: String,
        #[serde(default)]
        last_bar_ts: Option<i64>,
        #[serde(default)]
        last_processed_bar_ts: Option<i64>,
        #[serde(default)]
        bootstrap_seen: bool,
        #[serde(default)]
        runtime_state_restored: bool,
        #[serde(default)]
        live_ready: bool,
        #[serde(default)]
        hybrid_state: String,
        #[serde(default)]
        current_date_local: Option<String>,
        #[serde(default)]
        day_open: Option<f64>,
        #[serde(default)]
        day_high: Option<f64>,
        #[serde(default)]
        day_low: Option<f64>,
        #[serde(default)]
        day_volume_sum: f64,
        #[serde(default)]
        day_vwap_num: f64,
        #[serde(default)]
        session_start_local: Option<String>,
        #[serde(default)]
        bo_was_long_today: bool,
        #[serde(default)]
        bo_was_short_today: bool,
        #[serde(default)]
        cash: f64,
        #[serde(default)]
        pending_entry_owner: Option<String>,
        #[serde(default)]
        pending_entry_side: Option<String>,
        #[serde(default)]
        pending_request_ids: Vec<Uuid>,
        #[serde(default)]
        tracked_order_ids: Vec<i64>,
        #[serde(default)]
        entry_intent_inflight: bool,
        #[serde(default)]
        pending_entry_reason: Option<String>,
        #[serde(default)]
        pending_entry_scale_at_signal: Option<f64>,
        #[serde(default)]
        pending_entry_signal_price: Option<f64>,
        #[serde(default)]
        pending_entry_stop1: Option<f64>,
        #[serde(default)]
        pending_entry_stop2: Option<f64>,
        #[serde(default)]
        open_position_owner: Option<String>,
        #[serde(default)]
        open_position_side: Option<String>,
        #[serde(default)]
        exit_intent_inflight: bool,
        #[serde(default)]
        open_position_qty: f64,
        #[serde(default)]
        open_position_entry_ts: Option<String>,
        #[serde(default)]
        open_position_entry_price: Option<f64>,
        #[serde(default)]
        open_position_stop_price: Option<f64>,
        #[serde(default)]
        open_position_take_price: Option<f64>,
        #[serde(default)]
        open_position_stop1: Option<f64>,
        #[serde(default)]
        open_position_stop2: Option<f64>,
    },
    #[serde(rename = "RiAuthor41_42Live")]
    RiAuthor4142Live {
        #[serde(default)]
        mode: String,
        #[serde(default)]
        profile_id: String,
        #[serde(default)]
        timeframe: String,
        #[serde(default)]
        allow_order_emission: bool,
        #[serde(default)]
        execution_path: String,
        #[serde(default)]
        last_bar_ts: Option<i64>,
        #[serde(default)]
        last_model_bar_ts: Option<i64>,
        #[serde(default)]
        model_bars_seen: u64,
        #[serde(default)]
        suppressed_service_bars: u64,
        #[serde(default)]
        model_decisions_seen: u64,
        #[serde(default)]
        last_decision_key: Option<String>,
        #[serde(default = "default_ri_author4142_phase")]
        phase: String,
        #[serde(default)]
        current_component: Option<String>,
        #[serde(default)]
        current_side: Option<String>,
        #[serde(default)]
        current_cycle_id: Option<String>,
        #[serde(default)]
        current_entry_ts_local: Option<String>,
        #[serde(default)]
        current_exit_ts_local: Option<String>,
        #[serde(default)]
        last_transition_reason: Option<String>,
        #[serde(default)]
        live_adapter_enabled: bool,
        #[serde(default)]
        pending_entry_request_id: Option<uuid::Uuid>,
        #[serde(default)]
        pending_exit_request_id: Option<uuid::Uuid>,
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

const fn default_strategy_state_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyStateEnvelope {
    pub strategy_kind: crate::StrategyKind,
    #[serde(default = "default_strategy_state_version")]
    pub state_version: u32,
    pub payload: StrategyState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrategyStateEnvelopeCompat {
    Legacy(StrategyState),
    Envelope(StrategyStateEnvelope),
}

impl StrategyStateEnvelopeCompat {
    pub fn from_strategy_state(strategy_kind: crate::StrategyKind, payload: StrategyState) -> Self {
        Self::Envelope(StrategyStateEnvelope {
            strategy_kind,
            state_version: default_strategy_state_version(),
            payload,
        })
    }

    pub fn into_payload(self) -> StrategyState {
        match self {
            Self::Legacy(payload) => payload,
            Self::Envelope(envelope) => envelope.payload,
        }
    }

    pub fn strategy_kind(&self) -> Option<crate::StrategyKind> {
        match self {
            Self::Legacy(_) => None,
            Self::Envelope(envelope) => Some(envelope.strategy_kind),
        }
    }
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
    use super::{
        SessionGapLivePhase, StrategyState, StrategyStateEnvelope, StrategyStateEnvelopeCompat,
    };
    use crate::StrategyKind;

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

    #[test]
    fn hybrid_intraday_runtime_deserializes_with_missing_new_fields() {
        let legacy_json = r#"{
            "HybridIntradayRuntime": {
                "active_cycle_id": null,
                "next_cycle_seq": 0,
                "last_position_qty": 0.0,
                "current_owner": null,
                "current_side": null,
                "pending_entry_owner": null,
                "pending_entry_side": null,
                "pending_entry_cycle_id": null,
                "pending_entry_request_id": null,
                "pending_entry_created_ts_utc": null,
                "deferred_entry_owner": null,
                "deferred_entry_side": null,
                "deferred_entry_cycle_id": null,
                "deferred_entry_entry_style": null,
                "deferred_entry_reason": null,
                "deferred_entry_stop_price": null,
                "deferred_entry_take_price": null,
                "deferred_entry_ts_utc": null,
                "deferred_entry_request_id": null,
                "pending_exit_request_id": null,
                "pending_exit_created_ts_utc": null,
                "deferred_exit_owner": null,
                "deferred_exit_reason": null,
                "deferred_exit_cycle_id": null,
                "deferred_exit_ts_utc": null,
                "deferred_exit_request_id": null,
                "pending_tp_request_id": null,
                "pending_tp_created_ts_utc": null,
                "pending_sl_request_id": null,
                "pending_sl_created_ts_utc": null,
                "tp_order_id": null,
                "sl_stop_order_id": null,
                "sl_exchange_order_id": null,
                "sl_triggered_ts": null,
                "mr_take_price": null,
                "mr_stop_price": null,
                "repair_deadline_ts": null,
                "next_repair_at_ts": null,
                "repair_backoff_level": 0,
                "repair_attempts": 0,
                "safe_mode_close_only": false,
                "safe_mode_reason": null,
                "entry_ready": false,
                "last_bar_close": 101.5,
                "last_day_local": "2026-03-08",
                "current_day_high": 102.0,
                "current_day_low": 100.5,
                "prev_day_range": 12.5
            }
        }"#;
        let state: StrategyState = serde_json::from_str(legacy_json).unwrap();

        match state {
            StrategyState::HybridIntradayRuntime {
                prev_day_close,
                current_day_close,
                prev_day_return,
                day_before_close,
                today_start_local,
                was_long_today,
                was_short_today,
                overnight_exit_armed_date,
                ..
            } => {
                assert_eq!(prev_day_close, None);
                assert_eq!(current_day_close, None);
                assert_eq!(prev_day_return, None);
                assert_eq!(day_before_close, None);
                assert_eq!(today_start_local, None);
                assert!(!was_long_today);
                assert!(!was_short_today);
                assert_eq!(overnight_exit_armed_date, None);
            }
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn ri_author4142_live_deserializes_with_flat_phase_default() {
        let legacy_json = r#"{
            "RiAuthor41_42Live": {
                "mode": "shadow",
                "profile_id": "ri_author41_42_primary_combo_cost2",
                "timeframe": "10m",
                "allow_order_emission": false,
                "execution_path": "action_scoped_only",
                "last_bar_ts": null,
                "last_model_bar_ts": null,
                "model_bars_seen": 12,
                "suppressed_service_bars": 0,
                "model_decisions_seen": 2,
                "last_decision_key": null,
                "live_adapter_enabled": false
            }
        }"#;
        let state: StrategyState = serde_json::from_str(legacy_json).unwrap();

        match state {
            StrategyState::RiAuthor4142Live {
                phase,
                current_component,
                current_side,
                current_cycle_id,
                current_entry_ts_local,
                current_exit_ts_local,
                last_transition_reason,
                ..
            } => {
                assert_eq!(phase, "flat");
                assert_eq!(current_component, None);
                assert_eq!(current_side, None);
                assert_eq!(current_cycle_id, None);
                assert_eq!(current_entry_ts_local, None);
                assert_eq!(current_exit_ts_local, None);
                assert_eq!(last_transition_reason, None);
            }
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn strategy_state_envelope_compat_deserializes_legacy_shape() {
        let legacy_json = r#"{
            "SessionGapStandalone": {
                "phase": "Flat"
            }
        }"#;

        let compat: StrategyStateEnvelopeCompat = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(compat.strategy_kind(), None);
        assert!(matches!(
            compat.into_payload(),
            StrategyState::SessionGapStandalone { .. }
        ));
    }

    #[test]
    fn strategy_state_envelope_deserializes_new_shape() {
        let envelope_json = r#"{
            "strategy_kind": "session_gap_standalone",
            "state_version": 1,
            "payload": {
                "SessionGapStandalone": {
                    "phase": "Flat"
                }
            }
        }"#;

        let envelope: StrategyStateEnvelope = serde_json::from_str(envelope_json).unwrap();
        assert_eq!(envelope.strategy_kind, StrategyKind::SessionGapStandalone);
        assert_eq!(envelope.state_version, 1);
        assert!(matches!(
            envelope.payload,
            StrategyState::SessionGapStandalone { .. }
        ));
    }

    #[test]
    fn strategy_state_envelope_defaults_state_version_when_missing() {
        let envelope_json = r#"{
            "strategy_kind": "hybrid_intraday",
            "payload": "Idle"
        }"#;

        let envelope: StrategyStateEnvelope = serde_json::from_str(envelope_json).unwrap();
        assert_eq!(envelope.strategy_kind, StrategyKind::HybridIntraday);
        assert_eq!(envelope.state_version, 1);
        assert!(matches!(envelope.payload, StrategyState::Idle));
    }
}
