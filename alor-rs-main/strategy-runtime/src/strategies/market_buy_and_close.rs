use alor_protocol::{AckStatus, CommandAck, Side};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::live_guard::GatewayPhase;
use crate::state::StrategyState;
use crate::{BarEvent, CloseTrigger, Intent, PositionEvent, Strategy, StrategyCtx, TradeMode};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketBuyAndCloseLiveOrderStyle {
    Market,
    MarketableLimit,
}

impl Default for MarketBuyAndCloseLiveOrderStyle {
    fn default() -> Self {
        Self::Market
    }
}

impl MarketBuyAndCloseLiveOrderStyle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::MarketableLimit => "marketable_limit",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MarketBuyAndCloseConfig {
    pub symbol: String,
    pub qty: f64,
    pub side: Side,
    pub live_order_style: MarketBuyAndCloseLiveOrderStyle,
    pub tick_size: f64,
    pub marketable_limit_offset_ticks: i64,
    pub close_trigger: CloseTrigger,
    pub entry_ack_timeout_ms: u64,
    pub entry_fill_timeout_ms: u64,
    pub exit_ack_timeout_ms: u64,
    pub exit_fill_timeout_ms: u64,
}

#[derive(Debug)]
pub struct MarketBuyAndCloseStrategy {
    pub config: MarketBuyAndCloseConfig,
    pub state: StrategyState,
    pub last_processed_bar_ts: Option<i64>,
    pub last_bar_close: Option<f64>,
}

