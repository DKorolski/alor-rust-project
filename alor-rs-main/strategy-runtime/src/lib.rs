pub mod config;
pub mod health_server;
pub mod live_guard;
pub mod redis_transport;
pub mod runtime;
pub mod state;
pub mod strategies;
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
pub enum StrategySpecificConfig {
    LimitCancel(LimitCancelSettings),
    MarketBuyAndClose(MarketBuyAndCloseSettings),
    ToySessionTiming(ToySessionTimingSettings),
    SessionGapStandalone(SessionGapStandaloneSettings),
    MockLiveProbe(MockLiveProbeSettings),
    HybridIntraday(HybridIntradayStrategySettings),
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
