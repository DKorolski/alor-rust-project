use std::env;
use std::fs;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct AlorGatewayConfig {
    pub portfolio: String,
    pub exchange: String,
    pub instrument_group: String,
    pub symbols: Vec<String>,
    pub tf_sec: i64,
    pub from_ts: i64,
    pub ws_url: String,
    pub cws_url: String,
    pub oauth_url: String,
    pub refresh_token: String,
    pub skip_history_bars: bool,
    pub skip_history_positions: bool,
    pub skip_history_orders: bool,
    pub split_adjust: bool,
    pub format: String,
    pub frequency_ms: i64,
    pub backoff_initial_ms: u64,
    pub backoff_max_ms: u64,
    pub backoff_multiplier: u8,
    pub max_silence_bars_sec: u64,
    pub history_sessions: u8,
    pub history_days_back: u8,
    pub session_rollover_hour_utc: u8,
    pub bars_only: bool,
}

impl AlorGatewayConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        if let Ok(path) = env::var("ALOR_GATEWAY_CONFIG") {
            return Self::from_file(path);
        }

        Ok(Self {
            portfolio: get_required("ALOR_PORTFOLIO")?,
            exchange: env::var("ALOR_EXCHANGE").unwrap_or_else(|_| "MOEX".to_string()),
            instrument_group: env::var("ALOR_INSTRUMENT_GROUP")
                .unwrap_or_else(|_| "RFUD".to_string()),
            symbols: parse_list(
                &env::var("ALOR_SYMBOLS").unwrap_or_else(|_| "USDRUBF".to_string()),
            ),
            tf_sec: parse_i64("ALOR_TF_SEC", 60)?,
            from_ts: parse_i64("ALOR_FROM_TS", 0)?,
            ws_url: env::var("ALOR_WS_URL").unwrap_or_else(|_| "wss://api.alor.ru/ws".into()),
            cws_url: env::var("ALOR_CWS_URL")
                .unwrap_or_else(|_| "wss://api.alor.ru/cws".into()),
            oauth_url: env::var("ALOR_OAUTH_URL")
                .unwrap_or_else(|_| "https://oauth.alor.ru/refresh".into()),
            refresh_token: get_required("ALOR_REFRESH_TOKEN")?,
            skip_history_bars: parse_bool("ALOR_SKIP_HISTORY_BARS", false),
            skip_history_positions: parse_bool("ALOR_SKIP_HISTORY_POSITIONS", false),
            skip_history_orders: parse_bool("ALOR_SKIP_HISTORY_ORDERS", false),
            split_adjust: parse_bool("ALOR_SPLIT_ADJUST", true),
            format: env::var("ALOR_FORMAT").unwrap_or_else(|_| "Simple".to_string()),
            frequency_ms: parse_i64("ALOR_FREQUENCY_MS", 250)?,
            backoff_initial_ms: parse_u64("ALOR_BACKOFF_INITIAL_MS", 1_000)?,
            backoff_max_ms: parse_u64("ALOR_BACKOFF_MAX_MS", 30_000)?,
            backoff_multiplier: parse_u8("ALOR_BACKOFF_MULTIPLIER", 2)?,
            max_silence_bars_sec: parse_u64("ALOR_MAX_SILENCE_BARS_SEC", 180)?,
            history_sessions: parse_u8("ALOR_HISTORY_SESSIONS", 2)?,
            history_days_back: parse_u8("ALOR_HISTORY_DAYS_BACK", 5)?,
            session_rollover_hour_utc: parse_u8("ALOR_SESSION_ROLLOVER_HOUR_UTC", 7)?,
            bars_only: parse_bool("ALOR_BARS_ONLY", false),
        })
    }

    fn from_file(path: String) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(&path).map_err(|err| ConfigError::ReadFile {
            path: path.clone(),
            source: err,
        })?;
        let file_cfg: FileConfig = toml::from_str(&contents)
            .map_err(|err| ConfigError::ParseToml { path: path.clone(), source: err })?;

        Ok(Self {
            portfolio: file_cfg
                .portfolio
                .ok_or(ConfigError::MissingField("portfolio"))?,
            exchange: file_cfg.exchange.unwrap_or_else(|| "MOEX".to_string()),
            instrument_group: file_cfg
                .instrument_group
                .unwrap_or_else(|| "RFUD".to_string()),
            symbols: file_cfg.symbols.unwrap_or_else(|| vec!["USDRUBF".to_string()]),
            tf_sec: file_cfg.tf_sec.unwrap_or(60),
            from_ts: file_cfg.from_ts.unwrap_or(0),
            ws_url: file_cfg
                .ws_url
                .unwrap_or_else(|| "wss://api.alor.ru/ws".into()),
            cws_url: file_cfg
                .cws_url
                .unwrap_or_else(|| "wss://api.alor.ru/cws".into()),
            oauth_url: file_cfg
                .oauth_url
                .unwrap_or_else(|| "https://oauth.alor.ru/refresh".into()),
            refresh_token: file_cfg
                .refresh_token
                .ok_or(ConfigError::MissingField("refresh_token"))?,
            skip_history_bars: file_cfg.skip_history_bars.unwrap_or(false),
            skip_history_positions: file_cfg.skip_history_positions.unwrap_or(false),
            skip_history_orders: file_cfg.skip_history_orders.unwrap_or(false),
            split_adjust: file_cfg.split_adjust.unwrap_or(true),
            format: file_cfg.format.unwrap_or_else(|| "Simple".to_string()),
            frequency_ms: file_cfg.frequency_ms.unwrap_or(250),
            backoff_initial_ms: file_cfg.backoff_initial_ms.unwrap_or(1_000),
            backoff_max_ms: file_cfg.backoff_max_ms.unwrap_or(30_000),
            backoff_multiplier: file_cfg.backoff_multiplier.unwrap_or(2),
            max_silence_bars_sec: file_cfg.max_silence_bars_sec.unwrap_or(180),
            history_sessions: file_cfg.history_sessions.unwrap_or(2),
            history_days_back: file_cfg.history_days_back.unwrap_or(5),
            session_rollover_hour_utc: file_cfg.session_rollover_hour_utc.unwrap_or(7),
            bars_only: file_cfg.bars_only.unwrap_or(false),
        })
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing env var {0}")]
    MissingVar(&'static str),
    #[error("missing required field {0}")]
    MissingField(&'static str),
    #[error("invalid int env var {0}: {1}")]
    InvalidInt(&'static str, #[source] std::num::ParseIntError),
    #[error("failed to read config file {path}: {source}")]
    ReadFile { path: String, #[source] source: std::io::Error },
    #[error("failed to parse toml config {path}: {source}")]
    ParseToml { path: String, #[source] source: toml::de::Error },
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    portfolio: Option<String>,
    exchange: Option<String>,
    instrument_group: Option<String>,
    symbols: Option<Vec<String>>,
    tf_sec: Option<i64>,
    from_ts: Option<i64>,
    ws_url: Option<String>,
    cws_url: Option<String>,
    oauth_url: Option<String>,
    refresh_token: Option<String>,
    skip_history_bars: Option<bool>,
    skip_history_positions: Option<bool>,
    skip_history_orders: Option<bool>,
    split_adjust: Option<bool>,
    format: Option<String>,
    frequency_ms: Option<i64>,
    backoff_initial_ms: Option<u64>,
    backoff_max_ms: Option<u64>,
    backoff_multiplier: Option<u8>,
    max_silence_bars_sec: Option<u64>,
    history_sessions: Option<u8>,
    history_days_back: Option<u8>,
    session_rollover_hour_utc: Option<u8>,
    bars_only: Option<bool>,
}

fn get_required(key: &'static str) -> Result<String, ConfigError> {
    env::var(key).map_err(|_| ConfigError::MissingVar(key))
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn parse_i64(key: &'static str, default: i64) -> Result<i64, ConfigError> {
    match env::var(key) {
        Ok(value) => value.parse::<i64>().map_err(|err| ConfigError::InvalidInt(key, err)),
        Err(_) => Ok(default),
    }
}

fn parse_u64(key: &'static str, default: u64) -> Result<u64, ConfigError> {
    match env::var(key) {
        Ok(value) => value.parse::<u64>().map_err(|err| ConfigError::InvalidInt(key, err)),
        Err(_) => Ok(default),
    }
}

fn parse_u8(key: &'static str, default: u8) -> Result<u8, ConfigError> {
    match env::var(key) {
        Ok(value) => value.parse::<u8>().map_err(|err| ConfigError::InvalidInt(key, err)),
        Err(_) => Ok(default),
    }
}

fn parse_bool(key: &'static str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(default)
}
