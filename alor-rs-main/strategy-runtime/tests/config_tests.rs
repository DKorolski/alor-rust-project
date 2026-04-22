use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use strategy_runtime::config::load_runtime_config;

struct EnvGuard {
    key: String,
    value: Option<String>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.value {
            env::set_var(&self.key, value);
        } else {
            env::remove_var(&self.key);
        }
    }
}

fn clear_env_vars(keys: &[&str]) -> Vec<EnvGuard> {
    keys.iter()
        .map(|key| EnvGuard {
            key: (*key).to_string(),
            value: env::var(key).ok(),
        })
        .inspect(|guard| env::remove_var(&guard.key))
        .collect()
}

fn set_env_var(key: &str, value: &str) -> EnvGuard {
    let guard = EnvGuard {
        key: key.to_string(),
        value: env::var(key).ok(),
    };
    env::set_var(key, value);
    guard
}

fn write_temp_config(contents: &str) -> PathBuf {
    let filename = format!("strategy-runtime-{}.toml", uuid::Uuid::new_v4());
    let path = env::temp_dir().join(filename);
    fs::write(&path, contents).expect("write temp config");
    path
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("lock env mutex")
}

#[test]
fn loads_toml_defaults() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "REDIS_URL",
        "PORTFOLIO",
        "EXCHANGE",
        "SOURCE",
        "CONSUMER_GROUP",
        "CONSUMER_NAME",
        "TRADE_MODE",
        "ALLOW_LIVE_ORDERS",
        "GUARD_LOG_INTERVAL_MS",
        "BOOTSTRAP_DUMP",
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "PAPER_TRADES_CSV",
        "PAPER_SUMMARY_JSON",
        "PAPER_APPEND",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
        "BACKTEST_TRADES_CSV",
        "BACKTEST_SUMMARY_JSON",
        "BACKTEST_APPEND",
        "STRATEGY_ID",
        "SYMBOL",
        "QTY",
        "SIDE",
        "PLACE_OFFSET_TICKS",
        "TICK_SIZE",
        "SNAPSHOTS_STREAM",
        "STREAM_HEALTH",
    ]);
    let path = write_temp_config("redis_url = \"redis://example/\"\n");

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(resolved.config.redis_url, "redis://example/");
    assert_eq!(resolved.config.portfolio, "demo");
    assert_eq!(resolved.config.strategy.strategy_id, "limit_cancel");
    assert_eq!(resolved.config.streams.bars, "md.bars.demo.1m");
    assert_eq!(resolved.config.streams.trades, "broker.trades.demo");
}

#[test]
fn env_overrides_take_precedence() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "REDIS_URL",
        "PORTFOLIO",
        "EXCHANGE",
        "SOURCE",
        "CONSUMER_GROUP",
        "CONSUMER_NAME",
        "TRADE_MODE",
        "ALLOW_LIVE_ORDERS",
        "GUARD_LOG_INTERVAL_MS",
        "BOOTSTRAP_DUMP",
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "PAPER_TRADES_CSV",
        "PAPER_SUMMARY_JSON",
        "PAPER_APPEND",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
        "BACKTEST_TRADES_CSV",
        "BACKTEST_SUMMARY_JSON",
        "BACKTEST_APPEND",
        "STRATEGY_ID",
        "SYMBOL",
        "QTY",
        "SIDE",
        "PLACE_OFFSET_TICKS",
        "TICK_SIZE",
        "SNAPSHOTS_STREAM",
        "STREAM_HEALTH",
    ]);
    let _redis_guard = set_env_var("REDIS_URL", "redis://env/");
    let _portfolio_guard = set_env_var("PORTFOLIO", "env-portfolio");
    let _strategy_guard = set_env_var("STRATEGY_ID", "env-strategy");
    let _health_guard = set_env_var("STREAM_HEALTH", "");

    let path = write_temp_config(
        r#"
redis_url = "redis://file/"
portfolio = "file-portfolio"

[strategy]
strategy_id = "file-strategy"
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(resolved.config.redis_url, "redis://env/");
    assert_eq!(resolved.config.portfolio, "env-portfolio");
    assert_eq!(resolved.config.strategy.strategy_id, "env-strategy");
    assert_eq!(resolved.config.streams.health, None);
}

