use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};
use tracing::warn;

use crate::health::HealthState;
use crate::transport::{EventMessage, EventSink};

#[derive(Debug, Clone)]
pub struct EventPublisherConfig {
    pub queue_capacity: usize,
    pub publish_timeout_ms: u64,
    pub retry_base_ms: u64,
    pub retry_max_ms: u64,
    pub degraded_threshold_sec: u64,
    pub drain_timeout_ms: u64,
}

impl Default for EventPublisherConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 2048,
            publish_timeout_ms: 2000,
            retry_base_ms: 50,
            retry_max_ms: 2000,
            degraded_threshold_sec: 10,
            drain_timeout_ms: 300,
        }
    }
}

#[derive(Clone)]
pub struct EventPublisherHandle {
    tx: mpsc::Sender<EventMessage>,
    health: Arc<parking_lot::RwLock<HealthState>>,
}

impl EventPublisherHandle {
    pub fn publish_lossy(&self, event: EventMessage) {
        match self.tx.try_send(event) {
            Ok(()) => {
                self.health.write().event_backpressure_lagged = false;
            }
            Err(TrySendError::Full(_)) => {
                let mut guard = self.health.write();
                guard.event_backpressure_lagged = true;
                guard.event_queue_full_drops_total =
                    guard.event_queue_full_drops_total.saturating_add(1);
                warn!("event queue full; dropping lossy event");
            }
            Err(TrySendError::Closed(_)) => {
                warn!("event queue closed; dropping lossy event");
            }
        }
    }

    pub async fn publish_critical(&self, event: EventMessage) {
        if let Err(error) = self.tx.send(event).await {
            let mut guard = self.health.write();
            guard.event_backpressure_lagged = true;
            warn!(?error, "event queue closed; dropping critical event");
            return;
        }
        self.health.write().event_backpressure_lagged = false;
    }
}

pub fn start_event_publisher(
    sink: Arc<dyn EventSink>,
    health: Arc<parking_lot::RwLock<HealthState>>,
    shutdown_rx: watch::Receiver<bool>,
    config: EventPublisherConfig,
) -> (EventPublisherHandle, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(config.queue_capacity);
    let handle = EventPublisherHandle {
        tx,
        health: health.clone(),
    };
    let join_handle = tokio::spawn(async move {
        run_event_publisher(rx, sink, health, shutdown_rx, config).await;
    });
    (handle, join_handle)
}

async fn run_event_publisher(
    mut rx: mpsc::Receiver<EventMessage>,
    sink: Arc<dyn EventSink>,
    health: Arc<parking_lot::RwLock<HealthState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    config: EventPublisherConfig,
) {
    let drain_timeout = Duration::from_millis(config.drain_timeout_ms);
    let mut degraded_since: Option<Instant> = None;
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    let deadline = Instant::now() + drain_timeout;
                    while Instant::now() < deadline {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        match tokio::time::timeout(remaining.min(Duration::from_millis(50)), rx.recv()).await {
                            Ok(Some(msg)) => {
                                publish_with_retry(&sink, &health, &config, msg, &mut degraded_since).await;
                            }
                            _ => break,
                        }
                    }
                    break;
                }
            }
            msg = rx.recv() => {
                let Some(msg) = msg else {
                    break;
                };
                publish_with_retry(&sink, &health, &config, msg, &mut degraded_since).await;
            }
        }
    }
}

async fn publish_with_retry(
    sink: &Arc<dyn EventSink>,
    health: &Arc<parking_lot::RwLock<HealthState>>,
    config: &EventPublisherConfig,
    msg: EventMessage,
    degraded_since: &mut Option<Instant>,
) {
    let mut backoff = Duration::from_millis(config.retry_base_ms);
    loop {
        let result = tokio::time::timeout(
            Duration::from_millis(config.publish_timeout_ms),
            publish_event(sink, msg.clone()),
        )
        .await;
        match result {
            Ok(Ok(())) => {
                let mut guard = health.write();
                guard.event_sink_degraded = false;
                guard.last_event_publish_ts = chrono::Utc::now().timestamp();
                *degraded_since = None;
                return;
            }
            Ok(Err(error)) => {
                let mut guard = health.write();
                guard.event_publish_fail_total = guard.event_publish_fail_total.saturating_add(1);
                guard.event_publish_retries_total =
                    guard.event_publish_retries_total.saturating_add(1);
                guard.event_sink_degraded = true;
                warn!(?error, "event publish failed; retrying");
            }
            Err(_) => {
                let mut guard = health.write();
                guard.event_publish_timeout_total =
                    guard.event_publish_timeout_total.saturating_add(1);
                guard.event_publish_retries_total =
                    guard.event_publish_retries_total.saturating_add(1);
                guard.event_sink_degraded = true;
                warn!("event publish timed out; retrying");
            }
        }

        if degraded_since.is_none() {
            *degraded_since = Some(Instant::now());
        }
        if degraded_since
            .as_ref()
            .is_some_and(|start| start.elapsed().as_secs() >= config.degraded_threshold_sec)
        {
            let mut guard = health.write();
            guard.event_sink_degraded = true;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_millis(config.retry_max_ms));
    }
}

