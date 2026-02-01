use alor_protocol::{AckStatus, CommandAck, Side};

use crate::state::StrategyState;
use crate::{BarEvent, Intent, OrderEvent, Strategy, StrategyCtx};

#[derive(Debug, Clone)]
pub struct LimitCancelConfig {
    pub symbol: String,
    pub tick_size: f64,
    pub offset_ticks: i64,
    pub qty: f64,
    pub side: Side,
    pub max_wait_bars_for_ack: u32,
}

#[derive(Debug)]
pub struct LimitCancelStrategy {
    pub config: LimitCancelConfig,
    pub state: StrategyState,
    pub last_processed_bar_ts: Option<i64>,
}

impl LimitCancelStrategy {
    pub fn new(config: LimitCancelConfig) -> Self {
        Self {
            config,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
        }
    }

    fn build_place_intent(&self, bar: &BarEvent) -> Intent {
        let price = match self.config.side {
            Side::Buy => bar.close - (self.config.offset_ticks as f64) * self.config.tick_size,
            Side::Sell => bar.close + (self.config.offset_ticks as f64) * self.config.tick_size,
        };
        Intent::Place {
            price,
            qty: self.config.qty,
            side: self.config.side,
        }
    }

    fn build_cancel_intent(&self, order_id: i64) -> Intent {
        Intent::Cancel { order_id }
    }
}

