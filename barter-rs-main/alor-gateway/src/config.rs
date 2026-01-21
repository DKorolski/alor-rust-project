use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub bar_silence_resync_min_sec: u64,
    pub data_report_path: Option<String>,
    pub bar_dump_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub config: AlorGatewayConfig,
    pub sources: BTreeMap<&'static str, String>,
    pub derived: DerivedConfig,
}

#[derive(Debug, Clone)]
pub struct DerivedConfig {
    pub computed_from_ts: i64,
    pub skip_history_effective: bool,
    pub mode: HistoryMode,
}

#[derive(Debug, Clone, Copy)]
pub enum HistoryMode {
    ColdStart,
    WarmReconnect,
    NoHistory,
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
            bar_silence_resync_min_sec: parse_u64(
                "ALOR_BAR_SILENCE_RESYNC_MIN_SEC",
                300,
            )?,
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        })
    }

    pub fn from_env_with_sources() -> Result<ResolvedConfig, ConfigError> {
        if let Ok(path) = env::var("ALOR_GATEWAY_CONFIG") {
            return Self::from_file_with_sources(path);
        }

        let mut sources = BTreeMap::new();
        let subscribe_ack_timeout_ms = parse_u64_with_source(
            "ALOR_SUBSCRIBE_ACK_TIMEOUT_MS",
            5_000,
            &mut sources,
        )?;
        let subscribe_ack_timeout_positions_ms = parse_u64_with_source(
            "ALOR_SUBSCRIBE_ACK_TIMEOUT_POSITIONS_MS",
            subscribe_ack_timeout_ms,
            &mut sources,
        )?;

        let config = Self {
            portfolio: get_required_with_source("ALOR_PORTFOLIO", &mut sources)?,
            exchange: env_with_source("ALOR_EXCHANGE", "MOEX", &mut sources),
            instrument_group: env_with_source("ALOR_INSTRUMENT_GROUP", "RFUD", &mut sources),
            symbols: parse_list(
                &env_with_source("ALOR_SYMBOLS", "USDRUBF", &mut sources),
            ),
            tf_sec: parse_i64_with_source("ALOR_TF_SEC", 60, &mut sources)?,
            from_ts: parse_i64_with_source("ALOR_FROM_TS", 0, &mut sources)?,
            ws_url: env_with_source("ALOR_WS_URL", "wss://api.alor.ru/ws", &mut sources),
            cws_url: env_with_source("ALOR_CWS_URL", "wss://api.alor.ru/cws", &mut sources),
            oauth_url: env_with_source(
                "ALOR_OAUTH_URL",
                "https://oauth.alor.ru/refresh",
                &mut sources,
            ),
            refresh_token: get_required_with_source("ALOR_REFRESH_TOKEN", &mut sources)?,
            skip_history_bars: parse_bool_with_source(
                "ALOR_SKIP_HISTORY_BARS",
                false,
                &mut sources,
            ),
            skip_history_positions: parse_bool_with_source(
                "ALOR_SKIP_HISTORY_POSITIONS",
                false,
                &mut sources,
            ),
            skip_history_orders: parse_bool_with_source(
                "ALOR_SKIP_HISTORY_ORDERS",
                false,
                &mut sources,
            ),
            split_adjust: parse_bool_with_source("ALOR_SPLIT_ADJUST", true, &mut sources),
            format: env_with_source("ALOR_FORMAT", "Simple", &mut sources),
            frequency_ms: parse_i64_with_source("ALOR_FREQUENCY_MS", 250, &mut sources)?,
            backoff_initial_ms: parse_u64_with_source(
                "ALOR_BACKOFF_INITIAL_MS",
                1_000,
                &mut sources,
            )?,
            backoff_max_ms: parse_u64_with_source(
                "ALOR_BACKOFF_MAX_MS",
                30_000,
                &mut sources,
            )?,
            backoff_multiplier: parse_u8_with_source(
                "ALOR_BACKOFF_MULTIPLIER",
                2,
                &mut sources,
            )?,
            max_silence_bars_sec: parse_u64_with_source(
                "ALOR_MAX_SILENCE_BARS_SEC",
                180,
                &mut sources,
            )?,
            history_sessions: parse_u8_with_source("ALOR_HISTORY_SESSIONS", 2, &mut sources)?,
            history_days_back: parse_u8_with_source("ALOR_HISTORY_DAYS_BACK", 5, &mut sources)?,
            session_rollover_hour_utc: parse_u8_with_source(
                "ALOR_SESSION_ROLLOVER_HOUR_UTC",
                7,
                &mut sources,
            )?,
            health_listen_addr: env_with_source("ALOR_HEALTH_ADDR", "127.0.0.1:8080", &mut sources),
            price_step: parse_f64_with_source("ALOR_PRICE_STEP", 0.0, &mut sources)?,
            volume_step: parse_f64_with_source("ALOR_VOLUME_STEP", 0.0, &mut sources)?,
            log_positions_filter: {
                let raw = env_with_source("ALOR_LOG_POSITIONS_FILTER", "", &mut sources);
                if raw.trim().is_empty() {
                    parse_list(&env_with_source("ALOR_SYMBOLS", "USDRUBF", &mut sources))
                } else {
                    parse_list(&raw)
                }
            },
            log_cash_positions: parse_bool_with_source(
                "ALOR_LOG_CASH_POSITIONS",
                false,
                &mut sources,
            ),
            cash_symbols: {
                let raw = env_with_source("ALOR_CASH_SYMBOLS", "RUB,SUR", &mut sources);
                parse_list(&raw)
            },
            log_existing_snapshot_orders: parse_bool_with_source(
                "ALOR_LOG_EXISTING_SNAPSHOT_ORDERS",
                false,
                &mut sources,
            ),
            ws_idle_timeout_sec: parse_u64_with_source(
                "ALOR_WS_IDLE_TIMEOUT_SEC",
                70,
                &mut sources,
            )?,
            ws_ping_interval_sec: parse_u64_with_source(
                "ALOR_WS_PING_INTERVAL_SEC",
                30,
                &mut sources,
            )?,
            ws_ping_timeout_sec: parse_u64_with_source(
                "ALOR_WS_PING_TIMEOUT_SEC",
                15,
                &mut sources,
            )?,
            subscribe_ack_timeout_ms,
            subscribe_ack_timeout_positions_ms,
            subscribe_ack_retries: parse_u8_with_source(
                "ALOR_SUBSCRIBE_ACK_RETRIES",
                3,
                &mut sources,
            )?,
            warm_reconnect_max_gap_sec: parse_u64_with_source(
                "ALOR_WARM_RECONNECT_MAX_GAP_SEC",
                21_600,
                &mut sources,
            )?,
            gap_backfill_padding_bars: parse_u8_with_source(
                "ALOR_GAP_BACKFILL_PADDING_BARS",
                2,
                &mut sources,
            )?,
            cold_start_history_days_back: parse_u8_with_source(
                "ALOR_COLD_START_HISTORY_DAYS_BACK",
                4,
                &mut sources,
            )?,
            bar_silence_resync_min_sec: parse_u64_with_source(
                "ALOR_BAR_SILENCE_RESYNC_MIN_SEC",
                300,
                &mut sources,
            )?,
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        };

        let derived = derive_config(&config);
        sources.insert(
            "data_report_path",
            if env::var("DATA_REPORT_PATH").is_ok() {
                "env".to_string()
            } else {
                "default".to_string()
            },
        );
        sources.insert(
            "bar_dump_path",
            if env::var("BAR_DUMP_PATH").is_ok() {
                "env".to_string()
            } else {
                "default".to_string()
            },
        );
        Ok(ResolvedConfig {
            config,
            sources,
            derived,
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
            bar_silence_resync_min_sec: file_cfg
                .reconnect
                .as_ref()
                .and_then(|reconnect| reconnect.bar_silence_resync_min_sec)
                .unwrap_or(300),
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        })
    }

    pub fn from_file_with_sources(path: String) -> Result<ResolvedConfig, ConfigError> {
        let contents = fs::read_to_string(&path).map_err(|err| ConfigError::ReadFile {
            path: path.clone(),
            source: err,
        })?;
        let file_cfg: FileConfig = toml::from_str(&contents)
            .map_err(|err| ConfigError::ParseToml { path: path.clone(), source: err })?;

        let mut sources = BTreeMap::new();
        let source_label = |field: &'static str| format!("file:{}", field);
        let default_label = |field: &'static str| format!("default:{}", field);

        let symbols = file_cfg
            .symbols
            .clone()
            .unwrap_or_else(|| vec!["USDRUBF".to_string()]);
        if file_cfg.symbols.is_some() {
            sources.insert("symbols", source_label("symbols"));
        } else {
            sources.insert("symbols", default_label("symbols"));
        }
        let log_positions_filter = file_cfg
            .log_positions_filter
            .clone()
            .unwrap_or_else(|| symbols.clone());
        if file_cfg.log_positions_filter.is_some() {
            sources.insert("log_positions_filter", source_label("log_positions_filter"));
        } else {
            sources.insert("log_positions_filter", default_label("log_positions_filter"));
        }
        let subscribe_ack_timeout_ms = file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.subscribe_ack_timeout_ms)
            .unwrap_or(5_000);
        sources.insert(
            "subscribe_ack_timeout_ms",
            if file_cfg
                .ws
                .as_ref()
                .and_then(|ws| ws.subscribe_ack_timeout_ms)
                .is_some()
            {
                source_label("ws.subscribe_ack_timeout_ms")
            } else {
                default_label("subscribe_ack_timeout_ms")
            },
        );
        let subscribe_ack_timeout_positions_ms = file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.subscribe_ack_timeout_positions_ms)
            .unwrap_or(subscribe_ack_timeout_ms);
        sources.insert(
            "subscribe_ack_timeout_positions_ms",
            if file_cfg
                .ws
                .as_ref()
                .and_then(|ws| ws.subscribe_ack_timeout_positions_ms)
                .is_some()
            {
                source_label("ws.subscribe_ack_timeout_positions_ms")
            } else {
                default_label("subscribe_ack_timeout_positions_ms")
            },
        );

        let config = Self {
            portfolio: file_cfg
                .portfolio
                .clone()
                .ok_or(ConfigError::MissingField("portfolio"))?,
            exchange: file_cfg.exchange.clone().unwrap_or_else(|| "MOEX".to_string()),
            instrument_group: file_cfg
                .instrument_group
                .clone()
                .unwrap_or_else(|| "RFUD".to_string()),
            symbols: symbols.clone(),
            tf_sec: file_cfg.tf_sec.unwrap_or(60),
            from_ts: file_cfg.from_ts.unwrap_or(0),
            ws_url: file_cfg
                .ws_url
                .clone()
                .unwrap_or_else(|| "wss://api.alor.ru/ws".into()),
            cws_url: file_cfg
                .cws_url
                .clone()
                .unwrap_or_else(|| "wss://api.alor.ru/cws".into()),
            oauth_url: file_cfg
                .oauth_url
                .clone()
                .unwrap_or_else(|| "https://oauth.alor.ru/refresh".into()),
            refresh_token: file_cfg
                .refresh_token
                .clone()
                .ok_or(ConfigError::MissingField("refresh_token"))?,
            skip_history_bars: file_cfg.skip_history_bars.unwrap_or(false),
            skip_history_positions: file_cfg.skip_history_positions.unwrap_or(false),
            skip_history_orders: file_cfg.skip_history_orders.unwrap_or(false),
            split_adjust: file_cfg.split_adjust.unwrap_or(true),
            format: file_cfg.format.clone().unwrap_or_else(|| "Simple".to_string()),
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
                .clone()
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
                .clone()
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
            bar_silence_resync_min_sec: file_cfg
                .reconnect
                .as_ref()
                .and_then(|reconnect| reconnect.bar_silence_resync_min_sec)
                .unwrap_or(300),
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        };

        track_file_sources(&file_cfg, &mut sources);
        sources.insert("config_path", format!("file:{}", path));
        let derived = derive_config(&config);
        Ok(ResolvedConfig {
            config,
            sources,
            derived,
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
    bar_silence_resync_min_sec: Option<u64>,
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

pub fn derive_config(cfg: &AlorGatewayConfig) -> DerivedConfig {
    let skip_history_effective = cfg.skip_history_bars
        || (cfg.history_days_back == 0
            && cfg.history_sessions == 0
            && cfg.cold_start_history_days_back == 0
            && cfg.from_ts == 0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let computed_from_ts = if skip_history_effective {
        now
    } else if cfg.from_ts != 0 {
        cfg.from_ts
    } else {
        now - (cfg.cold_start_history_days_back as i64 * 86_400)
    };
    let mode = if skip_history_effective {
        HistoryMode::NoHistory
    } else if cfg.from_ts == 0 {
        HistoryMode::ColdStart
    } else {
        HistoryMode::WarmReconnect
    };
    DerivedConfig {
        computed_from_ts,
        skip_history_effective,
        mode,
    }
}

fn env_with_source(key: &'static str, default: &str, sources: &mut BTreeMap<&'static str, String>) -> String {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            value
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            default.to_string()
        }
    }
}

fn get_required_with_source(
    key: &'static str,
    sources: &mut BTreeMap<&'static str, String>,
) -> Result<String, ConfigError> {
    env::var(key)
        .map(|value| {
            sources.insert(key, "env".to_string());
            value
        })
        .map_err(|_| ConfigError::MissingVar(key))
}

fn parse_i64_with_source(
    key: &'static str,
    default: i64,
    sources: &mut BTreeMap<&'static str, String>,
) -> Result<i64, ConfigError> {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            value.parse::<i64>().map_err(|err| ConfigError::InvalidInt(key, err))
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            Ok(default)
        }
    }
}

