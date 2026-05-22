use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use strategy_runtime::strategies::hybrid_intraday::{
    Action, BreakoutEodMode, High180MrConfig, High180MrEngine, High180Open, High180Signal,
    HybridOrchestrator, HybridOrchestratorConfig, IntradayBreakoutConfig, IntradayBreakoutEngine,
    MeanReversionConfig, MeanReversionEngine, MinRangeMode, Owner, Side,
};

const DEFAULT_BUNDLE_DIR: &str = "pre_rust_handoff/replay_data/imoexf_2023_2026";
const DEFAULT_OUT_DIR: &str = "./tmp/hybrid_out";
const DEFAULT_CASH: f64 = 100_000.0;
const DEFAULT_SIZE_PCT: f64 = 0.9;
const DEFAULT_COMMISSION: f64 = 1.0 / 28_000.0;
const NUMERIC_EPS: f64 = 1e-9;
const SUMMARY_EPS: f64 = 1e-4;
const RISK_GATE_LOOKBACK_SESSIONS: usize = 120;
const RISK_GATE_MIN_HISTORY_SESSIONS: usize = 60;
const RISK_GATE_MAKER_COST_POINTS: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    Train,
    Test,
    Golden,
    Hybrid,
}

impl Split {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "train" => Ok(Self::Train),
            "test" => Ok(Self::Test),
            "golden" => Ok(Self::Golden),
            "hybrid" | "all" => Ok(Self::Hybrid),
            other => bail!("unsupported split: {other}"),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Test => "test",
            Self::Golden => "golden",
            Self::Hybrid => "hybrid",
        }
    }

    fn prepared_file(&self) -> &'static str {
        match self {
            Self::Train => "prepared_train.csv",
            Self::Test => "prepared_test.csv",
            Self::Golden => "prepared_golden.csv",
            Self::Hybrid => "prepared_hybrid.csv",
        }
    }

    fn has_expected(&self) -> bool {
        !matches!(self, Self::Hybrid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WeekendPolicy {
    BaselineSkip,
    NoTradeButUpdate,
}

impl WeekendPolicy {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "baseline_skip" => Ok(Self::BaselineSkip),
            "no_trade_but_update" => Ok(Self::NoTradeButUpdate),
            other => bail!("unsupported weekend policy: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReplayProfile {
    BaselineRuntimeHybrid,
    ImoexfBoK053,
    ImoexfPrimaryRiskgateK053,
}

impl ReplayProfile {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "baseline_runtime_hybrid" | "baseline" => Ok(Self::BaselineRuntimeHybrid),
            "imoexf_bo_k053" | "bo_new_k053" => Ok(Self::ImoexfBoK053),
            "imoexf_primary_riskgate_k053" | "hybrid_mr_riskgate_high180_lb120__bo_new_k053" => {
                Ok(Self::ImoexfPrimaryRiskgateK053)
            }
            other => bail!("unsupported replay profile: {other}"),
        }
    }

    fn uses_close_bar_high180_replay(self) -> bool {
        matches!(self, Self::ImoexfPrimaryRiskgateK053)
    }
}

#[derive(Debug)]
struct Cli {
    bundle_dir: PathBuf,
    split: Split,
    out_dir: PathBuf,
    profile: ReplayProfile,
    check: bool,
    strict: bool,
    allow_non_strict: bool,
    verify_checksums: bool,
    assert_gap_flatten: bool,
    weekend_policy: WeekendPolicy,
    cash: f64,
    size_pct: f64,
    commission: f64,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            bundle_dir: PathBuf::from(DEFAULT_BUNDLE_DIR),
            split: Split::Golden,
            out_dir: PathBuf::from(DEFAULT_OUT_DIR),
            profile: ReplayProfile::BaselineRuntimeHybrid,
            check: false,
            strict: false,
            allow_non_strict: false,
            verify_checksums: false,
            assert_gap_flatten: false,
            weekend_policy: WeekendPolicy::BaselineSkip,
            cash: DEFAULT_CASH,
            size_pct: DEFAULT_SIZE_PCT,
            commission: DEFAULT_COMMISSION,
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedBar {
    ts: NaiveDateTime,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    close_prev: f64,
    day_range_prev: f64,
}

#[derive(Debug, Deserialize)]
struct PreparedBarCsv {
    datetime: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    close_prev: Option<f64>,
    dayrangeprev: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ActionRow {
    bar_ts: String,
    action: String,
    owner: String,
    side: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TradeRow {
    owner: String,
    side: String,
    entry_ts: String,
    exit_ts: String,
    entry_price: f64,
    exit_price: f64,
    size: i64,
    pnl: f64,
    pnl_comm: f64,
}

#[derive(Debug, Clone, Serialize)]
struct GapFlattenViolation {
    owner: String,
    reason: String,
    submitted_ts: String,
    fill_ts: String,
    submitted_weekday: String,
    fill_weekday: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SummaryRow {
    split: String,
    final_value: f64,
    total_return_pct: f64,
    annualized_sharpe: f64,
    max_drawdown_pct: f64,
    trade_count: usize,
    win_rate_pct: f64,
}

#[derive(Debug, Serialize)]
struct FirstDivergence {
    split: String,
    artifact: String,
    row_idx: usize,
    col: String,
    expected: String,
    actual: String,
}

#[derive(Debug, Serialize)]
struct ParityReport {
    bundle_dir: String,
    split: String,
    profile: ReplayProfile,
    weekend_policy: WeekendPolicy,
    verify_checksums_result: String,
    expected_actions: usize,
    actual_actions: usize,
    expected_trades: usize,
    actual_trades: usize,
    assert_gap_flatten: bool,
    gap_flatten_violations: usize,
    first_gap_flatten_violation: Option<GapFlattenViolation>,
    bo_gap_flatten_actions: usize,
    first_divergence: Option<FirstDivergence>,
    verdict: String,
}

#[derive(Debug, Clone)]
struct PendingEntry {
    submitted_ts: NaiveDateTime,
    owner: Owner,
    side: Side,
    stop_price: Option<f64>,
    take_price: Option<f64>,
    valid_until: Option<NaiveDateTime>,
    size: i64,
}

#[derive(Debug, Clone)]
struct PendingExit {
    submitted_ts: NaiveDateTime,
    owner: Owner,
    reason: String,
}

#[derive(Debug, Clone)]
struct Position {
    owner: Owner,
    side: Side,
    entry_ts: NaiveDateTime,
    entry_price: f64,
    size: i64,
    entry_fee: f64,
}

#[derive(Debug, Clone)]
struct ActiveBracket {
    owner: Owner,
    side: Side,
    stop_price: f64,
    take_price: f64,
    valid_until: NaiveDateTime,
}

#[derive(Debug, Clone)]
struct EngineState {
    cash: f64,
    commission: f64,
    equity_curve: Vec<(NaiveDateTime, f64)>,
    action_rows: Vec<ActionRow>,
    trade_rows: Vec<TradeRow>,
    gap_flatten_violations: Vec<GapFlattenViolation>,
    pending_entry: Option<PendingEntry>,
    pending_exit: Option<PendingExit>,
    position: Option<Position>,
    bracket: Option<ActiveBracket>,
    high180_open: Option<High180Open>,
}

impl EngineState {
    fn new(cash: f64, commission: f64) -> Self {
        Self {
            cash,
            commission,
            equity_curve: Vec::new(),
            action_rows: Vec::new(),
            trade_rows: Vec::new(),
            gap_flatten_violations: Vec::new(),
            pending_entry: None,
            pending_exit: None,
            position: None,
            bracket: None,
            high180_open: None,
        }
    }

    fn has_live_orders(&self) -> bool {
        self.pending_entry.is_some() || self.pending_exit.is_some() || self.bracket.is_some()
    }
}

fn main() -> Result<()> {
    let cli = parse_cli()?;
    if cli.verify_checksums {
        verify_checksums(&cli.bundle_dir)?;
    }
    fs::create_dir_all(&cli.out_dir).context("create out dir")?;

    let bars = read_prepared_bars(
        &cli.bundle_dir.join(cli.split.prepared_file()),
        cli.allow_non_strict,
    )?;
    if bars.is_empty() {
        bail!("prepared data is empty");
    }

    let mut state = EngineState::new(cli.cash, cli.commission);
    if cli.profile.uses_close_bar_high180_replay() {
        run_close_bar_high180_replay(&cli, &bars, &mut state)?;
    } else {
        let mut orchestrator = build_orchestrator(cli.profile);
        run_replay(&cli, &bars, &mut orchestrator, &mut state)?;
    }
    let summary = build_summary(cli.split, cli.cash, &bars, &state);

    write_outputs(&cli, &state.action_rows, &state.trade_rows, &summary)?;
    let parity = if cli.check && cli.split.has_expected() {
        compare_with_expected(&cli, &state.action_rows, &state.trade_rows, &summary)?
    } else {
        ParityReport {
            bundle_dir: cli.bundle_dir.display().to_string(),
            split: cli.split.as_str().to_string(),
            profile: cli.profile,
            weekend_policy: cli.weekend_policy,
            verify_checksums_result: if cli.verify_checksums {
                "ok".to_string()
            } else {
                "skipped".to_string()
            },
            expected_actions: 0,
            actual_actions: state.action_rows.len(),
            expected_trades: 0,
            actual_trades: state.trade_rows.len(),
            assert_gap_flatten: false,
            gap_flatten_violations: 0,
            first_gap_flatten_violation: None,
            bo_gap_flatten_actions: 0,
            first_divergence: None,
            verdict: "SKIPPED".to_string(),
        }
    };
    let parity = attach_gap_flatten_report(&cli, &state, parity);
    write_parity_report(&cli, &parity)?;

    match parity.verdict.as_str() {
        "PASS" | "SKIPPED" => Ok(()),
        "FAIL" => std::process::exit(2),
        _ => std::process::exit(1),
    }
}

fn parse_cli() -> Result<Cli> {
    let mut cli = Cli::default();
    let mut args = env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bundle-dir" => {
                let value = args.next().context("missing value for --bundle-dir")?;
                cli.bundle_dir = PathBuf::from(value);
            }
            "--split" => {
                let value = args.next().context("missing value for --split")?;
                cli.split = Split::parse(&value)?;
            }
            "--out-dir" => {
                let value = args.next().context("missing value for --out-dir")?;
                cli.out_dir = PathBuf::from(value);
            }
            "--profile" => {
                let value = args.next().context("missing value for --profile")?;
                cli.profile = ReplayProfile::parse(&value)?;
            }
            "--weekend-policy" => {
                let value = args.next().context("missing value for --weekend-policy")?;
                cli.weekend_policy = WeekendPolicy::parse(&value)?;
            }
            "--cash" => {
                let value = args.next().context("missing value for --cash")?;
                cli.cash = value.parse::<f64>().context("invalid --cash")?;
            }
            "--size-pct" => {
                let value = args.next().context("missing value for --size-pct")?;
                cli.size_pct = value.parse::<f64>().context("invalid --size-pct")?;
            }
            "--commission" => {
                let value = args.next().context("missing value for --commission")?;
                cli.commission = value.parse::<f64>().context("invalid --commission")?;
            }
            "--check" => cli.check = true,
            "--strict" => cli.strict = true,
            "--allow-non-strict" => cli.allow_non_strict = true,
            "--verify-checksums" => cli.verify_checksums = true,
            "--assert-gap-flatten" => cli.assert_gap_flatten = true,
            _ => bail!("unknown arg: {arg}"),
        }
    }
    Ok(cli)
}

fn build_orchestrator(profile: ReplayProfile) -> HybridOrchestrator {
    let mr = MeanReversionEngine::new(MeanReversionConfig {
        min_range_long: 0.013,
        max_range_long: 0.040,
        k_long: 0.032,
        take_k_long: 0.11,
        stop_k_long: 0.44,
        min_range_short: 0.010,
        max_range_short: 0.050,
        k_short: 0.055,
        take_k_short: 0.16,
        stop_k_short: 0.43,
        tick_size: 0.5,
        session_end_time: NaiveTime::from_hms_opt(11, 59, 0).unwrap_or(NaiveTime::MIN),
        exit_offset: chrono::Duration::minutes(5),
    });
    let br = IntradayBreakoutEngine::new(breakout_config(profile));
    HybridOrchestrator::new(
        mr,
        br,
        HybridOrchestratorConfig {
            breakout_eod_mode: BreakoutEodMode::SameDay,
            breakout_overnight_exit_time: NaiveTime::from_hms_opt(9, 30, 0)
                .unwrap_or(NaiveTime::MIN),
        },
    )
}

fn breakout_config(profile: ReplayProfile) -> IntradayBreakoutConfig {
    match profile {
        ReplayProfile::BaselineRuntimeHybrid => IntradayBreakoutConfig {
            k: 0.55,
            stop1_range: 0.51,
            stop2_range: 0.35,
            big_move_threshold: 0.025,
            min_range: 0.0,
            min_range_mode: MinRangeMode::Disabled,
            exclude_weekends: true,
            wait_hours: 3.0,
        },
        ReplayProfile::ImoexfBoK053 | ReplayProfile::ImoexfPrimaryRiskgateK053 => {
            IntradayBreakoutConfig {
                k: 0.53,
                stop1_range: 0.35,
                stop2_range: 0.70,
                big_move_threshold: 0.025,
                min_range: 1.01,
                min_range_mode: MinRangeMode::Absolute,
                exclude_weekends: true,
                wait_hours: 4.0,
            }
        }
    }
}

fn run_close_bar_high180_replay(
    cli: &Cli,
    bars: &[PreparedBar],
    state: &mut EngineState,
) -> Result<()> {
    let mr_enabled_dates = riskgate_high180_lb120_enabled_dates(bars);
    let mut mr = High180MrEngine::new(High180MrConfig::default());
    let mut breakout = IntradayBreakoutEngine::new(breakout_config(cli.profile));

    for bar in bars {
        if !is_regular_tradable_bar(bar.ts) {
            continue;
        }
        if bar.close == 0.0 {
            continue;
        }

        mr.on_bar(bar.ts, bar.high, bar.low);
        breakout.on_bar(bar.ts, bar.open, bar.high, bar.low, bar.close);

        if close_bar_high180_exit(bar, &mr, state) {
            state
                .equity_curve
                .push((bar.ts, mark_to_market_equity(state, bar.close)));
            continue;
        }
        if close_bar_breakout_exit(bar, &breakout, state) {
            state
                .equity_curve
                .push((bar.ts, mark_to_market_equity(state, bar.close)));
            continue;
        }
        if state.position.is_none() {
            if mr_enabled_dates.contains(&bar.ts.date()) {
                if let Some(signal) =
                    mr.evaluate_entry(bar.ts, bar.close, bar.close_prev, bar.day_range_prev)
                {
                    open_close_bar_entry(cli, bar, signal, Owner::MeanReversion, state);
                    state.equity_curve.push((bar.ts, state.cash));
                    continue;
                }
            }
            if let Some(signal) = breakout.evaluate_entry(bar.ts, bar.close) {
                open_breakout_close_bar_entry(cli, bar, signal.side, &mut breakout, state);
            }
        }
        state
            .equity_curve
            .push((bar.ts, mark_to_market_equity(state, bar.close)));
    }
    Ok(())
}

fn open_close_bar_entry(
    cli: &Cli,
    bar: &PreparedBar,
    signal: High180Signal,
    owner: Owner,
    state: &mut EngineState,
) {
    state.action_rows.push(ActionRow {
        bar_ts: format_ts(bar.ts),
        action: "submit_entry".to_string(),
        owner: owner.as_str().to_string(),
        side: signal.side.as_str().to_string(),
        reason: signal.reason.as_str().to_string(),
    });
    let size = close_bar_position_size(cli, state.cash, bar, signal.side);
    let entry_fee = (size.abs() as f64) * bar.close * state.commission;
    state.cash -= entry_fee;
    state.position = Some(Position {
        owner,
        side: signal.side,
        entry_ts: bar.ts,
        entry_price: bar.close,
        size,
        entry_fee,
    });
    state.high180_open = Some(High180Open::from_signal(
        &signal,
        High180MrConfig::default(),
    ));
}

fn open_breakout_close_bar_entry(
    cli: &Cli,
    bar: &PreparedBar,
    side: Side,
    breakout: &mut IntradayBreakoutEngine,
    state: &mut EngineState,
) {
    state.action_rows.push(ActionRow {
        bar_ts: format_ts(bar.ts),
        action: "submit_entry".to_string(),
        owner: Owner::IntradayBreakout.as_str().to_string(),
        side: side.as_str().to_string(),
        reason: match side {
            Side::Long => "breakout_long",
            Side::Short => "breakout_short",
        }
        .to_string(),
    });
    let size = close_bar_position_size(cli, state.cash, bar, side);
    let entry_fee = (size.abs() as f64) * bar.close * state.commission;
    state.cash -= entry_fee;
    state.position = Some(Position {
        owner: Owner::IntradayBreakout,
        side,
        entry_ts: bar.ts,
        entry_price: bar.close,
        size,
        entry_fee,
    });
    breakout.mark_entry(side);
}

fn close_bar_position_size(cli: &Cli, cash: f64, bar: &PreparedBar, side: Side) -> i64 {
    let mut size = (cash * cli.size_pct / bar.close) as i64;
    if size <= 0 {
        size = 1;
    }
    match side {
        Side::Long => size,
        Side::Short => -size,
    }
}

fn close_bar_high180_exit(
    bar: &PreparedBar,
    mr: &High180MrEngine,
    state: &mut EngineState,
) -> bool {
    let Some(position) = state.position.clone() else {
        return false;
    };
    if position.owner != Owner::MeanReversion {
        return false;
    }
    let Some(open) = state.high180_open.clone() else {
        return false;
    };
    let Some((reason, exit_price)) =
        mr.evaluate_exit(&open, position.entry_ts, position.side, bar.ts, bar.close)
    else {
        return false;
    };
    state.action_rows.push(ActionRow {
        bar_ts: format_ts(bar.ts),
        action: "submit_exit".to_string(),
        owner: Owner::MeanReversion.as_str().to_string(),
        side: String::new(),
        reason: reason.to_string(),
    });
    close_trade(bar.ts, exit_price, &position, reason, state);
    state.position = None;
    state.high180_open = None;
    true
}

fn close_bar_breakout_exit(
    bar: &PreparedBar,
    breakout: &IntradayBreakoutEngine,
    state: &mut EngineState,
) -> bool {
    let Some(position) = state.position.clone() else {
        return false;
    };
    if position.owner != Owner::IntradayBreakout {
        return false;
    }
    if bar.ts.date() != position.entry_ts.date() {
        state.action_rows.push(ActionRow {
            bar_ts: format_ts(bar.ts),
            action: "submit_exit".to_string(),
            owner: Owner::IntradayBreakout.as_str().to_string(),
            side: String::new(),
            reason: "bo_gap_flatten".to_string(),
        });
        close_trade(bar.ts, bar.open, &position, "bo_gap_flatten", state);
        state.position = None;
        return true;
    }
    let Some(exit) = breakout.evaluate_exit(bar.ts, bar.close, position.side) else {
        return false;
    };
    state.action_rows.push(ActionRow {
        bar_ts: format_ts(bar.ts),
        action: "submit_exit".to_string(),
        owner: Owner::IntradayBreakout.as_str().to_string(),
        side: String::new(),
        reason: exit.reason.as_str().to_string(),
    });
    close_trade(bar.ts, bar.close, &position, exit.reason.as_str(), state);
    state.position = None;
    true
}

fn riskgate_high180_lb120_enabled_dates(bars: &[PreparedBar]) -> BTreeSet<NaiveDate> {
    let mut daily_pnl = BTreeMap::<NaiveDate, f64>::new();
    for (exit_date, pnl) in simulate_high180_shadow_daily_pnl(bars) {
        *daily_pnl.entry(exit_date).or_insert(0.0) += pnl;
    }

    let mut date_set = BTreeSet::<NaiveDate>::new();
    for bar in bars {
        if is_regular_tradable_bar(bar.ts) {
            date_set.insert(bar.ts.date());
        }
    }
    let dates = date_set.into_iter().collect::<Vec<_>>();

    let mut enabled = BTreeSet::<NaiveDate>::new();
    for (idx, day) in dates.iter().copied().enumerate() {
        if idx < RISK_GATE_MIN_HISTORY_SESSIONS {
            continue;
        }
        let start = idx.saturating_sub(RISK_GATE_LOOKBACK_SESSIONS);
        let trailing_pnl: f64 = dates[start..idx]
            .iter()
            .map(|date| daily_pnl.get(date).copied().unwrap_or(0.0))
            .sum();
        if trailing_pnl > 0.0 {
            enabled.insert(day);
        }
    }
    enabled
}

fn simulate_high180_shadow_daily_pnl(bars: &[PreparedBar]) -> Vec<(NaiveDate, f64)> {
    let mut mr = High180MrEngine::new(High180MrConfig::default());
    let mut position: Option<Position> = None;
    let mut high180_open: Option<High180Open> = None;
    let mut pnl_by_exit = Vec::new();

    for bar in bars {
        if !is_regular_tradable_bar(bar.ts) || bar.close == 0.0 {
            continue;
        }
        mr.on_bar(bar.ts, bar.high, bar.low);
        let mut closed_this_bar = false;
        if let (Some(pos), Some(open)) = (position.clone(), high180_open.clone()) {
            if let Some((_reason, exit_price)) =
                mr.evaluate_exit(&open, pos.entry_ts, pos.side, bar.ts, bar.close)
            {
                let pnl = if pos.size > 0 {
                    exit_price - pos.entry_price
                } else {
                    pos.entry_price - exit_price
                } - RISK_GATE_MAKER_COST_POINTS;
                pnl_by_exit.push((bar.ts.date(), pnl));
                position = None;
                high180_open = None;
                closed_this_bar = true;
            }
        }
        if position.is_none() && !closed_this_bar {
            if let Some(signal) =
                mr.evaluate_entry(bar.ts, bar.close, bar.close_prev, bar.day_range_prev)
            {
                let size = match signal.side {
                    Side::Long => 1,
                    Side::Short => -1,
                };
                position = Some(Position {
                    owner: Owner::MeanReversion,
                    side: signal.side,
                    entry_ts: bar.ts,
                    entry_price: bar.close,
                    size,
                    entry_fee: 0.0,
                });
                high180_open = Some(High180Open::from_signal(
                    &signal,
                    High180MrConfig::default(),
                ));
            }
        }
    }
    pnl_by_exit
}

fn read_prepared_bars(path: &Path, allow_non_strict: bool) -> Result<Vec<PreparedBar>> {
    let mut rdr = ReaderBuilder::new()
        .from_path(path)
        .with_context(|| format!("open prepared file: {}", path.display()))?;
    let mut rows = Vec::new();
    let mut prev_ts: Option<NaiveDateTime> = None;
    for item in rdr.deserialize::<PreparedBarCsv>() {
        let row = item.with_context(|| format!("decode prepared row in {}", path.display()))?;
        let ts = parse_ts(&row.datetime)?;
        if let Some(prev) = prev_ts {
            if ts <= prev {
                if allow_non_strict {
                    continue;
                }
                bail!("non-monotonic timestamp: {ts} <= {prev}");
            }
        }
        for v in [row.open, row.high, row.low, row.close] {
            if !v.is_finite() {
                bail!("non-finite numeric value at {}", row.datetime);
            }
        }
        rows.push(PreparedBar {
            ts,
            open: row.open,
            high: row.high,
            low: row.low,
            close: row.close,
            close_prev: row.close_prev.unwrap_or(f64::NAN),
            day_range_prev: row.dayrangeprev.unwrap_or(f64::NAN),
        });
        prev_ts = Some(ts);
    }
    Ok(rows)
}

fn run_replay(
    cli: &Cli,
    bars: &[PreparedBar],
    orchestrator: &mut HybridOrchestrator,
    state: &mut EngineState,
) -> Result<()> {
    for bar in bars {
        let is_weekend = matches!(bar.ts.weekday(), Weekday::Sat | Weekday::Sun);
        if is_weekend && matches!(cli.weekend_policy, WeekendPolicy::BaselineSkip) {
            continue;
        }

        if state.pending_entry.is_some() {
            process_pending_entry(bar, orchestrator, state);
        }
        if state.pending_exit.is_some() {
            process_pending_exit(bar, orchestrator, state);
        }
        process_active_bracket(bar, orchestrator, state);

        if bar.close == 0.0 {
            continue;
        }

        if bar.close_prev.is_nan() || bar.day_range_prev.is_nan() {
            state
                .equity_curve
                .push((bar.ts, mark_to_market_equity(state, bar.close)));
            continue;
        }

        let actions = orchestrator.on_bar(
            strategy_runtime::strategies::hybrid_intraday::orchestrator::BarInput {
                dt: bar.ts,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                close_prev: bar.close_prev,
                day_range_prev: bar.day_range_prev,
                has_open_position: state.position.is_some(),
                has_live_orders: state.has_live_orders(),
            },
        );
        if is_weekend && matches!(cli.weekend_policy, WeekendPolicy::NoTradeButUpdate) {
            state
                .equity_curve
                .push((bar.ts, mark_to_market_equity(state, bar.close)));
            continue;
        }

        for action in actions {
            apply_action(cli, bar, action, orchestrator, state);
        }
        state
            .equity_curve
            .push((bar.ts, mark_to_market_equity(state, bar.close)));
    }

    if cli.strict && (state.pending_entry.is_some() || state.pending_exit.is_some()) {
        bail!("pending market order remains at end of dataset");
    }
    Ok(())
}

fn apply_action(
    cli: &Cli,
    bar: &PreparedBar,
    action: Action,
    _orchestrator: &mut HybridOrchestrator,
    state: &mut EngineState,
) {
    let (action_name, owner, side, reason) = match &action {
        Action::SubmitEntry(entry) => (
            "submit_entry",
            entry.owner.as_str().to_string(),
            entry.side.as_str().to_string(),
            entry.reason.as_str().to_string(),
        ),
        Action::SubmitExit { owner, reason } => (
            "submit_exit",
            owner.as_str().to_string(),
            String::new(),
            reason.as_str().to_string(),
        ),
        Action::ArmOvernightExit { owner, reason, .. } => (
            "arm_overnight_exit",
            owner.as_str().to_string(),
            String::new(),
            reason.as_str().to_string(),
        ),
    };
    state.action_rows.push(ActionRow {
        bar_ts: format_ts(bar.ts),
        action: action_name.to_string(),
        owner,
        side,
        reason,
    });

    match action {
        Action::SubmitEntry(entry) => {
            if state.pending_entry.is_some() || state.position.is_some() {
                return;
            }
            let mut size = (state.cash * cli.size_pct / bar.close) as i64;
            if size <= 0 {
                size = 1;
            }
            let signed_size = if entry.side == Side::Long {
                size
            } else {
                -size
            };
            let valid_until = if entry.entry_style.as_str() == "bracket" {
                Some(
                    bar.ts
                        .date()
                        .and_hms_opt(11, 59, 0)
                        .unwrap_or(NaiveDateTime::MIN),
                )
            } else {
                None
            };
            state.pending_entry = Some(PendingEntry {
                submitted_ts: bar.ts,
                owner: entry.owner,
                side: entry.side,
                stop_price: entry.stop_price,
                take_price: entry.take_price,
                valid_until,
                size: signed_size,
            });
        }
        Action::SubmitExit { owner, reason } => {
            if state.pending_exit.is_some() {
                return;
            }
            if owner == Owner::MeanReversion {
                state.bracket = None;
            }
            state.pending_exit = Some(PendingExit {
                submitted_ts: bar.ts,
                owner,
                reason: reason.as_str().to_string(),
            });
        }
        Action::ArmOvernightExit { .. } => {}
    }
}

fn process_pending_entry(
    bar: &PreparedBar,
    orchestrator: &mut HybridOrchestrator,
    state: &mut EngineState,
) {
    let Some(pending) = state.pending_entry.clone() else {
        return;
    };
    if pending.submitted_ts >= bar.ts {
        return;
    }
    let fill_price = bar.open;
    let entry_fee = (pending.size.abs() as f64) * fill_price * state.commission;
    state.cash -= entry_fee;
    state.position = Some(Position {
        owner: pending.owner,
        side: pending.side,
        entry_ts: bar.ts,
        entry_price: fill_price,
        size: pending.size,
        entry_fee,
    });
    state.pending_entry = None;
    if let (Some(stop), Some(take), Some(valid_until)) =
        (pending.stop_price, pending.take_price, pending.valid_until)
    {
        if bar.ts <= valid_until {
            state.bracket = Some(ActiveBracket {
                owner: pending.owner,
                side: pending.side,
                stop_price: stop,
                take_price: take,
                valid_until,
            });
        }
    }
    orchestrator.on_order_filled("entry", pending.owner, Some(pending.side));
}

fn process_pending_exit(
    bar: &PreparedBar,
    orchestrator: &mut HybridOrchestrator,
    state: &mut EngineState,
) {
    let Some(exit) = state.pending_exit.clone() else {
        return;
    };
    if exit.submitted_ts >= bar.ts {
        return;
    }
    let Some(position) = state.position.clone() else {
        state.pending_exit = None;
        return;
    };
    if let Some(violation) = gap_flatten_violation(&exit, bar) {
        state.gap_flatten_violations.push(violation);
    }
    close_trade(bar.ts, bar.open, &position, &exit.reason, state);
    state.pending_exit = None;
    state.position = None;
    state.bracket = None;
    orchestrator.on_order_filled("exit", exit.owner, Some(position.side));
}

fn gap_flatten_violation(exit: &PendingExit, bar: &PreparedBar) -> Option<GapFlattenViolation> {
    if exit.owner != Owner::IntradayBreakout || exit.reason != "breakout_eod_exit" {
        return None;
    }
    if exit.submitted_ts.date() == bar.ts.date() && !is_weekend_ts(bar.ts) {
        return None;
    }
    Some(GapFlattenViolation {
        owner: exit.owner.as_str().to_string(),
        reason: exit.reason.clone(),
        submitted_ts: format_ts(exit.submitted_ts),
        fill_ts: format_ts(bar.ts),
        submitted_weekday: format!("{:?}", exit.submitted_ts.weekday()),
        fill_weekday: format!("{:?}", bar.ts.weekday()),
    })
}

fn is_weekend_ts(ts: NaiveDateTime) -> bool {
    matches!(ts.weekday(), Weekday::Sat | Weekday::Sun)
}

fn is_regular_tradable_bar(ts: NaiveDateTime) -> bool {
    if is_weekend_ts(ts) {
        return false;
    }
    let start = NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN);
    let end = NaiveTime::from_hms_opt(23, 49, 59).expect("valid model session end time");
    ts.time() >= start && ts.time() <= end
}

fn process_active_bracket(
    bar: &PreparedBar,
    orchestrator: &mut HybridOrchestrator,
    state: &mut EngineState,
) {
    let Some(bracket) = state.bracket.clone() else {
        return;
    };
    let Some(position) = state.position.clone() else {
        state.bracket = None;
        return;
    };
    if bar.ts <= position.entry_ts {
        return;
    }
    if bar.ts > bracket.valid_until {
        state.bracket = None;
        return;
    }

    let hit = match bracket.side {
        Side::Long => {
            let stop_hit = if bar.open <= bracket.stop_price {
                Some((bar.open, "stop"))
            } else if bar.low <= bracket.stop_price {
                Some((bracket.stop_price, "stop"))
            } else {
                None
            };
            let take_hit = if bar.open >= bracket.take_price {
                Some((bar.open, "take"))
            } else if bar.high >= bracket.take_price {
                Some((bracket.take_price, "take"))
            } else {
                None
            };
            stop_hit.or(take_hit)
        }
        Side::Short => {
            let stop_hit = if bar.open >= bracket.stop_price {
                Some((bar.open, "stop"))
            } else if bar.high >= bracket.stop_price {
                Some((bracket.stop_price, "stop"))
            } else {
                None
            };
            let take_hit = if bar.open <= bracket.take_price {
                Some((bar.open, "take"))
            } else if bar.low <= bracket.take_price {
                Some((bracket.take_price, "take"))
            } else {
                None
            };
            stop_hit.or(take_hit)
        }
    };

    let Some((fill_price, role)) = hit else {
        return;
    };
    let reason = if role == "take" { "take" } else { "stop" };
    close_trade(bar.ts, fill_price, &position, reason, state);
    state.position = None;
    state.bracket = None;
    orchestrator.on_order_filled(role, bracket.owner, Some(position.side));
}

fn close_trade(
    ts: NaiveDateTime,
    exit_price: f64,
    position: &Position,
    reason: &str,
    state: &mut EngineState,
) {
    let size = position.size;
    let pnl = if size > 0 {
        (exit_price - position.entry_price) * size as f64
    } else {
        (position.entry_price - exit_price) * (-size) as f64
    };
    let exit_fee = (size.abs() as f64) * exit_price * state.commission;
    let fees = position.entry_fee + exit_fee;
    let pnl_comm = pnl - fees;
    state.cash += pnl - exit_fee;
    state.trade_rows.push(TradeRow {
        owner: position.owner.as_str().to_string(),
        side: position.side.as_str().to_string(),
        entry_ts: format_ts(position.entry_ts),
        exit_ts: format_ts(ts),
        entry_price: position.entry_price,
        exit_price,
        size,
        pnl,
        pnl_comm,
    });
    let _ = reason;
}

fn mark_to_market_equity(state: &EngineState, close_price: f64) -> f64 {
    if let Some(pos) = &state.position {
        let unrealized = if pos.size > 0 {
            (close_price - pos.entry_price) * pos.size as f64
        } else {
            (pos.entry_price - close_price) * (-pos.size) as f64
        };
        state.cash + unrealized
    } else {
        state.cash
    }
}

fn build_summary(
    split: Split,
    initial_cash: f64,
    bars: &[PreparedBar],
    state: &EngineState,
) -> SummaryRow {
    let mut all_days = BTreeSet::<NaiveDate>::new();
    for bar in bars {
        all_days.insert(bar.ts.date());
    }

    let mut by_day = BTreeMap::<NaiveDate, f64>::new();
    let mut max_equity = initial_cash;
    let mut max_dd = 0.0;
    for (ts, eq) in &state.equity_curve {
        by_day.insert(ts.date(), *eq);
        if *eq > max_equity {
            max_equity = *eq;
        }
        if max_equity > 0.0 {
            let dd = (max_equity - *eq) / max_equity * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    let mut daily_returns = Vec::new();
    let mut prev = initial_cash;
    for day in all_days {
        let equity = by_day.get(&day).copied().unwrap_or(prev);
        let ret = if prev != 0.0 {
            equity / prev - 1.0
        } else {
            0.0
        };
        daily_returns.push(ret);
        prev = equity;
    }
    let mean = if daily_returns.is_empty() {
        0.0
    } else {
        daily_returns.iter().sum::<f64>() / daily_returns.len() as f64
    };
    let var = if daily_returns.is_empty() {
        0.0
    } else {
        daily_returns
            .iter()
            .map(|r| (r - mean) * (r - mean))
            .sum::<f64>()
            / daily_returns.len() as f64
    };
    let std = var.sqrt();
    let sharpe = if std > 0.0 {
        mean / std * 252.0f64.sqrt()
    } else {
        0.0
    };
    let wins = state.trade_rows.iter().filter(|t| t.pnl > 0.0).count();
    let trade_count = state.trade_rows.len();
    let win_rate = if trade_count > 0 {
        wins as f64 / trade_count as f64 * 100.0
    } else {
        0.0
    };

    SummaryRow {
        split: split.as_str().to_string(),
        final_value: state.cash,
        total_return_pct: (state.cash / initial_cash - 1.0) * 100.0,
        annualized_sharpe: sharpe,
        max_drawdown_pct: max_dd,
        trade_count,
        win_rate_pct: win_rate,
    }
}

fn write_outputs(
    cli: &Cli,
    actions: &[ActionRow],
    trades: &[TradeRow],
    summary: &SummaryRow,
) -> Result<()> {
    let split = cli.split.as_str();
    let actions_path = cli.out_dir.join(format!("actual_actions_{split}.csv"));
    let trades_path = cli.out_dir.join(format!("actual_trades_{split}.csv"));
    let summary_path = cli.out_dir.join(format!("actual_summary_{split}.json"));

    let mut w_actions = WriterBuilder::new()
        .has_headers(false)
        .from_path(&actions_path)?;
    w_actions.write_record(["bar_ts", "action", "owner", "side", "reason"])?;
    for row in actions {
        w_actions.serialize(row)?;
    }
    w_actions.flush()?;

    let mut w_trades = WriterBuilder::new()
        .has_headers(false)
        .from_path(&trades_path)?;
    w_trades.write_record([
        "owner",
        "side",
        "entry_ts",
        "exit_ts",
        "entry_price",
        "exit_price",
        "size",
        "pnl",
        "pnl_comm",
    ])?;
    for row in trades {
        w_trades.serialize(row)?;
    }
    w_trades.flush()?;

    fs::write(summary_path, serde_json::to_string_pretty(summary)?)?;
    Ok(())
}

fn compare_with_expected(
    cli: &Cli,
    actions: &[ActionRow],
    trades: &[TradeRow],
    summary: &SummaryRow,
) -> Result<ParityReport> {
    let split = cli.split.as_str();
    let expected_actions_path = cli.bundle_dir.join(format!("expected_actions_{split}.csv"));
    let expected_trades_path = cli.bundle_dir.join(format!("expected_trades_{split}.csv"));
    let expected_summary_path = cli
        .bundle_dir
        .join(format!("expected_summary_{split}.json"));
    let expected_actions = read_actions(&expected_actions_path)?;
    let expected_trades = read_trades(&expected_trades_path)?;
    let expected_summary: SummaryRow = serde_json::from_str(
        &fs::read_to_string(&expected_summary_path)
            .with_context(|| format!("read {}", expected_summary_path.display()))?,
    )?;

    let mut first_divergence = None;
    if let Some(diff) = compare_actions(split, &expected_actions, actions) {
        first_divergence = Some(diff);
    } else if let Some(diff) = compare_trades(split, &expected_trades, trades) {
        first_divergence = Some(diff);
    } else if let Some(diff) = compare_summary(split, &expected_summary, summary) {
        first_divergence = Some(diff);
    }

    let verdict = if first_divergence.is_none() {
        "PASS"
    } else {
        "FAIL"
    };
    Ok(ParityReport {
        bundle_dir: cli.bundle_dir.display().to_string(),
        split: split.to_string(),
        profile: cli.profile,
        weekend_policy: cli.weekend_policy,
        verify_checksums_result: if cli.verify_checksums {
            "ok".to_string()
        } else {
            "skipped".to_string()
        },
        expected_actions: expected_actions.len(),
        actual_actions: actions.len(),
        expected_trades: expected_trades.len(),
        actual_trades: trades.len(),
        assert_gap_flatten: false,
        gap_flatten_violations: 0,
        first_gap_flatten_violation: None,
        bo_gap_flatten_actions: 0,
        first_divergence,
        verdict: verdict.to_string(),
    })
}

fn attach_gap_flatten_report(
    cli: &Cli,
    state: &EngineState,
    mut report: ParityReport,
) -> ParityReport {
    report.assert_gap_flatten = cli.assert_gap_flatten;
    report.gap_flatten_violations = state.gap_flatten_violations.len();
    report.first_gap_flatten_violation = state.gap_flatten_violations.first().cloned();
    report.bo_gap_flatten_actions = state
        .action_rows
        .iter()
        .filter(|row| row.reason == "bo_gap_flatten")
        .count();
    if cli.assert_gap_flatten && !state.gap_flatten_violations.is_empty() {
        report.verdict = "FAIL".to_string();
    }
    report
}

fn compare_actions(
    split: &str,
    expected: &[ActionRow],
    actual: &[ActionRow],
) -> Option<FirstDivergence> {
    if expected.len() != actual.len() {
        return Some(FirstDivergence {
            split: split.to_string(),
            artifact: "actions".to_string(),
            row_idx: 0,
            col: "len".to_string(),
            expected: expected.len().to_string(),
            actual: actual.len().to_string(),
        });
    }
    for (idx, (e, a)) in expected.iter().zip(actual.iter()).enumerate() {
        let checks = [
            ("bar_ts", e.bar_ts.as_str(), a.bar_ts.as_str()),
            ("action", e.action.as_str(), a.action.as_str()),
            ("owner", e.owner.as_str(), a.owner.as_str()),
            ("side", e.side.as_str(), a.side.as_str()),
            ("reason", e.reason.as_str(), a.reason.as_str()),
        ];
        for (col, ev, av) in checks {
            if ev != av {
                return Some(FirstDivergence {
                    split: split.to_string(),
                    artifact: "actions".to_string(),
                    row_idx: idx,
                    col: col.to_string(),
                    expected: ev.to_string(),
                    actual: av.to_string(),
                });
            }
        }
    }
    None
}

fn compare_trades(
    split: &str,
    expected: &[TradeRow],
    actual: &[TradeRow],
) -> Option<FirstDivergence> {
    if expected.len() != actual.len() {
        return Some(FirstDivergence {
            split: split.to_string(),
            artifact: "trades".to_string(),
            row_idx: 0,
            col: "len".to_string(),
            expected: expected.len().to_string(),
            actual: actual.len().to_string(),
        });
    }
    for (idx, (e, a)) in expected.iter().zip(actual.iter()).enumerate() {
        let strict_checks = [
            ("owner", e.owner.as_str(), a.owner.as_str()),
            ("side", e.side.as_str(), a.side.as_str()),
            ("entry_ts", e.entry_ts.as_str(), a.entry_ts.as_str()),
            ("exit_ts", e.exit_ts.as_str(), a.exit_ts.as_str()),
        ];
        for (col, ev, av) in strict_checks {
            if ev != av {
                return Some(FirstDivergence {
                    split: split.to_string(),
                    artifact: "trades".to_string(),
                    row_idx: idx,
                    col: col.to_string(),
                    expected: ev.to_string(),
                    actual: av.to_string(),
                });
            }
        }
        if e.size != a.size {
            return Some(FirstDivergence {
                split: split.to_string(),
                artifact: "trades".to_string(),
                row_idx: idx,
                col: "size".to_string(),
                expected: e.size.to_string(),
                actual: a.size.to_string(),
            });
        }
        for (col, ev, av) in [
            ("entry_price", e.entry_price, a.entry_price),
            ("exit_price", e.exit_price, a.exit_price),
            ("pnl", e.pnl, a.pnl),
            ("pnl_comm", e.pnl_comm, a.pnl_comm),
        ] {
            if (ev - av).abs() > NUMERIC_EPS {
                return Some(FirstDivergence {
                    split: split.to_string(),
                    artifact: "trades".to_string(),
                    row_idx: idx,
                    col: col.to_string(),
                    expected: ev.to_string(),
                    actual: av.to_string(),
                });
            }
        }
    }
    None
}

fn compare_summary(
    split: &str,
    expected: &SummaryRow,
    actual: &SummaryRow,
) -> Option<FirstDivergence> {
    if expected.split != actual.split {
        return Some(diff_summary(split, "split", &expected.split, &actual.split));
    }
    if expected.trade_count != actual.trade_count {
        return Some(diff_summary(
            split,
            "trade_count",
            &expected.trade_count.to_string(),
            &actual.trade_count.to_string(),
        ));
    }
    for (col, ev, av) in [
        ("final_value", expected.final_value, actual.final_value),
        (
            "total_return_pct",
            expected.total_return_pct,
            actual.total_return_pct,
        ),
        (
            "annualized_sharpe",
            expected.annualized_sharpe,
            actual.annualized_sharpe,
        ),
        (
            "max_drawdown_pct",
            expected.max_drawdown_pct,
            actual.max_drawdown_pct,
        ),
        ("win_rate_pct", expected.win_rate_pct, actual.win_rate_pct),
    ] {
        if (ev - av).abs() > SUMMARY_EPS {
            return Some(diff_summary(split, col, &ev.to_string(), &av.to_string()));
        }
    }
    None
}

fn diff_summary(split: &str, col: &str, expected: &str, actual: &str) -> FirstDivergence {
    FirstDivergence {
        split: split.to_string(),
        artifact: "summary".to_string(),
        row_idx: 0,
        col: col.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    }
}

fn write_parity_report(cli: &Cli, report: &ParityReport) -> Result<()> {
    let path = cli
        .out_dir
        .join(format!("parity_report_{}.json", cli.split.as_str()));
    fs::write(path, serde_json::to_string_pretty(report)?)?;
    Ok(())
}

fn read_actions(path: &Path) -> Result<Vec<ActionRow>> {
    let mut rdr = ReaderBuilder::new().from_path(path)?;
    let mut rows = Vec::new();
    for row in rdr.deserialize::<ActionRow>() {
        rows.push(row?);
    }
    Ok(rows)
}

fn read_trades(path: &Path) -> Result<Vec<TradeRow>> {
    let mut rdr = ReaderBuilder::new().from_path(path)?;
    let mut rows = Vec::new();
    for row in rdr.deserialize::<TradeRow>() {
        rows.push(row?);
    }
    Ok(rows)
}

fn verify_checksums(bundle_dir: &Path) -> Result<()> {
    let checksums_path = bundle_dir.join("checksums.sha256");
    let file = File::open(&checksums_path)
        .with_context(|| format!("open {}", checksums_path.display()))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let expected = parts.next().context("missing digest")?;
        let name = parts.next().context("missing file")?;
        let p = bundle_dir.join(name);
        let bytes = fs::read(&p).with_context(|| format!("read {}", p.display()))?;
        let actual = sha256_hex(&bytes);
        if actual != expected {
            bail!("checksum mismatch for {name}: {actual} != {expected}");
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn parse_ts(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid datetime: {value}"))
}

fn format_ts(ts: NaiveDateTime) -> String {
    ts.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar_at(hour: u32, minute: u32, high: f64, low: f64, close: f64) -> PreparedBar {
        PreparedBar {
            ts: NaiveDate::from_ymd_opt(2026, 1, 5)
                .unwrap()
                .and_hms_opt(hour, minute, 0)
                .unwrap(),
            open: close,
            high,
            low,
            close,
            close_prev: 100.0,
            day_range_prev: 50.0,
        }
    }

    #[test]
    fn high180_long_requires_midpoint_above_entry_close() {
        let mut engine = High180MrEngine::new(High180MrConfig::default());
        let bar = bar_at(9, 0, 100.0, 90.0, 99.0);
        engine.on_bar(bar.ts, bar.high, bar.low);

        assert!(engine
            .evaluate_entry(bar.ts, bar.close, bar.close_prev, bar.day_range_prev)
            .is_none());
    }

    #[test]
    fn high180_short_requires_midpoint_below_entry_close() {
        let mut engine = High180MrEngine::new(High180MrConfig::default());
        let bar = bar_at(9, 0, 110.0, 100.0, 101.0);
        engine.on_bar(bar.ts, bar.high, bar.low);

        assert!(engine
            .evaluate_entry(bar.ts, bar.close, bar.close_prev, bar.day_range_prev)
            .is_none());
    }

    #[test]
    fn model_feed_excludes_service_and_weekend_bars() {
        let weekday_preopen = NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap()
            .and_hms_opt(8, 50, 0)
            .unwrap();
        let weekday_regular = NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        let weekend_regular = NaiveDate::from_ymd_opt(2026, 1, 10)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();

        assert!(!is_regular_tradable_bar(weekday_preopen));
        assert!(is_regular_tradable_bar(weekday_regular));
        assert!(!is_regular_tradable_bar(weekend_regular));
    }
}
