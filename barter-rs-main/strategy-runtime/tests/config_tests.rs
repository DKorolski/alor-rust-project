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
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
        "STRATEGY_ID",
        "SYMBOL",
        "QTY",
        "SIDE",
        "PLACE_OFFSET_TICKS",
        "TICK_SIZE",
        "STREAM_HEALTH",
    ]);
    let path = write_temp_config("redis_url = \"redis://example/\"\n");

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(resolved.config.redis_url, "redis://example/");
    assert_eq!(resolved.config.portfolio, "demo");
    assert_eq!(resolved.config.strategy.strategy_id, "limit_cancel");
    assert_eq!(resolved.config.streams.bars, "md.bars.demo.1m");
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
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
        "STRATEGY_ID",
        "SYMBOL",
        "QTY",
        "SIDE",
        "PLACE_OFFSET_TICKS",
        "TICK_SIZE",
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
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
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
        "PAPER_ENABLED",
        "PAPER_OUTPUT",
        "PAPER_FILE_PATH",
        "BACKTEST_ENABLED",
        "BACKTEST_TRADE_LOG",
    ]);
    let path = write_temp_config(
        r#"
[runtime]
trade_mode = "backtest"
allow_live_orders = true
guard_log_interval_ms = 1234

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
    assert!(resolved.config.allow_live_orders);
    assert_eq!(resolved.config.guard_log_interval_ms, 1234);
    assert!(!resolved.config.paper.enabled);
    assert_eq!(
        resolved.config.paper.output,
        strategy_runtime::PaperOutput::File
    );
    assert_eq!(resolved.config.paper.file_path, "./paper.jsonl");
    assert!(resolved.config.backtest.enabled);
    assert_eq!(resolved.config.backtest.trade_log, "./backtest.log");
}
