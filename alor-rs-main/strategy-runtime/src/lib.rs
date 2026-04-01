pub mod config;
pub mod health_server;
pub mod live_guard;
pub mod redis_transport;
pub mod runtime;
pub mod state;
pub mod strategies;
pub mod trade_ledger;

use std::collections::HashMap;
use std::time::Instant;

use alor_protocol::{CommandAction, IntentClass, OrderCommand, PlaceOrder, Side};
use alor_types::TradingPeriods;
use chrono::{Duration, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::strategies::hybrid_intraday::{
    BreakoutEodMode, HybridOrchestratorConfig, IntradayBreakoutConfig, MeanReversionConfig,
    MinRangeMode,
};
use crate::strategies::hybrid_intraday_runtime::HybridIntradayRuntimeConfig;
use crate::strategies::limit_cancel::LimitCancelConfig;
use crate::strategies::market_buy_and_close::{
    MarketBuyAndCloseConfig, MarketBuyAndCloseLiveOrderStyle,
};
use crate::strategies::mock_live_probe::{MockLiveProbeConfig, MockLiveProbeMode};
use crate::strategies::session_gap_standalone::SessionGapStandaloneConfig;
use crate::strategies::toy_session_timing::ToySessionTimingConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Classified {
        intent: Box<Intent>,
        intent_class: IntentClass,
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

    pub fn explicit_class(&self) -> Option<IntentClass> {
        match self {
            Intent::Classified { intent_class, .. } => Some(*intent_class),
            _ => None,
        }
    }

    pub fn base_intent(&self) -> &Intent {
        match self {
            Intent::Classified { intent, .. } => intent.base_intent(),
            _ => self,
        }
    }
}

pub trait Strategy: Send + Sync {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent>;
    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &alor_protocol::CommandAck) -> Vec<Intent>;
    fn on_order(&mut self, ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent>;
    fn on_stop_order(&mut self, _ctx: &StrategyCtx, _ord: &StopOrderEvent) -> Vec<Intent> {
        Vec::new()
    }
    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent>;
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
    fn state(&self) -> &crate::state::StrategyState;
    fn set_state(&mut self, state: crate::state::StrategyState);
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
    pub gateway_phase: crate::live_guard::GatewayPhase,
    pub position_qty: Option<f64>,
    event_ts_utc: i64,
    now_ts_utc: i64,
    last_bar_ts: Option<i64>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeConfig {
    pub redis_url: String,
    pub source: String,
    pub portfolio: String,
    pub exchange: String,
    pub streams: StreamNames,
    pub consumer_group: String,
    pub consumer_name: String,
    pub trade_mode: TradeMode,
    pub allow_live_orders: bool,
    pub allow_paper_orders: bool,
    pub guard_log_interval_ms: u64,
    pub still_blocked_log_period_sec: u64,
    pub gateway_health_stale_sec: u64,
    pub require_gateway_ready: bool,
    pub bootstrap_dump: bool,
    pub health: HealthServerConfig,
    pub read: ReadConfig,
    pub trim: TrimConfig,
    pub strategy: StrategyConfig,
    pub paper: PaperConfig,
    pub backtest: BacktestConfig,
    pub replay: ReplayConfig,
    pub reset_state_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthServerConfig {
    pub enabled: bool,
    pub listen_addr: String,
    pub expose_metrics: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeHealthSnapshot {
    pub uptime_start: Instant,
    pub runtime_phase: String,
    pub live_guard_status: String,
    pub live_guard_reasons: Vec<String>,
    pub live_guard_last_change_ts_utc: i64,
    pub gateway_health_last_ts_utc: Option<i64>,
    pub gateway_health_age_sec: Option<i64>,
    pub gateway_ready: Option<bool>,
    pub ws_connected: Option<bool>,
    pub cws_authorized: Option<bool>,
    pub gateway_scheduler_state: Option<String>,
    pub scheduler_state: String,
    pub now_local: String,
    pub scheduler_note: Option<String>,
    pub timezone_offset_hours: i32,
    pub last_bar_ts_utc: Option<i64>,
    pub last_ack_ts_utc: Option<i64>,
    pub last_intent_ts_utc: Option<i64>,
    pub orders_mode: String,
    pub allow_live_orders: bool,
    pub allow_paper_orders: bool,
    pub require_gateway_ready: bool,
    pub exit_recovery_active: bool,
    pub close_only_degraded: bool,
    pub operator_intervention_required: bool,
    pub open_risk_position_unflattened: bool,
    pub readiness: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamNames {
    pub bars: String,
    pub orders: String,
    pub trades: String,
    pub positions: String,
    pub commands: String,
    pub acks: String,
    pub snapshots: Option<String>,
    pub health: Option<String>,
    pub dlq_prefix: String,
    pub runtime_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadConfig {
    pub block_ms: usize,
    pub claim_idle_ms: usize,
    pub claim_batch: usize,
    pub poll_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrimConfig {
    pub bars: usize,
    pub orders: usize,
    pub trades: usize,
    pub positions: usize,
    pub commands: usize,
    pub acks: usize,
    pub health: usize,
    pub runtime_state: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyConfig {
    pub strategy_id: String,
    pub strategy_kind: StrategyKind,
    pub symbol: String,
    pub qty: f64,
    pub side: Side,
    #[serde(default)]
    pub live_order_style: MarketBuyAndCloseLiveOrderStyle,
    #[serde(default)]
    pub marketable_limit_offset_ticks: i64,
    pub place_offset_ticks: i64,
    pub tick_size: f64,
    pub max_wait_bars_for_ack: u32,
    pub close_trigger: CloseTrigger,
    pub entry_ack_timeout_ms: u64,
    pub entry_fill_timeout_ms: u64,
    pub exit_ack_timeout_ms: u64,
    pub exit_fill_timeout_ms: u64,
    pub session_open_hour: u32,
    pub session_open_minute: u32,
    pub session_close_hour: u32,
    pub session_close_minute: u32,
    pub entry_after_open_min: u32,
    pub exit_before_close_min: u32,
    pub timezone_offset_hours: i32,
    #[serde(default)]
    pub trading_periods: Option<TradingPeriods>,
    #[serde(default)]
    pub max_silence_bars_sec: u64,
    pub session_gap_k_long: f64,
    pub session_gap_k_short: f64,
    pub session_gap_wait_hours: i64,
    pub session_gap_k_tp_long: f64,
    pub session_gap_k_sl_long: f64,
    pub session_gap_k_tp_short: f64,
    pub session_gap_k_sl_short: f64,
    pub session_gap_long_ex_pct: f64,
    pub session_gap_short_ex_pct: f64,
    pub session_gap_start_cash: f64,
    pub session_gap_cash_factor: f64,
    pub session_gap_max_entry_hour: u32,
    pub session_gap_close_hour: u32,
    pub session_gap_close_minute: u32,
    pub session_gap_min: f64,
    pub session_gap_exit_offset_min: i64,
    pub session_gap_work_weekends: bool,
    #[serde(default)]
    pub hybrid_intraday: HybridIntradaySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HybridIntradaySettings {
    pub mr_min_range_long: f64,
    pub mr_max_range_long: f64,
    pub mr_k_long: f64,
    pub mr_take_k_long: f64,
    pub mr_stop_k_long: f64,
    pub mr_min_range_short: f64,
    pub mr_max_range_short: f64,
    pub mr_k_short: f64,
    pub mr_take_k_short: f64,
    pub mr_stop_k_short: f64,
    pub mr_session_end_time: String,
    pub mr_exit_offset_min: i64,
    pub bo_k: f64,
    pub bo_stop1_range: f64,
    pub bo_stop2_range: f64,
    pub bo_big_move_threshold: f64,
    pub bo_min_range: f64,
    pub bo_min_range_mode: String,
    pub bo_exclude_weekends: bool,
    pub bo_wait_hours: f64,
    pub orchestrator_breakout_eod_mode: String,
    pub orchestrator_breakout_overnight_exit_time: String,
    pub repair_deadline_sec: u64,
    pub sl_escalate_timeout_sec: u64,
    pub max_repair_retries: u32,
    pub repair_backoff_base_sec: u64,
    pub repair_backoff_max_sec: u64,
    pub pending_timeout_sec: u64,
    pub stop_end_buffer_sec: u64,
}

impl Default for HybridIntradaySettings {
    fn default() -> Self {
        Self {
            mr_min_range_long: 0.013,
            mr_max_range_long: 0.035,
            mr_k_long: 0.032,
            mr_take_k_long: 0.11,
            mr_stop_k_long: 0.44,
            mr_min_range_short: 0.010,
            mr_max_range_short: 0.045,
            mr_k_short: 0.055,
            mr_take_k_short: 0.16,
            mr_stop_k_short: 0.43,
            mr_session_end_time: "11:59:00".to_string(),
            mr_exit_offset_min: 5,
            bo_k: 0.65,
            bo_stop1_range: 0.51,
            bo_stop2_range: 0.35,
            bo_big_move_threshold: 0.025,
            bo_min_range: 1.01,
            bo_min_range_mode: "absolute".to_string(),
            bo_exclude_weekends: true,
            bo_wait_hours: 3.0,
            orchestrator_breakout_eod_mode: "same_day".to_string(),
            orchestrator_breakout_overnight_exit_time: "09:30:00".to_string(),
            repair_deadline_sec: 180,
            sl_escalate_timeout_sec: 30,
            max_repair_retries: 3,
            repair_backoff_base_sec: 5,
            repair_backoff_max_sec: 60,
            pending_timeout_sec: 60,
            stop_end_buffer_sec: 60,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TradeMode {
    Live,
    Paper,
    Backtest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    LimitCancel,
    MarketBuyAndClose,
    ToySessionTiming,
    SessionGapStandalone,
    MockLiveProbe,
    HybridIntraday,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloseTrigger {
    NextBar,
    PositionUpdate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaperOutput {
    Stdout,
    File,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaperExecutionMode {
    LiveOnly,
    HistorySim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaperConfig {
    pub enabled: bool,
    pub output: PaperOutput,
    pub execution_mode: PaperExecutionMode,
    pub file_path: String,
    pub trades_csv: String,
    pub summary_json: String,
    pub append: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayConfig {
    pub enabled: bool,
    pub bars_csv_path: Option<String>,
    pub reference_trades_csv_path: Option<String>,
    pub output_dir: String,
    pub price_tolerance: f64,
    pub strict_dedup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestConfig {
    pub enabled: bool,
    pub trade_log: String,
    pub trades_csv: String,
    pub summary_json: String,
    pub append: bool,
}

impl StrategyConfig {
    pub fn to_limit_cancel_config(&self) -> LimitCancelConfig {
        LimitCancelConfig {
            symbol: self.symbol.clone(),
            tick_size: self.tick_size,
            offset_ticks: self.place_offset_ticks,
            qty: self.qty,
            side: self.side,
            max_wait_bars_for_ack: self.max_wait_bars_for_ack,
        }
    }

    pub fn to_market_buy_and_close_config(&self) -> MarketBuyAndCloseConfig {
        MarketBuyAndCloseConfig {
            symbol: self.symbol.clone(),
            qty: self.qty,
            side: self.side,
            live_order_style: self.live_order_style,
            tick_size: self.tick_size,
            marketable_limit_offset_ticks: self.marketable_limit_offset_ticks,
            close_trigger: self.close_trigger,
            entry_ack_timeout_ms: self.entry_ack_timeout_ms,
            entry_fill_timeout_ms: self.entry_fill_timeout_ms,
            exit_ack_timeout_ms: self.exit_ack_timeout_ms,
            exit_fill_timeout_ms: self.exit_fill_timeout_ms,
        }
    }

    pub fn to_toy_session_timing_config(&self) -> ToySessionTimingConfig {
        ToySessionTimingConfig {
            symbol: self.symbol.clone(),
            qty: self.qty,
            entry_side: self.side,
            session_open_hour: self.session_open_hour,
            session_open_minute: self.session_open_minute,
            session_close_hour: self.session_close_hour,
            session_close_minute: self.session_close_minute,
            entry_after_open_min: self.entry_after_open_min,
            exit_before_close_min: self.exit_before_close_min,
            timezone_offset_hours: self.timezone_offset_hours,
        }
    }

    pub fn to_mock_live_probe_config(&self) -> MockLiveProbeConfig {
        MockLiveProbeConfig {
            symbol: self.symbol.clone(),
            qty: self.qty,
            side: self.side,
            tick_size: self.tick_size,
            offset_ticks: self.place_offset_ticks,
            trigger_after_live_bars: self.max_wait_bars_for_ack.max(1),
            mode: MockLiveProbeMode::parse(&self.strategy_id),
        }
    }

    pub fn to_session_gap_standalone_config(&self) -> SessionGapStandaloneConfig {
        SessionGapStandaloneConfig {
            symbol: self.symbol.clone(),
            timezone_offset_hours: self.timezone_offset_hours,
            place_offset_ticks: self.place_offset_ticks,
            tick_size: self.tick_size,
            close_hour: self.session_gap_close_hour,
            close_minute: self.session_gap_close_minute,
            entry_ack_timeout_ms: self.entry_ack_timeout_ms,
            entry_fill_timeout_ms: self.entry_fill_timeout_ms,
            exit_ack_timeout_ms: self.exit_ack_timeout_ms,
            exit_fill_timeout_ms: self.exit_fill_timeout_ms,
            k_long: self.session_gap_k_long,
            k_short: self.session_gap_k_short,
            wait_hours: self.session_gap_wait_hours,
            k_tp_long: self.session_gap_k_tp_long,
            k_sl_long: self.session_gap_k_sl_long,
            k_tp_short: self.session_gap_k_tp_short,
            k_sl_short: self.session_gap_k_sl_short,
            long_ex_pct: self.session_gap_long_ex_pct,
            short_ex_pct: self.session_gap_short_ex_pct,
            session_gap_min: self.session_gap_min,
            exit_offset_min: self.session_gap_exit_offset_min,
            work_weekends: self.session_gap_work_weekends,
            cash_factor: self.session_gap_cash_factor,
            start_cash: self.session_gap_start_cash,
            max_entry_hour: self.session_gap_max_entry_hour,
        }
    }

    pub fn to_hybrid_intraday_runtime_config(&self) -> HybridIntradayRuntimeConfig {
        let (session_close_hour, session_close_minute, weekends_off) = self
            .trading_periods
            .as_ref()
            .map(|p| (p.session_end.hour(), p.session_end.minute(), p.weekends_off))
            .unwrap_or((self.session_close_hour, self.session_close_minute, false));
        let mr_session_end_time =
            NaiveTime::parse_from_str(&self.hybrid_intraday.mr_session_end_time, "%H:%M:%S")
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(11, 59, 0).unwrap_or(NaiveTime::MIN));
        let bo_min_range_mode = match self
            .hybrid_intraday
            .bo_min_range_mode
            .to_ascii_lowercase()
            .as_str()
        {
            "disabled" => MinRangeMode::Disabled,
            "relative_prev_close" | "relativeprevclose" => MinRangeMode::RelativePrevClose,
            _ => MinRangeMode::Absolute,
        };
        let breakout_eod_mode = match self
            .hybrid_intraday
            .orchestrator_breakout_eod_mode
            .to_ascii_lowercase()
            .as_str()
        {
            "overnight" => BreakoutEodMode::Overnight,
            _ => BreakoutEodMode::SameDay,
        };
        let breakout_overnight_exit_time = NaiveTime::parse_from_str(
            &self
                .hybrid_intraday
                .orchestrator_breakout_overnight_exit_time,
            "%H:%M:%S",
        )
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(9, 30, 0).unwrap_or(NaiveTime::MIN));
        HybridIntradayRuntimeConfig {
            symbol: self.symbol.clone(),
            qty: self.qty.max(1.0),
            timezone_offset_hours: self.timezone_offset_hours,
            session_close_hour,
            session_close_minute,
            weekends_off,
            stop_end_buffer_sec: self.hybrid_intraday.stop_end_buffer_sec.max(1),
            repair_deadline_sec: self.hybrid_intraday.repair_deadline_sec.max(1),
            sl_escalate_timeout_sec: self.hybrid_intraday.sl_escalate_timeout_sec.max(1),
            max_repair_retries: self.hybrid_intraday.max_repair_retries.max(1),
            repair_backoff_base_sec: self.hybrid_intraday.repair_backoff_base_sec.max(1),
            repair_backoff_max_sec: self
                .hybrid_intraday
                .repair_backoff_max_sec
                .max(self.hybrid_intraday.repair_backoff_base_sec.max(1)),
            pending_timeout_sec: self.hybrid_intraday.pending_timeout_sec.max(1),
            mr_config: MeanReversionConfig {
                min_range_long: self.hybrid_intraday.mr_min_range_long,
                max_range_long: self.hybrid_intraday.mr_max_range_long,
                k_long: self.hybrid_intraday.mr_k_long,
                take_k_long: self.hybrid_intraday.mr_take_k_long,
                stop_k_long: self.hybrid_intraday.mr_stop_k_long,
                min_range_short: self.hybrid_intraday.mr_min_range_short,
                max_range_short: self.hybrid_intraday.mr_max_range_short,
                k_short: self.hybrid_intraday.mr_k_short,
                take_k_short: self.hybrid_intraday.mr_take_k_short,
                stop_k_short: self.hybrid_intraday.mr_stop_k_short,
                tick_size: self.tick_size,
                session_end_time: mr_session_end_time,
                exit_offset: Duration::minutes(self.hybrid_intraday.mr_exit_offset_min.max(0)),
            },
            breakout_config: IntradayBreakoutConfig {
                k: self.hybrid_intraday.bo_k,
                stop1_range: self.hybrid_intraday.bo_stop1_range,
                stop2_range: self.hybrid_intraday.bo_stop2_range,
                big_move_threshold: self.hybrid_intraday.bo_big_move_threshold,
                min_range: self.hybrid_intraday.bo_min_range,
                min_range_mode: bo_min_range_mode,
                exclude_weekends: self.hybrid_intraday.bo_exclude_weekends,
                wait_hours: self.hybrid_intraday.bo_wait_hours,
            },
            orchestrator_config: HybridOrchestratorConfig {
                breakout_eod_mode,
                breakout_overnight_exit_time,
            },
        }
    }
}

pub fn deterministic_request_id(
    strategy_id: &str,
    portfolio: &str,
    symbol: &str,
    action: &str,
    bar_ts: i64,
    seq: u8,
) -> Uuid {
    let name = format!("{strategy_id}|{portfolio}|{symbol}|{action}|{bar_ts}|{seq}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

pub fn market_request_seq(side: Side) -> u8 {
    if side == Side::Buy {
        3
    } else {
        4
    }
}

pub fn deterministic_market_request_id(
    strategy_id: &str,
    portfolio: &str,
    symbol: &str,
    created_ts_utc: i64,
    side: Side,
) -> Uuid {
    deterministic_request_id(
        strategy_id,
        portfolio,
        symbol,
        "market",
        created_ts_utc,
        market_request_seq(side),
    )
}

pub fn build_place_command(
    config: &LimitCancelConfig,
    strategy_id: &str,
    portfolio: &str,
    exchange: &str,
    bar: &BarEvent,
) -> OrderCommand {
    let price = match config.side {
        Side::Buy => bar.close - (config.offset_ticks as f64) * config.tick_size,
        Side::Sell => bar.close + (config.offset_ticks as f64) * config.tick_size,
    };
    let request_id = deterministic_request_id(
        strategy_id,
        portfolio,
        &bar.symbol,
        "place",
        bar.close_time_utc,
        0,
    );
    OrderCommand {
        request_id,
        created_ts_utc: bar.close_time_utc,
        strategy_id: strategy_id.to_string(),
        portfolio: portfolio.to_string(),
        exchange: exchange.to_string(),
        symbol: bar.symbol.clone(),
        action: CommandAction::Place(PlaceOrder {
            price,
            qty: config.qty,
            side: config.side,
            comment: None,
        }),
        intent_class: Some(IntentClass::Entry),
        ttl_ms: None,
    }
}

pub fn build_cancel_command(
    strategy_id: &str,
    portfolio: &str,
    exchange: &str,
    symbol: &str,
    order_id: i64,
    bar_ts: i64,
) -> OrderCommand {
    let request_id = deterministic_request_id(strategy_id, portfolio, symbol, "cancel", bar_ts, 1);
    OrderCommand {
        request_id,
        created_ts_utc: bar_ts,
        strategy_id: strategy_id.to_string(),
        portfolio: portfolio.to_string(),
        exchange: exchange.to_string(),
        symbol: symbol.to_string(),
        action: CommandAction::Cancel(alor_protocol::CancelOrder { order_id }),
        intent_class: Some(IntentClass::CancelCleanup),
        ttl_ms: None,
    }
}

#[derive(Debug, Default)]
pub struct RuntimeCaches {
    pub orders: HashMap<i64, OrderEvent>,
    pub positions: HashMap<String, PositionEvent>,
}