#[test]
fn health_default_is_events_health() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STREAM_HEALTH",
        "TRADE_MODE",
        "ALLOW_LIVE_ORDERS",
        "GUARD_LOG_INTERVAL_MS",
        "BOOTSTRAP_DUMP",
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "PAPER_TRADES_CSV",
        "PAPER_SUMMARY_JSON",
        "PAPER_APPEND",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
        "BACKTEST_TRADES_CSV",
        "BACKTEST_SUMMARY_JSON",
        "BACKTEST_APPEND",
        "SNAPSHOTS_STREAM",
    ]);
    let path = write_temp_config("redis_url = \"redis://example/\"\n");

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.streams.health.as_deref(),
        Some("events.health")
    );
}

#[test]
fn loads_runtime_mode_settings() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "TRADE_MODE",
        "ALLOW_LIVE_ORDERS",
        "GUARD_LOG_INTERVAL_MS",
        "BOOTSTRAP_DUMP",
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "PAPER_TRADES_CSV",
        "PAPER_SUMMARY_JSON",
        "PAPER_APPEND",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
        "BACKTEST_TRADES_CSV",
        "BACKTEST_SUMMARY_JSON",
        "BACKTEST_APPEND",
        "SNAPSHOTS_STREAM",
    ]);
    let path = write_temp_config(
        r#"
[runtime]
trade_mode = "backtest"
allow_live_orders = false
guard_log_interval_ms = 1234
bootstrap_dump = true

[paper]
enabled = false
output = "file"
file_path = "./paper.jsonl"

[backtest]
enabled = true
trade_log = "./backtest.log"
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.trade_mode,
        strategy_runtime::TradeMode::Backtest
    );
    assert!(!resolved.config.allow_live_orders);
    assert_eq!(resolved.config.guard_log_interval_ms, 1234);
    assert!(resolved.config.bootstrap_dump);
    assert!(!resolved.config.paper.enabled);
    assert_eq!(
        resolved.config.paper.output,
        strategy_runtime::PaperOutput::File
    );
    assert_eq!(resolved.config.paper.file_path, "./paper.jsonl");
    assert!(resolved.config.backtest.enabled);
    assert_eq!(resolved.config.backtest.trade_log, "./backtest.log");
}

#[test]
fn loads_paper_report_paths() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "PAPER_TRADES_CSV",
        "PAPER_SUMMARY_JSON",
        "PAPER_APPEND",
    ]);
    let path = write_temp_config(
        r#"
[paper]
enabled = true
trades_csv = "./custom_trades.csv"
summary_json = "./custom_summary.json"
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(resolved.config.paper.trades_csv, "./custom_trades.csv");
    assert_eq!(resolved.config.paper.summary_json, "./custom_summary.json");
}

#[test]
fn loads_toy_session_timing_strategy_fields() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_KIND",
        "SESSION_OPEN_HOUR",
        "SESSION_OPEN_MINUTE",
        "SESSION_CLOSE_HOUR",
        "SESSION_CLOSE_MINUTE",
        "ENTRY_AFTER_OPEN_MIN",
        "EXIT_BEFORE_CLOSE_MIN",
        "TIMEZONE_OFFSET_HOURS",
    ]);
    let path = write_temp_config(
        r#"
[strategy]
strategy_kind = "toy_session_timing"
session_open_hour = 9
session_open_minute = 30
session_close_hour = 18
session_close_minute = 45
entry_after_open_min = 59
exit_before_close_min = 20
timezone_offset_hours = 3
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::ToySessionTiming
    );
    assert_eq!(resolved.config.strategy.session_open_hour, 9);
    assert_eq!(resolved.config.strategy.session_open_minute, 30);
    assert_eq!(resolved.config.strategy.session_close_hour, 18);
    assert_eq!(resolved.config.strategy.session_close_minute, 45);
    assert_eq!(resolved.config.strategy.entry_after_open_min, 59);
    assert_eq!(resolved.config.strategy.exit_before_close_min, 20);
    assert_eq!(resolved.config.strategy.timezone_offset_hours, 3);
}

