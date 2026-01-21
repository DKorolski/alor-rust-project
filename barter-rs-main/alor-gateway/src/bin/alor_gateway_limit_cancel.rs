use std::env;

use alor_gateway::config::{
    AlorGatewayConfig, detect_config_path, log_resolved_config,
};
use alor_gateway::supervisor::Supervisor;
use alor_scalping::strategy::{
    Action, OrderSnapshot, OrdersSnapshot, StrategyBar, StrategyContext, StrategyCore,
};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Clone)]
enum OrderFlow {
    Idle,
    Submitted { submit_bar_ts: i64 },
    Acked {
        order_id: i64,
        status: String,
        status_bar_ts: i64,
    },
    CancelRequested {
        order_id: i64,
        status: String,
        status_bar_ts: i64,
        requested_bar_ts: i64,
    },
}

struct LimitCancelStrategy {
    symbol: String,
    price_step: f64,
    volume_step: f64,
    ticks_offset: f64,
    cancel_after_secs: i64,
    flow: OrderFlow,
    last_status_bar_ts: Option<i64>,
}

impl LimitCancelStrategy {
    fn new(symbol: String, price_step: f64, volume_step: f64) -> Self {
        Self {
            symbol,
            price_step,
            volume_step,
            ticks_offset: 10.0,
            cancel_after_secs: 60,
            flow: OrderFlow::Idle,
            last_status_bar_ts: None,
        }
    }

    fn desired_price(&self, close: f64) -> f64 {
        close - (self.price_step * self.ticks_offset)
    }

    fn desired_qty(&self) -> f64 {
        if self.volume_step > 0.0 {
            self.volume_step
        } else {
            1.0
        }
    }

    fn latest_order_for_symbol(&self, orders: &OrdersSnapshot) -> Option<OrderSnapshot> {
        orders
            .orders
            .values()
            .filter(|order| order.symbol == self.symbol)
            .max_by_key(|order| order.ts_utc)
            .cloned()
    }

    fn can_act_after_status(&self, bar_ts: i64) -> bool {
        self.last_status_bar_ts.map_or(true, |ts| bar_ts > ts)
    }

    fn normalize_status(status: &str) -> String {
        status.to_ascii_lowercase()
    }

    fn is_terminal_status(status: &str) -> bool {
        matches!(status, "filled" | "cancelled" | "canceled" | "rejected")
    }
}

