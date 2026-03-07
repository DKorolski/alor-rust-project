use std::collections::HashSet;

use alor_protocol::{AckStatus, CommandAck, IntentClass, Side as OrderSide};
use chrono::{FixedOffset, NaiveDate, NaiveDateTime};

use crate::state::StrategyState;
use crate::strategies::hybrid_intraday::{
    Action, BreakoutEodMode, HybridOrchestrator, HybridOrchestratorConfig, IntradayBreakoutConfig,
    IntradayBreakoutEngine, MeanReversionConfig, MeanReversionEngine, Owner, Side,
};
use crate::{BarEvent, Intent, OrderEvent, PositionEvent, StopOrderEvent, Strategy, StrategyCtx};

#[derive(Debug, Clone)]
pub struct HybridIntradayRuntimeConfig {
    pub symbol: String,
    pub qty: f64,
    pub timezone_offset_hours: i32,
}

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    owner: Owner,
    side: Side,
}

#[derive(Debug)]
pub struct HybridIntradayRuntimeStrategy {
    config: HybridIntradayRuntimeConfig,
    orchestrator: HybridOrchestrator,
    state: StrategyState,
    last_processed_bar_ts: Option<i64>,
    last_position_qty: f64,
    current_owner: Option<Owner>,
    current_side: Option<Side>,
    pending_entry: Option<PendingEntry>,
    last_bar_close: Option<f64>,
    last_day_local: Option<NaiveDate>,
    current_day_high: Option<f64>,
    current_day_low: Option<f64>,
    prev_day_range: Option<f64>,
    working_orders: HashSet<i64>,
    working_stop_orders: HashSet<String>,
}

impl HybridIntradayRuntimeStrategy {
    pub fn new(config: HybridIntradayRuntimeConfig) -> Self {
        let mr = MeanReversionEngine::new(MeanReversionConfig::default());
        let br = IntradayBreakoutEngine::new(IntradayBreakoutConfig::default());
        let orchestrator = HybridOrchestrator::new(
            mr,
            br,
            HybridOrchestratorConfig {
                breakout_eod_mode: BreakoutEodMode::SameDay,
                ..HybridOrchestratorConfig::default()
            },
        );
        Self {
            config,
            orchestrator,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
            last_position_qty: 0.0,
            current_owner: None,
            current_side: None,
            pending_entry: None,
            last_bar_close: None,
            last_day_local: None,
            current_day_high: None,
            current_day_low: None,
            prev_day_range: None,
            working_orders: HashSet::new(),
            working_stop_orders: HashSet::new(),
        }
    }

    fn utc_to_local_naive(&self, ts_utc: i64) -> Option<NaiveDateTime> {
        let offset = FixedOffset::east_opt(self.config.timezone_offset_hours.saturating_mul(3600))?;
        chrono::DateTime::from_timestamp(ts_utc, 0).map(|dt| dt.with_timezone(&offset).naive_local())
    }

    fn has_live_orders(&self) -> bool {
        !self.working_orders.is_empty() || !self.working_stop_orders.is_empty()
    }

    fn side_to_order_side(side: Side) -> OrderSide {
        match side {
            Side::Long => OrderSide::Buy,
            Side::Short => OrderSide::Sell,
        }
    }

    fn update_day_aggregates(&mut self, dt_local: NaiveDateTime, high: f64, low: f64) {
        let day = dt_local.date();
        match self.last_day_local {
            None => {
                self.last_day_local = Some(day);
                self.current_day_high = Some(high);
                self.current_day_low = Some(low);
            }
            Some(prev_day) if prev_day == day => {
                self.current_day_high = Some(self.current_day_high.unwrap_or(high).max(high));
                self.current_day_low = Some(self.current_day_low.unwrap_or(low).min(low));
            }
            Some(_) => {
                if let (Some(h), Some(l)) = (self.current_day_high, self.current_day_low) {
                    self.prev_day_range = Some((h - l).max(0.0));
                }
                self.last_day_local = Some(day);
                self.current_day_high = Some(high);
                self.current_day_low = Some(low);
            }
        }
    }

