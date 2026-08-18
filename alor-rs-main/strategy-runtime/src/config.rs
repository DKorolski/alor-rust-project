use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use alor_types::TradingPeriods;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle;
use crate::{
    BacktestConfig, CloseTrigger, HealthServerConfig, PaperConfig, PaperExecutionMode, PaperOutput,
    ReadConfig, ReplayConfig, RuntimeConfig, StrategyConfig, StrategyKind, StreamNames, TradeMode,
    TrimConfig,
};

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1/";
const DEFAULT_STRATEGY_KIND: StrategyKind = StrategyKind::LimitCancel;
const DEFAULT_PORTFOLIO: &str = "demo";
const DEFAULT_EXCHANGE: &str = "alor";
const DEFAULT_SOURCE: &str = "strategy-runtime";
const DEFAULT_CONSUMER_GROUP: &str = "strategy-runtime";
const DEFAULT_CONSUMER_NAME: &str = "auto";
const DEFAULT_HEALTH_STREAM: &str = "events.health";

const DEFAULT_TRADE_MODE: TradeMode = TradeMode::Paper;
const DEFAULT_ALLOW_LIVE_ORDERS: bool = false;
const DEFAULT_ALLOW_PAPER_ORDERS: bool = true;
const DEFAULT_GUARD_LOG_INTERVAL_MS: u64 = 5_000;
const DEFAULT_STILL_BLOCKED_LOG_PERIOD_SEC: u64 = 60;
const DEFAULT_GATEWAY_HEALTH_STALE_SEC: u64 = 20;
const DEFAULT_REQUIRE_GATEWAY_READY: bool = true;
const DEFAULT_BOOTSTRAP_DUMP: bool = false;
const DEFAULT_RUNTIME_HEALTH_ENABLED: bool = true;
const DEFAULT_RUNTIME_HEALTH_LISTEN_ADDR: &str = "127.0.0.1:8091";
const DEFAULT_RUNTIME_HEALTH_EXPOSE_METRICS: bool = false;
const DEFAULT_PAPER_ENABLED: bool = true;
const DEFAULT_PAPER_OUTPUT: PaperOutput = PaperOutput::Stdout;
const DEFAULT_PAPER_EXECUTION_MODE: PaperExecutionMode = PaperExecutionMode::LiveOnly;
const DEFAULT_PAPER_FILE_PATH: &str = "./paper_trades.jsonl";
const DEFAULT_PAPER_TRADES_CSV: &str = "./trades.csv";
const DEFAULT_PAPER_SUMMARY_JSON: &str = "./summary.json";
const DEFAULT_PAPER_APPEND: bool = false;
const DEFAULT_BACKTEST_ENABLED: bool = true;
const DEFAULT_BACKTEST_TRADE_LOG: &str = "./backtest_trades.log";
const DEFAULT_BACKTEST_TRADES_CSV: &str = "./trades.csv";
const DEFAULT_BACKTEST_SUMMARY_JSON: &str = "./summary.json";
const DEFAULT_BACKTEST_APPEND: bool = false;
const DEFAULT_REPLAY_ENABLED: bool = false;
const DEFAULT_REPLAY_BARS_CSV_PATH: Option<&str> = None;
const DEFAULT_REPLAY_REFERENCE_TRADES_CSV_PATH: Option<&str> = None;
const DEFAULT_REPLAY_OUTPUT_DIR: &str = "replay_out";
const DEFAULT_REPLAY_PRICE_TOLERANCE: f64 = 1e-8;
const DEFAULT_REPLAY_STRICT_DEDUP: bool = true;

const DEFAULT_BLOCK_MS: usize = 500;
const DEFAULT_CLAIM_IDLE_MS: usize = 5_000;
const DEFAULT_CLAIM_BATCH: usize = 50;
const DEFAULT_POLL_INTERVAL_MS: u64 = 100;

