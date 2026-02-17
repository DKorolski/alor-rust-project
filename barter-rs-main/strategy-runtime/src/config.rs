use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{
    BacktestConfig, CloseTrigger, PaperConfig, PaperOutput, ReadConfig, ReplayConfig,
    RuntimeConfig, StrategyConfig, StrategyKind, StreamNames, TradeMode, TrimConfig,
};

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1/";
const DEFAULT_STRATEGY_ID: &str = "limit_cancel";
const DEFAULT_STRATEGY_KIND: StrategyKind = StrategyKind::LimitCancel;
const DEFAULT_PORTFOLIO: &str = "demo";
const DEFAULT_EXCHANGE: &str = "alor";
const DEFAULT_SOURCE: &str = "strategy-runtime";
const DEFAULT_SYMBOL: &str = "SBER";
const DEFAULT_SIDE: &str = "buy";
const DEFAULT_PLACE_OFFSET_TICKS: i64 = 1;
const DEFAULT_QTY: f64 = 1.0;
const DEFAULT_TICK_SIZE: f64 = 0.01;
const DEFAULT_MAX_WAIT_BARS_FOR_ACK: u32 = 3;
const DEFAULT_CLOSE_TRIGGER: CloseTrigger = CloseTrigger::NextBar;
const DEFAULT_ENTRY_ACK_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_ENTRY_FILL_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_EXIT_ACK_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_EXIT_FILL_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_SESSION_OPEN_HOUR: u32 = 10;
const DEFAULT_SESSION_OPEN_MINUTE: u32 = 0;
const DEFAULT_SESSION_CLOSE_HOUR: u32 = 23;
const DEFAULT_SESSION_CLOSE_MINUTE: u32 = 50;
const DEFAULT_ENTRY_AFTER_OPEN_MIN: u32 = 59;
const DEFAULT_EXIT_BEFORE_CLOSE_MIN: u32 = 20;
const DEFAULT_TIMEZONE_OFFSET_HOURS: i32 = 3;
const DEFAULT_CONSUMER_GROUP: &str = "strategy-runtime";
const DEFAULT_CONSUMER_NAME: &str = "auto";
const DEFAULT_HEALTH_STREAM: &str = "events.health";

const DEFAULT_TRADE_MODE: TradeMode = TradeMode::Paper;
const DEFAULT_ALLOW_LIVE_ORDERS: bool = false;
const DEFAULT_GUARD_LOG_INTERVAL_MS: u64 = 5_000;
const DEFAULT_BOOTSTRAP_DUMP: bool = false;
const DEFAULT_PAPER_ENABLED: bool = true;
const DEFAULT_PAPER_OUTPUT: PaperOutput = PaperOutput::Stdout;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSources {
    pub trade_mode: ConfigSource,
    pub allow_live_orders: ConfigSource,
    pub guard_log_interval_ms: ConfigSource,
    pub bootstrap_dump: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperSources {
    pub enabled: ConfigSource,
    pub output: ConfigSource,
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
        }
    }
}

impl Default for RuntimeSources {
    fn default() -> Self {
        Self {
            trade_mode: ConfigSource::Default,
            allow_live_orders: ConfigSource::Default,
            guard_log_interval_ms: ConfigSource::Default,
            bootstrap_dump: ConfigSource::Default,
        }
    }
}