    fn map_action_to_intents(&mut self, ctx: &StrategyCtx, action: Action) -> Vec<Intent> {
        match action {
            Action::SubmitEntry(entry) => {
                self.pending_entry = Some(PendingEntry {
                    owner: entry.owner,
                    side: entry.side,
                });
                vec![Intent::Market {
                    qty: self.config.qty.max(1.0),
                    side: Self::side_to_order_side(entry.side),
                    fill_price: None,
                }
                .with_class(IntentClass::Entry)]
            }
            Action::SubmitExit { owner, .. } => {
                let Some(pos_qty) = ctx.position_qty else {
                    return Vec::new();
                };
                let qty = pos_qty.abs();
                if qty <= f64::EPSILON {
                    return Vec::new();
                }
                let side = if pos_qty >= 0.0 {
                    OrderSide::Sell
                } else {
                    OrderSide::Buy
                };
                self.current_owner = Some(owner);
                vec![Intent::Market {
                    qty,
                    side,
                    fill_price: None,
                }
                .with_class(IntentClass::Exit)]
            }
            Action::ArmOvernightExit { .. } => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StrategyCtx, TradeMode};

    fn test_ctx(position_qty: Option<f64>) -> StrategyCtx {
        StrategyCtx {
            strategy_id: "hyb-test".to_string(),
            portfolio: "demo".to_string(),
            exchange: "MOEX".to_string(),
            symbol: "IMOEXF".to_string(),
            tick_size: 0.5,
            trade_mode: TradeMode::Live,
            allow_live_orders: true,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty,
            last_bar_ts: Some(1),
        }
    }

    #[test]
    fn submit_exit_uses_current_position_qty_without_flip() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(HybridIntradayRuntimeConfig {
            symbol: "IMOEXF".to_string(),
            qty: 10.0,
            timezone_offset_hours: 3,
        });
        let ctx = test_ctx(Some(1.0));
        let intents = strategy.map_action_to_intents(
            &ctx,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            Intent::Classified { intent, intent_class } => {
                assert_eq!(*intent_class, IntentClass::Exit);
                match intent.as_ref() {
                    Intent::Market { qty, side, .. } => {
                        assert!((*qty - 1.0).abs() <= f64::EPSILON);
                        assert_eq!(*side, OrderSide::Sell);
                    }
                    other => panic!("unexpected base intent: {other:?}"),
                }
            }
            other => panic!("expected classified intent, got {other:?}"),
        }
    }
}

impl Strategy for HybridIntradayRuntimeStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            return Vec::new();
        }
        if self
            .last_processed_bar_ts
            .is_some_and(|last_ts| bar.close_time_utc <= last_ts)
        {
            return Vec::new();
        }
        let Some(dt_local) = self.utc_to_local_naive(bar.close_time_utc) else {
            return Vec::new();
        };
        self.update_day_aggregates(dt_local, bar.h, bar.l);

        let close_prev = self.last_bar_close.unwrap_or(bar.close);
        let day_range_prev = self.prev_day_range.unwrap_or(0.0);
        let has_open_position = ctx.position_qty.unwrap_or(0.0).abs() > f64::EPSILON;
        let actions = self.orchestrator.on_bar(crate::strategies::hybrid_intraday::orchestrator::BarInput {
            dt: dt_local,
            open: bar.o,
            high: bar.h,
            low: bar.l,
            close: bar.close,
            close_prev,
            day_range_prev,
            has_open_position,
            has_live_orders: self.has_live_orders(),
        });
        self.last_processed_bar_ts = Some(bar.close_time_utc);
        self.last_bar_close = Some(bar.close);
        self.state = StrategyState::Idle;
        actions
            .into_iter()
            .flat_map(|action| self.map_action_to_intents(ctx, action))
            .collect()
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        if matches!(
            ack.status,
            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
        ) && self.pending_entry.is_some()
        {
            self.orchestrator.on_order_rejected("entry");
            self.pending_entry = None;
        }
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent> {
        if ord.symbol != self.config.symbol {
            return Vec::new();
        }
        let status = ord.status.to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "filled" | "canceled" | "cancelled" | "expired" | "rejected"
        ) {
            self.working_orders.remove(&ord.order_id);
        } else if ord.order_id > 0 {
            self.working_orders.insert(ord.order_id);
        }
        Vec::new()
    }

    fn on_stop_order(&mut self, _ctx: &StrategyCtx, ord: &StopOrderEvent) -> Vec<Intent> {
        if ord.symbol != self.config.symbol {
            return Vec::new();
        }
        let status = ord.status.to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "filled" | "canceled" | "cancelled" | "expired" | "rejected" | "executed" | "triggered" | "done" | "completed"
        ) {
            self.working_stop_orders.remove(&ord.stop_order_id);
        } else if !ord.stop_order_id.trim().is_empty() {
            self.working_stop_orders.insert(ord.stop_order_id.clone());
        }
        Vec::new()
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }
        let prev = self.last_position_qty;
        let cur = pos.qty;
        if prev.abs() <= f64::EPSILON && cur.abs() > f64::EPSILON {
            let filled = self.pending_entry.take();
            if let Some(entry) = filled {
                self.current_owner = Some(entry.owner);
                self.current_side = Some(entry.side);
                self.orchestrator
                    .on_order_filled("entry", entry.owner, Some(entry.side));
            }
        } else if prev.abs() > f64::EPSILON && cur.abs() <= f64::EPSILON {
            let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
            self.orchestrator.on_order_filled("exit", owner, None);
            self.current_owner = None;
            self.current_side = None;
            self.pending_entry = None;
        }
        self.last_position_qty = cur;
        Vec::new()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        self.state = state;
    }
}
