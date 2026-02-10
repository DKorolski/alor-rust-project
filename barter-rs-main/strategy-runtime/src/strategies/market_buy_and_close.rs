use alor_protocol::{CommandAck, Side};

use crate::live_guard::GatewayPhase;
use crate::state::StrategyState;
use crate::{BarEvent, CloseTrigger, Intent, PositionEvent, Strategy, StrategyCtx, TradeMode};

#[derive(Debug, Clone)]
pub struct MarketBuyAndCloseConfig {
    pub symbol: String,
    pub qty: f64,
    pub side: Side,
    pub close_trigger: CloseTrigger,
}

#[derive(Debug)]
pub struct MarketBuyAndCloseStrategy {
    pub config: MarketBuyAndCloseConfig,
    pub state: StrategyState,
    pub last_processed_bar_ts: Option<i64>,
}

impl MarketBuyAndCloseStrategy {
    pub fn new(config: MarketBuyAndCloseConfig) -> Self {
        Self {
            config,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
        }
    }

    fn effective_close_trigger(&self, ctx: &StrategyCtx) -> CloseTrigger {
        match ctx.trade_mode {
            TradeMode::Live => self.config.close_trigger,
            TradeMode::Paper | TradeMode::Backtest => CloseTrigger::NextBar,
        }
    }

    fn build_market_intent(&self, side: Side, fill_price: Option<f64>) -> Intent {
        Intent::Market {
            qty: self.config.qty,
            side,
            fill_price,
        }
    }

    fn close_side(&self) -> Side {
        match self.config.side {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }

    fn should_block_live(&self, ctx: &StrategyCtx, bar: &BarEvent) -> bool {
        ctx.trade_mode == TradeMode::Live
            && (bar.origin != crate::DataOrigin::Live
                || !ctx.allow_live_orders
                || ctx.gateway_phase != GatewayPhase::LiveReady)
    }

    fn update_baseline_pre_buy(state: &mut StrategyState, qty: f64) {
        match state {
            StrategyState::MarketBuyPending { baseline_qty, .. } => {
                if baseline_qty.is_none() {
                    *baseline_qty = Some(qty);
                }
            }
            _ => {}
        }
    }
}

impl Strategy for MarketBuyAndCloseStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if self.should_block_live(ctx, bar) {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        }
        if bar.symbol != self.config.symbol {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        }
        if self
            .last_processed_bar_ts
            .is_some_and(|last_ts| bar.close_time_utc <= last_ts)
        {
            return Vec::new();
        }

