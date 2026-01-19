use tracing::{info, warn};

use crate::health::ResyncMode;

#[derive(Debug)]
pub enum GatewayEvent {
    Connected,
    Reconnecting { attempt: u64 },
    Subscribed { symbol: String, subscription_type: String },
    AckTimeout { symbol: String, subscription_type: String },
    Lagged { duration_ms: u64 },
    ResyncStarted { mode: ResyncMode, reason: &'static str },
    ResyncDone {
        mode: ResyncMode,
        gap_sec: u64,
        bars_backfilled: u64,
    },
}

pub fn log_event(event: GatewayEvent) {
    match event {
        GatewayEvent::Connected => info!("Connected to the WebSocket."),
        GatewayEvent::Reconnecting { attempt } => info!("Reconnecting, attempt {}", attempt),
        GatewayEvent::Subscribed {
            symbol,
            subscription_type,
        } => info!("Subscribed to {} ({})", symbol, subscription_type),
        GatewayEvent::AckTimeout {
            symbol,
            subscription_type,
        } => warn!("AckTimeout for {} ({})", symbol, subscription_type),
        GatewayEvent::Lagged { duration_ms } => {
            warn!("Lagged detected: {} ms", duration_ms)
        }
        GatewayEvent::ResyncStarted { mode, reason } => {
            info!(?mode, reason, "Resync started")
        }
        GatewayEvent::ResyncDone {
            mode,
            gap_sec,
            bars_backfilled,
        } => {
            info!(
                ?mode,
                gap_sec,
                bars_backfilled,
                "Resync completed"
            )
        }
    }
}
