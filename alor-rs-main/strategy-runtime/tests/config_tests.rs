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

fn repo_config_path(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(relative_path)
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
fn loads_ri_author41_42_shadow_config() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&[
        "STRATEGY_ID",
        "STRATEGY_KIND",
        "SYMBOL",
        "QTY",
        "TRADE_MODE",
        "ALLOW_LIVE_ORDERS",
        "PAPER_ENABLED",
        "SNAPSHOTS_STREAM",
    ]);
    let path = write_temp_config(
        r#"
[strategy]
strategy_id = "ri_author41_42.shadow.test"
strategy_kind = "ri_author41_42"
symbol = "RIM6"
qty = 1
tick_size = 10
timezone_offset_hours = 3

[strategy.ri_author41_42]
profile_id = "ri_author41_42_primary_combo_cost2"
timeframe = "10m"
mode = "shadow"
allow_order_emission = false
execution_path = "action_scoped_only"
decision_journal_path = "./reports/ri_author41_42_decisions.jsonl"
decision_journal_append = true
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");

    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::RiAuthor4142
    );
    assert_eq!(resolved.config.strategy.symbol, "RIM6");
    let settings = resolved
        .config
        .strategy
        .ri_author41_42()
        .expect("ri settings");
    assert_eq!(settings.profile_id, "ri_author41_42_primary_combo_cost2");
    assert_eq!(settings.timeframe, "10m");
    assert_eq!(settings.mode, "shadow");
    assert!(!settings.allow_order_emission);
    assert_eq!(settings.execution_path, "action_scoped_only");
    assert_eq!(
        settings.decision_journal_path.as_deref(),
        Some("./reports/ri_author41_42_decisions.jsonl")
    );
    assert!(settings.decision_journal_append);
}

#[test]
fn loads_ri_author41_42_7502miw_shadow_config() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let path = repo_root.join("configs/runtime.ri_author41_42.shadow.7502MIW.toml");

    let resolved = load_runtime_config(path, false).expect("load ri 7502MIW config");

    assert_eq!(resolved.config.portfolio, "7502MIW");
    assert_eq!(
        resolved.config.consumer_group,
        "strategy-runtime-ri-author41-42-shadow-7502MIW"
    );
    assert_eq!(resolved.config.streams.bars, "md.bars.7502MIW.RIM6.10m");
    assert_eq!(
        resolved.config.streams.commands,
        "cmd.orders.7502MIW.ri_author41_42.shadow"
    );
    assert_eq!(
        resolved.config.streams.acks,
        "cmd.acks.7502MIW.ri_author41_42.shadow"
    );
    assert_eq!(
        resolved.config.streams.health.as_deref(),
        Some("events.health.ri_author41_42.7502MIW")
    );
    assert_eq!(
        resolved.config.streams.runtime_state,
        "runtime.state.ri_author41_42.shadow.7502MIW"
    );
    assert_eq!(
        resolved.config.trade_mode,
        strategy_runtime::TradeMode::Paper
    );
    assert!(!resolved.config.allow_live_orders);
    assert_eq!(
        resolved.config.strategy.strategy_kind,
        strategy_runtime::StrategyKind::RiAuthor4142
    );
    assert_eq!(resolved.config.strategy.symbol, "RIM6");
    let settings = resolved
        .config
        .strategy
        .ri_author41_42()
        .expect("ri settings");
    assert_eq!(settings.mode, "shadow");
    assert!(!settings.allow_order_emission);
    assert_eq!(settings.execution_path, "action_scoped_only");
    assert_eq!(settings.order_symbol.as_deref(), Some("RTS-6.26"));
}

#[test]
fn loads_ri_author41_42_7502miw_pending_micro_config_as_locked_candidate() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let path = repo_root.join("configs/runtime.ri_author41_42.micro.7502MIW.pending.toml");

    let resolved = load_runtime_config(path, false).expect("load pending ri micro config");

    assert_eq!(resolved.config.portfolio, "7502MIW");
    assert_eq!(
        resolved.config.consumer_group,
        "strategy-runtime-ri-author41-42-micro-7502MIW"
    );
    assert_eq!(resolved.config.streams.bars, "md.bars.7502MIW.RIM6.10m");
    assert_eq!(
        resolved.config.streams.commands,
        "cmd.orders.7502MIW.ri_author41_42.micro"
    );
    assert_eq!(
        resolved.config.streams.acks,
        "cmd.acks.7502MIW.ri_author41_42.micro"
    );
    assert_eq!(
        resolved.config.streams.runtime_state,
        "runtime.state.ri_author41_42.micro.7502MIW"
    );
    assert_eq!(
        resolved.config.trade_mode,
        strategy_runtime::TradeMode::Live
    );
    assert!(resolved.config.allow_live_orders);

    let settings = resolved
        .config
        .strategy
        .ri_author41_42()
        .expect("ri settings");
    assert_eq!(settings.mode, "micro_live");
    assert!(settings.allow_order_emission);
    assert_eq!(settings.execution_path, "action_scoped_only");
    assert_eq!(settings.order_symbol.as_deref(), Some("RTS-6.26"));
    assert_eq!(
        settings.decision_journal_path.as_deref(),
        Some("/reports/ri_author41_42_7502MIW_micro_decisions.jsonl")
    );
}

