use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use strategy_runtime::strategies::hybrid_intraday::{
    Action, BreakoutEodMode, HybridOrchestrator, HybridOrchestratorConfig, IntradayBreakoutConfig,
    IntradayBreakoutEngine, MeanReversionConfig, MeanReversionEngine, MinRangeMode, Owner, Side,
};

const DEFAULT_BUNDLE_DIR: &str = "pre_rust_handoff/replay_data/imoexf_2023_2026";
const DEFAULT_OUT_DIR: &str = "./tmp/hybrid_out";
const DEFAULT_CASH: f64 = 100_000.0;
const DEFAULT_SIZE_PCT: f64 = 0.9;
const DEFAULT_COMMISSION: f64 = 1.0 / 28_000.0;
const NUMERIC_EPS: f64 = 1e-9;
const SUMMARY_EPS: f64 = 1e-4;

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

#[derive(Debug)]
struct Cli {
    bundle_dir: PathBuf,
    split: Split,
    out_dir: PathBuf,
    check: bool,
    strict: bool,
    allow_non_strict: bool,
    verify_checksums: bool,
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
            check: false,
            strict: false,
            allow_non_strict: false,
            verify_checksums: false,
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
    close_prev: f64,
    dayrangeprev: f64,
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
    weekend_policy: WeekendPolicy,
    verify_checksums_result: String,
    expected_actions: usize,
    actual_actions: usize,
    expected_trades: usize,
    actual_trades: usize,
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
    pending_entry: Option<PendingEntry>,
    pending_exit: Option<PendingExit>,
    position: Option<Position>,
    bracket: Option<ActiveBracket>,
}

impl EngineState {
    fn new(cash: f64, commission: f64) -> Self {
        Self {
            cash,
            commission,
            equity_curve: Vec::new(),
            action_rows: Vec::new(),
            trade_rows: Vec::new(),
            pending_entry: None,
            pending_exit: None,
            position: None,
            bracket: None,
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

    let mut orchestrator = build_orchestrator();
    let mut state = EngineState::new(cli.cash, cli.commission);
    run_replay(&cli, &bars, &mut orchestrator, &mut state)?;
    let summary = build_summary(cli.split, cli.cash, &state);

    write_outputs(&cli, &state.action_rows, &state.trade_rows, &summary)?;
    let parity = if cli.check && cli.split.has_expected() {
        compare_with_expected(&cli, &state.action_rows, &state.trade_rows, &summary)?
    } else {
        ParityReport {
            bundle_dir: cli.bundle_dir.display().to_string(),
            split: cli.split.as_str().to_string(),
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
            first_divergence: None,
            verdict: "SKIPPED".to_string(),
        }
    };
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
            _ => bail!("unknown arg: {arg}"),
        }
    }
    Ok(cli)
}

