use std::env;
use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Timelike};
use serde::Deserialize;
use serde::Serialize;

const DEFAULT_BARS_CSV: &str = "paper_bars_2.csv";
const DEFAULT_TRADES_CSV: &str = "paper_trades_2.csv";
const DEFAULT_BARS_NORMALIZED_CSV: &str = "bars_normalized.csv";
const DEFAULT_INTENTS_LOG_CSV: &str = "intents_log.csv";
const DEFAULT_TRADES_RUNTIME_CSV: &str = "trades_runtime.csv";
const DEFAULT_PARITY_REPORT_JSON: &str = "parity_report.json";
const DEFAULT_OUTPUT_DIR: &str = ".";
const DEFAULT_SYMBOL: &str = "USDRUBF";
const START_CASH: f64 = 30_000.0;

#[derive(Debug, Clone, Copy)]
struct StrategyConfig {
    k_long: f64,
    k_short: f64,
    wait_hours: i64,
    k_tp_long: f64,
    k_sl_long: f64,
    k_tp_short: f64,
    k_sl_short: f64,
    long_ex_pct: f64,
    short_ex_pct: f64,
    max_entry_hour: u32,
    close_hour: u32,
    close_minute: u32,
    session_gap_min: f64,
    exit_offset_min: i64,
    stake: i64,
    test: bool,
    work_weekends: bool,
    cash_factor: f64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            k_long: 0.5,
            k_short: 0.46,
            wait_hours: 3,
            k_tp_long: 0.28,
            k_sl_long: 0.68,
            k_tp_short: 0.28,
            k_sl_short: 0.65,
            long_ex_pct: 2.2,
            short_ex_pct: 2.2,
            max_entry_hour: 19,
            close_hour: 23,
            close_minute: 49,
            session_gap_min: 60.0,
            exit_offset_min: 20,
            stake: 1,
            test: false,
            work_weekends: false,
            cash_factor: 0.9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    direction: Direction,
    size: i64,
}

#[derive(Debug, Clone)]
struct PendingExit {
    reason: String,
}

#[derive(Debug, Clone)]
struct Position {
    direction: Direction,
    size: i64,
    entry_price: f64,
    entry_time: DateTime<FixedOffset>,
    tp: f64,
    sl: f64,
}

#[derive(Debug, Clone)]
struct Bar {
    time: DateTime<FixedOffset>,
    ts_utc: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Deserialize)]
struct CsvBar {
    time: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Serialize)]
struct IntentLogRow {
    bar_ts: i64,
    symbol: String,
    intent_type: String,
    side: String,
    qty: i64,
    reason_code: String,
    state_snapshot: String,
}

#[derive(Debug, Serialize)]
struct TradeRuntimeRow {
    entry_ts: i64,
    exit_ts: i64,
    entry_time: String,
    exit_time: String,
    side: String,
    qty: i64,
    entry_price: f64,
    exit_price: f64,
    exit_reason: String,
    pnl: f64,
    cash_after: f64,
}

#[derive(Debug, Deserialize)]
struct ReferenceTradeRow {
    entry_time: String,
    exit_time: String,
    direction: String,
    size: i64,
    entry_price: f64,
    exit_price: f64,
    reason: String,
    pnl: f64,
}

#[derive(Debug, Serialize)]
struct ParityReport {
    status: String,
    tolerance: f64,
    runtime_trades: usize,
    reference_trades: usize,
    matched_trades: usize,
    first_divergence: Option<String>,
}

struct ReplayState {
    cfg: StrategyConfig,
    cash: f64,
    last_dt: Option<DateTime<FixedOffset>>,
    session_start_dt: Option<DateTime<FixedOffset>>,
    session_end_dt: Option<DateTime<FixedOffset>>,
    session_high: Option<f64>,
    session_low: Option<f64>,
    session_close: Option<f64>,
    yesterday_close: Option<f64>,
    yesterday_range: Option<f64>,
    pre_prev_close: Option<f64>,
    first_min_high: Option<f64>,
    first_min_low: Option<f64>,
    first_hour_price: Option<f64>,
    traded_session: bool,
    pending_entry: Option<PendingEntry>,
    pending_exit: Option<PendingExit>,
    position: Option<Position>,
}

impl ReplayState {
    fn new(cfg: StrategyConfig) -> Self {
        Self {
            cfg,
            cash: START_CASH,
            last_dt: None,
            session_start_dt: None,
            session_end_dt: None,
            session_high: None,
            session_low: None,
            session_close: None,
            yesterday_close: None,
            yesterday_range: None,
            pre_prev_close: None,
            first_min_high: None,
            first_min_low: None,
            first_hour_price: None,
            traded_session: false,
            pending_entry: None,
            pending_exit: None,
            position: None,
        }
    }

