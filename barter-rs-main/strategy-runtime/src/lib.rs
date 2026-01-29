pub mod redis_transport;
pub mod runtime;
pub mod state;
pub mod strategy_limit_cancel;

use std::collections::HashMap;

use alor_protocol::{CommandAction, OrderCommand, PlaceOrder, Side};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::strategy_limit_cancel::LimitCancelConfig;

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

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub redis_url: String,
    pub source: String,
    pub strategy_id: String,
    pub portfolio: String,
    pub exchange: String,
    pub streams: StreamNames,
    pub runtime_state_stream: String,
    pub trim_maxlen_runtime_state: usize,
    pub consumer_group: String,
    pub consumer_name: String,
    pub block_ms: usize,
    pub claim_idle_ms: usize,
    pub claim_batch: usize,
    pub poll_interval_ms: u64,
    pub trim_maxlen_bars: usize,
    pub trim_maxlen_orders: usize,
    pub trim_maxlen_positions: usize,
    pub trim_maxlen_commands: usize,
    pub trim_maxlen_acks: usize,
    pub trim_maxlen_health: usize,
    pub limit_cancel: LimitCancelConfig,
    pub reset_state_on_start: bool,
}

#[derive(Debug, Clone)]
pub struct StreamNames {
    pub bars: String,
    pub orders: String,
    pub positions: String,
    pub commands: String,
    pub acks: String,
    pub health: Option<String>,
    pub dlq_prefix: String,
}

pub fn deterministic_request_id(
    strategy_id: &str,
    portfolio: &str,
    symbol: &str,
    action: &str,
    bar_ts: i64,
    seq: u8,
) -> Uuid {
    let name = format!(
        "{strategy_id}|{portfolio}|{symbol}|{action}|{bar_ts}|{seq}"
    );
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
