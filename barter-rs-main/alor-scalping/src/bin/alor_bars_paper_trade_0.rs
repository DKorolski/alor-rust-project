use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tracing::{debug, info, warn};

const OAUTH_URL: &str = "https://oauth.alor.ru/refresh";
const WS_URL: &str = "wss://api.alor.ru/ws";

const DEFAULT_PORTFOLIO: &str = "7502T0U";
const DEFAULT_SYMBOL: &str = "USDRUBF";
const DEFAULT_EXCHANGE: &str = "MOEX";
const DEFAULT_INSTRUMENT_GROUP: &str = "RFUD";
const DEFAULT_TIMEFRAME_SEC: i64 = 60;
const DEFAULT_SKIP_HISTORY: bool = false;
const DEFAULT_SPLIT_ADJUST: bool = true;
const DEFAULT_FORMAT: &str = "Simple";
const DEFAULT_FREQUENCY_MS: i64 = 250;
const DEFAULT_TRADE_LOG: &str = "paper_trades_1.csv";
const DEFAULT_START_CASH: f64 = 30_000.0;
const DEFAULT_FROM_DATE: &str = "2025-12-31";
const DEFAULT_HISTORY_BATCH_LIMIT: usize = 4999;
const DEFAULT_HISTORY_ONLY: bool = true;
const DEFAULT_HISTORY_MAX_GAP_MIN: i64 = 10_080;
const MOSCOW_OFFSET_HOURS: i32 = 3;

#[derive(Debug, Clone, Copy)]
struct Bar {
    time: DateTime<FixedOffset>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone)]
struct PendingEntry {
    direction: Direction,
    size: i64,
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
            wait_hours: 2,
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

struct TradeLogger {
    writer: csv::Writer<std::fs::File>,
}

impl TradeLogger {
    fn new(path: &str) -> anyhow::Result<Self> {
        let csv_path = std::path::Path::new(path);
        let create_header = !csv_path.exists();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(csv_path)
            .with_context(|| format!("failed to open trade log {path}"))?;
        let mut writer = csv::Writer::from_writer(file);
        if create_header {
            writer.write_record([
                "entry_time",
                "exit_time",
                "direction",
                "size",
                "entry_price",
                "exit_price",
                "reason",
                "pnl",
                "cash_after",
            ])?;
            writer.flush()?;
        }
        Ok(Self { writer })
    }

    fn log_trade(
        &mut self,
        position: &Position,
        exit_time: DateTime<FixedOffset>,
        exit_price: f64,
        reason: &str,
        cash_after: f64,
    ) -> anyhow::Result<()> {
        let pnl = match position.direction {
            Direction::Long => (exit_price - position.entry_price) * position.size as f64,
            Direction::Short => (position.entry_price - exit_price) * position.size as f64,
        };

        self.writer.write_record([
            position.entry_time.to_rfc3339(),
            exit_time.to_rfc3339(),
            format!("{:?}", position.direction),
            position.size.to_string(),
            format!("{:.4}", position.entry_price),
            format!("{:.4}", exit_price),
            reason.to_string(),
            format!("{:.4}", pnl),
            format!("{:.2}", cash_after),
        ])?;
        self.writer.flush()?;
        Ok(())
    }
}

struct StrategyState {
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
    position: Option<Position>,
}

impl StrategyState {
    fn new(cfg: StrategyConfig, cash: f64) -> Self {
        Self {
            cfg,
            cash,
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
            position: None,
        }
    }

    fn on_bar(&mut self, bar: Bar, trade_log: &mut TradeLogger) -> anyhow::Result<()> {
        if !self.cfg.work_weekends && bar.time.weekday().number_from_monday() >= 6 {
            debug!("weekend bar skipped");
            return Ok(());
        }

        self.update_session(&bar);
        self.execute_pending_entry(&bar);
        self.handle_exit_conditions(&bar, trade_log)?;
        self.maybe_generate_signal(&bar);

        Ok(())
    }