    fn on_bar(
        &mut self,
        symbol: &str,
        bar: &Bar,
        intents: &mut Vec<IntentLogRow>,
        trades: &mut Vec<TradeRuntimeRow>,
    ) {
        if !self.cfg.work_weekends && bar.time.weekday().number_from_monday() >= 6 {
            return;
        }

        self.update_session(bar);
        self.execute_pending_entry(symbol, bar, intents);
        self.execute_pending_exit(symbol, bar, intents, trades);
        self.handle_exit_conditions(bar);
        self.maybe_generate_signal(symbol, bar, intents);
    }

    fn update_session(&mut self, bar: &Bar) {
        match self.last_dt {
            None => self.reset_session(bar),
            Some(last_dt) => {
                let diff_min = (bar.time - last_dt).num_seconds() as f64 / 60.0;
                if diff_min > self.cfg.session_gap_min {
                    self.pre_prev_close = self.yesterday_close;
                    self.yesterday_close = self.session_close;
                    self.yesterday_range = match (self.session_high, self.session_low) {
                        (Some(high), Some(low)) => Some(high - low),
                        _ => None,
                    };
                    self.reset_session(bar);
                } else {
                    self.session_high = Some(self.session_high.unwrap_or(bar.high).max(bar.high));
                    self.session_low = Some(self.session_low.unwrap_or(bar.low).min(bar.low));
                    self.session_close = Some(bar.close);

                    if let Some(session_start_dt) = self.session_start_dt {
                        if bar.time.time() == session_start_dt.time() {
                            self.first_min_high = Some(bar.high);
                            self.first_min_low = Some(bar.low);
                        }

                        if self.first_hour_price.is_none()
                            && (bar.time - session_start_dt).num_seconds() >= 3600
                        {
                            self.first_hour_price = Some(bar.close);
                        }
                    }
                }
            }
        }

        self.last_dt = Some(bar.time);
    }

    fn reset_session(&mut self, bar: &Bar) {
        let session_start_dt = bar.time;
        let offset = moscow_offset();
        let session_end_dt = offset
            .with_ymd_and_hms(
                bar.time.year(),
                bar.time.month(),
                bar.time.day(),
                self.cfg.close_hour,
                self.cfg.close_minute,
                0,
            )
            .single()
            .unwrap_or_else(|| offset.timestamp_opt(0, 0).unwrap());

        self.session_start_dt = Some(session_start_dt);
        self.session_end_dt = Some(session_end_dt);
        self.session_high = Some(bar.high);
        self.session_low = Some(bar.low);
        self.session_close = Some(bar.close);
        self.first_min_high = Some(bar.high);
        self.first_min_low = Some(bar.low);
        self.first_hour_price = None;
        self.traded_session = false;
        self.pending_entry = None;
        self.pending_exit = None;
    }

    fn execute_pending_entry(&mut self, symbol: &str, bar: &Bar, intents: &mut Vec<IntentLogRow>) {
        if self.position.is_some() {
            self.pending_entry = None;
            return;
        }

        let pending = match self.pending_entry.take() {
            Some(entry) => entry,
            None => return,
        };

        let range = match self.yesterday_range {
            Some(range) => range,
            None => return,
        };

        let (tp, sl) = match pending.direction {
            Direction::Long => (
                bar.open + self.cfg.k_tp_long * range,
                bar.open - self.cfg.k_sl_long * range,
            ),
            Direction::Short => (
                bar.open - self.cfg.k_tp_short * range,
                bar.open + self.cfg.k_sl_short * range,
            ),
        };

        let position = Position {
            direction: pending.direction,
            size: pending.size,
            entry_price: bar.open,
            entry_time: bar.time,
            tp,
            sl,
        };

        match pending.direction {
            Direction::Long => self.cash -= bar.open * pending.size as f64,
            Direction::Short => self.cash += bar.open * pending.size as f64,
        }

        intents.push(IntentLogRow {
            bar_ts: bar.ts_utc,
            symbol: symbol.to_string(),
            intent_type: "entry".to_string(),
            side: format!("{:?}", pending.direction).to_lowercase(),
            qty: pending.size,
            reason_code: "pending_execute".to_string(),
            state_snapshot: self.snapshot(),
        });

        self.position = Some(position);
    }

