use std::collections::HashSet;

use anyhow::{bail, Result};
use chrono::{NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state::StrategyState;
use crate::strategies::moex_author41_42::{
    build_ri_author41_42_combo_shadow_journal, Component, ModelBar, ModelProfile, OverlapDecision,
    RegularSessionPolicy, ShadowJournalRecord, ShadowSide,
};
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
    RuntimeStateRestored, Strategy, StrategyCtx,
};
use alor_protocol::{IntentClass, Side as OrderSide};

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142JournalDecision {
    ShadowRecorded,
    IntentSuppressed,
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
    pub fn validate_pre_go(&self) -> Result<()> {
        if self.execution_path != RiAuthor4142ExecutionPath::ActionScopedOnly {
            bail!("ri_author41_42 requires action_scoped_only execution");
        }
        if self.allow_order_emission {
            bail!("ri_author41_42 live order emission is blocked until GO/NO-GO is approved");
        }
        if self.mode.can_emit_orders() {
            bail!("ri_author41_42 micro_live mode is blocked until GO/NO-GO is approved");
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
    state: StrategyState,
}

impl RiAuthor4142LiveStrategy {
    pub fn new(config: RiAuthor4142LiveConfig) -> Result<Self> {
        config.validate_pre_go()?;
        let profile = ModelProfile::ri_shadow_10m();
        let session_policy = profile.session_policy;
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
                live_adapter_enabled: false,
            },
            config,
            session_policy,
            model_bars: Vec::new(),
            emitted_decision_keys: HashSet::new(),
            journal_records: Vec::new(),
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
            live_adapter_enabled = false,
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
            self.apply_dry_run_decision(&decision);
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
            live_adapter_enabled = false,
        );
    }

    fn apply_dry_run_decision(&mut self, decision: &RiAuthor4142ModelDecision) {
        match decision.action {
            RiAuthor4142DecisionAction::Suppress => {
                self.record_suppressed_journal(decision);
                self.log_transition("suppressed", "ri_intent_suppressed", decision);
            }
            RiAuthor4142DecisionAction::Enter => {
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
            live_adapter_enabled = false,
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
            live_adapter_enabled = false,
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
            live_adapter_enabled = false,
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
                live_adapter_enabled = false,
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
                live_adapter_enabled = false,
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
            live_adapter_enabled = false,
        );
    }

    pub fn can_emit_orders(&self) -> bool {
        self.config.mode.can_emit_orders() && self.config.allow_order_emission
    }

    pub fn phase_for_test(&self) -> Option<String> {
        match &self.state {
            StrategyState::RiAuthor4142Live { phase, .. } => Some(phase.clone()),
            _ => None,
        }
    }

    pub fn journal_records_for_test(&self) -> &[RiAuthor4142JournalRecord] {
        &self.journal_records
    }
}

impl Strategy for RiAuthor4142LiveStrategy {
    fn on_bar(&mut self, _ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        let _decisions = self.update_bar_state(bar);
        Vec::new()
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &alor_protocol::CommandAck) -> Vec<Intent> {
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
    if value != 0.0 {
        value
    } else {
        close
    }
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
        BootstrapSnapshot, OrderEvent, PositionEvent, RuntimeStateRestored, StopOrderEvent,
        Strategy,
    };
    use alor_protocol::{IntentClass, Side as OrderSide};
    use chrono::{NaiveDate, NaiveDateTime};
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
    fn allow_order_emission_is_blocked_until_go() {
        let mut config = default_config();
        config.allow_order_emission = true;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("order emission must remain blocked")
            .to_string();
        assert!(err.contains("blocked until GO/NO-GO"));
    }

    #[test]
    fn micro_live_mode_is_blocked_until_go() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("micro_live must remain blocked")
            .to_string();
        assert!(err.contains("micro_live mode is blocked"));
    }

    #[test]
    fn legacy_execution_path_is_rejected() {
        let err = RiAuthor4142ExecutionPath::parse("legacy_long_lived")
            .expect_err("legacy cws path must not be accepted")
            .to_string();

        assert!(err.contains("unsupported ri_author41_42 execution_path"));
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
}