#[test]
fn loads_replay_settings_from_file() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "REPLAY_ENABLED",
        "REPLAY_BARS_CSV_PATH",
        "REPLAY_REFERENCE_TRADES_CSV_PATH",
        "REPLAY_OUTPUT_DIR",
        "REPLAY_PRICE_TOLERANCE",
        "REPLAY_STRICT_DEDUP",
    ]);

    let path = write_temp_config(
        r#"
[replay]
enabled = true
bars_csv_path = "./paper_bars_2.csv"
reference_trades_csv_path = "./paper_trades_2.csv"
output_dir = "./artifacts"
price_tolerance = 0.0001
strict_dedup = false
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert!(resolved.config.replay.enabled);
    assert_eq!(
        resolved.config.replay.bars_csv_path.as_deref(),
        Some("./paper_bars_2.csv")
    );
    assert_eq!(
        resolved.config.replay.reference_trades_csv_path.as_deref(),
        Some("./paper_trades_2.csv")
    );
    assert_eq!(resolved.config.replay.output_dir, "./artifacts");
    assert_eq!(resolved.config.replay.price_tolerance, 0.0001);
    assert!(!resolved.config.replay.strict_dedup);
}

#[test]
fn loads_split_market_buy_and_close_strategy_sections() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_KIND",
        "LIVE_ORDER_STYLE",
        "MARKETABLE_LIMIT_OFFSET_TICKS",
        "ENTRY_ACK_TIMEOUT_MS",
        "EXIT_FILL_TIMEOUT_MS",
    ]);

    let path = write_temp_config(
        r#"
[strategy.common]
strategy_id = "market_buy_and_close"
strategy_kind = "market_buy_and_close"
symbol = "USDRUBF"
qty = 2.0
side = "buy"

[strategy.market_buy_and_close]
live_order_style = "marketable_limit"
marketable_limit_offset_ticks = 4
close_trigger = "position_update"
entry_ack_timeout_ms = 1111
exit_fill_timeout_ms = 2222
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .market_buy_and_close()
        .expect("market buy and close settings");

    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::MarketBuyAndClose
    );
    assert_eq!(resolved.config.strategy.symbol, "USDRUBF");
    assert_eq!(resolved.config.strategy.qty, 2.0);
    assert_eq!(
        settings.live_order_style,
        strategy_runtime::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle::MarketableLimit
    );
    assert_eq!(settings.marketable_limit_offset_ticks, 4);
    assert_eq!(
        settings.close_trigger,
        strategy_runtime::CloseTrigger::PositionUpdate
    );
    assert_eq!(settings.entry_ack_timeout_ms, 1111);
    assert_eq!(settings.exit_fill_timeout_ms, 2222);
    assert_eq!(
        resolved.sources.strategy.live_order_style,
        strategy_runtime::config::ConfigSource::File
    );
    assert_eq!(
        resolved.sources.strategy.marketable_limit_offset_ticks,
        strategy_runtime::config::ConfigSource::File
    );
}

#[test]
fn loads_split_session_gap_sections_with_specific_runtime_fields() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_KIND",
        "PLACE_OFFSET_TICKS",
        "ENTRY_ACK_TIMEOUT_MS",
        "EXIT_FILL_TIMEOUT_MS",
    ]);

    let path = write_temp_config(
        r#"
[strategy.common]
strategy_id = "session_gap_standalone"
strategy_kind = "session_gap_standalone"
symbol = "USDRUBF"
qty = 1.0
side = "buy"

[strategy.session_gap]
place_offset_ticks = 7
entry_ack_timeout_ms = 1234
exit_fill_timeout_ms = 5678
k_long = 0.81
close_hour = 21
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .session_gap_standalone()
        .expect("session gap settings");

    assert_eq!(settings.place_offset_ticks, 7);
    assert_eq!(settings.entry_ack_timeout_ms, 1234);
    assert_eq!(settings.exit_fill_timeout_ms, 5678);
    assert_eq!(settings.k_long, 0.81);
    assert_eq!(settings.close_hour, 21);
    assert_eq!(
        resolved.sources.strategy.place_offset_ticks,
        strategy_runtime::config::ConfigSource::File
    );
    assert_eq!(
        resolved.sources.strategy.entry_ack_timeout_ms,
        strategy_runtime::config::ConfigSource::File
    );
}

