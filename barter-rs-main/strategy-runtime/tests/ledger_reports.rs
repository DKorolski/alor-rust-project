use std::fs;

use strategy_runtime::trade_ledger::{TradeLedger, TradeRecord};
use tempfile::tempdir;

#[test]
fn ledger_exports_reports() {
    let dir = tempdir().expect("tempdir");
    let trades_csv = dir.path().join("trades.csv");
    let summary_json = dir.path().join("summary.json");

    let mut ledger = TradeLedger::default();
    ledger.record_fill(TradeRecord {
        ts_utc: 1,
        order_id: 1,
        symbol: "IMOEXF".to_string(),
        side: "sell".to_string(),
        qty: 1.0,
        price: 2800.0,
        commission: 0.0,
    });
    ledger.record_fill(TradeRecord {
        ts_utc: 2,
        order_id: 2,
        symbol: "IMOEXF".to_string(),
        side: "buy".to_string(),
        qty: 1.0,
        price: 2790.0,
        commission: 0.0,
    });

    ledger
        .persist_reports(
            "limit_cancel",
            "IMOEXF",
            trades_csv.to_str().expect("trades path"),
            summary_json.to_str().expect("summary path"),
        )
        .expect("persist reports");

    let trades_contents = fs::read_to_string(&trades_csv).expect("read trades");
    let trades_lines: Vec<_> = trades_contents.lines().collect();
    assert_eq!(trades_lines.len(), 2, "header + 1 trade");
    assert!(trades_lines[0].contains("pnl_net"));

    let summary_contents = fs::read_to_string(&summary_json).expect("read summary");
    let summary: serde_json::Value =
        serde_json::from_str(&summary_contents).expect("parse summary");
    assert_eq!(summary["trades_total"], 1);
    assert_eq!(summary["pnl_gross_total"], 10.0);
    assert_eq!(summary["pnl_net_total"], 10.0);
}