impl MarketBuyAndCloseStrategy {
    pub fn new(config: MarketBuyAndCloseConfig) -> Self {
        Self {
            config,
            state: StrategyState::Idle,
            last_processed_bar_ts: None,
            last_bar_close: None,
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

    fn blocked(&mut self, reason: impl Into<String>, ts: i64) {
        self.state = StrategyState::Blocked {
            reason: reason.into(),
            last_bar_ts: ts,
        };
    }

    fn live_marketable_price_from_reference(&self, side: Side, reference_price: f64) -> f64 {
        if self.config.tick_size <= 0.0 {
            return reference_price;
        }
        // Keep one extra tick of aggressiveness so gateway normalization does not
        // turn a marketable limit back into a passive one.
        let aggressive_ticks = self.config.marketable_limit_offset_ticks.max(0) + 1;
        let shift = aggressive_ticks as f64 * self.config.tick_size;
        match side {
            Side::Buy => reference_price + shift,
            Side::Sell => reference_price - shift,
        }
    }

    fn build_live_intent(
        &self,
        request_id: Uuid,
        side: Side,
        qty: f64,
        reference_price: f64,
        reason: &'static str,
    ) -> Intent {
        match self.config.live_order_style {
            MarketBuyAndCloseLiveOrderStyle::Market => {
                info!(
                    strategy = "market_buy_and_close",
                    live_order_style = self.config.live_order_style.as_str(),
                    request_id = %request_id,
                    side = ?side,
                    qty,
                    reason,
                    "market_buy_and_close live intent prepared"
                );
                Intent::Market {
                    qty,
                    side,
                    fill_price: None,
                    comment: None,
                }
            }
            MarketBuyAndCloseLiveOrderStyle::MarketableLimit => {
                let price = self.live_marketable_price_from_reference(side, reference_price);
                info!(
                    strategy = "market_buy_and_close",
                    live_order_style = self.config.live_order_style.as_str(),
                    request_id = %request_id,
                    side = ?side,
                    qty,
                    price,
                    reason,
                    "market_buy_and_close live intent prepared"
                );
                Intent::Place {
                    price,
                    qty,
                    side,
                    comment: None,
                }
            }
        }
    }

    fn live_request_id(
        live_order_style: MarketBuyAndCloseLiveOrderStyle,
        ctx: &StrategyCtx,
        symbol: &str,
        side: Side,
    ) -> Uuid {
        match live_order_style {
            MarketBuyAndCloseLiveOrderStyle::Market => crate::deterministic_market_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                symbol,
                ctx.event_ts_utc(),
                side,
            ),
            MarketBuyAndCloseLiveOrderStyle::MarketableLimit => crate::deterministic_request_id(
                &ctx.strategy_id,
                &ctx.portfolio,
                symbol,
                "place",
                ctx.event_ts_utc(),
                0,
            ),
        }
    }

    fn maybe_open_for_paper_backtest(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        let mut intent = None;
        let close_side = self.close_side();
        let qty = self.config.qty;
        match &mut self.state {
            StrategyState::Idle => {
                self.state = StrategyState::MarketBuyPending {
                    buy_request_id: crate::deterministic_request_id(
                        &ctx.strategy_id,
                        &ctx.portfolio,
                        &bar.symbol,
                        "market_buy",
                        bar.close_time_utc,
                        0,
                    ),
                    baseline_qty: ctx.position_qty,
                    close_trigger: CloseTrigger::NextBar,
                    pending_bar_ts: bar.close_time_utc,
                    last_bar_ts: bar.close_time_utc,
                };
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
                    intent = Some(Intent::Market {
                        qty: self.config.qty,
                        side: self.config.side,
                        fill_price: Some(bar.o),
                        comment: None,
                    });
                }
            }
            StrategyState::MarketBuySent {
                buy_bar_ts,
                last_bar_ts,
                baseline_qty,
                ..
            } => {
                *last_bar_ts = bar.close_time_utc;
                if bar.close_time_utc > *buy_bar_ts {
                    intent = Some(Intent::Market {
                        qty,
                        side: close_side,
                        fill_price: Some(bar.o),
                        comment: None,
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
            StrategyState::MarketCloseSent { last_bar_ts, .. }
            | StrategyState::Done { last_bar_ts }
            | StrategyState::Blocked { last_bar_ts, .. } => *last_bar_ts = bar.close_time_utc,
            _ => {}
        }
        intent.into_iter().collect()
    }

    fn maybe_send_live_entry(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if ctx.last_bar_ts().is_none() {
            info!(
                strategy = "market_buy_and_close",
                ts_utc = bar.close_time_utc,
                "market_buy_and_close live entry deferred until second bar"
            );
            return Vec::new();
        }
        let baseline_qty = ctx.position_qty.unwrap_or(0.0);
        let request_guid =
            Self::live_request_id(self.config.live_order_style, ctx, &bar.symbol, self.config.side);
        self.state = StrategyState::MarketLivePendingEntry {
            request_guid,
            side: self.config.side,
            qty: self.config.qty,
            baseline_qty,
            close_trigger: self.config.close_trigger,
            sent_ts: ctx.now_ts_utc(),
            acked: false,
            entry_confirmed_ts: None,
            last_bar_ts: bar.close_time_utc,
        };
        vec![self.build_live_intent(
            request_guid,
            self.config.side,
            self.config.qty,
            bar.close,
            "entry",
        )]
    }

    fn check_live_timeouts(&mut self, now_ts_utc: i64, bar_ts: i64) {
        let now_ms = now_ts_utc.saturating_mul(1_000);
        match &self.state {
            StrategyState::MarketLivePendingEntry {
                sent_ts,
                acked,
                entry_confirmed_ts,
                ..
            } => {
                let elapsed = now_ms.saturating_sub(sent_ts.saturating_mul(1_000)) as u64;
                if !acked && elapsed > self.config.entry_ack_timeout_ms {
                    self.blocked(
                        format!(
                            "entry_ack_timeout_ms exceeded: {}",
                            self.config.entry_ack_timeout_ms
                        ),
                        bar_ts,
                    );
                } else if entry_confirmed_ts.is_none()
                    && elapsed > self.config.entry_fill_timeout_ms
                {
                    self.blocked(
                        format!(
                            "entry_fill_timeout_ms exceeded: {}",
                            self.config.entry_fill_timeout_ms
                        ),
                        bar_ts,
                    );
                }
            }
            StrategyState::MarketLivePendingExit { sent_ts, acked, .. } => {
                let elapsed = now_ms.saturating_sub(sent_ts.saturating_mul(1_000)) as u64;
                if !acked && elapsed > self.config.exit_ack_timeout_ms {
                    self.blocked(
                        format!(
                            "exit_ack_timeout_ms exceeded: {}",
                            self.config.exit_ack_timeout_ms
                        ),
                        bar_ts,
                    );
                } else if elapsed > self.config.exit_fill_timeout_ms {
                    self.blocked(
                        format!(
                            "exit_fill_timeout_ms exceeded: {}",
                            self.config.exit_fill_timeout_ms
                        ),
                        bar_ts,
                    );
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

        self.last_bar_close = Some(bar.close);
        let intents = match ctx.trade_mode {
            TradeMode::Paper | TradeMode::Backtest => self.maybe_open_for_paper_backtest(ctx, bar),
            TradeMode::Live => {
                self.check_live_timeouts(ctx.now_ts_utc(), bar.close_time_utc);
                let close_side = self.close_side();
                match &mut self.state {
                    StrategyState::Idle => self.maybe_send_live_entry(ctx, bar),
                    StrategyState::MarketLiveInPosition {
                        close_trigger,
                        opened_ts,
                        baseline_qty,
                        ..
                    } => {
                        if *close_trigger == CloseTrigger::NextBar
                            && bar.close_time_utc > *opened_ts
                        {
                            let baseline = *baseline_qty;
                            info!(
                                from = "market_live_in_position",
                                to = "market_live_pending_exit",
                                reason = "next_bar",
                                baseline_qty = baseline,
                                ts_utc = bar.close_time_utc,
                                "strategy_state_transition"
                            );
                            let request_guid = Self::live_request_id(
                                self.config.live_order_style,
                                ctx,
                                &bar.symbol,
                                close_side,
                            );
                            self.state = StrategyState::MarketLivePendingExit {
                                request_guid,
                                reason: "next_bar".to_string(),
                                side: close_side,
                                qty: self.config.qty,
                                baseline_qty: baseline,
                                sent_ts: ctx.now_ts_utc(),
                                acked: false,
                                last_bar_ts: bar.close_time_utc,
                            };
                            vec![self.build_live_intent(
                                request_guid,
                                close_side,
                                self.config.qty,
                                bar.close,
                                "flatten",
                            )]
                        } else {
                            Vec::new()
                        }
                    }
                    StrategyState::MarketLivePendingEntry { last_bar_ts, .. }
                    | StrategyState::MarketLivePendingExit { last_bar_ts, .. }
                    | StrategyState::Blocked { last_bar_ts, .. }
                    | StrategyState::Done { last_bar_ts } => {
                        *last_bar_ts = bar.close_time_utc;
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
        };

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        match &mut self.state {
            StrategyState::MarketLivePendingEntry {
                request_guid,
                acked,
                ..
            } => {
                if ack.request_id == *request_guid {
                    match ack.status {
                        AckStatus::Accepted | AckStatus::Confirmed | AckStatus::Duplicate => {
                            *acked = true;
                        }
                        AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                            self.blocked(
                                format!("entry_rejected status={:?}", ack.status),
                                ack.processed_ts_utc,
                            );
                        }
                    }
                }
            }
            StrategyState::MarketLivePendingExit {
                request_guid,
                acked,
                ..
            } => {
                if ack.request_id == *request_guid {
                    match ack.status {
                        AckStatus::Accepted | AckStatus::Confirmed | AckStatus::Duplicate => {
                            *acked = true;
                        }
                        AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                            self.blocked(
                                format!("exit_rejected status={:?}", ack.status),
                                ack.processed_ts_utc,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _event: &crate::OrderEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_position(&mut self, ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if pos.symbol != self.config.symbol {
            return Vec::new();
        }

        let now_ts = ctx.last_bar_ts().unwrap_or(pos.ts_utc);
        let dispatch_ts = ctx.now_ts_utc();
        let close_side = self.close_side();
        match &mut self.state {
            StrategyState::MarketLivePendingEntry {
                baseline_qty,
                close_trigger,
                entry_confirmed_ts,
                last_bar_ts,
                ..
            } => {
                *last_bar_ts = now_ts;
                if (pos.qty - *baseline_qty).abs() > f64::EPSILON {
                    *entry_confirmed_ts = Some(now_ts);
                    info!(
                        from = "market_live_pending_entry",
                        to = "market_live_in_position",
                        baseline_qty = *baseline_qty,
                        broker_qty = pos.qty,
                        avg_price = pos.avg_price,
                        ts_utc = now_ts,
                        "strategy_state_transition"
                    );
                    self.state = StrategyState::MarketLiveInPosition {
                        side: self.config.side,
                        qty: self.config.qty,
                        avg_price: pos.avg_price,
                        baseline_qty: *baseline_qty,
                        close_trigger: *close_trigger,
                        opened_ts: now_ts,
                        last_bar_ts: now_ts,
                    };
                }
            }
            StrategyState::MarketLiveInPosition {
                baseline_qty,
                close_trigger,
                ..
            } => {
                if (pos.qty - *baseline_qty).abs() <= f64::EPSILON {
                    warn!(
                        qty = pos.qty,
                        baseline = *baseline_qty,
                        "state corrected by broker"
                    );
                    self.state = StrategyState::Idle;
                    return Vec::new();
                }
                if *close_trigger == CloseTrigger::PositionUpdate {
                    info!(
                        from = "market_live_in_position",
                        to = "market_live_pending_exit",
                        reason = "position_update",
                        baseline_qty = *baseline_qty,
                        broker_qty = pos.qty,
                        ts_utc = now_ts,
                        "strategy_state_transition"
                    );
                    let request_guid = Self::live_request_id(
                        self.config.live_order_style,
                        ctx,
                        &pos.symbol,
                        close_side,
                    );
                    self.state = StrategyState::MarketLivePendingExit {
                        request_guid,
                        reason: "position_update".to_string(),
                        side: close_side,
                        qty: self.config.qty,
                        baseline_qty: *baseline_qty,
                        sent_ts: dispatch_ts,
                        acked: false,
                        last_bar_ts: now_ts,
                    };
                    let reference_price = self.last_bar_close.unwrap_or(pos.avg_price);
                    return vec![self.build_live_intent(
                        request_guid,
                        close_side,
                        self.config.qty,
                        reference_price,
                        "flatten",
                    )];
                }
            }
            StrategyState::MarketLivePendingExit {
                baseline_qty,
                last_bar_ts,
                ..
            } => {
                *last_bar_ts = now_ts;
                if (pos.qty - *baseline_qty).abs() <= f64::EPSILON {
                    info!(
                        from = "market_live_pending_exit",
                        to = "done",
                        baseline_qty = *baseline_qty,
                        broker_qty = pos.qty,
                        ts_utc = now_ts,
                        "strategy_state_transition"
                    );
                    self.state = StrategyState::Done {
                        last_bar_ts: now_ts,
                    };
                }
            }
            StrategyState::Idle => {
                if pos.qty.abs() > f64::EPSILON {
                    warn!(qty = pos.qty, "state corrected by broker");
                    self.state = StrategyState::MarketLiveInPosition {
                        side: self.config.side,
                        qty: pos.qty.abs(),
                        avg_price: pos.avg_price,
                        baseline_qty: 0.0,
                        close_trigger: self.config.close_trigger,
                        opened_ts: now_ts,
                        last_bar_ts: now_ts,
                    };
                }
            }
            _ => {}
        }
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
    use crate::{DataOrigin, StrategyCtx, TradeMode};

    fn config() -> MarketBuyAndCloseConfig {
        MarketBuyAndCloseConfig {
            symbol: "SBER".to_string(),
            qty: 1.0,
            side: Side::Buy,
            live_order_style: MarketBuyAndCloseLiveOrderStyle::Market,
            tick_size: 0.01,
            marketable_limit_offset_ticks: 0,
            close_trigger: CloseTrigger::NextBar,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
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
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders: true,
            gateway_phase: GatewayPhase::LiveReady,
            position_qty: Some(0.0),
            event_ts_utc: 0,
            now_ts_utc: 0,
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
    fn live_needs_position_confirmation_before_close() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let mut ctx = ctx(TradeMode::Live);
        let bar1 = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 10,
            close: 10.0,
            o: 9.5,
            h: 10.0,
            l: 9.5,
            v: 1.0,
            origin: DataOrigin::Live,
        };
        let bar2 = BarEvent {
            close_time_utc: 20,
            o: 10.5,
            close: 11.0,
            ..bar1.clone()
        };

        assert!(strategy.on_bar(&ctx, &bar1).is_empty());
        ctx.last_bar_ts = Some(10);
        assert_eq!(strategy.on_bar(&ctx, &bar2).len(), 1);

        ctx.last_bar_ts = Some(20);
        let pos = PositionEvent {
            symbol: "SBER".to_string(),
            qty: 1.0,
            existing: false,
            avg_price: 10.2,
            ts_utc: 20,
        };
        assert!(strategy.on_position(&ctx, &pos).is_empty());

        let bar3 = BarEvent {
            close_time_utc: 30,
            o: 11.0,
            close: 11.2,
            ..bar1
        };
        assert_eq!(strategy.on_bar(&ctx, &bar3).len(), 1);
    }

    #[test]
    fn live_market_mode_entry_still_uses_market_intent() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let mut ctx = ctx(TradeMode::Live);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 10,
            close: 10.0,
            o: 9.5,
            h: 10.0,
            l: 9.5,
            v: 1.0,
            origin: DataOrigin::Live,
        };

        ctx.last_bar_ts = Some(5);
        let intents = strategy.on_bar(&ctx, &bar);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Market {
                qty,
                side: Side::Buy,
                ..
            }] if (*qty - 1.0).abs() <= f64::EPSILON
        ));
    }

    #[test]
    fn marketable_limit_entry_uses_place_intent() {
        let mut cfg = config();
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = MarketBuyAndCloseStrategy::new(cfg);
        let mut ctx = ctx(TradeMode::Live);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 10,
            close: 10.0,
            o: 9.5,
            h: 10.0,
            l: 9.5,
            v: 1.0,
            origin: DataOrigin::Live,
        };

        ctx.last_bar_ts = Some(5);
        let intents = strategy.on_bar(&ctx, &bar);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Place {
                price,
                qty,
                side: Side::Buy,
                ..
            }] if (*qty - 1.0).abs() <= f64::EPSILON && (*price - 10.01).abs() <= 1e-9
        ));
        let expected_request_id = crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &bar.symbol,
            "place",
            ctx.event_ts_utc(),
            0,
        );
        assert!(matches!(
            strategy.state(),
            StrategyState::MarketLivePendingEntry { request_guid, .. } if *request_guid == expected_request_id
        ));
    }
    #[test]
    fn live_blocks_on_entry_ack_timeout() {
        let mut cfg = config();
        cfg.entry_ack_timeout_ms = 1_000;
        let mut strategy = MarketBuyAndCloseStrategy::new(cfg);
        let mut ctx = ctx(TradeMode::Live);
        let bar1 = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 10,
            close: 10.0,
            o: 9.5,
            h: 10.0,
            l: 9.5,
            v: 1.0,
            origin: DataOrigin::Live,
        };
        let bar2 = BarEvent {
            close_time_utc: 12,
            ..bar1.clone()
        };
        let bar3 = BarEvent {
            close_time_utc: 14,
            ..bar1.clone()
        };
        assert!(strategy.on_bar(&ctx, &bar1).is_empty());
        ctx.last_bar_ts = Some(10);
        ctx.now_ts_utc = 12;
        assert_eq!(strategy.on_bar(&ctx, &bar2).len(), 1);
        ctx.now_ts_utc = 14;
        assert!(strategy.on_bar(&ctx, &bar3).is_empty());
        assert!(matches!(strategy.state(), StrategyState::Blocked { .. }));
    }

    #[test]
    fn live_waits_for_second_bar_before_entry() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let ctx = ctx(TradeMode::Live);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 10,
            close: 10.0,
            o: 9.5,
            h: 10.0,
            l: 9.5,
            v: 1.0,
            origin: DataOrigin::Live,
        };

        let intents = strategy.on_bar(&ctx, &bar);
        assert!(intents.is_empty());
        assert!(matches!(strategy.state(), StrategyState::Idle));
    }

    #[test]
    fn live_position_update_trigger_transitions_to_pending_exit() {
        let mut cfg = config();
        cfg.close_trigger = CloseTrigger::PositionUpdate;
        let mut strategy = MarketBuyAndCloseStrategy::new(cfg);
        let mut ctx = ctx(TradeMode::Live);

        strategy.state = StrategyState::MarketLiveInPosition {
            side: Side::Buy,
            qty: 1.0,
            avg_price: 100.0,
            baseline_qty: 0.0,
            close_trigger: CloseTrigger::PositionUpdate,
            opened_ts: 100,
            last_bar_ts: 100,
        };

        ctx.last_bar_ts = Some(120);
        let pos = PositionEvent {
            symbol: "SBER".to_string(),
            qty: 1.0,
            existing: false,
            avg_price: 101.0,
            ts_utc: 120,
        };

        let intents = strategy.on_position(&ctx, &pos);
        assert_eq!(intents.len(), 1);
        assert!(matches!(
            strategy.state(),
            StrategyState::MarketLivePendingExit {
                reason,
                baseline_qty,
                ..
            } if reason == "position_update" && (*baseline_qty - 0.0).abs() <= f64::EPSILON
        ));
    }

    #[test]
    fn marketable_limit_position_update_uses_place_for_flatten() {
        let mut cfg = config();
        cfg.close_trigger = CloseTrigger::PositionUpdate;
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = MarketBuyAndCloseStrategy::new(cfg);
        let mut ctx = ctx(TradeMode::Live);

        strategy.last_bar_close = Some(101.0);
        strategy.state = StrategyState::MarketLiveInPosition {
            side: Side::Buy,
            qty: 1.0,
            avg_price: 100.0,
            baseline_qty: 0.0,
            close_trigger: CloseTrigger::PositionUpdate,
            opened_ts: 100,
            last_bar_ts: 100,
        };

        ctx.last_bar_ts = Some(120);
        let pos = PositionEvent {
            symbol: "SBER".to_string(),
            qty: 1.0,
            existing: false,
            avg_price: 101.0,
            ts_utc: 120,
        };

        let intents = strategy.on_position(&ctx, &pos);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Place {
                price,
                qty,
                side: Side::Sell,
                ..
            }] if (*qty - 1.0).abs() <= f64::EPSILON && (*price - 100.99).abs() <= 1e-9
        ));
    }

