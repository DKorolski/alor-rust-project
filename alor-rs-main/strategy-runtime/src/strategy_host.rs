use std::collections::HashMap;

use alor_protocol::{IntentClass, Side};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::live_guard::GatewayPhase;
use crate::{PaperExecutionMode, TradeMode};

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Classified {
        intent: Box<Intent>,
        intent_class: IntentClass,
    },
    Routed {
        intent: Box<Intent>,
        symbol: String,
    },
    Place {
        price: f64,
        qty: f64,
        side: Side,
        comment: Option<String>,
    },
    Market {
        qty: f64,
        side: Side,
        fill_price: Option<f64>,
        comment: Option<String>,
    },
    Cancel {
        order_id: i64,
    },
    Replace {
        order_id: i64,
        new_price: f64,
        new_qty: f64,
    },
    CreateStopLimit {
        side: Side,
        qty: f64,
        trigger_price: f64,
        price: f64,
        condition: alor_protocol::StopLimitCondition,
        stop_end_unix_time: i64,
        comment: Option<String>,
        instrument_group: Option<String>,
        check_duplicates: Option<bool>,
    },
    DeleteStopLimit {
        order_id: String,
        side: Option<Side>,
        check_duplicates: Option<bool>,
    },
}

impl Intent {
    pub fn with_class(self, intent_class: IntentClass) -> Self {
        Self::Classified {
            intent: Box::new(self),
            intent_class,
        }
    }

    pub fn with_symbol(self, symbol: impl Into<String>) -> Self {
        Self::Routed {
            intent: Box::new(self),
            symbol: symbol.into(),
        }
    }

    pub fn explicit_class(&self) -> Option<IntentClass> {
        match self {
            Intent::Classified { intent_class, .. } => Some(*intent_class),
            Intent::Routed { intent, .. } => intent.explicit_class(),
            _ => None,
        }
    }