#[test]
fn loads_ri_author41_42_riu6_roll_candidates() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();

    for (portfolio, qty) in [("7502MIW", 1.0), ("7502T0U", 1.0)] {
        let path = repo_root.join(format!(
            "configs/runtime.ri_author41_42.micro.{portfolio}.RIU6.roll-2026-06-12.toml"
        ));
        let resolved = load_runtime_config(path, false).expect("load RIU6 roll candidate");

        assert_eq!(resolved.config.portfolio, portfolio);
        assert_eq!(
            resolved.config.streams.bars,
            format!("md.bars.{portfolio}.RIU6.10m")
        );
        assert_eq!(resolved.config.strategy.symbol, "RIU6");
        assert_eq!(resolved.config.strategy.qty, qty);
        assert!(resolved.config.reset_state_on_start);

        let settings = resolved
            .config
            .strategy
            .ri_author41_42()
            .expect("ri settings");
        assert_eq!(settings.mode, "micro_live");
        assert!(settings.allow_order_emission);
        assert_eq!(settings.execution_path, "action_scoped_only");
        assert_eq!(settings.order_symbol.as_deref(), Some("RTS-9.26"));
        assert_eq!(
            settings.excluded_model_dates,
            ["2026-06-12".to_string(), "2026-11-04".to_string()]
        );
        assert_eq!(settings.min_anchor_bars, 80);
        assert_eq!(settings.anchor_first_bar_at_or_before, "09:10:00");
        assert_eq!(settings.anchor_last_bar_at_or_after, "23:30:00");
        assert_eq!(settings.actual_expiry_date.as_deref(), Some("2026-09-17"));
        assert_eq!(settings.roll_target_sessions_before, 1);
        assert_eq!(settings.roll_fallback_sessions_before, 2);
    }
}

#[test]
fn loads_moex_early_session_shadow_configs_as_diagnostics_only() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    for (relative_path, expected_kind, expected_state) in [
        (
            "configs/runtime.ri_author41_42.shadow09.7502MIW.toml",
            strategy_runtime::StrategyKind::RiAuthor4142,
            "runtime.state.ri_author41_42.shadow09.7502MIW",
        ),
        (
            "configs/runtime.ri_author41_42.shadow07.7502MIW.toml",
            strategy_runtime::StrategyKind::RiAuthor4142,
            "runtime.state.ri_author41_42.shadow07.prospective_v1.7502MIW",
        ),
        (
            "configs/runtime.alor_usdrubf.shadow09.7502MIW.toml",
            strategy_runtime::StrategyKind::AlorUsdrubfHybrid,
            "runtime.state.alor_usdrubf_hybrid_v1.shadow09.usdrubf.7502MIW",
        ),
        (
            "configs/runtime.alor_usdrubf.shadow07.7502MIW.toml",
            strategy_runtime::StrategyKind::AlorUsdrubfHybrid,
            "runtime.state.alor_usdrubf_hybrid_v1.shadow07.usdrubf.7502MIW",
        ),
        (
            "configs/runtime.hybrid_imoexf.shadow09.7502MIW.toml",
            strategy_runtime::StrategyKind::HybridIntraday,
            "runtime.state.hybrid_intraday.shadow09.imoexf.7502MIW",
        ),
        (
            "configs/runtime.hybrid_imoexf.shadow07.7502MIW.toml",
            strategy_runtime::StrategyKind::HybridIntraday,
            "runtime.state.hybrid_intraday.shadow07.imoexf.7502MIW",
        ),
    ] {
        let resolved = load_runtime_config(repo_config_path(relative_path), false)
            .expect("load shadow config");

        assert_eq!(
            resolved.config.trade_mode,
            strategy_runtime::TradeMode::Paper
        );
        assert!(!resolved.config.allow_live_orders);
        assert!(!resolved.config.allow_paper_orders);
        assert!(!resolved.config.require_gateway_ready);
        if matches!(
            expected_kind,
            strategy_runtime::StrategyKind::AlorUsdrubfHybrid
                | strategy_runtime::StrategyKind::HybridIntraday
        ) {
            assert!(
                resolved.config.paper.append,
                "USDRUBF/IMOEXF shadow reports must survive restarts: {relative_path}"
            );
        }
        assert_eq!(resolved.config.strategy.strategy_kind, expected_kind);
        assert_eq!(resolved.config.streams.runtime_state, expected_state);
        assert!(
            resolved.config.streams.commands.contains(".shadow."),
            "shadow configs must use isolated non-live command streams: {relative_path}"
        );
        assert!(
            resolved.config.streams.acks.contains(".shadow."),
            "shadow configs must use isolated non-live ack streams: {relative_path}"
        );
    }
}

