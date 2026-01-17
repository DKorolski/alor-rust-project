use alor_scalping::strategy::{
    Action, OrderSnapshot, PositionSnapshot, StrategyBar, StrategyContext, StrategyCore,
};
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

                if bar.origin == DataOrigin::History {
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

                let ctx = StrategyContext {
                    positions: map_positions(self.positions.snapshot()),
                    orders: map_orders(self.orders.snapshot()),
                };
                let should_trade = phase == GatewayPhase::LiveReady && bar.origin == DataOrigin::Live;
                let bar = map_bar(bar);
                let actions = self.strategy.on_bar(bar, ctx);
                if should_trade {
                    for action in actions {
                        if let Err(error) = execute_action(
                            &self.cws,
                            &self.portfolio,
                            &self.exchange,
                            action,
                        )
                        .await
                        {
                            warn!(?error, "strategy action failed");
                        }
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

fn map_positions(snapshot: PositionsSnapshot) -> alor_scalping::strategy::PositionsSnapshot {
    let positions = snapshot
        .positions
        .into_iter()
        .map(|(symbol, event)| {
            (
                symbol,
                PositionSnapshot {
                    symbol: event.symbol,
                    qty: event.qty,
                    avg_price: event.avg_price,
                    ts_utc: event.ts_utc,
                },
            )
        })
        .collect();
    alor_scalping::strategy::PositionsSnapshot { positions }
}

fn map_orders(snapshot: OrdersSnapshot) -> alor_scalping::strategy::OrdersSnapshot {
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
    alor_scalping::strategy::OrdersSnapshot { orders }
}

async fn execute_action(
    cws: &CwsHandle,
    portfolio: &str,
    exchange: &str,
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
            let _ = cws
                .create_limit(portfolio, exchange, &symbol, price, qty, side.as_str())
                .await?;
        }
        Action::Cancel { order_id } => {
            info!(order_id, "strategy cancel order");
            let _ = cws.cancel(order_id).await?;
        }
        Action::Replace {
            order_id,
            new_price,
            new_qty,
        } => {
            info!(order_id, new_price, new_qty, "strategy replace order");
            let _ = cws.replace(order_id, new_price, new_qty).await?;
        }
        Action::Noop => {}
    }

    Ok(())
}

trait SideAsStr {
    fn as_str(&self) -> &'static str;
}

impl SideAsStr for alor_scalping::strategy::Side {
    fn as_str(&self) -> &'static str {
        match self {
            alor_scalping::strategy::Side::Buy => "buy",
            alor_scalping::strategy::Side::Sell => "sell",
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
