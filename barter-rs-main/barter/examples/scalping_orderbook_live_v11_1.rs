use std::{cmp::Ordering, fs::File, io, path::Path};

use barter_data::{
    books::OrderBook,
    event::{DataKind, MarketEvent},
    exchange::bybit::futures::BybitPerpetualsUsd,
    streams::{
        Streams,
        consumer::MarketStreamResult,
        reconnect::{Event, stream::ReconnectingStream},
    },
    subscription::{book::OrderBookEvent, trade::PublicTrades},
};
use barter_instrument::instrument::market_data::{
    MarketDataInstrument, kind::MarketDataInstrumentKind,
};
use chrono::{DateTime, TimeZone, Utc};
use csv::Writer;
use futures::StreamExt;
use rust_decimal::{Decimal, prelude::ToPrimitive};
use tokio::signal;
use tracing::{info, warn};

/// Minimal live translation of the Python scalping prototype into Rust using Barter's Bybit
/// connectors. The example subscribes to Bybit USDT-perpetual trades & L2 order books, maintains a
/// local book, and runs the cluster-driven entry/exit logic against the live feed.
#[derive(Clone)]
struct StrategyConfig {
    symbol: &'static str,
    depth: usize,

    tick_size: f64,
    order_size: f64,

    fee_rate_maker: f64,
    fee_rate_taker: f64,
    broker_fee_abs: f64,

    tp_ticks: i64,

    entry_cluster_q: f64,
    max_cluster_depth_entry: usize,

    exit_cluster_q: f64,
    max_cluster_depth_exit: usize,
    exit_order_timeout_ms: i64,
    exit_cluster_max_diff_ticks: i64,
    exit_cluster_start_ms: i64,

    entry_order_timeout_ms: i64,
    min_history_for_quantiles: usize,
    adverse_ticks: i64,
    cluster_diff: f64,

    entry_bid_thr: Option<f64>,
    entry_ask_thr: Option<f64>,
    exit_bid_thr: Option<f64>,
    exit_ask_thr: Option<f64>,

    csv_path: &'static str,
    order_placement_delay_ms: i64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            symbol: "BTCUSDT",
            depth: 50,
            tick_size: 0.1,
            order_size: 1.0,
            fee_rate_maker: 0.0,
            fee_rate_taker: 0.0066 / 100.0,
            broker_fee_abs: 1.0,
            tp_ticks: 500,
            entry_cluster_q: 0.995,
            max_cluster_depth_entry: 3,
            exit_cluster_q: 0.05,
            max_cluster_depth_exit: 10,
            exit_order_timeout_ms: 500,
            exit_cluster_max_diff_ticks: 495,
            exit_cluster_start_ms: 500,
            entry_order_timeout_ms: 800,
            min_history_for_quantiles: 50,
            adverse_ticks: 100,
            cluster_diff: 1.0,
            entry_bid_thr: Some(15.4710),
            entry_ask_thr: Some(14.3710),
            exit_bid_thr: Some(0.2130),
            exit_ask_thr: Some(0.1660),
            csv_path: "trades_live_v11_live.csv",
            order_placement_delay_ms: 10,
        }
    }
}

#[derive(Debug)]
struct TradeResult {
    entry_time: DateTime<Utc>,
    exit_time: DateTime<Utc>,
    direction: &'static str,
    side: i8,
    entry_price: f64,
    exit_price: f64,
    reason: String,
    gross_ticks: f64,
    net_ticks: f64,
    gross_ret: f64,
    net_ret: f64,
    waited_ms_for_fill: i64,
    entry_fee: f64,
    exit_fee: f64,
    total_fee: f64,
    entry_liquidity: &'static str,
    exit_liquidity: &'static str,
    tp_plain: f64,
    exit_from_cluster: bool,
    hold_ms: i64,
}

struct OrderbookL2 {
    book: OrderBook,
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
}

impl Default for OrderbookL2 {
    fn default() -> Self {
        Self {
            book: OrderBook::default(),
            bids: Vec::new(),
            asks: Vec::new(),
        }
    }
}

impl OrderbookL2 {
    fn apply(&mut self, event: &OrderBookEvent) {
        self.book.update(event);
        self.bids = self
            .book
            .bids()
            .levels()
            .iter()
            .map(|lvl| (decimal_to_f64(lvl.price), decimal_to_f64(lvl.amount)))
            .collect();
        self.asks = self
            .book
            .asks()
            .levels()
            .iter()
            .map(|lvl| (decimal_to_f64(lvl.price), decimal_to_f64(lvl.amount)))
            .collect();
    }

