use alor_types::{Action, OrderSnapshot, StrategyBar, StrategyContext, StrategyCore};
use std::collections::VecDeque;

use chrono::{Datelike, FixedOffset, TimeZone, Utc};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::cws_client::CwsHandle;
use crate::health::GatewayPhase;
use crate::models::{BarEvent, DataOrigin, OrdersSnapshot, PositionsSnapshot};
use crate::state::orders_manager::OrdersManagerHandle;
use crate::state::positions_manager::PositionsManagerHandle;

pub struct StrategyRunner<S> {
    strategy: S,
    positions: PositionsManagerHandle,
    orders: OrdersManagerHandle,
    cws: CwsHandle,
    portfolio: String,
    exchange: String,
    phase_rx: watch::Receiver<GatewayPhase>,
    history_sessions: u8,
    session_rollover_hour_utc: u8,
    price_step: f64,
    volume_step: f64,
}

impl<S> StrategyRunner<S>
where
    S: StrategyCore + Send + 'static,
{
    pub fn new(
        strategy: S,
        positions: PositionsManagerHandle,
        orders: OrdersManagerHandle,
        cws: CwsHandle,
        portfolio: String,
        exchange: String,
        phase_rx: watch::Receiver<GatewayPhase>,
        history_sessions: u8,
        session_rollover_hour_utc: u8,
        price_step: f64,
        volume_step: f64,
    ) -> Self {
        Self {
            strategy,
            positions,
            orders,
            cws,
            portfolio,
            exchange,
            phase_rx,
            history_sessions,
            session_rollover_hour_utc,
            price_step,
            volume_step,
        }
    }

    pub fn start(mut self, mut bars_rx: mpsc::Receiver<BarEvent>) {
        tokio::spawn(async move {
            let mut history_buffer: VecDeque<BarEvent> = VecDeque::new();
            let mut session_ids: VecDeque<i32> = VecDeque::new();
            let mut last_phase = *self.phase_rx.borrow();
            while let Some(bar) = bars_rx.recv().await {
                let phase = *self.phase_rx.borrow();
                if phase != last_phase {
                    if phase == GatewayPhase::LiveReady
                        && session_ids.len() < self.history_sessions as usize
                    {
                        warn!(
                            expected_sessions = self.history_sessions,
                            loaded_sessions = session_ids.len(),
                            "history sessions fewer than expected"
                        );
                    }
                    last_phase = phase;
                }

                if matches!(bar.origin, DataOrigin::History | DataOrigin::HistoryGap) {
                    let session_key =
                        session_id(bar.close_time_utc, self.session_rollover_hour_utc);
                    if !session_ids.contains(&session_key) {
                        session_ids.push_back(session_key);
                        while session_ids.len() > self.history_sessions as usize {
                            if let Some(removed) = session_ids.pop_front() {
                                history_buffer.retain(|b| {
                                    session_id(b.close_time_utc, self.session_rollover_hour_utc)
                                        != removed
                                });
                            }
                        }
                    }
                    history_buffer.push_back(bar.clone());
                }

                let should_trade =
                    phase == GatewayPhase::LiveReady && bar.origin == DataOrigin::Live;
                if !should_trade {
                    continue;
                }

                let ctx = StrategyContext {
                    positions: map_positions(self.positions.snapshot()),
                    orders: map_orders(self.orders.snapshot()),
                };
                let bar = map_bar(bar);
                let actions = self.strategy.on_bar(bar, ctx);
                for action in actions {
                    if let Err(error) = execute_action(
                        &self.cws,
                        &self.portfolio,
                        &self.exchange,
                        self.price_step,
                        self.volume_step,
                        action,
                    )
                    .await
                    {
                        warn!(?error, "strategy action failed");
                    }
                }
            }
        });
    }
}

fn map_bar(bar: BarEvent) -> StrategyBar {
    let tz = FixedOffset::east_opt(0).unwrap();
    let time = tz
        .timestamp_opt(bar.close_time_utc, 0)
        .single()
        .expect("valid bar timestamp");

    StrategyBar {
        time,
        open: bar.o,
        high: bar.h,
        low: bar.l,
        close: bar.c,
        volume: bar.v,
        symbol: bar.symbol,
    }
}

fn map_positions(snapshot: PositionsSnapshot) -> alor_types::PositionsSnapshot {
    snapshot
}

fn map_orders(snapshot: OrdersSnapshot) -> alor_types::OrdersSnapshot {
    let orders = snapshot
        .orders
        .into_iter()
        .map(|(order_id, event)| {
            (
                order_id,
                OrderSnapshot {
                    order_id: event.order_id,
                    symbol: event.symbol,
                    status: event.status,
                    filled: event.filled,
                    price: event.price,
                    ts_utc: event.ts_utc,
                },
            )
        })
        .collect();
    alor_types::OrdersSnapshot { orders }
}

async fn execute_action(
    cws: &CwsHandle,
    portfolio: &str,
    exchange: &str,
    price_step: f64,
    volume_step: f64,
    action: Action,
) -> anyhow::Result<()> {
    match action {
        Action::PlaceLimit {
            symbol,
            price,
            qty,
            side,
        } => {
            info!(?symbol, price, qty, ?side, "strategy place limit");
            let price = normalize_step(price, price_step);
            let qty = normalize_step(qty, volume_step);
            let _ = cws
                .create_limit(portfolio, exchange, &symbol, price, qty, side.as_str())
                .await?;
        }
        Action::Cancel { order_id } => {
            info!(order_id, "strategy cancel order");
            let _ = cws.cancel(portfolio, exchange, order_id).await?;
        }
        Action::Replace {
            order_id,
            new_price,
            new_qty,
        } => {
            info!(order_id, new_price, new_qty, "strategy replace order");
            let new_price = normalize_step(new_price, price_step);
            let new_qty = normalize_step(new_qty, volume_step);
            let _ = cws
                .replace(
                    portfolio, exchange, None, None, order_id, new_price, new_qty,
                )
                .await?;
        }
        Action::Noop => {}
    }

    Ok(())
}

trait SideAsStr {
    fn as_str(&self) -> &'static str;
}

impl SideAsStr for alor_types::Side {
    fn as_str(&self) -> &'static str {
        match self {
            alor_types::Side::Buy => "buy",
            alor_types::Side::Sell => "sell",
        }
    }
}

fn session_id(close_time_utc: i64, rollover_hour_utc: u8) -> i32 {
    let shifted = close_time_utc - (rollover_hour_utc as i64 * 3600);
    let date = Utc
        .timestamp_opt(shifted, 0)
        .single()
        .expect("valid timestamp");
    date.date_naive().num_days_from_ce()
}

fn normalize_step(value: f64, step: f64) -> f64 {
    if step <= 0.0 {
        value
    } else {
        (value / step).round() * step
    }
}
