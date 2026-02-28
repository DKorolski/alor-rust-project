use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use chrono::Utc;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::debug;

use crate::models::{PositionEvent, PositionsSnapshot};

#[derive(Clone)]
pub struct PositionsManagerHandle {
    store: Arc<RwLock<HashMap<String, PositionEvent>>>,
    synced: Arc<AtomicBool>,
    last_ts: Arc<AtomicI64>,
}

pub struct PositionsManager;

impl PositionsManager {
    pub fn start(
        mut rx: mpsc::Receiver<PositionEvent>,
        log_positions_filter: Vec<String>,
        log_cash_positions: bool,
        cash_symbols: Vec<String>,
    ) -> PositionsManagerHandle {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let synced = Arc::new(AtomicBool::new(false));
        let last_ts = Arc::new(AtomicI64::new(0));
        let store_clone = store.clone();
        let synced_clone = synced.clone();
        let last_ts_clone = last_ts.clone();
        let log_filter: HashSet<String> = log_positions_filter
            .into_iter()
            .map(|s| s.to_uppercase())
            .collect();
        let cash_filter: HashSet<String> =
            cash_symbols.into_iter().map(|s| s.to_uppercase()).collect();
        let mut last_qty_by_symbol: HashMap<String, f64> = HashMap::new();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let recv_ts = Utc::now().timestamp();
                let symbol_upper = event.symbol.to_uppercase();
                let should_log = log_filter.contains(&symbol_upper)
                    || (log_cash_positions && cash_filter.contains(&symbol_upper));
                let last_qty = last_qty_by_symbol.get(&event.symbol).copied();
                if should_log && (last_qty != Some(event.qty)) {
                    debug!(
                        symbol = %event.symbol,
                        qty = event.qty,
                        event_ts = event.ts_utc,
                        recv_ts,
                        "position update"
                    );
                }
                last_qty_by_symbol.insert(event.symbol.clone(), event.qty);
                let ts = event.ts_utc;
                store_clone.write().insert(event.symbol.clone(), event);
                synced_clone.store(true, Ordering::SeqCst);
                last_ts_clone.store(ts, Ordering::SeqCst);
            }
        });

        PositionsManagerHandle {
            store,
            synced,
            last_ts,
        }
    }
}

impl PositionsManagerHandle {
    pub fn snapshot(&self) -> PositionsSnapshot {
        let positions = self.store.read().clone();
        PositionsSnapshot { positions }
    }

    pub fn synced(&self) -> bool {
        self.synced.load(Ordering::SeqCst)
    }

    pub fn last_ts(&self) -> i64 {
        self.last_ts.load(Ordering::SeqCst)
    }

    pub fn mark_synced(&self) {
        self.synced.store(true, Ordering::SeqCst);
        self.last_ts.store(Utc::now().timestamp(), Ordering::SeqCst);
    }
}