        let mut intent = None;
        let close_side = self.close_side();
        let qty = self.config.qty;
        match &mut self.state {
            StrategyState::Idle => {
                let close_trigger = self.effective_close_trigger(ctx);
                let baseline_qty = ctx.position_qty;
                if ctx.trade_mode == TradeMode::Live {
                    self.state = StrategyState::MarketBuySent {
                        buy_request_id: crate::deterministic_request_id(
                            &ctx.strategy_id,
                            &ctx.portfolio,
                            &bar.symbol,
                            "market_buy",
                            bar.close_time_utc,
                            0,
                        ),
                        baseline_qty,
                        close_trigger,
                        buy_bar_ts: bar.close_time_utc,
                        last_bar_ts: bar.close_time_utc,
                    };
                    intent = Some(self.build_market_intent(self.config.side, None));
                } else {
                    self.state = StrategyState::MarketBuyPending {
                        buy_request_id: crate::deterministic_request_id(
                            &ctx.strategy_id,
                            &ctx.portfolio,
                            &bar.symbol,
                            "market_buy",
                            bar.close_time_utc,
                            0,
                        ),
                        baseline_qty,
                        close_trigger,
                        pending_bar_ts: bar.close_time_utc,
                        last_bar_ts: bar.close_time_utc,
                    };
                }
            }
            StrategyState::MarketBuyPending {
                buy_request_id,
                close_trigger,
                pending_bar_ts,
                last_bar_ts,
                baseline_qty,
            } => {
                *last_bar_ts = bar.close_time_utc;
                if bar.close_time_utc > *pending_bar_ts {
                    self.state = StrategyState::MarketBuySent {
                        buy_request_id: *buy_request_id,
                        baseline_qty: *baseline_qty,
                        close_trigger: *close_trigger,
                        buy_bar_ts: bar.close_time_utc,
                        last_bar_ts: bar.close_time_utc,
                    };
                    intent = Some(self.build_market_intent(self.config.side, Some(bar.o)));
                }
            }
            StrategyState::MarketBuySent {
                close_trigger,
                buy_bar_ts,
                baseline_qty,
                last_bar_ts,
                ..
            } => {
                *last_bar_ts = bar.close_time_utc;
                if *close_trigger == CloseTrigger::NextBar && bar.close_time_utc > *buy_bar_ts {
                    let fill_price = match ctx.trade_mode {
                        TradeMode::Live => None,
                        TradeMode::Paper | TradeMode::Backtest => Some(bar.o),
                    };
                    intent = Some(Intent::Market {
                        qty,
                        side: close_side,
                        fill_price,
                    });
                    self.state = StrategyState::MarketCloseSent {
                        close_request_id: crate::deterministic_request_id(
                            &ctx.strategy_id,
                            &ctx.portfolio,
                            &bar.symbol,
                            "market_close",
                            bar.close_time_utc,
                            1,
                        ),
                        baseline_qty: *baseline_qty,
                        last_bar_ts: bar.close_time_utc,
                    };
                }
            }
            StrategyState::MarketCloseSent { last_bar_ts, .. } => {
                *last_bar_ts = bar.close_time_utc;
            }
            StrategyState::Placed { last_bar_ts, .. }
            | StrategyState::CancelSent { last_bar_ts, .. }
            | StrategyState::Done { last_bar_ts } => {
                *last_bar_ts = bar.close_time_utc;
            }
        }

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intent.into_iter().collect()
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &CommandAck) -> Vec<Intent> {
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _event: &crate::OrderEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }
        let close_side = self.close_side();
        let qty = self.config.qty;
        Self::update_baseline_pre_buy(&mut self.state, pos.qty);

        let mut intent = None;
        match &mut self.state {
            StrategyState::MarketBuySent {
                close_trigger,
                buy_bar_ts,
                baseline_qty,
                last_bar_ts,
                ..
            } => {
                *last_bar_ts = ctx.last_bar_ts().unwrap_or(*buy_bar_ts);
                if *close_trigger == CloseTrigger::PositionUpdate {
                    if baseline_qty.is_none() {
                        *baseline_qty = Some(pos.qty);
                        return Vec::new();
                    }
                    if let Some(baseline) = baseline_qty {
                        if (pos.qty - *baseline).abs() > f64::EPSILON {
                            intent = Some(Intent::Market {
                                qty,
                                side: close_side,
                                fill_price: None,
                            });
                            self.state = StrategyState::MarketCloseSent {
                                close_request_id: crate::deterministic_request_id(
                                    &ctx.strategy_id,
                                    &ctx.portfolio,
                                    &pos.symbol,
                                    "market_close",
                                    ctx.last_bar_ts().unwrap_or(*buy_bar_ts),
                                    1,
                                ),
                                baseline_qty: Some(*baseline),
                                last_bar_ts: ctx.last_bar_ts().unwrap_or(*buy_bar_ts),
                            };
                        }
                    }
                }
            }
            StrategyState::MarketCloseSent {
                baseline_qty,
                last_bar_ts,
                ..
            } => {
                *last_bar_ts = ctx.last_bar_ts().unwrap_or(*last_bar_ts);
                if let Some(baseline) = baseline_qty {
                    if (pos.qty - *baseline).abs() <= f64::EPSILON {
                        self.state = StrategyState::Done {
                            last_bar_ts: ctx.last_bar_ts().unwrap_or(*last_bar_ts),
                        };
                    }
                }
            }
            _ => {}
        }

        intent.into_iter().collect()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataOrigin, StrategyCtx, TradeMode};

    fn config() -> MarketBuyAndCloseConfig {
        MarketBuyAndCloseConfig {
            symbol: "SBER".to_string(),
            qty: 1.0,
            side: Side::Buy,
            close_trigger: CloseTrigger::NextBar,
        }
    }

    fn ctx(trade_mode: TradeMode) -> StrategyCtx {
        StrategyCtx {
            strategy_id: "strat".to_string(),
            portfolio: "demo".to_string(),
            exchange: "alor".to_string(),
            symbol: "SBER".to_string(),
            tick_size: 0.01,
            trade_mode,
            allow_live_orders: true,
            gateway_phase: GatewayPhase::LiveReady,
            position_qty: Some(0.0),
            last_bar_ts: None,
        }
    }

    #[test]
    fn live_blocks_without_live_origin() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let mut ctx = ctx(TradeMode::Live);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 1,
            close: 10.0,
            o: 9.0,
            h: 10.0,
            l: 9.0,
            v: 1.0,
            origin: DataOrigin::History,
        };
        ctx.gateway_phase = GatewayPhase::LiveReady;
        let intents = strategy.on_bar(&ctx, &bar);
        assert!(intents.is_empty());
    }

    #[test]
    fn paper_market_buy_then_close_next_bar() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let ctx = ctx(TradeMode::Paper);
        let bar1 = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 10,
            close: 10.0,
            o: 9.5,
            h: 10.0,
            l: 9.5,
            v: 1.0,
            origin: DataOrigin::History,
        };
        let bar2 = BarEvent { close_time_utc: 20, o: 10.5, close: 11.0, ..bar1.clone() };
        let bar3 = BarEvent { close_time_utc: 30, o: 11.5, close: 12.0, ..bar1.clone() };

        assert!(strategy.on_bar(&ctx, &bar1).is_empty());
        let intents = strategy.on_bar(&ctx, &bar2);
        assert_eq!(intents.len(), 1);
        let intents = strategy.on_bar(&ctx, &bar3);
        assert_eq!(intents.len(), 1);
        assert!(matches!(strategy.state(), StrategyState::MarketCloseSent { .. }));

        let position_closed = PositionEvent {
            symbol: "SBER".to_string(),
            qty: 0.0,
            existing: false,
            avg_price: 0.0,
            ts_utc: 31,
        };
        assert!(strategy.on_position(&ctx, &position_closed).is_empty());
        assert!(matches!(strategy.state(), StrategyState::Done { .. }));
    }
}