    fn execute_pending_exit(
        &mut self,
        symbol: &str,
        bar: &Bar,
        intents: &mut Vec<IntentLogRow>,
        trades: &mut Vec<TradeRuntimeRow>,
    ) {
        let pending = match self.pending_exit.take() {
            Some(pending) => pending,
            None => return,
        };
        let position = match self.position.clone() {
            Some(position) => position,
            None => return,
        };
        self.exit_position(
            symbol,
            bar,
            bar.open,
            &pending.reason,
            &position,
            intents,
            trades,
        );
    }

    fn handle_exit_conditions(&mut self, bar: &Bar) {
        if self.pending_exit.is_some() {
            return;
        }
        let position = match self.position.as_ref() {
            Some(pos) => pos,
            None => return,
        };

        if let Some(session_end_dt) = self.session_end_dt {
            let exit_threshold = session_end_dt - Duration::minutes(self.cfg.exit_offset_min);
            if bar.time >= exit_threshold {
                self.pending_exit = Some(PendingExit {
                    reason: "session_end".to_string(),
                });
                return;
            }
        }

        match position.direction {
            Direction::Long => {
                if bar.high >= position.tp {
                    self.pending_exit = Some(PendingExit {
                        reason: "tp".to_string(),
                    });
                } else if bar.low <= position.sl {
                    self.pending_exit = Some(PendingExit {
                        reason: "sl".to_string(),
                    });
                }
            }
            Direction::Short => {
                if bar.low <= position.tp {
                    self.pending_exit = Some(PendingExit {
                        reason: "tp".to_string(),
                    });
                } else if bar.high >= position.sl {
                    self.pending_exit = Some(PendingExit {
                        reason: "sl".to_string(),
                    });
                }
            }
        }
    }

    fn exit_position(
        &mut self,
        symbol: &str,
        bar: &Bar,
        exit_price: f64,
        reason: &str,
        position: &Position,
        intents: &mut Vec<IntentLogRow>,
        trades: &mut Vec<TradeRuntimeRow>,
    ) {
        match position.direction {
            Direction::Long => self.cash += exit_price * position.size as f64,
            Direction::Short => self.cash -= exit_price * position.size as f64,
        }

        let pnl = match position.direction {
            Direction::Long => (exit_price - position.entry_price) * position.size as f64,
            Direction::Short => (position.entry_price - exit_price) * position.size as f64,
        };

        intents.push(IntentLogRow {
            bar_ts: bar.ts_utc,
            symbol: symbol.to_string(),
            intent_type: "exit".to_string(),
            side: format!("{:?}", position.direction).to_lowercase(),
            qty: position.size,
            reason_code: reason.to_string(),
            state_snapshot: self.snapshot(),
        });

        trades.push(TradeRuntimeRow {
            entry_ts: position.entry_time.timestamp(),
            exit_ts: bar.ts_utc,
            entry_time: position.entry_time.to_rfc3339(),
            exit_time: bar.time.to_rfc3339(),
            side: format!("{:?}", position.direction),
            qty: position.size,
            entry_price: position.entry_price,
            exit_price,
            exit_reason: reason.to_string(),
            pnl,
            cash_after: self.cash,
        });

        self.position = None;
        self.pending_exit = None;
    }

    fn maybe_generate_signal(&mut self, symbol: &str, bar: &Bar, intents: &mut Vec<IntentLogRow>) {
        if self.traded_session {
            return;
        }

        let (session_start_dt, session_end_dt) = match (self.session_start_dt, self.session_end_dt)
        {
            (Some(start), Some(end)) => (start, end),
            _ => return,
        };

        if bar.time >= session_end_dt || bar.time.hour() >= self.cfg.max_entry_hour {
            return;
        }

        let (yesterday_close, yesterday_range, pre_prev_close) = match (
            self.yesterday_close,
            self.yesterday_range,
            self.pre_prev_close,
        ) {
            (Some(close), Some(range), Some(prev_close)) => (close, range, prev_close),
            _ => return,
        };

        if bar.time.minute() == 59
            && (bar.time - session_start_dt).num_seconds() >= self.cfg.wait_hours * 3600
        {
            let price = bar.close;
            let signal = if price > yesterday_close + self.cfg.k_long * yesterday_range
                && self.first_min_high.is_some_and(|high| price > high)
                && self
                    .first_hour_price
                    .is_some_and(|first_hour| price > first_hour)
                && yesterday_close > (1.0 - self.cfg.long_ex_pct / 100.0) * pre_prev_close
            {
                Some(Direction::Long)
            } else if price < yesterday_close - self.cfg.k_short * yesterday_range
                && self.first_min_low.is_some_and(|low| price < low)
                && self
                    .first_hour_price
                    .is_some_and(|first_hour| price < first_hour)
                && yesterday_close < (1.0 + self.cfg.short_ex_pct / 100.0) * pre_prev_close
            {
                Some(Direction::Short)
            } else {
                None
            };

            if let Some(direction) = signal {
                if self.pending_entry.is_some() {
                    return;
                }

                let size = if self.cfg.test {
                    self.cfg.stake
                } else {
                    let available_cash = self.cfg.cash_factor * self.cash;
                    let mut computed = (available_cash / bar.close).floor() as i64;
                    if computed < 1 {
                        computed = 1;
                    }
                    computed
                };

                self.pending_entry = Some(PendingEntry { direction, size });
                self.traded_session = true;

                intents.push(IntentLogRow {
                    bar_ts: bar.ts_utc,
                    symbol: symbol.to_string(),
                    intent_type: "signal".to_string(),
                    side: format!("{:?}", direction).to_lowercase(),
                    qty: size,
                    reason_code: "signal_threshold".to_string(),
                    state_snapshot: self.snapshot(),
                });
            }
        }
    }

