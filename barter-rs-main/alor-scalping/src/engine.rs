use chrono::{DateTime, Utc};
use csv::Writer;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, fs::File, path::Path};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub enum EngineEvent {
    OrderBookL2 {
        ts: DateTime<Utc>,
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
    },
    Trade {
        ts: DateTime<Utc>,
        price: f64,
        qty: f64,
        side: TradeSide,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug)]
pub struct StrategyConfig {
    pub symbol: String,
    pub depth: usize,
    pub tick_size: f64,
    pub contract_size: f64,
    pub order_size: f64,
    pub maker_fee: f64,
    pub taker_fee: f64,
    pub broker_fee_abs: f64,
    pub tp_ticks: i64,
    pub entry_cluster_q: f64,
    pub exit_cluster_q: f64,
    pub max_cluster_depth_entry: usize,
    pub max_cluster_depth_exit: usize,
    pub entry_order_timeout_ms: i64,
    pub exit_order_timeout_ms: i64,
    pub exit_cluster_start_ms: i64,
    pub exit_cluster_max_diff_ticks: i64,
    pub adverse_ticks: i64,
    pub min_history_for_quantiles: usize,
    pub entry_bid_thr: Option<f64>,
    pub entry_ask_thr: Option<f64>,
    pub exit_bid_thr: Option<f64>,
    pub exit_ask_thr: Option<f64>,
    pub order_placement_delay_ms: i64,
    pub csv_path: String,
}

#[derive(Debug)]
pub enum OrderCommand {
    PlaceLimit {
        side: OrderSide,
        price: f64,
        qty: f64,
        reason: OrderReason,
    },
    CancelAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderReason {
    EntryCluster,
    ExitCluster,
    ExitAdverse,
    ForceFlatten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Flat,
    WaitingEntry,
    InPosition,
}

#[derive(Debug, Clone)]
struct OrderTicket {
    price: f64,
    queue_ahead: f64,
    ts: DateTime<Utc>,
    aggressive: bool,
}

#[derive(Debug, Clone)]
struct TradeResult {
    entry_time: DateTime<Utc>,
    exit_time: DateTime<Utc>,
    direction: String,
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
    entry_liquidity: String,
    exit_liquidity: String,
    tp_plain: f64,
    exit_from_cluster: bool,
    hold_ms: i64,
}

#[derive(Debug)]
pub struct ScalpingEngine {
    cfg: StrategyConfig,
    state: State,
    side: i8,
    entry_order: Option<OrderTicket>,
    exit_order: Option<(OrderTicket, OrderReason)>,
    entry_price: Option<f64>,
    entry_ts: Option<i64>,
    entry_time: Option<DateTime<Utc>>,
    entry_order_ts: Option<i64>,
    tp_plain: Option<f64>,
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
    entry_liquidity: Option<String>,
    exit_liquidity: Option<String>,
    entry_order_is_aggressive: bool,
    exit_order_is_aggressive: bool,
    n_entry_taker: usize,
    n_exit_taker: usize,
    trades: Vec<TradeResult>,
    csv_writer: Writer<File>,
}

impl ScalpingEngine {
    pub fn new(cfg: StrategyConfig) -> Self {
        let csv_path = Path::new(&cfg.csv_path);
        let create_header = !csv_path.exists();
        let file = File::options()
            .create(true)
            .append(true)
            .open(csv_path)
            .expect("failed to open csv file");
        let mut csv_writer = Writer::from_writer(file);
        if create_header {
            csv_writer
                .write_record([
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
                ])
                .expect("failed to write csv header");
            csv_writer.flush().expect("failed to flush csv header");
        }

        Self {
            cfg,
            state: State::Flat,
            side: 0,
            entry_order: None,
            exit_order: None,
            entry_price: None,
            entry_ts: None,
            entry_time: None,
            entry_order_ts: None,
            tp_plain: None,
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
            trades: Vec::new(),
            csv_writer,
        }
    }

    pub fn on_event(&mut self, event: EngineEvent) -> Vec<OrderCommand> {
        match event {
            EngineEvent::OrderBookL2 { ts, bids, asks } => self.on_orderbook(ts, bids, asks),
            EngineEvent::Trade { ts, price, qty, .. } => self.on_trade(ts, price, qty),
        }
    }

