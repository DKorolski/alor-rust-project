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
    cycle_id: [u8; 10],
}

#[derive(Debug, Clone)]
struct HybridTag {
    sid: String,
    cycle: String,
}

#[derive(Debug, Clone, Copy)]
enum TagRole {
    Entry,
    Exit,
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
    active_cycle_id: Option<[u8; 10]>,
    next_cycle_seq: u32,
    last_bar_close: Option<f64>,
    last_day_local: Option<NaiveDate>,
    current_day_high: Option<f64>,
    current_day_low: Option<f64>,
    prev_day_range: Option<f64>,
    entry_ready: bool,
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
            active_cycle_id: None,
            next_cycle_seq: 0,
            last_bar_close: None,
            last_day_local: None,
            current_day_high: None,
            current_day_low: None,
            prev_day_range: None,
            entry_ready: false,
            working_orders: HashSet::new(),
            working_stop_orders: HashSet::new(),
        }
    }

    fn utc_to_local_naive(&self, ts_utc: i64) -> Option<NaiveDateTime> {
        let offset = FixedOffset::east_opt(self.config.timezone_offset_hours.saturating_mul(3600))?;
        chrono::DateTime::from_timestamp(ts_utc, 0)
            .map(|dt| dt.with_timezone(&offset).naive_local())
    }

    fn has_live_orders(&self) -> bool {
        !self.working_orders.is_empty() || !self.working_stop_orders.is_empty()
    }

    fn owner_code(owner: Owner) -> &'static str {
        match owner {
            Owner::MeanReversion => "MR",
            Owner::IntradayBreakout => "BO",
        }
    }

    fn role_code(role: TagRole) -> &'static str {
        match role {
            TagRole::Entry => "ENTRY",
            TagRole::Exit => "EXIT",
        }
    }

    fn format_cycle_id(cycle: &[u8; 10]) -> String {
        let mut out = String::with_capacity(10);
        for b in cycle {
            out.push(*b as char);
        }
        out
    }

    fn parse_cycle_id(raw: &str) -> Option<[u8; 10]> {
        if raw.len() != 10 || !raw.is_ascii() {
            return None;
        }
        let mut out = [b'0'; 10];
        for (i, b) in raw.as_bytes().iter().enumerate() {
            if !(*b as char).is_ascii_hexdigit() {
                return None;
            }
            out[i] = *b;
        }
        Some(out)
    }

    fn next_cycle_id(&mut self, ts_utc: i64) -> [u8; 10] {
        let ts = (ts_utc.max(0) as u64) & 0xffff_ffff;
        let seq = self.next_cycle_seq & 0xff;
        self.next_cycle_seq = self.next_cycle_seq.wrapping_add(1);
        let value = format!("{ts:08x}{seq:02x}");
        let mut out = [b'0'; 10];
        out.copy_from_slice(value.as_bytes());
        out
    }

    fn parse_hybrid_tag(comment: Option<&str>) -> Option<HybridTag> {
        let comment = comment?;
        if !comment.is_ascii() || !comment.starts_with("HYB|") {
            return None;
        }
        let mut sid = None;
        let mut cycle = None;
        for part in comment.split('|').skip(1) {
            let (key, value) = part.split_once('=')?;
            match key {
                "sid" => sid = Some(value.to_string()),
                "c" => cycle = Some(value.to_string()),
                _ => {}
            }
        }
        Some(HybridTag {
            sid: sid?,
            cycle: cycle?,
        })
    }

    fn is_our_tag(&self, ctx: &StrategyCtx, comment: Option<&str>) -> bool {
        let Some(tag) = Self::parse_hybrid_tag(comment) else {
            return false;
        };
        tag.sid == ctx.strategy_id
    }

    fn ensure_active_cycle_from_comment(&mut self, comment: Option<&str>) {
        if self.active_cycle_id.is_some() {
            return;
        }
        let Some(tag) = Self::parse_hybrid_tag(comment) else {
            return;
        };
        if let Some(cycle) = Self::parse_cycle_id(&tag.cycle) {
            self.active_cycle_id = Some(cycle);
        }
    }

    fn build_comment(
        &self,
        ctx: &StrategyCtx,
        cycle_id: &[u8; 10],
        owner: Owner,
        role: TagRole,
    ) -> String {
        let cycle = Self::format_cycle_id(cycle_id);
        let owner = Self::owner_code(owner);
        let role = Self::role_code(role);
        // HYB|sid=<sid>|c=<cycle>|o=<MR|BO>|r=<ENTRY|EXIT>
        format!("HYB|sid={}|c={cycle}|o={owner}|r={role}", ctx.strategy_id)
    }

    fn infer_owner_for_recovered_position(pos_qty: f64) -> (Owner, Side) {
        let side = if pos_qty >= 0.0 {
            Side::Long
        } else {
            Side::Short
        };
        (Owner::MeanReversion, side)
    }

    fn sync_state(&mut self) {
        self.state = StrategyState::HybridIntradayRuntime {
            active_cycle_id: self.active_cycle_id.map(|v| Self::format_cycle_id(&v)),
            next_cycle_seq: self.next_cycle_seq,
            last_position_qty: self.last_position_qty,
            current_owner: self.current_owner,
            current_side: self.current_side,
            pending_entry_owner: self.pending_entry.map(|v| v.owner),
            pending_entry_side: self.pending_entry.map(|v| v.side),
            pending_entry_cycle_id: self
                .pending_entry
                .as_ref()
                .map(|v| Self::format_cycle_id(&v.cycle_id)),
            entry_ready: self.entry_ready,
        };
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
        self.entry_ready = self.prev_day_range.is_some();
    }

    fn map_action_to_intents(
        &mut self,
        ctx: &StrategyCtx,
        created_ts_utc: i64,
        action: Action,
    ) -> Vec<Intent> {
        match action {
            Action::SubmitEntry(entry) => {
                if !self.entry_ready {
                    return Vec::new();
                }
                let cycle_id = self.next_cycle_id(created_ts_utc);
                self.pending_entry = Some(PendingEntry {
                    owner: entry.owner,
                    side: entry.side,
                    cycle_id,
                });
                self.active_cycle_id = Some(cycle_id);
                let comment = self.build_comment(ctx, &cycle_id, entry.owner, TagRole::Entry);
                self.sync_state();
                vec![Intent::Market {
                    qty: self.config.qty.max(1.0),
                    side: Self::side_to_order_side(entry.side),
                    fill_price: None,
                    comment: Some(comment),
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
                let cycle_id = self
                    .active_cycle_id
                    .unwrap_or_else(|| self.next_cycle_id(created_ts_utc));
                self.active_cycle_id = Some(cycle_id);
                self.current_owner = Some(owner);
                let comment = self.build_comment(ctx, &cycle_id, owner, TagRole::Exit);
                self.sync_state();
                vec![Intent::Market {
                    qty,
                    side,
                    fill_price: None,
                    comment: Some(comment),
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
            1,
            Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason: crate::strategies::hybrid_intraday::ReasonCode::MeanRevTimeCutoff,
            },
        );

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            Intent::Classified {
                intent,
                intent_class,
            } => {
                assert_eq!(*intent_class, IntentClass::Exit);
                match intent.as_ref() {
                    Intent::Market {
                        qty, side, comment, ..
                    } => {
                        assert!((*qty - 1.0).abs() <= f64::EPSILON);
                        assert_eq!(*side, OrderSide::Sell);
                        let comment = comment.clone().unwrap_or_default();
                        assert!(comment.contains("HYB|sid=hyb-test|"));
                        assert!(comment.contains("|o=MR|"));
                        assert!(comment.contains("|r=EXIT"));
                    }
                    other => panic!("unexpected base intent: {other:?}"),
                }
            }
            other => panic!("expected classified intent, got {other:?}"),
        }
    }

    #[test]
    fn warmup_blocks_entry_until_prev_day_range_ready() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(HybridIntradayRuntimeConfig {
            symbol: "IMOEXF".to_string(),
            qty: 1.0,
            timezone_offset_hours: 3,
        });
        let ctx = test_ctx(Some(0.0));
        let entry_action = Action::SubmitEntry(crate::strategies::hybrid_intraday::EntrySignal {
            owner: Owner::MeanReversion,
            side: Side::Long,
            entry_style: crate::strategies::hybrid_intraday::EntryStyle::Market,
            reason: crate::strategies::hybrid_intraday::ReasonCode::MorningMeanReversionLong,
            stop_price: None,
            take_price: None,
        });

        let intents_blocked = strategy.map_action_to_intents(&ctx, 1, entry_action.clone());
        assert!(intents_blocked.is_empty());

        strategy.entry_ready = true;
        let intents_ready = strategy.map_action_to_intents(&ctx, 2, entry_action);
        assert_eq!(intents_ready.len(), 1);
        match &intents_ready[0] {
            Intent::Classified { intent, .. } => match intent.as_ref() {
                Intent::Market { comment, .. } => {
                    let comment = comment.clone().unwrap_or_default();
                    assert!(comment.contains("HYB|sid=hyb-test|"));
                    assert!(comment.contains("|o=MR|"));
                    assert!(comment.contains("|r=ENTRY"));
                }
                other => panic!("unexpected base intent: {other:?}"),
            },
            other => panic!("expected classified intent, got {other:?}"),
        }
    }

    #[test]
    fn ignores_foreign_working_orders_for_live_order_gate() {
        let mut strategy = HybridIntradayRuntimeStrategy::new(HybridIntradayRuntimeConfig {
            symbol: "IMOEXF".to_string(),
            qty: 1.0,
            timezone_offset_hours: 3,
        });
        let ctx = test_ctx(Some(0.0));
        let _ = strategy.on_order(
            &ctx,
            &OrderEvent {
                order_id: 42,
                request_id: None,
                symbol: "IMOEXF".to_string(),
                status: "working".to_string(),
                side: "buy".to_string(),
                order_type: "limit".to_string(),
                qty: 1.0,
                filled: 0.0,
                price: 100.0,
                existing: false,
                comment: None,
                ts_utc: 1,
            },
        );
        assert!(!strategy.has_live_orders());
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
        let actions =
            self.orchestrator
                .on_bar(crate::strategies::hybrid_intraday::orchestrator::BarInput {
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
        self.sync_state();
        actions
            .into_iter()
            .flat_map(|action| self.map_action_to_intents(ctx, bar.close_time_utc, action))
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
            self.active_cycle_id = None;
            self.sync_state();
        }
        Vec::new()
    }

    fn on_order(&mut self, ctx: &StrategyCtx, ord: &OrderEvent) -> Vec<Intent> {
        if ord.symbol != self.config.symbol {
            return Vec::new();
        }
        let is_ours = self.is_our_tag(ctx, ord.comment.as_deref());
        if !is_ours {
            return Vec::new();
        }
        self.ensure_active_cycle_from_comment(ord.comment.as_deref());
        let status = ord.status.to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "filled" | "canceled" | "cancelled" | "expired" | "rejected"
        ) {
            self.working_orders.remove(&ord.order_id);
        } else if ord.order_id > 0 {
            self.working_orders.insert(ord.order_id);
        }
        self.sync_state();
        Vec::new()
    }

    fn on_stop_order(&mut self, ctx: &StrategyCtx, ord: &StopOrderEvent) -> Vec<Intent> {
        if ord.symbol != self.config.symbol {
            return Vec::new();
        }
        let is_ours = self.is_our_tag(ctx, ord.comment.as_deref());
        if !is_ours {
            return Vec::new();
        }
        self.ensure_active_cycle_from_comment(ord.comment.as_deref());
        let status = ord.status.to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "filled"
                | "canceled"
                | "cancelled"
                | "expired"
                | "rejected"
                | "executed"
                | "triggered"
                | "done"
                | "completed"
        ) {
            self.working_stop_orders.remove(&ord.stop_order_id);
        } else if !ord.stop_order_id.trim().is_empty() {
            self.working_stop_orders.insert(ord.stop_order_id.clone());
        }
        self.sync_state();
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
                self.active_cycle_id = Some(entry.cycle_id);
                self.orchestrator
                    .on_order_filled("entry", entry.owner, Some(entry.side));
            } else {
                let (owner, side) = Self::infer_owner_for_recovered_position(cur);
                self.current_owner = Some(owner);
                self.current_side = Some(side);
                if self.active_cycle_id.is_none() {
                    self.active_cycle_id = Some(self.next_cycle_id(pos.ts_utc));
                }
            }
        } else if prev.abs() > f64::EPSILON && cur.abs() <= f64::EPSILON {
            let owner = self.current_owner.unwrap_or(Owner::MeanReversion);
            self.orchestrator.on_order_filled("exit", owner, None);
            self.current_owner = None;
            self.current_side = None;
            self.pending_entry = None;
            self.active_cycle_id = None;
        }
        self.last_position_qty = cur;
        self.sync_state();
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        ctx: &StrategyCtx,
        snapshot: &crate::BootstrapSnapshot,
    ) -> Vec<Intent> {
        self.working_orders.clear();
        self.working_stop_orders.clear();
        for (order_id, order) in &snapshot.working_orders_strategy {
            if order.symbol != self.config.symbol {
                continue;
            }
            if self.is_our_tag(ctx, order.comment.as_deref()) {
                self.working_orders.insert(*order_id);
                self.ensure_active_cycle_from_comment(order.comment.as_deref());
            }
        }
        for (stop_order_id, stop_order) in &snapshot.working_stop_orders_strategy {
            if stop_order.symbol != self.config.symbol {
                continue;
            }
            if self.is_our_tag(ctx, stop_order.comment.as_deref()) {
                self.working_stop_orders.insert(stop_order_id.clone());
                self.ensure_active_cycle_from_comment(stop_order.comment.as_deref());
            }
        }
        if let Some(position) = snapshot.positions_strategy.get(&self.config.symbol) {
            self.last_position_qty = position.qty;
        }
        self.sync_state();
        Vec::new()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        if let StrategyState::HybridIntradayRuntime {
            active_cycle_id,
            next_cycle_seq,
            last_position_qty,
            current_owner,
            current_side,
            pending_entry_owner,
            pending_entry_side,
            pending_entry_cycle_id,
            entry_ready,
        } = &state
        {
            self.active_cycle_id = active_cycle_id.as_deref().and_then(Self::parse_cycle_id);
            self.next_cycle_seq = *next_cycle_seq;
            self.last_position_qty = *last_position_qty;
            self.current_owner = *current_owner;
            self.current_side = *current_side;
            self.pending_entry = match (
                *pending_entry_owner,
                *pending_entry_side,
                pending_entry_cycle_id
                    .as_deref()
                    .and_then(Self::parse_cycle_id),
            ) {
                (Some(owner), Some(side), Some(cycle_id)) => Some(PendingEntry {
                    owner,
                    side,
                    cycle_id,
                }),
                _ => None,
            };
            self.entry_ready = *entry_ready;
        }
        self.state = state;
    }
}