const DEFAULT_TRIM_BARS: usize = 200_000;
const DEFAULT_TRIM_ORDERS: usize = 100_000;
const DEFAULT_TRIM_TRADES: usize = 100_000;
const DEFAULT_TRIM_POSITIONS: usize = 50_000;
const DEFAULT_TRIM_COMMANDS: usize = 50_000;
const DEFAULT_TRIM_ACKS: usize = 100_000;
const DEFAULT_TRIM_HEALTH: usize = 10_000;
const DEFAULT_TRIM_RUNTIME_STATE: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    File,
    Env,
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            ConfigSource::Default => "default",
            ConfigSource::File => "file",
            ConfigSource::Env => "env",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSources {
    pub redis_url: ConfigSource,
    pub portfolio: ConfigSource,
    pub exchange: ConfigSource,
    pub source: ConfigSource,
    pub streams: StreamSources,
    pub consumer_group: ConfigSource,
    pub consumer_name: ConfigSource,
    pub read: ReadSources,
    pub trim: TrimSources,
    pub strategy: StrategySources,
    pub runtime: RuntimeSources,
    pub paper: PaperSources,
    pub backtest: BacktestSources,
    pub replay: ReplaySources,
    pub reset_state_on_start: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSources {
    pub bars: ConfigSource,
    pub orders: ConfigSource,
    pub trades: ConfigSource,
    pub positions: ConfigSource,
    pub commands: ConfigSource,
    pub acks: ConfigSource,
    pub snapshots: ConfigSource,
    pub health: ConfigSource,
    pub dlq_prefix: ConfigSource,
    pub runtime_state: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadSources {
    pub block_ms: ConfigSource,
    pub claim_idle_ms: ConfigSource,
    pub claim_batch: ConfigSource,
    pub poll_interval_ms: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimSources {
    pub bars: ConfigSource,
    pub orders: ConfigSource,
    pub trades: ConfigSource,
    pub positions: ConfigSource,
    pub commands: ConfigSource,
    pub acks: ConfigSource,
    pub health: ConfigSource,
    pub runtime_state: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategySources {
    pub strategy_id: ConfigSource,
    pub strategy_kind: ConfigSource,
    pub symbol: ConfigSource,
    pub qty: ConfigSource,
    pub side: ConfigSource,
    pub live_order_style: ConfigSource,
    pub marketable_limit_offset_ticks: ConfigSource,
    pub place_offset_ticks: ConfigSource,
    pub tick_size: ConfigSource,
    pub max_wait_bars_for_ack: ConfigSource,
    pub close_trigger: ConfigSource,
    pub entry_ack_timeout_ms: ConfigSource,
    pub entry_fill_timeout_ms: ConfigSource,
    pub exit_ack_timeout_ms: ConfigSource,
    pub exit_fill_timeout_ms: ConfigSource,
    pub session_open_hour: ConfigSource,
    pub session_open_minute: ConfigSource,
    pub session_close_hour: ConfigSource,
    pub session_close_minute: ConfigSource,
    pub entry_after_open_min: ConfigSource,
    pub exit_before_close_min: ConfigSource,
    pub timezone_offset_hours: ConfigSource,
    pub trading_periods: ConfigSource,
    pub max_silence_bars_sec: ConfigSource,
    pub session_gap_k_long: ConfigSource,
    pub session_gap_k_short: ConfigSource,
    pub session_gap_signal_minute: ConfigSource,
    pub session_gap_wait_hours: ConfigSource,
    pub session_gap_k_tp_long: ConfigSource,
    pub session_gap_k_sl_long: ConfigSource,
    pub session_gap_k_tp_short: ConfigSource,
    pub session_gap_k_sl_short: ConfigSource,
    pub session_gap_long_ex_pct: ConfigSource,
    pub session_gap_short_ex_pct: ConfigSource,
    pub session_gap_start_cash: ConfigSource,
    pub session_gap_cash_factor: ConfigSource,
    pub session_gap_max_entry_hour: ConfigSource,
    pub session_gap_close_hour: ConfigSource,
    pub session_gap_close_minute: ConfigSource,
    pub session_gap_min: ConfigSource,
    pub session_gap_exit_offset_min: ConfigSource,
    pub session_gap_work_weekends: ConfigSource,
    pub hybrid_intraday: ConfigSource,
    pub alor_usdrubf_hybrid: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSources {
    pub trade_mode: ConfigSource,
    pub allow_live_orders: ConfigSource,
    pub allow_paper_orders: ConfigSource,
    pub guard_log_interval_ms: ConfigSource,
    pub still_blocked_log_period_sec: ConfigSource,
    pub gateway_health_stale_sec: ConfigSource,
    pub require_gateway_ready: ConfigSource,
    pub bootstrap_dump: ConfigSource,
    pub health_enabled: ConfigSource,
    pub health_listen_addr: ConfigSource,
    pub health_expose_metrics: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperSources {
    pub enabled: ConfigSource,
    pub output: ConfigSource,
    pub execution_mode: ConfigSource,
    pub file_path: ConfigSource,
    pub trades_csv: ConfigSource,
    pub summary_json: ConfigSource,
    pub append: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestSources {
    pub enabled: ConfigSource,
    pub trade_log: ConfigSource,
    pub trades_csv: ConfigSource,
    pub summary_json: ConfigSource,
    pub append: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySources {
    pub enabled: ConfigSource,
    pub bars_csv_path: ConfigSource,
    pub reference_trades_csv_path: ConfigSource,
    pub output_dir: ConfigSource,
    pub price_tolerance: ConfigSource,
    pub strict_dedup: ConfigSource,
}

impl Default for ConfigSources {
    fn default() -> Self {
        Self {
            redis_url: ConfigSource::Default,
            portfolio: ConfigSource::Default,
            exchange: ConfigSource::Default,
            source: ConfigSource::Default,
            streams: StreamSources::default(),
            consumer_group: ConfigSource::Default,
            consumer_name: ConfigSource::Default,
            read: ReadSources::default(),
            trim: TrimSources::default(),
            strategy: StrategySources::default(),
            runtime: RuntimeSources::default(),
            paper: PaperSources::default(),
            backtest: BacktestSources::default(),
            replay: ReplaySources::default(),
            reset_state_on_start: ConfigSource::Default,
        }
    }
}

impl Default for StreamSources {
    fn default() -> Self {
        Self {
            bars: ConfigSource::Default,
            orders: ConfigSource::Default,
            trades: ConfigSource::Default,
            positions: ConfigSource::Default,
            commands: ConfigSource::Default,
            acks: ConfigSource::Default,
            snapshots: ConfigSource::Default,
            health: ConfigSource::Default,
            dlq_prefix: ConfigSource::Default,
            runtime_state: ConfigSource::Default,
        }
    }
}

impl Default for ReadSources {
    fn default() -> Self {
        Self {
            block_ms: ConfigSource::Default,
            claim_idle_ms: ConfigSource::Default,
            claim_batch: ConfigSource::Default,
            poll_interval_ms: ConfigSource::Default,
        }
    }
}

impl Default for TrimSources {
    fn default() -> Self {
        Self {
            bars: ConfigSource::Default,
            orders: ConfigSource::Default,
            trades: ConfigSource::Default,
            positions: ConfigSource::Default,
            commands: ConfigSource::Default,
            acks: ConfigSource::Default,
            health: ConfigSource::Default,
            runtime_state: ConfigSource::Default,
        }
    }
}

impl Default for StrategySources {
    fn default() -> Self {
        Self {
            strategy_id: ConfigSource::Default,
            strategy_kind: ConfigSource::Default,
            symbol: ConfigSource::Default,
            qty: ConfigSource::Default,
            side: ConfigSource::Default,
            live_order_style: ConfigSource::Default,
            marketable_limit_offset_ticks: ConfigSource::Default,
            place_offset_ticks: ConfigSource::Default,
            tick_size: ConfigSource::Default,
            max_wait_bars_for_ack: ConfigSource::Default,
            close_trigger: ConfigSource::Default,
            entry_ack_timeout_ms: ConfigSource::Default,
            entry_fill_timeout_ms: ConfigSource::Default,
            exit_ack_timeout_ms: ConfigSource::Default,
            exit_fill_timeout_ms: ConfigSource::Default,
            session_open_hour: ConfigSource::Default,
            session_open_minute: ConfigSource::Default,
            session_close_hour: ConfigSource::Default,
            session_close_minute: ConfigSource::Default,
            entry_after_open_min: ConfigSource::Default,
            exit_before_close_min: ConfigSource::Default,
            timezone_offset_hours: ConfigSource::Default,
            trading_periods: ConfigSource::Default,
            max_silence_bars_sec: ConfigSource::Default,
            session_gap_k_long: ConfigSource::Default,
            session_gap_k_short: ConfigSource::Default,
            session_gap_signal_minute: ConfigSource::Default,
            session_gap_wait_hours: ConfigSource::Default,
            session_gap_k_tp_long: ConfigSource::Default,
            session_gap_k_sl_long: ConfigSource::Default,
            session_gap_k_tp_short: ConfigSource::Default,
            session_gap_k_sl_short: ConfigSource::Default,
            session_gap_long_ex_pct: ConfigSource::Default,
            session_gap_short_ex_pct: ConfigSource::Default,
            session_gap_start_cash: ConfigSource::Default,
            session_gap_cash_factor: ConfigSource::Default,
            session_gap_max_entry_hour: ConfigSource::Default,
            session_gap_close_hour: ConfigSource::Default,
            session_gap_close_minute: ConfigSource::Default,
            session_gap_min: ConfigSource::Default,
            session_gap_exit_offset_min: ConfigSource::Default,
            session_gap_work_weekends: ConfigSource::Default,
            hybrid_intraday: ConfigSource::Default,
            alor_usdrubf_hybrid: ConfigSource::Default,
        }
    }
}

impl Default for RuntimeSources {
    fn default() -> Self {
        Self {
            trade_mode: ConfigSource::Default,
            allow_live_orders: ConfigSource::Default,
            allow_paper_orders: ConfigSource::Default,
            guard_log_interval_ms: ConfigSource::Default,
            still_blocked_log_period_sec: ConfigSource::Default,
            gateway_health_stale_sec: ConfigSource::Default,
            require_gateway_ready: ConfigSource::Default,
            bootstrap_dump: ConfigSource::Default,
            health_enabled: ConfigSource::Default,
            health_listen_addr: ConfigSource::Default,
            health_expose_metrics: ConfigSource::Default,
        }
    }
}

impl Default for PaperSources {
    fn default() -> Self {
        Self {
            enabled: ConfigSource::Default,
            output: ConfigSource::Default,
            execution_mode: ConfigSource::Default,
            file_path: ConfigSource::Default,
            trades_csv: ConfigSource::Default,
            summary_json: ConfigSource::Default,
            append: ConfigSource::Default,
        }
    }
}

impl Default for BacktestSources {
    fn default() -> Self {
        Self {
            enabled: ConfigSource::Default,
            trade_log: ConfigSource::Default,
            trades_csv: ConfigSource::Default,
            summary_json: ConfigSource::Default,
            append: ConfigSource::Default,
        }
    }
}

impl Default for ReplaySources {
    fn default() -> Self {
        Self {
            enabled: ConfigSource::Default,
            bars_csv_path: ConfigSource::Default,
            reference_trades_csv_path: ConfigSource::Default,
            output_dir: ConfigSource::Default,
            price_tolerance: ConfigSource::Default,
            strict_dedup: ConfigSource::Default,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRuntimeConfig {
    pub config: RuntimeConfig,
    pub sources: ConfigSources,
    pub path: PathBuf,
    pub file_loaded: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeConfigFile {
    redis_url: Option<String>,
    portfolio: Option<String>,
    exchange: Option<String>,
    source: Option<String>,
    streams: Option<StreamNamesFile>,
    runtime: Option<RuntimeSettingsFile>,
    consumer_group: Option<String>,
    consumer_name: Option<String>,
    read: Option<ReadConfigFile>,
    trim: Option<TrimConfigFile>,
    strategy: Option<StrategyConfigFile>,
    trading_periods: Option<TradingPeriods>,
    paper: Option<PaperConfigFile>,
    backtest: Option<BacktestConfigFile>,
    replay: Option<ReplayConfigFile>,
    reset_state_on_start: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeSettingsFile {
    trade_mode: Option<String>,
    allow_live_orders: Option<bool>,
    allow_paper_orders: Option<bool>,
    guard_log_interval_ms: Option<u64>,
    still_blocked_log_period_sec: Option<u64>,
    gateway_health_stale_sec: Option<u64>,
    require_gateway_ready: Option<bool>,
    bootstrap_dump: Option<bool>,
    health: Option<HealthServerSettingsFile>,
}

#[derive(Debug, Default, Deserialize)]
struct HealthServerSettingsFile {
    enabled: Option<bool>,
    listen_addr: Option<String>,
    expose_metrics: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct StreamNamesFile {
    bars: Option<String>,
    orders: Option<String>,
    trades: Option<String>,
    positions: Option<String>,
    commands: Option<String>,
    acks: Option<String>,
    snapshots: Option<String>,
    health: Option<String>,
    dlq_prefix: Option<String>,
    runtime_state: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ReadConfigFile {
    block_ms: Option<usize>,
    claim_idle_ms: Option<usize>,
    claim_batch: Option<usize>,
    poll_interval_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct TrimConfigFile {
    bars: Option<usize>,
    orders: Option<usize>,
    trades: Option<usize>,
    positions: Option<usize>,
    commands: Option<usize>,
    acks: Option<usize>,
    health: Option<usize>,
    runtime_state: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct StrategyConfigFile {
    #[serde(flatten)]
    common_legacy: StrategyCommonConfigFile,
    #[serde(flatten)]
    legacy_specific: StrategyLegacySpecificConfigFile,
    common: Option<StrategyCommonConfigFile>,
    limit_cancel: Option<LimitCancelConfigFile>,
    market_buy_and_close: Option<MarketBuyAndCloseConfigFile>,
    mock_live_probe: Option<MockLiveProbeConfigFile>,
    session_gap: Option<SessionGapConfigFile>,
    hybrid_intraday: Option<HybridIntradayConfigFile>,
    alor_usdrubf_hybrid: Option<AlorUsdrubfHybridConfigFile>,
    alor_skeleton: Option<AlorUsdrubfHybridConfigFile>,
    ri_author41_42: Option<RiAuthor4142ConfigFile>,
}

#[derive(Debug, Default, Deserialize)]
struct StrategyCommonConfigFile {
    strategy_id: Option<String>,
    strategy_kind: Option<String>,
    symbol: Option<String>,
    qty: Option<f64>,
    side: Option<String>,
    tick_size: Option<f64>,
    session_open_hour: Option<u32>,
    session_open_minute: Option<u32>,
    session_close_hour: Option<u32>,
    session_close_minute: Option<u32>,
    entry_after_open_min: Option<u32>,
    exit_before_close_min: Option<u32>,
    timezone_offset_hours: Option<i32>,
    trading_periods: Option<TradingPeriods>,
    max_silence_bars_sec: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct StrategyLegacySpecificConfigFile {
    live_order_style: Option<String>,
    marketable_limit_offset_ticks: Option<i64>,
    place_offset_ticks: Option<i64>,
    max_wait_bars_for_ack: Option<u32>,
    close_trigger: Option<String>,
    entry_ack_timeout_ms: Option<u64>,
    entry_fill_timeout_ms: Option<u64>,
    exit_ack_timeout_ms: Option<u64>,
    exit_fill_timeout_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct LimitCancelConfigFile {
    place_offset_ticks: Option<i64>,
    max_wait_bars_for_ack: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct MarketBuyAndCloseConfigFile {
    live_order_style: Option<String>,
    marketable_limit_offset_ticks: Option<i64>,
    close_trigger: Option<String>,
    entry_ack_timeout_ms: Option<u64>,
    entry_fill_timeout_ms: Option<u64>,
    exit_ack_timeout_ms: Option<u64>,
    exit_fill_timeout_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct MockLiveProbeConfigFile {
    place_offset_ticks: Option<i64>,
    max_wait_bars_for_ack: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct HybridIntradayConfigFile {
    live_order_style: Option<String>,
    marketable_limit_offset_ticks: Option<i64>,
    profile: Option<String>,
    mr_variant: Option<String>,
    live_mr_entries_enabled: Option<bool>,
    mr_gate_policy: Option<String>,
    risk_gate_mode: Option<String>,
    risk_gate_seed_file: Option<String>,
    risk_gate_ledger_key: Option<String>,
    risk_gate_persist_in_shadow: Option<bool>,
    risk_gate_legacy_session_start_time: Option<String>,
    risk_gate_legacy_session_end_time: Option<String>,
    risk_gate_session_policy_transition_date: Option<String>,
    model_session_start_time: Option<String>,
    model_session_end_time: Option<String>,
    mr_min_range_long: Option<f64>,
    mr_max_range_long: Option<f64>,
    mr_k_long: Option<f64>,
    mr_take_k_long: Option<f64>,
    mr_stop_k_long: Option<f64>,
    mr_min_range_short: Option<f64>,
    mr_max_range_short: Option<f64>,
    mr_k_short: Option<f64>,
    mr_take_k_short: Option<f64>,
    mr_stop_k_short: Option<f64>,
    mr_session_end_time: Option<String>,
    mr_exit_offset_min: Option<i64>,
    bo_k: Option<f64>,
    bo_stop1_range: Option<f64>,
    bo_stop2_range: Option<f64>,
    bo_big_move_threshold: Option<f64>,
    bo_min_range: Option<f64>,
    bo_min_range_mode: Option<String>,
    bo_exclude_weekends: Option<bool>,
    bo_wait_hours: Option<f64>,
    orchestrator_breakout_eod_mode: Option<String>,
    orchestrator_breakout_overnight_exit_time: Option<String>,
    repair_deadline_sec: Option<u64>,
    sl_escalate_timeout_sec: Option<u64>,
    max_repair_retries: Option<u32>,
    repair_backoff_base_sec: Option<u64>,
    repair_backoff_max_sec: Option<u64>,
    pending_timeout_sec: Option<u64>,
    partial_entry_fill_timeout_ms: Option<u64>,
    stop_end_buffer_sec: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct SessionGapConfigFile {
    place_offset_ticks: Option<i64>,
    entry_ack_timeout_ms: Option<u64>,
    entry_fill_timeout_ms: Option<u64>,
    exit_ack_timeout_ms: Option<u64>,
    exit_fill_timeout_ms: Option<u64>,
    signal_minute: Option<u32>,
    k_long: Option<f64>,
    k_short: Option<f64>,
    wait_hours: Option<i64>,
    k_tp_long: Option<f64>,
    k_sl_long: Option<f64>,
    k_tp_short: Option<f64>,
    k_sl_short: Option<f64>,
    long_ex_pct: Option<f64>,
    short_ex_pct: Option<f64>,
    start_cash: Option<f64>,
    cash_factor: Option<f64>,
    max_entry_hour: Option<u32>,
    close_hour: Option<u32>,
    close_minute: Option<u32>,
    session_gap_min: Option<f64>,
    exit_offset_min: Option<i64>,
    work_weekends: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct AlorUsdrubfHybridConfigFile {
    model_session_start_time: Option<String>,
    model_session_end_time: Option<String>,
    mr_min_rel_range: Option<f64>,
    mr_max_rel_range: Option<f64>,
    mr_k_short: Option<f64>,
    mr_take_k_short: Option<f64>,
    mr_stop_k_short: Option<f64>,
    mr_last_entry_time: Option<String>,
    mr_force_exit_time: Option<String>,
    bo_k: Option<f64>,
    bo_stop1_range: Option<f64>,
    bo_stop2_range: Option<f64>,
    bo_big_move_threshold: Option<f64>,
    bo_wait_hours: Option<f64>,
    bo_eod_exit_time: Option<String>,
    commission_pct_per_side: Option<f64>,
    position_size_fraction: Option<f64>,
    initial_cash: Option<f64>,
    enable_live_execution: Option<bool>,
    use_fixed_live_size: Option<bool>,
    live_fixed_units: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RiAuthor4142ConfigFile {
    profile_id: Option<String>,
    timeframe: Option<String>,
    mode: Option<String>,
    allow_order_emission: Option<bool>,
    execution_path: Option<String>,
    order_symbol: Option<String>,
    session_start_time: Option<String>,
    session_end_time: Option<String>,
    author41_entry_end_time: Option<String>,
    author41_time_exit: Option<String>,
    author42_exit_time: Option<String>,
    author42_last_entry_time: Option<String>,
    author42_max_entries_per_day: Option<u32>,
    excluded_model_dates: Option<Vec<String>>,
    min_anchor_bars: Option<usize>,
    anchor_first_bar_at_or_before: Option<String>,
    anchor_last_bar_at_or_after: Option<String>,
    anchor_transition_date: Option<String>,
    pre_transition_min_anchor_bars: Option<usize>,
    pre_transition_anchor_first_bar_at_or_before: Option<String>,
    pre_transition_anchor_last_bar_at_or_after: Option<String>,
    actual_expiry_date: Option<String>,
    roll_target_sessions_before: Option<u32>,
    roll_fallback_sessions_before: Option<u32>,
    decision_journal_path: Option<String>,
    decision_journal_append: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct PaperConfigFile {
    enabled: Option<bool>,
    output: Option<String>,
    execution_mode: Option<String>,
    file_path: Option<String>,
    trades_csv: Option<String>,
    summary_json: Option<String>,
    append: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct BacktestConfigFile {
    enabled: Option<bool>,
    trade_log: Option<String>,
    trades_csv: Option<String>,
    summary_json: Option<String>,
    append: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct ReplayConfigFile {
    enabled: Option<bool>,
    bars_csv_path: Option<String>,
    reference_trades_csv_path: Option<String>,
    output_dir: Option<String>,
    price_tolerance: Option<f64>,
    strict_dedup: Option<bool>,
}

fn apply_strategy_common_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    common_file: &StrategyCommonConfigFile,
    source: ConfigSource,
    kind_context: &str,
) -> Result<()> {
    if let Some(value) = &common_file.strategy_id {
        strategy.strategy_id = value.clone();
        sources.strategy_id = source;
    }
    if let Some(value) = &common_file.strategy_kind {
        strategy.set_kind(
            parse_strategy_kind(value)
                .with_context(|| format!("invalid {kind_context}: {value}"))?,
        );
        sources.strategy_kind = source;
    }
    if let Some(value) = &common_file.symbol {
        strategy.symbol = value.clone();
        sources.symbol = source;
    }
    if let Some(value) = common_file.qty {
        strategy.qty = value;
        sources.qty = source;
    }
    if let Some(value) = &common_file.side {
        strategy.side = parse_side(value);
        sources.side = source;
    }
    if let Some(value) = common_file.tick_size {
        strategy.tick_size = value;
        sources.tick_size = source;
    }
    if let Some(value) = common_file.session_open_hour {
        strategy.session_open_hour = value;
        sources.session_open_hour = source;
    }
    if let Some(value) = common_file.session_open_minute {
        strategy.session_open_minute = value;
        sources.session_open_minute = source;
    }
    if let Some(value) = common_file.session_close_hour {
        strategy.session_close_hour = value;
        sources.session_close_hour = source;
    }
    if let Some(value) = common_file.session_close_minute {
        strategy.session_close_minute = value;
        sources.session_close_minute = source;
    }
    if let Some(value) = common_file.entry_after_open_min {
        strategy.entry_after_open_min = value;
        sources.entry_after_open_min = source;
    }
    if let Some(value) = common_file.exit_before_close_min {
        strategy.exit_before_close_min = value;
        sources.exit_before_close_min = source;
    }
    if let Some(value) = common_file.timezone_offset_hours {
        strategy.timezone_offset_hours = value;
        sources.timezone_offset_hours = source;
    }
    if let Some(value) = &common_file.trading_periods {
        strategy.trading_periods = Some(value.clone());
        sources.trading_periods = source;
    }
    if let Some(value) = common_file.max_silence_bars_sec {
        strategy.max_silence_bars_sec = value;
        sources.max_silence_bars_sec = source;
    }
    Ok(())
}

fn apply_live_order_style(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: MarketBuyAndCloseLiveOrderStyle,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::MarketBuyAndClose => {
            if let Some(settings) = strategy.market_buy_and_close_mut() {
                settings.live_order_style = value;
            }
        }
        StrategyKind::HybridIntraday => {
            if let Some(settings) = strategy.hybrid_intraday_mut() {
                settings.live_order_style = value;
            }
        }
        _ => {}
    }
    sources.live_order_style = source;
}

fn apply_marketable_limit_offset_ticks(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: i64,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::MarketBuyAndClose => {
            if let Some(settings) = strategy.market_buy_and_close_mut() {
                settings.marketable_limit_offset_ticks = value;
            }
        }
        StrategyKind::HybridIntraday => {
            if let Some(settings) = strategy.hybrid_intraday_mut() {
                settings.marketable_limit_offset_ticks = value;
            }
        }
        _ => {}
    }
    sources.marketable_limit_offset_ticks = source;
}

fn apply_place_offset_ticks(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: i64,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::LimitCancel => {
            if let Some(settings) = strategy.limit_cancel_mut() {
                settings.place_offset_ticks = value;
            }
        }
        StrategyKind::MockLiveProbe => {
            if let Some(settings) = strategy.mock_live_probe_mut() {
                settings.place_offset_ticks = value;
            }
        }
        StrategyKind::SessionGapStandalone => {
            if let Some(settings) = strategy.session_gap_standalone_mut() {
                settings.place_offset_ticks = value;
            }
        }
        _ => {}
    }
    sources.place_offset_ticks = source;
}

fn apply_max_wait_bars_for_ack(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: u32,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::LimitCancel => {
            if let Some(settings) = strategy.limit_cancel_mut() {
                settings.max_wait_bars_for_ack = value;
            }
        }
        StrategyKind::MockLiveProbe => {
            if let Some(settings) = strategy.mock_live_probe_mut() {
                settings.max_wait_bars_for_ack = value;
            }
        }
        _ => {}
    }
    sources.max_wait_bars_for_ack = source;
}

fn apply_close_trigger(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: CloseTrigger,
    source: ConfigSource,
) {
    if let Some(settings) = strategy.market_buy_and_close_mut() {
        settings.close_trigger = value;
    }
    sources.close_trigger = source;
}

fn apply_entry_ack_timeout_ms(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: u64,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::MarketBuyAndClose => {
            if let Some(settings) = strategy.market_buy_and_close_mut() {
                settings.entry_ack_timeout_ms = value;
            }
        }
        StrategyKind::SessionGapStandalone => {
            if let Some(settings) = strategy.session_gap_standalone_mut() {
                settings.entry_ack_timeout_ms = value;
            }
        }
        _ => {}
    }
    sources.entry_ack_timeout_ms = source;
}

fn apply_entry_fill_timeout_ms(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: u64,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::MarketBuyAndClose => {
            if let Some(settings) = strategy.market_buy_and_close_mut() {
                settings.entry_fill_timeout_ms = value;
            }
        }
        StrategyKind::SessionGapStandalone => {
            if let Some(settings) = strategy.session_gap_standalone_mut() {
                settings.entry_fill_timeout_ms = value;
            }
        }
        _ => {}
    }
    sources.entry_fill_timeout_ms = source;
}

fn apply_exit_ack_timeout_ms(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: u64,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::MarketBuyAndClose => {
            if let Some(settings) = strategy.market_buy_and_close_mut() {
                settings.exit_ack_timeout_ms = value;
            }
        }
        StrategyKind::SessionGapStandalone => {
            if let Some(settings) = strategy.session_gap_standalone_mut() {
                settings.exit_ack_timeout_ms = value;
            }
        }
        _ => {}
    }
    sources.exit_ack_timeout_ms = source;
}

fn apply_exit_fill_timeout_ms(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    value: u64,
    source: ConfigSource,
) {
    match strategy.strategy_kind {
        StrategyKind::MarketBuyAndClose => {
            if let Some(settings) = strategy.market_buy_and_close_mut() {
                settings.exit_fill_timeout_ms = value;
            }
        }
        StrategyKind::SessionGapStandalone => {
            if let Some(settings) = strategy.session_gap_standalone_mut() {
                settings.exit_fill_timeout_ms = value;
            }
        }
        _ => {}
    }
    sources.exit_fill_timeout_ms = source;
}

fn apply_legacy_specific_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    specific_file: &StrategyLegacySpecificConfigFile,
    source: ConfigSource,
) {
    if let Some(value) = &specific_file.live_order_style {
        apply_live_order_style(strategy, sources, parse_live_order_style(value), source);
    }
    if let Some(value) = specific_file.marketable_limit_offset_ticks {
        apply_marketable_limit_offset_ticks(strategy, sources, value, source);
    }
    if let Some(value) = specific_file.place_offset_ticks {
        apply_place_offset_ticks(strategy, sources, value, source);
    }
    if let Some(value) = specific_file.max_wait_bars_for_ack {
        apply_max_wait_bars_for_ack(strategy, sources, value, source);
    }
    if let Some(value) = &specific_file.close_trigger {
        apply_close_trigger(strategy, sources, parse_close_trigger(value), source);
    }
    if let Some(value) = specific_file.entry_ack_timeout_ms {
        apply_entry_ack_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = specific_file.entry_fill_timeout_ms {
        apply_entry_fill_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = specific_file.exit_ack_timeout_ms {
        apply_exit_ack_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = specific_file.exit_fill_timeout_ms {
        apply_exit_fill_timeout_ms(strategy, sources, value, source);
    }
}

fn apply_limit_cancel_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    limit_cancel_file: &LimitCancelConfigFile,
    source: ConfigSource,
) {
    if let Some(value) = limit_cancel_file.place_offset_ticks {
        apply_place_offset_ticks(strategy, sources, value, source);
    }
    if let Some(value) = limit_cancel_file.max_wait_bars_for_ack {
        apply_max_wait_bars_for_ack(strategy, sources, value, source);
    }
}

fn apply_market_buy_and_close_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    market_buy_and_close_file: &MarketBuyAndCloseConfigFile,
    source: ConfigSource,
) {
    if let Some(value) = &market_buy_and_close_file.live_order_style {
        apply_live_order_style(strategy, sources, parse_live_order_style(value), source);
    }
    if let Some(value) = market_buy_and_close_file.marketable_limit_offset_ticks {
        apply_marketable_limit_offset_ticks(strategy, sources, value, source);
    }
    if let Some(value) = &market_buy_and_close_file.close_trigger {
        apply_close_trigger(strategy, sources, parse_close_trigger(value), source);
    }
    if let Some(value) = market_buy_and_close_file.entry_ack_timeout_ms {
        apply_entry_ack_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = market_buy_and_close_file.entry_fill_timeout_ms {
        apply_entry_fill_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = market_buy_and_close_file.exit_ack_timeout_ms {
        apply_exit_ack_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = market_buy_and_close_file.exit_fill_timeout_ms {
        apply_exit_fill_timeout_ms(strategy, sources, value, source);
    }
}

fn apply_mock_live_probe_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    mock_live_probe_file: &MockLiveProbeConfigFile,
    source: ConfigSource,
) {
    if let Some(value) = mock_live_probe_file.place_offset_ticks {
        apply_place_offset_ticks(strategy, sources, value, source);
    }
    if let Some(value) = mock_live_probe_file.max_wait_bars_for_ack {
        apply_max_wait_bars_for_ack(strategy, sources, value, source);
    }
}

fn apply_session_gap_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    session_gap_file: &SessionGapConfigFile,
    source: ConfigSource,
) {
    if let Some(value) = session_gap_file.place_offset_ticks {
        apply_place_offset_ticks(strategy, sources, value, source);
    }
    if let Some(value) = session_gap_file.entry_ack_timeout_ms {
        apply_entry_ack_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = session_gap_file.entry_fill_timeout_ms {
        apply_entry_fill_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = session_gap_file.exit_ack_timeout_ms {
        apply_exit_ack_timeout_ms(strategy, sources, value, source);
    }
    if let Some(value) = session_gap_file.exit_fill_timeout_ms {
        apply_exit_fill_timeout_ms(strategy, sources, value, source);
    }

    if let Some(settings) = strategy.session_gap_standalone_mut() {
        if let Some(value) = session_gap_file.k_long {
            settings.k_long = value;
            sources.session_gap_k_long = source;
        }
        if let Some(value) = session_gap_file.k_short {
            settings.k_short = value;
            sources.session_gap_k_short = source;
        }
        if let Some(value) = session_gap_file.signal_minute {
            settings.signal_minute = value;
            sources.session_gap_signal_minute = source;
        }
        if let Some(value) = session_gap_file.wait_hours {
            settings.wait_hours = value;
            sources.session_gap_wait_hours = source;
        }
        if let Some(value) = session_gap_file.k_tp_long {
            settings.k_tp_long = value;
            sources.session_gap_k_tp_long = source;
        }
        if let Some(value) = session_gap_file.k_sl_long {
            settings.k_sl_long = value;
            sources.session_gap_k_sl_long = source;
        }
        if let Some(value) = session_gap_file.k_tp_short {
            settings.k_tp_short = value;
            sources.session_gap_k_tp_short = source;
        }
        if let Some(value) = session_gap_file.k_sl_short {
            settings.k_sl_short = value;
            sources.session_gap_k_sl_short = source;
        }
        if let Some(value) = session_gap_file.long_ex_pct {
            settings.long_ex_pct = value;
            sources.session_gap_long_ex_pct = source;
        }
        if let Some(value) = session_gap_file.short_ex_pct {
            settings.short_ex_pct = value;
            sources.session_gap_short_ex_pct = source;
        }
        if let Some(value) = session_gap_file.start_cash {
            settings.start_cash = value;
            sources.session_gap_start_cash = source;
        }
        if let Some(value) = session_gap_file.cash_factor {
            settings.cash_factor = value;
            sources.session_gap_cash_factor = source;
        }
        if let Some(value) = session_gap_file.max_entry_hour {
            settings.max_entry_hour = value;
            sources.session_gap_max_entry_hour = source;
        }
        if let Some(value) = session_gap_file.close_hour {
            settings.close_hour = value;
            sources.session_gap_close_hour = source;
        }
        if let Some(value) = session_gap_file.close_minute {
            settings.close_minute = value;
            sources.session_gap_close_minute = source;
        }
        if let Some(value) = session_gap_file.session_gap_min {
            settings.session_gap_min = value;
            sources.session_gap_min = source;
        }
        if let Some(value) = session_gap_file.exit_offset_min {
            settings.exit_offset_min = value;
            sources.session_gap_exit_offset_min = source;
        }
        if let Some(value) = session_gap_file.work_weekends {
            settings.work_weekends = value;
            sources.session_gap_work_weekends = source;
        }
    }
}

fn apply_hybrid_intraday_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    hybrid_file: &HybridIntradayConfigFile,
    source: ConfigSource,
) {
    if let Some(value) = &hybrid_file.live_order_style {
        apply_live_order_style(strategy, sources, parse_live_order_style(value), source);
    }
    if let Some(value) = hybrid_file.marketable_limit_offset_ticks {
        apply_marketable_limit_offset_ticks(strategy, sources, value, source);
    }

    if let Some(settings) = strategy.hybrid_intraday_mut() {
        sources.hybrid_intraday = source;
        let strategy = &mut settings.strategy;
        if let Some(value) = &hybrid_file.profile {
            strategy.profile = value.clone();
        }
        if let Some(value) = &hybrid_file.mr_variant {
            strategy.mr_variant = value.clone();
        }
        if let Some(value) = hybrid_file.live_mr_entries_enabled {
            strategy.live_mr_entries_enabled = value;
        }
        if let Some(value) = &hybrid_file.mr_gate_policy {
            strategy.mr_gate_policy = value.clone();
        }
        if let Some(value) = &hybrid_file.risk_gate_mode {
            strategy.risk_gate_mode = value.clone();
        }
        if let Some(value) = &hybrid_file.risk_gate_seed_file {
            strategy.risk_gate_seed_file = Some(value.clone());
        }
        if let Some(value) = &hybrid_file.risk_gate_ledger_key {
            strategy.risk_gate_ledger_key = Some(value.clone());
        }
        if let Some(value) = hybrid_file.risk_gate_persist_in_shadow {
            strategy.risk_gate_persist_in_shadow = value;
        }
        if let Some(value) = &hybrid_file.risk_gate_legacy_session_start_time {
            strategy.risk_gate_legacy_session_start_time = Some(value.clone());
        }
        if let Some(value) = &hybrid_file.risk_gate_legacy_session_end_time {
            strategy.risk_gate_legacy_session_end_time = Some(value.clone());
        }
        if let Some(value) = &hybrid_file.risk_gate_session_policy_transition_date {
            strategy.risk_gate_session_policy_transition_date = Some(value.clone());
        }
        if let Some(value) = &hybrid_file.model_session_start_time {
            strategy.model_session_start_time = value.clone();
        }
        if let Some(value) = &hybrid_file.model_session_end_time {
            strategy.model_session_end_time = value.clone();
        }
        if let Some(value) = hybrid_file.mr_min_range_long {
            strategy.mr_min_range_long = value;
        }
        if let Some(value) = hybrid_file.mr_max_range_long {
            strategy.mr_max_range_long = value;
        }
        if let Some(value) = hybrid_file.mr_k_long {
            strategy.mr_k_long = value;
        }
        if let Some(value) = hybrid_file.mr_take_k_long {
            strategy.mr_take_k_long = value;
        }
        if let Some(value) = hybrid_file.mr_stop_k_long {
            strategy.mr_stop_k_long = value;
        }
        if let Some(value) = hybrid_file.mr_min_range_short {
            strategy.mr_min_range_short = value;
        }
        if let Some(value) = hybrid_file.mr_max_range_short {
            strategy.mr_max_range_short = value;
        }
        if let Some(value) = hybrid_file.mr_k_short {
            strategy.mr_k_short = value;
        }
        if let Some(value) = hybrid_file.mr_take_k_short {
            strategy.mr_take_k_short = value;
        }
        if let Some(value) = hybrid_file.mr_stop_k_short {
            strategy.mr_stop_k_short = value;
        }
        if let Some(value) = &hybrid_file.mr_session_end_time {
            strategy.mr_session_end_time = value.clone();
        }
        if let Some(value) = hybrid_file.mr_exit_offset_min {
            strategy.mr_exit_offset_min = value;
        }
        if let Some(value) = hybrid_file.bo_k {
            strategy.bo_k = value;
        }
        if let Some(value) = hybrid_file.bo_stop1_range {
            strategy.bo_stop1_range = value;
        }
        if let Some(value) = hybrid_file.bo_stop2_range {
            strategy.bo_stop2_range = value;
        }
        if let Some(value) = hybrid_file.bo_big_move_threshold {
            strategy.bo_big_move_threshold = value;
        }
        if let Some(value) = hybrid_file.bo_min_range {
            strategy.bo_min_range = value;
        }
        if let Some(value) = &hybrid_file.bo_min_range_mode {
            strategy.bo_min_range_mode = value.clone();
        }
        if let Some(value) = hybrid_file.bo_exclude_weekends {
            strategy.bo_exclude_weekends = value;
        }
        if let Some(value) = hybrid_file.bo_wait_hours {
            strategy.bo_wait_hours = value;
        }
        if let Some(value) = &hybrid_file.orchestrator_breakout_eod_mode {
            strategy.orchestrator_breakout_eod_mode = value.clone();
        }
        if let Some(value) = &hybrid_file.orchestrator_breakout_overnight_exit_time {
            strategy.orchestrator_breakout_overnight_exit_time = value.clone();
        }
        if let Some(value) = hybrid_file.repair_deadline_sec {
            strategy.repair_deadline_sec = value;
        }
        if let Some(value) = hybrid_file.sl_escalate_timeout_sec {
            strategy.sl_escalate_timeout_sec = value;
        }
        if let Some(value) = hybrid_file.max_repair_retries {
            strategy.max_repair_retries = value;
        }
        if let Some(value) = hybrid_file.repair_backoff_base_sec {
            strategy.repair_backoff_base_sec = value;
        }
        if let Some(value) = hybrid_file.repair_backoff_max_sec {
            strategy.repair_backoff_max_sec = value;
        }
        if let Some(value) = hybrid_file.pending_timeout_sec {
            strategy.pending_timeout_sec = value;
        }
        if let Some(value) = hybrid_file.partial_entry_fill_timeout_ms {
            strategy.partial_entry_fill_timeout_ms = value;
        }
        if let Some(value) = hybrid_file.stop_end_buffer_sec {
            strategy.stop_end_buffer_sec = value;
        }
    }
}

fn apply_alor_usdrubf_hybrid_config_file(
    strategy: &mut StrategyConfig,
    sources: &mut StrategySources,
    alor_file: &AlorUsdrubfHybridConfigFile,
    source: ConfigSource,
) {
    if let Some(settings) = strategy.alor_usdrubf_hybrid_mut() {
        sources.alor_usdrubf_hybrid = source;
        if let Some(value) = &alor_file.model_session_start_time {
            settings.model_session_start_time = value.clone();
        }
        if let Some(value) = &alor_file.model_session_end_time {
            settings.model_session_end_time = value.clone();
        }
        if let Some(value) = alor_file.mr_min_rel_range {
            settings.mr_min_rel_range = value;
        }
        if let Some(value) = alor_file.mr_max_rel_range {
            settings.mr_max_rel_range = value;
        }
        if let Some(value) = alor_file.mr_k_short {
            settings.mr_k_short = value;
        }
        if let Some(value) = alor_file.mr_take_k_short {
            settings.mr_take_k_short = value;
        }
        if let Some(value) = alor_file.mr_stop_k_short {
            settings.mr_stop_k_short = value;
        }
        if let Some(value) = &alor_file.mr_last_entry_time {
            settings.mr_last_entry_time = value.clone();
        }
        if let Some(value) = &alor_file.mr_force_exit_time {
            settings.mr_force_exit_time = value.clone();
        }
        if let Some(value) = alor_file.bo_k {
            settings.bo_k = value;
        }
        if let Some(value) = alor_file.bo_stop1_range {
            settings.bo_stop1_range = value;
        }
        if let Some(value) = alor_file.bo_stop2_range {
            settings.bo_stop2_range = value;
        }
        if let Some(value) = alor_file.bo_big_move_threshold {
            settings.bo_big_move_threshold = value;
        }
        if let Some(value) = alor_file.bo_wait_hours {
            settings.bo_wait_hours = value;
        }
        if let Some(value) = &alor_file.bo_eod_exit_time {
            settings.bo_eod_exit_time = value.clone();
        }
        if let Some(value) = alor_file.commission_pct_per_side {
            settings.commission_pct_per_side = value;
        }
        if let Some(value) = alor_file.position_size_fraction {
            settings.position_size_fraction = value;
        }
        if let Some(value) = alor_file.initial_cash {
            settings.initial_cash = value;
        }
        if let Some(value) = alor_file.enable_live_execution {
            settings.enable_live_execution = value;
        }
        if let Some(value) = alor_file.use_fixed_live_size {
            settings.use_fixed_live_size = value;
        }
        if let Some(value) = alor_file.live_fixed_units {
            settings.live_fixed_units = value;
        }
    }
}

fn apply_ri_author41_42_config_file(
    strategy: &mut StrategyConfig,
    ri_file: &RiAuthor4142ConfigFile,
) {
    if let Some(settings) = strategy.ri_author41_42_mut() {
        if let Some(value) = &ri_file.profile_id {
            settings.profile_id = value.clone();
        }
        if let Some(value) = &ri_file.timeframe {
            settings.timeframe = value.clone();
        }
        if let Some(value) = &ri_file.mode {
            settings.mode = value.clone();
        }
        if let Some(value) = ri_file.allow_order_emission {
            settings.allow_order_emission = value;
        }
        if let Some(value) = &ri_file.execution_path {
            settings.execution_path = value.clone();
        }
        if let Some(value) = &ri_file.order_symbol {
            settings.order_symbol = Some(value.clone());
        }
        if let Some(value) = &ri_file.session_start_time {
            settings.session_start_time = value.clone();
        }
        if let Some(value) = &ri_file.session_end_time {
            settings.session_end_time = value.clone();
        }
        if let Some(value) = &ri_file.author41_entry_end_time {
            settings.author41_entry_end_time = value.clone();
        }
        if let Some(value) = &ri_file.author41_time_exit {
            settings.author41_time_exit = value.clone();
        }
        if let Some(value) = &ri_file.author42_exit_time {
            settings.author42_exit_time = value.clone();
        }
        if let Some(value) = &ri_file.author42_last_entry_time {
            settings.author42_last_entry_time = Some(value.clone());
        }
        if let Some(value) = ri_file.author42_max_entries_per_day {
            settings.author42_max_entries_per_day = Some(value);
        }
        if let Some(value) = &ri_file.excluded_model_dates {
            settings.excluded_model_dates = value.clone();
        }
        if let Some(value) = ri_file.min_anchor_bars {
            settings.min_anchor_bars = value;
        }
        if let Some(value) = &ri_file.anchor_first_bar_at_or_before {
            settings.anchor_first_bar_at_or_before = value.clone();
        }
        if let Some(value) = &ri_file.anchor_last_bar_at_or_after {
            settings.anchor_last_bar_at_or_after = value.clone();
        }
        if let Some(value) = &ri_file.anchor_transition_date {
            settings.anchor_transition_date = Some(value.clone());
        }
        if let Some(value) = ri_file.pre_transition_min_anchor_bars {
            settings.pre_transition_min_anchor_bars = Some(value);
        }
        if let Some(value) = &ri_file.pre_transition_anchor_first_bar_at_or_before {
            settings.pre_transition_anchor_first_bar_at_or_before = Some(value.clone());
        }
        if let Some(value) = &ri_file.pre_transition_anchor_last_bar_at_or_after {
            settings.pre_transition_anchor_last_bar_at_or_after = Some(value.clone());
        }
        if let Some(value) = &ri_file.actual_expiry_date {
            settings.actual_expiry_date = Some(value.clone());
        }
        if let Some(value) = ri_file.roll_target_sessions_before {
            settings.roll_target_sessions_before = value;
        }
        if let Some(value) = ri_file.roll_fallback_sessions_before {
            settings.roll_fallback_sessions_before = value;
        }
        if let Some(value) = &ri_file.decision_journal_path {
            settings.decision_journal_path = Some(value.clone());
        }
        if let Some(value) = ri_file.decision_journal_append {
            settings.decision_journal_append = value;
        }
    }
}

fn validate_matching_strategy_specific_sections(
    strategy_file: &StrategyConfigFile,
    kind: StrategyKind,
) -> Result<()> {
    let mut mismatches = Vec::new();

    if strategy_file.limit_cancel.is_some() && kind != StrategyKind::LimitCancel {
        mismatches.push("strategy.limit_cancel");
    }
    if strategy_file.market_buy_and_close.is_some() && kind != StrategyKind::MarketBuyAndClose {
        mismatches.push("strategy.market_buy_and_close");
    }
    if strategy_file.mock_live_probe.is_some() && kind != StrategyKind::MockLiveProbe {
        mismatches.push("strategy.mock_live_probe");
    }
    if strategy_file.session_gap.is_some() && kind != StrategyKind::SessionGapStandalone {
        mismatches.push("strategy.session_gap");
    }
    if strategy_file.hybrid_intraday.is_some() && kind != StrategyKind::HybridIntraday {
        mismatches.push("strategy.hybrid_intraday");
    }
    if strategy_file.alor_usdrubf_hybrid.is_some() && kind != StrategyKind::AlorUsdrubfHybrid {
        mismatches.push("strategy.alor_usdrubf_hybrid");
    }
    if strategy_file.alor_skeleton.is_some() && kind != StrategyKind::AlorUsdrubfHybrid {
        mismatches.push("strategy.alor_skeleton");
    }
    if strategy_file.ri_author41_42.is_some() && kind != StrategyKind::RiAuthor4142 {
        mismatches.push("strategy.ri_author41_42");
    }

    if mismatches.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "non-matching strategy specific section(s) for strategy_kind={:?}: {}",
        kind,
        mismatches.join(", ")
    );
}

pub fn load_runtime_config(
    config_path: PathBuf,
    allow_missing: bool,
) -> Result<ResolvedRuntimeConfig> {
    let (file_config, file_loaded) = load_file_config(&config_path, allow_missing)?;
    let mut sources = ConfigSources::default();

    let mut redis_url = DEFAULT_REDIS_URL.to_string();
    let mut portfolio = DEFAULT_PORTFOLIO.to_string();
    let mut exchange = DEFAULT_EXCHANGE.to_string();
    let mut source = DEFAULT_SOURCE.to_string();

    let mut consumer_group = DEFAULT_CONSUMER_GROUP.to_string();
    let mut consumer_name = DEFAULT_CONSUMER_NAME.to_string();

    let mut strategy = StrategyConfig::defaults_for_kind(DEFAULT_STRATEGY_KIND);

    let mut read = ReadConfig {
        block_ms: DEFAULT_BLOCK_MS,
        claim_idle_ms: DEFAULT_CLAIM_IDLE_MS,
        claim_batch: DEFAULT_CLAIM_BATCH,
        poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
    };

    let mut trade_mode = DEFAULT_TRADE_MODE;
    let mut allow_live_orders = DEFAULT_ALLOW_LIVE_ORDERS;
    let mut allow_paper_orders = DEFAULT_ALLOW_PAPER_ORDERS;
    let mut guard_log_interval_ms = DEFAULT_GUARD_LOG_INTERVAL_MS;
    let mut still_blocked_log_period_sec = DEFAULT_STILL_BLOCKED_LOG_PERIOD_SEC;
    let mut gateway_health_stale_sec = DEFAULT_GATEWAY_HEALTH_STALE_SEC;
    let mut require_gateway_ready = DEFAULT_REQUIRE_GATEWAY_READY;
    let mut bootstrap_dump = DEFAULT_BOOTSTRAP_DUMP;
    let mut health = HealthServerConfig {
        enabled: DEFAULT_RUNTIME_HEALTH_ENABLED,
        listen_addr: DEFAULT_RUNTIME_HEALTH_LISTEN_ADDR.to_string(),
        expose_metrics: DEFAULT_RUNTIME_HEALTH_EXPOSE_METRICS,
    };
    let mut paper = PaperConfig {
        enabled: DEFAULT_PAPER_ENABLED,
        output: DEFAULT_PAPER_OUTPUT,
        execution_mode: DEFAULT_PAPER_EXECUTION_MODE,
        file_path: DEFAULT_PAPER_FILE_PATH.to_string(),
        trades_csv: DEFAULT_PAPER_TRADES_CSV.to_string(),
        summary_json: DEFAULT_PAPER_SUMMARY_JSON.to_string(),
        append: DEFAULT_PAPER_APPEND,
    };
    let mut backtest = BacktestConfig {
        enabled: DEFAULT_BACKTEST_ENABLED,
        trade_log: DEFAULT_BACKTEST_TRADE_LOG.to_string(),
        trades_csv: DEFAULT_BACKTEST_TRADES_CSV.to_string(),
        summary_json: DEFAULT_BACKTEST_SUMMARY_JSON.to_string(),
        append: DEFAULT_BACKTEST_APPEND,
    };
    let mut replay = ReplayConfig {
        enabled: DEFAULT_REPLAY_ENABLED,
        bars_csv_path: DEFAULT_REPLAY_BARS_CSV_PATH.map(ToString::to_string),
        reference_trades_csv_path: DEFAULT_REPLAY_REFERENCE_TRADES_CSV_PATH
            .map(ToString::to_string),
        output_dir: DEFAULT_REPLAY_OUTPUT_DIR.to_string(),
        price_tolerance: DEFAULT_REPLAY_PRICE_TOLERANCE,
        strict_dedup: DEFAULT_REPLAY_STRICT_DEDUP,
    };

    let mut trim = TrimConfig {
        bars: DEFAULT_TRIM_BARS,
        orders: DEFAULT_TRIM_ORDERS,
        trades: DEFAULT_TRIM_TRADES,
        positions: DEFAULT_TRIM_POSITIONS,
        commands: DEFAULT_TRIM_COMMANDS,
        acks: DEFAULT_TRIM_ACKS,
        health: DEFAULT_TRIM_HEALTH,
        runtime_state: DEFAULT_TRIM_RUNTIME_STATE,
    };

    let mut reset_state_on_start = false;

    if let Some(file_config) = &file_config {
        if let Some(value) = &file_config.redis_url {
            redis_url = value.clone();
            sources.redis_url = ConfigSource::File;
        }
        if let Some(value) = &file_config.portfolio {
            portfolio = value.clone();
            sources.portfolio = ConfigSource::File;
        }
        if let Some(value) = &file_config.exchange {
            exchange = value.clone();
            sources.exchange = ConfigSource::File;
        }
        if let Some(value) = &file_config.source {
            source = value.clone();
            sources.source = ConfigSource::File;
        }
        if let Some(value) = &file_config.consumer_group {
            consumer_group = value.clone();
            sources.consumer_group = ConfigSource::File;
        }
        if let Some(value) = &file_config.consumer_name {
            consumer_name = value.clone();
            sources.consumer_name = ConfigSource::File;
        }
        if let Some(read_file) = &file_config.read {
            if let Some(value) = read_file.block_ms {
                read.block_ms = value;
                sources.read.block_ms = ConfigSource::File;
            }
            if let Some(value) = read_file.claim_idle_ms {
                read.claim_idle_ms = value;
                sources.read.claim_idle_ms = ConfigSource::File;
            }
            if let Some(value) = read_file.claim_batch {
                read.claim_batch = value;
                sources.read.claim_batch = ConfigSource::File;
            }
            if let Some(value) = read_file.poll_interval_ms {
                read.poll_interval_ms = value;
                sources.read.poll_interval_ms = ConfigSource::File;
            }
        }
        if let Some(trim_file) = &file_config.trim {
            if let Some(value) = trim_file.bars {
                trim.bars = value;
                sources.trim.bars = ConfigSource::File;
            }
            if let Some(value) = trim_file.orders {
                trim.orders = value;
                sources.trim.orders = ConfigSource::File;
            }
            if let Some(value) = trim_file.trades {
                trim.trades = value;
                sources.trim.trades = ConfigSource::File;
            }
            if let Some(value) = trim_file.positions {
                trim.positions = value;
                sources.trim.positions = ConfigSource::File;
            }
            if let Some(value) = trim_file.commands {
                trim.commands = value;
                sources.trim.commands = ConfigSource::File;
            }
            if let Some(value) = trim_file.acks {
                trim.acks = value;
                sources.trim.acks = ConfigSource::File;
            }
            if let Some(value) = trim_file.health {
                trim.health = value;
                sources.trim.health = ConfigSource::File;
            }
            if let Some(value) = trim_file.runtime_state {
                trim.runtime_state = value;
                sources.trim.runtime_state = ConfigSource::File;
            }
        }
        if let Some(strategy_file) = &file_config.strategy {
            apply_strategy_common_config_file(
                &mut strategy,
                &mut sources.strategy,
                &strategy_file.common_legacy,
                ConfigSource::File,
                "strategy.strategy_kind",
            )?;
            if let Some(common_file) = &strategy_file.common {
                apply_strategy_common_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    common_file,
                    ConfigSource::File,
                    "strategy.common.strategy_kind",
                )?;
            }
            if strategy.trading_periods.is_none() {
                if let Some(value) = &file_config.trading_periods {
                    strategy.trading_periods = Some(value.clone());
                    sources.strategy.trading_periods = ConfigSource::File;
                }
            }
            validate_matching_strategy_specific_sections(strategy_file, strategy.strategy_kind)?;
            apply_legacy_specific_config_file(
                &mut strategy,
                &mut sources.strategy,
                &strategy_file.legacy_specific,
                ConfigSource::File,
            );
            if let Some(limit_cancel_file) = &strategy_file.limit_cancel {
                apply_limit_cancel_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    limit_cancel_file,
                    ConfigSource::File,
                );
            }
            if let Some(market_buy_and_close_file) = &strategy_file.market_buy_and_close {
                apply_market_buy_and_close_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    market_buy_and_close_file,
                    ConfigSource::File,
                );
            }
            if let Some(mock_live_probe_file) = &strategy_file.mock_live_probe {
                apply_mock_live_probe_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    mock_live_probe_file,
                    ConfigSource::File,
                );
            }
            if let Some(session_gap_file) = &strategy_file.session_gap {
                apply_session_gap_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    session_gap_file,
                    ConfigSource::File,
                );
            }
            if let Some(hybrid_file) = &strategy_file.hybrid_intraday {
                apply_hybrid_intraday_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    hybrid_file,
                    ConfigSource::File,
                );
            }
            if let Some(alor_file) = strategy_file
                .alor_usdrubf_hybrid
                .as_ref()
                .or(strategy_file.alor_skeleton.as_ref())
            {
                apply_alor_usdrubf_hybrid_config_file(
                    &mut strategy,
                    &mut sources.strategy,
                    alor_file,
                    ConfigSource::File,
                );
            }
            if let Some(ri_file) = &strategy_file.ri_author41_42 {
                apply_ri_author41_42_config_file(&mut strategy, ri_file);
            }
        }
        if let Some(runtime_file) = &file_config.runtime {
            if let Some(value) = &runtime_file.trade_mode {
                trade_mode = parse_trade_mode(value);
                sources.runtime.trade_mode = ConfigSource::File;
            }
            if let Some(value) = runtime_file.allow_live_orders {
                allow_live_orders = value;
                sources.runtime.allow_live_orders = ConfigSource::File;
            }
            if let Some(value) = runtime_file.allow_paper_orders {
                allow_paper_orders = value;
                sources.runtime.allow_paper_orders = ConfigSource::File;
            }
            if let Some(value) = runtime_file.guard_log_interval_ms {
                guard_log_interval_ms = value;
                sources.runtime.guard_log_interval_ms = ConfigSource::File;
            }
            if let Some(value) = runtime_file.still_blocked_log_period_sec {
                still_blocked_log_period_sec = value;
                sources.runtime.still_blocked_log_period_sec = ConfigSource::File;
            }
            if let Some(value) = runtime_file.gateway_health_stale_sec {
                gateway_health_stale_sec = value;
                sources.runtime.gateway_health_stale_sec = ConfigSource::File;
            }
            if let Some(value) = runtime_file.require_gateway_ready {
                require_gateway_ready = value;
                sources.runtime.require_gateway_ready = ConfigSource::File;
            }
            if let Some(value) = runtime_file.bootstrap_dump {
                bootstrap_dump = value;
                sources.runtime.bootstrap_dump = ConfigSource::File;
            }
            if let Some(health_file) = &runtime_file.health {
                if let Some(value) = health_file.enabled {
                    health.enabled = value;
                    sources.runtime.health_enabled = ConfigSource::File;
                }
                if let Some(value) = &health_file.listen_addr {
                    health.listen_addr = value.clone();
                    sources.runtime.health_listen_addr = ConfigSource::File;
                }
                if let Some(value) = health_file.expose_metrics {
                    health.expose_metrics = value;
                    sources.runtime.health_expose_metrics = ConfigSource::File;
                }
            }
        }
        if let Some(paper_file) = &file_config.paper {
            if let Some(value) = paper_file.enabled {
                paper.enabled = value;
                sources.paper.enabled = ConfigSource::File;
            }
            if let Some(value) = &paper_file.output {
                paper.output = parse_paper_output(value);
                sources.paper.output = ConfigSource::File;
            }
            if let Some(value) = &paper_file.execution_mode {
                paper.execution_mode = parse_paper_execution_mode(value);
                sources.paper.execution_mode = ConfigSource::File;
            }
            if let Some(value) = &paper_file.file_path {
                paper.file_path = value.clone();
                sources.paper.file_path = ConfigSource::File;
            }
            if let Some(value) = &paper_file.trades_csv {
                paper.trades_csv = value.clone();
                sources.paper.trades_csv = ConfigSource::File;
            }
            if let Some(value) = &paper_file.summary_json {
                paper.summary_json = value.clone();
                sources.paper.summary_json = ConfigSource::File;
            }
            if let Some(value) = paper_file.append {
                paper.append = value;
                sources.paper.append = ConfigSource::File;
            }
        }
        if let Some(backtest_file) = &file_config.backtest {
            if let Some(value) = backtest_file.enabled {
                backtest.enabled = value;
                sources.backtest.enabled = ConfigSource::File;
            }
            if let Some(value) = &backtest_file.trade_log {
                backtest.trade_log = value.clone();
                sources.backtest.trade_log = ConfigSource::File;
            }
            if let Some(value) = &backtest_file.trades_csv {
                backtest.trades_csv = value.clone();
                sources.backtest.trades_csv = ConfigSource::File;
            }
            if let Some(value) = &backtest_file.summary_json {
                backtest.summary_json = value.clone();
                sources.backtest.summary_json = ConfigSource::File;
            }
            if let Some(value) = backtest_file.append {
                backtest.append = value;
                sources.backtest.append = ConfigSource::File;
            }
        }
        if let Some(replay_file) = &file_config.replay {
            if let Some(value) = replay_file.enabled {
                replay.enabled = value;
                sources.replay.enabled = ConfigSource::File;
            }
            if let Some(value) = &replay_file.bars_csv_path {
                replay.bars_csv_path = Some(value.clone());
                sources.replay.bars_csv_path = ConfigSource::File;
            }
            if let Some(value) = &replay_file.reference_trades_csv_path {
                replay.reference_trades_csv_path = Some(value.clone());
                sources.replay.reference_trades_csv_path = ConfigSource::File;
            }
            if let Some(value) = &replay_file.output_dir {
                replay.output_dir = value.clone();
                sources.replay.output_dir = ConfigSource::File;
            }
            if let Some(value) = replay_file.price_tolerance {
                replay.price_tolerance = value;
                sources.replay.price_tolerance = ConfigSource::File;
            }
            if let Some(value) = replay_file.strict_dedup {
                replay.strict_dedup = value;
                sources.replay.strict_dedup = ConfigSource::File;
            }
        }
        if let Some(value) = file_config.reset_state_on_start {
            reset_state_on_start = value;
            sources.reset_state_on_start = ConfigSource::File;
        }
    }

    if let Ok(value) = env::var("REDIS_URL") {
        redis_url = value;
        sources.redis_url = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PORTFOLIO") {
        portfolio = value;
        sources.portfolio = ConfigSource::Env;
    }
    if let Ok(value) = env::var("EXCHANGE") {
        exchange = value;
        sources.exchange = ConfigSource::Env;
    }
    if let Ok(value) = env::var("SOURCE") {
        source = value;
        sources.source = ConfigSource::Env;
    }
    if let Ok(value) = env::var("CONSUMER_GROUP") {
        consumer_group = value;
        sources.consumer_group = ConfigSource::Env;
    }
    if let Ok(value) = env::var("CONSUMER_NAME") {
        consumer_name = value;
        sources.consumer_name = ConfigSource::Env;
    }
    if let Some(value) = env_parse("BLOCK_MS") {
        read.block_ms = value;
        sources.read.block_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("CLAIM_IDLE_MS") {
        read.claim_idle_ms = value;
        sources.read.claim_idle_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("CLAIM_BATCH") {
        read.claim_batch = value;
        sources.read.claim_batch = ConfigSource::Env;
    }
    if let Some(value) = env_parse("POLL_INTERVAL_MS") {
        read.poll_interval_ms = value;
        sources.read.poll_interval_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_BARS") {
        trim.bars = value;
        sources.trim.bars = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_ORDERS") {
        trim.orders = value;
        sources.trim.orders = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_TRADES") {
        trim.trades = value;
        sources.trim.trades = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_POSITIONS") {
        trim.positions = value;
        sources.trim.positions = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_COMMANDS") {
        trim.commands = value;
        sources.trim.commands = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_ACKS") {
        trim.acks = value;
        sources.trim.acks = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_HEALTH") {
        trim.health = value;
        sources.trim.health = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TRIM_MAXLEN_RUNTIME_STATE") {
        trim.runtime_state = value;
        sources.trim.runtime_state = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STRATEGY_ID") {
        strategy.strategy_id = value;
        sources.strategy.strategy_id = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STRATEGY_KIND") {
        strategy.set_kind(
            parse_strategy_kind(&value)
                .with_context(|| format!("invalid STRATEGY_KIND: {value}"))?,
        );
        sources.strategy.strategy_kind = ConfigSource::Env;
    }
    if let Ok(value) = env::var("SYMBOL") {
        strategy.symbol = value;
        sources.strategy.symbol = ConfigSource::Env;
    }
    if let Some(value) = env_parse("QTY") {
        strategy.qty = value;
        sources.strategy.qty = ConfigSource::Env;
    }
    if let Ok(value) = env::var("SIDE") {
        strategy.side = parse_side(&value);
        sources.strategy.side = ConfigSource::Env;
    }
    if let Ok(value) = env::var("LIVE_ORDER_STYLE") {
        apply_live_order_style(
            &mut strategy,
            &mut sources.strategy,
            parse_live_order_style(&value),
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("MARKETABLE_LIMIT_OFFSET_TICKS") {
        apply_marketable_limit_offset_ticks(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("PLACE_OFFSET_TICKS") {
        apply_place_offset_ticks(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("TICK_SIZE") {
        strategy.tick_size = value;
        sources.strategy.tick_size = ConfigSource::Env;
    }
    if let Some(value) = env_parse("MAX_WAIT_BARS_FOR_ACK") {
        apply_max_wait_bars_for_ack(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Ok(value) = env::var("CLOSE_TRIGGER") {
        apply_close_trigger(
            &mut strategy,
            &mut sources.strategy,
            parse_close_trigger(&value),
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("ENTRY_ACK_TIMEOUT_MS") {
        apply_entry_ack_timeout_ms(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("ENTRY_FILL_TIMEOUT_MS") {
        apply_entry_fill_timeout_ms(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("EXIT_ACK_TIMEOUT_MS") {
        apply_exit_ack_timeout_ms(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("EXIT_FILL_TIMEOUT_MS") {
        apply_exit_fill_timeout_ms(
            &mut strategy,
            &mut sources.strategy,
            value,
            ConfigSource::Env,
        );
    }
    if let Some(value) = env_parse("SESSION_OPEN_HOUR") {
        strategy.session_open_hour = value;
        sources.strategy.session_open_hour = ConfigSource::Env;
    }
    if let Some(value) = env_parse("SESSION_OPEN_MINUTE") {
        strategy.session_open_minute = value;
        sources.strategy.session_open_minute = ConfigSource::Env;
    }
    if let Some(value) = env_parse("SESSION_CLOSE_HOUR") {
        strategy.session_close_hour = value;
        sources.strategy.session_close_hour = ConfigSource::Env;
    }
    if let Some(value) = env_parse("SESSION_CLOSE_MINUTE") {
        strategy.session_close_minute = value;
        sources.strategy.session_close_minute = ConfigSource::Env;
    }
    if let Some(value) = env_parse("ENTRY_AFTER_OPEN_MIN") {
        strategy.entry_after_open_min = value;
        sources.strategy.entry_after_open_min = ConfigSource::Env;
    }
    if let Some(value) = env_parse("EXIT_BEFORE_CLOSE_MIN") {
        strategy.exit_before_close_min = value;
        sources.strategy.exit_before_close_min = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TIMEZONE_OFFSET_HOURS") {
        strategy.timezone_offset_hours = value;
        sources.strategy.timezone_offset_hours = ConfigSource::Env;
    }
    if let Ok(value) = env::var("TRADE_MODE") {
        trade_mode = parse_trade_mode(&value);
        sources.runtime.trade_mode = ConfigSource::Env;
    }
    if let Ok(value) = env::var("ALLOW_LIVE_ORDERS") {
        allow_live_orders = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.allow_live_orders = ConfigSource::Env;
    }
    if let Ok(value) = env::var("ALLOW_PAPER_ORDERS") {
        allow_paper_orders = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.allow_paper_orders = ConfigSource::Env;
    }
    if let Some(value) = env_parse("GUARD_LOG_INTERVAL_MS") {
        guard_log_interval_ms = value;
        sources.runtime.guard_log_interval_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("STILL_BLOCKED_LOG_PERIOD_SEC") {
        still_blocked_log_period_sec = value;
        sources.runtime.still_blocked_log_period_sec = ConfigSource::Env;
    }
    if let Some(value) = env_parse("GATEWAY_HEALTH_STALE_SEC") {
        gateway_health_stale_sec = value;
        sources.runtime.gateway_health_stale_sec = ConfigSource::Env;
    }
    if let Ok(value) = env::var("REQUIRE_GATEWAY_READY") {
        require_gateway_ready = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.require_gateway_ready = ConfigSource::Env;
    }
    if let Ok(value) = env::var("BOOTSTRAP_DUMP") {
        bootstrap_dump = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.bootstrap_dump = ConfigSource::Env;
    }
    if let Ok(value) = env::var("RUNTIME_HEALTH_ENABLED") {
        health.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.health_enabled = ConfigSource::Env;
    }
    if let Ok(value) = env::var("RUNTIME_HEALTH_LISTEN_ADDR") {
        health.listen_addr = value;
        sources.runtime.health_listen_addr = ConfigSource::Env;
    }
    if let Ok(value) = env::var("RUNTIME_HEALTH_EXPOSE_METRICS") {
        health.expose_metrics = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.health_expose_metrics = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PAPER_ENABLED") {
        paper.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.paper.enabled = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PAPER_OUTPUT") {
        paper.output = parse_paper_output(&value);
        sources.paper.output = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PAPER_EXECUTION_MODE") {
        paper.execution_mode = parse_paper_execution_mode(&value);
        sources.paper.execution_mode = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PAPER_FILE_PATH") {
        paper.file_path = value;
        sources.paper.file_path = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PAPER_TRADES_CSV") {
        paper.trades_csv = value;
        sources.paper.trades_csv = ConfigSource::Env;
    }
    if let Ok(value) = env::var("PAPER_SUMMARY_JSON") {
        paper.summary_json = value;
        sources.paper.summary_json = ConfigSource::Env;
    }
    if let Some(value) = env_parse("PAPER_APPEND") {
        paper.append = value;
        sources.paper.append = ConfigSource::Env;
    }
    if let Ok(value) = env::var("BACKTEST_ENABLED") {
        backtest.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.backtest.enabled = ConfigSource::Env;
    }
    if let Ok(value) = env::var("BACKTEST_TRADE_LOG") {
        backtest.trade_log = value;
        sources.backtest.trade_log = ConfigSource::Env;
    }
    if let Ok(value) = env::var("BACKTEST_TRADES_CSV") {
        backtest.trades_csv = value;
        sources.backtest.trades_csv = ConfigSource::Env;
    }
    if let Ok(value) = env::var("BACKTEST_SUMMARY_JSON") {
        backtest.summary_json = value;
        sources.backtest.summary_json = ConfigSource::Env;
    }
    if let Some(value) = env_parse("BACKTEST_APPEND") {
        backtest.append = value;
        sources.backtest.append = ConfigSource::Env;
    }
    if let Ok(value) = env::var("REPLAY_ENABLED") {
        replay.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.replay.enabled = ConfigSource::Env;
    }
    if let Ok(value) = env::var("REPLAY_BARS_CSV_PATH") {
        replay.bars_csv_path = Some(value);
        sources.replay.bars_csv_path = ConfigSource::Env;
    }
    if let Ok(value) = env::var("REPLAY_REFERENCE_TRADES_CSV_PATH") {
        replay.reference_trades_csv_path = Some(value);
        sources.replay.reference_trades_csv_path = ConfigSource::Env;
    }
    if let Ok(value) = env::var("REPLAY_OUTPUT_DIR") {
        replay.output_dir = value;
        sources.replay.output_dir = ConfigSource::Env;
    }
    if let Some(value) = env_parse::<f64>("REPLAY_PRICE_TOLERANCE") {
        replay.price_tolerance = value;
        sources.replay.price_tolerance = ConfigSource::Env;
    }
    if let Some(value) = env_parse("REPLAY_STRICT_DEDUP") {
        replay.strict_dedup = value;
        sources.replay.strict_dedup = ConfigSource::Env;
    }
    if let Ok(value) = env::var("RESET_STATE_ON_START") {
        reset_state_on_start = value == "1" || value.eq_ignore_ascii_case("true");
        sources.reset_state_on_start = ConfigSource::Env;
    }

    let mut streams = default_streams(&portfolio, &strategy.strategy_id);
    if let Some(streams_file) = file_config.as_ref().and_then(|cfg| cfg.streams.as_ref()) {
        if let Some(value) = &streams_file.bars {
            streams.bars = value.clone();
            sources.streams.bars = ConfigSource::File;
        }
        if let Some(value) = &streams_file.orders {
            streams.orders = value.clone();
            sources.streams.orders = ConfigSource::File;
        }
        if let Some(value) = &streams_file.trades {
            streams.trades = value.clone();
            sources.streams.trades = ConfigSource::File;
        }
        if let Some(value) = &streams_file.positions {
            streams.positions = value.clone();
            sources.streams.positions = ConfigSource::File;
        }
        if let Some(value) = &streams_file.commands {
            streams.commands = value.clone();
            sources.streams.commands = ConfigSource::File;
        }
        if let Some(value) = &streams_file.acks {
            streams.acks = value.clone();
            sources.streams.acks = ConfigSource::File;
        }
        if let Some(value) = &streams_file.snapshots {
            streams.snapshots = parse_optional_stream(value);
            sources.streams.snapshots = ConfigSource::File;
        }
        if let Some(value) = &streams_file.health {
            streams.health = parse_optional_stream(value);
            sources.streams.health = ConfigSource::File;
        }
        if let Some(value) = &streams_file.dlq_prefix {
            streams.dlq_prefix = value.clone();
            sources.streams.dlq_prefix = ConfigSource::File;
        }
        if let Some(value) = &streams_file.runtime_state {
            streams.runtime_state = value.clone();
            sources.streams.runtime_state = ConfigSource::File;
        }
    }

    if let Ok(value) = env::var("STREAM_BARS") {
        streams.bars = value;
        sources.streams.bars = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_ORDERS") {
        streams.orders = value;
        sources.streams.orders = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_TRADES") {
        streams.trades = value;
        sources.streams.trades = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_POSITIONS") {
        streams.positions = value;
        sources.streams.positions = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_COMMANDS") {
        streams.commands = value;
        sources.streams.commands = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_ACKS") {
        streams.acks = value;
        sources.streams.acks = ConfigSource::Env;
    }
    if let Ok(value) = env::var("SNAPSHOTS_STREAM") {
        streams.snapshots = parse_optional_stream(&value);
        sources.streams.snapshots = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_HEALTH") {
        streams.health = parse_optional_stream(&value);
        sources.streams.health = ConfigSource::Env;
    }
    if let Ok(value) = env::var("STREAM_DLQ_PREFIX") {
        streams.dlq_prefix = value;
        sources.streams.dlq_prefix = ConfigSource::Env;
    }
    if let Ok(value) = env::var("RUNTIME_STATE_STREAM") {
        streams.runtime_state = value;
        sources.streams.runtime_state = ConfigSource::Env;
    }

    let config = RuntimeConfig {
        redis_url,
        source,
        portfolio,
        exchange,
        streams,
        consumer_group,
        consumer_name,
        trade_mode,
        allow_live_orders,
        allow_paper_orders,
        guard_log_interval_ms,
        still_blocked_log_period_sec,
        gateway_health_stale_sec,
        require_gateway_ready,
        bootstrap_dump,
        health,
        read,
        trim,
        strategy,
        paper,
        backtest,
        replay,
        reset_state_on_start,
    };

    validate_trade_mode(&config)?;
    validate_timezone_offset_hours(&config)?;

    Ok(ResolvedRuntimeConfig {
        config,
        sources,
        path: config_path,
        file_loaded,
    })
}

fn load_file_config(path: &Path, allow_missing: bool) -> Result<(Option<RuntimeConfigFile>, bool)> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let config: RuntimeConfigFile = toml::from_str(&contents)
                .with_context(|| format!("failed to parse config file {}", path.display()))?;
            Ok((Some(config), true))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
            Ok((None, false))
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to read config file {}", path.display()))
        }
    }
}

fn default_streams(portfolio: &str, strategy_id: &str) -> StreamNames {
    StreamNames {
        bars: format!("md.bars.{portfolio}.1m"),
        orders: format!("broker.orders.{portfolio}"),
        trades: format!("broker.trades.{portfolio}"),
        positions: format!("broker.positions.{portfolio}"),
        commands: format!("cmd.orders.{portfolio}"),
        acks: format!("cmd.acks.{portfolio}"),
        snapshots: Some(format!("broker.snapshots.{portfolio}")),
        health: Some(DEFAULT_HEALTH_STREAM.to_string()),
        dlq_prefix: "dlq".to_string(),
        runtime_state: format!("runtime.state.{strategy_id}.{portfolio}"),
    }
}

fn parse_optional_stream(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_side(value: &str) -> alor_protocol::Side {
    match value.to_lowercase().as_str() {
        "sell" => alor_protocol::Side::Sell,
        _ => alor_protocol::Side::Buy,
    }
}

fn parse_trade_mode(value: &str) -> TradeMode {
    match value.to_lowercase().as_str() {
        "paper" => TradeMode::Paper,
        "backtest" => TradeMode::Backtest,
        _ => TradeMode::Live,
    }
}

fn parse_strategy_kind(value: &str) -> Result<StrategyKind> {
    match value.to_lowercase().as_str() {
        "limit_cancel" | "limitcancel" => Ok(StrategyKind::LimitCancel),
        "market_buy_and_close" | "marketbuyandclose" => Ok(StrategyKind::MarketBuyAndClose),
        "mock_live_probe" | "mockliveprobe" => Ok(StrategyKind::MockLiveProbe),
        "toy_session_timing" | "toysessiontiming" => Ok(StrategyKind::ToySessionTiming),
        "session_gap_standalone" | "sessiongapstandalone" => Ok(StrategyKind::SessionGapStandalone),
        "hybrid_intraday" | "hybridintraday" | "hybrid" => Ok(StrategyKind::HybridIntraday),
        "alor_usdrubf_hybrid" | "alorusdrubfhybrid" => Ok(StrategyKind::AlorUsdrubfHybrid),
        "alor_skeleton" | "alorskeleton" | "alor" => Ok(StrategyKind::AlorUsdrubfHybrid),
        "ri_author41_42" | "riauthor4142" | "ri_author4142" => Ok(StrategyKind::RiAuthor4142),
        _ => Err(anyhow!("unknown strategy_kind: {value}")),
    }
}

fn parse_close_trigger(value: &str) -> CloseTrigger {
    match value.to_lowercase().as_str() {
        "position_update" | "positionupdate" => CloseTrigger::PositionUpdate,
        _ => CloseTrigger::NextBar,
    }
}

fn parse_live_order_style(value: &str) -> MarketBuyAndCloseLiveOrderStyle {
    match value.to_ascii_lowercase().as_str() {
        "marketable_limit" | "marketablelimit" => MarketBuyAndCloseLiveOrderStyle::MarketableLimit,
        _ => MarketBuyAndCloseLiveOrderStyle::Market,
    }
}

fn parse_paper_output(value: &str) -> PaperOutput {
    match value.to_lowercase().as_str() {
        "file" => PaperOutput::File,
        _ => PaperOutput::Stdout,
    }
}

fn parse_paper_execution_mode(value: &str) -> PaperExecutionMode {
    match value.to_lowercase().as_str() {
        "history_sim" | "historysim" => PaperExecutionMode::HistorySim,
        _ => PaperExecutionMode::LiveOnly,
    }
}

fn validate_trade_mode(config: &RuntimeConfig) -> Result<()> {
    let mut conflicts = Vec::new();
    match config.trade_mode {
        TradeMode::Live => {
            if config.paper.enabled {
                conflicts.push("paper.enabled=true");
            }
            if config.backtest.enabled {
                conflicts.push("backtest.enabled=true");
            }
            if !config.allow_live_orders {
                conflicts.push("allow_live_orders=false");
            }
            if config.streams.snapshots.is_none() {
                conflicts.push("streams.snapshots=none");
            }
        }
        TradeMode::Paper | TradeMode::Backtest => {
            if config.allow_live_orders {
                conflicts.push("allow_live_orders=true");
            }
        }
    }
    if conflicts.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "invalid runtime mode: trade_mode={:?} conflicts with {}",
        config.trade_mode,
        conflicts.join(", ")
    );
}

fn validate_timezone_offset_hours(config: &RuntimeConfig) -> Result<()> {
    let value = config.strategy.timezone_offset_hours;
    if (-23..=23).contains(&value) {
        return Ok(());
    }

    anyhow::bail!("invalid strategy.timezone_offset_hours={value}; expected range -23..=23");
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    env::var(key).ok().and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp_config(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "strategy-runtime-{name}-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, body).expect("write temp config");
        path
    }

    #[test]
    fn runtime_reads_trading_periods_from_top_level_when_strategy_section_missing() {
        let path = write_temp_config(
            "top-level-periods",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[strategy]
strategy_id = "session_gap_standalone"
strategy_kind = "session_gap_standalone"
symbol = "USDRUBF"
qty = 1.0
side = "buy"

[trading_periods]
session_start = "09:00:00"
session_end = "23:49:00"
break_start_1 = "14:00:00"
break_end_1 = "14:05:00"
break_start_2 = "18:50:00"
break_end_2 = "19:05:00"
weekends_off = true
timezone_offset_hours = 3
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        let periods = resolved
            .config
            .strategy
            .trading_periods
            .clone()
            .expect("periods");
        assert_eq!(periods.timezone_offset_hours, 3);
        assert_eq!(
            resolved.sources.strategy.trading_periods,
            ConfigSource::File
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_parses_market_buy_and_close_market_style() {
        let path = write_temp_config(
            "market-buy-close-market-style",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[strategy]
strategy_id = "market_buy_and_close"
strategy_kind = "market_buy_and_close"
symbol = "USDRUBF"
qty = 1.0
side = "buy"
live_order_style = "market"
marketable_limit_offset_ticks = 2
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        let strategy = resolved
            .config
            .strategy
            .market_buy_and_close()
            .expect("market buy and close settings");
        assert_eq!(
            strategy.live_order_style,
            MarketBuyAndCloseLiveOrderStyle::Market
        );
        assert_eq!(strategy.marketable_limit_offset_ticks, 2);
        assert_eq!(
            resolved.sources.strategy.live_order_style,
            ConfigSource::File
        );
        assert_eq!(
            resolved.sources.strategy.marketable_limit_offset_ticks,
            ConfigSource::File
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_parses_market_buy_and_close_marketable_limit_style() {
        let path = write_temp_config(
            "market-buy-close-marketable-limit",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[strategy]
strategy_id = "market_buy_and_close"
strategy_kind = "market_buy_and_close"
symbol = "USDRUBF"
qty = 1.0
side = "buy"
live_order_style = "marketable_limit"
marketable_limit_offset_ticks = 3
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        let strategy = resolved
            .config
            .strategy
            .market_buy_and_close()
            .expect("market buy and close settings");
        assert_eq!(
            strategy.live_order_style,
            MarketBuyAndCloseLiveOrderStyle::MarketableLimit
        );
        assert_eq!(strategy.marketable_limit_offset_ticks, 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_defaults_market_buy_and_close_live_order_style_to_market() {
        let path = write_temp_config(
            "market-buy-close-default-style",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[strategy]
strategy_id = "market_buy_and_close"
strategy_kind = "market_buy_and_close"
symbol = "USDRUBF"
qty = 1.0
side = "buy"
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        let strategy = resolved
            .config
            .strategy
            .market_buy_and_close()
            .expect("market buy and close settings");
        assert_eq!(
            strategy.live_order_style,
            MarketBuyAndCloseLiveOrderStyle::Market
        );
        assert_eq!(strategy.marketable_limit_offset_ticks, 0);
        assert_eq!(
            resolved.sources.strategy.live_order_style,
            ConfigSource::Default
        );
        assert_eq!(
            resolved.sources.strategy.marketable_limit_offset_ticks,
            ConfigSource::Default
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_rejects_invalid_timezone_offset_hours() {
        let path = write_temp_config(
            "invalid-timezone-offset",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[strategy]
strategy_id = "session_gap_standalone"
strategy_kind = "session_gap_standalone"
symbol = "USDRUBF"
qty = 1.0
side = "buy"
timezone_offset_hours = 30
"#,
        );

        let err = load_runtime_config(path.clone(), false)
            .expect_err("must fail for invalid timezone offset");
        let message = format!("{err:#}");
        assert!(message.contains("timezone_offset_hours"));
        assert!(message.contains("-23..=23"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_parses_hybrid_strategy_kind_aliases() {
        let path = write_temp_config(
            "hybrid-kind",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[strategy]
strategy_id = "hybrid_intraday"
strategy_kind = "hybrid"
symbol = "IMOEXF"
qty = 1.0
side = "buy"
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        assert_eq!(
            resolved.config.strategy.strategy_kind,
            StrategyKind::HybridIntraday
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_parses_alor_usdrubf_model_session_window() {
        let path = write_temp_config(
            "alor-usdrubf-model-session-window",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "7502MIW"
exchange = "MOEX"

[strategy]
strategy_id = "alor_usdrubf_hybrid_v1"
strategy_kind = "alor_usdrubf_hybrid"
symbol = "USDRUBF"
qty = 2.0
side = "buy"

[strategy.alor_usdrubf_hybrid]
model_session_start_time = "07:00:00"
model_session_end_time = "23:49:59"
mr_last_entry_time = "09:40:00"
mr_force_exit_time = "09:50:00"
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        let settings = resolved
            .config
            .strategy
            .alor_usdrubf_hybrid()
            .expect("alor-usdrubf settings");
        assert_eq!(settings.model_session_start_time, "07:00:00");
        assert_eq!(settings.model_session_end_time, "23:49:59");
        assert_eq!(settings.mr_last_entry_time, "09:40:00");
        assert_eq!(settings.mr_force_exit_time, "09:50:00");
        assert_eq!(
            resolved.sources.strategy.alor_usdrubf_hybrid,
            ConfigSource::File
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_parses_paper_execution_mode_history_sim() {
        let path = write_temp_config(
            "paper-execution-mode",
            r#"
redis_url = "redis://127.0.0.1/"
portfolio = "demo"
exchange = "MOEX"

[paper]
enabled = true
execution_mode = "history_sim"
"#,
        );

        let resolved = load_runtime_config(path.clone(), false).expect("load config");
        assert_eq!(
            resolved.config.paper.execution_mode,
            PaperExecutionMode::HistorySim
        );
        assert_eq!(resolved.sources.paper.execution_mode, ConfigSource::File);
        let _ = std::fs::remove_file(path);
    }
}