    fn on_orderbook(
        &mut self,
        ts: DateTime<Utc>,
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
    ) -> Vec<OrderCommand> {
        if bids.is_empty() || asks.is_empty() {
            return Vec::new();
        }

        let (bbp, bbv) = bids[0];
        let (bap, bav) = asks[0];
        let depth = self
            .cfg
            .max_cluster_depth_entry
            .max(self.cfg.max_cluster_depth_exit)
            .max(1);
        let (cbp, cbv, cbd) = find_cluster(&bids, depth);
        let (cap, cav, cad) = find_cluster(&asks, depth);

        debug!(
            ts = ?ts,
            bbp,
            bbv,
            bap,
            bav,
            cbp,
            cbv,
            cbd,
            cap,
            cav,
            cad,
            "orderbook update"
        );

        self.last_mid = Some((bbp + bap) / 2.0);
        self.last_time = Some(ts);

        if self.state == State::InPosition {
            if self.side == 1 {
                self.favorable_price = Some(self.favorable_price.unwrap_or(bbp).max(bbp));
            } else if self.side == -1 {
                self.favorable_price = Some(self.favorable_price.unwrap_or(bap).min(bap));
            }
        }

        self.update_quantiles(cbv, cav);

        let mut cmds = Vec::new();

        if let Some(order) = &self.entry_order {
            let elapsed = ts.timestamp_millis() - order.ts.timestamp_millis();
            if elapsed >= self.cfg.entry_order_timeout_ms {
                warn!(ts = ?ts, "entry limit timeout -> cancel");
                cmds.push(OrderCommand::CancelAll);
                self.reset_entry_waiting();
            }
        }

        if let Some((order, _)) = &self.exit_order {
            let elapsed = ts.timestamp_millis() - order.ts.timestamp_millis();
            if elapsed >= self.cfg.exit_order_timeout_ms {
                warn!(ts = ?ts, "exit limit timeout -> cancel exit order");
                self.exit_order = None;
            }
        }

        if self.state == State::InPosition {
            self.try_place_cluster_exit(ts, bbp, bbv, bap, bav, cbp, cbv, cbd, cap, cav, cad);
            self.try_place_adverse_exit(ts, bbp, bbv, bap, bav);
        }

        if self.state == State::Flat && self.thresholds_ready() {
            let is_bid_entry = cbv >= self.bid_entry_thr.unwrap_or(0.0)
                && cbd
                    .map(|d| d <= self.cfg.max_cluster_depth_entry)
                    .unwrap_or(false);
            if is_bid_entry {
                if let Some(cbp) = cbp {
                    let mut price = cbp + self.cfg.tick_size;
                    if price > bbp {
                        price = bbp;
                    }
                    let queue_ahead = if (price - bbp).abs() <= f64::EPSILON {
                        bbv.max(0.0)
                    } else {
                        0.0
                    };
                    info!(ts = ?ts, price, queue_ahead, "entry signal -> long");
                    self.place_entry(ts, 1, price, queue_ahead, bap, &mut cmds);
                }
            }

            let is_ask_entry = cav >= self.ask_entry_thr.unwrap_or(0.0)
                && cad
                    .map(|d| d <= self.cfg.max_cluster_depth_entry)
                    .unwrap_or(false);
            if is_ask_entry {
                if let Some(cap) = cap {
                    let mut price = cap - self.cfg.tick_size;
                    if price < bap {
                        price = bap;
                    }
                    let queue_ahead = if (price - bap).abs() <= f64::EPSILON {
                        bav.max(0.0)
                    } else {
                        0.0
                    };
                    info!(ts = ?ts, price, queue_ahead, "entry signal -> short");
                    self.place_entry(ts, -1, price, queue_ahead, bbp, &mut cmds);
                }
            }
        }

        cmds
    }

    fn on_trade(&mut self, ts: DateTime<Utc>, price: f64, size: f64) -> Vec<OrderCommand> {
        let mut cmds = Vec::new();
        let ts_ms = ts.timestamp_millis();
        self.last_trade_price = Some(price);
        self.last_time = Some(ts);

        if let Some(order) = self.entry_order.take() {
            let visible_ts = order.ts.timestamp_millis() + self.cfg.order_placement_delay_ms;
            if ts_ms >= visible_ts {
                if let Some(order) = self.fill_entry(order, price, size, ts, &mut cmds) {
                    self.entry_order = Some(order);
                }
            } else {
                self.entry_order = Some(order);
            }
        }

        if let Some((order, reason)) = self.exit_order.take() {
            let visible_ts = order.ts.timestamp_millis() + self.cfg.order_placement_delay_ms;
            if ts_ms >= visible_ts {
                if let Some(order) = self.fill_exit(order, price, size, ts, reason, &mut cmds) {
                    self.exit_order = Some((order, reason));
                }
            } else {
                self.exit_order = Some((order, reason));
            }
        }

        cmds
    }