    fn update_session(&mut self, bar: &Bar) {
        match self.last_dt {
            None => {
                self.reset_session(bar);
            }
            Some(last_dt) => {
                let diff_min = (bar.time - last_dt).num_seconds() as f64 / 60.0;
                if diff_min < 0.0 {
                    warn!(
                        "time moved backwards: last_dt={} current_dt={} diff_min={:.2}",
                        last_dt, bar.time, diff_min
                    );
                }
                if diff_min > self.cfg.session_gap_min {
                    info!(
                        "Session gap {:.2} мин, завершение сессии. Position: {:?}. last_dt={} current_dt={}",
                        diff_min, self.position, last_dt, bar.time
                    );
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
        let session_end_dt = moscow_offset()
            .with_ymd_and_hms(
                bar.time.year(),
                bar.time.month(),
                bar.time.day(),
                self.cfg.close_hour,
                self.cfg.close_minute,
                0,
            )
            .single()
            .unwrap_or_else(|| moscow_offset().timestamp_opt(0, 0).unwrap());

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

        info!("Session start {} end {}", session_start_dt, session_end_dt);
    }

    fn execute_pending_entry(&mut self, bar: &Bar) {
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
            None => {
                warn!("Pending entry without yesterday range; skipping");
                return;
            }
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
            Direction::Long => {
                self.cash -= bar.open * pending.size as f64;
            }
            Direction::Short => {
                self.cash += bar.open * pending.size as f64;
            }
        }

        info!(
            "ENTRY {:?} size={} price={:.4} tp={:.4} sl={:.4} cash={:.2}",
            position.direction,
            position.size,
            position.entry_price,
            position.tp,
            position.sl,
            self.cash
        );

        self.position = Some(position);
    }

    fn handle_exit_conditions(
        &mut self,
        bar: &Bar,
        trade_log: &mut TradeLogger,
    ) -> anyhow::Result<()> {
        let position = match self.position.clone() {
            Some(pos) => pos,
            None => return Ok(()),
        };

        if let Some(session_end_dt) = self.session_end_dt {
            let exit_threshold =
                session_end_dt - chrono::Duration::minutes(self.cfg.exit_offset_min);
            if bar.time >= exit_threshold {
                info!(
                    "Session end exit at {} (threshold {})",
                    bar.time, exit_threshold
                );
                self.exit_position(bar.time, bar.close, "session_end", &position, trade_log)?;
                return Ok(());
            }
        }

        match position.direction {
            Direction::Long => {
                if bar.high >= position.tp {
                    self.exit_position(bar.time, position.tp, "tp", &position, trade_log)?;
                } else if bar.low <= position.sl {
                    self.exit_position(bar.time, position.sl, "sl", &position, trade_log)?;
                }
            }
            Direction::Short => {
                if bar.low <= position.tp {
                    self.exit_position(bar.time, position.tp, "tp", &position, trade_log)?;
                } else if bar.high >= position.sl {
                    self.exit_position(bar.time, position.sl, "sl", &position, trade_log)?;
                }
            }
        }

        Ok(())
    }

    fn exit_position(
        &mut self,
        exit_time: DateTime<FixedOffset>,
        exit_price: f64,
        reason: &str,
        position: &Position,
        trade_log: &mut TradeLogger,
    ) -> anyhow::Result<()> {
        match position.direction {
            Direction::Long => {
                self.cash += exit_price * position.size as f64;
            }
            Direction::Short => {
                self.cash -= exit_price * position.size as f64;
            }
        }

        info!(
            "EXIT {:?} size={} price={:.4} reason={} cash={:.2}",
            position.direction, position.size, exit_price, reason, self.cash
        );

        trade_log.log_trade(position, exit_time, exit_price, reason, self.cash)?;
        self.position = None;
        Ok(())
    }

    fn maybe_generate_signal(&mut self, bar: &Bar) {
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
            _ => {
                debug!("insufficient data for signal");
                return;
            }
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

                let entry_price = bar.close;
                let size = if self.cfg.test {
                    self.cfg.stake
                } else {
                    let available_cash = self.cfg.cash_factor * self.cash;
                    let mut size = (available_cash / entry_price).floor() as i64;
                    if size < 1 {
                        size = 1;
                    }
                    size
                };

                info!(
                    "SIGNAL {:?} close={:.4} size={} cash={:.2}",
                    direction, entry_price, size, self.cash
                );

                self.pending_entry = Some(PendingEntry { direction, size });
                self.traded_session = true;
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let access_token = get_access_token().await?;
    let portfolio = get_env_or_default("ALOR_PORTFOLIO", DEFAULT_PORTFOLIO);
    let symbol = get_env_or_default("ALOR_SYMBOL", DEFAULT_SYMBOL);
    let exchange = get_env_or_default("ALOR_EXCHANGE", DEFAULT_EXCHANGE);
    let instrument_group = get_env_or_default("ALOR_INSTRUMENT_GROUP", DEFAULT_INSTRUMENT_GROUP);
    let timeframe_sec = std::env::var("ALOR_TF")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEFRAME_SEC);
    let format = get_env_or_default("ALOR_FORMAT", DEFAULT_FORMAT);
    let frequency_ms = std::env::var("ALOR_FREQUENCY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_FREQUENCY_MS);
    let skip_history = std::env::var("ALOR_SKIP_HISTORY")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_SKIP_HISTORY);
    let split_adjust = std::env::var("ALOR_SPLIT_ADJUST")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_SPLIT_ADJUST);
    let trade_log = get_env_or_default("ALOR_TRADE_LOG", DEFAULT_TRADE_LOG);
    let history_batch_limit = std::env::var("ALOR_HISTORY_BATCH_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_HISTORY_BATCH_LIMIT);
    let history_only = std::env::var("ALOR_HISTORY_ONLY")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(DEFAULT_HISTORY_ONLY);
    let history_max_gap_min = std::env::var("ALOR_HISTORY_MAX_GAP_MIN")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_HISTORY_MAX_GAP_MIN);

    let from_start = start_of_utc_day();
    let from_ts = std::env::var("ALOR_FROM_TS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| parse_date_env("ALOR_FROM_DATE"))
        .unwrap_or_else(|| parse_date(DEFAULT_FROM_DATE).unwrap_or_else(|| from_start.timestamp()));
    let to_ts = std::env::var("ALOR_TO_TS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| parse_date_env("ALOR_TO_DATE"));

    info!(
        "start portfolio={portfolio} symbol={symbol} exchange={exchange} group={instrument_group} tf={timeframe_sec} format={format} frequency={frequency_ms} skip_history={skip_history} split_adjust={split_adjust} from_ts={from_ts} to_ts={to_ts:?} history_batch_limit={history_batch_limit} history_only={history_only} history_max_gap_min={history_max_gap_min}"
    );

    let mut strategy = StrategyState::new(StrategyConfig::default(), DEFAULT_START_CASH);
    let mut trade_log = TradeLogger::new(&trade_log)?;

    let last_history_ts = fetch_history_batches(
        &access_token,
        &symbol,
        &exchange,
        &instrument_group,
        timeframe_sec,
        from_ts,
        false,
        split_adjust,
        &format,
        frequency_ms,
        &mut strategy,
        &mut trade_log,
        to_ts,
        history_batch_limit,
        history_max_gap_min,
    )
    .await?;

    if history_only || to_ts.is_some() {
        return Ok(());
    }

    let live_from_ts = last_history_ts.unwrap_or(from_ts);
    let mut reconnect_delay = Duration::from_secs(1);
    loop {
        match run_live_stream(
            &access_token,
            &symbol,
            &exchange,
            &instrument_group,
            timeframe_sec,
            live_from_ts,
            true,
            split_adjust,
            &format,
            frequency_ms,
            &mut strategy,
            &mut trade_log,
            None,
        )
        .await
        {
            Ok(()) => {
                warn!("stream ended; reconnecting");
            }
            Err(error) => {
                warn!(?error, "stream error; reconnecting");
            }
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(30));
    }
}

async fn run_live_stream(
    access_token: &str,
    symbol: &str,
    exchange: &str,
    instrument_group: &str,
    timeframe_sec: i64,
    from_ts: i64,
    skip_history: bool,
    split_adjust: bool,
    format: &str,
    frequency_ms: i64,
    strategy: &mut StrategyState,
    trade_log: &mut TradeLogger,
    to_ts: Option<i64>,
) -> anyhow::Result<()> {
    let (ws_data, _) = connect_async(WS_URL).await?;
    let (mut ws_sink, mut ws_stream) = ws_data.split();

    let (first_msg, subscribe_rt_ms) = subscribe_bars(
        &mut ws_sink,
        &mut ws_stream,
        access_token,
        symbol,
        exchange,
        instrument_group,
        timeframe_sec,
        from_ts,
        skip_history,
        split_adjust,
        format,
        frequency_ms,
    )
    .await?;

    info!(
        "SUBSCRIBE first message in {:.2} ms: {}",
        subscribe_rt_ms, first_msg
    );

    loop {
        let msg = match ws_stream.next().await {
            Some(msg) => msg?,
            None => return Ok(()),
        };

        match msg {
            Message::Text(txt) => {
                if let Ok(payload) = serde_json::from_str::<Value>(&txt) {
                    let bars = extract_bars(&payload);
                    for bar in bars {
                        if let Some(to_ts) = to_ts {
                            if bar.time.timestamp() >= to_ts {
                                info!("reached to_ts={}, stopping stream", to_ts);
                                return Ok(());
                            }
                        }
                        strategy.on_bar(bar, trade_log)?;
                    }
                }
            }
            Message::Ping(payload) => {
                ws_sink.send(Message::Pong(payload)).await?;
            }
            Message::Close(frame) => {
                info!(?frame, "ws close received");
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn fetch_history_batches(
    access_token: &str,
    symbol: &str,
    exchange: &str,
    instrument_group: &str,
    timeframe_sec: i64,
    from_ts: i64,
    skip_history: bool,
    split_adjust: bool,
    format: &str,
    frequency_ms: i64,
    strategy: &mut StrategyState,
    trade_log: &mut TradeLogger,
    to_ts: Option<i64>,
    history_batch_limit: usize,
    history_max_gap_min: i64,
) -> anyhow::Result<Option<i64>> {
    let mut current_from_ts = from_ts;
    let mut last_seen_ts = None;

    loop {
        let (ws_data, _) = connect_async(WS_URL).await?;
        let (mut ws_sink, mut ws_stream) = ws_data.split();

        let (first_msg, subscribe_rt_ms) = subscribe_bars(
            &mut ws_sink,
            &mut ws_stream,
            access_token,
            symbol,
            exchange,
            instrument_group,
            timeframe_sec,
            current_from_ts,
            skip_history,
            split_adjust,
            format,
            frequency_ms,
        )
        .await?;

        let bars = read_history_batch(
            &mut ws_stream,
            first_msg,
            history_max_gap_min,
        )
        .await?;
        info!(
            "HISTORY batch from_ts={} bars={} rt_ms={:.2}",
            current_from_ts,
            bars.len(),
            subscribe_rt_ms
        );

        if bars.is_empty() {
            warn!("HISTORY batch empty, stopping history fetch");
            return Ok(last_seen_ts);
        }

        for bar in &bars {
            let bar = *bar;
            if let Some(to_ts) = to_ts {
                if bar.time.timestamp() >= to_ts {
                    return Ok(Some(bar.time.timestamp()));
                }
            }
            strategy.on_bar(bar, trade_log)?;
            last_seen_ts = Some(bar.time.timestamp());
        }

        if bars.len() < history_batch_limit {
            return Ok(last_seen_ts);
        }

        let next_from_ts = last_seen_ts.unwrap_or(current_from_ts) + timeframe_sec;
        current_from_ts = next_from_ts;
    }
}

async fn read_history_batch(
    ws_stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    first_msg: Value,
    history_max_gap_min: i64,
) -> anyhow::Result<Vec<Bar>> {
    let mut bars = extract_bars(&first_msg);
    let mut idle_rounds = 0;
    let mut last_ts = bars.last().map(|bar| bar.time.timestamp());

    loop {
        match timeout(Duration::from_millis(500), ws_stream.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                if let Ok(payload) = serde_json::from_str::<Value>(&txt) {
                    let incoming = extract_bars(&payload);
                    for bar in incoming {
                        if let Some(prev_ts) = last_ts {
                            let diff_min = (bar.time.timestamp() - prev_ts) / 60;
                            if diff_min > history_max_gap_min {
                                info!(
                                    "HISTORY gap {} min exceeds max {}, stopping batch at {}",
                                    diff_min, history_max_gap_min, bar.time
                                );
                                return Ok(bars);
                            }
                        }
                        last_ts = Some(bar.time.timestamp());
                        bars.push(bar);
                    }
                }
                idle_rounds = 0;
            }
            Ok(Some(Ok(Message::Ping(_)))) => {
                idle_rounds = 0;
            }
            Ok(Some(Ok(Message::Close(_)))) => {
                break;
            }
            Ok(Some(Ok(_))) => {
                idle_rounds = 0;
            }
            Ok(Some(Err(error))) => return Err(error.into()),
            Ok(None) => break,
            Err(_) => {
                idle_rounds += 1;
                if idle_rounds >= 2 {
                    break;
                }
            }
        }
    }

    Ok(bars)
}

fn extract_bars(payload: &Value) -> Vec<Bar> {
    let mut bars = if let Some(data) = payload.get("data") {
        extract_bars_from_value(data)
    } else {
        extract_bars_from_value(payload)
    };

    bars.sort_by_key(|bar| bar.time.timestamp());
    if let (Some(first), Some(last)) = (bars.first(), bars.last()) {
        debug!(
            "bars batch size={} first_ts={} last_ts={}",
            bars.len(),
            first.time,
            last.time
        );
    }
    bars
}

fn extract_bars_from_value(value: &Value) -> Vec<Bar> {
    match value {
        Value::Array(items) => items.iter().filter_map(parse_bar).collect(),
        Value::Object(map) => {
            if let Some(bars_value) = map.get("bars") {
                return extract_bars_from_value(bars_value);
            }
            parse_bar(value).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn parse_bar(value: &Value) -> Option<Bar> {
    match value {
        Value::Array(items) => parse_bar_from_array(items),
        Value::Object(_) => parse_bar_from_object(value),
        _ => None,
    }
}

fn parse_bar_from_array(items: &[Value]) -> Option<Bar> {
    if items.len() < 5 {
        return None;
    }

    let time = parse_time(&items[0])?;
    let open = parse_f64(&items[1])?;
    let high = parse_f64(&items[2])?;
    let low = parse_f64(&items[3])?;
    let close = parse_f64(&items[4])?;

    Some(Bar {
        time,
        open,
        high,
        low,
        close,
    })
}

fn parse_bar_from_object(value: &Value) -> Option<Bar> {
    let time = value
        .get("time")
        .or_else(|| value.get("timestamp"))
        .or_else(|| value.get("t"))
        .and_then(parse_time)?;
    let open = value
        .get("open")
        .or_else(|| value.get("o"))
        .and_then(parse_f64)?;
    let high = value
        .get("high")
        .or_else(|| value.get("h"))
        .and_then(parse_f64)?;
    let low = value
        .get("low")
        .or_else(|| value.get("l"))
        .and_then(parse_f64)?;
    let close = value
        .get("close")
        .or_else(|| value.get("c"))
        .and_then(parse_f64)?;

    Some(Bar {
        time,
        open,
        high,
        low,
        close,
    })
}

fn parse_time(value: &Value) -> Option<DateTime<FixedOffset>> {
    match value {
        Value::Number(num) => num.as_i64().and_then(ts_to_datetime),
        Value::String(s) => {
            if let Ok(ts) = s.parse::<i64>() {
                return ts_to_datetime(ts);
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&moscow_offset()));
            }
            None
        }
        _ => None,
    }
}

fn ts_to_datetime(ts: i64) -> Option<DateTime<FixedOffset>> {
    let ts = if ts > 1_000_000_000_000 {
        ts / 1000
    } else {
        ts
    };
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.with_timezone(&moscow_offset()))
}

fn parse_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(num) => num.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with_ansi(cfg!(debug_assertions))
        .init();
}

fn get_env_or_default(key: &str, default: &str) -> String {
    std::env::var(key)
        .map(|v| v.trim_matches('"').trim().to_string())
        .unwrap_or_else(|_| default.to_string())
}

async fn get_access_token() -> anyhow::Result<String> {
    let refresh = std::env::var("ALOR_REFRESH_TOKEN")
        .map(|v| v.trim_matches('"').trim().to_string())
        .unwrap_or_default();

    if refresh.is_empty() {
        anyhow::bail!("Нужно задать ALOR_REFRESH_TOKEN в окружении");
    }

    let t0 = Instant::now();
    let response = reqwest::Client::new()
        .post(OAUTH_URL)
        .query(&[("token", refresh)])
        .send()
        .await?;
    let dt = duration_ms(t0.elapsed());

    let payload: Value = response.error_for_status()?.json().await?;
    let access = payload
        .get("AccessToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("AccessToken not in response: {payload}"))?;

    info!("get_access_token_ms={dt:.2}");

    Ok(access.trim().to_string())
}

async fn subscribe_bars(
    sink: &mut (impl futures_util::sink::Sink<Message, Error = WsError> + Unpin),
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    access_token: &str,
    symbol: &str,
    exchange: &str,
    instrument_group: &str,
    timeframe_sec: i64,
    from_ts: i64,
    skip_history: bool,
    split_adjust: bool,
    format: &str,
    frequency_ms: i64,
) -> anyhow::Result<(Value, f64)> {
    let guid = new_guid();
    let msg = serde_json::json!({
        "opcode": "BarsGetAndSubscribe",
        "exchange": exchange,
        "code": symbol,
        "instrumentGroup": instrument_group,
        "tf": timeframe_sec,
        "from": from_ts,
        "skipHistory": skip_history,
        "splitAdjust": split_adjust,
        "format": format,
        "frequency": frequency_ms,
        "guid": guid,
        "token": access_token,
    });

    let payload = serde_json::to_string(&msg)?;
    let t0 = Instant::now();
    sink.send(Message::Text(payload.into())).await?;

    let resp = read_until_guid(stream, &guid, Duration::from_secs(5)).await?;
    let dt = duration_ms(t0.elapsed());

    if resp.get("httpCode").and_then(Value::as_i64) == Some(200) {
        return Ok((resp, dt));
    }

    if resp.get("data").is_some() {
        return Ok((resp, dt));
    }

    anyhow::bail!("BarsGetAndSubscribe failed: {resp}");
}

async fn read_until_guid(
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, WsError>> + Unpin),
    guid: &str,
    timeout_dur: Duration,
) -> anyhow::Result<Value> {
    let fut = async move {
        while let Some(msg) = stream.next().await {
            let msg = msg?;
            if let Message::Text(txt) = msg {
                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                    if guid_of(&val).as_deref() == Some(guid) {
                        return Ok(val);
                    }
                    if val.get("httpCode").is_some() {
                        return Ok(val);
                    }
                }
            }
        }
        anyhow::bail!("WS stream ended before response");
    };

    match timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => anyhow::bail!("WS subscribe timeout"),
    }
}

fn guid_of(event: &Value) -> Option<String> {
    event
        .get("requestGuid")
        .or_else(|| event.get("guid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn duration_ms(dur: Duration) -> f64 {
    dur.as_secs_f64() * 1000.0
}

fn start_of_utc_day() -> DateTime<Utc> {
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
}

fn moscow_offset() -> FixedOffset {
    FixedOffset::east_opt(MOSCOW_OFFSET_HOURS * 3600)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())
}

fn parse_date_env(key: &str) -> Option<i64> {
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(parse_date)
}

fn parse_date(value: &str) -> Option<i64> {
    let value = value.trim();
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    let dt = Utc
        .with_ymd_and_hms(parsed.year(), parsed.month(), parsed.day(), 0, 0, 0)
        .single()?;
    Some(dt.timestamp())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" => Some(true),
        "0" | "false" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn new_guid() -> String {
    use rand::{Rng, distr::Alphanumeric};

    let rng = rand::rng();
    rng.sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}