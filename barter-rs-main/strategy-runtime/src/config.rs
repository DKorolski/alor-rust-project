use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{
    BacktestConfig, PaperConfig, PaperOutput, ReadConfig, RuntimeConfig, StrategyConfig,
    StreamNames, TradeMode, TrimConfig,
};

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1/";
const DEFAULT_STRATEGY_ID: &str = "limit_cancel";
const DEFAULT_PORTFOLIO: &str = "demo";
const DEFAULT_EXCHANGE: &str = "alor";
const DEFAULT_SOURCE: &str = "strategy-runtime";
const DEFAULT_SYMBOL: &str = "SBER";
const DEFAULT_SIDE: &str = "buy";
const DEFAULT_PLACE_OFFSET_TICKS: i64 = 1;
const DEFAULT_QTY: f64 = 1.0;
const DEFAULT_TICK_SIZE: f64 = 0.01;
const DEFAULT_MAX_WAIT_BARS_FOR_ACK: u32 = 3;
const DEFAULT_CONSUMER_GROUP: &str = "strategy-runtime";
const DEFAULT_CONSUMER_NAME: &str = "auto";
const DEFAULT_HEALTH_STREAM: &str = "events.health";

const DEFAULT_TRADE_MODE: TradeMode = TradeMode::Live;
const DEFAULT_ALLOW_LIVE_ORDERS: bool = false;
const DEFAULT_GUARD_LOG_INTERVAL_MS: u64 = 5_000;
const DEFAULT_PAPER_ENABLED: bool = true;
const DEFAULT_PAPER_OUTPUT: PaperOutput = PaperOutput::Stdout;
const DEFAULT_PAPER_FILE_PATH: &str = "./paper_trades.jsonl";
const DEFAULT_BACKTEST_ENABLED: bool = true;
const DEFAULT_BACKTEST_TRADE_LOG: &str = "./backtest_trades.log";

const DEFAULT_BLOCK_MS: usize = 500;
const DEFAULT_CLAIM_IDLE_MS: usize = 5_000;
const DEFAULT_CLAIM_BATCH: usize = 50;
const DEFAULT_POLL_INTERVAL_MS: u64 = 100;

const DEFAULT_TRIM_BARS: usize = 200_000;
const DEFAULT_TRIM_ORDERS: usize = 100_000;
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
    pub reset_state_on_start: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSources {
    pub bars: ConfigSource,
    pub orders: ConfigSource,
    pub positions: ConfigSource,
    pub commands: ConfigSource,
    pub acks: ConfigSource,
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
    pub positions: ConfigSource,
    pub commands: ConfigSource,
    pub acks: ConfigSource,
    pub health: ConfigSource,
    pub runtime_state: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategySources {
    pub strategy_id: ConfigSource,
    pub symbol: ConfigSource,
    pub qty: ConfigSource,
    pub side: ConfigSource,
    pub place_offset_ticks: ConfigSource,
    pub tick_size: ConfigSource,
    pub max_wait_bars_for_ack: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSources {
    pub trade_mode: ConfigSource,
    pub allow_live_orders: ConfigSource,
    pub guard_log_interval_ms: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperSources {
    pub enabled: ConfigSource,
    pub output: ConfigSource,
    pub file_path: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestSources {
    pub enabled: ConfigSource,
    pub trade_log: ConfigSource,
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
            reset_state_on_start: ConfigSource::Default,
        }
    }
}

impl Default for StreamSources {
    fn default() -> Self {
        Self {
            bars: ConfigSource::Default,
            orders: ConfigSource::Default,
            positions: ConfigSource::Default,
            commands: ConfigSource::Default,
            acks: ConfigSource::Default,
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
            symbol: ConfigSource::Default,
            qty: ConfigSource::Default,
            side: ConfigSource::Default,
            place_offset_ticks: ConfigSource::Default,
            tick_size: ConfigSource::Default,
            max_wait_bars_for_ack: ConfigSource::Default,
        }
    }
}

impl Default for RuntimeSources {
    fn default() -> Self {
        Self {
            trade_mode: ConfigSource::Default,
            allow_live_orders: ConfigSource::Default,
            guard_log_interval_ms: ConfigSource::Default,
        }
    }
}

impl Default for PaperSources {
    fn default() -> Self {
        Self {
            enabled: ConfigSource::Default,
            output: ConfigSource::Default,
            file_path: ConfigSource::Default,
        }
    }
}

impl Default for BacktestSources {
    fn default() -> Self {
        Self {
            enabled: ConfigSource::Default,
            trade_log: ConfigSource::Default,
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
    reset_state_on_start: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeSettingsFile {
    trade_mode: Option<String>,
    allow_live_orders: Option<bool>,
    guard_log_interval_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct StreamNamesFile {
    bars: Option<String>,
    orders: Option<String>,
    positions: Option<String>,
    commands: Option<String>,
    acks: Option<String>,
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
    positions: Option<usize>,
    commands: Option<usize>,
    acks: Option<usize>,
    health: Option<usize>,
    runtime_state: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct StrategyConfigFile {
    strategy_id: Option<String>,
    symbol: Option<String>,
    qty: Option<f64>,
    side: Option<String>,
    place_offset_ticks: Option<i64>,
    tick_size: Option<f64>,
    max_wait_bars_for_ack: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct PaperConfigFile {
    enabled: Option<bool>,
    output: Option<String>,
    file_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct BacktestConfigFile {
    enabled: Option<bool>,
    trade_log: Option<String>,
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
        symbol: DEFAULT_SYMBOL.to_string(),
        qty: DEFAULT_QTY,
        side: parse_side(DEFAULT_SIDE),
        place_offset_ticks: DEFAULT_PLACE_OFFSET_TICKS,
        tick_size: DEFAULT_TICK_SIZE,
        max_wait_bars_for_ack: DEFAULT_MAX_WAIT_BARS_FOR_ACK,
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
    let mut paper = PaperConfig {
        enabled: DEFAULT_PAPER_ENABLED,
        output: DEFAULT_PAPER_OUTPUT,
        file_path: DEFAULT_PAPER_FILE_PATH.to_string(),
    };
    let mut backtest = BacktestConfig {
        enabled: DEFAULT_BACKTEST_ENABLED,
        trade_log: DEFAULT_BACKTEST_TRADE_LOG.to_string(),
    };

    let mut trim = TrimConfig {
        bars: DEFAULT_TRIM_BARS,
        orders: DEFAULT_TRIM_ORDERS,
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
    if let Some(value) = env::var("BACKTEST_ENABLED").ok() {
        backtest.enabled = value == "1" || value.eq_ignore_ascii_case("true");
        sources.backtest.enabled = ConfigSource::Env;
    }
    if let Some(value) = env::var("BACKTEST_TRADE_LOG").ok() {
        backtest.trade_log = value;
        sources.backtest.trade_log = ConfigSource::Env;
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
        read,
        trim,
        strategy,
        paper,
        backtest,
        reset_state_on_start,
    };

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
        positions: format!("broker.positions.{portfolio}"),
        commands: format!("cmd.orders.{portfolio}"),
        acks: format!("cmd.acks.{portfolio}"),
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

fn parse_paper_output(value: &str) -> PaperOutput {
    match value.to_lowercase().as_str() {
        "file" => PaperOutput::File,
        _ => PaperOutput::Stdout,
    }
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    env::var(key).ok().and_then(|value| value.parse().ok())
}
