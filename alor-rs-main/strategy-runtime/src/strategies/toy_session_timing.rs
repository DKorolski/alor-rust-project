use alor_protocol::{CommandAck, Side};
use chrono::{TimeZone, Timelike, Utc};

use crate::state::StrategyState;
use crate::{BarEvent, Intent, PositionEvent, Strategy, StrategyCtx, TradeMode};

#[derive(Debug, Clone)]
pub struct ToySessionTimingConfig {
    pub symbol: String,
    pub qty: f64,
    pub entry_side: Side,
    pub session_open_hour: u32,
    pub session_open_minute: u32,
    pub session_close_hour: u32,
    pub session_close_minute: u32,
    pub entry_after_open_min: u32,
    pub exit_before_close_min: u32,
    pub timezone_offset_hours: i32,
}

impl ToySessionTimingConfig {
    fn entry_minute_of_day(&self) -> u32 {
        self.session_open_hour * 60 + self.session_open_minute + self.entry_after_open_min
    }

    fn exit_minute_of_day(&self) -> u32 {
        self.session_close_hour * 60 + self.session_close_minute - self.exit_before_close_min
    }
}

#[derive(Debug)]
pub struct ToySessionTimingStrategy {
    pub config: ToySessionTimingConfig,
    state: StrategyState,
    last_processed_bar_ts: Option<i64>,
    position_open: bool,
    traded_day_key: Option<String>,
}

impl ToySessionTimingStrategy {
    pub fn new(config: ToySessionTimingConfig) -> Self {
        Self {
            config,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
            position_open: false,
            traded_day_key: None,
        }
    }

    fn close_side(&self) -> Side {
        match self.config.entry_side {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }

    fn local_day_and_minute(&self, ts_utc: i64) -> Option<(String, u32)> {
        let dt_utc = Utc.timestamp_opt(ts_utc, 0).single()?;
        let dt_local = dt_utc + chrono::Duration::hours(self.config.timezone_offset_hours as i64);
        let day_key = dt_local.date_naive().to_string();
        let minute_of_day = dt_local.hour() * 60 + dt_local.minute();
        Some((day_key, minute_of_day))
    }

    fn fill_price_for_mode(&self, bar: &BarEvent, mode: TradeMode) -> Option<f64> {
        match mode {
            TradeMode::Live => None,
            TradeMode::Paper | TradeMode::Backtest => Some(bar.o),
        }
    }
}

impl Strategy for ToySessionTimingStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            return Vec::new();
        }
        if self
            .last_processed_bar_ts
            .is_some_and(|last| bar.close_time_utc <= last)
        {
            return Vec::new();
        }

        let Some((day_key, minute_of_day)) = self.local_day_and_minute(bar.close_time_utc) else {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        };

        if self.traded_day_key.as_deref() != Some(day_key.as_str()) && !self.position_open {
            self.traded_day_key = None;
        }

        let entry_min = self.config.entry_minute_of_day();
        let exit_min = self.config.exit_minute_of_day();
        let mut intents = Vec::new();

        if self.position_open {
            if minute_of_day >= exit_min {
                intents.push(Intent::Market {
                    qty: self.config.qty,
                    side: self.close_side(),
                    fill_price: self.fill_price_for_mode(bar, ctx.trade_mode),
                    comment: None,
                });
                self.position_open = false;
                self.state = StrategyState::Idle;
            }
        } else if self.traded_day_key.is_none()
            && minute_of_day >= entry_min
            && minute_of_day < exit_min
        {
            intents.push(Intent::Market {
                qty: self.config.qty,
                side: self.config.entry_side,
                fill_price: self.fill_price_for_mode(bar, ctx.trade_mode),
                comment: None,
            });
            self.position_open = true;
            self.traded_day_key = Some(day_key);
            self.state = StrategyState::MarketBuySent {
                buy_request_id: crate::deterministic_request_id(
                    &ctx.strategy_id,
                    &ctx.portfolio,
                    &bar.symbol,
                    "market_buy",
                    bar.close_time_utc,
                    0,
                ),
                baseline_qty: None,
                close_trigger: crate::CloseTrigger::NextBar,
                buy_bar_ts: bar.close_time_utc,
                last_bar_ts: bar.close_time_utc,
            };
        }

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &CommandAck) -> Vec<Intent> {
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &crate::OrderEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }
        self.position_open = pos.qty.abs() > f64::EPSILON;
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
    use crate::{DataOrigin, StrategyCtx};

    fn mk_bar(ts: i64, minute: u32) -> BarEvent {
        // Build ts by replacing minute in the supplied day UTC+3 assumption for tests
        let dt = Utc.timestamp_opt(ts, 0).single().unwrap();
        let dt = dt
            .with_hour((minute / 60) % 24)
            .unwrap()
            .with_minute(minute % 60)
            .unwrap()
            .with_second(0)
            .unwrap();
        BarEvent {
            symbol: "IMOEXF".to_string(),
            close_time_utc: dt.timestamp(),
            close: 100.0,
            o: 100.0,
            h: 100.0,
            l: 100.0,
            v: 1.0,
            origin: DataOrigin::History,
        }
    }

    fn ctx() -> StrategyCtx {
        StrategyCtx {
            strategy_id: "toy_session_timing".to_string(),
            portfolio: "p".to_string(),
            exchange: "x".to_string(),
            symbol: "IMOEXF".to_string(),
            tick_size: 0.5,
            trade_mode: TradeMode::Backtest,
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders: false,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty: None,
            last_bar_ts: None,
        }
    }

    #[test]
    fn emits_entry_then_exit_once_per_day() {
        let mut strategy = ToySessionTimingStrategy::new(ToySessionTimingConfig {
            symbol: "IMOEXF".to_string(),
            qty: 1.0,
            entry_side: Side::Buy,
            session_open_hour: 0,
            session_open_minute: 0,
            session_close_hour: 23,
            session_close_minute: 59,
            entry_after_open_min: 59,
            exit_before_close_min: 20,
            timezone_offset_hours: 0,
        });
        let c = ctx();
        let day_start_ts = 1_735_689_600; // 2025-01-15 00:00:00 UTC

        let no_entry = strategy.on_bar(&c, &mk_bar(day_start_ts, 58));
        assert!(no_entry.is_empty());

        let entry = strategy.on_bar(&c, &mk_bar(day_start_ts, 59));
        assert!(matches!(
            entry.as_slice(),
            [Intent::Market {
                side: Side::Buy,
                ..
            }]
        ));

        let no_second_entry = strategy.on_bar(&c, &mk_bar(day_start_ts, 65));
        assert!(no_second_entry.is_empty());

        let exit = strategy.on_bar(&c, &mk_bar(day_start_ts, 23 * 60 + 39));
        assert!(matches!(
            exit.as_slice(),
            [Intent::Market {
                side: Side::Sell,
                ..
            }]
        ));
    }
}
