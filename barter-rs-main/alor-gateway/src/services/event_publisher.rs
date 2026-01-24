use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};
use tokio::sync::mpsc::error::TrySendError;
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
}

impl Default for EventPublisherConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 2048,
            publish_timeout_ms: 2000,
            retry_base_ms: 50,
            retry_max_ms: 2000,
            degraded_threshold_sec: 10,
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
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let mut guard = self.health.write();
                guard.backpressure_lagged = true;
                guard.event_queue_full_drops_total = guard.event_queue_full_drops_total.saturating_add(1);
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
            guard.backpressure_lagged = true;
            warn!(?error, "event queue closed; dropping critical event");
        }
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
    let drain_timeout = Duration::from_millis(300);
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
                guard.event_publish_retries_total = guard.event_publish_retries_total.saturating_add(1);
                guard.event_sink_degraded = true;
                warn!(?error, "event publish failed; retrying");
            }
            Err(_) => {
                let mut guard = health.write();
                guard.event_publish_timeout_total = guard.event_publish_timeout_total.saturating_add(1);
                guard.event_publish_retries_total = guard.event_publish_retries_total.saturating_add(1);
                guard.event_sink_degraded = true;
                warn!("event publish timed out; retrying");
            }
        }

        if degraded_since.is_none() {
            *degraded_since = Some(Instant::now());
        }
        if let Some(start) = degraded_since {
            if start.elapsed().as_secs() >= config.degraded_threshold_sec {
                let mut guard = health.write();
                guard.event_sink_degraded = true;
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_millis(config.retry_max_ms));
    }
}

async fn publish_event(sink: &Arc<dyn EventSink>, msg: EventMessage) -> anyhow::Result<()> {
    match msg {
        EventMessage::Bar(event) => sink.publish_bar(event).await,
        EventMessage::Order(event) => sink.publish_order(event).await,
        EventMessage::Position(event) => sink.publish_position(event).await,
        EventMessage::Health(event) => sink.publish_health(event).await,
        EventMessage::SnapshotOrders(event) => sink.publish_snapshot_orders(event).await,
        EventMessage::SnapshotPositions(event) => sink.publish_snapshot_positions(event).await,
    }
}