    fn best(&self) -> Option<((f64, f64), (f64, f64))> {
        match (self.bids.first(), self.asks.first()) {
            (Some(b), Some(a)) => Some((*b, *a)),
            _ => None,
        }
    }

    fn cluster(&self, max_depth: usize, is_bid: bool) -> (Option<f64>, f64, Option<usize>) {
        let levels = if is_bid { &self.bids } else { &self.asks };
        let mut best_px = None;
        let mut best_sz = 0.0;
        let mut depth = None;

        for (idx, (price, size)) in levels.iter().take(max_depth).enumerate() {
            if *size > best_sz {
                best_sz = *size;
                best_px = Some(*price);
                depth = Some(idx);
            }
        }

        (best_px, best_sz, depth)
    }
}

struct ScalpingEngine {
    cfg: StrategyConfig,

    state: State,
    side: i8,

    order_price: Option<f64>,
    order_ts: Option<i64>,
    queue_ahead: f64,
    entry_order_ts: Option<i64>,

    entry_price: Option<f64>,
    entry_ts: Option<i64>,
    entry_time: Option<DateTime<Utc>>,
    tp_plain: Option<f64>,

    exit_order_price: Option<f64>,
    exit_order_ts: Option<i64>,
    exit_queue_ahead: f64,
    exit_reason_target: Option<String>,

    favorable_price: Option<f64>,

    bid_cluster_hist: Vec<f64>,
    ask_cluster_hist: Vec<f64>,

    bid_entry_thr: Option<f64>,
    ask_entry_thr: Option<f64>,
    bid_exit_thr: Option<f64>,
    ask_exit_thr: Option<f64>,

    last_trade_price: Option<f64>,
    last_mid: Option<f64>,
    last_time: Option<DateTime<Utc>>,

    entry_liquidity: Option<&'static str>,
    exit_liquidity: Option<&'static str>,

    entry_order_is_aggressive: bool,
    exit_order_is_aggressive: bool,

    n_entry_taker: usize,
    n_exit_taker: usize,

    n_signals: usize,
    n_orders_placed: usize,
    n_filled: usize,
    n_order_timeouts: usize,
    n_exit_orders_placed: usize,
    n_exit_orders_timeout: usize,

    trades: Vec<TradeResult>,
    csv_writer: Writer<File>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum State {
    Flat,
    WaitingEntry,
    InPosition,
}

impl ScalpingEngine {
    fn new(cfg: StrategyConfig) -> io::Result<Self> {
        let csv_path = Path::new(cfg.csv_path);
        let create_header = !csv_path.exists();
        let file = File::options().create(true).append(true).open(csv_path)?;
        let mut csv_writer = Writer::from_writer(file);

        if create_header {
            csv_writer.write_record([
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
                "entry_liquidity",
                "exit_liquidity",
                "tp_plain",
                "exit_from_cluster",
                "entry_fee",
                "exit_fee",
                "total_fee",
                "waited_ms_for_fill",
                "hold_ms",
            ])?;
            csv_writer.flush()?;
        }

        Ok(Self {
            cfg,
            state: State::Flat,
            side: 0,
            order_price: None,
            order_ts: None,
            queue_ahead: 0.0,
            entry_order_ts: None,
            entry_price: None,
            entry_ts: None,
            entry_time: None,
            tp_plain: None,
            exit_order_price: None,
            exit_order_ts: None,
            exit_queue_ahead: 0.0,
            exit_reason_target: None,
            favorable_price: None,
            bid_cluster_hist: Vec::new(),
            ask_cluster_hist: Vec::new(),
            bid_entry_thr: None,
            ask_entry_thr: None,
            bid_exit_thr: None,
            ask_exit_thr: None,
            last_trade_price: None,
            last_mid: None,
            last_time: None,
            entry_liquidity: None,
            exit_liquidity: None,
            entry_order_is_aggressive: false,
            exit_order_is_aggressive: false,
            n_entry_taker: 0,
            n_exit_taker: 0,
            n_signals: 0,
            n_orders_placed: 0,
            n_filled: 0,
            n_order_timeouts: 0,
            n_exit_orders_placed: 0,
            n_exit_orders_timeout: 0,
            trades: Vec::new(),
            csv_writer,
        })
    }

