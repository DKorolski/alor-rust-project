use std::env;

use thiserror::Error;

#[derive(Debug, Clone)]
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
}

impl AlorGatewayConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            portfolio: get_required("ALOR_PORTFOLIO")?,
            exchange: env::var("ALOR_EXCHANGE").unwrap_or_else(|_| "MOEX".to_string()),
            instrument_group: env::var("ALOR_INSTRUMENT_GROUP")
                .unwrap_or_else(|_| "RFUD".to_string()),
            symbols: parse_list(
                &env::var("ALOR_SYMBOLS").unwrap_or_else(|_| "IMOEXF".to_string()),
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
        })
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing env var {0}")]
    MissingVar(&'static str),
    #[error("invalid int env var {0}: {1}")]
    InvalidInt(&'static str, #[source] std::num::ParseIntError),
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
