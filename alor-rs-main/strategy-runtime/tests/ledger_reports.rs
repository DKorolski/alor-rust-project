use std::fs;

use strategy_runtime::trade_ledger::{TradeLedger, TradeRecord};
use tempfile::tempdir;

fn record_round_trip(
    ledger: &mut TradeLedger,
    entry_ts_utc: i64,
    exit_ts_utc: i64,
    entry_price: f64,
    exit_price: f64,
) {
    ledger.record_fill(TradeRecord {
        ts_utc: entry_ts_utc,
        order_id: entry_ts_utc,
        symbol: "IMOEXF".to_string(),
        side: "buy".to_string(),
        qty: 1.0,
        price: entry_price,
        commission: 0.0,
        owned: true,
        comment: None,
    });
    ledger.record_fill(TradeRecord {
        ts_utc: exit_ts_utc,
        order_id: exit_ts_utc,
        symbol: "IMOEXF".to_string(),
        side: "sell".to_string(),
        qty: 1.0,
        price: exit_price,
        commission: 0.0,
        owned: true,
        comment: None,
    });
}

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
        owned: true,
        comment: None,
    });
    ledger.record_fill(TradeRecord {
        ts_utc: 2,
        order_id: 2,
        symbol: "IMOEXF".to_string(),
        side: "buy".to_string(),
        qty: 1.0,
        price: 2790.0,
        commission: 0.0,
        owned: true,
        comment: None,
    });

    ledger
        .persist_reports(
            "limit_cancel",
            "IMOEXF",
            trades_csv.to_str().expect("trades path"),
            summary_json.to_str().expect("summary path"),
            false,
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

#[test]
fn append_reports_preserve_trades_across_runtime_generations() {
    let dir = tempdir().expect("tempdir");
    let trades_csv = dir.path().join("trades.csv");
    let summary_json = dir.path().join("summary.json");

    let mut first_generation = TradeLedger::default();
    record_round_trip(&mut first_generation, 1, 2, 100.0, 110.0);
    first_generation
        .persist_reports(
            "hybrid_intraday",
            "IMOEXF",
            trades_csv.to_str().expect("trades path"),
            summary_json.to_str().expect("summary path"),
            true,
        )
        .expect("persist first generation");

    let mut second_generation = TradeLedger::default();
    record_round_trip(&mut second_generation, 3, 4, 120.0, 115.0);
    second_generation
        .persist_reports(
            "hybrid_intraday",
            "IMOEXF",
            trades_csv.to_str().expect("trades path"),
            summary_json.to_str().expect("summary path"),
            true,
        )
        .expect("persist second generation");

    let trades_contents = fs::read_to_string(&trades_csv).expect("read trades");
    assert_eq!(trades_contents.lines().count(), 3, "header + 2 trades");
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&summary_json).expect("read summary"))
            .expect("parse summary");
    assert_eq!(summary["trades_total"], 2);
    assert_eq!(summary["pnl_net_total"], 5.0);
}

#[test]
fn append_reports_deduplicate_replayed_closed_trades() {
    let dir = tempdir().expect("tempdir");
    let trades_csv = dir.path().join("trades.csv");
    let summary_json = dir.path().join("summary.json");

    for _ in 0..2 {
        let mut replayed_generation = TradeLedger::default();
        record_round_trip(&mut replayed_generation, 1, 2, 100.0, 110.0);
        replayed_generation
            .persist_reports(
                "hybrid_intraday",
                "IMOEXF",
                trades_csv.to_str().expect("trades path"),
                summary_json.to_str().expect("summary path"),
                true,
            )
            .expect("persist replayed generation");
    }

    let trades_contents = fs::read_to_string(&trades_csv).expect("read trades");
    assert_eq!(trades_contents.lines().count(), 2, "header + 1 trade");
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&summary_json).expect("read summary"))
            .expect("parse summary");
    assert_eq!(summary["trades_total"], 1);
    assert_eq!(summary["pnl_net_total"], 10.0);
}

#[test]
fn append_reports_reads_legacy_csv_without_component_columns() {
    let dir = tempdir().expect("tempdir");
    let trades_csv = dir.path().join("trades.csv");
    let summary_json = dir.path().join("summary.json");
    fs::write(
        &trades_csv,
        "entry_ts_utc,exit_ts_utc,symbol,side,qty,entry_price,exit_price,commission_total,pnl_gross,pnl_net\n\
1,2,IMOEXF,buy,1,100,110,0,10,10\n",
    )
    .expect("write legacy csv");

    let mut next_generation = TradeLedger::default();
    record_round_trip(&mut next_generation, 3, 4, 120.0, 121.0);
    next_generation
        .persist_reports(
            "hybrid_intraday",
            "IMOEXF",
            trades_csv.to_str().expect("trades path"),
            summary_json.to_str().expect("summary path"),
            true,
        )
        .expect("append to legacy csv");

    let trades_contents = fs::read_to_string(&trades_csv).expect("read trades");
    assert_eq!(trades_contents.lines().count(), 3, "header + 2 trades");
    assert!(trades_contents
        .lines()
        .next()
        .unwrap()
        .contains("strategy_component"));
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&summary_json).expect("read summary"))
            .expect("parse summary");
    assert_eq!(summary["trades_total"], 2);
    assert_eq!(summary["pnl_net_total"], 11.0);
}

#[test]
fn hybrid_comment_tags_export_component_context() {
    let dir = tempdir().expect("tempdir");
    let trades_csv = dir.path().join("trades.csv");
    let summary_json = dir.path().join("summary.json");

    let mut ledger = TradeLedger::default();
    ledger.record_fill(TradeRecord {
        ts_utc: 1,
        order_id: 1,
        symbol: "IMOEXF".to_string(),
        side: "sell".to_string(),
        qty: 6.0,
        price: 2200.0,
        commission: 0.0,
        owned: true,
        comment: Some("HYB|sid=hybrid_imoexf_shadow|c=abc1230000|o=MR|r=ENTRY".to_string()),
    });
    ledger.record_fill(TradeRecord {
        ts_utc: 2,
        order_id: 2,
        symbol: "IMOEXF".to_string(),
        side: "buy".to_string(),
        qty: 6.0,
        price: 2190.0,
        commission: 0.0,
        owned: true,
        comment: Some("HYB|sid=hybrid_imoexf_shadow|c=abc1230000|o=MR|r=EXIT".to_string()),
    });

    ledger
        .persist_reports(
            "hybrid_intraday",
            "IMOEXF",
            trades_csv.to_str().expect("trades path"),
            summary_json.to_str().expect("summary path"),
            false,
        )
        .expect("persist reports");

    let trades_contents = fs::read_to_string(&trades_csv).expect("read trades");
    assert!(trades_contents
        .lines()
        .next()
        .unwrap()
        .contains("strategy_component"));
    assert!(trades_contents.contains(",MR,sell,"));
    assert!(trades_contents.contains(",ENTRY,EXIT,"));
}