    fn fill_entry(
        &mut self,
        mut order: OrderTicket,
        price: f64,
        qty: f64,
        ts: DateTime<Utc>,
        cmds: &mut Vec<OrderCommand>,
    ) -> Option<OrderTicket> {
        let filled = match self.side {
            1 => price + f64::EPSILON >= order.price,
            -1 => price <= order.price + f64::EPSILON,
            _ => false,
        };

        if filled {
            order.queue_ahead -= qty;
            if order.queue_ahead <= 0.0 {
                let entry_price = order.price;
                self.entry_price = Some(entry_price);
                self.entry_ts = Some(ts.timestamp_millis());
                self.entry_time = Some(ts);
                self.entry_order_ts = Some(order.ts.timestamp_millis());
                self.tp_plain = Some(if self.side == 1 {
                    entry_price + self.cfg.tp_ticks as f64 * self.cfg.tick_size
                } else {
                    entry_price - self.cfg.tp_ticks as f64 * self.cfg.tick_size
                });
                self.favorable_price = Some(entry_price);
                let is_taker = order.aggressive;
                self.entry_liquidity = Some(if is_taker { "taker" } else { "maker" }.to_string());
                if is_taker {
                    self.n_entry_taker += 1;
                }
                self.state = State::InPosition;
                info!(ts = ?ts, price = entry_price, side = self.side, "entry filled");
                cmds.push(OrderCommand::CancelAll);
                self.entry_order = None;
                self.exit_order = None;
                return None;
            }
        }

        Some(order)
    }

    fn fill_exit(
        &mut self,
        mut order: OrderTicket,
        price: f64,
        qty: f64,
        ts: DateTime<Utc>,
        reason: OrderReason,
        cmds: &mut Vec<OrderCommand>,
    ) -> Option<OrderTicket> {
        let filled = match self.side {
            1 => price + f64::EPSILON >= order.price,
            -1 => price <= order.price + f64::EPSILON,
            _ => false,
        };

        if filled {
            order.queue_ahead -= qty;
            if order.queue_ahead <= 0.0 {
                self.exit_liquidity =
                    Some(if order.aggressive { "taker" } else { "maker" }.to_string());
                if order.aggressive {
                    self.n_exit_taker += 1;
                }
                self.exit_order_is_aggressive = order.aggressive;
                self.exit_order = None;
                self.close_position(ts, order.price, reason, cmds);
                return None;
            }
        }

        Some(order)
    }

    fn try_place_cluster_exit(
        &mut self,
        ts: DateTime<Utc>,
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
        if self.exit_order.is_some() || self.entry_price.is_none() || self.entry_ts.is_none() {
            return;
        }
        if !self.thresholds_ready() {
            return;
        }
        let entry_ts = self.entry_ts.unwrap();
        if ts.timestamp_millis() - entry_ts < self.cfg.exit_cluster_start_ms {
            return;
        }

        match self.side {
            1 => {
                let is_big_ask = cav >= self.ask_exit_thr.unwrap_or(0.0)
                    && cad
                        .map(|d| d <= self.cfg.max_cluster_depth_exit)
                        .unwrap_or(false)
                    && cap.is_some();
                if is_big_ask {
                    let raw_price = cap.unwrap() - self.cfg.tick_size;
                    let candidate = raw_price.max(bap);
                    if let Some(tp) = self.tp_plain {
                        if (candidate - tp).abs()
                            > self.cfg.exit_cluster_max_diff_ticks as f64 * self.cfg.tick_size
                        {
                            return;
                        }
                    }
                    self.exit_order = Some((
                        OrderTicket {
                            price: candidate,
                            queue_ahead: if (candidate - bap).abs() <= f64::EPSILON {
                                bav.max(0.0)
                            } else {
                                0.0
                            },
                            ts,
                            aggressive: false,
                        },
                        OrderReason::ExitCluster,
                    ));
                    info!(ts = ?ts, price = candidate, queue = self.exit_order.as_ref().map(|(o, _)| o.queue_ahead), "place cluster exit (long)");
                }
            }
            -1 => {
                let is_big_bid = cbv >= self.bid_exit_thr.unwrap_or(0.0)
                    && cbd
                        .map(|d| d <= self.cfg.max_cluster_depth_exit)
                        .unwrap_or(false)
                    && cbp.is_some();
                if is_big_bid {
                    let raw_price = cbp.unwrap() + self.cfg.tick_size;
                    let candidate = raw_price.min(bbp);
                    if let Some(tp) = self.tp_plain {
                        if (candidate - tp).abs()
                            > self.cfg.exit_cluster_max_diff_ticks as f64 * self.cfg.tick_size
                        {
                            return;
                        }
                    }
                    self.exit_order = Some((
                        OrderTicket {
                            price: candidate,
                            queue_ahead: if (candidate - bbp).abs() <= f64::EPSILON {
                                bbv.max(0.0)
                            } else {
                                0.0
                            },
                            ts,
                            aggressive: false,
                        },
                        OrderReason::ExitCluster,
                    ));
                    info!(ts = ?ts, price = candidate, queue = self.exit_order.as_ref().map(|(o, _)| o.queue_ahead), "place cluster exit (short)");
                }
            }
            _ => {}
        }
    }