#[test]
fn loads_split_alor_skeleton_sections() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let path = write_temp_config(
        r#"
[strategy.common]
strategy_kind = "alor"
symbol = "ALRS"
qty = 3.0
side = "sell"

[strategy.alor_usdrubf_hybrid]
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::AlorUsdrubfHybrid
    );
    assert_eq!(
        resolved.config.strategy.strategy_id,
        "alor_usdrubf_hybrid_v1"
    );
    assert_eq!(resolved.config.strategy.symbol, "ALRS");
    assert_eq!(resolved.config.strategy.qty, 3.0);
    assert_eq!(resolved.config.strategy.side, alor_protocol::Side::Sell);
    assert!(resolved.config.strategy.alor_usdrubf_hybrid().is_some());
}

#[test]
fn loads_split_alor_skeleton_runtime_fields() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let path = write_temp_config(
        r#"
[strategy.common]
strategy_kind = "alor_skeleton"
symbol = "USDRUBF"
tick_size = 0.01

[strategy.alor_usdrubf_hybrid]
mr_min_rel_range = 0.01
mr_max_rel_range = 0.06
bo_wait_hours = 2.5
commission_pct_per_side = 0.004
enable_live_execution = true
use_fixed_live_size = true
live_fixed_units = 1.0
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .alor_usdrubf_hybrid()
        .expect("alor skeleton settings");

    assert_eq!(settings.mr_min_rel_range, 0.01);
    assert_eq!(settings.mr_max_rel_range, 0.06);
    assert_eq!(settings.bo_wait_hours, 2.5);
    assert_eq!(settings.commission_pct_per_side, 0.004);
    assert!(settings.enable_live_execution);
    assert!(settings.use_fixed_live_size);
    assert_eq!(settings.live_fixed_units, 1.0);
    assert_eq!(
        resolved.sources.strategy.alor_usdrubf_hybrid,
        strategy_runtime::config::ConfigSource::File
    );
}

#[test]
fn replay_env_overrides_take_precedence() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "REPLAY_ENABLED",
        "REPLAY_BARS_CSV_PATH",
        "REPLAY_REFERENCE_TRADES_CSV_PATH",
        "REPLAY_OUTPUT_DIR",
        "REPLAY_PRICE_TOLERANCE",
        "REPLAY_STRICT_DEDUP",
    ]);
    let _enabled = set_env_var("REPLAY_ENABLED", "true");
    let _bars = set_env_var("REPLAY_BARS_CSV_PATH", "./env_bars.csv");
    let _reference = set_env_var("REPLAY_REFERENCE_TRADES_CSV_PATH", "./env_trades.csv");
    let _out = set_env_var("REPLAY_OUTPUT_DIR", "./env_out");
    let _tol = set_env_var("REPLAY_PRICE_TOLERANCE", "0.25");
    let _dedup = set_env_var("REPLAY_STRICT_DEDUP", "false");

    let path = write_temp_config(
        r#"
[replay]
enabled = false
bars_csv_path = "./file_bars.csv"
reference_trades_csv_path = "./file_trades.csv"
output_dir = "./file_out"
price_tolerance = 0.00000001
strict_dedup = true
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert!(resolved.config.replay.enabled);
    assert_eq!(
        resolved.config.replay.bars_csv_path.as_deref(),
        Some("./env_bars.csv")
    );
    assert_eq!(
        resolved.config.replay.reference_trades_csv_path.as_deref(),
        Some("./env_trades.csv")
    );
    assert_eq!(resolved.config.replay.output_dir, "./env_out");
    assert_eq!(resolved.config.replay.price_tolerance, 0.25);
    assert!(!resolved.config.replay.strict_dedup);
}