#[test]
fn loads_moex_early_session_ri_shadow_policies() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let legacy = load_runtime_config(
        repo_config_path("configs/runtime.ri_author41_42.shadow09.7502MIW.toml"),
        false,
    )
    .expect("load legacy09 ri shadow config");
    let canonical = load_runtime_config(
        repo_config_path("configs/runtime.ri_author41_42.shadow07.7502MIW.toml"),
        false,
    )
    .expect("load canonical07 ri shadow config");

    let legacy_settings = legacy
        .config
        .strategy
        .ri_author41_42()
        .expect("legacy ri settings");
    let canonical_settings = canonical
        .config
        .strategy
        .ri_author41_42()
        .expect("canonical ri settings");

    assert_eq!(legacy_settings.mode, "shadow");
    assert_eq!(canonical_settings.mode, "prospective_shadow");
    assert!(!legacy_settings.allow_order_emission);
    assert!(!canonical_settings.allow_order_emission);
    assert_eq!(legacy_settings.order_symbol.as_deref(), Some("RTS-9.26"));
    assert_eq!(canonical_settings.order_symbol.as_deref(), Some("RTS-9.26"));

    assert_eq!(legacy_settings.session_start_time, "09:00:00");
    assert_eq!(legacy_settings.author41_entry_end_time, "12:00:00");
    assert_eq!(legacy_settings.min_anchor_bars, 80);
    assert_eq!(legacy_settings.anchor_first_bar_at_or_before, "09:10:00");

    assert_eq!(canonical_settings.session_start_time, "07:00:00");
    assert_eq!(canonical_settings.author41_entry_end_time, "10:00:00");
    assert_eq!(canonical_settings.author41_time_exit, "20:00:00");
    assert_eq!(canonical_settings.author42_exit_time, "23:00:00");
    assert_eq!(canonical_settings.min_anchor_bars, 92);
    assert_eq!(canonical_settings.anchor_first_bar_at_or_before, "07:10:00");
    assert_eq!(canonical_settings.anchor_last_bar_at_or_after, "23:30:00");
}

#[test]
fn loads_moex_early_session_usdrubf_shadow_policies() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let legacy = load_runtime_config(
        repo_config_path("configs/runtime.alor_usdrubf.shadow09.7502MIW.toml"),
        false,
    )
    .expect("load legacy09 usdrubf shadow config");
    let canonical = load_runtime_config(
        repo_config_path("configs/runtime.alor_usdrubf.shadow07.7502MIW.toml"),
        false,
    )
    .expect("load canonical07 usdrubf shadow config");

    let legacy_settings = legacy
        .config
        .strategy
        .alor_usdrubf_hybrid()
        .expect("legacy usdrubf settings");
    let canonical_settings = canonical
        .config
        .strategy
        .alor_usdrubf_hybrid()
        .expect("canonical usdrubf settings");

    assert_eq!(legacy_settings.mr_last_entry_time, "11:40:00");
    assert_eq!(legacy_settings.mr_force_exit_time, "11:50:00");
    assert_eq!(legacy_settings.bo_wait_hours, 2.0);
    assert!(!legacy_settings.enable_live_execution);

    assert_eq!(canonical_settings.mr_last_entry_time, "09:40:00");
    assert_eq!(canonical_settings.mr_force_exit_time, "09:50:00");
    assert_eq!(canonical_settings.bo_wait_hours, 2.0);
    assert_eq!(canonical_settings.bo_eod_exit_time, "23:30:00");
    assert!(!canonical_settings.enable_live_execution);
}