impl Default for PaperSources {
    fn default() -> Self {
        Self {
            enabled: ConfigSource::Default,
            output: ConfigSource::Default,
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
    paper: Option<PaperConfigFile>,
    backtest: Option<BacktestConfigFile>,
    replay: Option<ReplayConfigFile>,
    reset_state_on_start: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeSettingsFile {
    trade_mode: Option<String>,
    allow_live_orders: Option<bool>,
    guard_log_interval_ms: Option<u64>,
    bootstrap_dump: Option<bool>,
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
    strategy_id: Option<String>,
    strategy_kind: Option<String>,
    symbol: Option<String>,
    qty: Option<f64>,
    side: Option<String>,
    place_offset_ticks: Option<i64>,
    tick_size: Option<f64>,
    max_wait_bars_for_ack: Option<u32>,
    close_trigger: Option<String>,
    entry_ack_timeout_ms: Option<u64>,
    entry_fill_timeout_ms: Option<u64>,
    exit_ack_timeout_ms: Option<u64>,
    exit_fill_timeout_ms: Option<u64>,
    session_open_hour: Option<u32>,
    session_open_minute: Option<u32>,
    session_close_hour: Option<u32>,
    session_close_minute: Option<u32>,
    entry_after_open_min: Option<u32>,
    exit_before_close_min: Option<u32>,
    timezone_offset_hours: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
struct PaperConfigFile {
    enabled: Option<bool>,
    output: Option<String>,
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

    let mut strategy = StrategyConfig {
        strategy_id: DEFAULT_STRATEGY_ID.to_string(),
        strategy_kind: DEFAULT_STRATEGY_KIND,
        symbol: DEFAULT_SYMBOL.to_string(),
        qty: DEFAULT_QTY,
        side: parse_side(DEFAULT_SIDE),
        place_offset_ticks: DEFAULT_PLACE_OFFSET_TICKS,
        tick_size: DEFAULT_TICK_SIZE,
        max_wait_bars_for_ack: DEFAULT_MAX_WAIT_BARS_FOR_ACK,
        close_trigger: DEFAULT_CLOSE_TRIGGER,
        entry_ack_timeout_ms: DEFAULT_ENTRY_ACK_TIMEOUT_MS,
        entry_fill_timeout_ms: DEFAULT_ENTRY_FILL_TIMEOUT_MS,
        exit_ack_timeout_ms: DEFAULT_EXIT_ACK_TIMEOUT_MS,
        exit_fill_timeout_ms: DEFAULT_EXIT_FILL_TIMEOUT_MS,
        session_open_hour: DEFAULT_SESSION_OPEN_HOUR,
        session_open_minute: DEFAULT_SESSION_OPEN_MINUTE,
        session_close_hour: DEFAULT_SESSION_CLOSE_HOUR,
        session_close_minute: DEFAULT_SESSION_CLOSE_MINUTE,
        entry_after_open_min: DEFAULT_ENTRY_AFTER_OPEN_MIN,
        exit_before_close_min: DEFAULT_EXIT_BEFORE_CLOSE_MIN,
        timezone_offset_hours: DEFAULT_TIMEZONE_OFFSET_HOURS,
    };

    let mut read = ReadConfig {
        block_ms: DEFAULT_BLOCK_MS,
        claim_idle_ms: DEFAULT_CLAIM_IDLE_MS,
        claim_batch: DEFAULT_CLAIM_BATCH,
        poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
    };

    let mut trade_mode = DEFAULT_TRADE_MODE;
    let mut allow_live_orders = DEFAULT_ALLOW_LIVE_ORDERS;
    let mut guard_log_interval_ms = DEFAULT_GUARD_LOG_INTERVAL_MS;
    let mut bootstrap_dump = DEFAULT_BOOTSTRAP_DUMP;
    let mut paper = PaperConfig {
        enabled: DEFAULT_PAPER_ENABLED,
        output: DEFAULT_PAPER_OUTPUT,
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
            if let Some(value) = &strategy_file.strategy_id {
                strategy.strategy_id = value.clone();
                sources.strategy.strategy_id = ConfigSource::File;
            }
            if let Some(value) = &strategy_file.strategy_kind {
                strategy.strategy_kind = parse_strategy_kind(value);
                sources.strategy.strategy_kind = ConfigSource::File;
            }
            if let Some(value) = &strategy_file.symbol {
                strategy.symbol = value.clone();
                sources.strategy.symbol = ConfigSource::File;
            }
            if let Some(value) = strategy_file.qty {
                strategy.qty = value;
                sources.strategy.qty = ConfigSource::File;
            }
            if let Some(value) = &strategy_file.side {
                strategy.side = parse_side(value);
                sources.strategy.side = ConfigSource::File;
            }
            if let Some(value) = strategy_file.place_offset_ticks {
                strategy.place_offset_ticks = value;
                sources.strategy.place_offset_ticks = ConfigSource::File;
            }
            if let Some(value) = strategy_file.tick_size {
                strategy.tick_size = value;
                sources.strategy.tick_size = ConfigSource::File;
            }
            if let Some(value) = strategy_file.max_wait_bars_for_ack {
                strategy.max_wait_bars_for_ack = value;
                sources.strategy.max_wait_bars_for_ack = ConfigSource::File;
            }
            if let Some(value) = &strategy_file.close_trigger {
                strategy.close_trigger = parse_close_trigger(value);
                sources.strategy.close_trigger = ConfigSource::File;
            }
            if let Some(value) = strategy_file.entry_ack_timeout_ms {
                strategy.entry_ack_timeout_ms = value;
                sources.strategy.entry_ack_timeout_ms = ConfigSource::File;
            }
            if let Some(value) = strategy_file.entry_fill_timeout_ms {
                strategy.entry_fill_timeout_ms = value;
                sources.strategy.entry_fill_timeout_ms = ConfigSource::File;
            }
            if let Some(value) = strategy_file.exit_ack_timeout_ms {
                strategy.exit_ack_timeout_ms = value;
                sources.strategy.exit_ack_timeout_ms = ConfigSource::File;
            }
            if let Some(value) = strategy_file.exit_fill_timeout_ms {
                strategy.exit_fill_timeout_ms = value;
                sources.strategy.exit_fill_timeout_ms = ConfigSource::File;
            }
            if let Some(value) = strategy_file.session_open_hour {
                strategy.session_open_hour = value;
                sources.strategy.session_open_hour = ConfigSource::File;
            }
            if let Some(value) = strategy_file.session_open_minute {
                strategy.session_open_minute = value;
                sources.strategy.session_open_minute = ConfigSource::File;
            }
            if let Some(value) = strategy_file.session_close_hour {
                strategy.session_close_hour = value;
                sources.strategy.session_close_hour = ConfigSource::File;
            }
            if let Some(value) = strategy_file.session_close_minute {
                strategy.session_close_minute = value;
                sources.strategy.session_close_minute = ConfigSource::File;
            }
            if let Some(value) = strategy_file.entry_after_open_min {
                strategy.entry_after_open_min = value;
                sources.strategy.entry_after_open_min = ConfigSource::File;
            }
            if let Some(value) = strategy_file.exit_before_close_min {
                strategy.exit_before_close_min = value;
                sources.strategy.exit_before_close_min = ConfigSource::File;
            }
            if let Some(value) = strategy_file.timezone_offset_hours {
                strategy.timezone_offset_hours = value;
                sources.strategy.timezone_offset_hours = ConfigSource::File;
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
            if let Some(value) = runtime_file.guard_log_interval_ms {
                guard_log_interval_ms = value;
                sources.runtime.guard_log_interval_ms = ConfigSource::File;
            }
            if let Some(value) = runtime_file.bootstrap_dump {
                bootstrap_dump = value;
                sources.runtime.bootstrap_dump = ConfigSource::File;
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

    if let Some(value) = env::var("REDIS_URL").ok() {
        redis_url = value;
        sources.redis_url = ConfigSource::Env;
    }
    if let Some(value) = env::var("PORTFOLIO").ok() {
        portfolio = value;
        sources.portfolio = ConfigSource::Env;
    }
    if let Some(value) = env::var("EXCHANGE").ok() {
        exchange = value;
        sources.exchange = ConfigSource::Env;
    }
    if let Some(value) = env::var("SOURCE").ok() {
        source = value;
        sources.source = ConfigSource::Env;
    }
    if let Some(value) = env::var("CONSUMER_GROUP").ok() {
        consumer_group = value;
        sources.consumer_group = ConfigSource::Env;
    }
    if let Some(value) = env::var("CONSUMER_NAME").ok() {
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
    if let Some(value) = env::var("STRATEGY_ID").ok() {
        strategy.strategy_id = value;
        sources.strategy.strategy_id = ConfigSource::Env;
    }
    if let Some(value) = env::var("STRATEGY_KIND").ok() {
        strategy.strategy_kind = parse_strategy_kind(&value);
        sources.strategy.strategy_kind = ConfigSource::Env;
    }
    if let Some(value) = env::var("SYMBOL").ok() {
        strategy.symbol = value;
        sources.strategy.symbol = ConfigSource::Env;
    }
    if let Some(value) = env_parse("QTY") {
        strategy.qty = value;
        sources.strategy.qty = ConfigSource::Env;
    }
    if let Some(value) = env::var("SIDE").ok() {
        strategy.side = parse_side(&value);
        sources.strategy.side = ConfigSource::Env;
    }
    if let Some(value) = env_parse("PLACE_OFFSET_TICKS") {
        strategy.place_offset_ticks = value;
        sources.strategy.place_offset_ticks = ConfigSource::Env;
    }
    if let Some(value) = env_parse("TICK_SIZE") {
        strategy.tick_size = value;
        sources.strategy.tick_size = ConfigSource::Env;
    }
    if let Some(value) = env_parse("MAX_WAIT_BARS_FOR_ACK") {
        strategy.max_wait_bars_for_ack = value;
        sources.strategy.max_wait_bars_for_ack = ConfigSource::Env;
    }
    if let Some(value) = env::var("CLOSE_TRIGGER").ok() {
        strategy.close_trigger = parse_close_trigger(&value);
        sources.strategy.close_trigger = ConfigSource::Env;
    }
    if let Some(value) = env_parse("ENTRY_ACK_TIMEOUT_MS") {
        strategy.entry_ack_timeout_ms = value;
        sources.strategy.entry_ack_timeout_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("ENTRY_FILL_TIMEOUT_MS") {
        strategy.entry_fill_timeout_ms = value;
        sources.strategy.entry_fill_timeout_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("EXIT_ACK_TIMEOUT_MS") {
        strategy.exit_ack_timeout_ms = value;
        sources.strategy.exit_ack_timeout_ms = ConfigSource::Env;
    }
    if let Some(value) = env_parse("EXIT_FILL_TIMEOUT_MS") {
        strategy.exit_fill_timeout_ms = value;
        sources.strategy.exit_fill_timeout_ms = ConfigSource::Env;
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
    if let Some(value) = env::var("TRADE_MODE").ok() {
        trade_mode = parse_trade_mode(&value);
        sources.runtime.trade_mode = ConfigSource::Env;
    }
    if let Some(value) = env::var("ALLOW_LIVE_ORDERS").ok() {
        allow_live_orders = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.allow_live_orders = ConfigSource::Env;
    }
    if let Some(value) = env_parse("GUARD_LOG_INTERVAL_MS") {
        guard_log_interval_ms = value;
        sources.runtime.guard_log_interval_ms = ConfigSource::Env;
    }
    if let Some(value) = env::var("BOOTSTRAP_DUMP").ok() {
        bootstrap_dump = value == "1" || value.eq_ignore_ascii_case("true");
        sources.runtime.bootstrap_dump = ConfigSource::Env;
    }
    if let Some(value) = env::var("PAPER_ENABLED").ok() {
        paper.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.paper.enabled = ConfigSource::Env;
    }
    if let Some(value) = env::var("PAPER_OUTPUT").ok() {
        paper.output = parse_paper_output(&value);
        sources.paper.output = ConfigSource::Env;
    }
    if let Some(value) = env::var("PAPER_FILE_PATH").ok() {
        paper.file_path = value;
        sources.paper.file_path = ConfigSource::Env;
    }
    if let Some(value) = env::var("PAPER_TRADES_CSV").ok() {
        paper.trades_csv = value;
        sources.paper.trades_csv = ConfigSource::Env;
    }
    if let Some(value) = env::var("PAPER_SUMMARY_JSON").ok() {
        paper.summary_json = value;
        sources.paper.summary_json = ConfigSource::Env;
    }
    if let Some(value) = env_parse("PAPER_APPEND") {
        paper.append = value;
        sources.paper.append = ConfigSource::Env;
    }
    if let Some(value) = env::var("BACKTEST_ENABLED").ok() {
        backtest.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.backtest.enabled = ConfigSource::Env;
    }
    if let Some(value) = env::var("BACKTEST_TRADE_LOG").ok() {
        backtest.trade_log = value;
        sources.backtest.trade_log = ConfigSource::Env;
    }
    if let Some(value) = env::var("BACKTEST_TRADES_CSV").ok() {
        backtest.trades_csv = value;
        sources.backtest.trades_csv = ConfigSource::Env;
    }
    if let Some(value) = env::var("BACKTEST_SUMMARY_JSON").ok() {
        backtest.summary_json = value;
        sources.backtest.summary_json = ConfigSource::Env;
    }
    if let Some(value) = env_parse("BACKTEST_APPEND") {
        backtest.append = value;
        sources.backtest.append = ConfigSource::Env;
    }
    if let Some(value) = env::var("REPLAY_ENABLED").ok() {
        replay.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.replay.enabled = ConfigSource::Env;
    }
    if let Some(value) = env::var("REPLAY_BARS_CSV_PATH").ok() {
        replay.bars_csv_path = Some(value);
        sources.replay.bars_csv_path = ConfigSource::Env;
    }
    if let Some(value) = env::var("REPLAY_REFERENCE_TRADES_CSV_PATH").ok() {
        replay.reference_trades_csv_path = Some(value);
        sources.replay.reference_trades_csv_path = ConfigSource::Env;
    }
    if let Some(value) = env::var("REPLAY_OUTPUT_DIR").ok() {
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
    if let Some(value) = env::var("RESET_STATE_ON_START").ok() {
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

    if let Some(value) = env::var("STREAM_BARS").ok() {
        streams.bars = value;
        sources.streams.bars = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_ORDERS").ok() {
        streams.orders = value;
        sources.streams.orders = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_TRADES").ok() {
        streams.trades = value;
        sources.streams.trades = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_POSITIONS").ok() {
        streams.positions = value;
        sources.streams.positions = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_COMMANDS").ok() {
        streams.commands = value;
        sources.streams.commands = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_ACKS").ok() {
        streams.acks = value;
        sources.streams.acks = ConfigSource::Env;
    }
    if let Some(value) = env::var("SNAPSHOTS_STREAM").ok() {
        streams.snapshots = parse_optional_stream(&value);
        sources.streams.snapshots = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_HEALTH").ok() {
        streams.health = parse_optional_stream(&value);
        sources.streams.health = ConfigSource::Env;
    }
    if let Some(value) = env::var("STREAM_DLQ_PREFIX").ok() {
        streams.dlq_prefix = value;
        sources.streams.dlq_prefix = ConfigSource::Env;
    }
    if let Some(value) = env::var("RUNTIME_STATE_STREAM").ok() {
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
        guard_log_interval_ms,
        bootstrap_dump,
        read,
        trim,
        strategy,
        paper,
        backtest,
        replay,
        reset_state_on_start,
    };

    validate_trade_mode(&config)?;

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

fn parse_strategy_kind(value: &str) -> StrategyKind {
    match value.to_lowercase().as_str() {
        "market_buy_and_close" | "marketbuyandclose" => StrategyKind::MarketBuyAndClose,
        "toy_session_timing" | "toysessiontiming" => StrategyKind::ToySessionTiming,
        "session_gap_standalone" | "sessiongapstandalone" => StrategyKind::SessionGapStandalone,
        _ => StrategyKind::LimitCancel,
    }
}

fn parse_close_trigger(value: &str) -> CloseTrigger {
    match value.to_lowercase().as_str() {
        "position_update" | "positionupdate" => CloseTrigger::PositionUpdate,
        _ => CloseTrigger::NextBar,
    }
}

fn parse_paper_output(value: &str) -> PaperOutput {
    match value.to_lowercase().as_str() {
        "file" => PaperOutput::File,
        _ => PaperOutput::Stdout,
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

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    env::var(key).ok().and_then(|value| value.parse().ok())
}