fn parse_u64_with_source(
    key: &'static str,
    default: u64,
    sources: &mut BTreeMap<&'static str, String>,
) -> Result<u64, ConfigError> {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            value.parse::<u64>().map_err(|err| ConfigError::InvalidInt(key, err))
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            Ok(default)
        }
    }
}

fn parse_u8_with_source(
    key: &'static str,
    default: u8,
    sources: &mut BTreeMap<&'static str, String>,
) -> Result<u8, ConfigError> {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            value.parse::<u8>().map_err(|err| ConfigError::InvalidInt(key, err))
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            Ok(default)
        }
    }
}

fn parse_f64_with_source(
    key: &'static str,
    default: f64,
    sources: &mut BTreeMap<&'static str, String>,
) -> Result<f64, ConfigError> {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            value.parse::<f64>().map_err(|err| ConfigError::InvalidFloat(key, err))
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            Ok(default)
        }
    }
}

fn parse_bool_with_source(
    key: &'static str,
    default: bool,
    sources: &mut BTreeMap<&'static str, String>,
) -> bool {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            default
        }
    }
}

fn track_file_sources(file_cfg: &FileConfig, sources: &mut BTreeMap<&'static str, String>) {
    let set_source = |sources: &mut BTreeMap<&'static str, String>, key: &'static str, present: bool| {
        sources.insert(key, if present { "file".to_string() } else { "default".to_string() });
    };
    set_source(sources, "portfolio", file_cfg.portfolio.is_some());
    set_source(sources, "exchange", file_cfg.exchange.is_some());
    set_source(sources, "instrument_group", file_cfg.instrument_group.is_some());
    set_source(sources, "tf_sec", file_cfg.tf_sec.is_some());
    set_source(sources, "from_ts", file_cfg.from_ts.is_some());
    set_source(sources, "ws_url", file_cfg.ws_url.is_some());
    set_source(sources, "cws_url", file_cfg.cws_url.is_some());
    set_source(sources, "oauth_url", file_cfg.oauth_url.is_some());
    set_source(sources, "refresh_token", file_cfg.refresh_token.is_some());
    set_source(sources, "skip_history_bars", file_cfg.skip_history_bars.is_some());
    set_source(sources, "skip_history_positions", file_cfg.skip_history_positions.is_some());
    set_source(sources, "skip_history_orders", file_cfg.skip_history_orders.is_some());
    set_source(sources, "split_adjust", file_cfg.split_adjust.is_some());
    set_source(sources, "format", file_cfg.format.is_some());
    set_source(sources, "frequency_ms", file_cfg.frequency_ms.is_some());
    set_source(sources, "backoff_initial_ms", file_cfg.backoff_initial_ms.is_some());
    set_source(sources, "backoff_max_ms", file_cfg.backoff_max_ms.is_some());
    set_source(sources, "backoff_multiplier", file_cfg.backoff_multiplier.is_some());
    set_source(sources, "max_silence_bars_sec", file_cfg.max_silence_bars_sec.is_some());
    set_source(sources, "history_sessions", file_cfg.history_sessions.is_some());
    set_source(sources, "history_days_back", file_cfg.history_days_back.is_some());
    set_source(
        sources,
        "session_rollover_hour_utc",
        file_cfg.session_rollover_hour_utc.is_some(),
    );
    set_source(sources, "health_listen_addr", file_cfg.health_listen_addr.is_some());
    set_source(sources, "price_step", file_cfg.price_step.is_some());
    set_source(sources, "volume_step", file_cfg.volume_step.is_some());
    set_source(sources, "log_cash_positions", file_cfg.log_cash_positions.is_some());
    set_source(sources, "cash_symbols", file_cfg.cash_symbols.is_some());
    set_source(
        sources,
        "log_existing_snapshot_orders",
        file_cfg.log_existing_snapshot_orders.is_some(),
    );
    set_source(
        sources,
        "ws_idle_timeout_sec",
        file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.ws_idle_timeout_sec)
            .is_some(),
    );
    set_source(
        sources,
        "ws_ping_interval_sec",
        file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.ws_ping_interval_sec)
            .is_some(),
    );
    set_source(
        sources,
        "ws_ping_timeout_sec",
        file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.ws_ping_timeout_sec)
            .is_some(),
    );
    set_source(
        sources,
        "subscribe_ack_retries",
        file_cfg
            .ws
            .as_ref()
            .and_then(|ws| ws.subscribe_ack_retries)
            .is_some(),
    );
    set_source(
        sources,
        "warm_reconnect_max_gap_sec",
        file_cfg
            .reconnect
            .as_ref()
            .and_then(|reconnect| reconnect.warm_reconnect_max_gap_sec)
            .is_some(),
    );
    set_source(
        sources,
        "gap_backfill_padding_bars",
        file_cfg
            .reconnect
            .as_ref()
            .and_then(|reconnect| reconnect.gap_backfill_padding_bars)
            .is_some(),
    );
    set_source(
        sources,
        "cold_start_history_days_back",
        file_cfg
            .reconnect
            .as_ref()
            .and_then(|reconnect| reconnect.cold_start_history_days_back)
            .is_some(),
    );
    set_source(
        sources,
        "bar_silence_resync_min_sec",
        file_cfg
            .reconnect
            .as_ref()
            .and_then(|reconnect| reconnect.bar_silence_resync_min_sec)
            .is_some(),
    );
    sources.insert(
        "data_report_path",
        if env::var("DATA_REPORT_PATH").is_ok() {
            "env".to_string()
        } else {
            "default".to_string()
        },
    );
    sources.insert(
        "bar_dump_path",
        if env::var("BAR_DUMP_PATH").is_ok() {
            "env".to_string()
        } else {
            "default".to_string()
        },
    );
}

