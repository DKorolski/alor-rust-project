use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::warn;

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::cws_client::CwsClient;
use crate::health::HealthState;
use crate::router::Router;
use crate::state::orders_manager::OrdersManager;
use crate::state::positions_manager::PositionsManager;
use crate::strategy_adapter::StrategyRunner;
use crate::ws_hub::{ConnEvent, WsEvent, WsHub};

pub struct Supervisor {
    cfg: AlorGatewayConfig,
    token_provider: TokenProvider,
    health: Arc<RwLock<HealthState>>,
}

impl Supervisor {
    pub fn new(cfg: AlorGatewayConfig) -> Self {
        let token_provider = TokenProvider::new(cfg.oauth_url.clone(), cfg.refresh_token.clone());
        let health = Arc::new(RwLock::new(HealthState::default()));
        Self {
            cfg,
            token_provider,
            health,
        }
    }

    pub fn health_state(&self) -> Arc<RwLock<HealthState>> {
        self.health.clone()
    }

    pub async fn run<S>(&self, strategy: S) -> anyhow::Result<()>
    where
        S: alor_scalping::strategy::StrategyCore + Send + 'static,
    {
        let (hub_handle, mut ws_events) = WsHub::start(self.cfg.clone(), self.token_provider.clone());
        let (raw_tx, raw_rx) = mpsc::channel(1024);
        let last_bar_instant = Arc::new(RwLock::new(Instant::now()));
        let last_bar_ts = Arc::new(RwLock::new(None::<i64>));

        tokio::spawn({
            let health = self.health.clone();
            async move {
                while let Some(event) = ws_events.recv().await {
                    match event {
                        WsEvent::Raw(value) => {
                            let _ = raw_tx.send(value).await;
                        }
                        WsEvent::Conn(conn) => {
                            let mut guard = health.write();
                            match conn {
                                ConnEvent::Connected => guard.ws_connected = true,
                                ConnEvent::Disconnected => guard.ws_connected = false,
                                ConnEvent::Reconnecting => guard.ws_connected = false,
                            }
                        }
                    }
                }
            }
        });

        let streams = Router::start(raw_rx);
        let positions_manager = PositionsManager::start(streams.positions_rx);
        let orders_manager = OrdersManager::start(streams.orders_rx);
        let cws_handle = CwsClient::start(self.cfg.clone(), self.token_provider.clone());

        let (bars_tx, bars_rx) = mpsc::channel(1024);
        tokio::spawn({
            let health = self.health.clone();
            let last_bar_instant = last_bar_instant.clone();
            let last_bar_ts = last_bar_ts.clone();
            async move {
                let mut bars_rx_inner = streams.bars_rx;
                while let Some(bar) = bars_rx_inner.recv().await {
                    {
                        let mut guard = health.write();
                        guard.last_bar_ts = bar.close_time_utc;
                    }
                    *last_bar_instant.write() = Instant::now();
                    *last_bar_ts.write() = Some(bar.close_time_utc);
                    let _ = bars_tx.send(bar).await;
                }
            }
        });

        StrategyRunner::new(
            strategy,
            positions_manager.clone(),
            orders_manager.clone(),
            cws_handle,
            self.cfg.portfolio.clone(),
            self.cfg.exchange.clone(),
        )
        .start(bars_rx);

        let silence_threshold = Duration::from_secs(self.cfg.max_silence_bars_sec);

        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            {
                let mut guard = self.health.write();
                guard.last_bar_age_sec = last_bar_instant.read().elapsed().as_secs();
            }

            if last_bar_instant.read().elapsed() > silence_threshold {
                warn!("bar silence detected; resubscribing");
                let from_ts = last_bar_ts
                    .read()
                    .map(|ts| ts - (self.cfg.tf_sec * 2));
                if let Some(from_ts) = from_ts {
                    hub_handle.resubscribe_from(from_ts).await;
                } else {
                    hub_handle.resubscribe_all().await;
                }
                *last_bar_instant.write() = Instant::now();
            }
        }
    }

}
