use alor_protocol::{AckStatus, CommandAck, Side};
use tracing::{info, warn};
use uuid::Uuid;

use crate::live_guard::GatewayPhase;
use crate::state::StrategyState;
use crate::{
    BarEvent, DataOrigin, Intent, OrderEvent, PositionEvent, Strategy, StrategyCtx, TradeMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockLiveProbeMode {
    PlaceMarketOnce,
    PlaceLimitBadPrice,
    PlaceLimitBadStep,
    CancelAfterTerminal,
}

impl MockLiveProbeMode {
    pub fn parse(strategy_id: &str) -> Self {
        let normalized = strategy_id.to_ascii_lowercase();
        let suffix = normalized
            .rsplit(['.', ':', '/', '-'])
            .next()
            .unwrap_or(normalized.as_str());
        match suffix {
            "place_limit_bad_price" | "bad_price" => Self::PlaceLimitBadPrice,
            "place_limit_bad_step" | "bad_step" => Self::PlaceLimitBadStep,
            "cancel_after_terminal" | "terminal_cancel" => Self::CancelAfterTerminal,
            _ => Self::PlaceMarketOnce,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::PlaceMarketOnce => "place_market_once",
            Self::PlaceLimitBadPrice => "place_limit_bad_price",
            Self::PlaceLimitBadStep => "place_limit_bad_step",
            Self::CancelAfterTerminal => "cancel_after_terminal",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MockLiveProbeConfig {
    pub symbol: String,
    pub qty: f64,
    pub side: Side,
    pub tick_size: f64,
    pub offset_ticks: i64,
    pub trigger_after_live_bars: u32,
    pub mode: MockLiveProbeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbePhase {
    Idle,
    WaitingAck,
    WaitingTerminalThenCancel,
    CancelSent,
    Done,
    Blocked,
}

#[derive(Debug)]
pub struct MockLiveProbeStrategy {
    config: MockLiveProbeConfig,
    state: StrategyState,
    phase: ProbePhase,
    last_processed_bar_ts: Option<i64>,
    live_bars_seen: u32,
    primary_request_id: Option<Uuid>,
    cancel_request_id: Option<Uuid>,
    broker_order_id: Option<i64>,
}

impl MockLiveProbeStrategy {
    pub fn new(config: MockLiveProbeConfig) -> Self {
        Self {
            config,
            state: StrategyState::Idle,
            phase: ProbePhase::Idle,
            last_processed_bar_ts: None,
            live_bars_seen: 0,
            primary_request_id: None,
            cancel_request_id: None,
            broker_order_id: None,
        }
    }

    fn should_skip_live_bar(&self, ctx: &StrategyCtx, bar: &BarEvent) -> bool {
        ctx.trade_mode == TradeMode::Live
            && (bar.origin != DataOrigin::Live
                || !ctx.allow_live_orders
                || ctx.gateway_phase != GatewayPhase::LiveReady)
    }

    fn compute_limit_price(&self, bar: &BarEvent) -> f64 {
        match self.config.mode {
            MockLiveProbeMode::PlaceLimitBadPrice => {
                let base = bar.close.abs().max(self.config.tick_size.max(0.01));
                match self.config.side {
                    Side::Buy => base * 100.0 + 1_000_000.0,
                    Side::Sell => (base * 0.01).max(0.01),
                }
            }
            MockLiveProbeMode::PlaceLimitBadStep => {
                let half_step = if self.config.tick_size > 0.0 {
                    self.config.tick_size / 2.0
                } else {
                    0.000_123
                };
                let dir = match self.config.side {
                    Side::Buy => 1.0,
                    Side::Sell => -1.0,
                };
                (bar.close + dir * half_step).max(0.01)
            }
            _ => {
                let offset = self.config.offset_ticks as f64 * self.config.tick_size;
                match self.config.side {
                    Side::Buy => (bar.close - offset).max(0.01),
                    Side::Sell => (bar.close + offset).max(0.01),
                }
            }
        }
    }

    fn set_done(&mut self, ts: i64) {
        self.phase = ProbePhase::Done;
        self.state = StrategyState::Done { last_bar_ts: ts };
    }

    fn set_blocked(&mut self, reason: String, ts: i64) {
        self.phase = ProbePhase::Blocked;
        self.state = StrategyState::Blocked {
            reason,
            last_bar_ts: ts,
        };
    }

    fn primary_request_id_for_bar(&self, ctx: &StrategyCtx, bar: &BarEvent) -> Uuid {
        match self.config.mode {
            MockLiveProbeMode::PlaceMarketOnce | MockLiveProbeMode::CancelAfterTerminal => {
                crate::deterministic_market_request_id(
                    &ctx.strategy_id,
                    &ctx.portfolio,
                    &bar.symbol,
                    bar.close_time_utc,
                    self.config.side,
                )
            }
            MockLiveProbeMode::PlaceLimitBadPrice | MockLiveProbeMode::PlaceLimitBadStep => {
                crate::deterministic_request_id(
                    &ctx.strategy_id,
                    &ctx.portfolio,
                    &bar.symbol,
                    "place",
                    bar.close_time_utc,
                    0,
                )
            }
        }
    }

    fn emit_primary_intent(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        let request_id = self.primary_request_id_for_bar(ctx, bar);
        self.primary_request_id = Some(request_id);
        match self.config.mode {
            MockLiveProbeMode::PlaceMarketOnce => {
                self.phase = ProbePhase::WaitingAck;
                self.state = StrategyState::MarketLivePendingEntry {
                    request_guid: request_id,
                    side: self.config.side,
                    qty: self.config.qty,
                    baseline_qty: ctx.position_qty.unwrap_or(0.0),
                    close_trigger: crate::CloseTrigger::PositionUpdate,
                    sent_ts: bar.close_time_utc,
                    acked: false,
                    entry_confirmed_ts: None,
                    last_bar_ts: bar.close_time_utc,
                };
                vec![Intent::Market {
                    qty: self.config.qty,
                    side: self.config.side,
                    fill_price: None,
                }]
            }
            MockLiveProbeMode::CancelAfterTerminal => {
                self.phase = ProbePhase::WaitingTerminalThenCancel;
                self.state = StrategyState::MarketLivePendingEntry {
                    request_guid: request_id,
                    side: self.config.side,
                    qty: self.config.qty,
                    baseline_qty: ctx.position_qty.unwrap_or(0.0),
                    close_trigger: crate::CloseTrigger::PositionUpdate,
                    sent_ts: bar.close_time_utc,
                    acked: false,
                    entry_confirmed_ts: None,
                    last_bar_ts: bar.close_time_utc,
                };
                vec![Intent::Market {
                    qty: self.config.qty,
                    side: self.config.side,
                    fill_price: None,
                }]
            }
            MockLiveProbeMode::PlaceLimitBadPrice | MockLiveProbeMode::PlaceLimitBadStep => {
                let price = self.compute_limit_price(bar);
                self.phase = ProbePhase::WaitingAck;
                self.state = StrategyState::Placed {
                    place_request_id: request_id,
                    order_id: None,
                    cancel_due: false,
                    cancel_bar_ts: None,
                    placed_bar_ts: bar.close_time_utc,
                    last_bar_ts: bar.close_time_utc,
                    bars_waited: 0,
                };
                vec![Intent::Place {
                    price,
                    qty: self.config.qty,
                    side: self.config.side,
                }]
            }
        }
    }

    fn terminal_order_status(status: &str) -> bool {
        matches!(
            status.trim().to_ascii_lowercase().as_str(),
            "filled" | "canceled" | "cancelled" | "rejected" | "expired"
        )
    }

    fn reject_reason(ack: &CommandAck) -> String {
        let code = ack.error_code.as_deref().unwrap_or("unknown_code");
        let msg = ack.error_msg.as_deref().unwrap_or("unknown_error");
        let cws_http = ack.cws_http_code.unwrap_or_default();
        let cws_msg = ack.cws_message.as_deref().unwrap_or("");
        format!("ack_rejected code={code} msg={msg} cws_http={cws_http} cws_msg={cws_msg}")
    }
}

impl Strategy for MockLiveProbeStrategy {
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

        if self.should_skip_live_bar(ctx, bar) {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        }

        if ctx.trade_mode != TradeMode::Live {
            self.last_processed_bar_ts = Some(bar.close_time_utc);
            return Vec::new();
        }

        if bar.origin == DataOrigin::Live {
            self.live_bars_seen = self.live_bars_seen.saturating_add(1);
        }

        let should_fire = self.phase == ProbePhase::Idle
            && self.live_bars_seen >= self.config.trigger_after_live_bars.max(1);

        let intents = if should_fire {
            info!(
                strategy = "mock_live_probe",
                mode = self.config.mode.as_str(),
                live_bars_seen = self.live_bars_seen,
                "probe_emitting_intent"
            );
            self.emit_primary_intent(ctx, bar)
        } else {
            Vec::new()
        };

        self.last_processed_bar_ts = Some(bar.close_time_utc);
        intents
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, ack: &CommandAck) -> Vec<Intent> {
        if self.cancel_request_id == Some(ack.request_id) {
            match ack.status {
                AckStatus::Accepted | AckStatus::Confirmed | AckStatus::Duplicate => {
                    self.set_done(ack.processed_ts_utc);
                }
                AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                    self.set_blocked(Self::reject_reason(ack), ack.processed_ts_utc);
                }
            }
            return Vec::new();
        }

        if self.primary_request_id != Some(ack.request_id) {
            return Vec::new();
        }

        if let Some(order_id) = ack.broker_order_id {
            self.broker_order_id = Some(order_id);
        }

        match ack.status {
            AckStatus::Accepted | AckStatus::Confirmed | AckStatus::Duplicate => {
                if matches!(self.phase, ProbePhase::WaitingAck)
                    && self.config.mode != MockLiveProbeMode::CancelAfterTerminal
                {
                    self.set_done(ack.processed_ts_utc);
                }
            }
            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error => {
                self.set_blocked(Self::reject_reason(ack), ack.processed_ts_utc);
            }
        }

        Vec::new()
    }

    fn on_order(&mut self, ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent> {
        if self.config.mode != MockLiveProbeMode::CancelAfterTerminal
            || self.phase != ProbePhase::WaitingTerminalThenCancel
        {
            return Vec::new();
        }

        let request_matches = ord
            .request_id
            .zip(self.primary_request_id)
            .is_some_and(|(lhs, rhs)| lhs == rhs);
        let order_matches = self.broker_order_id.is_some_and(|id| id == ord.order_id);

        if !request_matches && !order_matches {
            return Vec::new();
        }

        if !Self::terminal_order_status(&ord.status) {
            if let StrategyState::MarketLivePendingEntry { acked, .. } = &mut self.state {
                *acked = true;
            }
            return Vec::new();
        }

        let cancel_request_id = crate::deterministic_request_id(
            &ctx.strategy_id,
            &ctx.portfolio,
            &ord.symbol,
            "cancel",
            ord.ts_utc,
            1,
        );
        self.cancel_request_id = Some(cancel_request_id);
        self.phase = ProbePhase::CancelSent;
        self.state = StrategyState::CancelSent {
            cancel_request_id,
            order_id: ord.order_id,
            last_bar_ts: ord.ts_utc,
        };
        warn!(
            strategy = "mock_live_probe",
            mode = self.config.mode.as_str(),
            order_id = ord.order_id,
            status = %ord.status,
            "probe_terminal_order_seen_emitting_cancel"
        );
        vec![Intent::Cancel {
            order_id: ord.order_id,
        }]
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        self.state = state;
        self.phase = match self.state {
            StrategyState::Done { .. } => ProbePhase::Done,
            StrategyState::Blocked { .. } => ProbePhase::Blocked,
            _ => ProbePhase::Idle,
        };
        self.primary_request_id = None;
        self.cancel_request_id = None;
        self.broker_order_id = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StrategyCtx, TradeMode};

    fn ctx() -> StrategyCtx {
        StrategyCtx {
            strategy_id: "mock_live_probe.place_limit_bad_step".to_string(),
            portfolio: "p".to_string(),
            exchange: "MOEX".to_string(),
            symbol: "IMOEXF".to_string(),
            tick_size: 0.01,
            trade_mode: TradeMode::Live,
            allow_live_orders: true,
            gateway_phase: GatewayPhase::LiveReady,
            position_qty: None,
            last_bar_ts: None,
        }
    }

    fn bar(ts: i64) -> BarEvent {
        BarEvent {
            symbol: "IMOEXF".to_string(),
            close_time_utc: ts,
            close: 100.0,
            o: 100.0,
            h: 100.0,
            l: 100.0,
            v: 1.0,
            origin: DataOrigin::Live,
        }
    }

    #[test]
    fn parses_mode_from_strategy_id_suffix() {
        assert_eq!(
            MockLiveProbeMode::parse("mock_live_probe.place_limit_bad_price"),
            MockLiveProbeMode::PlaceLimitBadPrice
        );
        assert_eq!(
            MockLiveProbeMode::parse("mock_live_probe:bad_step"),
            MockLiveProbeMode::PlaceLimitBadStep
        );
        assert_eq!(
            MockLiveProbeMode::parse("mock_live_probe-terminal_cancel"),
            MockLiveProbeMode::CancelAfterTerminal
        );
    }

    #[test]
    fn emits_bad_step_limit_once_on_first_live_bar() {
        let mut s = MockLiveProbeStrategy::new(MockLiveProbeConfig {
            symbol: "IMOEXF".to_string(),
            qty: 1.0,
            side: Side::Buy,
            tick_size: 0.01,
            offset_ticks: 0,
            trigger_after_live_bars: 1,
            mode: MockLiveProbeMode::PlaceLimitBadStep,
        });
        let intents = s.on_bar(&ctx(), &bar(1_700_000_000));
        assert!(matches!(intents.as_slice(), [Intent::Place { .. }]));
        let second = s.on_bar(&ctx(), &bar(1_700_000_060));
        assert!(second.is_empty());
    }
}
