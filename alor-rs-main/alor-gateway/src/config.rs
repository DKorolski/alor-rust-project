use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use thiserror::Error;

use alor_types::TradingPeriods;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ControlCwsMode {
    #[default]
    LegacyLongLived,
    ActionScoped,
}

impl ControlCwsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegacyLongLived => "legacy_long_lived",
            Self::ActionScoped => "action_scoped",
        }
    }

    fn parse(raw: &str) -> Result<Self, ConfigError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "legacy_long_lived" => Ok(Self::LegacyLongLived),
            "action_scoped" => Ok(Self::ActionScoped),
            _ => Err(ConfigError::InvalidControlCwsMode(raw.to_string())),
        }
    }
}

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
    pub trading_periods: Option<TradingPeriods>,
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
    pub control_path_stale_after_sec: u64,
    pub control_path_pre_entry_recycle_enabled: bool,
    pub control_path_pre_exit_recycle_enabled: bool,
    pub control_path_recycle_timeout_ms: u64,
    pub control_path_recycle_timeout_ms_exit: u64,
    pub control_path_post_recycle_exit_send_window_ms: u64,
    pub control_path_hardening_log_only: bool,
    pub control_cws_mode: ControlCwsMode,
    pub action_scope_enable_create_limit: bool,
    pub action_scope_enable_market: bool,
    pub action_scope_enable_delete_limit: bool,
    pub action_scope_enable_replace_limit: bool,
    pub action_scope_enable_exit: bool,
    pub action_scope_open_timeout_ms: u64,
    pub action_scope_authorize_timeout_ms: u64,
    pub action_scope_force_token_refresh_before_authorize: bool,
    pub action_scope_followup_window_ms: u64,
    pub action_scope_max_session_lifetime_ms: u64,
    pub action_scope_close_timeout_ms: u64,
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
            cws_url: env::var("ALOR_CWS_URL").unwrap_or_else(|_| "wss://api.alor.ru/cws".into()),
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
            trading_periods: None,
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
                    parse_list(&env::var("ALOR_SYMBOLS").unwrap_or_else(|_| "USDRUBF".to_string()))
                } else {
                    parse_list(&raw)
                }
            },
            log_cash_positions: parse_bool("ALOR_LOG_CASH_POSITIONS", false),
            cash_symbols: {
                let raw = env::var("ALOR_CASH_SYMBOLS").unwrap_or_else(|_| "RUB,SUR".to_string());
                parse_list(&raw)
            },
            log_existing_snapshot_orders: parse_bool("ALOR_LOG_EXISTING_SNAPSHOT_ORDERS", false),
            ws_idle_timeout_sec: parse_u64("ALOR_WS_IDLE_TIMEOUT_SEC", 70)?,
            ws_ping_interval_sec: parse_u64("ALOR_WS_PING_INTERVAL_SEC", 30)?,
            ws_ping_timeout_sec: parse_u64("ALOR_WS_PING_TIMEOUT_SEC", 15)?,
            subscribe_ack_timeout_ms,
            subscribe_ack_timeout_positions_ms,
            subscribe_ack_retries: parse_u8("ALOR_SUBSCRIBE_ACK_RETRIES", 3)?,
            warm_reconnect_max_gap_sec: parse_u64("ALOR_WARM_RECONNECT_MAX_GAP_SEC", 21_600)?,
            gap_backfill_padding_bars: parse_u8("ALOR_GAP_BACKFILL_PADDING_BARS", 2)?,
            cold_start_history_days_back: parse_u8("ALOR_COLD_START_HISTORY_DAYS_BACK", 4)?,
            bar_silence_resync_min_sec: parse_u64("ALOR_BAR_SILENCE_RESYNC_MIN_SEC", 300)?,
            control_path_stale_after_sec: parse_u64("ALOR_CONTROL_PATH_STALE_AFTER_SEC", 900)?,
            control_path_pre_entry_recycle_enabled: parse_bool(
                "ALOR_CONTROL_PATH_PRE_ENTRY_RECYCLE_ENABLED",
                true,
            ),
            control_path_pre_exit_recycle_enabled: parse_bool(
                "ALOR_CONTROL_PATH_PRE_EXIT_RECYCLE_ENABLED",
                true,
            ),
            control_path_recycle_timeout_ms: parse_u64(
                "ALOR_CONTROL_PATH_RECYCLE_TIMEOUT_MS",
                5_000,
            )?,
            control_path_recycle_timeout_ms_exit: parse_u64(
                "ALOR_CONTROL_PATH_RECYCLE_TIMEOUT_MS_EXIT",
                10_000,
            )?,
            control_path_post_recycle_exit_send_window_ms: parse_u64(
                "ALOR_CONTROL_PATH_POST_RECYCLE_EXIT_SEND_WINDOW_MS",
                5_000,
            )?,
            control_path_hardening_log_only: parse_bool(
                "ALOR_CONTROL_PATH_HARDENING_LOG_ONLY",
                false,
            ),
            control_cws_mode: parse_control_cws_mode(
                "ALOR_CONTROL_CWS_MODE",
                ControlCwsMode::LegacyLongLived,
            )?,
            action_scope_enable_create_limit: parse_bool(
                "ALOR_ACTION_SCOPE_ENABLE_CREATE_LIMIT",
                false,
            ),
            action_scope_enable_market: parse_bool("ALOR_ACTION_SCOPE_ENABLE_MARKET", false),
            action_scope_enable_delete_limit: parse_bool(
                "ALOR_ACTION_SCOPE_ENABLE_DELETE_LIMIT",
                false,
            ),
            action_scope_enable_replace_limit: parse_bool(
                "ALOR_ACTION_SCOPE_ENABLE_REPLACE_LIMIT",
                false,
            ),
            action_scope_enable_exit: parse_bool("ALOR_ACTION_SCOPE_ENABLE_EXIT", false),
            action_scope_open_timeout_ms: parse_u64("ALOR_ACTION_SCOPE_OPEN_TIMEOUT_MS", 5_000)?,
            action_scope_authorize_timeout_ms: parse_u64(
                "ALOR_ACTION_SCOPE_AUTHORIZE_TIMEOUT_MS",
                5_000,
            )?,
            action_scope_force_token_refresh_before_authorize: parse_bool(
                "ALOR_ACTION_SCOPE_FORCE_TOKEN_REFRESH_BEFORE_AUTHORIZE",
                false,
            ),
            action_scope_followup_window_ms: parse_u64(
                "ALOR_ACTION_SCOPE_FOLLOWUP_WINDOW_MS",
                5_000,
            )?,
            action_scope_max_session_lifetime_ms: parse_u64(
                "ALOR_ACTION_SCOPE_MAX_SESSION_LIFETIME_MS",
                15_000,
            )?,
            action_scope_close_timeout_ms: parse_u64("ALOR_ACTION_SCOPE_CLOSE_TIMEOUT_MS", 2_000)?,
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        })
    }

    pub fn from_env_with_sources() -> Result<ResolvedConfig, ConfigError> {
        if let Ok(path) = env::var("ALOR_GATEWAY_CONFIG") {
            return Self::from_file_with_sources(path);
        }

        let mut sources = BTreeMap::new();
        let subscribe_ack_timeout_ms =
            parse_u64_with_source("ALOR_SUBSCRIBE_ACK_TIMEOUT_MS", 5_000, &mut sources)?;
        let subscribe_ack_timeout_positions_ms = parse_u64_with_source(
            "ALOR_SUBSCRIBE_ACK_TIMEOUT_POSITIONS_MS",
            subscribe_ack_timeout_ms,
            &mut sources,
        )?;

        let config = Self {
            portfolio: get_required_with_source("ALOR_PORTFOLIO", &mut sources)?,
            exchange: env_with_source("ALOR_EXCHANGE", "MOEX", &mut sources),
            instrument_group: env_with_source("ALOR_INSTRUMENT_GROUP", "RFUD", &mut sources),
            symbols: parse_list(&env_with_source("ALOR_SYMBOLS", "USDRUBF", &mut sources)),
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
            backoff_max_ms: parse_u64_with_source("ALOR_BACKOFF_MAX_MS", 30_000, &mut sources)?,
            backoff_multiplier: parse_u8_with_source("ALOR_BACKOFF_MULTIPLIER", 2, &mut sources)?,
            max_silence_bars_sec: parse_u64_with_source(
                "ALOR_MAX_SILENCE_BARS_SEC",
                180,
                &mut sources,
            )?,
            trading_periods: None,
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
            control_path_stale_after_sec: parse_u64_with_source(
                "ALOR_CONTROL_PATH_STALE_AFTER_SEC",
                900,
                &mut sources,
            )?,
            control_path_pre_entry_recycle_enabled: parse_bool_with_source(
                "ALOR_CONTROL_PATH_PRE_ENTRY_RECYCLE_ENABLED",
                true,
                &mut sources,
            ),
            control_path_pre_exit_recycle_enabled: parse_bool_with_source(
                "ALOR_CONTROL_PATH_PRE_EXIT_RECYCLE_ENABLED",
                true,
                &mut sources,
            ),
            control_path_recycle_timeout_ms: parse_u64_with_source(
                "ALOR_CONTROL_PATH_RECYCLE_TIMEOUT_MS",
                5_000,
                &mut sources,
            )?,
            control_path_recycle_timeout_ms_exit: parse_u64_with_source(
                "ALOR_CONTROL_PATH_RECYCLE_TIMEOUT_MS_EXIT",
                10_000,
                &mut sources,
            )?,
            control_path_post_recycle_exit_send_window_ms: parse_u64_with_source(
                "ALOR_CONTROL_PATH_POST_RECYCLE_EXIT_SEND_WINDOW_MS",
                5_000,
                &mut sources,
            )?,
            control_path_hardening_log_only: parse_bool_with_source(
                "ALOR_CONTROL_PATH_HARDENING_LOG_ONLY",
                false,
                &mut sources,
            ),
            control_cws_mode: parse_control_cws_mode_with_source(
                "ALOR_CONTROL_CWS_MODE",
                ControlCwsMode::LegacyLongLived,
                &mut sources,
            )?,
            action_scope_enable_create_limit: parse_bool_with_source(
                "ALOR_ACTION_SCOPE_ENABLE_CREATE_LIMIT",
                false,
                &mut sources,
            ),
            action_scope_enable_market: parse_bool_with_source(
                "ALOR_ACTION_SCOPE_ENABLE_MARKET",
                false,
                &mut sources,
            ),
            action_scope_enable_delete_limit: parse_bool_with_source(
                "ALOR_ACTION_SCOPE_ENABLE_DELETE_LIMIT",
                false,
                &mut sources,
            ),
            action_scope_enable_replace_limit: parse_bool_with_source(
                "ALOR_ACTION_SCOPE_ENABLE_REPLACE_LIMIT",
                false,
                &mut sources,
            ),
            action_scope_enable_exit: parse_bool_with_source(
                "ALOR_ACTION_SCOPE_ENABLE_EXIT",
                false,
                &mut sources,
            ),
            action_scope_open_timeout_ms: parse_u64_with_source(
                "ALOR_ACTION_SCOPE_OPEN_TIMEOUT_MS",
                5_000,
                &mut sources,
            )?,
            action_scope_authorize_timeout_ms: parse_u64_with_source(
                "ALOR_ACTION_SCOPE_AUTHORIZE_TIMEOUT_MS",
                5_000,
                &mut sources,
            )?,
            action_scope_force_token_refresh_before_authorize: parse_bool_with_source(
                "ALOR_ACTION_SCOPE_FORCE_TOKEN_REFRESH_BEFORE_AUTHORIZE",
                false,
                &mut sources,
            ),
            action_scope_followup_window_ms: parse_u64_with_source(
                "ALOR_ACTION_SCOPE_FOLLOWUP_WINDOW_MS",
                5_000,
                &mut sources,
            )?,
            action_scope_max_session_lifetime_ms: parse_u64_with_source(
                "ALOR_ACTION_SCOPE_MAX_SESSION_LIFETIME_MS",
                15_000,
                &mut sources,
            )?,
            action_scope_close_timeout_ms: parse_u64_with_source(
                "ALOR_ACTION_SCOPE_CLOSE_TIMEOUT_MS",
                2_000,
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
        let file_cfg: FileConfig =
            toml::from_str(&contents).map_err(|err| ConfigError::ParseToml {
                path: path.clone(),
                source: err,
            })?;
        let symbols = file_cfg
            .symbols
            .clone()
            .unwrap_or_else(|| vec!["USDRUBF".to_string()]);
        let log_positions_filter = file_cfg
            .log_positions_filter
            .clone()
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
        let (refresh_token, _refresh_token_source) = resolve_refresh_token_from_file(&file_cfg)?;

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
            refresh_token,
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
            trading_periods: file_cfg.trading_periods.clone(),
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
            control_path_stale_after_sec: file_cfg.control_path_stale_after_sec.unwrap_or(900),
            control_path_pre_entry_recycle_enabled: file_cfg
                .control_path_pre_entry_recycle_enabled
                .unwrap_or(true),
            control_path_pre_exit_recycle_enabled: file_cfg
                .control_path_pre_exit_recycle_enabled
                .unwrap_or(true),
            control_path_recycle_timeout_ms: file_cfg
                .control_path_recycle_timeout_ms
                .unwrap_or(5_000),
            control_path_recycle_timeout_ms_exit: file_cfg
                .control_path_recycle_timeout_ms_exit
                .unwrap_or(10_000),
            control_path_post_recycle_exit_send_window_ms: file_cfg
                .control_path_post_recycle_exit_send_window_ms
                .unwrap_or(5_000),
            control_path_hardening_log_only: file_cfg
                .control_path_hardening_log_only
                .unwrap_or(false),
            control_cws_mode: file_cfg.control_cws_mode.unwrap_or_default(),
            action_scope_enable_create_limit: file_cfg
                .action_scope_enable_create_limit
                .unwrap_or(false),
            action_scope_enable_market: file_cfg.action_scope_enable_market.unwrap_or(false),
            action_scope_enable_delete_limit: file_cfg
                .action_scope_enable_delete_limit
                .unwrap_or(false),
            action_scope_enable_replace_limit: file_cfg
                .action_scope_enable_replace_limit
                .unwrap_or(false),
            action_scope_enable_exit: file_cfg.action_scope_enable_exit.unwrap_or(false),
            action_scope_open_timeout_ms: file_cfg.action_scope_open_timeout_ms.unwrap_or(5_000),
            action_scope_authorize_timeout_ms: file_cfg
                .action_scope_authorize_timeout_ms
                .unwrap_or(5_000),
            action_scope_force_token_refresh_before_authorize: file_cfg
                .action_scope_force_token_refresh_before_authorize
                .unwrap_or(false),
            action_scope_followup_window_ms: file_cfg
                .action_scope_followup_window_ms
                .unwrap_or(5_000),
            action_scope_max_session_lifetime_ms: file_cfg
                .action_scope_max_session_lifetime_ms
                .unwrap_or(15_000),
            action_scope_close_timeout_ms: file_cfg.action_scope_close_timeout_ms.unwrap_or(2_000),
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        })
    }

    pub fn from_file_with_sources(path: String) -> Result<ResolvedConfig, ConfigError> {
        let contents = fs::read_to_string(&path).map_err(|err| ConfigError::ReadFile {
            path: path.clone(),
            source: err,
        })?;
        let file_cfg: FileConfig =
            toml::from_str(&contents).map_err(|err| ConfigError::ParseToml {
                path: path.clone(),
                source: err,
            })?;
        let mut sources = BTreeMap::new();
        let source_label = |field: &'static str| format!("file:{field}");
        let default_label = |field: &'static str| format!("default:{field}");

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
            sources.insert(
                "log_positions_filter",
                default_label("log_positions_filter"),
            );
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

        let (refresh_token, refresh_token_source) = resolve_refresh_token_from_file(&file_cfg)?;

        let config = Self {
            portfolio: file_cfg
                .portfolio
                .clone()
                .ok_or(ConfigError::MissingField("portfolio"))?,
            exchange: file_cfg
                .exchange
                .clone()
                .unwrap_or_else(|| "MOEX".to_string()),
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
            refresh_token,
            skip_history_bars: file_cfg.skip_history_bars.unwrap_or(false),
            skip_history_positions: file_cfg.skip_history_positions.unwrap_or(false),
            skip_history_orders: file_cfg.skip_history_orders.unwrap_or(false),
            split_adjust: file_cfg.split_adjust.unwrap_or(true),
            format: file_cfg
                .format
                .clone()
                .unwrap_or_else(|| "Simple".to_string()),
            frequency_ms: file_cfg.frequency_ms.unwrap_or(250),
            backoff_initial_ms: file_cfg.backoff_initial_ms.unwrap_or(1_000),
            backoff_max_ms: file_cfg.backoff_max_ms.unwrap_or(30_000),
            backoff_multiplier: file_cfg.backoff_multiplier.unwrap_or(2),
            max_silence_bars_sec: file_cfg.max_silence_bars_sec.unwrap_or(180),
            trading_periods: file_cfg.trading_periods.clone(),
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
            control_path_stale_after_sec: file_cfg.control_path_stale_after_sec.unwrap_or(900),
            control_path_pre_entry_recycle_enabled: file_cfg
                .control_path_pre_entry_recycle_enabled
                .unwrap_or(true),
            control_path_pre_exit_recycle_enabled: file_cfg
                .control_path_pre_exit_recycle_enabled
                .unwrap_or(true),
            control_path_recycle_timeout_ms: file_cfg
                .control_path_recycle_timeout_ms
                .unwrap_or(5_000),
            control_path_recycle_timeout_ms_exit: file_cfg
                .control_path_recycle_timeout_ms_exit
                .unwrap_or(10_000),
            control_path_post_recycle_exit_send_window_ms: file_cfg
                .control_path_post_recycle_exit_send_window_ms
                .unwrap_or(5_000),
            control_path_hardening_log_only: file_cfg
                .control_path_hardening_log_only
                .unwrap_or(false),
            control_cws_mode: file_cfg.control_cws_mode.unwrap_or_default(),
            action_scope_enable_create_limit: file_cfg
                .action_scope_enable_create_limit
                .unwrap_or(false),
            action_scope_enable_market: file_cfg.action_scope_enable_market.unwrap_or(false),
            action_scope_enable_delete_limit: file_cfg
                .action_scope_enable_delete_limit
                .unwrap_or(false),
            action_scope_enable_replace_limit: file_cfg
                .action_scope_enable_replace_limit
                .unwrap_or(false),
            action_scope_enable_exit: file_cfg.action_scope_enable_exit.unwrap_or(false),
            action_scope_open_timeout_ms: file_cfg.action_scope_open_timeout_ms.unwrap_or(5_000),
            action_scope_authorize_timeout_ms: file_cfg
                .action_scope_authorize_timeout_ms
                .unwrap_or(5_000),
            action_scope_force_token_refresh_before_authorize: file_cfg
                .action_scope_force_token_refresh_before_authorize
                .unwrap_or(false),
            action_scope_followup_window_ms: file_cfg
                .action_scope_followup_window_ms
                .unwrap_or(5_000),
            action_scope_max_session_lifetime_ms: file_cfg
                .action_scope_max_session_lifetime_ms
                .unwrap_or(15_000),
            action_scope_close_timeout_ms: file_cfg.action_scope_close_timeout_ms.unwrap_or(2_000),
            data_report_path: env::var("DATA_REPORT_PATH").ok(),
            bar_dump_path: env::var("BAR_DUMP_PATH").ok(),
        };

        track_file_sources(&file_cfg, &mut sources);
        sources.insert("refresh_token", refresh_token_source.to_string());
        sources.insert("config_path", format!("file:{path}"));
        let derived = derive_config(&config);
        Ok(ResolvedConfig {
            config,
            sources,
            derived,
        })
    }
}

