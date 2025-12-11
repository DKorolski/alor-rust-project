mod alor_market;
mod engine;

use crate::alor_market::alor_siz5_stream;
use crate::engine::{ScalpingEngine, StrategyConfig};
use futures_util::StreamExt;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    alor_market::load_local_env();
    alor_market::init_logging();

    let cfg = StrategyConfig {
        symbol: "SIZ5".to_string(),
        depth: 10,
        tick_size: 1.0,
        contract_size: 1.0,
        order_size: 1.0,
        maker_fee: 0.0,
        taker_fee: 0.0,
        broker_fee_abs: 1.0,
        tp_ticks: 50,
        entry_cluster_q: 0.795,
        exit_cluster_q: 0.05,
        max_cluster_depth_entry: 3,
        max_cluster_depth_exit: 10,
        entry_order_timeout_ms: 800,
        exit_order_timeout_ms: 500,
        exit_cluster_start_ms: 500,
        exit_cluster_max_diff_ticks: 45,
        adverse_ticks: 10,
        min_history_for_quantiles: 50,
        entry_bid_thr: Some(74.0),
        entry_ask_thr: Some(86.0),
        exit_bid_thr: Some(14.0),
        exit_ask_thr: Some(14.0),
        order_placement_delay_ms: 0,
        csv_path: "alor-scalping/data/trades_live_alor_siz5.csv".to_string(),
    };

    let mut engine = ScalpingEngine::new(cfg);
    let mut stream = alor_siz5_stream().await;

    while let Some(event) = stream.next().await {
        let cmds = engine.on_event(event);
        for cmd in cmds {
            info!(?cmd, "strategy order command");
        }
    }

    Ok(())
}