    fn on_orderbook(
        &mut self,
        ts: i64,
        bbp: f64,
        bbv: f64,
        bap: f64,
        bav: f64,
        cbp: Option<f64>,
        cbv: f64,
        cbd: Option<usize>,
        cap: Option<f64>,
        cav: f64,
        cad: Option<usize>,
    ) {
        let cur_time = timestamp_to_datetime(ts);
        self.last_mid = Some((bbp + bap) / 2.0);
        self.last_time = Some(cur_time);

        self.update_quantiles(cbv, cav);

        if self.state == State::InPosition {
            if let Some(entry_price) = self.entry_price {
                match self.side {
                    1 => {
                        let best = self.favorable_price.unwrap_or(entry_price).max(bbp);
                        self.favorable_price = Some(best);
                    }
                    -1 => {
                        let best = self.favorable_price.unwrap_or(entry_price).min(bap);
                        self.favorable_price = Some(best);
                    }
                    _ => {}
                }
            }
        }

        if self.state == State::WaitingEntry {
            if let Some(order_ts) = self.order_ts {
                if ts - order_ts >= self.cfg.entry_order_timeout_ms {
                    warn!("Entry limit timeout -> cancel order");
                    self.reset_entry_waiting();
                    self.n_order_timeouts += 1;
                }
            }
        }

        if self.state == State::InPosition {
            if let (Some(exit_ts), Some(_)) = (self.exit_order_ts, self.exit_order_price) {
                if ts - exit_ts >= self.cfg.exit_order_timeout_ms {
                    warn!("Exit limit timeout -> cancel exit order");
                    self.exit_order_price = None;
                    self.exit_order_ts = None;
                    self.exit_queue_ahead = 0.0;
                    self.exit_reason_target = None;
                    self.n_exit_orders_timeout += 1;
                }
            }
        }

        if self.state == State::InPosition
            && self.entry_ts.is_some()
            && self.exit_order_price.is_none()
            && self.tp_plain.is_some()
            && self.thresholds_ready()
            && ts - self.entry_ts.unwrap() >= self.cfg.exit_cluster_start_ms
        {
            match self.side {
                1 => {
                    let is_big_ask = cav >= self.ask_exit_thr.unwrap()
                        && cad
                            .map(|d| d <= self.cfg.max_cluster_depth_exit)
                            .unwrap_or(false)
                        && cap.is_some();
                    if is_big_ask {
                        let cap = cap.unwrap();
                        let raw_price = cap - self.cfg.tick_size;
                        let candidate = raw_price.max(bap);
                        if (candidate - self.tp_plain.unwrap()).abs()
                            <= self.cfg.exit_cluster_max_diff_ticks as f64 * self.cfg.tick_size
                        {
                            self.exit_order_price = Some(candidate);
                            self.exit_order_ts = Some(ts);
                            self.exit_queue_ahead = if (candidate - bap).abs() <= f64::EPSILON {
                                bav.max(0.0)
                            } else {
                                0.0
                            };
                            self.exit_reason_target = Some("cluster_exit".to_string());
                            self.exit_order_is_aggressive = false;
                            self.n_exit_orders_placed += 1;
                        }
                    }
                }
                -1 => {
                    let is_big_bid = cbv >= self.bid_exit_thr.unwrap()
                        && cbd
                            .map(|d| d <= self.cfg.max_cluster_depth_exit)
                            .unwrap_or(false)
                        && cbp.is_some();
                    if is_big_bid {
                        let cbp = cbp.unwrap();
                        let raw_price = cbp + self.cfg.tick_size;
                        let candidate = raw_price.min(bbp);
                        if (candidate - self.tp_plain.unwrap()).abs()
                            <= self.cfg.exit_cluster_max_diff_ticks as f64 * self.cfg.tick_size
                        {
                            self.exit_order_price = Some(candidate);
                            self.exit_order_ts = Some(ts);
                            self.exit_queue_ahead = if (candidate - bbp).abs() <= f64::EPSILON {
                                bbv.max(0.0)
                            } else {
                                0.0
                            };
                            self.exit_reason_target = Some("cluster_exit".to_string());
                            self.exit_order_is_aggressive = false;
                            self.n_exit_orders_placed += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        if self.state == State::InPosition
            && self.entry_price.is_some()
            && self.exit_order_price.is_none()
            && self.favorable_price.is_some()
        {
            match self.side {
                1 => {
                    let adverse_ticks = (self.favorable_price.unwrap() - bbp) / self.cfg.tick_size;
                    if adverse_ticks >= self.cfg.adverse_ticks as f64 {
                        self.exit_order_price = Some(bap);
                        self.exit_order_ts = Some(ts);
                        self.exit_queue_ahead = bav.max(0.0);
                        self.exit_reason_target = Some("adverse_exit".to_string());
                        self.exit_order_is_aggressive = false;
                        self.n_exit_orders_placed += 1;
                    }
                }
                -1 => {
                    let adverse_ticks = (bap - self.favorable_price.unwrap()) / self.cfg.tick_size;
                    if adverse_ticks >= self.cfg.adverse_ticks as f64 {
                        self.exit_order_price = Some(bbp);
                        self.exit_order_ts = Some(ts);
                        self.exit_queue_ahead = bbv.max(0.0);
                        self.exit_reason_target = Some("adverse_exit".to_string());
                        self.exit_order_is_aggressive = false;
                        self.n_exit_orders_placed += 1;
                    }
                }
                _ => {}
            }
        }

        if self.state == State::Flat && self.thresholds_ready() {
            let is_bid_entry = cbv >= self.bid_entry_thr.unwrap()
                && cbd
                    .map(|d| d <= self.cfg.max_cluster_depth_entry)
                    .unwrap_or(false);
            let is_ask_entry = cav >= self.ask_entry_thr.unwrap()
                && cad
                    .map(|d| d <= self.cfg.max_cluster_depth_entry)
                    .unwrap_or(false);

            if is_bid_entry {
                if let Some(cbp) = cbp {
                    self.state = State::WaitingEntry;
                    self.side = 1;
                    let mut candidate = cbp + self.cfg.tick_size;
                    if candidate > bbp + f64::EPSILON {
                        candidate = bbp;
                    }
                    self.order_price = Some(candidate);
                    self.order_ts = Some(ts);
                    self.entry_order_ts = Some(ts);
                    self.queue_ahead = if (candidate - bbp).abs() <= f64::EPSILON {
                        bbv.max(0.0)
                    } else {
                        0.0
                    };
                    self.entry_liquidity = None;
                    self.entry_order_is_aggressive = candidate >= bap - f64::EPSILON;
                    self.n_signals += 1;
                    self.n_orders_placed += 1;
                }
            } else if is_ask_entry {
                if let Some(cap) = cap {
                    self.state = State::WaitingEntry;
                    self.side = -1;
                    let mut candidate = cap - self.cfg.tick_size;
                    if candidate < bap - f64::EPSILON {
                        candidate = bap;
                    }
                    self.order_price = Some(candidate);
                    self.order_ts = Some(ts);
                    self.entry_order_ts = Some(ts);
                    self.queue_ahead = if (candidate - bap).abs() <= f64::EPSILON {
                        bav.max(0.0)
                    } else {
                        0.0
                    };
                    self.entry_liquidity = None;
                    self.entry_order_is_aggressive = candidate <= bbp + f64::EPSILON;
                    self.n_signals += 1;
                    self.n_orders_placed += 1;
                }
            }
        }
    }

    fn on_trade(&mut self, ts: i64, price: f64, size: f64) {
        let cur_time = timestamp_to_datetime(ts);
        self.last_trade_price = Some(price);
        self.last_time = Some(cur_time);

        if let State::WaitingEntry = self.state {
            if let Some(order_ts) = self.order_ts {
                if ts < order_ts + self.cfg.order_placement_delay_ms {
                    return;
                }
            }
        }

        if self.state == State::InPosition && self.exit_order_ts.is_some() {
            if let Some(exit_ts) = self.exit_order_ts {
                if ts < exit_ts + self.cfg.order_placement_delay_ms {
                    return;
                }
            }
        }

        if self.state == State::WaitingEntry {
            if let Some(order_price) = self.order_price {
                match self.side {
                    1 => {
                        if price >= order_price - f64::EPSILON {
                            self.queue_ahead -= size;
                            if self.queue_ahead <= 0.0 {
                                self.fill_entry(ts, cur_time, order_price);
                            }
                        }
                    }
                    -1 => {
                        if price <= order_price + f64::EPSILON {
                            self.queue_ahead -= size;
                            if self.queue_ahead <= 0.0 {
                                self.fill_entry(ts, cur_time, order_price);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if self.state == State::InPosition && self.exit_order_price.is_some() {
            let exit_price = self.exit_order_price.unwrap();
            match self.side {
                1 => {
                    if price >= exit_price - f64::EPSILON {
                        self.exit_queue_ahead -= size;
                        if self.exit_queue_ahead <= 0.0 {
                            self.exit_liquidity = Some(if self.exit_order_is_aggressive {
                                "taker"
                            } else {
                                "maker"
                            });
                            if self.exit_liquidity == Some("taker") {
                                self.n_exit_taker += 1;
                            }
                            self.close_position(ts, cur_time);
                        }
                    }
                }
                -1 => {
                    if price <= exit_price + f64::EPSILON {
                        self.exit_queue_ahead -= size;
                        if self.exit_queue_ahead <= 0.0 {
                            self.exit_liquidity = Some(if self.exit_order_is_aggressive {
                                "taker"
                            } else {
                                "maker"
                            });
                            if self.exit_liquidity == Some("taker") {
                                self.n_exit_taker += 1;
                            }
                            self.close_position(ts, cur_time);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn fill_entry(&mut self, ts: i64, cur_time: DateTime<Utc>, order_price: f64) {
        self.state = State::InPosition;
        self.entry_price = Some(order_price);
        self.entry_ts = Some(ts);
        self.entry_time = Some(cur_time);
        self.tp_plain = Some(if self.side == 1 {
            order_price + self.cfg.tp_ticks as f64 * self.cfg.tick_size
        } else {
            order_price - self.cfg.tp_ticks as f64 * self.cfg.tick_size
        });
        self.favorable_price = Some(order_price);
        let is_taker = self.entry_order_is_aggressive;
        self.entry_liquidity = Some(if is_taker { "taker" } else { "maker" });
        if is_taker {
            self.n_entry_taker += 1;
        }
        self.n_filled += 1;
        self.order_price = None;
        self.order_ts = None;
        self.queue_ahead = 0.0;
        self.entry_order_ts = None;
        self.entry_order_is_aggressive = false;
        self.exit_order_price = None;
        self.exit_order_ts = None;
        self.exit_queue_ahead = 0.0;
        self.exit_reason_target = None;
        self.exit_order_is_aggressive = false;
    }

    fn close_position(&mut self, _ts: i64, cur_time: DateTime<Utc>) {
        if let (Some(entry_price), Some(entry_ts), Some(entry_time)) =
            (self.entry_price, self.entry_ts, self.entry_time)
        {
            let exit_price = self.exit_order_price.unwrap_or(entry_price);
            let exit_time = cur_time;
            let reason = self
                .exit_reason_target
                .clone()
                .unwrap_or_else(|| "limit_exit".to_string());

            let side = self.side as f64;
            let pnl_px = side * (exit_price - entry_price) * self.cfg.order_size;
            let entry_liq = self.entry_liquidity.unwrap_or("maker");
            let exit_liq = self.exit_liquidity.unwrap_or("maker");

            let entry_fee_rate = if entry_liq == "taker" {
                self.cfg.fee_rate_taker
            } else {
                self.cfg.fee_rate_maker
            };
            let exit_fee_rate = if exit_liq == "taker" {
                self.cfg.fee_rate_taker
            } else {
                self.cfg.fee_rate_maker
            };

            let entry_fee =
                entry_fee_rate * entry_price * self.cfg.order_size + self.cfg.broker_fee_abs;
            let exit_fee =
                exit_fee_rate * exit_price * self.cfg.order_size + self.cfg.broker_fee_abs;
            let total_fee = entry_fee + exit_fee;

            let notional_entry = entry_price * self.cfg.order_size;
            let gross_ret = pnl_px / notional_entry;
            let net_ret = (pnl_px - total_fee) / notional_entry;

            let gross_ticks = side * (exit_price - entry_price) / self.cfg.tick_size;
            let rel_tick = self.cfg.tick_size / entry_price;
            let net_ticks = net_ret / rel_tick;

            let waited_ms = self
                .entry_order_ts
                .zip(Some(entry_ts))
                .map(|(order_ts, filled_ts)| filled_ts - order_ts)
                .unwrap_or_default();
            let hold_ms = exit_time.timestamp_millis() - entry_time.timestamp_millis();
            let exit_from_cluster = reason == "cluster_exit";
            let direction = if self.side == 1 { "long" } else { "short" };

            let result = TradeResult {
                entry_time,
                exit_time,
                direction,
                side: self.side,
                entry_price,
                exit_price,
                reason: reason.clone(),
                gross_ticks,
                net_ticks,
                gross_ret,
                net_ret,
                waited_ms_for_fill: waited_ms,
                entry_fee,
                exit_fee,
                total_fee,
                entry_liquidity: entry_liq,
                exit_liquidity: exit_liq,
                tp_plain: self.tp_plain.unwrap_or(f64::NAN),
                exit_from_cluster,
                hold_ms,
            };
            self.trades.push(result);
            self.write_last_trade();
            info!(
                direction,
                entry_price,
                exit_price,
                gross_ticks,
                net_ticks,
                reason,
                entry_liq,
                exit_liq,
                "closed trade"
            );
        }

        self.reset_after_close();
    }

    fn force_flatten(&mut self) {
        if self.state == State::InPosition && self.entry_price.is_some() {
            let exit_price = self
                .last_trade_price
                .or(self.last_mid)
                .unwrap_or(self.entry_price.unwrap());
            let exit_time = self.last_time.unwrap_or_else(Utc::now);
            self.exit_reason_target = Some("eod_force".to_string());
            self.exit_order_price = Some(exit_price);
            self.close_position(exit_time.timestamp_millis(), exit_time);
        }
    }

    fn write_last_trade(&mut self) {
        if let Some(tr) = self.trades.last() {
            let _ = self.csv_writer.write_record([
                tr.entry_time.to_rfc3339(),
                tr.exit_time.to_rfc3339(),
                tr.direction.to_string(),
                tr.side.to_string(),
                format!("{:.4}", tr.entry_price),
                format!("{:.4}", tr.exit_price),
                format!("{:.4}", tr.gross_ticks),
                format!("{:.4}", tr.net_ticks),
                format!("{:.8}", tr.gross_ret),
                format!("{:.8}", tr.net_ret),
                tr.reason.clone(),
                tr.entry_liquidity.to_string(),
                tr.exit_liquidity.to_string(),
                format!("{:.4}", tr.tp_plain),
                if tr.exit_from_cluster { "1" } else { "0" }.to_string(),
                format!("{:.4}", tr.entry_fee),
                format!("{:.4}", tr.exit_fee),
                format!("{:.4}", tr.total_fee),
                tr.waited_ms_for_fill.to_string(),
                tr.hold_ms.to_string(),
            ]);
            let _ = self.csv_writer.flush();
        }
    }

    fn reset_after_close(&mut self) {
        self.state = State::Flat;
        self.side = 0;
        self.order_price = None;
        self.order_ts = None;
        self.queue_ahead = 0.0;
        self.entry_price = None;
        self.entry_ts = None;
        self.entry_time = None;
        self.entry_order_ts = None;
        self.tp_plain = None;
        self.exit_order_price = None;
        self.exit_order_ts = None;
        self.exit_queue_ahead = 0.0;
        self.exit_reason_target = None;
        self.favorable_price = None;
        self.entry_liquidity = None;
        self.exit_liquidity = None;
        self.entry_order_is_aggressive = false;
        self.exit_order_is_aggressive = false;
    }

    fn thresholds_ready(&self) -> bool {
        self.bid_entry_thr.is_some()
            && self.ask_entry_thr.is_some()
            && self.bid_exit_thr.is_some()
            && self.ask_exit_thr.is_some()
    }

    fn update_quantiles(&mut self, cbv: f64, cav: f64) {
        if cbv > 0.0 {
            self.bid_cluster_hist.push(cbv);
        }
        if cav > 0.0 {
            self.ask_cluster_hist.push(cav);
        }

        if self.bid_cluster_hist.len() >= self.cfg.min_history_for_quantiles
            && self.ask_cluster_hist.len() >= self.cfg.min_history_for_quantiles
        {
            self.bid_entry_thr = Some(
                self.cfg
                    .entry_bid_thr
                    .unwrap_or_else(|| quantile(&self.bid_cluster_hist, self.cfg.entry_cluster_q)),
            );
            self.ask_entry_thr = Some(
                self.cfg
                    .entry_ask_thr
                    .unwrap_or_else(|| quantile(&self.ask_cluster_hist, self.cfg.entry_cluster_q)),
            );
            self.bid_exit_thr = Some(
                self.cfg
                    .exit_bid_thr
                    .unwrap_or_else(|| quantile(&self.bid_cluster_hist, self.cfg.exit_cluster_q)),
            );
            self.ask_exit_thr = Some(
                self.cfg
                    .exit_ask_thr
                    .unwrap_or_else(|| quantile(&self.ask_cluster_hist, self.cfg.exit_cluster_q)),
            );
        }
    }

    fn reset_entry_waiting(&mut self) {
        self.state = State::Flat;
        self.side = 0;
        self.order_price = None;
        self.order_ts = None;
        self.queue_ahead = 0.0;
        self.entry_order_ts = None;
    }
}

fn quantile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = ((sorted.len() - 1) as f64 * q).floor() as usize;
    sorted[pos]
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

fn timestamp_to_datetime(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ts)
        .single()
        .unwrap_or_else(Utc::now)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let cfg = StrategyConfig::default();
    let mut engine = ScalpingEngine::new(cfg.clone())?;
    let mut local_book = OrderbookL2::default();

    let mut stream = init_stream(&cfg)
        .await?
        .select_all()
        .with_error_handler(|error| {
            warn!(?error, "market stream error");
        });

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("signal received, closing");
                engine.force_flatten();
                break;
            }
            Some(event) = stream.next() => match event {
                Event::Reconnecting(exchange) => warn!(%exchange, "market stream reconnecting"),
                Event::Item(market_event) => {
                    match market_event.kind.clone() {
                        DataKind::OrderBook(book_event) => {
                            handle_orderbook_event(&mut local_book, &mut engine, market_event.map_kind(|_| book_event));
                        }
                        DataKind::Trade(trade) => {
                            handle_trade_event(&mut engine, market_event.map_kind(|_| trade));
                        }
                        _ => {}
                    }
                }
            },
            else => break,
        }
    }

    Ok(())
}

fn handle_orderbook_event(
    local_book: &mut OrderbookL2,
    engine: &mut ScalpingEngine,
    event: MarketEvent<MarketDataInstrument, OrderBookEvent>,
) {
    let ts = event.time_exchange.timestamp_millis();
    local_book.apply(&event.kind);
    if let Some(((bbp, bbv), (bap, bav))) = local_book.best() {
        let depth = engine
            .cfg
            .max_cluster_depth_entry
            .max(engine.cfg.max_cluster_depth_exit);
        let (cbp, cbv, cbd) = local_book.cluster(depth, true);
        let (cap, cav, cad) = local_book.cluster(depth, false);
        engine.on_orderbook(ts, bbp, bbv, bap, bav, cbp, cbv, cbd, cap, cav, cad);
    }
}

fn handle_trade_event(
    engine: &mut ScalpingEngine,
    event: MarketEvent<MarketDataInstrument, barter_data::subscription::trade::PublicTrade>,
) {
    let ts = event.time_exchange.timestamp_millis();
    let price = event.kind.price;
    let size = event.kind.amount;
    engine.on_trade(ts, price, size);
}

async fn init_stream(
    cfg: &StrategyConfig,
) -> Result<
    Streams<MarketStreamResult<MarketDataInstrument, DataKind>>,
    barter_data::error::DataError,
> {
    let (base, quote) = split_symbol(cfg.symbol);
    Streams::builder_multi()
        .add(Streams::<PublicTrades>::builder().subscribe([(
            BybitPerpetualsUsd::default(),
            base.as_str(),
            quote.as_str(),
            MarketDataInstrumentKind::Perpetual,
            PublicTrades,
        )]))
        .add(
            Streams::<barter_data::subscription::book::OrderBooksL2>::builder().subscribe([(
                BybitPerpetualsUsd::default(),
                base.as_str(),
                quote.as_str(),
                MarketDataInstrumentKind::Perpetual,
                barter_data::subscription::book::OrderBooksL2,
            )]),
        )
        .init()
        .await
}

fn split_symbol(symbol: &str) -> (String, String) {
    let lower = symbol.to_lowercase();
    if lower.ends_with("usdt") {
        let (base, quote) = lower.split_at(lower.len() - 4);
        (base.to_string(), quote.to_string())
    } else {
        (lower, "usdt".to_string())
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
        .json()
        .init();
}