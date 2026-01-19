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
    pub health_listen_addr: String,
    pub price_step: f64,
    pub volume_step: f64,
    pub log_positions_filter: Vec<String>,
    pub log_cash_positions: bool,
    pub cash_symbols: Vec<String>,
    pub log_existing_snapshot_orders: bool,
    pub ws_idle_timeout_sec: u64,
    pub ws_ping_interval_sec: u64,
    pub ws_ping_timeout_sec: u64,
    pub subscribe_ack_timeout_ms: u64,
    pub subscribe_ack_timeout_positions_ms: u64,
    pub subscribe_ack_retries: u8,
    pub warm_reconnect_max_gap_sec: u64,
    pub gap_backfill_padding_bars: u8,
    pub cold_start_history_days_back: u8,
}

impl AlorGatewayConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        if let Ok(path) = env::var("ALOR_GATEWAY_CONFIG") {
            return Self::from_file(path);
        }

        let subscribe_ack_timeout_ms = parse_u64("ALOR_SUBSCRIBE_ACK_TIMEOUT_MS", 5_000)?;
        let subscribe_ack_timeout_positions_ms = parse_u64(
            "ALOR_SUBSCRIBE_ACK_TIMEOUT_POSITIONS_MS",
            subscribe_ack_timeout_ms,
        )?;

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
            health_listen_addr: env::var("ALOR_HEALTH_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            price_step: parse_f64("ALOR_PRICE_STEP", 0.0)?,
            volume_step: parse_f64("ALOR_VOLUME_STEP", 0.0)?,
            log_positions_filter: {
                let raw = env::var("ALOR_LOG_POSITIONS_FILTER").unwrap_or_default();
                if raw.trim().is_empty() {
                    parse_list(
                        &env::var("ALOR_SYMBOLS").unwrap_or_else(|_| "USDRUBF".to_string()),
                    )
                } else {
                    parse_list(&raw)
                }
            },
            log_cash_positions: parse_bool("ALOR_LOG_CASH_POSITIONS", false),
            cash_symbols: {
                let raw = env::var("ALOR_CASH_SYMBOLS").unwrap_or_else(|_| "RUB,SUR".to_string());
                parse_list(&raw)
            },
            log_existing_snapshot_orders: parse_bool(
                "ALOR_LOG_EXISTING_SNAPSHOT_ORDERS",
                false,
            ),
            ws_idle_timeout_sec: parse_u64("ALOR_WS_IDLE_TIMEOUT_SEC", 70)?,
            ws_ping_interval_sec: parse_u64("ALOR_WS_PING_INTERVAL_SEC", 30)?,
            ws_ping_timeout_sec: parse_u64("ALOR_WS_PING_TIMEOUT_SEC", 15)?,
            subscribe_ack_timeout_ms,
            subscribe_ack_timeout_positions_ms,
            subscribe_ack_retries: parse_u8("ALOR_SUBSCRIBE_ACK_RETRIES", 3)?,
            warm_reconnect_max_gap_sec: parse_u64("ALOR_WARM_RECONNECT_MAX_GAP_SEC", 21_600)?,
            gap_backfill_padding_bars: parse_u8("ALOR_GAP_BACKFILL_PADDING_BARS", 2)?,
            cold_start_history_days_back: parse_u8("ALOR_COLD_START_HISTORY_DAYS_BACK", 4)?,
        })
    }

    fn from_file(path: String) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(&path).map_err(|err| ConfigError::ReadFile {
            path: path.clone(),
            source: err,
        })?;
        let file_cfg: FileConfig = toml::from_str(&contents)
            .map_err(|err| ConfigError::ParseToml { path: path.clone(), source: err })?;

        let symbols = file_cfg
            .symbols
            .clone()
            .unwrap_or_else(|| vec!["USDRUBF".to_string()]);
        let log_positions_filter = file_cfg
            .log_positions_filter
            .unwrap_or_else(|| symbols.clone());
        let subscribe_ack_timeout_ms = file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.subscribe_ack_timeout_ms)
            .unwrap_or(5_000);
        let subscribe_ack_timeout_positions_ms = file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.subscribe_ack_timeout_positions_ms)
            .unwrap_or(subscribe_ack_timeout_ms);

        Ok(Self {
            portfolio: file_cfg
                .portfolio
                .ok_or(ConfigError::MissingField("portfolio"))?,
            exchange: file_cfg.exchange.unwrap_or_else(|| "MOEX".to_string()),
            instrument_group: file_cfg
                .instrument_group
                .unwrap_or_else(|| "RFUD".to_string()),
            symbols: symbols.clone(),
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
            health_listen_addr: file_cfg
                .health_listen_addr
                .unwrap_or_else(|| "127.0.0.1:8080".to_string()),
            price_step: file_cfg
                .general
                .as_ref()
                .and_then(|general| general.price_step)
                .or(file_cfg.price_step)
                .unwrap_or(0.0),
            volume_step: file_cfg
                .general
                .as_ref()
                .and_then(|general| general.volume_step)
                .or(file_cfg.volume_step)
                .unwrap_or(0.0),
            log_positions_filter,
            log_cash_positions: file_cfg.log_cash_positions.unwrap_or(false),
            cash_symbols: file_cfg
                .cash_symbols
                .unwrap_or_else(|| vec!["RUB".to_string(), "SUR".to_string()]),
            log_existing_snapshot_orders: file_cfg.log_existing_snapshot_orders.unwrap_or(false),
            ws_idle_timeout_sec: file_cfg
                .ws
                .as_ref()
                .and_then(|ws| ws.ws_idle_timeout_sec)
                .unwrap_or(70),
            ws_ping_interval_sec: file_cfg
                .ws
                .as_ref()
                .and_then(|ws| ws.ws_ping_interval_sec)
                .unwrap_or(30),
            ws_ping_timeout_sec: file_cfg
                .ws
                .as_ref()
                .and_then(|ws| ws.ws_ping_timeout_sec)
                .unwrap_or(15),
            subscribe_ack_timeout_ms,
            subscribe_ack_timeout_positions_ms,
            subscribe_ack_retries: file_cfg
                .ws
                .as_ref()
                .and_then(|ws| ws.subscribe_ack_retries)
                .unwrap_or(3),
            warm_reconnect_max_gap_sec: file_cfg
                .reconnect
                .as_ref()
                .and_then(|reconnect| reconnect.warm_reconnect_max_gap_sec)
                .unwrap_or(21_600),
            gap_backfill_padding_bars: file_cfg
                .reconnect
                .as_ref()
                .and_then(|reconnect| reconnect.gap_backfill_padding_bars)
                .unwrap_or(2),
            cold_start_history_days_back: file_cfg
                .reconnect
                .as_ref()
                .and_then(|reconnect| reconnect.cold_start_history_days_back)
                .unwrap_or(4),
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
    #[error("invalid float env var {0}: {1}")]
    InvalidFloat(&'static str, #[source] std::num::ParseFloatError),
    #[error("failed to read config file {path}: {source}")]
    ReadFile { path: String, #[source] source: std::io::Error },
    #[error("failed to parse toml config {path}: {source}")]
    ParseToml { path: String, #[source] source: toml::de::Error },
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    general: Option<GeneralConfig>,
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
    health_listen_addr: Option<String>,
    price_step: Option<f64>,
    volume_step: Option<f64>,
    log_positions_filter: Option<Vec<String>>,
    log_cash_positions: Option<bool>,
    cash_symbols: Option<Vec<String>>,
    log_existing_snapshot_orders: Option<bool>,
    ws: Option<WsConfig>,
    reconnect: Option<ReconnectConfig>,
}

#[derive(Debug, Deserialize)]
struct GeneralConfig {
    price_step: Option<f64>,
    volume_step: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WsConfig {
    ws_idle_timeout_sec: Option<u64>,
    ws_ping_interval_sec: Option<u64>,
    ws_ping_timeout_sec: Option<u64>,
    subscribe_ack_timeout_ms: Option<u64>,
    subscribe_ack_timeout_positions_ms: Option<u64>,
    subscribe_ack_retries: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct ReconnectConfig {
    warm_reconnect_max_gap_sec: Option<u64>,
    gap_backfill_padding_bars: Option<u8>,
    cold_start_history_days_back: Option<u8>,
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

fn parse_f64(key: &'static str, default: f64) -> Result<f64, ConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse::<f64>()
            .map_err(|err| ConfigError::InvalidFloat(key, err)),
        Err(_) => Ok(default),
    }
}

fn parse_bool(key: &'static str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(default)
}
