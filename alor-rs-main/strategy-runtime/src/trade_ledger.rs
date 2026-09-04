use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

static REPORT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    pub comment: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTradeRecord {
    pub entry_ts_utc: i64,
    pub exit_ts_utc: i64,
    pub symbol: String,
    #[serde(default)]
    pub strategy_component: Option<String>,
    pub side: String,
    pub qty: f64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub commission_total: f64,
    pub pnl_gross: f64,
    pub pnl_net: f64,
    #[serde(default)]
    pub entry_role: Option<String>,
    #[serde(default)]
    pub exit_role: Option<String>,
    #[serde(default)]
    pub entry_comment: Option<String>,
    #[serde(default)]
    pub exit_comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClosedTradeKey {
    entry_ts_utc: i64,
    exit_ts_utc: i64,
    symbol: String,
    side: String,
    qty: u64,
    entry_price: u64,
    exit_price: u64,
    commission_total: u64,
    pnl_gross: u64,
    pnl_net: u64,
}

impl From<&ClosedTradeRecord> for ClosedTradeKey {
    fn from(trade: &ClosedTradeRecord) -> Self {
        Self {
            entry_ts_utc: trade.entry_ts_utc,
            exit_ts_utc: trade.exit_ts_utc,
            symbol: trade.symbol.clone(),
            side: trade.side.clone(),
            qty: trade.qty.to_bits(),
            entry_price: trade.entry_price.to_bits(),
            exit_price: trade.exit_price.to_bits(),
            commission_total: trade.commission_total.to_bits(),
            pnl_gross: trade.pnl_gross.to_bits(),
            pnl_net: trade.pnl_net.to_bits(),
        }
    }
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
    entry_comment: Option<String>,
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
        Self::summary_for_trades(strategy_id, symbol, &self.closed_trades)
    }

    fn summary_for_trades(
        strategy_id: &str,
        symbol: &str,
        closed_trades: &[ClosedTradeRecord],
    ) -> LedgerSummary {
        let trades_total = closed_trades.len();
        let mut gross_profit = 0.0;
        let mut gross_loss = 0.0;
        let mut max_pnl = 0.0;
        let mut min_pnl = 0.0;
        let mut pnl_gross_total = 0.0;
        let mut pnl_net_total = 0.0;
        let mut commission_total = 0.0;
        for (idx, trade) in closed_trades.iter().enumerate() {
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
        let wins = closed_trades
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
        append: bool,
    ) -> Result<()> {
        let report_trades = self.report_trades(trades_csv, append)?;
        Self::write_trades_csv(trades_csv, &report_trades)?;
        Self::write_summary_json(strategy_id, symbol, summary_json, &report_trades)?;
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

    fn report_trades(&self, path: &str, append: bool) -> Result<Vec<ClosedTradeRecord>> {
        let mut trades = if append && Path::new(path).exists() {
            Self::read_trades_csv(path)?
        } else {
            Vec::new()
        };
        trades.extend(self.closed_trades.iter().cloned());
        trades.sort_by(|left, right| {
            left.entry_ts_utc
                .cmp(&right.entry_ts_utc)
                .then(left.exit_ts_utc.cmp(&right.exit_ts_utc))
                .then(left.symbol.cmp(&right.symbol))
                .then(left.side.cmp(&right.side))
        });
        let mut seen = HashSet::with_capacity(trades.len());
        trades.retain(|trade| seen.insert(ClosedTradeKey::from(trade)));
        Ok(trades)
    }

    fn read_trades_csv(path: &str) -> Result<Vec<ClosedTradeRecord>> {
        if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() == 0) {
            return Ok(Vec::new());
        }
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)
            .with_context(|| format!("open existing trade report: {path}"))?;
        reader
            .deserialize()
            .collect::<std::result::Result<Vec<ClosedTradeRecord>, csv::Error>>()
            .with_context(|| format!("parse existing trade report: {path}"))
    }

    fn write_trades_csv(path: &str, trades: &[ClosedTradeRecord]) -> Result<()> {
        ensure_parent_dir(path)?;
        replace_file_atomically(path, |file| {
            let mut writer = csv::Writer::from_writer(file);
            for trade in trades {
                writer.serialize(trade)?;
            }
            writer.flush()?;
            Ok(())
        })
    }

    fn write_summary_json(
        strategy_id: &str,
        symbol: &str,
        path: &str,
        trades: &[ClosedTradeRecord],
    ) -> Result<()> {
        ensure_parent_dir(path)?;
        let summary = Self::summary_for_trades(strategy_id, symbol, trades);
        let payload = serde_json::to_string_pretty(&summary)?;
        replace_file_atomically(path, |file| {
            file.write_all(payload.as_bytes())?;
            Ok(())
        })
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
            let entry_comment = self.entry_comment.clone();
            let exit_comment = trade.comment.clone();
            let entry_tag = HybridTradeTag::parse(entry_comment.as_deref());
            let exit_tag = HybridTradeTag::parse(exit_comment.as_deref());
            let strategy_component = entry_tag
                .as_ref()
                .and_then(|tag| tag.owner.clone())
                .or_else(|| exit_tag.as_ref().and_then(|tag| tag.owner.clone()));
            if entry_price > 0.0 {
                self.closed_trades.push(ClosedTradeRecord {
                    entry_ts_utc: self.entry_ts_utc.unwrap_or(trade.ts_utc),
                    exit_ts_utc: trade.ts_utc,
                    symbol,
                    strategy_component,
                    side: entry_side,
                    qty: close_qty,
                    entry_price,
                    exit_price: price,
                    commission_total,
                    pnl_gross,
                    pnl_net,
                    entry_role: entry_tag.and_then(|tag| tag.role),
                    exit_role: exit_tag.and_then(|tag| tag.role),
                    entry_comment,
                    exit_comment,
                });
            }
            self.entry_ts_utc = None;
            self.entry_side = None;
            self.entry_symbol = None;
            self.entry_comment = None;
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
                self.entry_comment = trade.comment.clone();
                self.entry_price = self.average_price().abs();
                self.open_commission_total = trade_commission * (1.0 - close_ratio);
            }
        } else if was_flat && !is_flat {
            self.entry_ts_utc = Some(trade.ts_utc);
            self.entry_side = Some(trade.side.clone());
            self.entry_symbol = Some(trade.symbol.clone());
            self.entry_comment = trade.comment.clone();
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

#[derive(Debug, Clone)]
struct HybridTradeTag {
    owner: Option<String>,
    role: Option<String>,
}

impl HybridTradeTag {
    fn parse(comment: Option<&str>) -> Option<Self> {
        let comment = comment?;
        if !comment.is_ascii() || !comment.starts_with("HYB|") {
            return None;
        }
        let mut owner = None;
        let mut role = None;
        for part in comment.split('|').skip(1) {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            match key {
                "o" => match value {
                    "MR" | "BO" => owner = Some(value.to_string()),
                    _ => {}
                },
                "r" => match value {
                    "ENTRY" | "EXIT" | "TP" | "SL" | "CANCEL" | "REPAIR" => {
                        role = Some(value.to_string())
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Some(Self { owner, role })
    }
}

fn ensure_parent_dir(path: &str) -> Result<()> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn replace_file_atomically(path: &str, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    let sequence = REPORT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_path = format!("{path}.tmp-{}-{sequence}", std::process::id());
    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temporary_path)?;
        write(&mut file)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
            comment: None,
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
            comment: None,
        });
        assert_eq!(ledger.closed_trades().len(), 1);
        assert!(ledger.closed_trades()[0].pnl_gross > 0.0);
    }

    #[test]
    fn persist_reports_creates_parent_directories() {
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
            comment: None,
        });
        ledger.record_fill(TradeRecord {
            ts_utc: 2,
            order_id: 2,
            symbol: "SBER".to_string(),
            side: "sell".to_string(),
            qty: 1.0,
            price: 101.0,
            commission: 0.0,
            owned: true,
            comment: None,
        });
        let dir = tempdir().expect("tempdir");
        let trades = dir.path().join("nested/reports/trades.csv");
        let summary = dir.path().join("nested/reports/summary.json");
        ledger
            .persist_reports(
                "hybrid_intraday",
                "SBER",
                trades.to_str().expect("trades path utf8"),
                summary.to_str().expect("summary path utf8"),
                false,
            )
            .expect("persist reports");
        assert!(trades.exists());
        assert!(summary.exists());
    }
}
