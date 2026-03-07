use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use chrono::Utc;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::debug;

use crate::models::{StopOrderEvent, StopOrdersSnapshot};

#[derive(Clone)]
pub struct StopOrdersManagerHandle {
    store: Arc<RwLock<HashMap<String, StopOrderEvent>>>,
    synced: Arc<AtomicBool>,
    last_ts: Arc<AtomicI64>,
}

pub struct StopOrdersManager;

impl StopOrdersManager {
    pub fn start(mut rx: mpsc::Receiver<StopOrderEvent>) -> StopOrdersManagerHandle {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let synced = Arc::new(AtomicBool::new(false));
        let last_ts = Arc::new(AtomicI64::new(0));
        let store_clone = store.clone();
        let synced_clone = synced.clone();
        let last_ts_clone = last_ts.clone();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let recv_ts = Utc::now().timestamp();
                debug!(
                    stop_order_id = event.stop_order_id,
                    exchange_order_id = ?event.exchange_order_id,
                    symbol = event.symbol,
                    status = event.status,
                    side = event.side,
                    qty = event.qty,
                    filled = event.filled,
                    stop_price = event.stop_price,
                    price = event.price,
                    existing = event.existing,
                    event_ts = event.ts_utc,
                    recv_ts,
                    "stop order update"
                );
                Self::apply_event(&store_clone, &last_ts_clone, event);
                synced_clone.store(true, Ordering::SeqCst);
            }
        });

        StopOrdersManagerHandle {
            store,
            synced,
            last_ts,
        }
    }

    fn apply_event(
        store: &Arc<RwLock<HashMap<String, StopOrderEvent>>>,
        last_ts: &Arc<AtomicI64>,
        event: StopOrderEvent,
    ) {
        let incoming_ts = event.ts_utc;
        {
            let mut guard = store.write();
            let should_replace = match guard.get(&event.stop_order_id) {
                None => true,
                Some(existing) => existing.ts_utc <= 0 || (incoming_ts > 0 && incoming_ts >= existing.ts_utc),
            };
            if should_replace {
                guard.insert(event.stop_order_id.clone(), event);
            }
        }
        if incoming_ts > 0 {
            let prev = last_ts.load(Ordering::SeqCst);
            if incoming_ts > prev {
                last_ts.store(incoming_ts, Ordering::SeqCst);
            }
        }
    }
}

impl StopOrdersManagerHandle {
    pub fn snapshot(&self) -> StopOrdersSnapshot {
        let stop_orders = self.store.read().clone();
        StopOrdersSnapshot { stop_orders }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(stop_order_id: &str, ts_utc: i64, status: &str) -> StopOrderEvent {
        StopOrderEvent {
            stop_order_id: stop_order_id.to_string(),
            exchange_order_id: None,
            symbol: "IMOEXF".to_string(),
            status: status.to_string(),
            side: "buy".to_string(),
            qty: 1.0,
            filled: 0.0,
            stop_price: 100.0,
            price: 100.5,
            existing: false,
            comment: None,
            end_time: None,
            ts_utc,
        }
    }

    #[test]
    fn stop_orders_manager_ignores_older_event_for_same_id() {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let last_ts = Arc::new(AtomicI64::new(0));

        StopOrdersManager::apply_event(&store, &last_ts, sample_event("s1", 1000, "working"));
        StopOrdersManager::apply_event(&store, &last_ts, sample_event("s1", 900, "canceled"));

        assert_eq!(last_ts.load(Ordering::SeqCst), 1000);
        let current = store.read().get("s1").cloned().expect("missing stop order");
        assert_eq!(current.ts_utc, 1000);
        assert_eq!(current.status, "working");
    }
}
