use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use chrono::Utc;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::debug;

use crate::models::{OrderEvent, OrdersSnapshot};

#[derive(Clone)]
pub struct OrdersManagerHandle {
    store: Arc<RwLock<HashMap<i64, OrderEvent>>>,
    synced: Arc<AtomicBool>,
    last_ts: Arc<AtomicI64>,
}

pub struct OrdersManager;

impl OrdersManager {
    pub fn start(
        mut rx: mpsc::Receiver<OrderEvent>,
        log_existing_snapshot_orders: bool,
    ) -> OrdersManagerHandle {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let synced = Arc::new(AtomicBool::new(false));
        let last_ts = Arc::new(AtomicI64::new(0));
        let store_clone = store.clone();
        let synced_clone = synced.clone();
        let last_ts_clone = last_ts.clone();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let status = event.status.to_ascii_lowercase();
                let is_snapshot = event.existing && status != "working";
                if log_existing_snapshot_orders || !is_snapshot {
                    let recv_ts = Utc::now().timestamp();
                    debug!(
                        order_id = event.order_id,
                        status = %event.status,
                        side = %event.side,
                        order_type = %event.order_type,
                        price = event.price,
                        qty = event.qty,
                        filled = event.filled,
                        existing = event.existing,
                        event_ts = event.ts_utc,
                        recv_ts,
                        "order update"
                    );
                }
                let ts = event.ts_utc;
                store_clone.write().insert(event.order_id, event);
                synced_clone.store(true, Ordering::SeqCst);
                last_ts_clone.store(ts, Ordering::SeqCst);
            }
        });

        OrdersManagerHandle {
            store,
            synced,
            last_ts,
        }
    }
}

impl OrdersManagerHandle {
    pub fn snapshot(&self) -> OrdersSnapshot {
        let orders = self.store.read().clone();
        OrdersSnapshot { orders }
    }

    pub fn active_orders_for(&self, symbol: &str) -> Vec<i64> {
        self.store
            .read()
            .iter()
            .filter_map(|(id, order)| {
                if order.symbol == symbol && !is_terminal(&order.status) {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
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

fn is_terminal(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "filled" | "cancelled" | "canceled" | "rejected"
    )
}