#[test]
fn loads_session_gap_settings_from_nested_strategy_section() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_ID",
        "SYMBOL",
        "QTY",
        "SIDE",
        "PLACE_OFFSET_TICKS",
        "TICK_SIZE",
    ]);

    let path = write_temp_config(
        r#"
[strategy]
strategy_kind = "session_gap_standalone"

[strategy.session_gap]
signal_minute = 50
k_long = 0.77
k_short = 0.55
wait_hours = 4
k_tp_long = 0.33
k_sl_long = 0.88
k_tp_short = 0.21
k_sl_short = 0.74
long_ex_pct = 3.2
short_ex_pct = 1.8
start_cash = 12345
cash_factor = 0.42
max_entry_hour = 17
close_hour = 22
close_minute = 40
session_gap_min = 45.0
exit_offset_min = 12
work_weekends = true
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .session_gap_standalone()
        .expect("session gap settings");

    assert_eq!(settings.signal_minute, 50);
    assert_eq!(settings.k_long, 0.77);
    assert_eq!(settings.k_short, 0.55);
    assert_eq!(settings.wait_hours, 4);
    assert_eq!(settings.k_tp_long, 0.33);
    assert_eq!(settings.k_sl_long, 0.88);
    assert_eq!(settings.k_tp_short, 0.21);
    assert_eq!(settings.k_sl_short, 0.74);
    assert_eq!(settings.long_ex_pct, 3.2);
    assert_eq!(settings.short_ex_pct, 1.8);
    assert_eq!(settings.start_cash, 12345.0);
    assert_eq!(settings.cash_factor, 0.42);
    assert_eq!(settings.max_entry_hour, 17);
    assert_eq!(settings.close_hour, 22);
    assert_eq!(settings.close_minute, 40);
    assert_eq!(settings.session_gap_min, 45.0);
    assert_eq!(settings.exit_offset_min, 12);
    assert!(settings.work_weekends);
}

#[test]
fn session_gap_defaults_apply_when_section_missing() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_KIND",
        "SESSION_GAP_K_LONG",
        "SESSION_GAP_K_SHORT",
        "SESSION_GAP_SIGNAL_MINUTE",
        "SESSION_GAP_WAIT_HOURS",
        "SESSION_GAP_K_TP_LONG",
        "SESSION_GAP_K_SL_LONG",
        "SESSION_GAP_K_TP_SHORT",
        "SESSION_GAP_K_SL_SHORT",
        "SESSION_GAP_LONG_EX_PCT",
        "SESSION_GAP_SHORT_EX_PCT",
        "SESSION_GAP_START_CASH",
        "SESSION_GAP_CASH_FACTOR",
        "SESSION_GAP_MAX_ENTRY_HOUR",
        "SESSION_GAP_CLOSE_HOUR",
        "SESSION_GAP_CLOSE_MINUTE",
        "SESSION_GAP_MIN",
        "SESSION_GAP_EXIT_OFFSET_MIN",
        "SESSION_GAP_WORK_WEEKENDS",
    ]);

    let path = write_temp_config(
        r#"
[strategy]
strategy_kind = "session_gap_standalone"
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .session_gap_standalone()
        .expect("session gap settings");

    assert_eq!(settings.signal_minute, 59);
    assert_eq!(settings.k_long, 0.5);
    assert_eq!(settings.wait_hours, 2);
    assert_eq!(settings.k_tp_long, 0.28);
    assert_eq!(settings.session_gap_min, 60.0);
    assert_eq!(settings.exit_offset_min, 20);
    assert!(!settings.work_weekends);
}

