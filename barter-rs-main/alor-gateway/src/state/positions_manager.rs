use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

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
    pub fn start(mut rx: mpsc::Receiver<PositionEvent>) -> PositionsManagerHandle {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let synced = Arc::new(AtomicBool::new(false));
        let last_ts = Arc::new(AtomicI64::new(0));
        let store_clone = store.clone();
        let synced_clone = synced.clone();
        let last_ts_clone = last_ts.clone();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                debug!(symbol = %event.symbol, qty = event.qty, "position update");
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
}
