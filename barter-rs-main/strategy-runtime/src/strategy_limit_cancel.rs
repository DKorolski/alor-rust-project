use alor_protocol::{AckStatus, CommandAck, OrderCommand, Side};

use crate::{build_cancel_command, build_place_command, BarEvent, OrderEvent};
use crate::state::StrategyState;

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
pub struct LimitCancelStateMachine {
    pub strategy_id: String,
    pub portfolio: String,
    pub exchange: String,
    pub config: LimitCancelConfig,
    pub state: StrategyState,
    pub last_processed_bar_ts: Option<i64>,
}

impl LimitCancelStateMachine {
    pub fn new(
        strategy_id: impl Into<String>,
        portfolio: impl Into<String>,
        exchange: impl Into<String>,
        config: LimitCancelConfig,
    ) -> Self {
        Self {
            strategy_id: strategy_id.into(),
            portfolio: portfolio.into(),
            exchange: exchange.into(),
            config,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
        }
    }

    pub fn on_bar(&mut self, bar: &BarEvent) -> Option<OrderCommand> {
        if self
            .last_processed_bar_ts
            .is_some_and(|last_ts| bar.close_time_utc <= last_ts)
        {
            return None;
        }

        let command = match &mut self.state {
            StrategyState::Idle => {
                let cmd = build_place_command(
                    &self.config,
                    &self.strategy_id,
                    &self.portfolio,
                    &self.exchange,
                    bar,
                );
                self.state = StrategyState::Placed {
                    place_request_id: cmd.request_id,
                    order_id: None,
                    cancel_due: false,
                    cancel_bar_ts: None,
                    placed_bar_ts: bar.close_time_utc,
                    last_bar_ts: bar.close_time_utc,
                    bars_waited: 0,
                };
                Some(cmd)
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
                    let cancel_cmd = existing_order_id
                        .and_then(|order_id| self.build_cancel(bar.symbol.as_str(), order_id));
                    if let Some(cmd) = cancel_cmd.as_ref() {
                        self.state = StrategyState::CancelSent {
                            cancel_request_id: cmd.request_id,
                            order_id: existing_order_id
                                .expect("order_id present when cancel generated"),
                            last_bar_ts: bar.close_time_utc,
                        };
                    }
                    cancel_cmd
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
        command
    }

    pub fn on_ack(&mut self, ack: &CommandAck) -> Option<OrderCommand> {
        let strategy_id = self.strategy_id.clone();
        let portfolio = self.portfolio.clone();
        let exchange = self.exchange.clone();
        let symbol = self.config.symbol.clone();
        match &mut self.state {
            StrategyState::Placed {
                place_request_id,
                order_id,
                cancel_due,
                cancel_bar_ts,
                ..
            } => {
                if *place_request_id != ack.request_id {
                    return None;
                }
                match ack.status {
                    AckStatus::Success | AckStatus::Accepted | AckStatus::Duplicate => {
                        if let Some(broker_order_id) = ack.broker_order_id {
                            *order_id = Some(broker_order_id);
                        }
                        if *cancel_due {
                            if let Some(order_id) = *order_id {
                                let bar_ts = cancel_bar_ts.unwrap_or(ack.processed_ts_utc);
                                let cmd = build_cancel_command(
                                    &strategy_id,
                                    &portfolio,
                                    &exchange,
                                    &symbol,
                                    order_id,
                                    bar_ts,
                                );
                                self.state = StrategyState::CancelSent {
                                    cancel_request_id: cmd.request_id,
                                    order_id,
                                    last_bar_ts: bar_ts,
                                };
                                return Some(cmd);
                            }
                        }
                        None
                    }
                    AckStatus::Error => {
                        self.state = StrategyState::Done {
                            last_bar_ts: self.last_processed_bar_ts.unwrap_or(ack.processed_ts_utc),
                        };
                        None
                    }
                }
            }
            StrategyState::CancelSent {
                cancel_request_id,
                ..
            } => {
                if *cancel_request_id != ack.request_id {
                    return None;
                }
                self.state = StrategyState::Done {
                    last_bar_ts: self.last_processed_bar_ts.unwrap_or(ack.processed_ts_utc),
                };
                None
            }
            _ => None,
        }
    }

    pub fn on_order_event(&mut self, event: &OrderEvent) -> Option<OrderCommand> {
        let strategy_id = self.strategy_id.clone();
        let portfolio = self.portfolio.clone();
        let exchange = self.exchange.clone();
        let symbol = self.config.symbol.clone();
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
                        return None;
                    }
                }
                *order_id = Some(event.order_id);
                if *cancel_due {
                    let bar_ts = cancel_bar_ts.unwrap_or(self.last_processed_bar_ts.unwrap_or(0));
                    let cmd = build_cancel_command(
                        &strategy_id,
                        &portfolio,
                        &exchange,
                        &symbol,
                        event.order_id,
                        bar_ts,
                    );
                    self.state = StrategyState::CancelSent {
                        cancel_request_id: cmd.request_id,
                        order_id: event.order_id,
                        last_bar_ts: bar_ts,
                    };
                    return Some(cmd);
                }
                None
            }
            _ => None,
        }
    }

    fn build_cancel(&self, symbol: &str, order_id: i64) -> Option<OrderCommand> {
        self.last_processed_bar_ts.map(|bar_ts| {
            build_cancel_command(
                &self.strategy_id,
                &self.portfolio,
                &self.exchange,
                symbol,
                order_id,
                bar_ts,
            )
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use alor_protocol::{CommandAction, CommandAck, PlaceOrder, Side};
    use uuid::Uuid;

    use crate::{BarEvent, DataOrigin, deterministic_request_id};

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
        let mut machine = LimitCancelStateMachine::new("strat", "port", "ex", config());
        let bar1 = bar(10, 100.0);
        let place_cmd = machine.on_bar(&bar1).expect("place cmd");
        match place_cmd.action {
            CommandAction::Place(PlaceOrder { .. }) => {}
            _ => panic!("expected place"),
        }
        let bar2 = bar(20, 101.0);
        assert!(machine.on_bar(&bar2).is_none());

        let ack_place = CommandAck::success(place_cmd.request_id, Some(123));
        let cancel_cmd = machine.on_ack(&ack_place).expect("cancel cmd");
        match cancel_cmd.action {
            CommandAction::Cancel(_) => {}
            _ => panic!("expected cancel"),
        }

        let ack_cancel = CommandAck::accepted(cancel_cmd.request_id);
        machine.on_ack(&ack_cancel);
        matches!(machine.state, StrategyState::Done { .. });
    }

    #[test]
    fn ack_late_triggers_cancel() {
        let mut machine = LimitCancelStateMachine::new("strat", "port", "ex", config());
        let bar1 = bar(10, 100.0);
        let place_cmd = machine.on_bar(&bar1).expect("place cmd");
        let bar2 = bar(20, 101.0);
        assert!(machine.on_bar(&bar2).is_none());

        let ack_place = CommandAck::success(place_cmd.request_id, Some(555));
        let cancel_cmd = machine.on_ack(&ack_place).expect("cancel cmd");
        assert!(matches!(cancel_cmd.action, CommandAction::Cancel(_)));
    }

    #[test]
    fn order_event_before_ack_triggers_cancel() {
        let mut machine = LimitCancelStateMachine::new("strat", "port", "ex", config());
        let bar1 = bar(10, 100.0);
        let place_cmd = machine.on_bar(&bar1).expect("place cmd");
        let bar2 = bar(20, 101.0);
        assert!(machine.on_bar(&bar2).is_none());

        let order_event = OrderEvent {
            order_id: 999,
            request_id: Some(place_cmd.request_id),
        };
        let cancel_cmd = machine
            .on_order_event(&order_event)
            .expect("cancel cmd");
        assert!(matches!(cancel_cmd.action, CommandAction::Cancel(_)));
    }

    #[test]
    fn duplicate_bars_are_ignored() {
        let mut machine = LimitCancelStateMachine::new("strat", "port", "ex", config());
        let bar1 = bar(10, 100.0);
        assert!(machine.on_bar(&bar1).is_some());
        let bar_dup = bar(10, 100.0);
        assert!(machine.on_bar(&bar_dup).is_none());
    }

    #[test]
    fn timeout_waiting_for_ack_moves_to_done() {
        let mut machine = LimitCancelStateMachine::new("strat", "port", "ex", config());
        let bar1 = bar(10, 100.0);
        let _ = machine.on_bar(&bar1);
        let bar2 = bar(20, 101.0);
        let bar3 = bar(30, 102.0);
        let _ = machine.on_bar(&bar2);
        let _ = machine.on_bar(&bar3);
        matches!(machine.state, StrategyState::Done { .. });
    }

    #[test]
    fn deterministic_request_id_is_stable() {
        let id1 = deterministic_request_id("strat", "port", "SBER", "place", 10, 0);
        let id2 = deterministic_request_id("strat", "port", "SBER", "place", 10, 0);
        let id3 = deterministic_request_id("strat", "port", "SBER", "cancel", 10, 1);
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
        assert_ne!(id2, id3);
    }

    #[test]
    fn deterministic_request_id_changes_with_inputs() {
        let id1 = deterministic_request_id("strat", "port", "SBER", "place", 10, 0);
        let id2 = deterministic_request_id("strat", "port", "SBER", "place", 11, 0);
        assert_ne!(id1, id2);
    }

    #[test]
    fn request_id_matches_expected_uuid_v5() {
        let id = deterministic_request_id("strat", "port", "SBER", "place", 10, 0);
        let expected = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            "strat|port|SBER|place|10|0".as_bytes(),
        );
        assert_eq!(id, expected);
    }
}