fn resolve_refresh_token_from_file(
    file_cfg: &FileConfig,
) -> Result<(String, &'static str), ConfigError> {
    if let Ok(value) = env::var("ALOR_REFRESH_TOKEN") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok((trimmed.to_string(), "env:ALOR_REFRESH_TOKEN"));
        }
    }

    if let Some(value) = file_cfg.refresh_token.as_ref() {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok((trimmed.to_string(), "file:refresh_token"));
        }
    }

    Err(ConfigError::MissingField("refresh_token"))
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
    #[error("invalid control_cws_mode: {0}")]
    InvalidControlCwsMode(String),
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse toml config {path}: {source}")]
    ParseToml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
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
    trading_periods: Option<TradingPeriods>,
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
    control_path_stale_after_sec: Option<u64>,
    control_path_pre_entry_recycle_enabled: Option<bool>,
    control_path_recycle_timeout_ms: Option<u64>,
    control_path_pre_exit_recycle_enabled: Option<bool>,
    control_path_recycle_timeout_ms_exit: Option<u64>,
    control_path_post_recycle_exit_send_window_ms: Option<u64>,
    control_path_hardening_log_only: Option<bool>,
    control_cws_mode: Option<ControlCwsMode>,
    action_scope_enable_create_limit: Option<bool>,
    action_scope_enable_market: Option<bool>,
    action_scope_enable_delete_limit: Option<bool>,
    action_scope_enable_replace_limit: Option<bool>,
    action_scope_enable_exit: Option<bool>,
    action_scope_open_timeout_ms: Option<u64>,
    action_scope_authorize_timeout_ms: Option<u64>,
    action_scope_force_token_refresh_before_authorize: Option<bool>,
    action_scope_followup_window_ms: Option<u64>,
    action_scope_max_session_lifetime_ms: Option<u64>,
    action_scope_close_timeout_ms: Option<u64>,
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
        Ok(value) => value
            .parse::<i64>()
            .map_err(|err| ConfigError::InvalidInt(key, err)),
        Err(_) => Ok(default),
    }
}