    fn snapshot(&self) -> String {
        format!(
            "cash={:.2};y_close={:?};y_range={:?};pre_prev={:?};pending={:?};in_pos={}",
            self.cash,
            self.yesterday_close,
            self.yesterday_range,
            self.pre_prev_close,
            self.pending_entry
                .as_ref()
                .map(|p| format!("{:?}:{}", p.direction, p.size)),
            self.position.is_some()
        )
    }
}

fn main() -> Result<()> {
    let replay_enabled = env_bool("REPLAY_ENABLED", true);
    if !replay_enabled {
        println!("replay disabled (REPLAY_ENABLED=false)");
        return Ok(());
    }

    let bars_path =
        env::var("REPLAY_BARS_CSV_PATH").unwrap_or_else(|_| DEFAULT_BARS_CSV.to_string());
    let reference_trades_path = env::var("REPLAY_REFERENCE_TRADES_CSV_PATH")
        .or_else(|_| env::var("REPLAY_REFERENCE_TRADES_PATH"))
        .unwrap_or_else(|_| DEFAULT_TRADES_CSV.to_string());
    let symbol = env::var("REPLAY_SYMBOL").unwrap_or_else(|_| DEFAULT_SYMBOL.to_string());
    let tolerance = env::var("REPLAY_PRICE_TOLERANCE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1e-8);
    let strict_dedup = env::var("REPLAY_STRICT_DEDUP")
        .ok()
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true);

    let output_dir =
        env::var("REPLAY_OUTPUT_DIR").unwrap_or_else(|_| DEFAULT_OUTPUT_DIR.to_string());
    let bars_out = output_path(
        &output_dir,
        "REPLAY_BARS_NORMALIZED_PATH",
        DEFAULT_BARS_NORMALIZED_CSV,
    );
    let intents_out = output_path(
        &output_dir,
        "REPLAY_INTENTS_LOG_PATH",
        DEFAULT_INTENTS_LOG_CSV,
    );
    let trades_out = output_path(
        &output_dir,
        "REPLAY_TRADES_RUNTIME_PATH",
        DEFAULT_TRADES_RUNTIME_CSV,
    );
    let parity_out = output_path(
        &output_dir,
        "REPLAY_PARITY_REPORT_PATH",
        DEFAULT_PARITY_REPORT_JSON,
    );

    let bars = load_bars(&bars_path, strict_dedup)?;
    write_normalized_bars(&bars, &bars_out)?;

    let mut state = ReplayState::new(StrategyConfig::default());
    let mut intents = Vec::new();
    let mut trades_runtime = Vec::new();
    for bar in &bars {
        state.on_bar(&symbol, bar, &mut intents, &mut trades_runtime);
    }

    write_intents(&intents_out, &intents)?;
    write_runtime_trades(&trades_out, &trades_runtime)?;

    let reference = load_reference_trades(&reference_trades_path)?;
    let report = build_parity_report(&trades_runtime, &reference, tolerance);
    ensure_parent_dir(&parity_out)?;
    serde_json::to_writer_pretty(File::create(&parity_out)?, &report)?;

    println!(
        "done: bars={} intents={} trades={} parity={}",
        bars.len(),
        intents.len(),
        trades_runtime.len(),
        report.status
    );
    Ok(())
}

fn output_path(output_dir: &str, env_key: &str, default_name: &str) -> String {
    env::var(env_key).unwrap_or_else(|_| format!("{output_dir}/{default_name}"))
}

fn ensure_parent_dir(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }
    }
    Ok(())
}