pub fn log_resolved_config(resolved: &ResolvedConfig, config_path: Option<&str>) {
    use tracing::info;
    if let Some(path) = config_path {
        info!(path, "Using config");
    }
    let cfg = &resolved.config;
    info!(
        portfolio = %cfg.portfolio,
        exchange = %cfg.exchange,
        instrument_group = %cfg.instrument_group,
        symbols = ?cfg.symbols,
        tf_sec = cfg.tf_sec,
        from_ts = cfg.from_ts,
        ws_url = %cfg.ws_url,
        cws_url = %cfg.cws_url,
        oauth_url = %cfg.oauth_url,
        skip_history_bars = cfg.skip_history_bars,
        skip_history_positions = cfg.skip_history_positions,
        skip_history_orders = cfg.skip_history_orders,
        split_adjust = cfg.split_adjust,
        format = %cfg.format,
        frequency_ms = cfg.frequency_ms,
        backoff_initial_ms = cfg.backoff_initial_ms,
        backoff_max_ms = cfg.backoff_max_ms,
        backoff_multiplier = cfg.backoff_multiplier,
        max_silence_bars_sec = cfg.max_silence_bars_sec,
        history_sessions = cfg.history_sessions,
        history_days_back = cfg.history_days_back,
        session_rollover_hour_utc = cfg.session_rollover_hour_utc,
        health_listen_addr = %cfg.health_listen_addr,
        price_step = cfg.price_step,
        volume_step = cfg.volume_step,
        log_positions_filter = ?cfg.log_positions_filter,
        log_cash_positions = cfg.log_cash_positions,
        cash_symbols = ?cfg.cash_symbols,
        log_existing_snapshot_orders = cfg.log_existing_snapshot_orders,
        ws_idle_timeout_sec = cfg.ws_idle_timeout_sec,
        ws_ping_interval_sec = cfg.ws_ping_interval_sec,
        ws_ping_timeout_sec = cfg.ws_ping_timeout_sec,
        subscribe_ack_timeout_ms = cfg.subscribe_ack_timeout_ms,
        subscribe_ack_timeout_positions_ms = cfg.subscribe_ack_timeout_positions_ms,
        subscribe_ack_retries = cfg.subscribe_ack_retries,
        warm_reconnect_max_gap_sec = cfg.warm_reconnect_max_gap_sec,
        gap_backfill_padding_bars = cfg.gap_backfill_padding_bars,
        cold_start_history_days_back = cfg.cold_start_history_days_back,
        bar_silence_resync_min_sec = cfg.bar_silence_resync_min_sec,
        data_report_path = ?cfg.data_report_path,
        bar_dump_path = ?cfg.bar_dump_path,
        "Resolved config"
    );
    info!(
        computed_from_ts = resolved.derived.computed_from_ts,
        skip_history_effective = resolved.derived.skip_history_effective,
        mode = ?resolved.derived.mode,
        "Resolved config derived"
    );
    info!(sources = ?resolved.sources, "Resolved config sources");
}

pub fn detect_config_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            return iter.next().cloned();
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(path.to_string());
        }
    }
    None
}