    fn try_place_adverse_exit(
        &mut self,
        ts: DateTime<Utc>,
        bbp: f64,
        bbv: f64,
        bap: f64,
        bav: f64,
    ) {
        if self.exit_order.is_some() || self.entry_price.is_none() || self.favorable_price.is_none()
        {
            return;
        }

        match self.side {
            1 => {
                let adverse_ticks = (self.favorable_price.unwrap() - bbp) / self.cfg.tick_size;
                if adverse_ticks >= self.cfg.adverse_ticks as f64 {
                    self.exit_order = Some((
                        OrderTicket {
                            price: bap,
                            queue_ahead: bav.max(0.0),
                            ts,
                            aggressive: false,
                        },
                        OrderReason::ExitAdverse,
                    ));
                    warn!(ts = ?ts, adverse_ticks, price = bap, "place adverse exit (long)");
                }
            }
            -1 => {
                let adverse_ticks = (bap - self.favorable_price.unwrap()) / self.cfg.tick_size;
                if adverse_ticks >= self.cfg.adverse_ticks as f64 {
                    self.exit_order = Some((
                        OrderTicket {
                            price: bbp,
                            queue_ahead: bbv.max(0.0),
                            ts,
                            aggressive: false,
                        },
                        OrderReason::ExitAdverse,
                    ));
                    warn!(ts = ?ts, adverse_ticks, price = bbp, "place adverse exit (short)");
                }
            }
            _ => {}
        }
    }

