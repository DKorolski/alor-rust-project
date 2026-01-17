use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::debug;

use crate::models::{OrderEvent, OrdersSnapshot};

#[derive(Clone)]
pub struct OrdersManagerHandle {
    store: Arc<RwLock<HashMap<i64, OrderEvent>>>,
}

pub struct OrdersManager;

impl OrdersManager {
    pub fn start(mut rx: mpsc::Receiver<OrderEvent>) -> OrdersManagerHandle {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let store_clone = store.clone();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                debug!(order_id = event.order_id, status = %event.status, "order update");
                if is_terminal(&event.status) {
                    store_clone.write().remove(&event.order_id);
                } else {
                    store_clone.write().insert(event.order_id, event);
                }
            }
        });

        OrdersManagerHandle { store }
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
                if order.symbol == symbol {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }
}

fn is_terminal(status: &str) -> bool {
    matches!(status.to_ascii_lowercase().as_str(), "filled" | "cancelled" | "rejected")
}
