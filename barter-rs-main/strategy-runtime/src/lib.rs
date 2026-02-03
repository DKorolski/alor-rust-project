pub mod config;
pub mod live_guard;
pub mod redis_transport;
pub mod runtime;
pub mod state;
pub mod strategies;

use std::collections::HashMap;

use alor_protocol::{CommandAction, OrderCommand, PlaceOrder, Side};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::strategies::limit_cancel::LimitCancelConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Place {
        price: f64,
        qty: f64,
        side: Side,
    },
    Cancel {
        order_id: i64,
    },
    Replace {
        order_id: i64,
        new_price: f64,
        new_qty: f64,
    },
}

pub trait Strategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent>;
    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &alor_protocol::CommandAck) -> Vec<Intent>;
    fn on_order(&mut self, ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent>;
    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent>;
    fn state(&self) -> &crate::state::StrategyState;
    fn set_state(&mut self, state: crate::state::StrategyState);
}

#[derive(Debug, Clone)]
pub struct StrategyCtx {
    pub strategy_id: String,
    pub portfolio: String,
    pub exchange: String,
    pub symbol: String,
    pub tick_size: f64,
    pub trade_mode: TradeMode,
    pub allow_live_orders: bool,
    pub gateway_phase: crate::live_guard::GatewayPhase,
    last_bar_ts: Option<i64>,
}

impl StrategyCtx {
    pub fn last_bar_ts(&self) -> Option<i64> {
        self.last_bar_ts
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BarEvent {
    pub symbol: String,
    pub close_time_utc: i64,
    #[serde(default, alias = "c")]
    pub close: f64,
    #[serde(default)]
    pub o: f64,
    #[serde(default)]
    pub h: f64,
    #[serde(default)]
    pub l: f64,
    #[serde(default)]
    pub v: f64,
    pub origin: DataOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataOrigin {
    History,
    HistoryGap,
    Live,
    Replay,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderEvent {
    pub order_id: i64,
    pub request_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PositionEvent {
    pub symbol: String,
    pub qty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeConfig {
    pub redis_url: String,
    pub source: String,
    pub portfolio: String,
    pub exchange: String,
    pub streams: StreamNames,
    pub consumer_group: String,
    pub consumer_name: String,
    pub trade_mode: TradeMode,
    pub allow_live_orders: bool,
    pub guard_log_interval_ms: u64,
    pub read: ReadConfig,
    pub trim: TrimConfig,
    pub strategy: StrategyConfig,
    pub paper: PaperConfig,
    pub backtest: BacktestConfig,
    pub reset_state_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamNames {
    pub bars: String,
    pub orders: String,
    pub positions: String,
    pub commands: String,
    pub acks: String,
    pub health: Option<String>,
    pub dlq_prefix: String,
    pub runtime_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadConfig {
    pub block_ms: usize,
    pub claim_idle_ms: usize,
    pub claim_batch: usize,
    pub poll_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrimConfig {
    pub bars: usize,
    pub orders: usize,
    pub positions: usize,
    pub commands: usize,
    pub acks: usize,
    pub health: usize,
    pub runtime_state: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyConfig {
    pub strategy_id: String,
    pub symbol: String,
    pub qty: f64,
    pub side: Side,
    pub place_offset_ticks: i64,
    pub tick_size: f64,
    pub max_wait_bars_for_ack: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TradeMode {
    Live,
    Paper,
    Backtest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaperOutput {
    Stdout,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaperConfig {
    pub enabled: bool,
    pub output: PaperOutput,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestConfig {
    pub enabled: bool,
    pub trade_log: String,
}

impl StrategyConfig {
    pub fn to_limit_cancel_config(&self) -> LimitCancelConfig {
        LimitCancelConfig {
            symbol: self.symbol.clone(),
            tick_size: self.tick_size,
            offset_ticks: self.place_offset_ticks,
            qty: self.qty,
            side: self.side,
            max_wait_bars_for_ack: self.max_wait_bars_for_ack,
        }
    }
}

pub fn deterministic_request_id(
    strategy_id: &str,
    portfolio: &str,
    symbol: &str,
    action: &str,
    bar_ts: i64,
    seq: u8,
) -> Uuid {
    let name = format!("{strategy_id}|{portfolio}|{symbol}|{action}|{bar_ts}|{seq}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

pub fn build_place_command(
    config: &LimitCancelConfig,
    strategy_id: &str,
    portfolio: &str,
    exchange: &str,
    bar: &BarEvent,
) -> OrderCommand {
    let price = match config.side {
        Side::Buy => bar.close - (config.offset_ticks as f64) * config.tick_size,
        Side::Sell => bar.close + (config.offset_ticks as f64) * config.tick_size,
    };
    let request_id = deterministic_request_id(
        strategy_id,
        portfolio,
        &bar.symbol,
        "place",
        bar.close_time_utc,
        0,
    );
    OrderCommand {
        request_id,
        created_ts_utc: bar.close_time_utc,
        strategy_id: strategy_id.to_string(),
        portfolio: portfolio.to_string(),
        exchange: exchange.to_string(),
        symbol: bar.symbol.clone(),
        action: CommandAction::Place(PlaceOrder {
            price,
            qty: config.qty,
            side: config.side,
        }),
        ttl_ms: None,
    }
}

pub fn build_cancel_command(
    strategy_id: &str,
    portfolio: &str,
    exchange: &str,
    symbol: &str,
    order_id: i64,
    bar_ts: i64,
) -> OrderCommand {
    let request_id = deterministic_request_id(strategy_id, portfolio, symbol, "cancel", bar_ts, 1);
    OrderCommand {
        request_id,
        created_ts_utc: bar_ts,
        strategy_id: strategy_id.to_string(),
        portfolio: portfolio.to_string(),
        exchange: exchange.to_string(),
        symbol: symbol.to_string(),
        action: CommandAction::Cancel(alor_protocol::CancelOrder { order_id }),
        ttl_ms: None,
    }
}

#[derive(Debug, Default)]
pub struct RuntimeCaches {
    pub orders: HashMap<i64, OrderEvent>,
    pub positions: HashMap<String, PositionEvent>,
}
