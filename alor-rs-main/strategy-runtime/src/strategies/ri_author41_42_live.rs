use std::collections::{BTreeMap, HashSet};

use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state::StrategyState;
use crate::strategies::moex_author41_42::{
    build_ri_author41_42_combo_shadow_journal, Author41Config, Author42Config, Author42SideMode,
    Component, ModelBar, ModelProfile, OverlapDecision, RegularSessionPolicy, ShadowJournalRecord,
    ShadowSide,
};
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
    RuntimeStateRestored, Strategy, StrategyCtx,
};
use alor_protocol::{AckStatus, IntentClass, Side as OrderSide};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142RuntimeMode {
    Shadow,
    DryRun,
    MicroLive,
}

impl RiAuthor4142RuntimeMode {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "shadow" | "shadow_only" => Ok(Self::Shadow),
            "dry_run" | "dryrun" => Ok(Self::DryRun),
            "micro_live" | "live_micro" => Ok(Self::MicroLive),
            other => bail!("unsupported ri_author41_42 mode: {other}"),
        }
    }

    pub fn can_emit_orders(self) -> bool {
        matches!(self, Self::MicroLive)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::DryRun => "dry_run",
            Self::MicroLive => "micro_live",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142ExecutionPath {
    ActionScopedOnly,
}

impl RiAuthor4142ExecutionPath {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "action_scoped_only" | "action-scoped-only" => Ok(Self::ActionScopedOnly),
            other => bail!("unsupported ri_author41_42 execution_path: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActionScopedOnly => "action_scoped_only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiAuthor4142LiveConfig {
    pub symbol: String,
    pub profile_id: String,
    pub timeframe: String,
    pub mode: RiAuthor4142RuntimeMode,
    pub allow_order_emission: bool,
    pub execution_path: RiAuthor4142ExecutionPath,
    pub order_symbol: Option<String>,
    pub qty: f64,
    pub timezone_offset_hours: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142DecisionAction {
    Enter,
    Suppress,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142Phase {
    Flat,
    DryRunInPosition,
    ManualInterventionRequired,
}

impl RiAuthor4142Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::DryRunInPosition => "dry_run_in_position",
            Self::ManualInterventionRequired => "manual_intervention_required",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142Component {
    Author41Mr,
    Author42Bo,
}

impl RiAuthor4142Component {
    fn from_shadow(component: Component) -> Option<Self> {
        match component {
            Component::Author41Mr => Some(Self::Author41Mr),
            Component::Author42Bo => Some(Self::Author42Bo),
            Component::Combo => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Author41Mr => "author41_mr",
            Self::Author42Bo => "author42_bo",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142Side {
    Long,
    Short,
}

impl RiAuthor4142Side {
    fn from_shadow(side: ShadowSide) -> Self {
        match side {
            ShadowSide::Long => Self::Long,
            ShadowSide::Short => Self::Short,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }

    fn entry_order_side(self) -> OrderSide {
        match self {
            Self::Long => OrderSide::Buy,
            Self::Short => OrderSide::Sell,
        }
    }

    fn exit_order_side(self) -> OrderSide {
        match self {
            Self::Long => OrderSide::Sell,
            Self::Short => OrderSide::Buy,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiAuthor4142ModelDecision {
    pub action: RiAuthor4142DecisionAction,
    pub component: RiAuthor4142Component,
    pub side: Option<RiAuthor4142Side>,
    pub model_signal_ts_local: NaiveDateTime,
    pub scheduled_entry_ts_local: Option<NaiveDateTime>,
    pub scheduled_exit_ts_local: Option<NaiveDateTime>,
    pub reason: String,
    pub overlap_decision: String,
    pub shadow_pnl_points: Option<f64>,
    pub decision_key: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142CandidateRole {
    Entry,
    Exit,
}

impl RiAuthor4142CandidateRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Exit => "exit",
        }
    }

    fn intent_class(self) -> IntentClass {
        match self {
            Self::Entry => IntentClass::Entry,
            Self::Exit => IntentClass::Exit,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiAuthor4142CandidateIntent {
    pub role: RiAuthor4142CandidateRole,
    pub component: RiAuthor4142Component,
    pub side: OrderSide,
    pub qty: f64,
    pub order_style: &'static str,
    pub intent_class: IntentClass,
    pub scheduled_ts_local: NaiveDateTime,
    pub comment: String,
    pub execution_path: RiAuthor4142ExecutionPath,
    pub decision_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RiDailyStats {
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RiDailyAnchor {
    prev_close: f64,
    prev_low: f64,
    prev_range: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct RiAuthor4142LiveMrPosition {
    side: RiAuthor4142Side,
    entry_ts_local: NaiveDateTime,
    entry_price: f64,
    prev_close: f64,
    prev_range: f64,
    bars_held: u32,
    config: Author41Config,
    decision_key: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct RiAuthor4142LiveMrState {
    current_date: Option<NaiveDate>,
    entries_today: u32,
    position: Option<RiAuthor4142LiveMrPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RiAuthor4142LiveBoContext {
    prev_close: f64,
    prev2_close: f64,
    prev_range: f64,
    prev_hl_ratio: f64,
    prev_ret: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct RiAuthor4142LiveBoPosition {
    side: RiAuthor4142Side,
    entry_ts_local: NaiveDateTime,
    entry_price: f64,
    bars_held: u32,
    decision_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RiAuthor4142LiveBoPending {
    Entry(RiAuthor4142Side),
    Exit(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
struct RiAuthor4142LiveDecisionInput {
    component: RiAuthor4142Component,
    side: RiAuthor4142Side,
    model_signal_ts_local: NaiveDateTime,
    scheduled_entry_ts_local: Option<NaiveDateTime>,
    scheduled_exit_ts_local: Option<NaiveDateTime>,
    reason: &'static str,
    decision_key: String,
    shadow_pnl_points: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct RiAuthor4142LiveBoState {
    current_date: Option<NaiveDate>,
    context: Option<RiAuthor4142LiveBoContext>,
    first_bar: Option<ModelBar>,
    bar_index: usize,
    long_level: Option<f64>,
    short_level: Option<f64>,
    trade_allowed: bool,
    was_long_today: bool,
    was_short_today: bool,
    day_hh: f64,
    day_ll: f64,
    position: Option<RiAuthor4142LiveBoPosition>,
    pending: Option<RiAuthor4142LiveBoPending>,
}

impl Default for RiAuthor4142LiveBoState {
    fn default() -> Self {
        Self {
            current_date: None,
            context: None,
            first_bar: None,
            bar_index: 0,
            long_level: None,
            short_level: None,
            trade_allowed: true,
            was_long_today: false,
            was_short_today: false,
            day_hh: f64::NEG_INFINITY,
            day_ll: f64::INFINITY,
            position: None,
            pending: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142JournalDecision {
    ShadowRecorded,
    IntentSuppressed,
    IntentEmitted,
    ManualInterventionRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiAuthor4142JournalRecord {
    pub component: String,
    pub cycle_id: String,
    pub model_signal_ts_local: String,
    pub bar_ts_local: String,
    pub side: Option<String>,
    pub role: Option<String>,
    pub entry_exit_reason: String,
    pub no_overlap_decision: String,
    pub adapter_decision: RiAuthor4142JournalDecision,
    pub request_id: Option<String>,
    pub broker_order_id: Option<String>,
    pub position_before: Option<f64>,
    pub position_after: Option<f64>,
    pub candidate_order_side: Option<OrderSide>,
    pub candidate_qty: Option<f64>,
    pub candidate_order_style: Option<String>,
    pub candidate_intent_class: Option<IntentClass>,
    pub execution_path: String,
    pub decision_key: String,
}

impl RiAuthor4142LiveConfig {
    pub fn can_emit_orders(&self) -> bool {
        self.mode.can_emit_orders() && self.allow_order_emission
    }

    pub fn validate(&self) -> Result<()> {
        if self.execution_path != RiAuthor4142ExecutionPath::ActionScopedOnly {
            bail!("ri_author41_42 requires action_scoped_only execution");
        }
        if self.mode.can_emit_orders() != self.allow_order_emission {
            bail!(
                "ri_author41_42 requires mode=micro_live and allow_order_emission=true to be paired"
            );
        }
        if self.timeframe.trim() != "10m" {
            bail!(
                "ri_author41_42 frozen model requires 10m timeframe, got {}",
                self.timeframe
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RiAuthor4142LiveStrategy {
    config: RiAuthor4142LiveConfig,
    session_policy: RegularSessionPolicy,
    model_bars: Vec<ModelBar>,
    emitted_decision_keys: HashSet<String>,
    journal_records: Vec<RiAuthor4142JournalRecord>,
    live_mr: RiAuthor4142LiveMrState,
    live_bo: RiAuthor4142LiveBoState,
    state: StrategyState,
}

impl RiAuthor4142LiveStrategy {
    pub fn new(config: RiAuthor4142LiveConfig) -> Result<Self> {
        config.validate()?;
        let profile = ModelProfile::ri_shadow_10m();
        let session_policy = profile.session_policy;
        let live_adapter_enabled = config.can_emit_orders();
        Ok(Self {
            state: StrategyState::RiAuthor4142Live {
                mode: config.mode.as_str().to_string(),
                profile_id: config.profile_id.clone(),
                timeframe: config.timeframe.clone(),
                allow_order_emission: config.allow_order_emission,
                execution_path: config.execution_path.as_str().to_string(),
                last_bar_ts: None,
                last_model_bar_ts: None,
                model_bars_seen: 0,
                suppressed_service_bars: 0,
                model_decisions_seen: 0,
                last_decision_key: None,
                phase: RiAuthor4142Phase::Flat.as_str().to_string(),
                current_component: None,
                current_side: None,
                current_cycle_id: None,
                current_entry_ts_local: None,
                current_exit_ts_local: None,
                last_transition_reason: None,
                live_adapter_enabled,
            },
            config,
            session_policy,
            model_bars: Vec::new(),
            emitted_decision_keys: HashSet::new(),
            journal_records: Vec::new(),
            live_mr: RiAuthor4142LiveMrState::default(),
            live_bo: RiAuthor4142LiveBoState::default(),
        })
    }

    fn local_dt(&self, ts_utc: i64) -> Option<NaiveDateTime> {
        chrono::DateTime::from_timestamp(ts_utc, 0).map(|dt| {
            dt.naive_utc() + chrono::Duration::hours(self.config.timezone_offset_hours.into())
        })
    }

    fn update_bar_state(&mut self, bar: &BarEvent) -> Vec<RiAuthor4142ModelDecision> {
        let Some(dt_local) = self.local_dt(bar.close_time_utc) else {
            return Vec::new();
        };
        let is_model_bar =
            self.session_policy.is_model_bar(dt_local) && bar.origin != DataOrigin::HistoryGap;

        if let StrategyState::RiAuthor4142Live {
            last_bar_ts,
            last_model_bar_ts,
            model_bars_seen,
            suppressed_service_bars,
            ..
        } = &mut self.state
        {
            *last_bar_ts = Some(bar.close_time_utc);
            if is_model_bar {
                *last_model_bar_ts = Some(bar.close_time_utc);
                *model_bars_seen = model_bars_seen.saturating_add(1);
                self.model_bars.push(ModelBar {
                    ts_local: dt_local,
                    open: non_zero_or_close(bar.o, bar.close),
                    high: non_zero_or_close(bar.h, bar.close),
                    low: non_zero_or_close(bar.l, bar.close),
                    close: bar.close,
                    volume: bar.v,
                });
                if self.model_bars.len() > 5_000 {
                    let excess = self.model_bars.len() - 5_000;
                    self.model_bars.drain(..excess);
                }
            } else {
                *suppressed_service_bars = suppressed_service_bars.saturating_add(1);
            }
        }

        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_model_bar_observed",
            ts_utc = bar.close_time_utc,
            dt_local = %dt_local,
            hour = dt_local.time().hour(),
            minute = dt_local.time().minute(),
            is_model_bar,
            mode = self.config.mode.as_str(),
            allow_order_emission = self.config.allow_order_emission,
            live_adapter_enabled = self.can_emit_orders(),
        );

        if !is_model_bar {
            return Vec::new();
        }

        self.collect_new_decisions(dt_local)
    }

    fn collect_new_decisions(
        &mut self,
        current_dt_local: NaiveDateTime,
    ) -> Vec<RiAuthor4142ModelDecision> {
        let records =
            build_ri_author41_42_combo_shadow_journal(&self.model_bars, self.session_policy);
        let mut decisions = Vec::new();
        for record in records {
            if !is_finalized_record(&record, current_dt_local) {
                continue;
            }
            let key = decision_key(&record);
            if !self.emitted_decision_keys.insert(key.clone()) {
                continue;
            }
            let Some(decision) = RiAuthor4142ModelDecision::from_shadow_record(record, key) else {
                continue;
            };
            self.record_decision_state(&decision);
            self.record_shadow_journal(&decision);
            self.log_decision(&decision);
            if !self.can_emit_orders() {
                self.apply_dry_run_decision(&decision);
            }
            decisions.push(decision);
        }
        decisions
    }

    fn record_decision_state(&mut self, decision: &RiAuthor4142ModelDecision) {
        if let StrategyState::RiAuthor4142Live {
            model_decisions_seen,
            last_decision_key,
            ..
        } = &mut self.state
        {
            *model_decisions_seen = model_decisions_seen.saturating_add(1);
            *last_decision_key = Some(decision.decision_key.clone());
        }
    }

    fn log_decision(&self, decision: &RiAuthor4142ModelDecision) {
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_model_decision",
            decision_action = ?decision.action,
            component = decision.component.as_str(),
            side = decision.side.map(RiAuthor4142Side::as_str).unwrap_or("none"),
            model_signal_ts_local = %decision.model_signal_ts_local,
            scheduled_entry_ts_local = ?decision.scheduled_entry_ts_local,
            scheduled_exit_ts_local = ?decision.scheduled_exit_ts_local,
            reason = %decision.reason,
            overlap_decision = %decision.overlap_decision,
            shadow_pnl_points = ?decision.shadow_pnl_points,
            decision_key = %decision.decision_key,
            mode = self.config.mode.as_str(),
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn apply_dry_run_decision(&mut self, decision: &RiAuthor4142ModelDecision) {
        match decision.action {
            RiAuthor4142DecisionAction::Suppress => {
                self.record_suppressed_journal(decision);
                self.log_transition("suppressed", "ri_intent_suppressed", decision);
            }
            RiAuthor4142DecisionAction::Enter => {
                if let Some(reason) = self.decision_safety_reject_reason(decision) {
                    self.record_manual_intervention_journal(decision, reason);
                    self.log_manual_intervention_required(decision, reason);
                    self.enter_manual_intervention_required(reason.to_string());
                    return;
                }
                let candidates = self.candidate_intents_for_decision(decision);
                if candidates.is_empty() {
                    self.record_manual_intervention_journal(
                        decision,
                        "accepted_model_decision_without_complete_entry_exit_candidate",
                    );
                    self.log_manual_intervention_required(
                        decision,
                        "accepted_model_decision_without_complete_entry_exit_candidate",
                    );
                    return;
                }
                for candidate in &candidates {
                    self.record_candidate_suppressed_journal(candidate, decision);
                    self.log_candidate_intent_suppressed(candidate, decision);
                }
                self.transition_to_dry_run_in_position(decision);
                self.transition_to_flat_after_scheduled_exit(decision);
            }
        }
    }

    fn decision_safety_reject_reason(
        &self,
        decision: &RiAuthor4142ModelDecision,
    ) -> Option<&'static str> {
        let (Some(entry_ts), Some(exit_ts)) = (
            decision.scheduled_entry_ts_local,
            decision.scheduled_exit_ts_local,
        ) else {
            return None;
        };

        if exit_ts < entry_ts {
            return Some("exit_before_entry_not_allowed");
        }
        if exit_ts.date() != entry_ts.date() {
            return Some("cross_day_exit_not_allowed");
        }
        None
    }

    fn candidate_intents_for_decision(
        &self,
        decision: &RiAuthor4142ModelDecision,
    ) -> Vec<RiAuthor4142CandidateIntent> {
        if decision.action != RiAuthor4142DecisionAction::Enter {
            return Vec::new();
        }
        let Some(model_side) = decision.side else {
            return Vec::new();
        };
        let mut candidates = Vec::new();
        if let Some(entry_ts) = decision.scheduled_entry_ts_local {
            candidates.push(self.build_candidate_intent(
                decision,
                RiAuthor4142CandidateRole::Entry,
                model_side.entry_order_side(),
                entry_ts,
            ));
        }
        if let Some(exit_ts) = decision.scheduled_exit_ts_local {
            candidates.push(self.build_candidate_intent(
                decision,
                RiAuthor4142CandidateRole::Exit,
                model_side.exit_order_side(),
                exit_ts,
            ));
        }
        candidates
    }

    fn record_shadow_journal(&mut self, decision: &RiAuthor4142ModelDecision) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::ShadowRecorded,
            None,
            None,
        ));
    }

    fn record_suppressed_journal(&mut self, decision: &RiAuthor4142ModelDecision) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::IntentSuppressed,
            None,
            None,
        ));
    }

    fn record_candidate_suppressed_journal(
        &mut self,
        candidate: &RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::IntentSuppressed,
            Some(candidate),
            None,
        ));
    }

    fn record_manual_intervention_journal(
        &mut self,
        decision: &RiAuthor4142ModelDecision,
        reason: &'static str,
    ) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::ManualInterventionRequired,
            None,
            Some(reason),
        ));
    }

    fn push_journal_record(&mut self, record: RiAuthor4142JournalRecord) {
        self.journal_records.push(record);
        if self.journal_records.len() > 10_000 {
            let excess = self.journal_records.len() - 10_000;
            self.journal_records.drain(..excess);
        }
    }

    fn build_candidate_intent(
        &self,
        decision: &RiAuthor4142ModelDecision,
        role: RiAuthor4142CandidateRole,
        side: OrderSide,
        scheduled_ts_local: NaiveDateTime,
    ) -> RiAuthor4142CandidateIntent {
        RiAuthor4142CandidateIntent {
            role,
            component: decision.component,
            side,
            qty: self.config.qty,
            order_style: "market_p0",
            intent_class: role.intent_class(),
            scheduled_ts_local,
            comment: self.candidate_comment(decision, role),
            execution_path: self.config.execution_path,
            decision_key: decision.decision_key.clone(),
        }
    }

    fn candidate_comment(
        &self,
        decision: &RiAuthor4142ModelDecision,
        role: RiAuthor4142CandidateRole,
    ) -> String {
        format!(
            "ri_author41_42:{}:{}:{}",
            self.config.profile_id,
            decision.component.as_str(),
            role.as_str()
        )
    }

    fn log_candidate_intent_suppressed(
        &self,
        candidate: &RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) {
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_candidate_intent_suppressed",
            suppression_reason = "pre_go_order_emission_disabled",
            role = candidate.role.as_str(),
            component = candidate.component.as_str(),
            model_side = decision.side.map(RiAuthor4142Side::as_str).unwrap_or("none"),
            order_side = ?candidate.side,
            qty = candidate.qty,
            order_style = candidate.order_style,
            intent_class = ?candidate.intent_class,
            scheduled_ts_local = %candidate.scheduled_ts_local,
            comment = %candidate.comment,
            execution_path = candidate.execution_path.as_str(),
            decision_key = %candidate.decision_key,
            mode = self.config.mode.as_str(),
            allow_order_emission = self.config.allow_order_emission,
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn log_manual_intervention_required(
        &self,
        decision: &RiAuthor4142ModelDecision,
        reason: &'static str,
    ) {
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_manual_intervention_required",
            reason,
            component = decision.component.as_str(),
            side = decision.side.map(RiAuthor4142Side::as_str).unwrap_or("none"),
            decision_key = %decision.decision_key,
            mode = self.config.mode.as_str(),
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn enter_manual_intervention_required(&mut self, reason: String) {
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            ..
        } = &mut self.state
        {
            *phase = RiAuthor4142Phase::ManualInterventionRequired
                .as_str()
                .to_string();
            *current_component = None;
            *current_side = None;
            *current_cycle_id = None;
            *current_entry_ts_local = None;
            *current_exit_ts_local = None;
            *last_transition_reason = Some(reason.clone());
        }
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_manual_intervention_required",
            reason = %reason,
            mode = self.config.mode.as_str(),
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn handle_bootstrap_snapshot(&mut self, snapshot: &BootstrapSnapshot) {
        let symbol = self.config.symbol.as_str();
        let position_qty = snapshot
            .positions_strategy
            .get(symbol)
            .map(|position| position.qty)
            .unwrap_or(0.0);
        let working_orders = snapshot
            .working_orders_strategy
            .values()
            .filter(|order| order.symbol == symbol)
            .count();
        let working_stop_orders = snapshot
            .working_stop_orders_strategy
            .values()
            .filter(|order| order.symbol == symbol)
            .count();
        let reason = if position_qty.abs() > f64::EPSILON {
            Some(format!("bootstrap_non_flat_position_qty:{position_qty}"))
        } else if working_orders > 0 {
            Some(format!("bootstrap_working_orders:{working_orders}"))
        } else if working_stop_orders > 0 {
            Some(format!(
                "bootstrap_working_stop_orders:{working_stop_orders}"
            ))
        } else {
            None
        };

        if let Some(reason) = reason {
            self.enter_manual_intervention_required(reason);
        } else {
            info!(
                target: "strategy_runtime::ri_author41_42_live",
                action = "ri_bootstrap_reconciled_flat",
                symbol,
                snapshot_ts_utc = ?snapshot.snapshot_ts_utc,
                mode = self.config.mode.as_str(),
                live_adapter_enabled = self.can_emit_orders(),
            );
        }
    }

    fn handle_runtime_state_restored(&mut self, state: &RuntimeStateRestored) {
        let reason = if !state.pending_requests.is_empty() {
            Some(format!(
                "runtime_restore_pending_requests:{}",
                state.pending_requests.len()
            ))
        } else if !state.known_order_ids.is_empty() {
            Some(format!(
                "runtime_restore_known_order_ids:{}",
                state.known_order_ids.len()
            ))
        } else {
            None
        };

        if let Some(reason) = reason {
            self.enter_manual_intervention_required(reason);
        } else {
            info!(
                target: "strategy_runtime::ri_author41_42_live",
                action = "ri_runtime_state_restored_clean",
                mode = self.config.mode.as_str(),
                live_adapter_enabled = self.can_emit_orders(),
            );
        }
    }

    fn transition_to_dry_run_in_position(&mut self, decision: &RiAuthor4142ModelDecision) {
        let cycle_id = format!(
            "{}:{}",
            decision.component.as_str(),
            decision.model_signal_ts_local.format("%Y%m%d%H%M%S")
        );
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            ..
        } = &mut self.state
        {
            *phase = RiAuthor4142Phase::DryRunInPosition.as_str().to_string();
            *current_component = Some(decision.component.as_str().to_string());
            *current_side = decision.side.map(|side| side.as_str().to_string());
            *current_cycle_id = Some(cycle_id);
            *current_entry_ts_local = decision.scheduled_entry_ts_local.map(|ts| ts.to_string());
            *current_exit_ts_local = decision.scheduled_exit_ts_local.map(|ts| ts.to_string());
            *last_transition_reason = Some(decision.reason.clone());
        }
        self.log_transition("flat", "ri_dry_run_position_opened", decision);
    }

    fn transition_to_flat_after_scheduled_exit(&mut self, decision: &RiAuthor4142ModelDecision) {
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            ..
        } = &mut self.state
        {
            *phase = RiAuthor4142Phase::Flat.as_str().to_string();
            *current_component = None;
            *current_side = None;
            *current_cycle_id = None;
            *current_entry_ts_local = None;
            *current_exit_ts_local = None;
            *last_transition_reason = Some(format!("dry_run_exit:{}", decision.reason));
        }
        self.log_transition(
            "dry_run_in_position",
            "ri_dry_run_position_closed",
            decision,
        );
    }

    fn log_transition(
        &self,
        from_phase: &'static str,
        action: &'static str,
        decision: &RiAuthor4142ModelDecision,
    ) {
        let to_phase = match &self.state {
            StrategyState::RiAuthor4142Live { phase, .. } => phase.as_str(),
            _ => "unknown",
        };
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action,
            from_phase,
            to_phase,
            component = decision.component.as_str(),
            side = decision.side.map(RiAuthor4142Side::as_str).unwrap_or("none"),
            decision_key = %decision.decision_key,
            reason = %decision.reason,
            scheduled_entry_ts_local = ?decision.scheduled_entry_ts_local,
            scheduled_exit_ts_local = ?decision.scheduled_exit_ts_local,
            mode = self.config.mode.as_str(),
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn live_model_bar_for_emit(&self, ctx: &StrategyCtx, bar: &BarEvent) -> Option<ModelBar> {
        if !self.can_emit_orders()
            || ctx.trade_mode != crate::TradeMode::Live
            || !ctx.allow_live_orders
        {
            return None;
        }
        if bar.origin != DataOrigin::Live {
            return None;
        }
        let dt_local = self.local_dt(bar.close_time_utc)?;
        if !self.session_policy.is_model_bar(dt_local) {
            return None;
        }
        Some(ModelBar {
            ts_local: dt_local,
            open: non_zero_or_close(bar.o, bar.close),
            high: non_zero_or_close(bar.h, bar.close),
            low: non_zero_or_close(bar.l, bar.close),
            close: bar.close,
            volume: bar.v,
        })
    }

    fn live_intents_for_bar(&mut self, bar: ModelBar) -> Vec<Intent> {
        let mut intents = Vec::new();
        intents.extend(self.live_mr_intents_for_bar(bar));
        intents.extend(self.live_bo_intents_for_bar(bar));
        intents
    }

    fn live_mr_intents_for_bar(&mut self, bar: ModelBar) -> Vec<Intent> {
        let mut intents = Vec::new();
        if self.live_mr.current_date != Some(bar.ts_local.date()) {
            self.live_mr.current_date = Some(bar.ts_local.date());
            self.live_mr.entries_today = 0;
            self.live_mr.position = None;
        }

        if let Some(mut position) = self.live_mr.position.take() {
            position.bars_held = position.bars_held.saturating_add(1);
            if let Some((exit_price, reason)) = Self::mr_exit_signal(&position, bar) {
                let decision = self.live_decision(RiAuthor4142LiveDecisionInput {
                    component: RiAuthor4142Component::Author41Mr,
                    side: position.side,
                    model_signal_ts_local: position.entry_ts_local,
                    scheduled_entry_ts_local: Some(position.entry_ts_local),
                    scheduled_exit_ts_local: Some(bar.ts_local),
                    reason,
                    decision_key: position.decision_key.clone(),
                    shadow_pnl_points: Some(
                        Self::points_for_side(position.side, position.entry_price, exit_price)
                            - position.config.roundtrip_cost_points,
                    ),
                });
                let candidate = self.live_candidate_for_decision(
                    &decision,
                    RiAuthor4142CandidateRole::Exit,
                    position.side.exit_order_side(),
                    bar.ts_local,
                );
                intents.push(self.emit_live_candidate(candidate, &decision));
                self.transition_live_flat(&decision, "live_exit_emitted");
                return intents;
            } else {
                self.live_mr.position = Some(position);
                return intents;
            }
        }

        if self.live_mr.entries_today >= 2 {
            return intents;
        }
        if self.live_bo.position.is_some() {
            return intents;
        }
        let Some((anchor, side, config)) = self.mr_entry_signal(bar) else {
            return intents;
        };
        self.live_mr.entries_today = self.live_mr.entries_today.saturating_add(1);
        let decision_key = format!(
            "{}|author41_mr|{}|Some({:?})|live_prospective",
            self.config.profile_id, bar.ts_local, side
        );
        let decision = self.live_decision(RiAuthor4142LiveDecisionInput {
            component: RiAuthor4142Component::Author41Mr,
            side,
            model_signal_ts_local: bar.ts_local,
            scheduled_entry_ts_local: Some(bar.ts_local),
            scheduled_exit_ts_local: None,
            reason: "live_entry",
            decision_key: decision_key.clone(),
            shadow_pnl_points: None,
        });
        let candidate = self.live_candidate_for_decision(
            &decision,
            RiAuthor4142CandidateRole::Entry,
            side.entry_order_side(),
            bar.ts_local,
        );
        self.live_mr.position = Some(RiAuthor4142LiveMrPosition {
            side,
            entry_ts_local: bar.ts_local,
            entry_price: bar.close,
            prev_close: anchor.prev_close,
            prev_range: anchor.prev_range,
            bars_held: 0,
            config,
            decision_key,
        });
        intents.push(self.emit_live_candidate(candidate, &decision));
        self.transition_live_in_position(&decision, "live_entry_emitted");
        intents
    }

    fn live_bo_intents_for_bar(&mut self, bar: ModelBar) -> Vec<Intent> {
        let mut intents = Vec::new();
        self.ensure_bo_session(bar);
        let config = Author42Config::ri_grid_k042_both();

        if let Some(pending) = self.live_bo.pending.take() {
            match pending {
                RiAuthor4142LiveBoPending::Entry(side) => {
                    if self.live_mr.position.is_none() && self.live_bo.position.is_none() {
                        let decision_key = format!(
                            "{}|author42_bo|{}|Some({:?})|live_prospective",
                            self.config.profile_id, bar.ts_local, side
                        );
                        let decision = self.live_decision(RiAuthor4142LiveDecisionInput {
                            component: RiAuthor4142Component::Author42Bo,
                            side,
                            model_signal_ts_local: bar.ts_local,
                            scheduled_entry_ts_local: Some(bar.ts_local),
                            scheduled_exit_ts_local: None,
                            reason: "live_entry_next_bar",
                            decision_key: decision_key.clone(),
                            shadow_pnl_points: None,
                        });
                        let candidate = self.live_candidate_for_decision(
                            &decision,
                            RiAuthor4142CandidateRole::Entry,
                            side.entry_order_side(),
                            bar.ts_local,
                        );
                        self.live_bo.position = Some(RiAuthor4142LiveBoPosition {
                            side,
                            entry_ts_local: bar.ts_local,
                            entry_price: non_zero_or_close(bar.open, bar.close),
                            bars_held: 0,
                            decision_key,
                        });
                        match side {
                            RiAuthor4142Side::Long => self.live_bo.was_long_today = true,
                            RiAuthor4142Side::Short => self.live_bo.was_short_today = true,
                        }
                        intents.push(self.emit_live_candidate(candidate, &decision));
                        self.transition_live_in_position(&decision, "live_bo_entry_emitted");
                    }
                }
                RiAuthor4142LiveBoPending::Exit(reason) => {
                    if let Some(position) = self.live_bo.position.take() {
                        let exit_price = non_zero_or_close(bar.open, bar.close);
                        let decision = self.live_decision(RiAuthor4142LiveDecisionInput {
                            component: RiAuthor4142Component::Author42Bo,
                            side: position.side,
                            model_signal_ts_local: position.entry_ts_local,
                            scheduled_entry_ts_local: Some(position.entry_ts_local),
                            scheduled_exit_ts_local: Some(bar.ts_local),
                            reason,
                            decision_key: position.decision_key,
                            shadow_pnl_points: Some(
                                Self::points_for_side(
                                    position.side,
                                    position.entry_price,
                                    exit_price,
                                ) - config.roundtrip_cost_points,
                            ),
                        });
                        let candidate = self.live_candidate_for_decision(
                            &decision,
                            RiAuthor4142CandidateRole::Exit,
                            position.side.exit_order_side(),
                            bar.ts_local,
                        );
                        intents.push(self.emit_live_candidate(candidate, &decision));
                        self.transition_live_flat(&decision, "live_bo_exit_emitted");
                    }
                }
            }
        }

        self.live_bo.day_hh = self.live_bo.day_hh.max(bar.high);
        self.live_bo.day_ll = self.live_bo.day_ll.min(bar.low);

        if let Some(mut position) = self.live_bo.position.take() {
            position.bars_held = position.bars_held.saturating_add(1);
            if bar.ts_local.time() >= config.exit_time {
                let decision = self.live_decision(RiAuthor4142LiveDecisionInput {
                    component: RiAuthor4142Component::Author42Bo,
                    side: position.side,
                    model_signal_ts_local: position.entry_ts_local,
                    scheduled_entry_ts_local: Some(position.entry_ts_local),
                    scheduled_exit_ts_local: Some(bar.ts_local),
                    reason: "time_exit_same_bar_close",
                    decision_key: position.decision_key,
                    shadow_pnl_points: Some(
                        Self::points_for_side(position.side, position.entry_price, bar.close)
                            - config.roundtrip_cost_points,
                    ),
                });
                let candidate = self.live_candidate_for_decision(
                    &decision,
                    RiAuthor4142CandidateRole::Exit,
                    position.side.exit_order_side(),
                    bar.ts_local,
                );
                intents.push(self.emit_live_candidate(candidate, &decision));
                self.transition_live_flat(&decision, "live_bo_exit_emitted");
            } else {
                if let Some(context) = self.live_bo.context {
                    match position.side {
                        RiAuthor4142Side::Long => {
                            if bar.close < context.prev_close + config.stop_k * context.prev_range {
                                self.live_bo.pending = Some(RiAuthor4142LiveBoPending::Exit(
                                    "stop_emergency_long_next_open",
                                ));
                            } else if Self::is_author42_hour_check(
                                bar.ts_local,
                                self.live_bo.first_bar.map(|bar| bar.ts_local),
                            ) && bar.close
                                < context.prev_close + config.stop_hour_k * context.prev_range
                            {
                                self.live_bo.pending = Some(RiAuthor4142LiveBoPending::Exit(
                                    "stop_hour_long_next_open",
                                ));
                            }
                        }
                        RiAuthor4142Side::Short => {
                            if bar.close > context.prev_close - config.stop_k * context.prev_range {
                                self.live_bo.pending = Some(RiAuthor4142LiveBoPending::Exit(
                                    "stop_emergency_short_next_open",
                                ));
                            } else if Self::is_author42_hour_check(
                                bar.ts_local,
                                self.live_bo.first_bar.map(|bar| bar.ts_local),
                            ) && bar.close
                                > context.prev_close - config.stop_hour_k * context.prev_range
                            {
                                self.live_bo.pending = Some(RiAuthor4142LiveBoPending::Exit(
                                    "stop_hour_short_next_open",
                                ));
                            }
                        }
                    }
                }
                self.live_bo.position = Some(position);
            }
        }

        self.update_bo_levels_and_pending_entry(bar, config);
        self.live_bo.bar_index = self.live_bo.bar_index.saturating_add(1);
        intents
    }

    fn ensure_bo_session(&mut self, bar: ModelBar) {
        if self.live_bo.current_date == Some(bar.ts_local.date()) {
            return;
        }
        let date = bar.ts_local.date();
        let config = Author42Config::ri_grid_k042_both();
        let context = self.bo_context_for_date(date);
        let skip_by_date = (config.exclude_friday && date.weekday().number_from_monday() == 5)
            || (config.exclude_june_window && Self::in_author_june_window(date));
        let trade_allowed = context
            .map(|ctx| {
                !skip_by_date
                    && all_finite_live(&[
                        ctx.prev_close,
                        ctx.prev2_close,
                        ctx.prev_range,
                        ctx.prev_hl_ratio,
                        ctx.prev_ret,
                    ])
                    && ctx.prev_range > 0.0
                    && ctx.prev_close > 0.0
            })
            .unwrap_or(false);
        self.live_bo = RiAuthor4142LiveBoState {
            current_date: Some(date),
            context,
            first_bar: Some(bar),
            trade_allowed,
            ..RiAuthor4142LiveBoState::default()
        };
    }

    fn update_bo_levels_and_pending_entry(&mut self, bar: ModelBar, config: Author42Config) {
        let Some(context) = self.live_bo.context else {
            return;
        };
        if self.live_bo.bar_index == 5 {
            if let Some(first_bar) = self.live_bo.first_bar {
                self.live_bo.long_level = Some(
                    (context.prev_close + config.k * context.prev_range)
                        .max(bar.close)
                        .max(first_bar.high),
                );
                self.live_bo.short_level = Some(
                    (context.prev_close - config.k * context.prev_range)
                        .min(bar.close)
                        .min(first_bar.low),
                );
            }
            if config.use_first_hour_extreme_filter
                && (bar.close - context.prev_close).abs()
                    > config.first_hour_extreme_k * context.prev_range
            {
                self.live_bo.trade_allowed = false;
            }
        }
        if self.live_bo.pending.is_some()
            || self.live_bo.position.is_some()
            || self.live_mr.position.is_some()
            || !self.live_bo.trade_allowed
        {
            return;
        }
        let (Some(long_level), Some(short_level)) =
            (self.live_bo.long_level, self.live_bo.short_level)
        else {
            return;
        };
        if bar.ts_local.time() >= config.exit_time {
            return;
        }

        let range_ok = context.prev_hl_ratio > config.min_prev_hl_ratio;
        let mut buy_trig = range_ok && context.prev_ret > -config.prev_extreme_move;
        let mut short_trig = range_ok && context.prev_ret < config.prev_extreme_move;
        match config.side_mode {
            Author42SideMode::Long => short_trig = false,
            Author42SideMode::Short => buy_trig = false,
            Author42SideMode::Both => {}
        }

        if config.allow_reentry_on_day_extreme {
            if self.live_bo.was_long_today
                && bar.high >= self.live_bo.day_hh
                && bar.ts_local.time() < config.exit_time
            {
                self.live_bo.pending =
                    Some(RiAuthor4142LiveBoPending::Entry(RiAuthor4142Side::Long));
                return;
            }
            if self.live_bo.was_short_today
                && bar.low <= self.live_bo.day_ll
                && bar.ts_local.time() < config.exit_time
            {
                self.live_bo.pending =
                    Some(RiAuthor4142LiveBoPending::Entry(RiAuthor4142Side::Short));
                return;
            }
        }

        if Self::is_author42_hour_check(
            bar.ts_local,
            self.live_bo.first_bar.map(|bar| bar.ts_local),
        ) {
            if buy_trig && bar.close > long_level {
                self.live_bo.pending =
                    Some(RiAuthor4142LiveBoPending::Entry(RiAuthor4142Side::Long));
            } else if short_trig && bar.close < short_level {
                self.live_bo.pending =
                    Some(RiAuthor4142LiveBoPending::Entry(RiAuthor4142Side::Short));
            }
        }
    }

    fn mr_entry_signal(
        &self,
        bar: ModelBar,
    ) -> Option<(RiDailyAnchor, RiAuthor4142Side, Author41Config)> {
        let anchor = self.mr_anchor_for_date(bar.ts_local.date())?;
        if !anchor.prev_close.is_finite()
            || !anchor.prev_range.is_finite()
            || !anchor.prev_low.is_finite()
            || anchor.prev_range <= 0.0
            || anchor.prev_low <= 0.0
        {
            return None;
        }
        let short = Author41Config::ri_plateau_short_source();
        let long = Author41Config::ri_plateau_long_source();
        for (side, config) in [
            (RiAuthor4142Side::Short, short),
            (RiAuthor4142Side::Long, long),
        ] {
            if bar.ts_local.time() > config.entry_end {
                continue;
            }
            let rel_range = anchor.prev_range / anchor.prev_low;
            if !(rel_range.is_finite()
                && config.min_range < rel_range
                && rel_range < config.max_range)
            {
                continue;
            }
            let signal = match side {
                RiAuthor4142Side::Short => {
                    bar.close > anchor.prev_close
                        && bar.close < anchor.prev_close + config.k * anchor.prev_range
                }
                RiAuthor4142Side::Long => {
                    bar.close < anchor.prev_close
                        && bar.close > anchor.prev_close - config.k * anchor.prev_range
                }
            };
            if signal {
                return Some((anchor, side, config));
            }
        }
        None
    }

    fn mr_exit_signal(
        position: &RiAuthor4142LiveMrPosition,
        bar: ModelBar,
    ) -> Option<(f64, &'static str)> {
        let config = position.config;
        match position.side {
            RiAuthor4142Side::Short => {
                let stop_price = position.prev_close + config.stop_k * position.prev_range;
                let take_price = position.prev_close - config.k2 * position.prev_range;
                if bar.high >= stop_price {
                    Some((stop_price, "stop"))
                } else if bar.close < take_price {
                    Some((bar.close, "take_author_close"))
                } else if bar.ts_local.time() >= config.time_exit {
                    Some((bar.close, "time_exit"))
                } else if position.bars_held > config.breakeven_after_bars
                    && bar.low <= position.entry_price
                {
                    Some((position.entry_price, "breakeven_limit"))
                } else {
                    None
                }
            }
            RiAuthor4142Side::Long => {
                let stop_price = position.prev_close - config.stop_k * position.prev_range;
                let take_price = position.prev_close + config.k2 * position.prev_range;
                if bar.low <= stop_price {
                    Some((stop_price, "stop"))
                } else if bar.close > take_price {
                    Some((bar.close, "take_author_close"))
                } else if bar.ts_local.time() >= config.time_exit {
                    Some((bar.close, "time_exit"))
                } else if position.bars_held > config.breakeven_after_bars
                    && bar.high >= position.entry_price
                {
                    Some((position.entry_price, "breakeven_limit"))
                } else {
                    None
                }
            }
        }
    }

    fn live_decision(&self, input: RiAuthor4142LiveDecisionInput) -> RiAuthor4142ModelDecision {
        RiAuthor4142ModelDecision {
            action: RiAuthor4142DecisionAction::Enter,
            component: input.component,
            side: Some(input.side),
            model_signal_ts_local: input.model_signal_ts_local,
            scheduled_entry_ts_local: input.scheduled_entry_ts_local,
            scheduled_exit_ts_local: input.scheduled_exit_ts_local,
            reason: input.reason.to_string(),
            overlap_decision: "Accepted".to_string(),
            shadow_pnl_points: input.shadow_pnl_points,
            decision_key: input.decision_key,
        }
    }

    fn live_candidate_for_decision(
        &self,
        decision: &RiAuthor4142ModelDecision,
        role: RiAuthor4142CandidateRole,
        side: OrderSide,
        scheduled_ts_local: NaiveDateTime,
    ) -> RiAuthor4142CandidateIntent {
        self.build_candidate_intent(decision, role, side, scheduled_ts_local)
    }

    fn emit_live_candidate(
        &mut self,
        candidate: RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) -> Intent {
        self.record_candidate_emitted_journal(&candidate, decision);
        self.log_candidate_intent_emitted(&candidate, decision);
        let intent = Intent::Market {
            qty: candidate.qty,
            side: candidate.side,
            fill_price: None,
            comment: Some(candidate.comment),
        }
        .with_class(candidate.intent_class);
        if let Some(order_symbol) = self.config.order_symbol.as_deref() {
            if order_symbol != self.config.symbol {
                return intent.with_symbol(order_symbol.to_string());
            }
        }
        intent
    }

    fn record_candidate_emitted_journal(
        &mut self,
        candidate: &RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::IntentEmitted,
            Some(candidate),
            None,
        ));
    }

    fn log_candidate_intent_emitted(
        &self,
        candidate: &RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) {
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_intent_emitted",
            role = candidate.role.as_str(),
            component = candidate.component.as_str(),
            model_side = decision.side.map(RiAuthor4142Side::as_str).unwrap_or("none"),
            order_side = ?candidate.side,
            qty = candidate.qty,
            order_style = candidate.order_style,
            intent_class = ?candidate.intent_class,
            scheduled_ts_local = %candidate.scheduled_ts_local,
            comment = %candidate.comment,
            execution_path = candidate.execution_path.as_str(),
            order_symbol = self.config.order_symbol.as_deref().unwrap_or(self.config.symbol.as_str()),
            decision_key = %candidate.decision_key,
            mode = self.config.mode.as_str(),
            allow_order_emission = self.config.allow_order_emission,
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn transition_live_in_position(
        &mut self,
        decision: &RiAuthor4142ModelDecision,
        reason: &'static str,
    ) {
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            ..
        } = &mut self.state
        {
            *phase = "live_in_position".to_string();
            *current_component = Some(decision.component.as_str().to_string());
            *current_side = decision.side.map(|side| side.as_str().to_string());
            *current_cycle_id = Some(format!(
                "{}:{}",
                decision.component.as_str(),
                decision.model_signal_ts_local.format("%Y%m%d%H%M%S")
            ));
            *current_entry_ts_local = decision.scheduled_entry_ts_local.map(|ts| ts.to_string());
            *current_exit_ts_local = decision.scheduled_exit_ts_local.map(|ts| ts.to_string());
            *last_transition_reason = Some(reason.to_string());
        }
    }

    fn transition_live_flat(&mut self, decision: &RiAuthor4142ModelDecision, reason: &'static str) {
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            ..
        } = &mut self.state
        {
            *phase = RiAuthor4142Phase::Flat.as_str().to_string();
            *current_component = None;
            *current_side = None;
            *current_cycle_id = None;
            *current_entry_ts_local = None;
            *current_exit_ts_local = None;
            *last_transition_reason = Some(format!("{}:{}", reason, decision.reason));
        }
    }

    fn mr_anchor_for_date(&self, date: NaiveDate) -> Option<RiDailyAnchor> {
        let stats = self.daily_stats_before(date);
        let (_, prev) = stats.last().copied()?;
        Some(RiDailyAnchor {
            prev_close: prev.close,
            prev_low: prev.low,
            prev_range: prev.high - prev.low,
        })
    }

    fn bo_context_for_date(&self, date: NaiveDate) -> Option<RiAuthor4142LiveBoContext> {
        let stats = self.daily_stats_before(date);
        if stats.len() < 2 {
            return None;
        }
        let (_, prev2) = stats[stats.len() - 2];
        let (_, prev) = stats[stats.len() - 1];
        let prev_range = prev.high - prev.low;
        Some(RiAuthor4142LiveBoContext {
            prev_close: prev.close,
            prev2_close: prev2.close,
            prev_range,
            prev_hl_ratio: prev.high / prev.low,
            prev_ret: prev.close / prev2.close - 1.0,
        })
    }

    fn daily_stats_before(&self, date: NaiveDate) -> Vec<(NaiveDate, RiDailyStats)> {
        let mut by_day: BTreeMap<NaiveDate, RiDailyStats> = BTreeMap::new();
        for bar in &self.model_bars {
            if bar.ts_local.date() >= date {
                continue;
            }
            by_day
                .entry(bar.ts_local.date())
                .and_modify(|stats| {
                    stats.high = stats.high.max(bar.high);
                    stats.low = stats.low.min(bar.low);
                    stats.close = bar.close;
                })
                .or_insert(RiDailyStats {
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                });
        }
        by_day.into_iter().collect()
    }

    fn points_for_side(side: RiAuthor4142Side, entry_price: f64, exit_price: f64) -> f64 {
        match side {
            RiAuthor4142Side::Long => exit_price - entry_price,
            RiAuthor4142Side::Short => entry_price - exit_price,
        }
    }

    fn is_author42_hour_check(ts: NaiveDateTime, start_ts: Option<NaiveDateTime>) -> bool {
        let Some(start_ts) = start_ts else {
            return false;
        };
        ts.time().minute() == 50 && ts > start_ts + chrono::Duration::minutes(50)
    }

    fn in_author_june_window(date: NaiveDate) -> bool {
        let Some(start) = NaiveDate::from_ymd_opt(date.year(), 5, 21) else {
            return false;
        };
        let Some(end) = NaiveDate::from_ymd_opt(date.year(), 7, 1) else {
            return false;
        };
        start <= date && date <= end
    }

    pub fn can_emit_orders(&self) -> bool {
        self.config.can_emit_orders()
    }

    pub fn phase_for_test(&self) -> Option<String> {
        match &self.state {
            StrategyState::RiAuthor4142Live { phase, .. } => Some(phase.clone()),
            _ => None,
        }
    }

    fn clear_live_positions(&mut self) {
        self.live_mr.position = None;
        self.live_bo.position = None;
        self.live_bo.pending = None;
    }

    pub fn journal_records_for_test(&self) -> &[RiAuthor4142JournalRecord] {
        &self.journal_records
    }
}

impl Strategy for RiAuthor4142LiveStrategy {
    fn on_bar(&mut self, ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        let _decisions = self.update_bar_state(bar);
        let Some(model_bar) = self.live_model_bar_for_emit(ctx, bar) else {
            return Vec::new();
        };
        self.live_intents_for_bar(model_bar)
    }

    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &alor_protocol::CommandAck) -> Vec<Intent> {
        if !matches!(
            ack.status,
            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
        ) {
            return Vec::new();
        }
        let phase = self.phase_for_test();
        if phase.as_deref() == Some("live_in_position") {
            let broker_qty = ctx.position_qty.unwrap_or(0.0);
            if broker_qty.abs() <= f64::EPSILON {
                self.clear_live_positions();
                if let StrategyState::RiAuthor4142Live {
                    phase,
                    current_component,
                    current_side,
                    current_cycle_id,
                    current_entry_ts_local,
                    current_exit_ts_local,
                    last_transition_reason,
                    ..
                } = &mut self.state
                {
                    *phase = RiAuthor4142Phase::Flat.as_str().to_string();
                    *current_component = None;
                    *current_side = None;
                    *current_cycle_id = None;
                    *current_entry_ts_local = None;
                    *current_exit_ts_local = None;
                    *last_transition_reason = Some(format!(
                        "live_entry_ack_rejected:{}:{}",
                        ack.error_code.as_deref().unwrap_or("unknown"),
                        ack.error_msg.as_deref().unwrap_or("unknown")
                    ));
                }
                info!(
                    target: "strategy_runtime::ri_author41_42_live",
                    action = "ri_live_entry_rejected_rolled_back",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    error_msg = ?ack.error_msg,
                    broker_qty,
                    mode = self.config.mode.as_str(),
                    live_adapter_enabled = self.can_emit_orders(),
                );
            } else {
                self.enter_manual_intervention_required(format!(
                    "live_ack_rejected_with_broker_position:{}:{}",
                    ack.error_code.as_deref().unwrap_or("unknown"),
                    ack.error_msg.as_deref().unwrap_or("unknown")
                ));
            }
        }
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        _ctx: &StrategyCtx,
        snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        self.handle_bootstrap_snapshot(snapshot);
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        self.handle_runtime_state_restored(state);
        Vec::new()
    }

    fn warmup_from_history(&mut self, _ctx: &StrategyCtx, bars: &[BarEvent]) -> usize {
        for bar in bars {
            let _decisions = self.update_bar_state(bar);
        }
        bars.len()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn drain_observation_journal_records(&mut self) -> Vec<serde_json::Value> {
        self.journal_records
            .drain(..)
            .map(|record| {
                serde_json::to_value(record).expect("ri_author41_42 journal record must serialize")
            })
            .collect()
    }

    fn set_state(&mut self, state: StrategyState) {
        if let StrategyState::RiAuthor4142Live { .. } = state {
            self.state = state;
        }
    }
}

impl RiAuthor4142ModelDecision {
    fn from_shadow_record(record: ShadowJournalRecord, decision_key: String) -> Option<Self> {
        let component = RiAuthor4142Component::from_shadow(record.component)?;
        let side = record.side.map(RiAuthor4142Side::from_shadow);
        let action = if record.overlap_decision == OverlapDecision::DroppedMrOverlap {
            RiAuthor4142DecisionAction::Suppress
        } else {
            RiAuthor4142DecisionAction::Enter
        };
        let reason = record
            .skip_reason
            .clone()
            .or(record.exit_reason.clone())
            .unwrap_or_else(|| "model_decision".to_string());

        Some(Self {
            action,
            component,
            side,
            model_signal_ts_local: record.bar_ts_local,
            scheduled_entry_ts_local: record.scheduled_entry_ts_local,
            scheduled_exit_ts_local: record.scheduled_exit_ts_local,
            reason,
            overlap_decision: format!("{:?}", record.overlap_decision),
            shadow_pnl_points: record.shadow_pnl_points,
            decision_key,
        })
    }
}

impl RiAuthor4142JournalRecord {
    fn from_decision(
        decision: &RiAuthor4142ModelDecision,
        adapter_decision: RiAuthor4142JournalDecision,
        candidate: Option<&RiAuthor4142CandidateIntent>,
        reason_override: Option<&str>,
    ) -> Self {
        let cycle_id = format!(
            "{}:{}",
            decision.component.as_str(),
            decision.model_signal_ts_local.format("%Y%m%d%H%M%S")
        );
        Self {
            component: decision.component.as_str().to_string(),
            cycle_id,
            model_signal_ts_local: decision.model_signal_ts_local.to_string(),
            bar_ts_local: decision.model_signal_ts_local.to_string(),
            side: decision.side.map(|side| side.as_str().to_string()),
            role: candidate.map(|candidate| candidate.role.as_str().to_string()),
            entry_exit_reason: reason_override
                .unwrap_or(decision.reason.as_str())
                .to_string(),
            no_overlap_decision: decision.overlap_decision.clone(),
            adapter_decision,
            request_id: None,
            broker_order_id: None,
            position_before: None,
            position_after: None,
            candidate_order_side: candidate.map(|candidate| candidate.side),
            candidate_qty: candidate.map(|candidate| candidate.qty),
            candidate_order_style: candidate.map(|candidate| candidate.order_style.to_string()),
            candidate_intent_class: candidate.map(|candidate| candidate.intent_class),
            execution_path: candidate
                .map(|candidate| candidate.execution_path.as_str().to_string())
                .unwrap_or_else(|| "not_applicable_pre_go".to_string()),
            decision_key: decision.decision_key.clone(),
        }
    }
}

fn non_zero_or_close(value: f64, close: f64) -> f64 {
    if value.is_finite() && value != 0.0 {
        value
    } else {
        close
    }
}

fn all_finite_live(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn is_finalized_record(record: &ShadowJournalRecord, current_dt_local: NaiveDateTime) -> bool {
    let Some(exit_ts) = record.scheduled_exit_ts_local else {
        return false;
    };
    if exit_ts > current_dt_local {
        return false;
    }
    if record.exit_reason.as_deref() == Some("forced_last_bar_close")
        && exit_ts.date() == current_dt_local.date()
    {
        return false;
    }
    true
}

fn decision_key(record: &ShadowJournalRecord) -> String {
    format!(
        "{}|{}|{}|{:?}|{:?}|{}",
        record.profile_id.as_str(),
        match record.component {
            Component::Author41Mr => "author41_mr",
            Component::Author42Bo => "author42_bo",
            Component::Combo => "combo",
        },
        record.bar_ts_local,
        record.side,
        record.scheduled_exit_ts_local,
        record.exit_reason.as_deref().unwrap_or("none")
    )
}

#[cfg(test)]
mod tests {
    use super::{
        is_finalized_record, RiAuthor4142CandidateRole, RiAuthor4142DecisionAction,
        RiAuthor4142ExecutionPath, RiAuthor4142JournalDecision, RiAuthor4142LiveConfig,
        RiAuthor4142LiveStrategy, RiAuthor4142Phase, RiAuthor4142RuntimeMode, RiAuthor4142Side,
    };
    use crate::strategies::moex_author41_42::{
        Component, Instrument, OverlapDecision, ProfileId, ShadowJournalRecord, ShadowSide,
    };
    use crate::strategy_host::{
        BarEvent, BootstrapSnapshot, DataOrigin, OrderEvent, PositionEvent, RuntimeStateRestored,
        StopOrderEvent, Strategy,
    };
    use alor_protocol::{AckStatus, CommandAck, IntentClass, Side as OrderSide};
    use chrono::{Duration, NaiveDate, NaiveDateTime};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn default_config() -> RiAuthor4142LiveConfig {
        RiAuthor4142LiveConfig {
            symbol: "RIM6".to_string(),
            profile_id: "ri_author41_42_primary_combo_cost2".to_string(),
            timeframe: "10m".to_string(),
            mode: RiAuthor4142RuntimeMode::Shadow,
            allow_order_emission: false,
            execution_path: RiAuthor4142ExecutionPath::ActionScopedOnly,
            order_symbol: None,
            qty: 1.0,
            timezone_offset_hours: 3,
        }
    }

    #[test]
    fn shadow_mode_scaffold_cannot_emit_orders() {
        let strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        assert!(!strategy.can_emit_orders());
    }

    #[test]
    fn allow_order_emission_requires_micro_live_mode() {
        let mut config = default_config();
        config.allow_order_emission = true;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("order emission must be paired with micro_live")
            .to_string();
        assert!(err.contains("mode=micro_live and allow_order_emission=true"));
    }

    #[test]
    fn micro_live_mode_requires_order_emission_permission() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("micro_live must be paired with order emission")
            .to_string();
        assert!(err.contains("mode=micro_live and allow_order_emission=true"));
    }

    #[test]
    fn micro_live_with_order_emission_can_emit_orders() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;
        config.allow_order_emission = true;

        let strategy = RiAuthor4142LiveStrategy::new(config).expect("micro strategy");
        assert!(strategy.can_emit_orders());
    }

    #[test]
    fn micro_live_routes_orders_to_full_alor_symbol_when_configured() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;
        config.allow_order_emission = true;
        config.order_symbol = Some("RTS-6.26".to_string());
        let mut strategy = RiAuthor4142LiveStrategy::new(config).expect("micro strategy");

        let prev_day = bar_with_ohlc(
            dt(2026, 5, 1, 23, 40, 0),
            DataOrigin::History,
            100_000.0,
            101_000.0,
            99_000.0,
            100_000.0,
        );
        strategy.warmup_from_history(&live_ctx(), &[prev_day]);

        let entry_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 0, 0),
            DataOrigin::Live,
            100_050.0,
            100_120.0,
            100_030.0,
            100_100.0,
        );
        let entry_intents = strategy.on_bar(&live_ctx(), &entry_bar);

        assert_eq!(entry_intents.len(), 1);
        match &entry_intents[0] {
            crate::strategy_host::Intent::Routed { symbol, intent } => {
                assert_eq!(symbol, "RTS-6.26");
                assert!(matches!(
                    intent.base_intent(),
                    crate::strategy_host::Intent::Market { .. }
                ));
            }
            other => panic!("expected routed intent, got {other:?}"),
        }
        assert_eq!(entry_intents[0].explicit_class(), Some(IntentClass::Entry));
    }

    #[test]
    fn micro_live_rejected_entry_rolls_back_to_flat_when_broker_flat() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;
        config.allow_order_emission = true;
        let mut strategy = RiAuthor4142LiveStrategy::new(config).expect("micro strategy");

        let prev_day = bar_with_ohlc(
            dt(2026, 5, 1, 23, 40, 0),
            DataOrigin::History,
            100_000.0,
            101_000.0,
            99_000.0,
            100_000.0,
        );
        strategy.warmup_from_history(&live_ctx(), &[prev_day]);
        let entry_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 0, 0),
            DataOrigin::Live,
            100_050.0,
            100_120.0,
            100_030.0,
            100_100.0,
        );
        let entry_intents = strategy.on_bar(&live_ctx(), &entry_bar);
        assert_eq!(entry_intents.len(), 1);
        assert_eq!(strategy.phase_for_test().as_deref(), Some("live_in_position"));

        strategy.on_ack(
            &live_ctx(),
            &CommandAck {
                request_id: Uuid::new_v4(),
                status: AckStatus::Rejected,
                broker_order_id: None,
                broker_order_id_str: None,
                error_code: Some("cws_http_400".to_string()),
                error_msg: Some("unknown instrument".to_string()),
                cws_http_code: Some(400),
                cws_message: Some("unknown instrument".to_string()),
                cws_request_guid: None,
                processed_ts_utc: 1_776_000_000,
            },
        );

        assert_eq!(strategy.phase_for_test().as_deref(), Some("flat"));
        assert!(strategy.live_mr.position.is_none());
        assert!(strategy.live_bo.position.is_none());
    }

    #[test]
    fn micro_live_emits_author41_entry_and_exit_as_action_scoped_market_intents() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;
        config.allow_order_emission = true;
        let mut strategy = RiAuthor4142LiveStrategy::new(config).expect("micro strategy");

        let prev_day = bar_with_ohlc(
            dt(2026, 5, 1, 23, 40, 0),
            DataOrigin::History,
            100_000.0,
            101_000.0,
            99_000.0,
            100_000.0,
        );
        strategy.warmup_from_history(&live_ctx(), &[prev_day]);

        let entry_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 0, 0),
            DataOrigin::Live,
            100_050.0,
            100_120.0,
            100_030.0,
            100_100.0,
        );
        let entry_intents = strategy.on_bar(&live_ctx(), &entry_bar);

        assert_eq!(entry_intents.len(), 1);
        match entry_intents[0].base_intent() {
            crate::strategy_host::Intent::Market {
                side, qty, comment, ..
            } => {
                assert_eq!(*side, OrderSide::Sell);
                assert_eq!(*qty, 1.0);
                assert!(comment
                    .as_deref()
                    .unwrap_or("")
                    .contains("author41_mr:entry"));
            }
            other => panic!("unexpected entry intent: {other:?}"),
        }
        assert_eq!(entry_intents[0].explicit_class(), Some(IntentClass::Entry));

        let exit_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 10, 0),
            DataOrigin::Live,
            100_090.0,
            100_100.0,
            99_890.0,
            99_900.0,
        );
        let exit_intents = strategy.on_bar(&live_ctx(), &exit_bar);

        assert_eq!(exit_intents.len(), 1);
        match exit_intents[0].base_intent() {
            crate::strategy_host::Intent::Market {
                side, qty, comment, ..
            } => {
                assert_eq!(*side, OrderSide::Buy);
                assert_eq!(*qty, 1.0);
                assert!(comment
                    .as_deref()
                    .unwrap_or("")
                    .contains("author41_mr:exit"));
            }
            other => panic!("unexpected exit intent: {other:?}"),
        }
        assert_eq!(exit_intents[0].explicit_class(), Some(IntentClass::Exit));
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
        assert!(strategy
            .journal_records_for_test()
            .iter()
            .any(|row| row.adapter_decision == RiAuthor4142JournalDecision::IntentEmitted));
    }

    #[test]
    fn non_10m_timeframe_is_rejected() {
        let mut config = default_config();
        config.timeframe = "1m".to_string();

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("RI frozen model must stay on 10m")
            .to_string();
        assert!(err.contains("frozen model requires 10m timeframe"));
    }

    #[test]
    fn legacy_execution_path_is_rejected() {
        let err = RiAuthor4142ExecutionPath::parse("legacy_long_lived")
            .expect_err("legacy cws path must not be accepted")
            .to_string();

        assert!(err.contains("unsupported ri_author41_42 execution_path"));
    }

    #[test]
    fn model_feed_guard_excludes_service_weekend_and_history_gap_bars() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");

        let service_bar = bar(dt(2026, 5, 1, 8, 50, 0), DataOrigin::Live);
        let weekend_bar = bar(dt(2026, 5, 2, 10, 0, 0), DataOrigin::Live);
        let history_gap_bar = bar(dt(2026, 5, 1, 10, 0, 0), DataOrigin::HistoryGap);
        let regular_bar = bar(dt(2026, 5, 1, 10, 10, 0), DataOrigin::Live);

        assert!(strategy.update_bar_state(&service_bar).is_empty());
        assert!(strategy.update_bar_state(&weekend_bar).is_empty());
        assert!(strategy.update_bar_state(&history_gap_bar).is_empty());
        assert!(strategy.update_bar_state(&regular_bar).is_empty());

        assert_eq!(strategy.model_bars.len(), 1);
        assert_eq!(strategy.model_bars[0].ts_local, dt(2026, 5, 1, 10, 10, 0));
        if let crate::state::StrategyState::RiAuthor4142Live {
            model_bars_seen,
            suppressed_service_bars,
            ..
        } = &strategy.state
        {
            assert_eq!(*model_bars_seen, 1);
            assert_eq!(*suppressed_service_bars, 3);
        } else {
            panic!("unexpected strategy state");
        }
    }

    #[test]
    fn dropped_overlap_record_becomes_suppress_decision() {
        let record = sample_record(OverlapDecision::DroppedMrOverlap, "mr_interval_overlap");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        assert_eq!(decision.action, RiAuthor4142DecisionAction::Suppress);
        assert_eq!(decision.reason, "mr_interval_overlap");
    }

    #[test]
    fn same_day_forced_tail_is_not_finalized() {
        let record = sample_record(OverlapDecision::Accepted, "forced_last_bar_close");
        let current_dt = dt(2026, 5, 1, 23, 40, 0);

        assert!(!is_finalized_record(&record, current_dt));
    }

    #[test]
    fn previous_day_forced_tail_is_finalized() {
        let record = sample_record(OverlapDecision::Accepted, "forced_last_bar_close");
        let current_dt = dt(2026, 5, 2, 9, 0, 0);

        assert!(is_finalized_record(&record, current_dt));
    }

    #[test]
    fn dry_run_enter_decision_round_trips_to_flat() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::Accepted, "time_exit_same_bar_close");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        strategy.apply_dry_run_decision(&decision);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
    }

    #[test]
    fn suppress_decision_keeps_flat_phase() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::DroppedMrOverlap, "mr_interval_overlap");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        strategy.apply_dry_run_decision(&decision);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
    }

    #[test]
    fn accepted_decision_builds_entry_and_exit_candidates() {
        let strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::Accepted, "time_exit_same_bar_close");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        let candidates = strategy.candidate_intents_for_decision(&decision);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].role, RiAuthor4142CandidateRole::Entry);
        assert_eq!(candidates[0].side, OrderSide::Sell);
        assert_eq!(candidates[0].qty, 1.0);
        assert_eq!(candidates[0].order_style, "market_p0");
        assert_eq!(candidates[0].intent_class, IntentClass::Entry);
        assert_eq!(
            candidates[0].execution_path,
            RiAuthor4142ExecutionPath::ActionScopedOnly
        );
        assert!(candidates[0].comment.contains("author42_bo:entry"));
        assert_eq!(candidates[1].role, RiAuthor4142CandidateRole::Exit);
        assert_eq!(candidates[1].side, OrderSide::Buy);
        assert_eq!(candidates[1].intent_class, IntentClass::Exit);
        assert_eq!(
            candidates[1].execution_path,
            RiAuthor4142ExecutionPath::ActionScopedOnly
        );
        assert!(candidates[1].comment.contains("author42_bo:exit"));
    }

    #[test]
    fn candidate_adapter_keeps_all_roles_action_scoped_only() {
        let strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let cases = [
            (Component::Author41Mr, ShadowSide::Long, "author41_mr"),
            (Component::Author42Bo, ShadowSide::Short, "author42_bo"),
        ];

        for (component, side, component_label) in cases {
            let mut record = sample_record(OverlapDecision::Accepted, "time_exit_same_bar_close");
            record.component = component;
            record.side = Some(side);
            let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
                record,
                format!("{component_label}-decision-key"),
            )
            .expect("decision");

            let candidates = strategy.candidate_intents_for_decision(&decision);

            assert_eq!(candidates.len(), 2);
            assert_eq!(decision.side, Some(RiAuthor4142Side::from_shadow(side)));
            for candidate in candidates {
                assert_eq!(
                    candidate.execution_path,
                    RiAuthor4142ExecutionPath::ActionScopedOnly
                );
                assert_eq!(candidate.order_style, "market_p0");
                assert!(candidate.comment.contains(component_label));
                match candidate.role {
                    RiAuthor4142CandidateRole::Entry => {
                        assert_eq!(candidate.intent_class, IntentClass::Entry);
                    }
                    RiAuthor4142CandidateRole::Exit => {
                        assert_eq!(candidate.intent_class, IntentClass::Exit);
                    }
                }
            }
        }
    }

    #[test]
    fn cross_day_decision_requires_manual_intervention_before_candidate_lifecycle() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let mut record = sample_record(OverlapDecision::Accepted, "forced_last_bar_close");
        record.scheduled_exit_ts_local = Some(dt(2026, 5, 2, 9, 0, 0));
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "cross-day-decision-key".to_string(),
        )
        .expect("decision");

        strategy.apply_dry_run_decision(&decision);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::ManualInterventionRequired.as_str())
        );
        let journal = strategy.journal_records_for_test();
        assert_eq!(journal.len(), 1);
        assert_eq!(
            journal[0].adapter_decision,
            RiAuthor4142JournalDecision::ManualInterventionRequired
        );
        assert_eq!(journal[0].entry_exit_reason, "cross_day_exit_not_allowed");
        assert_eq!(journal[0].role, None);
        assert_eq!(journal[0].request_id, None);
    }

    #[test]
    fn suppressed_decision_builds_no_candidate_intents() {
        let strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::DroppedMrOverlap, "mr_interval_overlap");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        let candidates = strategy.candidate_intents_for_decision(&decision);

        assert!(candidates.is_empty());
    }

    #[test]
    fn accepted_decision_records_shadow_and_candidate_journal_entries() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::Accepted, "time_exit_same_bar_close");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        strategy.record_shadow_journal(&decision);
        strategy.apply_dry_run_decision(&decision);

        let journal = strategy.journal_records_for_test();
        assert_eq!(journal.len(), 3);
        assert_eq!(
            journal[0].adapter_decision,
            RiAuthor4142JournalDecision::ShadowRecorded
        );
        assert_eq!(journal[0].component, "author42_bo");
        assert_eq!(journal[0].cycle_id, "author42_bo:20260501130000");
        assert_eq!(journal[0].side.as_deref(), Some("short"));
        assert_eq!(journal[0].role, None);
        assert_eq!(journal[0].request_id, None);
        assert_eq!(journal[0].broker_order_id, None);
        assert_eq!(
            journal[1].adapter_decision,
            RiAuthor4142JournalDecision::IntentSuppressed
        );
        assert_eq!(journal[1].role.as_deref(), Some("entry"));
        assert_eq!(journal[1].candidate_order_side, Some(OrderSide::Sell));
        assert_eq!(journal[1].candidate_intent_class, Some(IntentClass::Entry));
        assert_eq!(journal[1].execution_path, "action_scoped_only");
        assert_eq!(journal[2].role.as_deref(), Some("exit"));
        assert_eq!(journal[2].candidate_order_side, Some(OrderSide::Buy));
        assert_eq!(journal[2].candidate_intent_class, Some(IntentClass::Exit));
    }

    #[test]
    fn suppressed_decision_records_shadow_and_suppression_journal_entries() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::DroppedMrOverlap, "mr_interval_overlap");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        strategy.record_shadow_journal(&decision);
        strategy.apply_dry_run_decision(&decision);

        let journal = strategy.journal_records_for_test();
        assert_eq!(journal.len(), 2);
        assert_eq!(
            journal[0].adapter_decision,
            RiAuthor4142JournalDecision::ShadowRecorded
        );
        assert_eq!(
            journal[1].adapter_decision,
            RiAuthor4142JournalDecision::IntentSuppressed
        );
        assert_eq!(journal[1].role, None);
        assert_eq!(journal[1].entry_exit_reason, "mr_interval_overlap");
        assert_eq!(journal[1].candidate_order_side, None);
    }

    #[test]
    fn drain_observation_journal_records_serializes_and_clears_buffer() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let record = sample_record(OverlapDecision::Accepted, "time_exit_same_bar_close");
        let decision = super::RiAuthor4142ModelDecision::from_shadow_record(
            record,
            "decision-key".to_string(),
        )
        .expect("decision");

        strategy.record_shadow_journal(&decision);
        strategy.apply_dry_run_decision(&decision);
        let drained = Strategy::drain_observation_journal_records(&mut strategy);

        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0]["adapter_decision"], "shadow_recorded");
        assert_eq!(drained[1]["adapter_decision"], "intent_suppressed");
        assert_eq!(drained[1]["role"], "entry");
        assert!(strategy.journal_records_for_test().is_empty());
    }

    #[test]
    fn bootstrap_flat_without_working_orders_keeps_flat_phase() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let snapshot = BootstrapSnapshot {
            positions_strategy: HashMap::new(),
            working_orders_strategy: HashMap::new(),
            working_stop_orders_strategy: HashMap::new(),
            snapshot_ts_utc: Some(1_776_000_000),
        };

        let _ = strategy.on_bootstrap_snapshot(&test_ctx(), &snapshot);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
    }

    #[test]
    fn bootstrap_non_flat_position_requires_manual_intervention() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let mut positions_strategy = HashMap::new();
        positions_strategy.insert(
            "RIM6".to_string(),
            PositionEvent {
                symbol: "RIM6".to_string(),
                qty: -1.0,
                existing: true,
                avg_price: 100_000.0,
                ts_utc: 1_776_000_000,
            },
        );
        let snapshot = BootstrapSnapshot {
            positions_strategy,
            working_orders_strategy: HashMap::new(),
            working_stop_orders_strategy: HashMap::new(),
            snapshot_ts_utc: Some(1_776_000_000),
        };

        let _ = strategy.on_bootstrap_snapshot(&test_ctx(), &snapshot);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::ManualInterventionRequired.as_str())
        );
    }

    #[test]
    fn bootstrap_working_orders_require_manual_intervention() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let mut working_orders_strategy = HashMap::new();
        working_orders_strategy.insert(
            42,
            OrderEvent {
                order_id: 42,
                symbol: "RIM6".to_string(),
                status: "working".to_string(),
                side: "buy".to_string(),
                qty: 1.0,
                existing: true,
                ..OrderEvent::default()
            },
        );
        let snapshot = BootstrapSnapshot {
            positions_strategy: HashMap::new(),
            working_orders_strategy,
            working_stop_orders_strategy: HashMap::new(),
            snapshot_ts_utc: Some(1_776_000_000),
        };

        let _ = strategy.on_bootstrap_snapshot(&test_ctx(), &snapshot);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::ManualInterventionRequired.as_str())
        );
    }

    #[test]
    fn bootstrap_working_stop_orders_require_manual_intervention() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let mut working_stop_orders_strategy = HashMap::new();
        working_stop_orders_strategy.insert(
            "sl-42".to_string(),
            StopOrderEvent {
                stop_order_id: "sl-42".to_string(),
                symbol: "RIM6".to_string(),
                status: "working".to_string(),
                side: "sell".to_string(),
                qty: 1.0,
                existing: true,
                exchange_order_id: None,
                filled: 0.0,
                stop_price: 99_000.0,
                price: 98_990.0,
                comment: None,
                end_time: None,
                ts_utc: 1_776_000_000,
            },
        );
        let snapshot = BootstrapSnapshot {
            positions_strategy: HashMap::new(),
            working_orders_strategy: HashMap::new(),
            working_stop_orders_strategy,
            snapshot_ts_utc: Some(1_776_000_000),
        };

        let _ = strategy.on_bootstrap_snapshot(&test_ctx(), &snapshot);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::ManualInterventionRequired.as_str())
        );
    }

    #[test]
    fn runtime_state_restore_empty_keeps_flat_phase() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let state = RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: Vec::new(),
        };

        let intents = strategy.on_runtime_state_restored(&test_ctx(), &state);

        assert!(intents.is_empty());
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
    }

    #[test]
    fn runtime_state_restore_pending_requests_require_manual_intervention() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let state = RuntimeStateRestored {
            known_order_ids: Vec::new(),
            pending_requests: vec![Uuid::nil()],
        };

        let intents = strategy.on_runtime_state_restored(&test_ctx(), &state);

        assert!(intents.is_empty());
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::ManualInterventionRequired.as_str())
        );
    }

    #[test]
    fn runtime_state_restore_known_orders_require_manual_intervention() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let state = RuntimeStateRestored {
            known_order_ids: vec![42],
            pending_requests: Vec::new(),
        };

        let intents = strategy.on_runtime_state_restored(&test_ctx(), &state);

        assert!(intents.is_empty());
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::ManualInterventionRequired.as_str())
        );
    }

    fn sample_record(overlap_decision: OverlapDecision, reason: &str) -> ShadowJournalRecord {
        ShadowJournalRecord {
            instrument: Instrument::Ri,
            profile_id: ProfileId::RiAuthor41_42PrimaryComboCost2,
            component: Component::Author42Bo,
            model_variant_id: "grid_k0.42_both".to_string(),
            bar_ts_local: dt(2026, 5, 1, 13, 0, 0),
            timeframe: "10m".to_string(),
            prev_regular_date: Some(NaiveDate::from_ymd_opt(2026, 4, 30).unwrap()),
            prev_close: Some(100_000.0),
            prev_high: Some(101_000.0),
            prev_low: Some(99_000.0),
            prev_range: Some(2_000.0),
            trigger_long: None,
            trigger_short: None,
            condition_values: Vec::new(),
            side: Some(ShadowSide::Short),
            skip_reason: (overlap_decision == OverlapDecision::DroppedMrOverlap)
                .then(|| reason.to_string()),
            scheduled_entry_ts_local: Some(dt(2026, 5, 1, 13, 0, 0)),
            scheduled_entry_price: Some(100_000.0),
            scheduled_exit_ts_local: Some(dt(2026, 5, 1, 23, 40, 0)),
            exit_reason: Some(reason.to_string()),
            overlap_decision,
            shadow_pnl_points: Some(10.0),
            feed_quality_flags: Vec::new(),
        }
    }

    fn dt(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, second)
            .unwrap()
    }

    fn bar(dt_local: NaiveDateTime, origin: DataOrigin) -> BarEvent {
        bar_with_ohlc(dt_local, origin, 99_990.0, 100_010.0, 99_980.0, 100_000.0)
    }

    fn bar_with_ohlc(
        dt_local: NaiveDateTime,
        origin: DataOrigin,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    ) -> BarEvent {
        BarEvent {
            symbol: "RIM6".to_string(),
            close_time_utc: (dt_local - Duration::hours(3)).and_utc().timestamp(),
            close,
            o: open,
            h: high,
            l: low,
            v: 10.0,
            origin,
        }
    }

    fn test_ctx() -> crate::StrategyCtx {
        crate::StrategyCtx {
            strategy_id: "ri_author41_42.shadow.test".to_string(),
            portfolio: "demo".to_string(),
            exchange: "MOEX".to_string(),
            symbol: "RIM6".to_string(),
            tick_size: 10.0,
            trade_mode: crate::TradeMode::Live,
            paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
            allow_live_orders: false,
            gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
            position_qty: None,
            event_ts_utc: 1_776_000_000,
            now_ts_utc: 1_776_000_000,
            last_bar_ts: None,
        }
    }

    fn live_ctx() -> crate::StrategyCtx {
        crate::StrategyCtx {
            allow_live_orders: true,
            ..test_ctx()
        }
    }
}