impl Strategy for LimitCancelStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if self
            .last_processed_bar_ts
            .is_some_and(|last_ts| bar.close_time_utc <= last_ts)
        {
            return Vec::new();
        }

        let intent = match &mut self.state {
            StrategyState::Idle => {
                self.state = StrategyState::Placed {
                    place_request_id: crate::deterministic_request_id(
                        &ctx.strategy_id,
                        &ctx.portfolio,
                        &bar.symbol,
                        "place",
                        bar.close_time_utc,
                        0,
                    ),
                    order_id: None,
                    cancel_due: false,
                    cancel_bar_ts: None,
                    placed_bar_ts: bar.close_time_utc,
                    last_bar_ts: bar.close_time_utc,
                    bars_waited: 0,
                };
                Some(self.build_place_intent(bar))
            }
            StrategyState::Placed {
                order_id,
                cancel_due,
                cancel_bar_ts,
                last_bar_ts,
                bars_waited,
                ..
            } => {
                *last_bar_ts = bar.close_time_utc;
                *bars_waited += 1;
                if *bars_waited > self.config.max_wait_bars_for_ack {
                    self.state = StrategyState::Done {
                        last_bar_ts: bar.close_time_utc,
                    };
                    None
                } else {
                    *cancel_due = true;
                    *cancel_bar_ts = Some(bar.close_time_utc);
                    let existing_order_id = *order_id;
                    let cancel_intent = existing_order_id.map(|order_id| {
                        let intent = self.build_cancel_intent(order_id);
                        self.state = StrategyState::CancelSent {
                            cancel_request_id: crate::deterministic_request_id(
                                &ctx.strategy_id,
                                &ctx.portfolio,
                                &bar.symbol,
                                "cancel",
                                bar.close_time_utc,
                                1,
                            ),
                            order_id,
                            last_bar_ts: bar.close_time_utc,
                        };
                        intent
                    });
                    cancel_intent
                }
            }
            StrategyState::CancelSent { last_bar_ts, .. } => {
                *last_bar_ts = bar.close_time_utc;
                None
            }
            StrategyState::Done { last_bar_ts } => {
                *last_bar_ts = bar.close_time_utc;
                None
            }
        };

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intent.into_iter().collect()
    }

    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        let symbol = ctx.symbol.as_str();
        match &mut self.state {
            StrategyState::Placed {
                place_request_id,
                order_id,
                cancel_due,
                cancel_bar_ts,
                ..
            } => {
                if *place_request_id != ack.request_id {
                    return Vec::new();
                }
                match ack.status {
                    AckStatus::Confirmed | AckStatus::Accepted | AckStatus::Duplicate => {
                        if let Some(broker_order_id) = ack.broker_order_id {
                            *order_id = Some(broker_order_id);
                        }
                        if *cancel_due {
                            if let Some(order_id) = *order_id {
                                let bar_ts = cancel_bar_ts
                                    .or_else(|| ctx.last_bar_ts())
                                    .unwrap_or(ack.processed_ts_utc);
                                self.state = StrategyState::CancelSent {
                                    cancel_request_id: crate::deterministic_request_id(
                                        &ctx.strategy_id,
                                        &ctx.portfolio,
                                        symbol,
                                        "cancel",
                                        bar_ts,
                                        1,
                                    ),
                                    order_id,
                                    last_bar_ts: bar_ts,
                                };
                                return vec![self.build_cancel_intent(order_id)];
                            }
                        }
                        Vec::new()
                    }
                    AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                        self.state = StrategyState::Done {
                            last_bar_ts: self.last_processed_bar_ts.unwrap_or(ack.processed_ts_utc),
                        };
                        Vec::new()
                    }
                }
            }
            StrategyState::CancelSent {
                cancel_request_id, ..
            } => {
                if *cancel_request_id != ack.request_id {
                    return Vec::new();
                }
                self.state = StrategyState::Done {
                    last_bar_ts: self.last_processed_bar_ts.unwrap_or(ack.processed_ts_utc),
                };
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_order(&mut self, ctx: &StrategyCtx, event: &OrderEvent) -> Vec<Intent> {
        let symbol = ctx.symbol.as_str();
        match &mut self.state {
            StrategyState::Placed {
                place_request_id,
                order_id,
                cancel_due,
                cancel_bar_ts,
                ..
            } => {
                if let Some(request_id) = event.request_id {
                    if request_id != *place_request_id {
                        return Vec::new();
                    }
                }
                *order_id = Some(event.order_id);
                if *cancel_due {
                    let bar_ts = cancel_bar_ts.or_else(|| ctx.last_bar_ts()).unwrap_or(0);
                    self.state = StrategyState::CancelSent {
                        cancel_request_id: crate::deterministic_request_id(
                            &ctx.strategy_id,
                            &ctx.portfolio,
                            symbol,
                            "cancel",
                            bar_ts,
                            1,
                        ),
                        order_id: event.order_id,
                        last_bar_ts: bar_ts,
                    };
                    return vec![self.build_cancel_intent(event.order_id)];
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &crate::PositionEvent) -> Vec<Intent> {
        Vec::new()
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
    use crate::{deterministic_request_id, DataOrigin};

    fn config() -> LimitCancelConfig {
        LimitCancelConfig {
            symbol: "SBER".to_string(),
            tick_size: 0.01,
            offset_ticks: 1,
            qty: 1.0,
            side: Side::Buy,
            max_wait_bars_for_ack: 2,
        }
    }

    fn ctx() -> StrategyCtx {
        StrategyCtx {
            strategy_id: "strat".to_string(),
            portfolio: "port".to_string(),
            exchange: "ex".to_string(),
            symbol: "SBER".to_string(),
            tick_size: 0.01,
            last_bar_ts: None,
        }
    }

    fn bar(ts: i64, close: f64) -> BarEvent {
        BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: ts,
            close,
            o: close,
            h: close,
            l: close,
            v: 0.0,
            origin: DataOrigin::Live,
        }
    }

    #[test]
    fn happy_path_place_then_cancel() {
        let mut strategy = LimitCancelStrategy::new(config());
        let mut ctx = ctx();
        let bar1 = bar(10, 100.0);
        let intents = strategy.on_bar(&ctx, &bar1);
        assert_eq!(intents.len(), 1);
        match &intents[0] {
            Intent::Place { price, qty, side } => {
                assert_eq!(*price, 99.99);
                assert_eq!(*qty, 1.0);
                assert_eq!(*side, Side::Buy);
            }
            _ => panic!("expected place"),
        }

        ctx.last_bar_ts = Some(10);
        let ack_place = CommandAck::confirmed(
            deterministic_request_id("strat", "port", "SBER", "place", 10, 0),
            Some(123),
        );
        let intents = strategy.on_ack(&ctx, &ack_place);
        assert_eq!(intents.len(), 0);

        let bar2 = bar(20, 101.0);
        let intents = strategy.on_bar(&ctx, &bar2);
        assert_eq!(intents.len(), 1);
        assert!(matches!(intents[0], Intent::Cancel { order_id: 123 }));
    }

    #[test]
    fn ack_late_triggers_cancel() {
        let mut strategy = LimitCancelStrategy::new(config());
        let mut ctx = ctx();
        let bar1 = bar(10, 100.0);
        let _ = strategy.on_bar(&ctx, &bar1);
        let bar2 = bar(20, 101.0);
        let _ = strategy.on_bar(&ctx, &bar2);

        ctx.last_bar_ts = Some(20);
        let ack_place = CommandAck::confirmed(
            deterministic_request_id("strat", "port", "SBER", "place", 10, 0),
            Some(555),
        );
        let intents = strategy.on_ack(&ctx, &ack_place);
        assert_eq!(intents.len(), 1);
        assert!(matches!(intents[0], Intent::Cancel { order_id: 555 }));
    }

    #[test]
    fn order_event_brings_order_id_before_ack() {
        let mut strategy = LimitCancelStrategy::new(config());
        let mut ctx = ctx();
        let bar1 = bar(10, 100.0);
        let _ = strategy.on_bar(&ctx, &bar1);
        let bar2 = bar(20, 101.0);
        let _ = strategy.on_bar(&ctx, &bar2);

        ctx.last_bar_ts = Some(20);
        let order_event = OrderEvent {
            order_id: 999,
            request_id: Some(deterministic_request_id(
                "strat", "port", "SBER", "place", 10, 0,
            )),
        };
        let intents = strategy.on_order(&ctx, &order_event);
        assert_eq!(intents.len(), 1);
        assert!(matches!(intents[0], Intent::Cancel { order_id: 999 }));
    }
}