    fn place_entry(
        &mut self,
        ts: DateTime<Utc>,
        side: i8,
        price: f64,
        queue_ahead: f64,
        opposite_best: f64,
        cmds: &mut Vec<OrderCommand>,
    ) {
        self.state = State::WaitingEntry;
        self.side = side;
        self.entry_order_is_aggressive = match side {
            1 => price >= opposite_best - f64::EPSILON,
            -1 => price <= opposite_best + f64::EPSILON,
            _ => false,
        };
        self.entry_order = Some(OrderTicket {
            price,
            queue_ahead,
            ts,
            aggressive: self.entry_order_is_aggressive,
        });
        cmds.push(OrderCommand::PlaceLimit {
            side: if side == 1 {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            },
            price,
            qty: self.cfg.order_size,
            reason: OrderReason::EntryCluster,
        });
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

    fn close_position(
        &mut self,
        ts: DateTime<Utc>,
        exit_price: f64,
        reason: OrderReason,
        cmds: &mut Vec<OrderCommand>,
    ) {
        if let (Some(entry_price), Some(entry_ts), Some(entry_time)) =
            (self.entry_price, self.entry_ts, self.entry_time)
        {
            let side = self.side;
            let exit_time = ts;
            let entry_liq = self
                .entry_liquidity
                .clone()
                .unwrap_or_else(|| "maker".to_string());
            let exit_liq = self
                .exit_liquidity
                .clone()
                .unwrap_or_else(|| "maker".to_string());

            let entry_fee_rate = if entry_liq == "taker" {
                self.cfg.taker_fee
            } else {
                self.cfg.maker_fee
            };
            let exit_fee_rate = if exit_liq == "taker" {
                self.cfg.taker_fee
            } else {
                self.cfg.maker_fee
            };

            let entry_fee =
                entry_fee_rate * entry_price * self.cfg.contract_size + self.cfg.broker_fee_abs;
            let exit_fee =
                exit_fee_rate * exit_price * self.cfg.contract_size + self.cfg.broker_fee_abs;
            let total_fee = entry_fee + exit_fee;

            let pnl_px = side as f64 * (exit_price - entry_price) * self.cfg.contract_size;
            let notional_entry = entry_price * self.cfg.contract_size;
            let gross_ret = if notional_entry != 0.0 {
                pnl_px / notional_entry
            } else {
                0.0
            };
            let net_ret = if notional_entry != 0.0 {
                (pnl_px - total_fee) / notional_entry
            } else {
                0.0
            };

            let gross_ticks = side as f64 * (exit_price - entry_price) / self.cfg.tick_size;
            let rel_tick = if entry_price != 0.0 {
                self.cfg.tick_size / entry_price
            } else {
                0.0
            };
            let net_ticks = if rel_tick != 0.0 {
                net_ret / rel_tick
            } else {
                0.0
            };

            let waited_ms = self
                .entry_order_ts
                .map(|order_ts| entry_ts - order_ts)
                .unwrap_or_default();
            let hold_ms = exit_time.timestamp_millis() - entry_time.timestamp_millis();
            let tp_plain = self.tp_plain.unwrap_or(f64::NAN);
            let exit_from_cluster = matches!(reason, OrderReason::ExitCluster);
            let direction = if side == 1 { "long" } else { "short" };

            let result = TradeResult {
                entry_time,
                exit_time,
                direction: direction.to_string(),
                side,
                entry_price,
                exit_price,
                reason: format!("{reason:?}"),
                gross_ticks,
                net_ticks,
                gross_ret,
                net_ret,
                waited_ms_for_fill: waited_ms,
                entry_fee,
                exit_fee,
                total_fee,
                entry_liquidity: entry_liq.clone(),
                exit_liquidity: exit_liq.clone(),
                tp_plain,
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
                ?reason,
                entry_liq,
                exit_liq,
                "closed trade"
            );

            self.reset_after_close();
        }

        cmds.push(OrderCommand::CancelAll);
    }

    pub fn force_flatten(&mut self) {
        if self.state == State::InPosition && self.entry_price.is_some() {
            let exit_price = self
                .last_trade_price
                .or(self.last_mid)
                .unwrap_or(self.entry_price.unwrap());
            let exit_time = self.last_time.unwrap_or_else(Utc::now);
            self.exit_liquidity = Some("maker".to_string());
            self.close_position(
                exit_time,
                exit_price,
                OrderReason::ForceFlatten,
                &mut Vec::new(),
            );
        }
    }

    fn write_last_trade(&mut self) {
        if let Some(tr) = self.trades.last() {
            let _ = self.csv_writer.write_record([
                tr.entry_time.to_rfc3339(),
                tr.exit_time.to_rfc3339(),
                tr.direction.clone(),
                tr.side.to_string(),
                format!("{:.4}", tr.entry_price),
                format!("{:.4}", tr.exit_price),
                format!("{:.4}", tr.gross_ticks),
                format!("{:.4}", tr.net_ticks),
                format!("{:.8}", tr.gross_ret),
                format!("{:.8}", tr.net_ret),
                tr.reason.clone(),
                tr.entry_liquidity.clone(),
                tr.exit_liquidity.clone(),
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
        self.entry_order = None;
        self.exit_order = None;
        self.entry_price = None;
        self.entry_ts = None;
        self.entry_time = None;
        self.entry_order_ts = None;
        self.tp_plain = None;
        self.favorable_price = None;
        self.entry_liquidity = None;
        self.exit_liquidity = None;
        self.entry_order_is_aggressive = false;
        self.exit_order_is_aggressive = false;
    }

    fn reset_entry_waiting(&mut self) {
        self.state = State::Flat;
        self.side = 0;
        self.entry_order = None;
        self.entry_price = None;
        self.entry_ts = None;
        self.entry_time = None;
        self.entry_order_ts = None;
        self.tp_plain = None;
        self.favorable_price = None;
        self.entry_liquidity = None;
        self.entry_order_is_aggressive = false;
    }
}

fn quantile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let pos = ((sorted.len() - 1) as f64 * q).floor() as usize;
    sorted[pos]
}

fn find_cluster(levels: &[(f64, f64)], depth: usize) -> (Option<f64>, f64, Option<usize>) {
    let mut best_volume = 0.0;
    let mut best_price = None;
    let mut best_depth = None;
    for (idx, (price, vol)) in levels.iter().take(depth).enumerate() {
        if *vol > best_volume {
            best_volume = *vol;
            best_price = Some(*price);
            best_depth = Some(idx);
        }
    }
    (best_price, best_volume, best_depth)
}