async fn publish_event(sink: &Arc<dyn EventSink>, msg: EventMessage) -> anyhow::Result<()> {
    match msg {
        EventMessage::Bar(event) => sink.publish_bar(event).await,
        EventMessage::Order(event) => sink.publish_order(event).await,
        EventMessage::StopOrder(event) => sink.publish_stop_order(event).await,
        EventMessage::Trade(event) => sink.publish_trade(event).await,
        EventMessage::Position(event) => sink.publish_position(event).await,
        EventMessage::Health(event) => sink.publish_health(event).await,
        EventMessage::SnapshotOrders(event) => sink.publish_snapshot_orders(event).await,
        EventMessage::SnapshotStopOrders(event) => sink.publish_snapshot_stop_orders(event).await,
        EventMessage::SnapshotPositions(event) => sink.publish_snapshot_positions(event).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::watch;

    struct TestSink {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EventSink for TestSink {
        async fn publish_bar(&self, _event: crate::models::BarEvent) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_order(&self, _event: crate::models::OrderEvent) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_stop_order(
            &self,
            _event: crate::models::StopOrderEvent,
        ) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_trade(&self, _event: crate::models::TradeEvent) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_position(
            &self,
            _event: crate::models::PositionEvent,
        ) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_health(&self, _health: crate::health::HealthState) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_snapshot_orders(
            &self,
            _snapshot: crate::models::OrdersSnapshot,
        ) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_snapshot_stop_orders(
            &self,
            _snapshot: crate::models::StopOrdersSnapshot,
        ) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
        async fn publish_snapshot_positions(
            &self,
            _snapshot: crate::models::PositionsSnapshot,
        ) -> anyhow::Result<()> {
            self.fail_once_then_ok()
        }
    }

    impl TestSink {
        fn fail_once_then_ok(&self) -> anyhow::Result<()> {
            let count = self.calls.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                Err(anyhow::anyhow!("fail once"))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn lossy_drops_when_queue_full() {
        let health = Arc::new(parking_lot::RwLock::new(HealthState::default()));
        let (tx, _rx) = mpsc::channel(1);
        let publisher = EventPublisherHandle {
            tx,
            health: health.clone(),
        };
        publisher.publish_lossy(EventMessage::Health(HealthState::default()));
        publisher.publish_lossy(EventMessage::Health(HealthState::default()));
        let guard = health.read();
        assert!(guard.event_queue_full_drops_total >= 1);
    }

    #[tokio::test]
    async fn critical_blocks_when_queue_full() {
        let health = Arc::new(parking_lot::RwLock::new(HealthState::default()));
        let (tx, _rx) = mpsc::channel(1);
        let publisher = EventPublisherHandle { tx, health };
        publisher
            .publish_critical(EventMessage::Health(HealthState::default()))
            .await;
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            publisher.publish_critical(EventMessage::Health(HealthState::default())),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn event_sink_degraded_on_failure() {
        let sink = Arc::new(TestSink {
            calls: AtomicUsize::new(0),
        });
        let health = Arc::new(parking_lot::RwLock::new(HealthState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let config = EventPublisherConfig {
            queue_capacity: 4,
            publish_timeout_ms: 200,
            retry_base_ms: 200,
            retry_max_ms: 200,
            degraded_threshold_sec: 1,
            drain_timeout_ms: 100,
        };
        let (publisher, _join_handle) =
            start_event_publisher(sink, health.clone(), shutdown_rx, config);
        publisher
            .publish_critical(EventMessage::Health(HealthState::default()))
            .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(health.read().event_sink_degraded);
        let _ = shutdown_tx.send(true);
    }
}
