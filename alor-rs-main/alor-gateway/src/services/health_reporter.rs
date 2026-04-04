use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::health::HealthState;
use crate::services::event_publisher::EventPublisherHandle;
use crate::state::orders_manager::OrdersManagerHandle;
use crate::state::positions_manager::PositionsManagerHandle;
use crate::state::stop_orders_manager::StopOrdersManagerHandle;
use crate::transport::EventMessage;

pub struct HealthReporterDeps {
    pub publisher: EventPublisherHandle,
    pub health: Arc<parking_lot::RwLock<HealthState>>,
    pub orders: OrdersManagerHandle,
    pub stop_orders: StopOrdersManagerHandle,
    pub positions: PositionsManagerHandle,
}

pub struct HealthReporterSchedule {
    pub health_interval: Duration,
    pub snapshot_interval: Duration,
}

pub async fn run_health_reporter(
    deps: HealthReporterDeps,
    mut shutdown_rx: watch::Receiver<bool>,
    schedule: HealthReporterSchedule,
) {
    let HealthReporterDeps {
        publisher,
        health,
        orders,
        stop_orders,
        positions,
    } = deps;
    let mut health_tick = tokio::time::interval(schedule.health_interval);
    let mut snapshot_tick = tokio::time::interval(schedule.snapshot_interval);
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
                    .publish_critical(EventMessage::SnapshotStopOrders(stop_orders.snapshot()))
                    .await;
                publisher
                    .publish_critical(EventMessage::SnapshotPositions(positions.snapshot()))
                    .await;
            }
        }
    }
}
