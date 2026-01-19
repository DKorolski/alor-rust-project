use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use parking_lot::RwLock;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::cws_client::CwsClient;
use crate::gateway_events::{GatewayEvent, log_event};
use crate::health::{GatewayPhase, HealthState};
use crate::router::{Router, RouterCommand, RouterControl};
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
        let mut cfg = self.cfg.clone();
        if cfg.from_ts == 0 {
            cfg.from_ts = Utc::now().timestamp() - (cfg.history_days_back as i64 * 86_400);
        }
        debug!(
            from_ts = cfg.from_ts,
            history_days_back = cfg.history_days_back,
            "history backfill start configured"
        );

        let (hub_handle, mut ws_events) = WsHub::start(cfg.clone(), self.token_provider.clone());
        let (raw_tx, raw_rx) = mpsc::channel(1024);
        let last_bar_instant = Arc::new(RwLock::new(Instant::now()));
        let last_bar_ts = Arc::new(RwLock::new(HashMap::<String, i64>::new()));
        let live_symbols = Arc::new(RwLock::new(HashSet::<String>::new()));
        let (phase_tx, phase_rx) = watch::channel(GatewayPhase::SyncingHistory);

        let (router_cmd_tx, streams) = Router::start(raw_rx, cfg.tf_sec);

        tokio::spawn({
            let health = self.health.clone();
            let router_cmd_tx = router_cmd_tx.clone();
            async move {
                while let Some(event) = ws_events.recv().await {
                    match event {
                        WsEvent::Raw(value) => {
                            match raw_tx.try_send(value) {
                                Ok(()) => {
                                    let mut guard = health.write();
                                    guard.backpressure_lagged = false;
                                }
                                Err(error) => {
                                    let mut guard = health.write();
                                    guard.backpressure_lagged = true;
                                    guard.readiness = false;
                                    log_event(GatewayEvent::Lagged { duration_ms: 0 });
                                    warn!(?error, "backpressure detected: raw queue full");
                                }
                            }
                        }
                        WsEvent::Conn(conn) => {
                            let mut guard = health.write();
                            match conn {
                                ConnEvent::Connected => guard.ws_connected = true,
                                ConnEvent::Disconnected => guard.ws_connected = false,
                                ConnEvent::Reconnecting => guard.ws_connected = false,
                            }
                        }
                        WsEvent::Subscribed { wallclock_ts } => {
                            let live_cutoff_ts = wallclock_ts - (2 * cfg.tf_sec);
                            debug!(
                                from_ts = cfg.from_ts,
                                from_ts_rfc3339 = %format_ts(cfg.from_ts),
                                wallclock_ts,
                                wallclock_ts_rfc3339 = %format_ts(wallclock_ts),
                                live_cutoff_ts,
                                live_cutoff_ts_rfc3339 = %format_ts(live_cutoff_ts),
                                tf_sec = cfg.tf_sec,
                                "history backfill window computed"
                            );
                            let _ = router_cmd_tx
                                .send(RouterCommand::UpdateSubscribeWallclock(wallclock_ts))
                                .await;
                        }
                    }
                }
            }
        });

        let (positions_tx, positions_rx) = mpsc::channel(1024);
        let (orders_tx, orders_rx) = mpsc::channel(1024);
        let positions_manager = PositionsManager::start(
            positions_rx,
            cfg.log_positions_filter.clone(),
            cfg.log_cash_positions,
            cfg.cash_symbols.clone(),
        );
        let orders_manager = OrdersManager::start(
            orders_rx,
            cfg.log_existing_snapshot_orders,
        );
        let cws_handle = CwsClient::start(cfg.clone(), self.token_provider.clone());

        tokio::spawn({
            let health = self.health.clone();
            async move {
                let mut positions_rx = streams.positions_rx;
                while let Some(position) = positions_rx.recv().await {
                    {
                        let mut guard = health.write();
                        guard.last_positions_ts = position.ts_utc;
                    }
                    let _ = positions_tx.send(position).await;
                }
            }
        });

        tokio::spawn({
            let health = self.health.clone();
            async move {
                let mut orders_rx = streams.orders_rx;
                while let Some(order) = orders_rx.recv().await {
                    {
                        let mut guard = health.write();
                        guard.last_orders_ts = order.ts_utc;
                    }
                    let _ = orders_tx.send(order).await;
                }
            }
        });

        tokio::spawn({
            let mut control_rx = streams.control_rx;
            let hub_handle = hub_handle.clone();
            async move {
                while let Some(control) = control_rx.recv().await {
                    match control {
                        RouterControl::AuthError(code) => {
                            warn!(http_code = code, "forcing ws reconnect due to auth error");
                            hub_handle.reconnect().await;
                        }
                    }
                }
            }
        });

        let (bars_tx, bars_rx) = mpsc::channel(1024);
        tokio::spawn({
            let health = self.health.clone();
            let last_bar_instant = last_bar_instant.clone();
            let last_bar_ts = last_bar_ts.clone();
            let live_symbols = live_symbols.clone();
            let phase_tx = phase_tx.clone();
            let positions_manager = positions_manager.clone();
            let orders_manager = orders_manager.clone();
            let symbols_len = cfg.symbols.len();
            async move {
                let mut bars_rx_inner = streams.bars_rx;
                let mut history_min: Option<i64> = None;
                let mut history_max: Option<i64> = None;
                let mut history_count: u64 = 0;
                let mut logged_live_start = false;
                while let Some(bar) = bars_rx_inner.recv().await {
                    {
                        let mut guard = health.write();
                        guard.last_bar_ts = bar.close_time_utc;
                    }
                    *last_bar_instant.write() = Instant::now();
                    last_bar_ts
                        .write()
                        .insert(bar.symbol.clone(), bar.close_time_utc);
                    if bar.origin == crate::models::DataOrigin::History {
                        history_count += 1;
                        history_min = Some(history_min.map_or(bar.close_time_utc, |min| {
                            min.min(bar.close_time_utc)
                        }));
                        history_max = Some(history_max.map_or(bar.close_time_utc, |max| {
                            max.max(bar.close_time_utc)
                        }));
                        debug!(
                            symbol = %bar.symbol,
                            close_time_utc = bar.close_time_utc,
                            open = bar.o,
                            high = bar.h,
                            low = bar.l,
                            close = bar.c,
                            volume = bar.v,
                            "history bar"
                        );
                    } else {
                        if !logged_live_start {
                            info!(
                                history_start_ts = history_min,
                                history_end_ts = history_max,
                                history_count,
                                live_start_ts = bar.close_time_utc,
                                "history backfill complete; live stream started"
                            );
                            logged_live_start = true;
                        }
                        debug!(
                            symbol = %bar.symbol,
                            close_time_utc = bar.close_time_utc,
                            open = bar.o,
                            high = bar.h,
                            low = bar.l,
                            close = bar.c,
                            volume = bar.v,
                            "live bar"
                        );
                        live_symbols.write().insert(bar.symbol.clone());
                    }
                    let bars_live_seen = live_symbols.read().len() >= symbols_len;
                    let positions_synced = positions_manager.synced();
                    let orders_synced = orders_manager.synced();
                    let live_ready = bars_live_seen && positions_synced && orders_synced;
                    if live_ready && *phase_tx.borrow() != GatewayPhase::LiveReady {
                        info!(
                            bars_live_seen,
                            positions_synced,
                            orders_synced,
                            "gateway phase transition: LiveReady"
                        );
                        let _ = phase_tx.send(GatewayPhase::LiveReady);
                    }
                    let _ = bars_tx.send(bar).await;
                }
            }
        });

        StrategyRunner::new(
            strategy,
            positions_manager.clone(),
            orders_manager.clone(),
            cws_handle,
            cfg.portfolio.clone(),
            cfg.exchange.clone(),
            phase_rx,
            cfg.history_sessions,
            cfg.session_rollover_hour_utc,
            cfg.price_step,
            cfg.volume_step,
        )
        .start(bars_rx);

        let silence_threshold = Duration::from_secs(cfg.max_silence_bars_sec);
        let mut ticker = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = tokio::signal::ctrl_c() => {
                    info!("shutdown signal received");
                    hub_handle.shutdown().await;
                    break;
                }
            }
            {
                let mut guard = self.health.write();
                guard.last_bar_age_sec = last_bar_instant.read().elapsed().as_secs();
                guard.gateway_phase = *phase_tx.borrow();
                guard.readiness = guard.gateway_phase == GatewayPhase::LiveReady
                    && guard.ws_connected
                    && !guard.backpressure_lagged;
            }

            if last_bar_instant.read().elapsed() > silence_threshold {
                warn!("bar silence detected; resubscribing");
                log_event(GatewayEvent::ResyncStarted);
                let from_ts = last_bar_ts.read().values().min().map(|ts| ts - (cfg.tf_sec * 2));
                if let Some(from_ts) = from_ts {
                    hub_handle.resubscribe_from(from_ts).await;
                } else {
                    hub_handle.resubscribe_all().await;
                }
                *last_bar_instant.write() = Instant::now();
                log_event(GatewayEvent::ResyncDone);
            }
        }

        Ok(())
    }

}

fn format_ts(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
        .to_rfc3339()
}
