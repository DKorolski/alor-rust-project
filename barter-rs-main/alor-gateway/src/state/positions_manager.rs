use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::debug;

use crate::models::{PositionEvent, PositionsSnapshot};

#[derive(Clone)]
pub struct PositionsManagerHandle {
    store: Arc<RwLock<HashMap<String, PositionEvent>>>,
}

pub struct PositionsManager;

impl PositionsManager {
    pub fn start(mut rx: mpsc::Receiver<PositionEvent>) -> PositionsManagerHandle {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let store_clone = store.clone();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                debug!(symbol = %event.symbol, qty = event.qty, "position update");
                store_clone.write().insert(event.symbol.clone(), event);
            }
        });

        PositionsManagerHandle { store }
    }
}

impl PositionsManagerHandle {
    pub fn snapshot(&self) -> PositionsSnapshot {
        let positions = self.store.read().clone();
        PositionsSnapshot { positions }
    }
}