#[test]
fn loads_moex_early_session_imoexf_shadow_policies() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let legacy = load_runtime_config(
        repo_config_path("configs/runtime.hybrid_imoexf.shadow09.7502MIW.toml"),
        false,
    )
    .expect("load legacy09 imoexf shadow config");
    let canonical = load_runtime_config(
        repo_config_path("configs/runtime.hybrid_imoexf.shadow07.7502MIW.toml"),
        false,
    )
    .expect("load canonical07 imoexf shadow config");

    let legacy_strategy = &legacy
        .config
        .strategy
        .hybrid_intraday()
        .expect("legacy imoexf settings")
        .strategy;
    let canonical_strategy = &canonical
        .config
        .strategy
        .hybrid_intraday()
        .expect("canonical imoexf settings")
        .strategy;

    assert_eq!(legacy_strategy.model_session_start_time, "09:00:00");
    assert!(legacy_strategy.live_mr_entries_enabled);
    assert_eq!(legacy_strategy.mr_session_end_time, "11:59:00");
    assert_eq!(legacy_strategy.risk_gate_mode, "normal_append");
    assert_eq!(legacy_strategy.bo_wait_hours, 3.0);
    assert_eq!(
        legacy_strategy.risk_gate_ledger_key.as_deref(),
        Some("runtime.riskgate.sessions.hybrid_imoexf_shadow09.imoexf_primary_high180_lb120")
    );
    assert!(legacy_strategy.risk_gate_persist_in_shadow);

    assert_eq!(canonical_strategy.model_session_start_time, "07:00:00");
    assert!(canonical_strategy.live_mr_entries_enabled);
    assert_eq!(canonical_strategy.mr_session_end_time, "09:59:00");
    assert_eq!(canonical_strategy.risk_gate_mode, "normal_append");
    assert_eq!(canonical_strategy.bo_wait_hours, 3.0);
    assert_eq!(
        canonical_strategy.risk_gate_ledger_key.as_deref(),
        Some("runtime.riskgate.sessions.hybrid_imoexf_shadow07.imoexf_primary_high180_lb120")
    );
    assert!(canonical_strategy.risk_gate_persist_in_shadow);
    assert_eq!(
        canonical_strategy.orchestrator_breakout_eod_mode,
        "same_day"
    );
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
fn loads_split_hybrid_riskgate_profile_fields() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let path = write_temp_config(
        r#"
[strategy.common]
strategy_kind = "hybrid_intraday"
symbol = "IMOEXF"
qty = 1.0
side = "buy"

[strategy.hybrid_intraday]
profile = "imoexf_primary_riskgate_high180_lb120"
mr_variant = "high180"
mr_gate_policy = "shadow_pnl_lb120_positive"
risk_gate_mode = "bootstrap_from_seed"
risk_gate_seed_file = "docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts/riskgate_high180_lb120_seed_2026-04-26.csv"
risk_gate_ledger_key = "runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120"
model_session_start_time = "09:00:00"
model_session_end_time = "23:49:59"
bo_k = 0.53
"#,
    );

    let resolved = load_runtime_config(path, false).expect("load config");
    let settings = resolved
        .config
        .strategy
        .hybrid_intraday()
        .expect("hybrid settings");
    let strategy = &settings.strategy;

    assert_eq!(strategy.profile, "imoexf_primary_riskgate_high180_lb120");
    assert_eq!(strategy.mr_variant, "high180");
    assert_eq!(strategy.mr_gate_policy, "shadow_pnl_lb120_positive");
    assert_eq!(strategy.risk_gate_mode, "bootstrap_from_seed");
    assert_eq!(
        strategy.risk_gate_seed_file.as_deref(),
        Some("docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts/riskgate_high180_lb120_seed_2026-04-26.csv")
    );
    assert_eq!(
        strategy.risk_gate_ledger_key.as_deref(),
        Some("runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120")
    );
    assert_eq!(strategy.model_session_start_time, "09:00:00");
    assert_eq!(strategy.model_session_end_time, "23:49:59");
    assert_eq!(strategy.bo_k, 0.53);
    assert_eq!(
        resolved.sources.strategy.hybrid_intraday,
        strategy_runtime::config::ConfigSource::File
    );
}

