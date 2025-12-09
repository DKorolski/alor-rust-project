use std::{
    collections::{BTreeMap, HashMap},
    fs::{OpenOptions, metadata},
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, TimeZone, Utc};
use csv::Writer;
use futures::{SinkExt, StreamExt, pin_mut};
use rust_decimal::prelude::*;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{select, signal, time::sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

const BYBIT_PUBLIC_LINEAR_WS: &str = "wss://stream.bybit.com/v5/public/linear";
type AppResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StrategyConfig {
    symbols: Vec<String>,
    depth: usize,
    tick_size: Decimal,
    order_size: Decimal,
    fee_rate_maker: Decimal,
    fee_rate_taker: Decimal,
    broker_fee_abs: Decimal,
    adverse_ticks: Decimal,
    tp_ticks: Decimal,
    entry_cluster_q: f64,
    max_cluster_depth_entry: usize,
    exit_cluster_q: f64,
    max_cluster_depth_exit: usize,
    min_history_for_quantiles: usize,
    exit_order_timeout_ms: u64,
    entry_order_timeout_ms: u64,
    exit_cluster_max_diff_ticks: Decimal,
    csv_path: String,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            symbols: vec!["BTCUSDT".to_string()],
            depth: 200,
            tick_size: dec!(0.1),
            order_size: dec!(1),
            fee_rate_maker: dec!(0),
            fee_rate_taker: dec!(0.000066),
            broker_fee_abs: dec!(1),
            adverse_ticks: dec!(50),
            tp_ticks: dec!(200),
            entry_cluster_q: 0.895,
            max_cluster_depth_entry: 12,
            exit_cluster_q: 0.05,
            max_cluster_depth_exit: 12,
            min_history_for_quantiles: 50,
            exit_order_timeout_ms: 1_500,
            entry_order_timeout_ms: 800,
            exit_cluster_max_diff_ticks: dec!(197),
            csv_path: "trades_live_v10_1.csv".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TradeResult {
    entry_time: DateTime<Utc>,
    exit_time: DateTime<Utc>,
    direction: String,
    side: i32,
    entry_price: Decimal,
    exit_price: Decimal,
    gross_ticks: Decimal,
    net_ticks: Decimal,
    gross_ret: Decimal,
    net_ret: Decimal,
    reason: String,
    exit_liquidity: String,
    tp_plain: Decimal,
    exit_from_cluster: bool,
    entry_fee: Decimal,
    exit_fee: Decimal,
    total_fee: Decimal,
    waited_ms_for_fill: i64,
    hold_ms: i64,
}

#[derive(Default)]
struct OrderbookL2 {
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
}

impl OrderbookL2 {
    fn apply_levels(side: &mut BTreeMap<Decimal, Decimal>, levels: &[[String; 2]]) {
        for [price, size] in levels {
            if let (Ok(price), Ok(size)) = (Decimal::from_str(price), Decimal::from_str(size)) {
                if size.is_zero() {
                    side.remove(&price);
                } else {
                    side.insert(price, size);
                }
            }
        }
    }

    fn apply_snapshot(&mut self, data: &OrderbookData) {
        self.bids.clear();
        self.asks.clear();
        Self::apply_levels(&mut self.bids, &data.bids);
        Self::apply_levels(&mut self.asks, &data.asks);
    }

    fn apply_delta(&mut self, data: &OrderbookData) {
        Self::apply_levels(&mut self.bids, &data.bids);
        Self::apply_levels(&mut self.asks, &data.asks);
    }

    fn top_levels(&self, depth: usize) -> (Vec<Level>, Vec<Level>) {
        let bids = self
            .bids
            .iter()
            .rev()
            .take(depth)
            .map(|(p, s)| Level {
                price: *p,
                size: *s,
            })
            .collect();
        let asks = self
            .asks
            .iter()
            .take(depth)
            .map(|(p, s)| Level {
                price: *p,
                size: *s,
            })
            .collect();
        (bids, asks)
    }
}

#[derive(Debug, Clone, Copy)]
struct Level {
    price: Decimal,
    size: Decimal,
}

#[derive(Debug)]
struct ClusterInfo {
    price: Decimal,
    size: Decimal,
    depth: usize,
}

#[derive(Debug)]
struct StrategyEngine {
    cfg: StrategyConfig,
    state: TradeState,
    side: i32,
    entry_price: Option<Decimal>,
    entry_ts: Option<i64>,
    entry_time: Option<DateTime<Utc>>,
    entry_order_ts: Option<i64>,
    order_price: Option<Decimal>,
    queue_ahead: Decimal,
    exit_order_price: Option<Decimal>,
    exit_order_ts: Option<i64>,
    exit_queue_ahead: Decimal,
    exit_reason_target: Option<String>,
    favorable_price: Option<Decimal>,
    tp_plain: Option<Decimal>,
    bid_cluster_hist: Vec<Decimal>,
    ask_cluster_hist: Vec<Decimal>,
    bid_entry_thr: Option<Decimal>,
    ask_entry_thr: Option<Decimal>,
    bid_exit_thr: Option<Decimal>,
    ask_exit_thr: Option<Decimal>,
    last_trade_price: Option<Decimal>,
    last_mid: Option<Decimal>,
    last_time: Option<DateTime<Utc>>,
    csv_writer: Arc<Mutex<Writer<std::fs::File>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TradeState {
    Flat,
    WaitingEntry,
    InPosition,
}

impl StrategyEngine {
    fn new(cfg: StrategyConfig, csv_writer: Arc<Mutex<Writer<std::fs::File>>>) -> Self {
        Self {
            cfg,
            state: TradeState::Flat,
            side: 0,
            entry_price: None,
            entry_ts: None,
            entry_time: None,
            entry_order_ts: None,
            order_price: None,
            queue_ahead: Decimal::ZERO,
            exit_order_price: None,
            exit_order_ts: None,
            exit_queue_ahead: Decimal::ZERO,
            exit_reason_target: None,
            favorable_price: None,
            tp_plain: None,
            bid_cluster_hist: Vec::new(),
            ask_cluster_hist: Vec::new(),
            bid_entry_thr: None,
            ask_entry_thr: None,
            bid_exit_thr: None,
            ask_exit_thr: None,
            last_trade_price: None,
            last_mid: None,
            last_time: None,
            csv_writer,
        }
    }

    fn thresholds_ready(&self) -> bool {
        self.bid_entry_thr.is_some()
            && self.ask_entry_thr.is_some()
            && self.bid_exit_thr.is_some()
            && self.ask_exit_thr.is_some()
    }

    fn update_cluster_history(&mut self, bid: Decimal, ask: Decimal) {
        if bid > Decimal::ZERO {
            self.bid_cluster_hist.push(bid);
        }
        if ask > Decimal::ZERO {
            self.ask_cluster_hist.push(ask);
        }

        if self.bid_cluster_hist.len() >= self.cfg.min_history_for_quantiles
            && self.ask_cluster_hist.len() >= self.cfg.min_history_for_quantiles
        {
            self.bid_entry_thr = Some(quantile(&self.bid_cluster_hist, self.cfg.entry_cluster_q));
            self.ask_entry_thr = Some(quantile(&self.ask_cluster_hist, self.cfg.entry_cluster_q));
            self.bid_exit_thr = Some(quantile(&self.bid_cluster_hist, self.cfg.exit_cluster_q));
            self.ask_exit_thr = Some(quantile(&self.ask_cluster_hist, self.cfg.exit_cluster_q));
        }
    }

    fn on_orderbook(
        &mut self,
        ts: i64,
        best_bid: Level,
        best_ask: Level,
        bid_cluster: ClusterInfo,
        ask_cluster: ClusterInfo,
    ) {
        let cur_time = Utc
            .timestamp_millis_opt(ts)
            .single()
            .unwrap_or_else(Utc::now);
        self.last_mid = Some((best_bid.price + best_ask.price) / dec!(2));
        self.last_time = Some(cur_time);

        self.update_cluster_history(bid_cluster.size, ask_cluster.size);

        if self.state == TradeState::InPosition {
            if let Some(entry) = self.entry_price {
                self.favorable_price = match (self.side, self.favorable_price) {
                    (1, Some(best)) => Some(best.max(best_bid.price.max(entry))),
                    (1, None) => Some(best_bid.price.max(entry)),
                    (-1, Some(best)) => Some(best.min(best_ask.price.min(entry))),
                    (-1, None) => Some(best_ask.price.min(entry)),
                    _ => self.favorable_price,
                };
            }
        }

        // Entry limit timeout
        if self.state == TradeState::WaitingEntry {
            if let Some(order_ts) = self.entry_order_ts {
                if ts - order_ts >= self.cfg.entry_order_timeout_ms as i64 {
                    debug!("Entry limit timeout -> cancel order");
                    self.reset_entry_wait();
                }
            }
        }

        // Exit limit timeout
        if self.state == TradeState::InPosition {
            if let (Some(exit_ts), Some(_)) = (self.exit_order_ts, self.exit_order_price) {
                if ts - exit_ts >= self.cfg.exit_order_timeout_ms as i64 {
                    debug!("Exit limit timeout -> cancel exit order");
                    self.exit_order_price = None;
                    self.exit_order_ts = None;
                    self.exit_queue_ahead = Decimal::ZERO;
                    self.exit_reason_target = None;
                }
            }
        }

        // Cluster exit placement
        if self.state == TradeState::InPosition
            && self.exit_order_price.is_none()
            && self.tp_plain.is_some()
            && self.thresholds_ready()
        {
            match self.side {
                1 => {
                    if let (Some(thr), Some(tp)) = (self.ask_exit_thr, self.tp_plain) {
                        if ask_cluster.size >= thr
                            && ask_cluster.depth <= self.cfg.max_cluster_depth_exit
                            && (ask_cluster.price - tp).abs()
                                <= self.cfg.exit_cluster_max_diff_ticks * self.cfg.tick_size
                        {
                            let candidate = ask_cluster.price - self.cfg.tick_size;
                            debug!("Place cluster-exit limit (long) at {}", candidate);
                            self.exit_order_price = Some(candidate);
                            self.exit_order_ts = Some(ts);
                            self.exit_queue_ahead = Decimal::ZERO;
                            self.exit_reason_target = Some("cluster_exit".to_string());
                        }
                    }
                }
                -1 => {
                    if let (Some(thr), Some(tp)) = (self.bid_exit_thr, self.tp_plain) {
                        if bid_cluster.size >= thr
                            && bid_cluster.depth <= self.cfg.max_cluster_depth_exit
                            && (bid_cluster.price - tp).abs()
                                <= self.cfg.exit_cluster_max_diff_ticks * self.cfg.tick_size
                        {
                            let candidate = bid_cluster.price + self.cfg.tick_size;
                            debug!("Place cluster-exit limit (short) at {}", candidate);
                            self.exit_order_price = Some(candidate);
                            self.exit_order_ts = Some(ts);
                            self.exit_queue_ahead = Decimal::ZERO;
                            self.exit_reason_target = Some("cluster_exit".to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // Adverse exit
        if self.state == TradeState::InPosition
            && self.exit_order_price.is_none()
            && self.entry_price.is_some()
            && self.favorable_price.is_some()
        {
            match self.side {
                1 => {
                    let adverse_ticks =
                        (self.favorable_price.unwrap() - best_bid.price) / self.cfg.tick_size;
                    if adverse_ticks >= self.cfg.adverse_ticks {
                        let queue = (best_ask.size - self.cfg.order_size).max(Decimal::ZERO);
                        self.exit_order_price = Some(best_ask.price);
                        self.exit_order_ts = Some(ts);
                        self.exit_queue_ahead = queue;
                        self.exit_reason_target = Some("adverse_exit".to_string());
                        debug!(
                            "Place adverse-exit (long) at {} adverse_ticks={}",
                            best_ask.price, adverse_ticks
                        );
                    }
                }
                -1 => {
                    let adverse_ticks =
                        (best_ask.price - self.favorable_price.unwrap()) / self.cfg.tick_size;
                    if adverse_ticks >= self.cfg.adverse_ticks {
                        let queue = (best_bid.size - self.cfg.order_size).max(Decimal::ZERO);
                        self.exit_order_price = Some(best_bid.price);
                        self.exit_order_ts = Some(ts);
                        self.exit_queue_ahead = queue;
                        self.exit_reason_target = Some("adverse_exit".to_string());
                        debug!(
                            "Place adverse-exit (short) at {} adverse_ticks={}",
                            best_bid.price, adverse_ticks
                        );
                    }
                }
                _ => {}
            }
        }

        // Entry signals
        if self.state == TradeState::Flat && self.thresholds_ready() {
            if let (Some(bid_thr), Some(ask_thr)) = (self.bid_entry_thr, self.ask_entry_thr) {
                let is_bid_entry = bid_cluster.size >= bid_thr
                    && bid_cluster.depth <= self.cfg.max_cluster_depth_entry
                    && bid_cluster.size > ask_cluster.size;

                let is_ask_entry = ask_cluster.size >= ask_thr
                    && ask_cluster.depth <= self.cfg.max_cluster_depth_entry
                    && ask_cluster.size > bid_cluster.size;

                if is_bid_entry {
                    self.state = TradeState::WaitingEntry;
                    self.side = 1;
                    self.order_price = Some(bid_cluster.price - self.cfg.tick_size);
                    self.entry_order_ts = Some(ts);
                    self.queue_ahead = (best_bid.size - self.cfg.order_size).max(Decimal::ZERO);
                    debug!("LONG signal at {}", ts);
                } else if is_ask_entry {
                    self.state = TradeState::WaitingEntry;
                    self.side = -1;
                    self.order_price = Some(ask_cluster.price + self.cfg.tick_size);
                    self.entry_order_ts = Some(ts);
                    self.queue_ahead = (best_ask.size - self.cfg.order_size).max(Decimal::ZERO);
                    debug!("SHORT signal at {}", ts);
                }
            }
        }
    }

    fn on_trade(&mut self, ts: i64, price: Decimal, size: Decimal) {
        let cur_time = Utc
            .timestamp_millis_opt(ts)
            .single()
            .unwrap_or_else(Utc::now);
        self.last_trade_price = Some(price);
        self.last_time = Some(cur_time);

        if self.state == TradeState::WaitingEntry {
            if let Some(order_price) = self.order_price {
                match self.side {
                    1 if price <= order_price => self.queue_ahead -= size,
                    -1 if price >= order_price => self.queue_ahead -= size,
                    _ => {}
                }

                if self.queue_ahead <= Decimal::ZERO {
                    self.fill_entry(ts, cur_time, order_price);
                }
            }
        }

        if self.state == TradeState::InPosition && self.exit_order_price.is_some() {
            let exit_price = self.exit_order_price.unwrap();
            match self.side {
                1 if price >= exit_price => self.exit_queue_ahead -= size,
                -1 if price <= exit_price => self.exit_queue_ahead -= size,
                _ => {}
            }

            if self.exit_queue_ahead <= Decimal::ZERO {
                self.close_position(ts, cur_time);
            }
        }
    }

    fn fill_entry(&mut self, ts: i64, cur_time: DateTime<Utc>, price: Decimal) {
        info!("Entry filled at {} side={}", price, self.side);
        self.state = TradeState::InPosition;
        self.entry_price = Some(price);
        self.entry_ts = Some(ts);
        self.entry_time = Some(cur_time);

        self.tp_plain = Some(if self.side == 1 {
            price + self.cfg.tp_ticks * self.cfg.tick_size
        } else {
            price - self.cfg.tp_ticks * self.cfg.tick_size
        });
        self.favorable_price = Some(price);

        self.order_price = None;
        self.entry_order_ts = Some(ts);
        self.queue_ahead = Decimal::ZERO;
        self.exit_order_price = None;
        self.exit_order_ts = None;
        self.exit_queue_ahead = Decimal::ZERO;
        self.exit_reason_target = None;
    }

    fn close_position(&mut self, _ts: i64, cur_time: DateTime<Utc>) {
        if let (Some(entry_price), Some(entry_ts), Some(entry_time)) =
            (self.entry_price, self.entry_ts, self.entry_time)
        {
            let exit_price = self.exit_order_price.unwrap_or(entry_price);
            let reason = self
                .exit_reason_target
                .clone()
                .unwrap_or_else(|| "limit_exit".to_string());

            let side = self.side;
            let cfg = &self.cfg;

            let pnl_px = Decimal::from(side) * (exit_price - entry_price) * cfg.order_size;
            let entry_fee = cfg.fee_rate_maker * entry_price * cfg.order_size + cfg.broker_fee_abs;
            let exit_fee = cfg.fee_rate_maker * exit_price * cfg.order_size + cfg.broker_fee_abs;
            let total_fee = entry_fee + exit_fee;

            let notional_entry = entry_price * cfg.order_size;
            let gross_ret = pnl_px / notional_entry;
            let net_ret = (pnl_px - total_fee) / notional_entry;

            let gross_ticks = Decimal::from(side) * (exit_price - entry_price) / cfg.tick_size;
            let rel_tick = cfg.tick_size / entry_price;
            let net_ticks = net_ret / rel_tick;

            let waited_ms = self
                .entry_order_ts
                .map(|ord_ts| entry_ts - ord_ts)
                .unwrap_or(0);
            let hold_ms = cur_time.timestamp_millis() - entry_time.timestamp_millis();

            let record = TradeResult {
                entry_time,
                exit_time: cur_time,
                direction: if side == 1 { "long" } else { "short" }.to_string(),
                side,
                entry_price,
                exit_price,
                gross_ticks,
                net_ticks,
                gross_ret,
                net_ret,
                reason: reason.clone(),
                exit_liquidity: "maker".to_string(),
                tp_plain: self.tp_plain.unwrap_or(entry_price),
                exit_from_cluster: reason == "cluster_exit",
                entry_fee,
                exit_fee,
                total_fee,
                waited_ms_for_fill: waited_ms,
                hold_ms,
            };

            if let Ok(mut writer) = self.csv_writer.lock() {
                if let Err(err) = writer.serialize(&record) {
                    error!(?err, "failed to write trade to csv");
                }
                if let Err(err) = writer.flush() {
                    error!(?err, "failed to flush trade csv");
                }
            }

            info!(
                "Closed trade: entry={} exit={} reason={}",
                entry_price, exit_price, reason
            );
        }

        self.reset_after_close();
    }

    fn reset_entry_wait(&mut self) {
        self.state = TradeState::Flat;
        self.side = 0;
        self.order_price = None;
        self.entry_order_ts = None;
        self.queue_ahead = Decimal::ZERO;
    }

    fn reset_after_close(&mut self) {
        self.state = TradeState::Flat;
        self.side = 0;
        self.entry_price = None;
        self.entry_ts = None;
        self.entry_time = None;
        self.order_price = None;
        self.entry_order_ts = None;
        self.queue_ahead = Decimal::ZERO;
        self.exit_order_price = None;
        self.exit_order_ts = None;
        self.exit_queue_ahead = Decimal::ZERO;
        self.exit_reason_target = None;
        self.favorable_price = None;
        self.tp_plain = None;
    }

    fn force_flatten(&mut self) {
        if self.state == TradeState::InPosition {
            let exit_price = self
                .last_trade_price
                .or(self.last_mid)
                .unwrap_or(self.entry_price.unwrap());
            let exit_time = self.last_time.unwrap_or_else(Utc::now);
            self.exit_order_price = Some(exit_price);
            self.exit_reason_target = Some("eod_force".to_string());
            self.close_position(exit_time.timestamp_millis(), exit_time);
        }
    }
}

fn quantile(data: &[Decimal], q: f64) -> Decimal {
    if data.is_empty() {
        return Decimal::ZERO;
    }
    let mut values: Vec<f64> = data.iter().filter_map(|d| d.to_f64()).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n == 1 {
        return Decimal::from_f64(values[0]).unwrap_or(Decimal::ZERO);
    }
    let pos = q.clamp(0.0, 1.0) * (n - 1) as f64;
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    if lower == upper {
        Decimal::from_f64(values[lower]).unwrap_or(Decimal::ZERO)
    } else {
        let weight = Decimal::from_f64(pos - lower as f64).unwrap_or(Decimal::ZERO);
        let lower_v = Decimal::from_f64(values[lower]).unwrap_or(Decimal::ZERO);
        let upper_v = Decimal::from_f64(values[upper]).unwrap_or(Decimal::ZERO);
        lower_v + (upper_v - lower_v) * weight
    }
}

#[derive(Deserialize)]
struct WsEnvelope {
    topic: Option<String>,
    #[serde(rename = "type")]
    typ: Option<String>,
    ts: Option<i64>,
    data: Option<Value>,
}

#[derive(Deserialize, Default)]
struct OrderbookData {
    #[serde(default, rename = "b")]
    bids: Vec<[String; 2]>,
    #[serde(default, rename = "a")]
    asks: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct TradeMsg {
    #[serde(rename = "T")]
    ts: Option<i64>,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "v")]
    size: String,
}

struct StrategyRunner {
    cfg: StrategyConfig,
    orderbooks: HashMap<String, OrderbookL2>,
    engines: HashMap<String, StrategyEngine>,
}

impl StrategyRunner {
    fn new(cfg: StrategyConfig) -> AppResult<Self> {
        let mut orderbooks = HashMap::new();
        let mut engines = HashMap::new();

        let file_has_data = metadata(&cfg.csv_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        let csv_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cfg.csv_path)?;
        let mut writer = Writer::from_writer(csv_file);
        if !file_has_data {
            writer.write_record([
                "entry_time",
                "exit_time",
                "direction",
                "side",
                "entry_price",
                "exit_price",
                "gross_ticks",
                "net_ticks",
                "gross_ret",
                "net_ret",
                "reason",
                "exit_liquidity",
                "tp_plain",
                "exit_from_cluster",
                "entry_fee",
                "exit_fee",
                "total_fee",
                "waited_ms_for_fill",
                "hold_ms",
            ])?;
            writer.flush()?;
        }
        let writer = Arc::new(Mutex::new(writer));

        for symbol in &cfg.symbols {
            orderbooks.insert(symbol.clone(), OrderbookL2::default());
            let writer_clone = writer.clone();
            engines.insert(
                symbol.clone(),
                StrategyEngine::new(cfg.clone(), writer_clone),
            );
        }

        Ok(Self {
            cfg,
            orderbooks,
            engines,
        })
    }

    fn symbol_from_topic(topic: &str) -> Option<String> {
        topic.split('.').last().map(|s| s.to_string())
    }

    fn handle_orderbook(&mut self, topic: &str, typ: &str, ts: i64, data: &Value) {
        let Some(symbol) = Self::symbol_from_topic(topic) else {
            return;
        };
        let Some(book) = self.orderbooks.get_mut(&symbol) else {
            return;
        };
        let Some(engine) = self.engines.get_mut(&symbol) else {
            return;
        };

        let data: OrderbookData = match serde_json::from_value(data.clone()) {
            Ok(d) => d,
            Err(err) => {
                warn!(?err, "Failed to decode orderbook data");
                return;
            }
        };

        if typ == "snapshot" {
            book.apply_snapshot(&data);
        } else {
            book.apply_delta(&data);
        }

        let max_depth = self
            .cfg
            .max_cluster_depth_entry
            .max(self.cfg.max_cluster_depth_exit)
            .max(1);
        let (bids, asks) = book.top_levels(max_depth);
        if bids.is_empty() || asks.is_empty() {
            return;
        }
        let best_bid = bids[0];
        let best_ask = asks[0];

        let bid_cluster = bids
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.size.partial_cmp(&b.size).unwrap())
            .map(|(idx, lvl)| ClusterInfo {
                price: lvl.price,
                size: lvl.size,
                depth: idx,
            })
            .unwrap();

        let ask_cluster = asks
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.size.partial_cmp(&b.size).unwrap())
            .map(|(idx, lvl)| ClusterInfo {
                price: lvl.price,
                size: lvl.size,
                depth: idx,
            })
            .unwrap();

        engine.on_orderbook(ts, best_bid, best_ask, bid_cluster, ask_cluster);
    }

    fn handle_trades(&mut self, topic: &str, _ts: i64, data: &Value) {
        let Some(symbol) = Self::symbol_from_topic(topic) else {
            return;
        };
        let Some(engine) = self.engines.get_mut(&symbol) else {
            return;
        };
        let trades: Vec<TradeMsg> = match serde_json::from_value(data.clone()) {
            Ok(t) => t,
            Err(err) => {
                warn!(?err, "Failed to decode trades");
                return;
            }
        };

        for trade in trades {
            if let (Some(ts_trade), Ok(price), Ok(size)) = (
                trade.ts,
                Decimal::from_str(&trade.price),
                Decimal::from_str(&trade.size),
            ) {
                engine.on_trade(ts_trade, price, size);
            }
        }
    }

    fn force_flatten(&mut self) {
        for engine in self.engines.values_mut() {
            engine.force_flatten();
        }
    }
}

#[tokio::main]
async fn main() -> AppResult<()> {
    barter::logging::init_logging();
    let cfg = StrategyConfig::default();
    info!("Starting live scalping for symbols: {:?}", cfg.symbols);

    let mut runner = StrategyRunner::new(cfg.clone())?;
    let topics: Vec<String> = cfg
        .symbols
        .iter()
        .flat_map(|s| {
            [
                format!("orderbook.{}.{}", cfg.depth, s),
                format!("publicTrade.{}", s),
            ]
        })
        .collect();

    let mut backoff = [3u64, 5, 10, 30, 60].into_iter().cycle();

    loop {
        info!("Connecting to {}", BYBIT_PUBLIC_LINEAR_WS);
        match connect_async(BYBIT_PUBLIC_LINEAR_WS).await {
            Ok((ws_stream, _)) => {
                let (mut write, mut read) = ws_stream.split();
                let sub_msg = serde_json::json!({
                    "op": "subscribe",
                    "args": topics,
                });
                write
                    .send(Message::Text(sub_msg.to_string().into()))
                    .await?;
                info!("Subscribed to live feeds");

                let shutdown = signal::ctrl_c();
                pin_mut!(shutdown);
                loop {
                    select! {
                        _ = &mut shutdown => {
                            info!("Shutdown signal received");
                            runner.force_flatten();
                            return Ok(());
                        }
                        msg = read.next() => {
                            match msg {
                                Some(Ok(Message::Text(txt))) => {
                                    if let Err(err) = handle_message(&txt, &mut runner) {
                                        warn!(?err, "Failed to process message");
                                    }
                                }
                                Some(Ok(Message::Ping(_))) => {}
                                Some(Ok(Message::Pong(_))) => {}
                                Some(Ok(Message::Binary(_))) => {}
                                Some(Ok(Message::Frame(_))) => {}
                                Some(Ok(Message::Close(frame))) => {
                                    warn!(?frame, "WebSocket closed by server");
                                    break;
                                }
                                Some(Err(err)) => {
                                    warn!(?err, "WebSocket error");
                                    break;
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
            Err(err) => warn!(?err, "WebSocket connect error"),
        }

        let delay = backoff.next().unwrap_or(30);
        warn!(?delay, "Reconnecting after backoff");
        sleep(Duration::from_secs(delay)).await;
    }
}

fn handle_message(raw: &str, runner: &mut StrategyRunner) -> AppResult<()> {
    let env: WsEnvelope = serde_json::from_str(raw)?;
    let Some(topic) = env.topic else {
        return Ok(());
    };
    let typ = env.typ.unwrap_or_default();
    let ts = env.ts.unwrap_or_else(|| Utc::now().timestamp_millis());

    match topic.as_str() {
        t if t.starts_with("orderbook.") => {
            if let Some(data) = env.data {
                runner.handle_orderbook(t, &typ, ts, &data);
            }
        }
        t if t.starts_with("publicTrade.") => {
            if let Some(data) = env.data {
                runner.handle_trades(t, ts, &data);
            }
        }
        _ => {}
    }

    Ok(())
}