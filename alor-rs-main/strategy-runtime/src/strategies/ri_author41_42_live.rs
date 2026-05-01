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
                live_adapter_enabled: false,
            },
            config,
            session_policy,
            model_bars: Vec::new(),
            emitted_decision_keys: HashSet::new(),
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
            self.log_decision(&decision);
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

    pub fn can_emit_orders(&self) -> bool {
        self.config.mode.can_emit_orders() && self.config.allow_order_emission
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
        _snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        let _ = state;
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
        is_finalized_record, RiAuthor4142DecisionAction, RiAuthor4142ExecutionPath,
        RiAuthor4142LiveConfig, RiAuthor4142LiveStrategy, RiAuthor4142RuntimeMode,
    };
    use crate::strategies::moex_author41_42::{
        Component, Instrument, OverlapDecision, ProfileId, ShadowJournalRecord, ShadowSide,
    };
    use chrono::{NaiveDate, NaiveDateTime};

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
}