#[test]
fn loads_live_imoexf_riskgate_shadow_configs() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let bootstrap_path =
        repo_root.join("configs/runtime.hybrid.live.7502SN6.riskgate-bootstrap.toml");
    let shadow_path = repo_root.join("configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml");

    let bootstrap = load_runtime_config(bootstrap_path, false).expect("load bootstrap config");
    let shadow = load_runtime_config(shadow_path, false).expect("load shadow config");

    for resolved in [&bootstrap, &shadow] {
        let common = &resolved.config.strategy.common;
        let hybrid = resolved
            .config
            .strategy
            .hybrid_intraday()
            .expect("hybrid strategy settings")
            .strategy
            .clone();

        assert_eq!(common.strategy_id, "hybrid_imoexf");
        assert_eq!(
            common.strategy_kind,
            strategy_runtime::StrategyKind::HybridIntraday
        );
        assert_eq!(common.symbol, "IMOEXF");
        assert_eq!(resolved.config.streams.bars, "md.bars.7502SN6.10m");
        assert_eq!(
            resolved.config.streams.runtime_state,
            "runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6"
        );
        assert_eq!(hybrid.profile, "imoexf_primary_riskgate_high180_lb120");
        assert_eq!(hybrid.mr_variant, "high180");
        assert_eq!(hybrid.mr_gate_policy, "shadow_pnl_lb120_positive");
        assert_eq!(
            hybrid.risk_gate_seed_file.as_deref(),
            Some("/configs/riskgate_high180_lb120_seed_2026-04-26.csv")
        );
        assert_eq!(
            hybrid.risk_gate_ledger_key.as_deref(),
            Some("runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120")
        );
        assert_eq!(hybrid.model_session_start_time, "09:00:00");
        assert_eq!(hybrid.model_session_end_time, "23:49:59");
        assert_eq!(hybrid.bo_k, 0.53);
    }

    assert_eq!(
        bootstrap
            .config
            .strategy
            .hybrid_intraday()
            .expect("bootstrap hybrid settings")
            .strategy
            .risk_gate_mode,
        "bootstrap_from_seed"
    );
    assert_eq!(
        shadow
            .config
            .strategy
            .hybrid_intraday()
            .expect("shadow hybrid settings")
            .strategy
            .risk_gate_mode,
        "normal_append"
    );
}

#[test]
fn loads_live_imoexf_canonical07_replacement_candidate() {
    let _env_guard = env_lock();
    let _guards = clear_env_vars(&["STRATEGY_KIND", "STRATEGY_ID"]);

    let resolved = load_runtime_config(
        repo_config_path("configs/runtime.hybrid.live.7502MIW.riskgate-canonical07.toml"),
        false,
    )
    .expect("load IMOEXF canonical07 live candidate");
    let common = &resolved.config.strategy.common;
    let hybrid = &resolved
        .config
        .strategy
        .hybrid_intraday()
        .expect("hybrid settings")
        .strategy;

    assert_eq!(resolved.config.portfolio, "7502MIW");
    assert_eq!(
        resolved.config.trade_mode,
        strategy_runtime::TradeMode::Live
    );
    assert!(resolved.config.allow_live_orders);
    assert!(resolved.config.require_gateway_ready);
    assert_eq!(common.strategy_id, "hybrid_imoexf");
    assert_eq!(common.symbol, "IMOEXF");
    assert_eq!(common.qty, 6.0);
    assert_eq!(
        resolved.config.streams.runtime_state,
        "runtime.state.hybrid_intraday.live.riskgate_canonical07_v1.imoexf.7502MIW"
    );
    assert_eq!(
        common
            .trading_periods
            .as_ref()
            .expect("trading periods")
            .session_start,
        chrono::NaiveTime::from_hms_opt(7, 0, 0).unwrap()
    );
    assert_eq!(hybrid.model_session_start_time, "07:00:00");
    assert!(!hybrid.live_mr_entries_enabled);
    assert_eq!(hybrid.mr_session_end_time, "09:59:00");
    assert_eq!(hybrid.mr_exit_offset_min, 10);
    assert_eq!(hybrid.bo_wait_hours, 3.0);
    assert_eq!(hybrid.risk_gate_mode, "normal_append");
    assert_eq!(
        hybrid.risk_gate_legacy_session_start_time.as_deref(),
        Some("09:00:00")
    );
    assert_eq!(
        hybrid.risk_gate_legacy_session_end_time.as_deref(),
        Some("23:49:59")
    );
    assert_eq!(
        hybrid.risk_gate_session_policy_transition_date.as_deref(),
        Some("2026-07-28")
    );
    assert_eq!(
        hybrid.risk_gate_ledger_key.as_deref(),
        Some("runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120")
    );
    assert!(!hybrid.risk_gate_persist_in_shadow);
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