    pub fn base_intent(&self) -> &Intent {
        match self {
            Intent::Classified { intent, .. } => intent.base_intent(),
            Intent::Routed { intent, .. } => intent.base_intent(),
            _ => self,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandPrepared {
    pub request_id: Uuid,
    pub intent_class: IntentClass,
    pub created_ts_utc: i64,
    pub symbol: String,
    pub action: String,
    pub target_order_id: Option<String>,
}

pub trait Strategy: Send + Sync {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent>;
    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &alor_protocol::CommandAck) -> Vec<Intent>;
    fn on_order(&mut self, ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent>;
    fn on_stop_order(&mut self, _ctx: &StrategyCtx, _ord: &StopOrderEvent) -> Vec<Intent> {
        Vec::new()
    }
    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent>;
    fn on_timer(&mut self, _ctx: &StrategyCtx, _now_ts_utc_ms: i64) -> Vec<Intent> {
        Vec::new()
    }
    fn on_bootstrap_snapshot(
        &mut self,
        _ctx: &StrategyCtx,
        _snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        Vec::new()
    }
    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        _state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        Vec::new()
    }
    fn warmup_from_history(&mut self, _ctx: &StrategyCtx, _bars: &[BarEvent]) -> usize {
        0
    }
    fn tracked_order_ids(&self) -> Vec<i64> {
        Vec::new()
    }
    fn intent_comment_tag(
        &self,
        _ctx: &StrategyCtx,
        _created_ts_utc: i64,
        _intent_class: IntentClass,
    ) -> Option<String> {
        None
    }
    fn on_command_prepared(&mut self, _ctx: &StrategyCtx, _command: &CommandPrepared) {}
    fn pending_request_ids(&self) -> Vec<Uuid> {
        Vec::new()
    }
    fn exit_risk_status(&self, has_open_position: bool) -> StrategyExitRiskStatus {
        let _ = has_open_position;
        StrategyExitRiskStatus::default()
    }
    fn risk_gate_session_finalizations(&self) -> Vec<RiskGateSessionFinalization> {
        Vec::new()
    }
    fn acknowledge_risk_gate_session_finalizations(&mut self, _session_dates: &[NaiveDate]) {}
    fn on_risk_gate_state(&mut self, _state: &RiskGateRuntimeState) {}
    fn drain_observation_journal_records(&mut self) -> Vec<serde_json::Value> {
        Vec::new()
    }
    fn state(&self) -> &crate::state::StrategyState;
    fn set_state(&mut self, state: crate::state::StrategyState);
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateSessionFinalization {
    pub session_date: NaiveDate,
    pub shadow_pnl_points: f64,
    pub shadow_trade_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateRuntimeState {
    pub profile_id: String,
    pub last_finalized_session_date: Option<NaiveDate>,
    pub rolling_sum_lb120: Option<f64>,
    pub mr_enabled_current_session: Option<bool>,
    pub mr_enabled_next_session: Option<bool>,
    pub ledger_rows_count: usize,
}

#[derive(Debug, Clone)]
pub struct StrategyCtx {
    pub strategy_id: String,
    pub portfolio: String,
    pub exchange: String,
    pub symbol: String,
    pub tick_size: f64,
    pub trade_mode: TradeMode,
    pub paper_execution_mode: PaperExecutionMode,
    pub allow_live_orders: bool,
    pub gateway_phase: GatewayPhase,
    pub position_qty: Option<f64>,
    pub(crate) event_ts_utc: i64,
    pub(crate) now_ts_utc: i64,
    pub(crate) last_bar_ts: Option<i64>,
}

impl StrategyCtx {
    pub fn event_ts_utc(&self) -> i64 {
        self.event_ts_utc
    }

    pub fn now_ts_utc(&self) -> i64 {
        self.now_ts_utc
    }

    pub fn last_bar_ts(&self) -> Option<i64> {
        self.last_bar_ts
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BarEvent {
    pub symbol: String,
    pub close_time_utc: i64,
    #[serde(default, alias = "c")]
    pub close: f64,
    #[serde(default)]
    pub o: f64,
    #[serde(default)]
    pub h: f64,
    #[serde(default)]
    pub l: f64,
    #[serde(default)]
    pub v: f64,
    pub origin: DataOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataOrigin {
    History,
    HistoryGap,
    Live,
    Replay,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderEvent {
    pub order_id: i64,
    pub request_id: Option<Uuid>,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub order_type: String,
    #[serde(default)]
    pub qty: f64,
    #[serde(default)]
    pub filled: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub existing: bool,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub ts_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TradeEvent {
    pub trade_id: String,
    pub order_id: i64,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub qty: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub commission: f64,
    #[serde(default)]
    pub existing: bool,
    #[serde(default)]
    pub ts_utc: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StopOrderEvent {
    pub stop_order_id: String,
    #[serde(default)]
    pub exchange_order_id: Option<i64>,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub qty: f64,
    #[serde(default)]
    pub filled: f64,
    #[serde(default)]
    pub stop_price: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub existing: bool,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub end_time: Option<i64>,
    #[serde(default)]
    pub ts_utc: i64,
}

impl Default for OrderEvent {
    fn default() -> Self {
        Self {
            order_id: 0,
            request_id: None,
            symbol: String::new(),
            status: String::new(),
            side: String::new(),
            order_type: String::new(),
            qty: 0.0,
            filled: 0.0,
            price: 0.0,
            existing: false,
            comment: None,
            ts_utc: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PositionEvent {
    pub symbol: String,
    pub qty: f64,
    #[serde(default)]
    pub existing: bool,
    #[serde(default)]
    pub avg_price: f64,
    #[serde(default)]
    pub ts_utc: i64,
}

#[derive(Debug, Clone)]
pub struct BootstrapSnapshot {
    pub positions_strategy: HashMap<String, PositionEvent>,
    pub working_orders_strategy: HashMap<i64, OrderEvent>,
    pub working_stop_orders_strategy: HashMap<String, StopOrderEvent>,
    pub snapshot_ts_utc: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RuntimeStateRestored {
    pub known_order_ids: Vec<i64>,
    pub pending_requests: Vec<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct StrategyExitRiskStatus {
    pub phase_override: Option<String>,
    pub exit_recovery_active: bool,
    pub operator_intervention_required: bool,
    pub open_risk_position_unflattened: bool,
}
