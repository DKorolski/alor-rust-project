use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::state::StrategyState;
use crate::strategies::moex_author41_42::{
    build_ri_author41_42_combo_shadow_journal_with_configs, Author41Config, Author42Config,
    Author42SideMode, Component, ModelBar, ModelProfile, OverlapDecision, RegularSessionPolicy,
    ShadowJournalRecord, ShadowSide,
};
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, CommandPrepared, DataOrigin, Intent, IntentBlockDisposition,
    IntentBlocked, OrderEvent, PositionEvent, RuntimeStateRestored, Strategy, StrategyCtx,
};
use alor_protocol::{AckStatus, IntentClass, Side as OrderSide};
use uuid::Uuid;

fn parse_ri_time(field: &'static str, raw: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(raw, "%H:%M:%S")
        .map_err(|err| anyhow::anyhow!("invalid ri_author41_42 {field} {raw}: {err}"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142RuntimeMode {
    Shadow,
    ProspectiveShadow,
    DryRun,
    MicroLive,
}

impl RiAuthor4142RuntimeMode {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "shadow" | "shadow_only" => Ok(Self::Shadow),
            "prospective_shadow" | "shadow_prospective" => Ok(Self::ProspectiveShadow),
            "dry_run" | "dryrun" => Ok(Self::DryRun),
            "micro_live" | "live_micro" => Ok(Self::MicroLive),
            other => bail!("unsupported ri_author41_42 mode: {other}"),
        }
    }

    pub fn can_emit_orders(self) -> bool {
        matches!(self, Self::MicroLive)
    }

    pub fn runs_prospective_adapter(self) -> bool {
        matches!(self, Self::ProspectiveShadow)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::ProspectiveShadow => "prospective_shadow",
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
    pub session_start_time: String,
    pub session_end_time: String,
    pub author41_entry_end_time: String,
    pub author41_time_exit: String,
    pub author42_exit_time: String,
    pub excluded_model_dates: Vec<String>,
    pub min_anchor_bars: usize,
    pub anchor_first_bar_at_or_before: String,
    pub anchor_last_bar_at_or_after: String,
    pub anchor_transition_date: Option<String>,
    pub pre_transition_min_anchor_bars: Option<usize>,
    pub pre_transition_anchor_first_bar_at_or_before: Option<String>,
    pub pre_transition_anchor_last_bar_at_or_after: Option<String>,
    pub actual_expiry_date: Option<String>,
    pub roll_target_sessions_before: u32,
    pub roll_fallback_sessions_before: u32,
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
    LiveEntryMissedRuntimeNotReady,
    ManualInterventionRequired,
}

impl RiAuthor4142Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::DryRunInPosition => "dry_run_in_position",
            Self::LiveEntryMissedRuntimeNotReady => "live_entry_missed_runtime_not_ready",
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
    bars: usize,
    first_bar: NaiveTime,
    last_bar: NaiveTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RiAnchorCompletenessRule {
    min_bars: usize,
    first_bar_at_or_before: NaiveTime,
    last_bar_at_or_after: NaiveTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RiAnchorTransitionRule {
    starts_on: NaiveDate,
    pre_transition: RiAnchorCompletenessRule,
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
    ShadowPathActive,
    ShadowPathSuperseded,
    ProspectiveIntentSuppressed,
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
    pub candidate_scheduled_ts_local: Option<String>,
    pub execution_path: String,
    pub decision_key: String,
    pub shadow_pnl_points: Option<f64>,
}

impl RiAuthor4142LiveConfig {
    pub fn can_emit_orders(&self) -> bool {
        self.mode.can_emit_orders() && self.allow_order_emission
    }

    pub fn runs_prospective_adapter(&self) -> bool {
        self.mode.runs_prospective_adapter() && !self.allow_order_emission
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
        for raw in &self.excluded_model_dates {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|err| {
                anyhow::anyhow!("invalid excluded_model_dates value {raw}: {err}")
            })?;
        }
        let session_start = parse_ri_time("session_start_time", &self.session_start_time)?;
        let session_end = parse_ri_time("session_end_time", &self.session_end_time)?;
        if session_start >= session_end {
            bail!(
                "ri_author41_42 requires session_start_time < session_end_time, got {} >= {}",
                self.session_start_time,
                self.session_end_time
            );
        }
        parse_ri_time("author41_entry_end_time", &self.author41_entry_end_time)?;
        parse_ri_time("author41_time_exit", &self.author41_time_exit)?;
        parse_ri_time("author42_exit_time", &self.author42_exit_time)?;
        NaiveTime::parse_from_str(&self.anchor_first_bar_at_or_before, "%H:%M:%S").map_err(
            |err| {
                anyhow::anyhow!(
                    "invalid anchor_first_bar_at_or_before {}: {err}",
                    self.anchor_first_bar_at_or_before
                )
            },
        )?;
        NaiveTime::parse_from_str(&self.anchor_last_bar_at_or_after, "%H:%M:%S").map_err(
            |err| {
                anyhow::anyhow!(
                    "invalid anchor_last_bar_at_or_after {}: {err}",
                    self.anchor_last_bar_at_or_after
                )
            },
        )?;
        let transition_fields_configured = self.anchor_transition_date.is_some()
            || self.pre_transition_min_anchor_bars.is_some()
            || self.pre_transition_anchor_first_bar_at_or_before.is_some()
            || self.pre_transition_anchor_last_bar_at_or_after.is_some();
        if transition_fields_configured {
            let transition_date = self.anchor_transition_date.as_deref().ok_or_else(|| {
                anyhow::anyhow!("anchor transition requires anchor_transition_date")
            })?;
            NaiveDate::parse_from_str(transition_date, "%Y-%m-%d").map_err(|err| {
                anyhow::anyhow!("invalid anchor_transition_date {transition_date}: {err}")
            })?;
            if self.pre_transition_min_anchor_bars.is_none()
                || self.pre_transition_anchor_first_bar_at_or_before.is_none()
                || self.pre_transition_anchor_last_bar_at_or_after.is_none()
            {
                bail!(
                    "anchor transition requires pre_transition_min_anchor_bars and both pre_transition anchor times"
                );
            }
            let first = self
                .pre_transition_anchor_first_bar_at_or_before
                .as_deref()
                .expect("validated above");
            NaiveTime::parse_from_str(first, "%H:%M:%S").map_err(|err| {
                anyhow::anyhow!(
                    "invalid pre_transition_anchor_first_bar_at_or_before {first}: {err}"
                )
            })?;
            let last = self
                .pre_transition_anchor_last_bar_at_or_after
                .as_deref()
                .expect("validated above");
            NaiveTime::parse_from_str(last, "%H:%M:%S").map_err(|err| {
                anyhow::anyhow!("invalid pre_transition_anchor_last_bar_at_or_after {last}: {err}")
            })?;
        }
        if let Some(raw) = &self.actual_expiry_date {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|err| anyhow::anyhow!("invalid actual_expiry_date {raw}: {err}"))?;
        }
        if self.roll_target_sessions_before == 0
            || self.roll_fallback_sessions_before < self.roll_target_sessions_before
        {
            bail!(
                "ri_author41_42 rollover requires target_sessions_before > 0 and fallback >= target"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RiAuthor4142LiveStrategy {
    config: RiAuthor4142LiveConfig,
    session_policy: RegularSessionPolicy,
    excluded_model_dates: HashSet<NaiveDate>,
    canonical_anchor_rule: RiAnchorCompletenessRule,
    anchor_transition_rule: Option<RiAnchorTransitionRule>,
    author41_entry_end: NaiveTime,
    author41_time_exit: NaiveTime,
    author42_exit_time: NaiveTime,
    logged_excluded_dates: HashSet<NaiveDate>,
    logged_anchor_context_dates: HashSet<NaiveDate>,
    model_bars: Vec<ModelBar>,
    emitted_decision_keys: HashSet<String>,
    shadow_path_keys_by_date: BTreeMap<NaiveDate, BTreeSet<String>>,
    shadow_decisions_by_key: BTreeMap<String, RiAuthor4142ModelDecision>,
    journal_records: Vec<RiAuthor4142JournalRecord>,
    live_mr: RiAuthor4142LiveMrState,
    live_bo: RiAuthor4142LiveBoState,
    state: StrategyState,
}

impl RiAuthor4142LiveStrategy {
    pub fn new(config: RiAuthor4142LiveConfig) -> Result<Self> {
        config.validate()?;
        let profile = ModelProfile::ri_shadow_10m();
        let session_start = parse_ri_time("session_start_time", &config.session_start_time)?;
        let session_end = parse_ri_time("session_end_time", &config.session_end_time)?;
        let session_policy = RegularSessionPolicy::new(session_start, session_end, true);
        let live_adapter_enabled = config.can_emit_orders();
        let excluded_model_dates = config
            .excluded_model_dates
            .iter()
            .map(|raw| NaiveDate::parse_from_str(raw, "%Y-%m-%d"))
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        let canonical_anchor_rule = RiAnchorCompletenessRule {
            min_bars: config.min_anchor_bars,
            first_bar_at_or_before: NaiveTime::parse_from_str(
                &config.anchor_first_bar_at_or_before,
                "%H:%M:%S",
            )?,
            last_bar_at_or_after: NaiveTime::parse_from_str(
                &config.anchor_last_bar_at_or_after,
                "%H:%M:%S",
            )?,
        };
        let anchor_transition_rule = match config.anchor_transition_date.as_deref() {
            None => None,
            Some(starts_on) => Some(RiAnchorTransitionRule {
                starts_on: NaiveDate::parse_from_str(starts_on, "%Y-%m-%d")?,
                pre_transition: RiAnchorCompletenessRule {
                    min_bars: config
                        .pre_transition_min_anchor_bars
                        .expect("validated transition configuration"),
                    first_bar_at_or_before: NaiveTime::parse_from_str(
                        config
                            .pre_transition_anchor_first_bar_at_or_before
                            .as_deref()
                            .expect("validated transition configuration"),
                        "%H:%M:%S",
                    )?,
                    last_bar_at_or_after: NaiveTime::parse_from_str(
                        config
                            .pre_transition_anchor_last_bar_at_or_after
                            .as_deref()
                            .expect("validated transition configuration"),
                        "%H:%M:%S",
                    )?,
                },
            }),
        };
        let author41_entry_end =
            parse_ri_time("author41_entry_end_time", &config.author41_entry_end_time)?;
        let author41_time_exit = parse_ri_time("author41_time_exit", &config.author41_time_exit)?;
        let author42_exit_time = parse_ri_time("author42_exit_time", &config.author42_exit_time)?;
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_model_policy_loaded",
            active_contract = %config.symbol,
            order_symbol = ?config.order_symbol,
            profile_id = %profile.profile_id.as_str(),
            model_session_start = %config.session_start_time,
            model_session_end = %config.session_end_time,
            author41_entry_end = %config.author41_entry_end_time,
            author41_time_exit = %config.author41_time_exit,
            author42_exit_time = %config.author42_exit_time,
            actual_expiry_date = ?config.actual_expiry_date,
            roll_target_sessions_before = config.roll_target_sessions_before,
            roll_fallback_sessions_before = config.roll_fallback_sessions_before,
            excluded_model_dates = ?config.excluded_model_dates,
            min_anchor_bars = config.min_anchor_bars,
            anchor_first_bar_at_or_before = %config.anchor_first_bar_at_or_before,
            anchor_last_bar_at_or_after = %config.anchor_last_bar_at_or_after,
            anchor_transition_date = ?config.anchor_transition_date,
            pre_transition_min_anchor_bars = ?config.pre_transition_min_anchor_bars,
            pre_transition_anchor_first_bar_at_or_before = ?config.pre_transition_anchor_first_bar_at_or_before,
            pre_transition_anchor_last_bar_at_or_after = ?config.pre_transition_anchor_last_bar_at_or_after,
            mode = config.mode.as_str(),
            allow_order_emission = config.allow_order_emission,
            prospective_adapter_enabled = config.runs_prospective_adapter(),
        );
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
                pending_entry_request_id: None,
                pending_exit_request_id: None,
            },
            config,
            session_policy,
            excluded_model_dates,
            canonical_anchor_rule,
            anchor_transition_rule,
            author41_entry_end,
            author41_time_exit,
            author42_exit_time,
            logged_excluded_dates: HashSet::new(),
            logged_anchor_context_dates: HashSet::new(),
            model_bars: Vec::new(),
            emitted_decision_keys: HashSet::new(),
            shadow_path_keys_by_date: BTreeMap::new(),
            shadow_decisions_by_key: BTreeMap::new(),
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
        let excluded_date = self.excluded_model_dates.contains(&dt_local.date());
        let is_model_bar = self.session_policy.is_model_bar(dt_local)
            && !self.is_operational_shadow_break_bar(dt_local)
            && !self.is_pre_transition_service_bar(dt_local)
            && !excluded_date
            && bar.origin != DataOrigin::HistoryGap;
        if excluded_date && self.logged_excluded_dates.insert(dt_local.date()) {
            info!(
                target: "strategy_runtime::ri_author41_42_live",
                action = "ri_model_session_excluded",
                calendar_date = %dt_local.date(),
                reason = "configured_non_regular_session",
                active_contract = %self.config.symbol,
            );
        }

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

        debug!(
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

    fn is_operational_shadow_break_bar(&self, dt_local: NaiveDateTime) -> bool {
        if !matches!(
            self.config.mode,
            RiAuthor4142RuntimeMode::Shadow | RiAuthor4142RuntimeMode::ProspectiveShadow
        ) {
            return false;
        }

        let bar_time = dt_local.time();
        (bar_time >= NaiveTime::from_hms_opt(14, 0, 0).unwrap_or(NaiveTime::MIN)
            && bar_time <= NaiveTime::from_hms_opt(14, 4, 59).unwrap_or(NaiveTime::MIN))
            || (bar_time >= NaiveTime::from_hms_opt(18, 50, 0).unwrap_or(NaiveTime::MIN)
                && bar_time <= NaiveTime::from_hms_opt(19, 4, 59).unwrap_or(NaiveTime::MIN))
    }

    fn is_pre_transition_service_bar(&self, dt_local: NaiveDateTime) -> bool {
        let Some(transition) = self.anchor_transition_rule else {
            return false;
        };
        matches!(
            self.config.mode,
            RiAuthor4142RuntimeMode::Shadow | RiAuthor4142RuntimeMode::ProspectiveShadow
        ) && dt_local.date() < transition.starts_on
            && dt_local.time() < NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN)
    }

    fn collect_new_decisions(
        &mut self,
        current_dt_local: NaiveDateTime,
    ) -> Vec<RiAuthor4142ModelDecision> {
        let eligible_anchor_sessions = self.daily_stats_before(current_dt_local.date());
        self.log_anchor_context(current_dt_local.date(), &eligible_anchor_sessions);
        let eligible_dates = eligible_anchor_sessions
            .into_iter()
            .map(|(date, _)| date)
            .collect::<HashSet<_>>();
        let eligible_model_bars = self
            .model_bars
            .iter()
            .copied()
            .filter(|bar| {
                bar.ts_local.date() == current_dt_local.date()
                    || eligible_dates.contains(&bar.ts_local.date())
            })
            .collect::<Vec<_>>();
        let records = build_ri_author41_42_combo_shadow_journal_with_configs(
            &eligible_model_bars,
            self.session_policy,
            self.author41_short_config(),
            self.author41_long_config(),
            self.author42_config(),
        );
        let finalized = finalized_decisions_from_records(records, current_dt_local);
        if !self.can_emit_orders() {
            return self.collect_shadow_decisions(finalized, current_dt_local);
        }
        self.collect_incremental_decisions(finalized)
    }

    fn collect_incremental_decisions(
        &mut self,
        finalized: Vec<RiAuthor4142ModelDecision>,
    ) -> Vec<RiAuthor4142ModelDecision> {
        let mut decisions = Vec::new();
        for decision in finalized {
            if !self
                .emitted_decision_keys
                .insert(decision.decision_key.clone())
            {
                continue;
            }
            self.record_decision_state(&decision);
            self.record_shadow_journal(&decision);
            self.log_decision(&decision);
            decisions.push(decision);
        }
        decisions
    }

    fn collect_shadow_decisions(
        &mut self,
        finalized: Vec<RiAuthor4142ModelDecision>,
        current_dt_local: NaiveDateTime,
    ) -> Vec<RiAuthor4142ModelDecision> {
        let new_decisions = self.collect_incremental_decisions(finalized.clone());
        if !self.config.runs_prospective_adapter() {
            for decision in &new_decisions {
                self.apply_dry_run_decision(decision);
            }
        }
        self.update_shadow_portfolio_path(&finalized, current_dt_local);
        new_decisions
    }

    fn update_shadow_portfolio_path(
        &mut self,
        finalized: &[RiAuthor4142ModelDecision],
        current_dt_local: NaiveDateTime,
    ) {
        let mut keys_by_date = BTreeMap::<NaiveDate, BTreeSet<String>>::new();
        keys_by_date.entry(current_dt_local.date()).or_default();
        for decision in finalized {
            self.shadow_decisions_by_key
                .insert(decision.decision_key.clone(), decision.clone());
            if !is_shadow_portfolio_path_member(decision) {
                continue;
            }
            keys_by_date
                .entry(decision_entry_date(decision))
                .or_default()
                .insert(decision.decision_key.clone());
        }

        for (date, current_keys) in keys_by_date {
            let previous_keys = self
                .shadow_path_keys_by_date
                .get(&date)
                .cloned()
                .unwrap_or_default();
            if previous_keys == current_keys {
                continue;
            }

            for removed_key in previous_keys.difference(&current_keys) {
                if let Some(decision) = self.shadow_decisions_by_key.get(removed_key).cloned() {
                    self.record_shadow_path_superseded_journal(&decision);
                    self.log_shadow_path_change(
                        "ri_shadow_path_superseded",
                        &decision,
                        date,
                        current_dt_local,
                    );
                }
            }
            for added_key in current_keys.difference(&previous_keys) {
                if let Some(decision) = self.shadow_decisions_by_key.get(added_key).cloned() {
                    self.record_shadow_path_active_journal(&decision);
                    self.log_shadow_path_change(
                        "ri_shadow_path_active",
                        &decision,
                        date,
                        current_dt_local,
                    );
                }
            }

            self.shadow_path_keys_by_date.insert(date, current_keys);
        }
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

    fn log_shadow_path_change(
        &self,
        action: &'static str,
        decision: &RiAuthor4142ModelDecision,
        path_date: NaiveDate,
        current_dt_local: NaiveDateTime,
    ) {
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action,
            path_date = %path_date,
            recomputed_at_local = %current_dt_local,
            component = decision.component.as_str(),
            side = decision.side.map(RiAuthor4142Side::as_str).unwrap_or("none"),
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

    fn record_shadow_path_active_journal(&mut self, decision: &RiAuthor4142ModelDecision) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::ShadowPathActive,
            None,
            None,
        ));
    }

    fn record_shadow_path_superseded_journal(&mut self, decision: &RiAuthor4142ModelDecision) {
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            RiAuthor4142JournalDecision::ShadowPathSuperseded,
            None,
            Some("shadow_path_recomputed"),
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

    fn runtime_adapter_model_bar(&self, ctx: &StrategyCtx, bar: &BarEvent) -> Option<ModelBar> {
        let live_emission_allowed = self.can_emit_orders()
            && ctx.trade_mode == crate::TradeMode::Live
            && ctx.allow_live_orders;
        if !live_emission_allowed && !self.config.runs_prospective_adapter() {
            return None;
        }
        if bar.origin != DataOrigin::Live {
            return None;
        }
        let dt_local = self.local_dt(bar.close_time_utc)?;
        if !self.session_policy.is_model_bar(dt_local)
            || self.is_operational_shadow_break_bar(dt_local)
            || self.excluded_model_dates.contains(&dt_local.date())
        {
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

    fn finalize_prospective_adapter_transition(&mut self) {
        match self.phase_for_test().as_deref() {
            Some("live_pending_entry") => {
                if let StrategyState::RiAuthor4142Live {
                    phase,
                    last_transition_reason,
                    pending_entry_request_id,
                    pending_exit_request_id,
                    ..
                } = &mut self.state
                {
                    *phase = "live_in_position".to_string();
                    *last_transition_reason =
                        Some("prospective_shadow_entry_assumed_filled".to_string());
                    *pending_entry_request_id = None;
                    *pending_exit_request_id = None;
                }
            }
            Some("live_pending_exit") | Some("live_deferred_exit") => {
                self.transition_live_flat_with_reason(
                    "prospective_shadow_exit_assumed_filled".to_string(),
                );
            }
            _ => {}
        }
    }

    fn live_intents_for_bar(&mut self, bar: ModelBar) -> Vec<Intent> {
        let mut intents = Vec::new();
        intents.extend(self.live_mr_intents_for_bar(bar));
        intents.extend(self.live_bo_intents_for_bar(bar));
        intents
    }

    fn live_mr_intents_for_bar(&mut self, bar: ModelBar) -> Vec<Intent> {
        let mut intents = Vec::new();
        if self.phase_is("live_pending_entry") {
            return intents;
        }
        if self.phase_is("live_pending_exit") {
            return intents;
        }
        if self.phase_is("live_deferred_exit") {
            if let Some(position) = self.live_mr.position.clone() {
                intents.push(self.emit_mr_exit_intent(
                    &position,
                    bar,
                    bar.close,
                    "deferred_exit_reissue",
                    "live_deferred_exit_reissued",
                ));
            }
            return intents;
        }
        if self.phase_blocks_fresh_entries() {
            return intents;
        }
        if self.live_mr.current_date != Some(bar.ts_local.date()) {
            self.live_mr.current_date = Some(bar.ts_local.date());
            self.live_mr.entries_today = 0;
            if self.phase_is(RiAuthor4142Phase::Flat.as_str()) {
                self.live_mr.position = None;
            }
        }

        if let Some(mut position) = self.live_mr.position.take() {
            position.bars_held = position.bars_held.saturating_add(1);
            if let Some((exit_price, reason)) = Self::mr_exit_signal(&position, bar) {
                intents.push(self.emit_mr_exit_intent(
                    &position,
                    bar,
                    exit_price,
                    reason,
                    "live_exit_emitted",
                ));
                self.live_mr.position = Some(position);
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
        self.transition_live_pending_entry(&decision, "live_entry_emitted");
        intents
    }

    fn live_bo_intents_for_bar(&mut self, bar: ModelBar) -> Vec<Intent> {
        let mut intents = Vec::new();
        if self.phase_is("live_pending_entry") {
            return intents;
        }
        if self.phase_is("live_pending_exit") {
            return intents;
        }
        if self.phase_is("live_deferred_exit") {
            if let Some(position) = self.live_bo.position.clone() {
                let config = self.author42_config();
                intents.push(self.emit_bo_exit_intent(
                    &position,
                    bar,
                    non_zero_or_close(bar.open, bar.close),
                    "deferred_exit_reissue",
                    "live_bo_deferred_exit_reissued",
                    config.roundtrip_cost_points,
                ));
            }
            return intents;
        }
        if self.phase_blocks_fresh_entries() {
            return intents;
        }
        self.ensure_bo_session(bar);
        let config = self.author42_config();

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
                        self.transition_live_pending_entry(&decision, "live_bo_entry_emitted");
                    }
                }
                RiAuthor4142LiveBoPending::Exit(reason) => {
                    if let Some(position) = self.live_bo.position.clone() {
                        let exit_price = non_zero_or_close(bar.open, bar.close);
                        intents.push(self.emit_bo_exit_intent(
                            &position,
                            bar,
                            exit_price,
                            reason,
                            "live_bo_exit_emitted",
                            config.roundtrip_cost_points,
                        ));
                        return intents;
                    }
                }
            }
        }

        self.live_bo.day_hh = self.live_bo.day_hh.max(bar.high);
        self.live_bo.day_ll = self.live_bo.day_ll.min(bar.low);

        if let Some(mut position) = self.live_bo.position.take() {
            position.bars_held = position.bars_held.saturating_add(1);
            if bar.ts_local.time() >= config.exit_time {
                intents.push(self.emit_bo_exit_intent(
                    &position,
                    bar,
                    bar.close,
                    "time_exit_same_bar_close",
                    "live_bo_exit_emitted",
                    config.roundtrip_cost_points,
                ));
                self.live_bo.position = Some(position);
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
        let config = self.author42_config();
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
        let short = self.author41_short_config();
        let long = self.author41_long_config();
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

    fn emit_mr_exit_intent(
        &mut self,
        position: &RiAuthor4142LiveMrPosition,
        bar: ModelBar,
        exit_price: f64,
        reason: &'static str,
        transition_reason: &'static str,
    ) -> Intent {
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
        let intent = self.emit_live_candidate(candidate, &decision);
        self.transition_live_pending_exit(&decision, transition_reason);
        intent
    }

    fn emit_bo_exit_intent(
        &mut self,
        position: &RiAuthor4142LiveBoPosition,
        bar: ModelBar,
        exit_price: f64,
        reason: &'static str,
        transition_reason: &'static str,
        roundtrip_cost_points: f64,
    ) -> Intent {
        let decision = self.live_decision(RiAuthor4142LiveDecisionInput {
            component: RiAuthor4142Component::Author42Bo,
            side: position.side,
            model_signal_ts_local: position.entry_ts_local,
            scheduled_entry_ts_local: Some(position.entry_ts_local),
            scheduled_exit_ts_local: Some(bar.ts_local),
            reason,
            decision_key: position.decision_key.clone(),
            shadow_pnl_points: Some(
                Self::points_for_side(position.side, position.entry_price, exit_price)
                    - roundtrip_cost_points,
            ),
        });
        let candidate = self.live_candidate_for_decision(
            &decision,
            RiAuthor4142CandidateRole::Exit,
            position.side.exit_order_side(),
            bar.ts_local,
        );
        let intent = self.emit_live_candidate(candidate, &decision);
        self.transition_live_pending_exit(&decision, transition_reason);
        intent
    }

    fn record_candidate_emitted_journal(
        &mut self,
        candidate: &RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) {
        let adapter_decision = if self.config.runs_prospective_adapter() {
            RiAuthor4142JournalDecision::ProspectiveIntentSuppressed
        } else {
            RiAuthor4142JournalDecision::IntentEmitted
        };
        self.push_journal_record(RiAuthor4142JournalRecord::from_decision(
            decision,
            adapter_decision,
            Some(candidate),
            None,
        ));
    }

    fn log_candidate_intent_emitted(
        &self,
        candidate: &RiAuthor4142CandidateIntent,
        decision: &RiAuthor4142ModelDecision,
    ) {
        if self.config.runs_prospective_adapter() {
            info!(
                target: "strategy_runtime::ri_author41_42_live",
                action = "ri_prospective_intent_suppressed",
                suppression_reason = "prospective_shadow_no_broker_emission",
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
                shadow_pnl_points = ?decision.shadow_pnl_points,
                mode = self.config.mode.as_str(),
                allow_order_emission = self.config.allow_order_emission,
                live_adapter_enabled = self.can_emit_orders(),
            );
            return;
        }
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

    fn transition_live_pending_entry(
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
            pending_entry_request_id,
            pending_exit_request_id,
            ..
        } = &mut self.state
        {
            *phase = "live_pending_entry".to_string();
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
            *pending_entry_request_id = None;
            *pending_exit_request_id = None;
        }
    }

    fn transition_live_pending_exit(
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
            pending_exit_request_id,
            ..
        } = &mut self.state
        {
            *phase = "live_pending_exit".to_string();
            *current_component = Some(decision.component.as_str().to_string());
            *current_side = decision.side.map(|side| side.as_str().to_string());
            *current_cycle_id = Some(format!(
                "{}:{}",
                decision.component.as_str(),
                decision.model_signal_ts_local.format("%Y%m%d%H%M%S")
            ));
            *current_entry_ts_local = decision.scheduled_entry_ts_local.map(|ts| ts.to_string());
            *current_exit_ts_local = decision.scheduled_exit_ts_local.map(|ts| ts.to_string());
            *last_transition_reason = Some(format!("{}:{}", reason, decision.reason));
            *pending_exit_request_id = None;
        }
    }

    fn transition_live_deferred_exit(&mut self, reason: String) {
        if let StrategyState::RiAuthor4142Live {
            phase,
            last_transition_reason,
            pending_exit_request_id,
            ..
        } = &mut self.state
        {
            *phase = "live_deferred_exit".to_string();
            *last_transition_reason = Some(reason);
            *pending_exit_request_id = None;
        }
    }

    fn transition_live_flat_with_reason(&mut self, reason: String) {
        self.clear_live_positions();
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            pending_entry_request_id,
            pending_exit_request_id,
            ..
        } = &mut self.state
        {
            *phase = RiAuthor4142Phase::Flat.as_str().to_string();
            *current_component = None;
            *current_side = None;
            *current_cycle_id = None;
            *current_entry_ts_local = None;
            *current_exit_ts_local = None;
            *last_transition_reason = Some(reason);
            *pending_entry_request_id = None;
            *pending_exit_request_id = None;
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
            if bar.ts_local.date() >= date
                || self.excluded_model_dates.contains(&bar.ts_local.date())
            {
                continue;
            }
            by_day
                .entry(bar.ts_local.date())
                .and_modify(|stats| {
                    stats.high = stats.high.max(bar.high);
                    stats.low = stats.low.min(bar.low);
                    stats.close = bar.close;
                    stats.bars += 1;
                    stats.first_bar = stats.first_bar.min(bar.ts_local.time());
                    stats.last_bar = stats.last_bar.max(bar.ts_local.time());
                })
                .or_insert(RiDailyStats {
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    bars: 1,
                    first_bar: bar.ts_local.time(),
                    last_bar: bar.ts_local.time(),
                });
        }
        by_day
            .into_iter()
            .filter(|(session_date, stats)| self.is_eligible_anchor_session(*session_date, *stats))
            .collect()
    }

    fn anchor_rule_for_date(&self, date: NaiveDate) -> RiAnchorCompletenessRule {
        match self.anchor_transition_rule {
            Some(transition) if date < transition.starts_on => transition.pre_transition,
            _ => self.canonical_anchor_rule,
        }
    }

    fn is_eligible_anchor_session(&self, date: NaiveDate, stats: RiDailyStats) -> bool {
        let rule = self.anchor_rule_for_date(date);
        stats.bars >= rule.min_bars
            && stats.first_bar <= rule.first_bar_at_or_before
            && stats.last_bar >= rule.last_bar_at_or_after
    }

    fn log_anchor_context(
        &mut self,
        current_date: NaiveDate,
        eligible_sessions: &[(NaiveDate, RiDailyStats)],
    ) {
        if !self.logged_anchor_context_dates.insert(current_date) {
            return;
        }
        let summarize = |offset: usize| {
            eligible_sessions
                .iter()
                .rev()
                .nth(offset)
                .map(|(date, stats)| {
                    let rule = self.anchor_rule_for_date(*date);
                    (
                        date.to_string(),
                        stats.bars,
                        stats.first_bar.to_string(),
                        stats.last_bar.to_string(),
                        rule.min_bars,
                        rule.first_bar_at_or_before.to_string(),
                        rule.last_bar_at_or_after.to_string(),
                    )
                })
        };
        let prev = summarize(0);
        let prev2 = summarize(1);
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_anchor_context_resolved",
            current_date = %current_date,
            eligible_anchor_sessions = eligible_sessions.len(),
            prev = ?prev,
            prev2 = ?prev2,
        );
    }

    fn author41_short_config(&self) -> Author41Config {
        let mut config = Author41Config::ri_plateau_short_source();
        config.entry_end = self.author41_entry_end;
        config.time_exit = self.author41_time_exit;
        config
    }

    fn author41_long_config(&self) -> Author41Config {
        let mut config = Author41Config::ri_plateau_long_source();
        config.entry_end = self.author41_entry_end;
        config.time_exit = self.author41_time_exit;
        config
    }

    fn author42_config(&self) -> Author42Config {
        let mut config = Author42Config::ri_grid_k042_both();
        config.exit_time = self.author42_exit_time;
        config
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

    fn phase_is(&self, expected: &str) -> bool {
        self.phase_for_test().as_deref() == Some(expected)
    }

    fn phase_blocks_fresh_entries(&self) -> bool {
        matches!(
            self.phase_for_test().as_deref(),
            Some("dry_run_in_position")
                | Some("manual_intervention_required")
                | Some("live_entry_missed_runtime_not_ready")
        )
    }

    fn guard_reasons_are_runtime_readiness_only(reasons: &[String]) -> bool {
        !reasons.is_empty()
            && reasons.iter().all(|reason| {
                reason == "gateway_ready=false"
                    || reason == "gateway_health_stale"
                    || reason == "ws_connected=false"
                    || reason == "cws_authorized=false"
                    || reason == "bootstrap:not_ready"
                    || reason.starts_with("phase=")
            })
    }

    fn transition_live_entry_missed_runtime_not_ready(&mut self, event: &IntentBlocked) {
        self.clear_live_positions();
        let mut component = None;
        let mut side = None;
        let mut cycle_id = None;
        let mut entry_ts_local = None;
        let mut exit_ts_local = None;
        if let StrategyState::RiAuthor4142Live {
            phase,
            current_component,
            current_side,
            current_cycle_id,
            current_entry_ts_local,
            current_exit_ts_local,
            last_transition_reason,
            pending_entry_request_id,
            pending_exit_request_id,
            ..
        } = &mut self.state
        {
            *phase = RiAuthor4142Phase::LiveEntryMissedRuntimeNotReady
                .as_str()
                .to_string();
            *last_transition_reason = Some(format!(
                "missed_due_runtime_not_ready:{}:{}",
                event.created_ts_utc,
                event.guard_reasons.join(",")
            ));
            *pending_entry_request_id = None;
            *pending_exit_request_id = None;
            component = current_component.clone();
            side = current_side.clone();
            cycle_id = current_cycle_id.clone();
            entry_ts_local = current_entry_ts_local.clone();
            exit_ts_local = current_exit_ts_local.clone();
        }
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_live_entry_missed_runtime_not_ready",
            reason = event.reason,
            blocked_action = event.action,
            intent_class = ?event.intent_class,
            created_ts_utc = event.created_ts_utc,
            guard_reasons = ?event.guard_reasons,
            component = ?component,
            side = ?side,
            cycle_id = ?cycle_id,
            entry_ts_local = ?entry_ts_local,
            exit_ts_local = ?exit_ts_local,
            mode = self.config.mode.as_str(),
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn pending_entry_request_id(&self) -> Option<Uuid> {
        match &self.state {
            StrategyState::RiAuthor4142Live {
                pending_entry_request_id,
                ..
            } => *pending_entry_request_id,
            _ => None,
        }
    }

    fn pending_exit_request_id(&self) -> Option<Uuid> {
        match &self.state {
            StrategyState::RiAuthor4142Live {
                pending_exit_request_id,
                ..
            } => *pending_exit_request_id,
            _ => None,
        }
    }

    fn clear_pending_entry_request_id(&mut self) {
        if let StrategyState::RiAuthor4142Live {
            pending_entry_request_id,
            ..
        } = &mut self.state
        {
            *pending_entry_request_id = None;
        }
    }

    fn set_pending_request_id(&mut self, intent_class: IntentClass, request_id: Uuid) {
        if let StrategyState::RiAuthor4142Live {
            phase,
            pending_entry_request_id,
            pending_exit_request_id,
            ..
        } = &mut self.state
        {
            match (phase.as_str(), intent_class) {
                ("live_pending_entry", IntentClass::Entry) => {
                    *pending_entry_request_id = Some(request_id);
                }
                ("live_pending_exit" | "live_deferred_exit", IntentClass::Exit) => {
                    *pending_exit_request_id = Some(request_id);
                }
                _ => {}
            }
        }
    }

    fn log_request_id_skew(&self, lifecycle: &'static str, expected: Option<Uuid>, actual: Uuid) {
        if let Some(expected) = expected {
            if expected != actual {
                info!(
                    target: "strategy_runtime::ri_author41_42_live",
                    action = "ri_pending_request_id_skew_detected",
                    lifecycle,
                    expected_request_id = %expected,
                    actual_request_id = %actual,
                    phase = ?self.phase_for_test(),
                    mode = self.config.mode.as_str(),
                    live_adapter_enabled = self.can_emit_orders(),
                );
            }
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
        let Some(model_bar) = self.runtime_adapter_model_bar(ctx, bar) else {
            return Vec::new();
        };
        let intents = self.live_intents_for_bar(model_bar);
        if self.config.runs_prospective_adapter() {
            self.finalize_prospective_adapter_transition();
            return Vec::new();
        }
        intents
    }

    fn on_ack(&mut self, ctx: &StrategyCtx, ack: &alor_protocol::CommandAck) -> Vec<Intent> {
        if !self.can_emit_orders() {
            return Vec::new();
        }
        if !matches!(
            ack.status,
            AckStatus::Rejected | AckStatus::Expired | AckStatus::Error
        ) {
            return Vec::new();
        }
        let phase = self.phase_for_test();
        if phase.as_deref() == Some("live_pending_exit")
            || phase.as_deref() == Some("live_deferred_exit")
        {
            let expected_request_id = self.pending_exit_request_id();
            if expected_request_id.is_some() && expected_request_id != Some(ack.request_id) {
                self.log_request_id_skew("exit", expected_request_id, ack.request_id);
                return Vec::new();
            }
            let broker_qty = ctx.position_qty.unwrap_or(f64::NAN);
            if ctx
                .position_qty
                .map(|qty| qty.abs() <= f64::EPSILON)
                .unwrap_or(false)
            {
                self.transition_live_flat_with_reason(format!(
                    "live_exit_ack_rejected_but_broker_flat:{}:{}",
                    ack.error_code.as_deref().unwrap_or("unknown"),
                    ack.error_msg.as_deref().unwrap_or("unknown")
                ));
                info!(
                    target: "strategy_runtime::ri_author41_42_live",
                    action = "ri_live_exit_rejected_broker_flat",
                    request_id = %ack.request_id,
                    status = ?ack.status,
                    error_code = ?ack.error_code,
                    error_msg = ?ack.error_msg,
                    broker_qty,
                    mode = self.config.mode.as_str(),
                    live_adapter_enabled = self.can_emit_orders(),
                );
            } else if ack.error_code.as_deref() == Some("trading_window_closed") {
                self.transition_live_deferred_exit(format!(
                    "live_exit_deferred:{}:{}",
                    ack.error_code.as_deref().unwrap_or("unknown"),
                    ack.error_msg.as_deref().unwrap_or("unknown")
                ));
                info!(
                    target: "strategy_runtime::ri_author41_42_live",
                    action = "ri_live_exit_deferred",
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
                    "live_exit_ack_rejected_with_broker_position:{}:{}",
                    ack.error_code.as_deref().unwrap_or("unknown"),
                    ack.error_msg.as_deref().unwrap_or("unknown")
                ));
            }
        } else if phase.as_deref() == Some("live_pending_entry")
            || phase.as_deref() == Some("live_in_position")
        {
            let expected_request_id = self.pending_entry_request_id();
            if expected_request_id.is_some() && expected_request_id != Some(ack.request_id) {
                self.log_request_id_skew("entry", expected_request_id, ack.request_id);
                return Vec::new();
            }
            let broker_qty = ctx.position_qty.unwrap_or(0.0);
            if broker_qty.abs() <= f64::EPSILON {
                self.transition_live_flat_with_reason(format!(
                    "live_entry_ack_rejected:{}:{}",
                    ack.error_code.as_deref().unwrap_or("unknown"),
                    ack.error_msg.as_deref().unwrap_or("unknown")
                ));
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

    fn on_position(&mut self, _ctx: &StrategyCtx, pos: &PositionEvent) -> Vec<Intent> {
        if !self.can_emit_orders() {
            return Vec::new();
        }
        let mut mark_flat = false;
        let mut clear_pending_entry = false;
        if let StrategyState::RiAuthor4142Live {
            phase,
            last_transition_reason,
            ..
        } = &mut self.state
        {
            if phase == "live_pending_entry" && pos.qty.abs() > f64::EPSILON {
                *phase = "live_in_position".to_string();
                *last_transition_reason = Some("live_position_confirmed".to_string());
                clear_pending_entry = true;
            }
            if matches!(
                phase.as_str(),
                "live_pending_exit" | "live_deferred_exit" | "live_in_position"
            ) && pos.qty.abs() <= f64::EPSILON
            {
                mark_flat = true;
            }
        }
        if mark_flat {
            self.transition_live_flat_with_reason("live_position_flat_confirmed".to_string());
        } else if clear_pending_entry {
            self.clear_pending_entry_request_id();
        }
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        _ctx: &StrategyCtx,
        snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        if self.config.runs_prospective_adapter() {
            return Vec::new();
        }
        self.handle_bootstrap_snapshot(snapshot);
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        if self.config.runs_prospective_adapter() {
            return Vec::new();
        }
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

    fn set_state(&mut self, mut state: StrategyState) {
        if let StrategyState::RiAuthor4142Live { phase, .. } = &state {
            if phase != "live_in_position"
                && phase != "live_pending_exit"
                && phase != "live_deferred_exit"
            {
                self.clear_live_positions();
            }
        }
        if let StrategyState::RiAuthor4142Live {
            phase,
            pending_entry_request_id,
            pending_exit_request_id,
            ..
        } = &mut state
        {
            if phase != "live_pending_entry" {
                *pending_entry_request_id = None;
            }
            if phase != "live_pending_exit" && phase != "live_deferred_exit" {
                *pending_exit_request_id = None;
            }
        }
        self.state = state;
    }

    fn on_command_prepared(&mut self, _ctx: &StrategyCtx, command: &CommandPrepared) {
        if !self.can_emit_orders() {
            return;
        }
        self.set_pending_request_id(command.intent_class, command.request_id);
        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_command_prepared",
            request_id = %command.request_id,
            intent_class = ?command.intent_class,
            created_ts_utc = command.created_ts_utc,
            symbol = %command.symbol,
            phase = ?self.phase_for_test(),
            mode = self.config.mode.as_str(),
            live_adapter_enabled = self.can_emit_orders(),
        );
    }

    fn on_intent_blocked(
        &mut self,
        _ctx: &StrategyCtx,
        event: &IntentBlocked,
    ) -> IntentBlockDisposition {
        if !self.can_emit_orders() {
            return IntentBlockDisposition::Rollback;
        }
        if event.reason == "live_guard"
            && event.intent_class == IntentClass::Entry
            && self.phase_is("live_pending_entry")
            && Self::guard_reasons_are_runtime_readiness_only(&event.guard_reasons)
        {
            self.transition_live_entry_missed_runtime_not_ready(event);
            return IntentBlockDisposition::KeepStrategyState;
        }
        IntentBlockDisposition::Rollback
    }

    fn pending_request_ids(&self) -> Vec<Uuid> {
        [
            self.pending_entry_request_id(),
            self.pending_exit_request_id(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn exit_risk_status(
        &self,
        has_open_position: bool,
    ) -> crate::strategy_host::StrategyExitRiskStatus {
        let phase = self.phase_for_test();
        if matches!(
            phase.as_deref(),
            Some("manual_intervention_required") | Some("live_entry_missed_runtime_not_ready")
        ) {
            return crate::strategy_host::StrategyExitRiskStatus {
                phase_override: phase,
                exit_recovery_active: false,
                operator_intervention_required: true,
                open_risk_position_unflattened: has_open_position,
            };
        }
        if matches!(
            phase.as_deref(),
            Some("live_pending_exit") | Some("live_deferred_exit")
        ) && has_open_position
        {
            return crate::strategy_host::StrategyExitRiskStatus {
                phase_override: Some("CloseOnlyDegraded".to_string()),
                exit_recovery_active: phase.as_deref() == Some("live_deferred_exit"),
                operator_intervention_required: false,
                open_risk_position_unflattened: true,
            };
        }
        crate::strategy_host::StrategyExitRiskStatus::default()
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
            candidate_scheduled_ts_local: candidate
                .map(|candidate| candidate.scheduled_ts_local.to_string()),
            execution_path: candidate
                .map(|candidate| candidate.execution_path.as_str().to_string())
                .unwrap_or_else(|| "not_applicable_pre_go".to_string()),
            decision_key: decision.decision_key.clone(),
            shadow_pnl_points: decision.shadow_pnl_points,
        }
    }
}

fn finalized_decisions_from_records(
    records: Vec<ShadowJournalRecord>,
    current_dt_local: NaiveDateTime,
) -> Vec<RiAuthor4142ModelDecision> {
    let mut decisions = records
        .into_iter()
        .filter(|record| is_finalized_record(record, current_dt_local))
        .filter_map(|record| {
            let key = decision_key(&record);
            RiAuthor4142ModelDecision::from_shadow_record(record, key)
        })
        .collect::<Vec<_>>();
    decisions.sort_by(|left, right| {
        (
            decision_entry_ts(left),
            decision_component_sort_key(left.component),
            left.scheduled_exit_ts_local,
            left.decision_key.as_str(),
        )
            .cmp(&(
                decision_entry_ts(right),
                decision_component_sort_key(right.component),
                right.scheduled_exit_ts_local,
                right.decision_key.as_str(),
            ))
    });
    decisions
}

fn decision_entry_ts(decision: &RiAuthor4142ModelDecision) -> NaiveDateTime {
    decision
        .scheduled_entry_ts_local
        .unwrap_or(decision.model_signal_ts_local)
}

fn decision_entry_date(decision: &RiAuthor4142ModelDecision) -> NaiveDate {
    decision_entry_ts(decision).date()
}

fn decision_component_sort_key(component: RiAuthor4142Component) -> u8 {
    match component {
        RiAuthor4142Component::Author41Mr => 0,
        RiAuthor4142Component::Author42Bo => 1,
    }
}

fn is_shadow_portfolio_path_member(decision: &RiAuthor4142ModelDecision) -> bool {
    decision.action == RiAuthor4142DecisionAction::Enter
        && decision.overlap_decision == "Accepted"
        && decision.scheduled_entry_ts_local.is_some()
        && decision.scheduled_exit_ts_local.is_some()
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
        Component, Instrument, ModelBar, OverlapDecision, ProfileId, ShadowJournalRecord,
        ShadowSide,
    };
    use crate::strategy_host::{
        BarEvent, BootstrapSnapshot, CommandPrepared, DataOrigin, IntentBlockDisposition,
        IntentBlocked, OrderEvent, PositionEvent, RuntimeStateRestored, StopOrderEvent, Strategy,
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
            session_start_time: "09:00:00".to_string(),
            session_end_time: "23:49:59".to_string(),
            author41_entry_end_time: "12:00:00".to_string(),
            author41_time_exit: "20:00:00".to_string(),
            author42_exit_time: "23:00:00".to_string(),
            excluded_model_dates: Vec::new(),
            min_anchor_bars: 0,
            anchor_first_bar_at_or_before: "23:59:59".to_string(),
            anchor_last_bar_at_or_after: "00:00:00".to_string(),
            anchor_transition_date: None,
            pre_transition_min_anchor_bars: None,
            pre_transition_anchor_first_bar_at_or_before: None,
            pre_transition_anchor_last_bar_at_or_after: None,
            actual_expiry_date: None,
            roll_target_sessions_before: 1,
            roll_fallback_sessions_before: 2,
            qty: 1.0,
            timezone_offset_hours: 3,
        }
    }

    fn canonical07_transition_config() -> RiAuthor4142LiveConfig {
        let mut config = default_config();
        config.session_start_time = "07:00:00".to_string();
        config.author41_entry_end_time = "10:00:00".to_string();
        config.min_anchor_bars = 92;
        config.anchor_first_bar_at_or_before = "07:10:00".to_string();
        config.anchor_last_bar_at_or_after = "23:30:00".to_string();
        config.anchor_transition_date = Some("2026-07-14".to_string());
        config.pre_transition_min_anchor_bars = Some(80);
        config.pre_transition_anchor_first_bar_at_or_before = Some("09:10:00".to_string());
        config.pre_transition_anchor_last_bar_at_or_after = Some("23:30:00".to_string());
        config
    }

    fn canonical07_transition_fixture_bars() -> Vec<ModelBar> {
        include_str!("../../tests/fixtures/ri_canonical07_transition_2026_07_10_15.csv")
            .lines()
            .skip(1)
            .map(|line| {
                let fields = line.split(',').collect::<Vec<_>>();
                assert_eq!(fields.len(), 6, "fixture row must have six columns");
                ModelBar {
                    ts_local: NaiveDateTime::parse_from_str(fields[0], "%Y-%m-%d %H:%M:%S")
                        .expect("fixture timestamp"),
                    open: fields[1].parse().expect("fixture open"),
                    high: fields[2].parse().expect("fixture high"),
                    low: fields[3].parse().expect("fixture low"),
                    close: fields[4].parse().expect("fixture close"),
                    volume: fields[5].parse().expect("fixture volume"),
                }
            })
            .collect()
    }

    #[test]
    fn shadow_mode_scaffold_cannot_emit_orders() {
        let strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        assert!(!strategy.can_emit_orders());
    }

    #[test]
    fn prospective_shadow_matches_live_adapter_without_emitting_orders() {
        let mut prospective_config = default_config();
        prospective_config.mode = RiAuthor4142RuntimeMode::ProspectiveShadow;
        let mut prospective =
            RiAuthor4142LiveStrategy::new(prospective_config).expect("prospective strategy");

        let mut live_config = default_config();
        live_config.mode = RiAuthor4142RuntimeMode::MicroLive;
        live_config.allow_order_emission = true;
        let mut live = RiAuthor4142LiveStrategy::new(live_config).expect("live strategy");

        let prev_day = bar_with_ohlc(
            dt(2026, 5, 1, 23, 40, 0),
            DataOrigin::History,
            100_000.0,
            101_000.0,
            99_000.0,
            100_000.0,
        );
        prospective.warmup_from_history(&test_ctx(), std::slice::from_ref(&prev_day));
        live.warmup_from_history(&live_ctx(), &[prev_day]);

        let entry_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 0, 0),
            DataOrigin::Live,
            100_050.0,
            100_120.0,
            100_030.0,
            100_100.0,
        );
        assert!(prospective.on_bar(&test_ctx(), &entry_bar).is_empty());
        assert_eq!(
            prospective.phase_for_test().as_deref(),
            Some("live_in_position")
        );

        assert_eq!(live.on_bar(&live_ctx(), &entry_bar).len(), 1);
        live.on_position(
            &live_ctx_with_position(-1.0),
            &PositionEvent {
                symbol: "RIM6".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100_100.0,
                ts_utc: entry_bar.close_time_utc,
            },
        );

        let exit_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 10, 0),
            DataOrigin::Live,
            100_090.0,
            100_100.0,
            99_890.0,
            99_900.0,
        );
        assert!(prospective.on_bar(&test_ctx(), &exit_bar).is_empty());
        assert_eq!(
            prospective.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
        assert_eq!(live.on_bar(&live_ctx(), &exit_bar).len(), 1);

        let prospective_rows = prospective
            .journal_records_for_test()
            .iter()
            .filter(|row| {
                row.adapter_decision == RiAuthor4142JournalDecision::ProspectiveIntentSuppressed
            })
            .collect::<Vec<_>>();
        let live_rows = live
            .journal_records_for_test()
            .iter()
            .filter(|row| row.adapter_decision == RiAuthor4142JournalDecision::IntentEmitted)
            .collect::<Vec<_>>();

        assert_eq!(prospective_rows.len(), 2);
        assert_eq!(live_rows.len(), 2);
        for (prospective_row, live_row) in prospective_rows.iter().zip(live_rows.iter()) {
            assert_eq!(prospective_row.component, live_row.component);
            assert_eq!(prospective_row.side, live_row.side);
            assert_eq!(prospective_row.role, live_row.role);
            assert_eq!(
                prospective_row.candidate_scheduled_ts_local,
                live_row.candidate_scheduled_ts_local
            );
            assert_eq!(
                prospective_row.entry_exit_reason,
                live_row.entry_exit_reason
            );
            assert_eq!(prospective_row.decision_key, live_row.decision_key);
            assert_eq!(
                prospective_row.shadow_pnl_points,
                live_row.shadow_pnl_points
            );
        }
        assert!(prospective
            .journal_records_for_test()
            .iter()
            .all(|row| { row.adapter_decision != RiAuthor4142JournalDecision::IntentEmitted }));
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
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_entry")
        );

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
    fn micro_live_readiness_blocked_entry_marks_missed_and_blocks_opposite_replacement() {
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

        let first_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 0, 0),
            DataOrigin::Live,
            100_000.0,
            100_050.0,
            99_850.0,
            99_900.0,
        );
        let first_intents = strategy.on_bar(&live_ctx(), &first_bar);
        assert_eq!(first_intents.len(), 1);
        match first_intents[0].base_intent() {
            crate::strategy_host::Intent::Market { side, .. } => {
                assert_eq!(*side, OrderSide::Buy);
            }
            other => panic!("unexpected first intent: {other:?}"),
        }
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_entry")
        );
        assert!(strategy.live_mr.position.is_some());

        let disposition = strategy.on_intent_blocked(
            &live_ctx(),
            &IntentBlocked {
                reason: "live_guard",
                action: "market",
                intent_class: IntentClass::Entry,
                created_ts_utc: first_bar.close_time_utc,
                guard_reasons: vec![
                    "phase=SyncingHistory".to_string(),
                    "gateway_ready=false".to_string(),
                ],
            },
        );

        assert_eq!(disposition, IntentBlockDisposition::KeepStrategyState);
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_entry_missed_runtime_not_ready")
        );
        assert!(strategy.live_mr.position.is_none());
        assert!(strategy.live_bo.position.is_none());
        if let crate::state::StrategyState::RiAuthor4142Live {
            current_component,
            current_side,
            current_entry_ts_local,
            ..
        } = strategy.state()
        {
            assert_eq!(current_component.as_deref(), Some("author41_mr"));
            assert_eq!(current_side.as_deref(), Some("long"));
            assert_eq!(
                current_entry_ts_local.as_deref(),
                Some("2026-05-04 09:00:00")
            );
        } else {
            panic!("unexpected strategy state");
        }

        let opposite_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 10, 0),
            DataOrigin::Live,
            100_000.0,
            100_140.0,
            99_990.0,
            100_100.0,
        );
        let replacement_intents = strategy.on_bar(&live_ctx(), &opposite_bar);
        assert!(
            replacement_intents.is_empty(),
            "missed long must not be replaced by opposite short: {replacement_intents:?}"
        );
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_entry_missed_runtime_not_ready")
        );
    }

    #[test]
    fn restored_flat_state_clears_unpersisted_live_positions() {
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
        let previous_state = strategy.state().clone();

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
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_entry")
        );
        assert!(strategy.live_mr.position.is_some());

        strategy.set_state(previous_state);

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
        assert!(strategy.live_mr.position.is_none());
        assert!(strategy.live_bo.position.is_none());

        let exit_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 10, 0),
            DataOrigin::Live,
            100_090.0,
            100_100.0,
            99_890.0,
            99_900.0,
        );
        let intents_after_restore = strategy.on_bar(&live_ctx(), &exit_bar);
        assert!(
            intents_after_restore
                .iter()
                .all(|intent| intent.explicit_class() != Some(IntentClass::Exit)),
            "restored flat state must not emit stale exit intents: {intents_after_restore:?}"
        );
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
        strategy.on_position(
            &live_ctx_with_position(-1.0),
            &PositionEvent {
                symbol: "RIM6".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100_100.0,
                ts_utc: 1_776_000_020,
            },
        );
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_in_position")
        );

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
            Some("live_pending_exit")
        );
        strategy.on_position(
            &live_ctx_with_position(0.0),
            &PositionEvent {
                symbol: "RIM6".to_string(),
                qty: 0.0,
                existing: false,
                avg_price: 0.0,
                ts_utc: 1_776_000_620,
            },
        );
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
    fn micro_live_trading_window_closed_exit_reject_enters_deferred_exit_and_reissues() {
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
        assert_eq!(strategy.on_bar(&live_ctx(), &entry_bar).len(), 1);
        strategy.on_position(
            &live_ctx_with_position(-1.0),
            &PositionEvent {
                symbol: "RIM6".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100_100.0,
                ts_utc: 1_776_000_020,
            },
        );

        let exit_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 10, 0),
            DataOrigin::Live,
            100_090.0,
            100_100.0,
            99_890.0,
            99_900.0,
        );
        let exit_intents = strategy.on_bar(&live_ctx_with_position(-1.0), &exit_bar);
        assert_eq!(exit_intents.len(), 1);
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_exit")
        );
        let pending_exit_request_id = Uuid::new_v4();
        strategy.on_command_prepared(
            &live_ctx_with_position(-1.0),
            &CommandPrepared {
                request_id: pending_exit_request_id,
                intent_class: IntentClass::Exit,
                created_ts_utc: 1_776_000_600,
                symbol: "RIM6".to_string(),
                action: "market".to_string(),
                target_order_id: None,
            },
        );
        assert_eq!(
            strategy.pending_request_ids(),
            vec![pending_exit_request_id]
        );

        strategy.on_ack(
            &live_ctx_with_position(-1.0),
            &CommandAck {
                request_id: pending_exit_request_id,
                status: AckStatus::Rejected,
                broker_order_id: None,
                broker_order_id_str: None,
                error_code: Some("trading_window_closed".to_string()),
                error_msg: Some("validation failed".to_string()),
                cws_http_code: None,
                cws_message: None,
                cws_request_guid: None,
                processed_ts_utc: 1_776_000_620,
            },
        );
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_deferred_exit")
        );
        assert!(strategy.pending_request_ids().is_empty());

        let reissue_bar = bar_with_ohlc(
            dt(2026, 5, 4, 9, 20, 0),
            DataOrigin::Live,
            99_920.0,
            99_950.0,
            99_850.0,
            99_900.0,
        );
        let reissued = strategy.on_bar(&live_ctx_with_position(-1.0), &reissue_bar);
        assert_eq!(reissued.len(), 1);
        assert_eq!(reissued[0].explicit_class(), Some(IntentClass::Exit));
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_exit")
        );
    }

    #[test]
    fn micro_live_entry_ack_with_request_id_skew_does_not_clear_pending_entry() {
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
        assert_eq!(strategy.on_bar(&live_ctx(), &entry_bar).len(), 1);
        let request_id = Uuid::new_v4();
        strategy.on_command_prepared(
            &live_ctx(),
            &CommandPrepared {
                request_id,
                intent_class: IntentClass::Entry,
                created_ts_utc: 1_776_000_000,
                symbol: "RIM6".to_string(),
                action: "market".to_string(),
                target_order_id: None,
            },
        );

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
                processed_ts_utc: 1_776_000_010,
            },
        );

        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_entry")
        );
        assert_eq!(strategy.pending_request_ids(), vec![request_id]);

        strategy.on_ack(
            &live_ctx(),
            &CommandAck {
                request_id,
                status: AckStatus::Rejected,
                broker_order_id: None,
                broker_order_id_str: None,
                error_code: Some("cws_http_400".to_string()),
                error_msg: Some("unknown instrument".to_string()),
                cws_http_code: Some(400),
                cws_message: Some("unknown instrument".to_string()),
                cws_request_guid: None,
                processed_ts_utc: 1_776_000_020,
            },
        );
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some(RiAuthor4142Phase::Flat.as_str())
        );
        assert!(strategy.pending_request_ids().is_empty());
    }

    #[test]
    fn micro_live_promotes_pending_entry_to_in_position_on_position_update() {
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
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_pending_entry")
        );

        strategy.on_position(
            &live_ctx(),
            &PositionEvent {
                symbol: "RTS-6.26".to_string(),
                qty: -1.0,
                existing: false,
                avg_price: 100_100.0,
                ts_utc: 1_776_000_020,
            },
        );
        assert_eq!(
            strategy.phase_for_test().as_deref(),
            Some("live_in_position")
        );
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
    fn canonical07_feed_guard_excludes_auction_and_accepts_continuous_bars() {
        let mut config = default_config();
        config.session_start_time = "07:00:00".to_string();
        config.author41_entry_end_time = "10:00:00".to_string();
        config.anchor_first_bar_at_or_before = "07:10:00".to_string();
        let mut strategy = RiAuthor4142LiveStrategy::new(config).expect("strategy");

        let auction_bar = bar(dt(2026, 7, 16, 6, 50, 0), DataOrigin::Live);
        let first_continuous_bar = bar(dt(2026, 7, 16, 7, 0, 0), DataOrigin::Live);
        let pre_legacy_bar = bar(dt(2026, 7, 16, 8, 50, 0), DataOrigin::Live);

        assert!(strategy.update_bar_state(&auction_bar).is_empty());
        assert!(strategy.update_bar_state(&first_continuous_bar).is_empty());
        assert!(strategy.update_bar_state(&pre_legacy_bar).is_empty());

        assert_eq!(strategy.model_bars.len(), 2);
        assert_eq!(strategy.model_bars[0].ts_local, dt(2026, 7, 16, 7, 0, 0));
        assert_eq!(strategy.model_bars[1].ts_local, dt(2026, 7, 16, 8, 50, 0));
    }

    #[test]
    fn transition_feed_guard_excludes_legacy_pre_session_service_bar() {
        let mut strategy = RiAuthor4142LiveStrategy::new(canonical07_transition_config())
            .expect("transition strategy");

        assert!(strategy
            .update_bar_state(&bar(dt(2026, 7, 13, 8, 50, 0), DataOrigin::History))
            .is_empty());
        assert!(strategy
            .update_bar_state(&bar(dt(2026, 7, 13, 9, 0, 0), DataOrigin::History))
            .is_empty());
        assert!(strategy
            .update_bar_state(&bar(dt(2026, 7, 14, 7, 0, 0), DataOrigin::History))
            .is_empty());

        assert_eq!(strategy.model_bars.len(), 2);
        assert_eq!(strategy.model_bars[0].ts_local, dt(2026, 7, 13, 9, 0, 0));
        assert_eq!(strategy.model_bars[1].ts_local, dt(2026, 7, 14, 7, 0, 0));

        let mut prospective_config = canonical07_transition_config();
        prospective_config.mode = RiAuthor4142RuntimeMode::ProspectiveShadow;
        let mut prospective =
            RiAuthor4142LiveStrategy::new(prospective_config).expect("prospective transition");
        for event in [
            bar(dt(2026, 7, 13, 8, 50, 0), DataOrigin::History),
            bar(dt(2026, 7, 13, 9, 0, 0), DataOrigin::History),
            bar(dt(2026, 7, 14, 7, 0, 0), DataOrigin::History),
        ] {
            assert!(prospective.update_bar_state(&event).is_empty());
        }
        assert_eq!(prospective.model_bars, strategy.model_bars);
    }

    #[test]
    fn operational_shadow_excludes_moex_break_bars_without_changing_live_contract() {
        let break_one = bar(dt(2026, 7, 15, 14, 0, 0), DataOrigin::Live);
        let break_two = bar(dt(2026, 7, 15, 18, 50, 0), DataOrigin::Live);

        let mut shadow = RiAuthor4142LiveStrategy::new(default_config()).expect("shadow");
        assert!(shadow.update_bar_state(&break_one).is_empty());
        assert!(shadow.update_bar_state(&break_two).is_empty());
        assert!(shadow.model_bars.is_empty());

        let mut prospective_config = default_config();
        prospective_config.mode = RiAuthor4142RuntimeMode::ProspectiveShadow;
        let mut prospective =
            RiAuthor4142LiveStrategy::new(prospective_config).expect("prospective");
        assert!(prospective.on_bar(&test_ctx(), &break_one).is_empty());
        assert!(prospective.on_bar(&test_ctx(), &break_two).is_empty());
        assert!(prospective.model_bars.is_empty());
        assert!(prospective.journal_records_for_test().iter().all(|row| {
            row.adapter_decision != RiAuthor4142JournalDecision::ProspectiveIntentSuppressed
        }));

        let mut live_config = default_config();
        live_config.mode = RiAuthor4142RuntimeMode::MicroLive;
        live_config.allow_order_emission = true;
        let mut live = RiAuthor4142LiveStrategy::new(live_config).expect("live");
        assert!(live.update_bar_state(&break_one).is_empty());
        assert!(live.update_bar_state(&break_two).is_empty());
        assert_eq!(live.model_bars.len(), 2);
    }

    #[test]
    fn legacy09_feed_guard_still_rejects_pre_0900_bars() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");

        assert!(strategy
            .update_bar_state(&bar(dt(2026, 7, 16, 8, 50, 0), DataOrigin::Live))
            .is_empty());
        assert!(strategy
            .update_bar_state(&bar(dt(2026, 7, 16, 9, 0, 0), DataOrigin::Live))
            .is_empty());

        assert_eq!(strategy.model_bars.len(), 1);
        assert_eq!(strategy.model_bars[0].ts_local, dt(2026, 7, 16, 9, 0, 0));
    }

    #[test]
    fn canonical07_clock_translation_keeps_late_exits_unchanged() {
        let mut config = default_config();
        config.session_start_time = "07:00:00".to_string();
        config.author41_entry_end_time = "10:00:00".to_string();
        config.author41_time_exit = "20:00:00".to_string();
        config.author42_exit_time = "23:00:00".to_string();

        let strategy = RiAuthor4142LiveStrategy::new(config).expect("strategy");

        assert_eq!(
            strategy.session_policy.start,
            chrono::NaiveTime::from_hms_opt(7, 0, 0).unwrap()
        );
        assert_eq!(
            strategy.author41_short_config().entry_end,
            chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap()
        );
        assert_eq!(
            strategy.author41_long_config().entry_end,
            chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap()
        );
        assert_eq!(
            strategy.author41_short_config().time_exit,
            chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap()
        );
        assert_eq!(
            strategy.author42_config().exit_time,
            chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap()
        );
    }

    #[test]
    fn author42_hour_check_sequence_is_relative_to_first_model_bar() {
        let first = dt(2026, 7, 16, 7, 0, 0);

        assert!(!RiAuthor4142LiveStrategy::is_author42_hour_check(
            dt(2026, 7, 16, 7, 50, 0),
            Some(first)
        ));
        assert!(RiAuthor4142LiveStrategy::is_author42_hour_check(
            dt(2026, 7, 16, 8, 50, 0),
            Some(first)
        ));
    }

    #[test]
    fn configured_special_session_is_excluded_from_model_and_live_emission() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;
        config.allow_order_emission = true;
        config.excluded_model_dates = vec!["2026-06-12".to_string()];
        let mut strategy = RiAuthor4142LiveStrategy::new(config).expect("strategy");

        let special_bar = bar_with_ohlc(
            dt(2026, 6, 12, 10, 0, 0),
            DataOrigin::Live,
            100_000.0,
            101_000.0,
            99_000.0,
            100_100.0,
        );

        assert!(strategy.on_bar(&live_ctx(), &special_bar).is_empty());
        assert!(strategy.model_bars.is_empty());
    }

    #[test]
    fn anchor_guard_skips_latest_incomplete_session() {
        let mut config = default_config();
        config.min_anchor_bars = 3;
        config.anchor_first_bar_at_or_before = "09:10:00".to_string();
        config.anchor_last_bar_at_or_after = "23:30:00".to_string();
        let mut strategy = RiAuthor4142LiveStrategy::new(config).expect("strategy");

        for bar in [
            bar_with_ohlc(
                dt(2026, 6, 11, 9, 0, 0),
                DataOrigin::History,
                100_000.0,
                100_100.0,
                99_900.0,
                100_000.0,
            ),
            bar_with_ohlc(
                dt(2026, 6, 11, 12, 0, 0),
                DataOrigin::History,
                100_000.0,
                101_000.0,
                99_000.0,
                100_500.0,
            ),
            bar_with_ohlc(
                dt(2026, 6, 11, 23, 40, 0),
                DataOrigin::History,
                100_500.0,
                100_700.0,
                100_300.0,
                100_600.0,
            ),
            bar_with_ohlc(
                dt(2026, 6, 12, 10, 0, 0),
                DataOrigin::History,
                90_000.0,
                90_100.0,
                89_900.0,
                90_000.0,
            ),
            bar_with_ohlc(
                dt(2026, 6, 12, 23, 30, 0),
                DataOrigin::History,
                90_000.0,
                91_000.0,
                89_000.0,
                90_500.0,
            ),
        ] {
            strategy.update_bar_state(&bar);
        }

        let anchor = strategy
            .mr_anchor_for_date(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap())
            .expect("eligible 2026-06-11 anchor");
        assert_eq!(anchor.prev_close, 100_600.0);
        assert_eq!(anchor.prev_low, 99_000.0);
        assert_eq!(anchor.prev_range, 2_000.0);
    }

    #[test]
    fn transition_anchor_guard_accepts_complete_legacy_sessions_only_before_cutover() {
        let strategy = RiAuthor4142LiveStrategy::new(canonical07_transition_config())
            .expect("transition strategy");
        let legacy_stats = super::RiDailyStats {
            high: 100.0,
            low: 90.0,
            close: 95.0,
            bars: 86,
            first_bar: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            last_bar: chrono::NaiveTime::from_hms_opt(23, 40, 0).unwrap(),
        };
        let canonical_stats = super::RiDailyStats {
            bars: 98,
            first_bar: chrono::NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
            ..legacy_stats
        };

        assert!(strategy.is_eligible_anchor_session(
            NaiveDate::from_ymd_opt(2026, 7, 13).unwrap(),
            legacy_stats
        ));
        assert!(!strategy.is_eligible_anchor_session(
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            legacy_stats
        ));
        assert!(strategy.is_eligible_anchor_session(
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            canonical_stats
        ));
    }

    #[test]
    fn transition_fixture_restores_expected_early_author42_bo_paths() {
        let mut transition = RiAuthor4142LiveStrategy::new(canonical07_transition_config())
            .expect("transition strategy");
        transition.model_bars = canonical07_transition_fixture_bars();
        let decisions = transition.collect_new_decisions(dt(2026, 7, 15, 23, 40, 0));

        let expected = [
            (dt(2026, 7, 14, 15, 0, 0), dt(2026, 7, 14, 23, 0, 0), 718.0),
            (dt(2026, 7, 15, 21, 0, 0), dt(2026, 7, 15, 23, 0, 0), 208.0),
        ];
        for (entry, exit, pnl) in expected {
            assert!(decisions.iter().any(|decision| {
                decision.component == super::RiAuthor4142Component::Author42Bo
                    && decision.overlap_decision == "Accepted"
                    && decision.scheduled_entry_ts_local == Some(entry)
                    && decision.scheduled_exit_ts_local == Some(exit)
                    && decision.shadow_pnl_points == Some(pnl)
            }));
        }

        let mut uniform_config = canonical07_transition_config();
        uniform_config.anchor_transition_date = None;
        uniform_config.pre_transition_min_anchor_bars = None;
        uniform_config.pre_transition_anchor_first_bar_at_or_before = None;
        uniform_config.pre_transition_anchor_last_bar_at_or_after = None;
        let mut uniform = RiAuthor4142LiveStrategy::new(uniform_config).expect("uniform strategy");
        uniform.model_bars = canonical07_transition_fixture_bars();
        let uniform_decisions = uniform.collect_new_decisions(dt(2026, 7, 15, 23, 40, 0));
        assert!(!uniform_decisions.iter().any(|decision| {
            decision.component == super::RiAuthor4142Component::Author42Bo
                && decision.scheduled_entry_ts_local == Some(dt(2026, 7, 14, 15, 0, 0))
        }));
    }

    #[test]
    fn rollover_policy_requires_ordered_positive_offsets() {
        let mut config = default_config();
        config.roll_target_sessions_before = 2;
        config.roll_fallback_sessions_before = 1;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("fallback cannot be later than target")
            .to_string();
        assert!(err.contains("rollover requires"));
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
    fn shadow_path_marks_recomputed_overlap_decision_as_superseded() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let later_long = accepted_shadow_decision(
            Component::Author41Mr,
            ShadowSide::Long,
            dt(2026, 7, 20, 9, 0, 0),
            dt(2026, 7, 20, 9, 10, 0),
            "take_author_close",
            208.0,
        );
        let later_long_key = later_long.decision_key.clone();

        strategy.collect_shadow_decisions(vec![later_long], dt(2026, 7, 20, 9, 20, 0));

        let active_after_first = strategy
            .shadow_path_keys_by_date
            .get(&NaiveDate::from_ymd_opt(2026, 7, 20).unwrap())
            .expect("path date");
        assert!(active_after_first.contains(&later_long_key));

        let earlier_short = accepted_shadow_decision(
            Component::Author41Mr,
            ShadowSide::Short,
            dt(2026, 7, 20, 7, 40, 0),
            dt(2026, 7, 20, 9, 30, 0),
            "take_author_close",
            918.0,
        );
        let earlier_short_key = earlier_short.decision_key.clone();

        strategy.collect_shadow_decisions(vec![earlier_short], dt(2026, 7, 20, 9, 40, 0));

        let active_after_recompute = strategy
            .shadow_path_keys_by_date
            .get(&NaiveDate::from_ymd_opt(2026, 7, 20).unwrap())
            .expect("path date");
        assert!(!active_after_recompute.contains(&later_long_key));
        assert!(active_after_recompute.contains(&earlier_short_key));

        let journal = strategy.journal_records_for_test();
        assert!(journal.iter().any(|row| {
            row.adapter_decision == RiAuthor4142JournalDecision::ShadowPathSuperseded
                && row.decision_key == later_long_key
        }));
        assert!(journal.iter().any(|row| {
            row.adapter_decision == RiAuthor4142JournalDecision::ShadowPathActive
                && row.decision_key == earlier_short_key
        }));
    }

    #[test]
    fn shadow_path_does_not_duplicate_active_rows_when_path_is_unchanged() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let decision = accepted_shadow_decision(
            Component::Author41Mr,
            ShadowSide::Long,
            dt(2026, 7, 20, 9, 0, 0),
            dt(2026, 7, 20, 9, 10, 0),
            "take_author_close",
            208.0,
        );

        strategy.collect_shadow_decisions(vec![decision.clone()], dt(2026, 7, 20, 9, 20, 0));
        strategy.collect_shadow_decisions(vec![decision], dt(2026, 7, 20, 9, 30, 0));

        let active_rows = strategy
            .journal_records_for_test()
            .iter()
            .filter(|row| row.adapter_decision == RiAuthor4142JournalDecision::ShadowPathActive)
            .count();
        let superseded_rows = strategy
            .journal_records_for_test()
            .iter()
            .filter(|row| row.adapter_decision == RiAuthor4142JournalDecision::ShadowPathSuperseded)
            .count();

        assert_eq!(active_rows, 1);
        assert_eq!(superseded_rows, 0);
    }

    #[test]
    fn shadow_path_keeps_late_bo_after_mr_time_exit() {
        let mut strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        let mr = accepted_shadow_decision(
            Component::Author41Mr,
            ShadowSide::Long,
            dt(2026, 7, 15, 8, 10, 0),
            dt(2026, 7, 15, 20, 0, 0),
            "time_exit",
            -1342.0,
        );
        let late_bo = accepted_shadow_decision(
            Component::Author42Bo,
            ShadowSide::Short,
            dt(2026, 7, 15, 21, 0, 0),
            dt(2026, 7, 15, 23, 0, 0),
            "time_exit_same_bar_close",
            208.0,
        );
        let late_bo_key = late_bo.decision_key.clone();

        strategy.collect_shadow_decisions(vec![mr, late_bo], dt(2026, 7, 15, 23, 10, 0));

        let active = strategy
            .shadow_path_keys_by_date
            .get(&NaiveDate::from_ymd_opt(2026, 7, 15).unwrap())
            .expect("path date");
        assert!(active.contains(&late_bo_key));
        assert!(strategy.journal_records_for_test().iter().any(|row| {
            row.adapter_decision == RiAuthor4142JournalDecision::ShadowPathActive
                && row.decision_key == late_bo_key
        }));
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

    fn accepted_shadow_decision(
        component: Component,
        side: ShadowSide,
        entry_ts: NaiveDateTime,
        exit_ts: NaiveDateTime,
        reason: &str,
        pnl_points: f64,
    ) -> super::RiAuthor4142ModelDecision {
        let mut record = sample_record(OverlapDecision::Accepted, reason);
        record.component = component;
        record.side = Some(side);
        record.bar_ts_local = entry_ts;
        record.scheduled_entry_ts_local = Some(entry_ts);
        record.scheduled_exit_ts_local = Some(exit_ts);
        record.exit_reason = Some(reason.to_string());
        record.shadow_pnl_points = Some(pnl_points);
        let key = super::decision_key(&record);
        super::RiAuthor4142ModelDecision::from_shadow_record(record, key).expect("decision")
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

    fn live_ctx_with_position(qty: f64) -> crate::StrategyCtx {
        crate::StrategyCtx {
            position_qty: Some(qty),
            ..live_ctx()
        }
    }
}