    #[test]
    fn live_next_bar_trigger_transitions_to_pending_exit() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let mut ctx = ctx(TradeMode::Live);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 120,
            close: 101.0,
            o: 100.5,
            h: 101.5,
            l: 100.0,
            v: 1.0,
            origin: DataOrigin::Live,
        };

        strategy.state = StrategyState::MarketLiveInPosition {
            side: Side::Buy,
            qty: 1.0,
            avg_price: 100.0,
            baseline_qty: 0.0,
            close_trigger: CloseTrigger::NextBar,
            opened_ts: 100,
            last_bar_ts: 100,
        };

        ctx.now_ts_utc = 120;
        let intents = strategy.on_bar(&ctx, &bar);
        assert_eq!(intents.len(), 1);
        assert!(matches!(
            strategy.state(),
            StrategyState::MarketLivePendingExit {
                reason,
                baseline_qty,
                sent_ts,
                ..
            } if reason == "next_bar" && (*baseline_qty - 0.0).abs() <= f64::EPSILON && *sent_ts == 120
        ));
    }

    #[test]
    fn marketable_limit_next_bar_uses_place_for_flatten() {
        let mut cfg = config();
        cfg.live_order_style = MarketBuyAndCloseLiveOrderStyle::MarketableLimit;
        let mut strategy = MarketBuyAndCloseStrategy::new(cfg);
        let ctx = ctx(TradeMode::Live);
        let bar = BarEvent {
            symbol: "SBER".to_string(),
            close_time_utc: 120,
            close: 101.0,
            o: 100.5,
            h: 101.5,
            l: 100.0,
            v: 1.0,
            origin: DataOrigin::Live,
        };

        strategy.state = StrategyState::MarketLiveInPosition {
            side: Side::Buy,
            qty: 1.0,
            avg_price: 100.0,
            baseline_qty: 0.0,
            close_trigger: CloseTrigger::NextBar,
            opened_ts: 100,
            last_bar_ts: 100,
        };

        let intents = strategy.on_bar(&ctx, &bar);
        assert!(matches!(
            intents.as_slice(),
            [Intent::Place {
                price,
                qty,
                side: Side::Sell,
                ..
            }] if (*qty - 1.0).abs() <= f64::EPSILON && (*price - 100.99).abs() <= 1e-9
        ));
        let expected_request_id = crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &bar.symbol,
            "place",
            ctx.event_ts_utc(),
            0,
        );
        assert!(matches!(
            strategy.state(),
            StrategyState::MarketLivePendingExit { request_guid, .. } if *request_guid == expected_request_id
        ));
    }

    #[test]
    fn ack_matches_pending_sets_acked_true() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let ctx = ctx(TradeMode::Live);
        let request_id = crate::deterministic_market_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ctx.symbol,
            1_000,
            Side::Buy,
        );
        strategy.state = StrategyState::MarketLivePendingEntry {
            request_guid: request_id,
            side: Side::Buy,
            qty: 1.0,
            baseline_qty: 0.0,
            close_trigger: CloseTrigger::NextBar,
            sent_ts: 1_000,
            acked: false,
            entry_confirmed_ts: None,
            last_bar_ts: 1_000,
        };

        let ack = CommandAck::accepted(request_id);
        let _ = strategy.on_ack(&ctx, &ack);

        assert!(matches!(
            strategy.state(),
            StrategyState::MarketLivePendingEntry { acked: true, .. }
        ));
    }

    #[test]
    fn reject_blocks_strategy_with_reason() {
        let mut strategy = MarketBuyAndCloseStrategy::new(config());
        let ctx = ctx(TradeMode::Live);
        let request_id = crate::deterministic_market_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ctx.symbol,
            2_000,
            Side::Buy,
        );
        strategy.state = StrategyState::MarketLivePendingEntry {
            request_guid: request_id,
            side: Side::Buy,
            qty: 1.0,
            baseline_qty: 0.0,
            close_trigger: CloseTrigger::NextBar,
            sent_ts: 2_000,
            acked: false,
            entry_confirmed_ts: None,
            last_bar_ts: 2_000,
        };

        let ack = CommandAck {
            request_id,
            status: AckStatus::Rejected,
            cws_message: None,
            error_code: Some("rejected".to_string()),
            error_msg: Some("broker rejected".to_string()),
            cws_http_code: None,
            cws_request_guid: None,
            broker_order_id: None,
            broker_order_id_str: None,
            processed_ts_utc: 2_001,
        };
        let _ = strategy.on_ack(&ctx, &ack);

        assert!(matches!(
            strategy.state(),
            StrategyState::Blocked { reason, .. } if reason.contains("entry_rejected")
        ));
    }
}
