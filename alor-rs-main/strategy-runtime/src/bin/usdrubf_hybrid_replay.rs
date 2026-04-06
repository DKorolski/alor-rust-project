use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};

const DEFAULT_BUNDLE_DIR: &str = "../../moex_usdrubf_pre_rust_handoff/replay_data/usdrubf_2023_2026";
const DEFAULT_OUT_DIR: &str = "./tmp/usdrubf_hybrid_out";
const STRATEGY_ID: &str = "alor_usdrubf_hybrid_v1";
const INITIAL_CASH: f64 = 100_000.0;
const COMMISSION: f64 = 0.004 / 100.0;
const POSITION_SIZE_FRACTION: f64 = 0.9;
const MR_MIN_REL_RANGE: f64 = 0.006;
const MR_MAX_REL_RANGE: f64 = 0.050;
const MR_K_SHORT: f64 = 0.045;
const MR_TAKE_K_SHORT: f64 = 0.16;
const MR_STOP_K_SHORT: f64 = 0.43;
const MR_TICK_SIZE: f64 = 0.0025;
const MR_LAST_ENTRY_HOUR: u32 = 11;
const MR_LAST_ENTRY_MINUTE: u32 = 40;
const MR_FORCE_EXIT_HOUR: u32 = 11;
const MR_FORCE_EXIT_MINUTE: u32 = 50;
const BO_K: f64 = 0.45;
const BO_STOP1_RANGE: f64 = 0.51;
const BO_STOP2_RANGE: f64 = 0.35;
const BO_BIG_MOVE_THRESHOLD: f64 = 0.020;
const BO_WAIT_HOURS: f64 = 2.0;
const BO_EOD_EXIT_HOUR: u32 = 23;
const BO_EOD_EXIT_MINUTE: u32 = 30;
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    Golden,
    Test,
    Train,
}

impl Split {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "golden" => Ok(Self::Golden),
            "test" => Ok(Self::Test),
            "train" => Ok(Self::Train),
            other => bail!("unsupported split: {other}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Golden => "golden",
            Self::Test => "test",
            Self::Train => "train",
        }
    }

    fn prepared_csv(self) -> &'static str {
        match self {
            Self::Golden => "prepared_golden.csv",
            Self::Test => "prepared_test.csv",
            Self::Train => "prepared_train.csv",
        }
    }
}