#[test]
fn session_gap_partial_section_uses_defaults_for_missing_fields() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_KIND",
        "SESSION_GAP_K_LONG",
        "SESSION_GAP_K_SHORT",
        "SESSION_GAP_SIGNAL_MINUTE",
        "SESSION_GAP_WAIT_HOURS",
        "SESSION_GAP_K_TP_LONG",
        "SESSION_GAP_K_SL_LONG",
        "SESSION_GAP_K_TP_SHORT",
        "SESSION_GAP_K_SL_SHORT",
        "SESSION_GAP_LONG_EX_PCT",
        "SESSION_GAP_SHORT_EX_PCT",
        "SESSION_GAP_START_CASH",
        "SESSION_GAP_CASH_FACTOR",
        "SESSION_GAP_MAX_ENTRY_HOUR",
        "SESSION_GAP_CLOSE_HOUR",
        "SESSION_GAP_CLOSE_MINUTE",
        "SESSION_GAP_MIN",
        "SESSION_GAP_EXIT_OFFSET_MIN",
        "SESSION_GAP_WORK_WEEKENDS",
    ]);

    let path = write_temp_config(
        r#"
[strategy]
strategy_kind = "session_gap_standalone"

[strategy.session_gap]
signal_minute = 50
k_long = 0.77
close_hour = 22
work_weekends = true
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .session_gap_standalone()
        .expect("session gap settings");

    assert_eq!(settings.signal_minute, 50);
    assert_eq!(settings.k_long, 0.77);
    assert_eq!(settings.close_hour, 22);
    assert!(settings.work_weekends);

    assert_eq!(settings.wait_hours, 2);
    assert_eq!(settings.k_tp_long, 0.28);
    assert_eq!(settings.session_gap_min, 60.0);
    assert_eq!(settings.exit_offset_min, 20);
}

#[test]
fn rejects_unknown_strategy_kind_from_file() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND"]);
    let path = write_temp_config(
        r#"
[strategy]
strategy_kind = "unknown_future_strategy"
"#,
    );

    let err = load_runtime_config(path, false).expect_err("unknown strategy_kind must fail");

    assert!(err
        .to_string()
        .contains("invalid strategy.strategy_kind: unknown_future_strategy"));
}

#[test]
fn rejects_unknown_strategy_kind_from_env() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND"]);
    let _kind_guard = set_env_var("STRATEGY_KIND", "unknown_env_strategy");
    let path = write_temp_config("redis_url = \"redis://example/\"\n");

    let err = load_runtime_config(path, false).expect_err("unknown STRATEGY_KIND must fail");

    assert!(err
        .to_string()
        .contains("invalid STRATEGY_KIND: unknown_env_strategy"));
}

#[test]
fn strategy_id_defaults_follow_strategy_kind_when_id_omitted_in_file() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);
    let path = write_temp_config(
        r#"
[strategy]
strategy_kind = "session_gap_standalone"
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::SessionGapStandalone
    );
    assert_eq!(
        resolved.config.strategy.strategy_id,
        strategy_runtime::StrategyKind::SessionGapStandalone.default_strategy_id()
    );
}

#[test]
fn strategy_id_defaults_follow_strategy_kind_when_id_omitted_in_env() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);
    let _kind_guard = set_env_var("STRATEGY_KIND", "hybrid_intraday");
    let path = write_temp_config("redis_url = \"redis://example/\"\n");

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::HybridIntraday
    );
    assert_eq!(
        resolved.config.strategy.strategy_id,
        strategy_runtime::StrategyKind::HybridIntraday.default_strategy_id()
    );
}

#[test]
fn rejects_non_matching_split_specific_section() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);
    let path = write_temp_config(
        r#"
[strategy.common]
strategy_kind = "session_gap_standalone"

[strategy.market_buy_and_close]
live_order_style = "market"
"#,
    );

    let err = load_runtime_config(path, false).expect_err("non-matching section must fail");
    let message = err.to_string();
    assert!(message.contains("non-matching strategy specific section"));
    assert!(message.contains("strategy.market_buy_and_close"));
}
