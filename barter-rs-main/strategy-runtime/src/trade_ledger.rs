use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;

use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TradeRecord {
    pub ts_utc: i64,
    pub order_id: i64,
    pub symbol: String,
    pub side: String,
    pub qty: f64,
    pub price: f64,
    pub commission: f64,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderRecord {
    pub order_id: i64,
    pub symbol: String,
    pub side: String,
    pub qty: f64,
    pub filled: f64,
    pub price: f64,
    pub status: String,
    pub ts_utc: i64,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClosedTradeRecord {
    pub entry_ts_utc: i64,
    pub exit_ts_utc: i64,
    pub symbol: String,
    pub side: String,
    pub qty: f64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub commission_total: f64,
    pub pnl_gross: f64,
    pub pnl_net: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LedgerSummary {
    pub strategy_id: String,
    pub symbol: String,
    pub trades_total: usize,
    pub win_rate: f64,
    pub pnl_gross_total: f64,
    pub pnl_net_total: f64,
    pub commission_total: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub avg_pnl: f64,
    pub max_pnl: f64,
    pub min_pnl: f64,
}

#[derive(Debug, Default)]
pub struct TradeLedger {
    orders: HashMap<i64, OrderRecord>,
    trades: Vec<TradeRecord>,
    closed_trades: Vec<ClosedTradeRecord>,
    realized_pnl: f64,
    position_qty: f64,
    position_cost: f64,
    entry_ts_utc: Option<i64>,
    entry_price: f64,
    entry_side: Option<String>,
    entry_symbol: Option<String>,
    open_commission_total: f64,
}

impl TradeLedger {
    pub fn record_order(&mut self, record: OrderRecord) {
        self.orders.insert(record.order_id, record);
    }

    pub fn record_fill(&mut self, trade: TradeRecord) {
        self.apply_fill(&trade);
        self.trades.push(trade);
    }

    pub fn summary(&self, strategy_id: &str, symbol: &str) -> LedgerSummary {
        let trades_total = self.closed_trades.len();
        let mut gross_profit = 0.0;
        let mut gross_loss = 0.0;
        let mut max_pnl = 0.0;
        let mut min_pnl = 0.0;
        let mut pnl_gross_total = 0.0;
        let mut pnl_net_total = 0.0;
        let mut commission_total = 0.0;
        for (idx, trade) in self.closed_trades.iter().enumerate() {
            pnl_gross_total += trade.pnl_gross;
            pnl_net_total += trade.pnl_net;
            commission_total += trade.commission_total;
            if trade.pnl_gross >= 0.0 {
                gross_profit += trade.pnl_gross;
            } else {
                gross_loss += trade.pnl_gross.abs();
            }
            if idx == 0 || trade.pnl_net > max_pnl {
                max_pnl = trade.pnl_net;
            }
            if idx == 0 || trade.pnl_net < min_pnl {
                min_pnl = trade.pnl_net;
            }
        }
        let pnl_total = pnl_net_total;
        let avg_pnl = if trades_total == 0 {
            0.0
        } else {
            pnl_total / trades_total as f64
        };
        let wins = self
            .closed_trades
            .iter()
            .filter(|trade| trade.pnl_net > 0.0)
            .count();
        let win_rate = if trades_total == 0 {
            0.0
        } else {
            wins as f64 / trades_total as f64
        };
        LedgerSummary {
            strategy_id: strategy_id.to_string(),
            symbol: symbol.to_string(),
            trades_total,
            win_rate,
            pnl_gross_total,
            pnl_net_total,
            commission_total,
            gross_profit,
            gross_loss,
            avg_pnl,
            max_pnl,
            min_pnl,
        }
    }

    pub fn persist_reports(
        &self,
        strategy_id: &str,
        symbol: &str,
        trades_csv: &str,
        summary_json: &str,
    ) -> Result<()> {
        self.write_trades_csv(trades_csv)?;
        self.write_summary_json(strategy_id, symbol, summary_json)?;
        Ok(())
    }

    pub fn order(&self, order_id: i64) -> Option<&OrderRecord> {
        self.orders.get(&order_id)
    }

    pub fn orders_total(&self) -> usize {
        self.orders.len()
    }

    pub fn trades(&self) -> &[TradeRecord] {
        &self.trades
    }

    pub fn closed_trades(&self) -> &[ClosedTradeRecord] {
        &self.closed_trades
    }

    fn write_trades_csv(&self, path: &str) -> Result<()> {
        let mut file = File::create(path)?;
        writeln!(
            file,
            "entry_ts_utc,exit_ts_utc,symbol,side,qty,entry_price,exit_price,commission_total,pnl_gross,pnl_net"
        )?;
        for trade in &self.closed_trades {
            writeln!(
                file,
                "{},{},{},{},{},{},{},{},{},{}",
                trade.entry_ts_utc,
                trade.exit_ts_utc,
                trade.symbol,
                trade.side,
                trade.qty,
                trade.entry_price,
                trade.exit_price,
                trade.commission_total,
                trade.pnl_gross,
                trade.pnl_net
            )?;
        }
        Ok(())
    }

    fn write_summary_json(&self, strategy_id: &str, symbol: &str, path: &str) -> Result<()> {
        let summary = self.summary(strategy_id, symbol);
        let payload = serde_json::to_string_pretty(&summary)?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(payload.as_bytes())?;
        Ok(())
    }

    fn apply_fill(&mut self, trade: &TradeRecord) {
        let before_qty = self.position_qty;
        let before_avg_price = if before_qty.abs() <= f64::EPSILON {
            0.0
        } else {
            self.position_cost / before_qty
        };
        let qty = trade.qty;
        let price = trade.price;
        let trade_commission = trade.commission;
        match trade.side.as_str() {
            "buy" => {
                if self.position_qty < 0.0 {
                    let avg_price = self.average_price();
                    let cover_qty = qty.min(self.position_qty.abs());
                    self.realized_pnl += (avg_price - price) * cover_qty;
                    self.position_qty += cover_qty;
                    self.position_cost += avg_price * cover_qty;
                    let remaining = qty - cover_qty;
                    if remaining > 0.0 {
                        self.position_qty += remaining;
                        self.position_cost += remaining * price;
                    }
                } else {
                    self.position_qty += qty;
                    self.position_cost += qty * price;
                }
            }
            "sell" => {
                if self.position_qty > 0.0 {
                    let avg_price = self.average_price();
                    let close_qty = qty.min(self.position_qty);
                    self.realized_pnl += (price - avg_price) * close_qty;
                    self.position_qty -= close_qty;
                    self.position_cost -= avg_price * close_qty;
                    let remaining = qty - close_qty;
                    if remaining > 0.0 {
                        self.position_qty -= remaining;
                        self.position_cost -= remaining * price;
                    }
                } else {
                    self.position_qty -= qty;
                    self.position_cost -= qty * price;
                }
            }
            _ => {}
        }
        let is_flat = self.position_qty.abs() <= f64::EPSILON;
        let was_flat = before_qty.abs() <= f64::EPSILON;
        let flipped = !was_flat && !is_flat && before_qty.signum() != self.position_qty.signum();
        let close_ratio = if flipped && qty > 0.0 {
            (before_qty.abs() / qty).min(1.0)
        } else {
            1.0
        };

        if !was_flat && (is_flat || flipped) {
            let entry_price = if self.entry_price > 0.0 {
                self.entry_price
            } else {
                before_avg_price.abs()
            };
            let entry_side = self.entry_side.clone().unwrap_or_else(|| {
                if before_qty > 0.0 {
                    "buy".to_string()
                } else {
                    "sell".to_string()
                }
            });
            let symbol = self
                .entry_symbol
                .clone()
                .unwrap_or_else(|| trade.symbol.clone());
            let close_qty = before_qty.abs();
            let pnl_gross = if entry_side == "buy" {
                (price - entry_price) * close_qty
            } else {
                (entry_price - price) * close_qty
            };
            let commission_total = if flipped {
                self.open_commission_total + (trade_commission * close_ratio)
            } else {
                self.open_commission_total + trade_commission
            };
            let pnl_net = pnl_gross - commission_total;
            if entry_price > 0.0 {
                self.closed_trades.push(ClosedTradeRecord {
                    entry_ts_utc: self.entry_ts_utc.unwrap_or(trade.ts_utc),
                    exit_ts_utc: trade.ts_utc,
                    symbol,
                    side: entry_side,
                    qty: close_qty,
                    entry_price,
                    exit_price: price,
                    commission_total,
                    pnl_gross,
                    pnl_net,
                });
            }
            self.entry_ts_utc = None;
            self.entry_side = None;
            self.entry_symbol = None;
            self.entry_price = 0.0;
            self.open_commission_total = 0.0;

            if flipped {
                self.entry_ts_utc = Some(trade.ts_utc);
                self.entry_side = Some(if self.position_qty > 0.0 {
                    "buy".to_string()
                } else {
                    "sell".to_string()
                });
                self.entry_symbol = Some(trade.symbol.clone());
                self.entry_price = self.average_price().abs();
                self.open_commission_total = trade_commission * (1.0 - close_ratio);
            }
        } else if was_flat && !is_flat {
            self.entry_ts_utc = Some(trade.ts_utc);
            self.entry_side = Some(trade.side.clone());
            self.entry_symbol = Some(trade.symbol.clone());
            self.entry_price = self.average_price().abs();
            self.open_commission_total = trade_commission;
        } else if !is_flat {
            self.entry_price = self.average_price().abs();
            self.open_commission_total += trade_commission;
        }
    }

    fn average_price(&self) -> f64 {
        if self.position_qty.abs() <= f64::EPSILON {
            0.0
        } else {
            self.position_cost / self.position_qty
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_records_round_trip() {
        let mut ledger = TradeLedger::default();
        ledger.record_fill(TradeRecord {
            ts_utc: 1,
            order_id: 1,
            symbol: "SBER".to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            price: 100.0,
            commission: 0.0,
            owned: true,
        });
        ledger.record_fill(TradeRecord {
            ts_utc: 2,
            order_id: 2,
            symbol: "SBER".to_string(),
            side: "sell".to_string(),
            qty: 1.0,
            price: 110.0,
            commission: 0.0,
            owned: true,
        });
        assert_eq!(ledger.closed_trades().len(), 1);
        assert!(ledger.closed_trades()[0].pnl_gross > 0.0);
    }
}