fn parse_u64(key: &'static str, default: u64) -> Result<u64, ConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|err| ConfigError::InvalidInt(key, err)),
        Err(_) => Ok(default),
    }
}

fn parse_u8(key: &'static str, default: u8) -> Result<u8, ConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u8>()
            .map_err(|err| ConfigError::InvalidInt(key, err)),
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

fn parse_control_cws_mode(
    key: &'static str,
    default: ControlCwsMode,
) -> Result<ControlCwsMode, ConfigError> {
    match env::var(key) {
        Ok(value) => ControlCwsMode::parse(&value),
        Err(_) => Ok(default),
    }
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

fn env_with_source(
    key: &'static str,
    default: &str,
    sources: &mut BTreeMap<&'static str, String>,
) -> String {
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
        .inspect(|_| {
            sources.insert(key, "env".to_string());
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
            value
                .parse::<i64>()
                .map_err(|err| ConfigError::InvalidInt(key, err))
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
            value
                .parse::<u64>()
                .map_err(|err| ConfigError::InvalidInt(key, err))
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
            value
                .parse::<u8>()
                .map_err(|err| ConfigError::InvalidInt(key, err))
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
            value
                .parse::<f64>()
                .map_err(|err| ConfigError::InvalidFloat(key, err))
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

fn parse_control_cws_mode_with_source(
    key: &'static str,
    default: ControlCwsMode,
    sources: &mut BTreeMap<&'static str, String>,
) -> Result<ControlCwsMode, ConfigError> {
    match env::var(key) {
        Ok(value) => {
            sources.insert(key, "env".to_string());
            ControlCwsMode::parse(&value)
        }
        Err(_) => {
            sources.insert(key, "default".to_string());
            Ok(default)
        }
    }
}

fn track_file_sources(file_cfg: &FileConfig, sources: &mut BTreeMap<&'static str, String>) {
    let set_source =
        |sources: &mut BTreeMap<&'static str, String>, key: &'static str, present: bool| {
            sources.insert(
                key,
                if present {
                    "file".to_string()
                } else {
                    "default".to_string()
                },
            );
        };
    set_source(sources, "portfolio", file_cfg.portfolio.is_some());
    set_source(sources, "exchange", file_cfg.exchange.is_some());
    set_source(
        sources,
        "instrument_group",
        file_cfg.instrument_group.is_some(),
    );
    set_source(sources, "tf_sec", file_cfg.tf_sec.is_some());
    set_source(sources, "from_ts", file_cfg.from_ts.is_some());
    set_source(sources, "ws_url", file_cfg.ws_url.is_some());
    set_source(sources, "cws_url", file_cfg.cws_url.is_some());
    set_source(sources, "oauth_url", file_cfg.oauth_url.is_some());
    set_source(sources, "refresh_token", file_cfg.refresh_token.is_some());
    set_source(
        sources,
        "skip_history_bars",
        file_cfg.skip_history_bars.is_some(),
    );
    set_source(
        sources,
        "skip_history_positions",
        file_cfg.skip_history_positions.is_some(),
    );
    set_source(
        sources,
        "skip_history_orders",
        file_cfg.skip_history_orders.is_some(),
    );
    set_source(sources, "split_adjust", file_cfg.split_adjust.is_some());
    set_source(sources, "format", file_cfg.format.is_some());
    set_source(sources, "frequency_ms", file_cfg.frequency_ms.is_some());
    set_source(
        sources,
        "backoff_initial_ms",
        file_cfg.backoff_initial_ms.is_some(),
    );
    set_source(sources, "backoff_max_ms", file_cfg.backoff_max_ms.is_some());
    set_source(
        sources,
        "backoff_multiplier",
        file_cfg.backoff_multiplier.is_some(),
    );
    set_source(
        sources,
        "max_silence_bars_sec",
        file_cfg.max_silence_bars_sec.is_some(),
    );
    set_source(
        sources,
        "history_sessions",
        file_cfg.history_sessions.is_some(),
    );
    set_source(
        sources,
        "history_days_back",
        file_cfg.history_days_back.is_some(),
    );
    set_source(
        sources,
        "session_rollover_hour_utc",
        file_cfg.session_rollover_hour_utc.is_some(),
    );
    set_source(
        sources,
        "health_listen_addr",
        file_cfg.health_listen_addr.is_some(),
    );
    set_source(sources, "price_step", file_cfg.price_step.is_some());
    set_source(sources, "volume_step", file_cfg.volume_step.is_some());
    set_source(
        sources,
        "log_cash_positions",
        file_cfg.log_cash_positions.is_some(),
    );
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
    set_source(
        sources,
        "control_path_stale_after_sec",
        file_cfg.control_path_stale_after_sec.is_some(),
    );
    set_source(
        sources,
        "control_path_pre_entry_recycle_enabled",
        file_cfg.control_path_pre_entry_recycle_enabled.is_some(),
    );
    set_source(
        sources,
        "control_path_pre_exit_recycle_enabled",
        file_cfg.control_path_pre_exit_recycle_enabled.is_some(),
    );
    set_source(
        sources,
        "control_path_recycle_timeout_ms",
        file_cfg.control_path_recycle_timeout_ms.is_some(),
    );
    set_source(
        sources,
        "control_path_recycle_timeout_ms_exit",
        file_cfg.control_path_recycle_timeout_ms_exit.is_some(),
    );
    set_source(
        sources,
        "control_path_post_recycle_exit_send_window_ms",
        file_cfg
            .control_path_post_recycle_exit_send_window_ms
            .is_some(),
    );
    set_source(
        sources,
        "control_path_hardening_log_only",
        file_cfg.control_path_hardening_log_only.is_some(),
    );
    set_source(
        sources,
        "control_cws_mode",
        file_cfg.control_cws_mode.is_some(),
    );
    set_source(
        sources,
        "action_scope_enable_create_limit",
        file_cfg.action_scope_enable_create_limit.is_some(),
    );
    set_source(
        sources,
        "action_scope_enable_market",
        file_cfg.action_scope_enable_market.is_some(),
    );
    set_source(
        sources,
        "action_scope_enable_delete_limit",
        file_cfg.action_scope_enable_delete_limit.is_some(),
    );
    set_source(
        sources,
        "action_scope_enable_replace_limit",
        file_cfg.action_scope_enable_replace_limit.is_some(),
    );
    set_source(
        sources,
        "action_scope_enable_exit",
        file_cfg.action_scope_enable_exit.is_some(),
    );
    set_source(
        sources,
        "action_scope_open_timeout_ms",
        file_cfg.action_scope_open_timeout_ms.is_some(),
    );
    set_source(
        sources,
        "action_scope_authorize_timeout_ms",
        file_cfg.action_scope_authorize_timeout_ms.is_some(),
    );
    set_source(
        sources,
        "action_scope_force_token_refresh_before_authorize",
        file_cfg
            .action_scope_force_token_refresh_before_authorize
            .is_some(),
    );
    set_source(
        sources,
        "action_scope_followup_window_ms",
        file_cfg.action_scope_followup_window_ms.is_some(),
    );
    set_source(
        sources,
        "action_scope_max_session_lifetime_ms",
        file_cfg.action_scope_max_session_lifetime_ms.is_some(),
    );
    set_source(
        sources,
        "action_scope_close_timeout_ms",
        file_cfg.action_scope_close_timeout_ms.is_some(),
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
        control_path_stale_after_sec = cfg.control_path_stale_after_sec,
        control_path_pre_entry_recycle_enabled = cfg.control_path_pre_entry_recycle_enabled,
        control_path_pre_exit_recycle_enabled = cfg.control_path_pre_exit_recycle_enabled,
        control_path_recycle_timeout_ms = cfg.control_path_recycle_timeout_ms,
        control_path_recycle_timeout_ms_exit = cfg.control_path_recycle_timeout_ms_exit,
        control_path_post_recycle_exit_send_window_ms =
            cfg.control_path_post_recycle_exit_send_window_ms,
        control_path_hardening_log_only = cfg.control_path_hardening_log_only,
        control_cws_mode = cfg.control_cws_mode.as_str(),
        action_scope_enable_create_limit = cfg.action_scope_enable_create_limit,
        action_scope_enable_market = cfg.action_scope_enable_market,
        action_scope_enable_delete_limit = cfg.action_scope_enable_delete_limit,
        action_scope_enable_replace_limit = cfg.action_scope_enable_replace_limit,
        action_scope_enable_exit = cfg.action_scope_enable_exit,
        action_scope_open_timeout_ms = cfg.action_scope_open_timeout_ms,
        action_scope_authorize_timeout_ms = cfg.action_scope_authorize_timeout_ms,
        action_scope_force_token_refresh_before_authorize =
            cfg.action_scope_force_token_refresh_before_authorize,
        action_scope_followup_window_ms = cfg.action_scope_followup_window_ms,
        action_scope_max_session_lifetime_ms = cfg.action_scope_max_session_lifetime_ms,
        action_scope_close_timeout_ms = cfg.action_scope_close_timeout_ms,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_temp_config(name: &str, body: &str) -> String {
        let path = std::env::temp_dir().join(format!(
            "alor-gateway-{name}-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, body).expect("write temp config");
        path.to_string_lossy().to_string()
    }

    #[test]
    fn file_config_uses_env_refresh_token_override() {
        let _guard = env_lock().lock().expect("lock env");
        unsafe {
            std::env::set_var("ALOR_REFRESH_TOKEN", "env-token");
        }

        let path = write_temp_config(
            "env-refresh-override",
            r#"
portfolio = "demo"
refresh_token = "file-token"
"#,
        );

        let resolved =
            AlorGatewayConfig::from_file_with_sources(path.clone()).expect("load config");
        assert_eq!(resolved.config.refresh_token, "env-token");
        assert_eq!(
            resolved.sources.get("refresh_token").map(String::as_str),
            Some("env:ALOR_REFRESH_TOKEN")
        );

        unsafe {
            std::env::remove_var("ALOR_REFRESH_TOKEN");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn file_config_fails_when_refresh_token_missing_or_blank() {
        let _guard = env_lock().lock().expect("lock env");
        unsafe {
            std::env::remove_var("ALOR_REFRESH_TOKEN");
        }

        let path = write_temp_config(
            "missing-refresh-token",
            r#"
portfolio = "demo"
refresh_token = "   "
"#,
        );

        let err = AlorGatewayConfig::from_file_with_sources(path.clone()).expect_err("must fail");
        assert!(matches!(err, ConfigError::MissingField("refresh_token")));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn loads_ri_author41_42_7502miw_gateway_config() {
        let _guard = env_lock().lock().expect("lock env");
        unsafe {
            std::env::set_var("ALOR_REFRESH_TOKEN", "test-token");
        }
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .to_path_buf();
        let path = repo_root
            .join("configs/gateway.ri_author41_42.shadow.7502MIW.toml")
            .to_string_lossy()
            .to_string();

        let resolved = AlorGatewayConfig::from_file_with_sources(path).expect("load config");

        assert_eq!(resolved.config.portfolio, "7502MIW");
        assert_eq!(resolved.config.exchange, "MOEX");
        assert_eq!(resolved.config.instrument_group, "RFUD");
        assert_eq!(resolved.config.symbols, vec!["RIM6".to_string()]);
        assert_eq!(resolved.config.tf_sec, 600);
        assert_eq!(resolved.config.max_silence_bars_sec, 1200);
        assert_eq!(resolved.config.history_sessions, 6);
        assert_eq!(resolved.config.history_days_back, 6);
        assert_eq!(resolved.config.health_listen_addr, "127.0.0.1:8084");
        assert_eq!(resolved.config.price_step, 10.0);
        assert_eq!(resolved.config.volume_step, 1.0);
        assert_eq!(
            resolved.config.log_positions_filter,
            vec!["RIM6".to_string()]
        );
        assert!(!resolved.config.log_existing_snapshot_orders);
        assert_eq!(
            resolved.config.control_cws_mode,
            ControlCwsMode::ActionScoped
        );
        assert!(resolved.config.action_scope_enable_market);
        assert!(resolved.config.action_scope_enable_create_limit);
        assert!(resolved.config.action_scope_enable_delete_limit);
        assert!(resolved.config.action_scope_enable_exit);
        assert!(!resolved.config.action_scope_enable_replace_limit);
        assert!(
            resolved
                .config
                .action_scope_force_token_refresh_before_authorize
        );
        assert_eq!(resolved.config.action_scope_max_session_lifetime_ms, 15_000);
        let trading_periods = resolved
            .config
            .trading_periods
            .as_ref()
            .expect("trading periods");
        assert!(trading_periods.weekends_off);
        assert_eq!(trading_periods.timezone_offset_hours, 3);
        assert_eq!(
            trading_periods.session_start,
            chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap()
        );
        assert_eq!(
            trading_periods.session_end,
            chrono::NaiveTime::from_hms_opt(23, 49, 59).unwrap()
        );

        unsafe {
            std::env::remove_var("ALOR_REFRESH_TOKEN");
        }
    }

    #[test]
    fn loads_ri_author41_42_7502miw_pending_micro_gateway_config() {
        let _guard = env_lock().lock().expect("lock env");
        unsafe {
            std::env::set_var("ALOR_REFRESH_TOKEN", "test-token");
        }
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .to_path_buf();
        let path = repo_root
            .join("configs/gateway.ri_author41_42.micro.7502MIW.pending.toml")
            .to_string_lossy()
            .to_string();

        let resolved = AlorGatewayConfig::from_file_with_sources(path).expect("load config");

        assert_eq!(resolved.config.portfolio, "7502MIW");
        assert_eq!(resolved.config.symbols, vec!["RIM6".to_string()]);
        assert_eq!(resolved.config.tf_sec, 600);
        assert_eq!(
            resolved.config.control_cws_mode,
            ControlCwsMode::ActionScoped
        );
        assert!(resolved.config.action_scope_enable_market);
        assert!(resolved.config.action_scope_enable_exit);
        assert!(
            resolved
                .config
                .action_scope_force_token_refresh_before_authorize
        );
        assert_eq!(resolved.config.action_scope_max_session_lifetime_ms, 15_000);

        unsafe {
            std::env::remove_var("ALOR_REFRESH_TOKEN");
        }
    }
}