impl StrategyCore for LimitCancelStrategy {
    fn on_bar(&mut self, bar: StrategyBar, ctx: StrategyContext) -> Vec<Action> {
        if bar.symbol != self.symbol {
            return Vec::new();
        }

        let mut actions = Vec::new();
        let bar_ts = bar.time.timestamp();
        let flow = std::mem::replace(&mut self.flow, OrderFlow::Idle);

        self.flow = match flow {
            OrderFlow::Idle => {
                if let Some(order) = self.latest_order_for_symbol(&ctx.orders) {
                    let status = Self::normalize_status(&order.status);
                    info!(
                        order_id = order.order_id,
                        status = %status,
                        "limit-cancel observed order status"
                    );
                    self.last_status_bar_ts = Some(bar_ts);
                    OrderFlow::Acked {
                        order_id: order.order_id,
                        status,
                        status_bar_ts: bar_ts,
                    }
                } else if self.can_act_after_status(bar_ts) {
                    let price = self.desired_price(bar.close);
                    let qty = self.desired_qty();
                    info!(price, qty, "limit-cancel placing order");
                    actions.push(Action::PlaceLimit {
                        symbol: self.symbol.clone(),
                        price,
                        qty,
                        side: alor_scalping::strategy::Side::Buy,
                    });
                    OrderFlow::Submitted {
                        submit_bar_ts: bar_ts,
                    }
                } else {
                    OrderFlow::Idle
                }
            }
            OrderFlow::Submitted { submit_bar_ts } => {
                if let Some(order) = self.latest_order_for_symbol(&ctx.orders) {
                    let status = Self::normalize_status(&order.status);
                    info!(
                        order_id = order.order_id,
                        status = %status,
                        "limit-cancel order acknowledged"
                    );
                    self.last_status_bar_ts = Some(bar_ts);
                    OrderFlow::Acked {
                        order_id: order.order_id,
                        status,
                        status_bar_ts: bar_ts,
                    }
                } else {
                    OrderFlow::Submitted { submit_bar_ts }
                }
            }
            OrderFlow::Acked {
                order_id,
                status,
                status_bar_ts,
            } => match ctx.orders.orders.get(&order_id) {
                Some(order) => {
                    let mut next_status = Self::normalize_status(&order.status);
                    let mut next_status_bar_ts = status_bar_ts;
                    if next_status != status {
                        info!(
                            order_id = order.order_id,
                            status = %next_status,
                            "limit-cancel order status updated"
                        );
                        next_status_bar_ts = bar_ts;
                        self.last_status_bar_ts = Some(bar_ts);
                    }

                    if Self::is_terminal_status(&next_status) {
                        if bar_ts > next_status_bar_ts {
                            OrderFlow::Idle
                        } else {
                            OrderFlow::Acked {
                                order_id,
                                status: next_status,
                                status_bar_ts: next_status_bar_ts,
                            }
                        }
                    } else if bar_ts > next_status_bar_ts
                        && bar_ts - order.ts_utc >= self.cancel_after_secs
                    {
                        info!(
                            order_id = order.order_id,
                            status = %next_status,
                            "limit-cancel cancelling order"
                        );
                        actions.push(Action::Cancel { order_id });
                        OrderFlow::CancelRequested {
                            order_id,
                            status: next_status,
                            status_bar_ts: next_status_bar_ts,
                            requested_bar_ts: bar_ts,
                        }
                    } else {
                        OrderFlow::Acked {
                            order_id,
                            status: next_status,
                            status_bar_ts: next_status_bar_ts,
                        }
                    }
                }
                None => {
                    warn!(
                        order_id,
                        "limit-cancel awaiting order status update from server"
                    );
                    OrderFlow::Acked {
                        order_id,
                        status,
                        status_bar_ts,
                    }
                }
            },
            OrderFlow::CancelRequested {
                order_id,
                status,
                status_bar_ts,
                requested_bar_ts,
            } => match ctx.orders.orders.get(&order_id) {
                Some(order) => {
                    let mut next_status = Self::normalize_status(&order.status);
                    let mut next_status_bar_ts = status_bar_ts;
                    if next_status != status {
                        info!(
                            order_id = order.order_id,
                            status = %next_status,
                            "limit-cancel order status updated"
                        );
                        next_status_bar_ts = bar_ts;
                        self.last_status_bar_ts = Some(bar_ts);
                    }

                    if Self::is_terminal_status(&next_status) && bar_ts > next_status_bar_ts {
                        OrderFlow::Idle
                    } else {
                        OrderFlow::CancelRequested {
                            order_id,
                            status: next_status,
                            status_bar_ts: next_status_bar_ts,
                            requested_bar_ts,
                        }
                    }
                }
                None => {
                    warn!(
                        order_id,
                        "limit-cancel awaiting cancel confirmation from server"
                    );
                    OrderFlow::CancelRequested {
                        order_id,
                        status,
                        status_bar_ts,
                        requested_bar_ts,
                    }
                }
            },
        };

        actions
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let args: Vec<String> = env::args().collect();
    let config_path = detect_config_path(&args);
    let data_report_path = detect_data_report_path(&args)
        .or_else(|| env::var("DATA_REPORT_PATH").ok());
    let resolved = if let Some(path) = config_path.clone() {
        AlorGatewayConfig::from_file_with_sources(path)?
    } else {
        AlorGatewayConfig::from_env_with_sources()?
    };
    log_resolved_config(&resolved, config_path.as_deref());
    let mut cfg = resolved.config.clone();
    cfg.from_ts = resolved.derived.computed_from_ts;
    cfg.skip_history_bars = resolved.derived.skip_history_effective;
    cfg.data_report_path = data_report_path.clone();
    let supervisor = Supervisor::new(cfg.clone());
    let health_state = supervisor.health_state();
    let health_addr = cfg.health_listen_addr.clone();
    tokio::spawn(async move {
        if let Err(error) = alor_gateway::health_server::serve(health_state, health_addr).await {
            warn!(?error, "health server stopped");
        }
    });

    let symbol = cfg
        .symbols
        .first()
        .cloned()
        .unwrap_or_else(|| "IMOEXF".to_string());
    let strategy = LimitCancelStrategy::new(symbol, cfg.price_step, cfg.volume_step);

    info!("starting alor gateway limit-cancel runner");
    if let Err(error) = supervisor.run(strategy).await {
        warn!(?error, "gateway stopped with error");
    }

    Ok(())
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_ansi(cfg!(debug_assertions))
        .init();
}

fn detect_data_report_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--data-report" {
            return iter.next().cloned();
        }
        if let Some(path) = arg.strip_prefix("--data-report=") {
            return Some(path.to_string());
        }
    }
    None
}
