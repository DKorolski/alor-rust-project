pub mod config;
pub mod health_server;
pub mod live_guard;
pub mod redis_transport;
pub mod risk_gate_store;
pub mod runtime;
pub mod state;
pub mod strategies;
pub mod strategy_adapters;
pub mod strategy_host;
pub mod strategy_registry;
pub mod trade_ledger;

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::time::Instant;

use alor_protocol::{CommandAction, IntentClass, OrderCommand, PlaceOrder, Side};
use alor_types::TradingPeriods;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::strategies::limit_cancel::LimitCancelConfig;
use crate::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle;
pub use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
    RuntimeStateRestored, StopOrderEvent, Strategy, StrategyCtx, TradeEvent,
};

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
    pub common: StrategyCommonConfig,
    pub specific: StrategySpecificConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyCommonConfig {
    pub strategy_id: String,
    pub strategy_kind: StrategyKind,
    pub symbol: String,
    pub qty: f64,
    pub side: Side,
    pub tick_size: f64,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LimitCancelSettings {
    pub place_offset_ticks: i64,
    pub max_wait_bars_for_ack: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketBuyAndCloseSettings {
    pub live_order_style: MarketBuyAndCloseLiveOrderStyle,
    pub marketable_limit_offset_ticks: i64,
    pub close_trigger: CloseTrigger,
    pub entry_ack_timeout_ms: u64,
    pub entry_fill_timeout_ms: u64,
    pub exit_ack_timeout_ms: u64,
    pub exit_fill_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToySessionTimingSettings;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockLiveProbeSettings {
    pub place_offset_ticks: i64,
    pub max_wait_bars_for_ack: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionGapStandaloneSettings {
    pub place_offset_ticks: i64,
    pub entry_ack_timeout_ms: u64,
    pub entry_fill_timeout_ms: u64,
    pub exit_ack_timeout_ms: u64,
    pub exit_fill_timeout_ms: u64,
    pub signal_minute: u32,
    pub k_long: f64,
    pub k_short: f64,
    pub wait_hours: i64,
    pub k_tp_long: f64,
    pub k_sl_long: f64,
    pub k_tp_short: f64,
    pub k_sl_short: f64,
    pub long_ex_pct: f64,
    pub short_ex_pct: f64,
    pub start_cash: f64,
    pub cash_factor: f64,
    pub max_entry_hour: u32,
    pub close_hour: u32,
    pub close_minute: u32,
    pub session_gap_min: f64,
    pub exit_offset_min: i64,
    pub work_weekends: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HybridIntradayStrategySettings {
    pub live_order_style: MarketBuyAndCloseLiveOrderStyle,
    pub marketable_limit_offset_ticks: i64,
    #[serde(default)]
    pub strategy: HybridIntradaySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlorUsdrubfHybridSettings {
    pub mr_min_rel_range: f64,
    pub mr_max_rel_range: f64,
    pub mr_k_short: f64,
    pub mr_take_k_short: f64,
    pub mr_stop_k_short: f64,
    pub mr_last_entry_time: String,
    pub mr_force_exit_time: String,
    pub bo_k: f64,
    pub bo_stop1_range: f64,
    pub bo_stop2_range: f64,
    pub bo_big_move_threshold: f64,
    pub bo_wait_hours: f64,
    pub bo_eod_exit_time: String,
    pub commission_pct_per_side: f64,
    pub position_size_fraction: f64,
    pub initial_cash: f64,
    pub enable_live_execution: bool,
    pub use_fixed_live_size: bool,
    pub live_fixed_units: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiAuthor4142Settings {
    pub profile_id: String,
    pub timeframe: String,
    pub mode: String,
    pub allow_order_emission: bool,
    pub execution_path: String,
    pub order_symbol: Option<String>,
    pub excluded_model_dates: Vec<String>,
    pub min_anchor_bars: usize,
    pub anchor_first_bar_at_or_before: String,
    pub anchor_last_bar_at_or_after: String,
    pub actual_expiry_date: Option<String>,
    pub roll_target_sessions_before: u32,
    pub roll_fallback_sessions_before: u32,
    pub decision_journal_path: Option<String>,
    pub decision_journal_append: bool,
}

impl Default for AlorUsdrubfHybridSettings {
    fn default() -> Self {
        Self {
            mr_min_rel_range: 0.006,
            mr_max_rel_range: 0.050,
            mr_k_short: 0.045,
            mr_take_k_short: 0.16,
            mr_stop_k_short: 0.43,
            mr_last_entry_time: "11:40:00".to_string(),
            mr_force_exit_time: "11:50:00".to_string(),
            bo_k: 0.45,
            bo_stop1_range: 0.51,
            bo_stop2_range: 0.35,
            bo_big_move_threshold: 0.020,
            bo_wait_hours: 2.0,
            bo_eod_exit_time: "23:30:00".to_string(),
            commission_pct_per_side: 0.004,
            position_size_fraction: 0.9,
            initial_cash: 100_000.0,
            enable_live_execution: false,
            use_fixed_live_size: true,
            live_fixed_units: 1.0,
        }
    }
}

impl Default for RiAuthor4142Settings {
    fn default() -> Self {
        Self {
            profile_id: "ri_author41_42_primary_combo_cost2".to_string(),
            timeframe: "10m".to_string(),
            mode: "shadow".to_string(),
            allow_order_emission: false,
            execution_path: "action_scoped_only".to_string(),
            order_symbol: None,
            excluded_model_dates: Vec::new(),
            min_anchor_bars: 0,
            anchor_first_bar_at_or_before: "23:59:59".to_string(),
            anchor_last_bar_at_or_after: "00:00:00".to_string(),
            actual_expiry_date: None,
            roll_target_sessions_before: 1,
            roll_fallback_sessions_before: 2,
            decision_journal_path: None,
            decision_journal_append: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StrategySpecificConfig {
    LimitCancel(LimitCancelSettings),
    MarketBuyAndClose(MarketBuyAndCloseSettings),
    ToySessionTiming(ToySessionTimingSettings),
    SessionGapStandalone(SessionGapStandaloneSettings),
    MockLiveProbe(MockLiveProbeSettings),
    HybridIntraday(HybridIntradayStrategySettings),
    AlorUsdrubfHybrid(AlorUsdrubfHybridSettings),
    RiAuthor4142(RiAuthor4142Settings),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HybridIntradaySettings {
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub mr_variant: String,
    #[serde(default)]
    pub mr_gate_policy: String,
    #[serde(default)]
    pub risk_gate_mode: String,
    #[serde(default)]
    pub risk_gate_seed_file: Option<String>,
    #[serde(default)]
    pub risk_gate_ledger_key: Option<String>,
    #[serde(default)]
    pub model_session_start_time: String,
    #[serde(default)]
    pub model_session_end_time: String,
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
    pub partial_entry_fill_timeout_ms: u64,
    pub stop_end_buffer_sec: u64,
}

impl Default for HybridIntradaySettings {
    fn default() -> Self {
        Self {
            profile: "baseline_runtime_hybrid".to_string(),
            mr_variant: "classic_prev_day_range".to_string(),
            mr_gate_policy: "disabled".to_string(),
            risk_gate_mode: "disabled".to_string(),
            risk_gate_seed_file: None,
            risk_gate_ledger_key: None,
            model_session_start_time: String::new(),
            model_session_end_time: String::new(),
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
            partial_entry_fill_timeout_ms: 3_000,
            stop_end_buffer_sec: 60,
        }
    }
}

impl Default for StrategyCommonConfig {
    fn default() -> Self {
        Self {
            strategy_id: "limit_cancel".to_string(),
            strategy_kind: StrategyKind::LimitCancel,
            symbol: "SBER".to_string(),
            qty: 1.0,
            side: Side::Buy,
            tick_size: 0.01,
            session_open_hour: 10,
            session_open_minute: 0,
            session_close_hour: 23,
            session_close_minute: 50,
            entry_after_open_min: 59,
            exit_before_close_min: 20,
            timezone_offset_hours: 3,
            trading_periods: None,
            max_silence_bars_sec: 0,
        }
    }
}

impl Default for LimitCancelSettings {
    fn default() -> Self {
        Self {
            place_offset_ticks: 1,
            max_wait_bars_for_ack: 3,
        }
    }
}

impl Default for MarketBuyAndCloseSettings {
    fn default() -> Self {
        Self {
            live_order_style: MarketBuyAndCloseLiveOrderStyle::Market,
            marketable_limit_offset_ticks: 0,
            close_trigger: CloseTrigger::NextBar,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
        }
    }
}

impl Default for MockLiveProbeSettings {
    fn default() -> Self {
        Self {
            place_offset_ticks: 1,
            max_wait_bars_for_ack: 3,
        }
    }
}

impl Default for SessionGapStandaloneSettings {
    fn default() -> Self {
        Self {
            place_offset_ticks: 1,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
            signal_minute: 59,
            k_long: 0.5,
            k_short: 0.46,
            wait_hours: 2,
            k_tp_long: 0.28,
            k_sl_long: 0.68,
            k_tp_short: 0.28,
            k_sl_short: 0.65,
            long_ex_pct: 2.2,
            short_ex_pct: 2.2,
            start_cash: 30_000.0,
            cash_factor: 0.9,
            max_entry_hour: 19,
            close_hour: 23,
            close_minute: 49,
            session_gap_min: 60.0,
            exit_offset_min: 20,
            work_weekends: false,
        }
    }
}

impl Default for HybridIntradayStrategySettings {
    fn default() -> Self {
        Self {
            live_order_style: MarketBuyAndCloseLiveOrderStyle::Market,
            marketable_limit_offset_ticks: 0,
            strategy: HybridIntradaySettings::default(),
        }
    }
}

impl StrategySpecificConfig {
    pub fn default_for_kind(kind: StrategyKind) -> Self {
        match kind {
            StrategyKind::LimitCancel => Self::LimitCancel(LimitCancelSettings::default()),
            StrategyKind::MarketBuyAndClose => {
                Self::MarketBuyAndClose(MarketBuyAndCloseSettings::default())
            }
            StrategyKind::ToySessionTiming => Self::ToySessionTiming(ToySessionTimingSettings),
            StrategyKind::SessionGapStandalone => {
                Self::SessionGapStandalone(SessionGapStandaloneSettings::default())
            }
            StrategyKind::MockLiveProbe => Self::MockLiveProbe(MockLiveProbeSettings::default()),
            StrategyKind::HybridIntraday => {
                Self::HybridIntraday(HybridIntradayStrategySettings::default())
            }
            StrategyKind::AlorUsdrubfHybrid => {
                Self::AlorUsdrubfHybrid(AlorUsdrubfHybridSettings::default())
            }
            StrategyKind::RiAuthor4142 => Self::RiAuthor4142(RiAuthor4142Settings::default()),
        }
    }

    pub fn kind(&self) -> StrategyKind {
        match self {
            StrategySpecificConfig::LimitCancel(_) => StrategyKind::LimitCancel,
            StrategySpecificConfig::MarketBuyAndClose(_) => StrategyKind::MarketBuyAndClose,
            StrategySpecificConfig::ToySessionTiming(_) => StrategyKind::ToySessionTiming,
            StrategySpecificConfig::SessionGapStandalone(_) => StrategyKind::SessionGapStandalone,
            StrategySpecificConfig::MockLiveProbe(_) => StrategyKind::MockLiveProbe,
            StrategySpecificConfig::HybridIntraday(_) => StrategyKind::HybridIntraday,
            StrategySpecificConfig::AlorUsdrubfHybrid(_) => StrategyKind::AlorUsdrubfHybrid,
            StrategySpecificConfig::RiAuthor4142(_) => StrategyKind::RiAuthor4142,
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
    AlorUsdrubfHybrid,
    #[serde(rename = "ri_author41_42")]
    RiAuthor4142,
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
    pub fn defaults_for_kind(kind: StrategyKind) -> Self {
        Self {
            common: StrategyCommonConfig {
                strategy_kind: kind,
                strategy_id: kind.default_strategy_id().to_string(),
                ..StrategyCommonConfig::default()
            },
            specific: StrategySpecificConfig::default_for_kind(kind),
        }
    }

    pub fn set_kind(&mut self, kind: StrategyKind) {
        let previous_kind = self.common.strategy_kind;
        let previous_default_id = previous_kind.default_strategy_id();
        self.common.strategy_kind = kind;
        if self.common.strategy_id == previous_default_id {
            self.common.strategy_id = kind.default_strategy_id().to_string();
        }
        if self.specific.kind() != kind {
            self.specific = StrategySpecificConfig::default_for_kind(kind);
        }
    }

    pub fn specific(&self) -> &StrategySpecificConfig {
        &self.specific
    }

    pub fn specific_mut(&mut self) -> &mut StrategySpecificConfig {
        &mut self.specific
    }

    pub fn limit_cancel(&self) -> Option<&LimitCancelSettings> {
        match &self.specific {
            StrategySpecificConfig::LimitCancel(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn limit_cancel_mut(&mut self) -> Option<&mut LimitCancelSettings> {
        match &mut self.specific {
            StrategySpecificConfig::LimitCancel(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn market_buy_and_close(&self) -> Option<&MarketBuyAndCloseSettings> {
        match &self.specific {
            StrategySpecificConfig::MarketBuyAndClose(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn market_buy_and_close_mut(&mut self) -> Option<&mut MarketBuyAndCloseSettings> {
        match &mut self.specific {
            StrategySpecificConfig::MarketBuyAndClose(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn mock_live_probe(&self) -> Option<&MockLiveProbeSettings> {
        match &self.specific {
            StrategySpecificConfig::MockLiveProbe(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn mock_live_probe_mut(&mut self) -> Option<&mut MockLiveProbeSettings> {
        match &mut self.specific {
            StrategySpecificConfig::MockLiveProbe(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn session_gap_standalone(&self) -> Option<&SessionGapStandaloneSettings> {
        match &self.specific {
            StrategySpecificConfig::SessionGapStandalone(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn session_gap_standalone_mut(&mut self) -> Option<&mut SessionGapStandaloneSettings> {
        match &mut self.specific {
            StrategySpecificConfig::SessionGapStandalone(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn hybrid_intraday(&self) -> Option<&HybridIntradayStrategySettings> {
        match &self.specific {
            StrategySpecificConfig::HybridIntraday(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn hybrid_intraday_mut(&mut self) -> Option<&mut HybridIntradayStrategySettings> {
        match &mut self.specific {
            StrategySpecificConfig::HybridIntraday(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn alor_usdrubf_hybrid(&self) -> Option<&AlorUsdrubfHybridSettings> {
        match &self.specific {
            StrategySpecificConfig::AlorUsdrubfHybrid(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn alor_usdrubf_hybrid_mut(&mut self) -> Option<&mut AlorUsdrubfHybridSettings> {
        match &mut self.specific {
            StrategySpecificConfig::AlorUsdrubfHybrid(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn alor_skeleton(&self) -> Option<&AlorUsdrubfHybridSettings> {
        self.alor_usdrubf_hybrid()
    }

    pub fn alor_skeleton_mut(&mut self) -> Option<&mut AlorUsdrubfHybridSettings> {
        self.alor_usdrubf_hybrid_mut()
    }

    pub fn ri_author41_42(&self) -> Option<&RiAuthor4142Settings> {
        match &self.specific {
            StrategySpecificConfig::RiAuthor4142(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn ri_author41_42_mut(&mut self) -> Option<&mut RiAuthor4142Settings> {
        match &mut self.specific {
            StrategySpecificConfig::RiAuthor4142(settings) => Some(settings),
            _ => None,
        }
    }
}

impl Deref for StrategyConfig {
    type Target = StrategyCommonConfig;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl DerefMut for StrategyConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
    }
}

impl StrategyKind {
    pub fn default_strategy_id(self) -> &'static str {
        match self {
            StrategyKind::LimitCancel => "limit_cancel",
            StrategyKind::MarketBuyAndClose => "market_buy_and_close",
            StrategyKind::ToySessionTiming => "toy_session_timing",
            StrategyKind::SessionGapStandalone => "session_gap_standalone",
            StrategyKind::MockLiveProbe => "mock_live_probe",
            StrategyKind::HybridIntraday => "hybrid_intraday",
            StrategyKind::AlorUsdrubfHybrid => "alor_usdrubf_hybrid_v1",
            StrategyKind::RiAuthor4142 => "ri_author41_42",
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