fn load_bars(path: &str, strict_dedup: bool) -> Result<Vec<Bar>> {
    let mut rdr = csv::Reader::from_path(path)
        .with_context(|| format!("failed to open bars csv at {path}"))?;
    let mut bars = Vec::new();
    let mut previous_ts = None;

    for (idx, row) in rdr.deserialize::<CsvBar>().enumerate() {
        let row_number = idx + 2;
        let raw = row.with_context(|| format!("invalid CSV row #{row_number}"))?;
        let time = DateTime::parse_from_rfc3339(&raw.time)
            .with_context(|| format!("invalid RFC3339 time at row #{row_number}: {}", raw.time))?;
        let ts_utc = time.timestamp();

        if let Some(prev) = previous_ts {
            if ts_utc < prev {
                bail!("bars csv is not ascending at row #{row_number}: {ts_utc} < {prev}");
            }
            if ts_utc == prev {
                if strict_dedup {
                    bail!("bars csv contains duplicate timestamp at row #{row_number}: {ts_utc}");
                }
                continue;
            }
        }
        previous_ts = Some(ts_utc);

        bars.push(Bar {
            time,
            ts_utc,
            open: raw.open,
            high: raw.high,
            low: raw.low,
            close: raw.close,
        });
    }

    Ok(bars)
}

fn write_normalized_bars(bars: &[Bar], output: &str) -> Result<()> {
    ensure_parent_dir(output)?;
    let mut writer = csv::Writer::from_path(output)?;
    writer.write_record(["time", "time_unix_utc", "open", "high", "low", "close"])?;
    for bar in bars {
        writer.write_record([
            bar.time.to_rfc3339(),
            bar.ts_utc.to_string(),
            format!("{:.4}", bar.open),
            format!("{:.4}", bar.high),
            format!("{:.4}", bar.low),
            format!("{:.4}", bar.close),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn write_intents(path: &str, rows: &[IntentLogRow]) -> Result<()> {
    ensure_parent_dir(path)?;
    let mut writer = csv::Writer::from_path(path)?;
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}

fn write_runtime_trades(path: &str, rows: &[TradeRuntimeRow]) -> Result<()> {
    ensure_parent_dir(path)?;
    let mut writer = csv::Writer::from_path(path)?;
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}

fn load_reference_trades(path: &str) -> Result<Vec<ReferenceTradeRow>> {
    if !Path::new(path).exists() {
        return Ok(Vec::new());
    }
    let mut rdr = csv::Reader::from_path(path)?;
    let mut out = Vec::new();
    for row in rdr.deserialize::<ReferenceTradeRow>() {
        out.push(row?);
    }
    Ok(out)
}

fn build_parity_report(
    runtime: &[TradeRuntimeRow],
    reference: &[ReferenceTradeRow],
    tolerance: f64,
) -> ParityReport {
    let mut first_divergence = None;
    let mut matched = 0usize;
    let common = runtime.len().min(reference.len());

    for index in 0..common {
        let rt = &runtime[index];
        let rf = &reference[index];

        let entry_match = DateTime::parse_from_rfc3339(&rf.entry_time)
            .ok()
            .map(|d| d.timestamp())
            == Some(rt.entry_ts);
        let exit_match = DateTime::parse_from_rfc3339(&rf.exit_time)
            .ok()
            .map(|d| d.timestamp())
            == Some(rt.exit_ts);
        let side_match = rf.direction.eq_ignore_ascii_case(&rt.side);
        let qty_match = rf.size == rt.qty;
        let entry_price_match = (rf.entry_price - rt.entry_price).abs() <= tolerance;
        let exit_price_match = (rf.exit_price - rt.exit_price).abs() <= tolerance;
        let pnl_match = (rf.pnl - rt.pnl).abs() <= tolerance;

        if entry_match
            && exit_match
            && side_match
            && qty_match
            && entry_price_match
            && exit_price_match
            && pnl_match
        {
            matched += 1;
            continue;
        }

        first_divergence = Some(format!(
            "index={index} entry_match={entry_match} exit_match={exit_match} side_match={side_match} qty_match={qty_match} entry_price_match={entry_price_match} exit_price_match={exit_price_match} pnl_match={pnl_match} reason_ref={} reason_runtime={}",
            rf.reason, rt.exit_reason
        ));
        break;
    }

    if first_divergence.is_none() && runtime.len() != reference.len() {
        first_divergence = Some(format!(
            "trade count mismatch runtime={} reference={}",
            runtime.len(),
            reference.len()
        ));
    }

    let status = if first_divergence.is_none() {
        "pass".to_string()
    } else {
        "fail".to_string()
    };

    ParityReport {
        status,
        tolerance,
        runtime_trades: runtime.len(),
        reference_trades: reference.len(),
        matched_trades: matched,
        first_divergence,
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn moscow_offset() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).expect("valid fixed offset")
}
