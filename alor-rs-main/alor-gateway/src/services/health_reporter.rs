use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::health::HealthState;
use crate::services::event_publisher::EventPublisherHandle;
use crate::state::orders_manager::OrdersManagerHandle;
use crate::state::positions_manager::PositionsManagerHandle;
use crate::transport::EventMessage;

pub async fn run_health_reporter(
    publisher: EventPublisherHandle,
    health: Arc<parking_lot::RwLock<HealthState>>,
    orders: OrdersManagerHandle,
    positions: PositionsManagerHandle,
    mut shutdown_rx: watch::Receiver<bool>,
    health_interval: Duration,
    snapshot_interval: Duration,
) {
    let mut health_tick = tokio::time::interval(health_interval);
    let mut snapshot_tick = tokio::time::interval(snapshot_interval);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = health_tick.tick() => {
                let snapshot = health.read().clone();
                publisher.publish_lossy(EventMessage::Health(snapshot));
            }
            _ = snapshot_tick.tick() => {
                publisher
                    .publish_critical(EventMessage::SnapshotOrders(orders.snapshot()))
                    .await;
                publisher
                    .publish_critical(EventMessage::SnapshotPositions(positions.snapshot()))
                    .await;
            }
        }
    }
}
