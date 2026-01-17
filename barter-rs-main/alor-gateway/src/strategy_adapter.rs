use alor_scalping::strategy::{
    Action, OrderSnapshot, PositionSnapshot, StrategyBar, StrategyContext, StrategyCore,
};
use chrono::{FixedOffset, TimeZone};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::cws_client::CwsHandle;
use crate::models::{BarEvent, OrdersSnapshot, PositionsSnapshot};
use crate::state::orders_manager::OrdersManagerHandle;
use crate::state::positions_manager::PositionsManagerHandle;

pub struct StrategyRunner<S> {
    strategy: S,
    positions: PositionsManagerHandle,
    orders: OrdersManagerHandle,
    cws: CwsHandle,
    portfolio: String,
    exchange: String,
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
    ) -> Self {
        Self {
            strategy,
            positions,
            orders,
            cws,
            portfolio,
            exchange,
        }
    }

    pub fn start(mut self, mut bars_rx: mpsc::Receiver<BarEvent>) {
        tokio::spawn(async move {
            while let Some(bar) = bars_rx.recv().await {
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