fn build_orchestrator() -> HybridOrchestrator {
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
    let br = IntradayBreakoutEngine::new(IntradayBreakoutConfig {
        k: 0.55,
        stop1_range: 0.51,
        stop2_range: 0.35,
        big_move_threshold: 0.025,
        min_range: 0.0,
        min_range_mode: MinRangeMode::Disabled,
        exclude_weekends: true,
        wait_hours: 3.0,
    });
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
        for v in [row.open, row.high, row.low, row.close, row.close_prev, row.dayrangeprev] {
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
            close_prev: row.close_prev,
            day_range_prev: row.dayrangeprev,
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
        if state.pending_entry.is_some() {
            process_pending_entry(bar, orchestrator, state);
        }
        if state.pending_exit.is_some() {
            process_pending_exit(bar, orchestrator, state);
        }
        process_active_bracket(bar, orchestrator, state);

        let is_weekend = matches!(bar.ts.weekday(), Weekday::Sat | Weekday::Sun);
        if is_weekend && matches!(cli.weekend_policy, WeekendPolicy::BaselineSkip) {
            continue;
        }

        let actions = orchestrator.on_bar(strategy_runtime::strategies::hybrid_intraday::orchestrator::BarInput {
            dt: bar.ts,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            close_prev: bar.close_prev,
            day_range_prev: bar.day_range_prev,
            has_open_position: state.position.is_some(),
            has_live_orders: state.has_live_orders(),
        });
        if is_weekend && matches!(cli.weekend_policy, WeekendPolicy::NoTradeButUpdate) {
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
            let signed_size = if entry.side == Side::Long { size } else { -size };
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
    if let Some(valid_until) = pending.valid_until {
        if bar.ts > valid_until {
            orchestrator.on_order_rejected("entry");
            state.pending_entry = None;
            return;
        }
    }

    let fill_price = bar.open;
    state.position = Some(Position {
        owner: pending.owner,
        side: pending.side,
        entry_ts: bar.ts,
        entry_price: fill_price,
        size: pending.size,
    });
    state.pending_entry = None;
    if let (Some(stop), Some(take), Some(valid_until)) =
        (pending.stop_price, pending.take_price, pending.valid_until)
    {
        state.bracket = Some(ActiveBracket {
            owner: pending.owner,
            side: pending.side,
            stop_price: stop,
            take_price: take,
            valid_until,
        });
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
    close_trade(
        bar.ts,
        bar.open,
        &position,
        &exit.reason,
        state,
    );
    state.pending_exit = None;
    state.position = None;
    state.bracket = None;
    orchestrator.on_order_filled("exit", exit.owner, Some(position.side));
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
    let reason = if role == "take" {
        "take"
    } else {
        "stop"
    };
    close_trade(bar.ts, fill_price, &position, reason, state);
    state.position = None;
    state.bracket = None;
    orchestrator.on_order_filled(role, bracket.owner, Some(position.side));
}

fn close_trade(ts: NaiveDateTime, exit_price: f64, position: &Position, reason: &str, state: &mut EngineState) {
    let size = position.size;
    let pnl = if size > 0 {
        (exit_price - position.entry_price) * size as f64
    } else {
        (position.entry_price - exit_price) * (-size) as f64
    };
    let fees =
        ((size.abs() as f64) * position.entry_price + (size.abs() as f64) * exit_price) * state.commission;
    let pnl_comm = pnl - fees;
    state.cash += pnl_comm;
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

fn build_summary(split: Split, initial_cash: f64, state: &EngineState) -> SummaryRow {
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
    if let (Some(first_day), Some(last_day)) = (by_day.keys().next().copied(), by_day.keys().last().copied())
    {
        let mut day = first_day;
        while day <= last_day {
            let equity = by_day.get(&day).copied().unwrap_or(prev);
            let ret = if prev != 0.0 { equity / prev - 1.0 } else { 0.0 };
            daily_returns.push(ret);
            prev = equity;
            if day == last_day {
                break;
            }
            day = day.succ_opt().unwrap_or(day);
        }
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

fn write_outputs(cli: &Cli, actions: &[ActionRow], trades: &[TradeRow], summary: &SummaryRow) -> Result<()> {
    let split = cli.split.as_str();
    let actions_path = cli.out_dir.join(format!("actual_actions_{split}.csv"));
    let trades_path = cli.out_dir.join(format!("actual_trades_{split}.csv"));
    let summary_path = cli.out_dir.join(format!("actual_summary_{split}.json"));

    let mut w_actions = WriterBuilder::new().has_headers(false).from_path(&actions_path)?;
    w_actions.write_record(["bar_ts", "action", "owner", "side", "reason"])?;
    for row in actions {
        w_actions.serialize(row)?;
    }
    w_actions.flush()?;

    let mut w_trades = WriterBuilder::new().has_headers(false).from_path(&trades_path)?;
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
    let expected_summary_path = cli.bundle_dir.join(format!("expected_summary_{split}.json"));
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
        first_divergence,
        verdict: verdict.to_string(),
    })
}

fn compare_actions(split: &str, expected: &[ActionRow], actual: &[ActionRow]) -> Option<FirstDivergence> {
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

fn compare_trades(split: &str, expected: &[TradeRow], actual: &[TradeRow]) -> Option<FirstDivergence> {
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

fn compare_summary(split: &str, expected: &SummaryRow, actual: &SummaryRow) -> Option<FirstDivergence> {
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
        ("total_return_pct", expected.total_return_pct, actual.total_return_pct),
        ("annualized_sharpe", expected.annualized_sharpe, actual.annualized_sharpe),
        ("max_drawdown_pct", expected.max_drawdown_pct, actual.max_drawdown_pct),
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
            bail!("checksum mismatch for {}: {} != {}", name, actual, expected);
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    out.iter().map(|b| format!("{:02x}", b)).collect::<String>()
}

fn parse_ts(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid datetime: {value}"))
}

fn format_ts(ts: NaiveDateTime) -> String {
    ts.format("%Y-%m-%d %H:%M:%S").to_string()
}