#[derive(Debug)]
struct Cli {
    bundle_dir: PathBuf,
    out_dir: PathBuf,
    split: Split,
    check: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            bundle_dir: PathBuf::from(DEFAULT_BUNDLE_DIR),
            out_dir: PathBuf::from(DEFAULT_OUT_DIR),
            split: Split::Golden,
            check: false,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReplayOwner {
    MeanRev,
    DayBreakoutWaitfix,
    Hybrid,
}

impl ReplayOwner {
    fn as_expected(self) -> &'static str {
        match self {
            Self::MeanRev => "mean_rev",
            Self::DayBreakoutWaitfix => "day_breakout_waitfix",
            Self::Hybrid => "hybrid",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReplaySide {
    Long,
    Short,
    None,
}

impl ReplaySide {
    fn as_expected(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HybridState {
    Flat,
    Pending,
    Open,
}

impl HybridState {
    fn as_expected(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Pending => "pending",
            Self::Open => "open",
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedBar {
    datetime: NaiveDateTime,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    session_vwap: f64,
    session_open: f64,
    session_range: f64,
    elapsed_hours: f64,
    ret_from_open: f64,
}

#[derive(Debug, Deserialize)]
struct PreparedBarCsv {
    datetime: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    #[serde(rename = "volume")]
    _volume: f64,
    session_vwap: f64,
    session_open: f64,
    session_range: f64,
    elapsed_hours: f64,
    ret_from_open: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct PendingEntry {
    owner: ReplayOwner,
    side: ReplaySide,
    reason: String,
    signal_ts: NaiveDateTime,
    scale_at_signal: f64,
    signal_price: f64,
    stop1: Option<f64>,
    stop2: Option<f64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct OpenPosition {
    owner: ReplayOwner,
    side: ReplaySide,
    entry_ts: NaiveDateTime,
    entry_price: f64,
    size: i64,
    scale_at_signal: f64,
    stop_price: Option<f64>,
    take_price: Option<f64>,
    stop1: Option<f64>,
    stop2: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ReplayAction {
    bar_ts: String,
    action: String,
    owner: String,
    side: String,
    reason: String,
    state_before: String,
    state_after: String,
    ref_price: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ReplayTrade {
    trade_id: usize,
    owner: String,
    side: String,
    entry_ts: String,
    exit_ts: String,
    entry_price: f64,
    exit_price: f64,
    size: i64,
    pnl_cash: f64,
    return_pct_on_cash: f64,
    exit_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplaySummary {
    final_value: f64,
    total_return_pct: f64,
    annualized_sharpe: f64,
    max_drawdown_pct: f64,
    trade_count: usize,
    active_days: usize,
    no_trade_days: usize,
    mr_trade_count: usize,
    bo_trade_count: usize,
    start_date: String,
    end_date: String,
    split: String,
    source_prepared_csv: String,
}

#[derive(Debug, Clone)]
struct ReplayState {
    hybrid_state: HybridState,
    pending_entry: Option<PendingEntry>,
    open_position: Option<OpenPosition>,
    actions: Vec<ReplayAction>,
    trades: Vec<ReplayTrade>,
    cash: f64,
    current_date: Option<NaiveDate>,
    day_start_cash: f64,
    day_pnl: f64,
    daily_rows: Vec<DailyRow>,
    bo_was_long_today: bool,
    bo_was_short_today: bool,
}

#[derive(Debug, Clone)]
struct DailyRow {
    pnl: f64,
    day_start_cash: f64,
    equity: f64,
}

#[derive(Debug, Deserialize)]
struct ExpectedActionRow {
    bar_ts: String,
    action: String,
    owner: String,
    side: String,
    reason: String,
    state_before: String,
    state_after: String,
    ref_price: f64,
}

#[derive(Debug, Deserialize)]
struct ExpectedTradeRow {
    trade_id: usize,
    owner: String,
    side: String,
    entry_ts: String,
    exit_ts: String,
    entry_price: f64,
    exit_price: f64,
    size: i64,
    pnl_cash: f64,
    return_pct_on_cash: f64,
    exit_reason: String,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            hybrid_state: HybridState::Flat,
            pending_entry: None,
            open_position: None,
            actions: Vec::new(),
            trades: Vec::new(),
            cash: INITIAL_CASH,
            current_date: None,
            day_start_cash: INITIAL_CASH,
            day_pnl: 0.0,
            daily_rows: Vec::new(),
            bo_was_long_today: false,
            bo_was_short_today: false,
        }
    }
}

fn main() -> Result<()> {
    let cli = parse_cli()?;
    fs::create_dir_all(&cli.out_dir).context("create output directory")?;
    let prepared_path = cli.bundle_dir.join(cli.split.prepared_csv());
    let bars = read_prepared_bars(&prepared_path)?;
    if bars.is_empty() {
        bail!("prepared CSV is empty: {}", prepared_path.display());
    }

    let mut state = ReplayState::new();
    run_replay(&bars, &mut state);
    let summary = build_summary(cli.split, cli.split.prepared_csv(), &bars, &state);

    write_outputs(&cli.out_dir, cli.split, &state.actions, &state.trades, &summary)?;
    if cli.check {
        check_expected(&cli.bundle_dir, cli.split, &state.actions, &state.trades, &summary)?;
    }

    println!(
        "strategy_id={STRATEGY_ID} split={} bars={} actions={} trades={}",
        cli.split.as_str(),
        bars.len(),
        state.actions.len(),
        state.trades.len()
    );
    Ok(())
}

fn parse_cli() -> Result<Cli> {
    let mut cli = Cli::default();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bundle-dir" => {
                let value = args.next().context("missing value for --bundle-dir")?;
                cli.bundle_dir = PathBuf::from(value);
            }
            "--out-dir" => {
                let value = args.next().context("missing value for --out-dir")?;
                cli.out_dir = PathBuf::from(value);
            }
            "--split" => {
                let value = args.next().context("missing value for --split")?;
                cli.split = Split::parse(&value)?;
            }
            "--check" => cli.check = true,
            _ => bail!("unknown argument: {arg}"),
        }
    }
    Ok(cli)
}

fn read_prepared_bars(path: &Path) -> Result<Vec<PreparedBar>> {
    let mut rdr = ReaderBuilder::new()
        .from_path(path)
        .with_context(|| format!("open prepared CSV: {}", path.display()))?;
    let mut out = Vec::new();
    let mut prev_ts: Option<NaiveDateTime> = None;
    for row in rdr.deserialize::<PreparedBarCsv>() {
        let row = row.with_context(|| format!("decode row in {}", path.display()))?;
        let dt = NaiveDateTime::parse_from_str(&row.datetime, "%Y-%m-%d %H:%M:%S")
            .with_context(|| format!("bad datetime: {}", row.datetime))?;
        if let Some(prev) = prev_ts {
            if dt <= prev {
                bail!("non-monotonic datetime: {} <= {}", row.datetime, prev);
            }
        }
        prev_ts = Some(dt);
        out.push(PreparedBar {
            datetime: dt,
            open: row.open,
            high: row.high,
            low: row.low,
            close: row.close,
            session_vwap: row.session_vwap,
            session_open: row.session_open,
            session_range: row.session_range,
            elapsed_hours: row.elapsed_hours,
            ret_from_open: row.ret_from_open,
        });
    }
    Ok(out)
}

fn run_replay(bars: &[PreparedBar], state: &mut ReplayState) {
    for (idx, bar) in bars.iter().enumerate() {
        let bar_date = bar.datetime.date();
        if state.current_date != Some(bar_date) {
            if state.current_date.is_some() {
                state.daily_rows.push(DailyRow {
                    pnl: state.day_pnl,
                    day_start_cash: state.day_start_cash,
                    equity: state.cash,
                });
            }
            state.current_date = Some(bar_date);
            state.day_start_cash = state.cash;
            state.day_pnl = 0.0;
            state.pending_entry = None;
            state.open_position = None;
            state.hybrid_state = HybridState::Flat;
            state.bo_was_long_today = false;
            state.bo_was_short_today = false;
        }

        fill_pending_entry(bar, state);

        if evaluate_and_apply_exit(bar, state) {
            continue;
        }

        if state.open_position.is_none() && state.pending_entry.is_none() && idx + 1 < bars.len() {
            let next_date = bars[idx + 1].datetime.date();
            if next_date != bar_date {
                state.actions.push(ReplayAction {
                    bar_ts: format_ts_iso(bar.datetime),
                    action: "session_no_trade".to_string(),
                    owner: ReplayOwner::Hybrid.as_expected().to_string(),
                    side: ReplaySide::None.as_expected().to_string(),
                    reason: "window_closed_without_signal".to_string(),
                    state_before: HybridState::Flat.as_expected().to_string(),
                    state_after: HybridState::Flat.as_expected().to_string(),
                    ref_price: 0.0,
                });
                continue;
            }
        }

        if state.open_position.is_none() && state.pending_entry.is_none() {
            let signal = evaluate_mr_signal(bar).or_else(|| evaluate_bo_signal(bar, state));
            if let Some(sig) = signal {
                state.pending_entry = Some(sig.clone());
                let before = state.hybrid_state;
                state.hybrid_state = HybridState::Pending;
                state.actions.push(ReplayAction {
                    bar_ts: format_ts_iso(bar.datetime),
                    action: "entry_signal".to_string(),
                    owner: sig.owner.as_expected().to_string(),
                    side: sig.side.as_expected().to_string(),
                    reason: sig.reason.clone(),
                    state_before: before.as_expected().to_string(),
                    state_after: state.hybrid_state.as_expected().to_string(),
                    ref_price: sig.signal_price,
                });
                if matches!(sig.owner, ReplayOwner::DayBreakoutWaitfix) {
                    if matches!(sig.side, ReplaySide::Long) {
                        state.bo_was_long_today = true;
                    } else if matches!(sig.side, ReplaySide::Short) {
                        state.bo_was_short_today = true;
                    }
                }
            }
        }
    }

    if state.current_date.is_some() {
        state.daily_rows.push(DailyRow {
            pnl: state.day_pnl,
            day_start_cash: state.day_start_cash,
            equity: state.cash,
        });
    }
}

fn build_summary(split: Split, source_prepared_csv: &str, bars: &[PreparedBar], state: &ReplayState) -> ReplaySummary {
    let start_date = bars
        .first()
        .map(|b| b.datetime.date().to_string())
        .unwrap_or_else(|| "1970-01-01".to_string());
    let end_date = bars
        .last()
        .map(|b| b.datetime.date().to_string())
        .unwrap_or_else(|| "1970-01-01".to_string());

    let active_days = state.daily_rows.iter().filter(|r| r.pnl != 0.0).count();
    let no_trade_days = state
        .actions
        .iter()
        .filter(|a| a.action == "session_no_trade")
        .count();
    let mr_trade_count = state
        .trades
        .iter()
        .filter(|t| t.owner == "mean_rev")
        .count();
    let bo_trade_count = state
        .trades
        .iter()
        .filter(|t| t.owner == "day_breakout_waitfix")
        .count();
    let final_value = state.cash;
    let total_return_pct = (final_value / INITIAL_CASH - 1.0) * 100.0;
    let daily_returns = state
        .daily_rows
        .iter()
        .map(|r| {
            if r.day_start_cash == 0.0 {
                0.0
            } else {
                r.pnl / r.day_start_cash
            }
        })
        .collect::<Vec<_>>();
    let annualized_sharpe = annualized_sharpe(&daily_returns);
    let max_drawdown_pct = max_drawdown_pct(
        &state
            .daily_rows
            .iter()
            .map(|r| r.equity)
            .collect::<Vec<_>>(),
    );

    ReplaySummary {
        final_value,
        total_return_pct,
        annualized_sharpe,
        max_drawdown_pct,
        trade_count: state.trades.len(),
        active_days,
        no_trade_days,
        mr_trade_count,
        bo_trade_count,
        start_date,
        end_date,
        split: split.as_str().to_string(),
        source_prepared_csv: source_prepared_csv.to_string(),
    }
}

fn write_outputs(
    out_dir: &Path,
    split: Split,
    actions: &[ReplayAction],
    trades: &[ReplayTrade],
    summary: &ReplaySummary,
) -> Result<()> {
    let split_name = split.as_str();
    let actions_path = out_dir.join(format!("actual_actions_{split_name}.csv"));
    let trades_path = out_dir.join(format!("actual_trades_{split_name}.csv"));
    let summary_path = out_dir.join(format!("actual_summary_{split_name}.json"));

    let mut actions_writer = WriterBuilder::new().from_path(actions_path)?;
    actions_writer.write_record([
        "bar_ts",
        "action",
        "owner",
        "side",
        "reason",
        "state_before",
        "state_after",
        "ref_price",
    ])?;
    for row in actions {
        actions_writer.serialize(row)?;
    }
    actions_writer.flush()?;

    let mut trades_writer = WriterBuilder::new().from_path(trades_path)?;
    trades_writer.write_record([
        "trade_id",
        "owner",
        "side",
        "entry_ts",
        "exit_ts",
        "entry_price",
        "exit_price",
        "size",
        "pnl_cash",
        "return_pct_on_cash",
        "exit_reason",
    ])?;
    for row in trades {
        trades_writer.serialize(row)?;
    }
    trades_writer.flush()?;

    fs::write(summary_path, serde_json::to_string_pretty(summary)?)?;
    Ok(())
}

fn check_expected(
    bundle_dir: &Path,
    split: Split,
    actions: &[ReplayAction],
    trades: &[ReplayTrade],
    summary: &ReplaySummary,
) -> Result<()> {
    let split_name = split.as_str();
    let expected_actions = bundle_dir.join(format!("expected_actions_{split_name}.csv"));
    let expected_trades = bundle_dir.join(format!("expected_trades_{split_name}.csv"));
    let expected_summary = bundle_dir.join(format!("expected_summary_{split_name}.json"));

    if !expected_actions.exists() || !expected_trades.exists() || !expected_summary.exists() {
        bail!("expected files are missing for split={split_name}");
    }

    let mut expected_actions_reader = ReaderBuilder::new().from_path(expected_actions)?;
    let mut expected_actions_rows = Vec::new();
    for row in expected_actions_reader.deserialize::<ExpectedActionRow>() {
        expected_actions_rows.push(row?);
    }

    let mut expected_trades_reader = ReaderBuilder::new().from_path(expected_trades)?;
    let mut expected_trade_rows = Vec::new();
    for row in expected_trades_reader.deserialize::<ExpectedTradeRow>() {
        expected_trade_rows.push(row?);
    }

    let expected_summary_value: ReplaySummary = serde_json::from_str(
        &fs::read_to_string(expected_summary).context("read expected summary json")?,
    )?;

    if actions.len() != expected_actions_rows.len() {
        bail!(
            "parity not reached yet: actions len mismatch {} vs {}",
            actions.len(),
            expected_actions_rows.len(),
        );
    }
    if trades.len() != expected_trade_rows.len() {
        bail!(
            "parity not reached yet: trades len mismatch {} vs {}",
            trades.len(),
            expected_trade_rows.len(),
        );
    }
    for (idx, (a, e)) in actions.iter().zip(expected_actions_rows.iter()).enumerate() {
        if a.bar_ts != e.bar_ts
            || a.action != e.action
            || a.owner != e.owner
            || a.side != e.side
            || a.reason != e.reason
            || a.state_before != e.state_before
            || a.state_after != e.state_after
            || (a.ref_price - e.ref_price).abs() > EPS
        {
            bail!("parity not reached yet: first actions diff at row {idx}");
        }
    }
    for (idx, (t, e)) in trades.iter().zip(expected_trade_rows.iter()).enumerate() {
        if t.trade_id != e.trade_id
            || t.owner != e.owner
            || t.side != e.side
            || t.entry_ts != e.entry_ts
            || t.exit_ts != e.exit_ts
            || (t.entry_price - e.entry_price).abs() > EPS
            || (t.exit_price - e.exit_price).abs() > EPS
            || t.size != e.size
            || (t.pnl_cash - e.pnl_cash).abs() > EPS
            || (t.return_pct_on_cash - e.return_pct_on_cash).abs() > EPS
            || t.exit_reason != e.exit_reason
        {
            bail!("parity not reached yet: first trades diff at row {idx}");
        }
    }
    if summary.trade_count != expected_summary_value.trade_count
        || (summary.final_value - expected_summary_value.final_value).abs() > 1e-6
        || (summary.total_return_pct - expected_summary_value.total_return_pct).abs() > 1e-6
        || (summary.annualized_sharpe - expected_summary_value.annualized_sharpe).abs() > 1e-6
        || (summary.max_drawdown_pct - expected_summary_value.max_drawdown_pct).abs() > 1e-6
        || summary.active_days != expected_summary_value.active_days
        || summary.no_trade_days != expected_summary_value.no_trade_days
        || summary.mr_trade_count != expected_summary_value.mr_trade_count
        || summary.bo_trade_count != expected_summary_value.bo_trade_count
    {
        bail!("parity not reached yet: summary mismatch");
    }
    Ok(())
}

fn fill_pending_entry(bar: &PreparedBar, state: &mut ReplayState) {
    let Some(pending) = state.pending_entry.clone() else {
        return;
    };
    let size = ((state.cash * POSITION_SIZE_FRACTION) / bar.open).floor() as i64;
    let size = size.max(1);
    let (stop_price, take_price) = if matches!(pending.owner, ReplayOwner::MeanRev) {
        (
            Some(round_to_tick(
                bar.open + MR_STOP_K_SHORT * pending.scale_at_signal,
                MR_TICK_SIZE,
            )),
            Some(round_to_tick(
                bar.open - MR_TAKE_K_SHORT * pending.scale_at_signal,
                MR_TICK_SIZE,
            )),
        )
    } else {
        (None, None)
    };
    let before = state.hybrid_state;
    state.open_position = Some(OpenPosition {
        owner: pending.owner,
        side: pending.side,
        entry_ts: bar.datetime,
        entry_price: bar.open,
        size,
        scale_at_signal: pending.scale_at_signal,
        stop_price,
        take_price,
        stop1: pending.stop1,
        stop2: pending.stop2,
    });
    state.pending_entry = None;
    state.hybrid_state = HybridState::Open;
    state.actions.push(ReplayAction {
        bar_ts: format_ts_iso(bar.datetime),
        action: "entry_fill".to_string(),
        owner: pending.owner.as_expected().to_string(),
        side: pending.side.as_expected().to_string(),
        reason: pending.reason,
        state_before: before.as_expected().to_string(),
        state_after: state.hybrid_state.as_expected().to_string(),
        ref_price: bar.open,
    });
}

fn evaluate_and_apply_exit(bar: &PreparedBar, state: &mut ReplayState) -> bool {
    let Some(position) = state.open_position.clone() else {
        return false;
    };
    let exit_signal = if matches!(position.owner, ReplayOwner::MeanRev) {
        evaluate_mr_exit(bar, &position)
    } else {
        evaluate_bo_exit(bar, &position)
    };
    let Some((reason, exit_price)) = exit_signal else {
        return false;
    };
    let before = state.hybrid_state;
    let pnl = mark_to_market_trade(position.side, position.entry_price, exit_price, position.size);
    state.cash += pnl;
    state.day_pnl += pnl;
    state.actions.push(ReplayAction {
        bar_ts: format_ts_iso(bar.datetime),
        action: "exit_fill".to_string(),
        owner: position.owner.as_expected().to_string(),
        side: position.side.as_expected().to_string(),
        reason: reason.to_string(),
        state_before: before.as_expected().to_string(),
        state_after: HybridState::Flat.as_expected().to_string(),
        ref_price: exit_price,
    });
    let trade_id = state.trades.len() + 1;
    let return_pct_on_cash = (pnl / position.entry_price / position.size as f64) * 100.0;
    state.trades.push(ReplayTrade {
        trade_id,
        owner: position.owner.as_expected().to_string(),
        side: position.side.as_expected().to_string(),
        entry_ts: format_ts_iso(position.entry_ts),
        exit_ts: format_ts_iso(bar.datetime),
        entry_price: position.entry_price,
        exit_price,
        size: position.size,
        pnl_cash: pnl,
        return_pct_on_cash,
        exit_reason: reason.to_string(),
    });
    state.open_position = None;
    state.hybrid_state = HybridState::Flat;
    true
}

fn evaluate_mr_signal(bar: &PreparedBar) -> Option<PendingEntry> {
    let entry_deadline =
        NaiveTime::from_hms_opt(MR_LAST_ENTRY_HOUR, MR_LAST_ENTRY_MINUTE, 0).unwrap_or(NaiveTime::MIN);
    if bar.datetime.time() > entry_deadline {
        return None;
    }
    let scale = bar.session_range;
    let close0 = bar.close;
    if !scale.is_finite() || scale <= 0.0 || close0 == 0.0 {
        return None;
    }
    let rel_scale = scale / close0;
    let dist = close0 - bar.session_vwap;
    if !(MR_MIN_REL_RANGE < rel_scale && rel_scale < MR_MAX_REL_RANGE) {
        return None;
    }
    if !(dist > 0.0 && dist < MR_K_SHORT * scale) {
        return None;
    }
    Some(PendingEntry {
        owner: ReplayOwner::MeanRev,
        side: ReplaySide::Short,
        reason: "mr_short_signal".to_string(),
        signal_ts: bar.datetime,
        scale_at_signal: scale,
        signal_price: close0,
        stop1: None,
        stop2: None,
    })
}

fn evaluate_bo_signal(bar: &PreparedBar, state: &ReplayState) -> Option<PendingEntry> {
    let scale = bar.session_range;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    if bar.elapsed_hours < BO_WAIT_HOURS {
        return None;
    }
    let can_long = bar.ret_from_open >= -BO_BIG_MOVE_THRESHOLD;
    let can_short = bar.ret_from_open <= BO_BIG_MOVE_THRESHOLD;
    let long_level = bar.session_open + BO_K * scale;
    let short_level = bar.session_open - BO_K * scale;
    if can_long && !state.bo_was_long_today && bar.close > long_level {
        return Some(PendingEntry {
            owner: ReplayOwner::DayBreakoutWaitfix,
            side: ReplaySide::Long,
            reason: "bo_long_signal".to_string(),
            signal_ts: bar.datetime,
            scale_at_signal: scale,
            signal_price: bar.close,
            stop1: Some(bar.session_open + BO_STOP1_RANGE * scale),
            stop2: Some(bar.session_open - BO_STOP2_RANGE * scale),
        });
    }
    if can_short && !state.bo_was_short_today && bar.close < short_level {
        return Some(PendingEntry {
            owner: ReplayOwner::DayBreakoutWaitfix,
            side: ReplaySide::Short,
            reason: "bo_short_signal".to_string(),
            signal_ts: bar.datetime,
            scale_at_signal: scale,
            signal_price: bar.close,
            stop1: Some(bar.session_open - BO_STOP1_RANGE * scale),
            stop2: Some(bar.session_open + BO_STOP2_RANGE * scale),
        });
    }
    None
}

fn evaluate_mr_exit(bar: &PreparedBar, position: &OpenPosition) -> Option<(&'static str, f64)> {
    let stop_price = position.stop_price?;
    let take_price = position.take_price?;
    if bar.high >= stop_price {
        return Some(("mr_stop", stop_price));
    }
    if bar.low <= take_price {
        return Some(("mr_take", take_price));
    }
    let force_time =
        NaiveTime::from_hms_opt(MR_FORCE_EXIT_HOUR, MR_FORCE_EXIT_MINUTE, 0).unwrap_or(NaiveTime::MIN);
    if bar.datetime.time() >= force_time {
        return Some(("mr_time_cutoff", bar.close));
    }
    None
}

fn evaluate_bo_exit(bar: &PreparedBar, position: &OpenPosition) -> Option<(&'static str, f64)> {
    let stop1 = position.stop1?;
    let stop2 = position.stop2?;
    match position.side {
        ReplaySide::Long => {
            if bar.low <= stop2 {
                return Some(("bo_stop2_long", stop2));
            }
            if bar.datetime.minute() == 50 && bar.close < stop1 {
                return Some(("bo_stop1_long", bar.close));
            }
            let eod = NaiveTime::from_hms_opt(BO_EOD_EXIT_HOUR, BO_EOD_EXIT_MINUTE, 0)
                .unwrap_or(NaiveTime::MIN);
            if bar.datetime.time() >= eod {
                return Some(("bo_eod_exit", bar.close));
            }
            None
        }
        ReplaySide::Short => {
            if bar.high >= stop2 {
                return Some(("bo_stop2_short", stop2));
            }
            if bar.datetime.minute() == 50 && bar.close > stop1 {
                return Some(("bo_stop1_short", bar.close));
            }
            let eod = NaiveTime::from_hms_opt(BO_EOD_EXIT_HOUR, BO_EOD_EXIT_MINUTE, 0)
                .unwrap_or(NaiveTime::MIN);
            if bar.datetime.time() >= eod {
                return Some(("bo_eod_exit", bar.close));
            }
            None
        }
        ReplaySide::None => None,
    }
}

fn round_to_tick(price: f64, tick: f64) -> f64 {
    if tick <= 0.0 {
        return price;
    }
    ((price / tick) + 0.5).floor() * tick
}

fn mark_to_market_trade(side: ReplaySide, entry_price: f64, exit_price: f64, size: i64) -> f64 {
    let gross = match side {
        ReplaySide::Long => (exit_price - entry_price) * size as f64,
        ReplaySide::Short => (entry_price - exit_price) * size as f64,
        ReplaySide::None => 0.0,
    };
    let fees = (entry_price + exit_price) * size as f64 * COMMISSION;
    gross - fees
}

fn annualized_sharpe(returns: &[f64]) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let var = returns
        .iter()
        .map(|r| {
            let d = *r - mean;
            d * d
        })
        .sum::<f64>()
        / returns.len() as f64;
    let std = var.sqrt();
    if std <= 0.0 {
        0.0
    } else {
        (mean / std) * 252.0f64.sqrt()
    }
}

fn max_drawdown_pct(equity: &[f64]) -> f64 {
    if equity.is_empty() {
        return 0.0;
    }
    let mut peak = equity[0];
    let mut min_dd = 0.0;
    for value in equity {
        if *value > peak {
            peak = *value;
        }
        if peak > 0.0 {
            let dd = (*value / peak) - 1.0;
            if dd < min_dd {
                min_dd = dd;
            }
        }
    }
    min_dd.abs() * 100.0
}

fn format_ts_iso(ts: NaiveDateTime) -> String {
    ts.format("%Y-%m-%dT%H:%M:%S").to_string()
}
