use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Instrument {
    Imoexf,
    Ri,
}

impl Instrument {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imoexf => "IMOEXF",
            Self::Ri => "RI",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileId {
    ImoexfAuthor41_42PrimaryComboCost2,
    RiAuthor41_42PrimaryComboCost2,
}

impl ProfileId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImoexfAuthor41_42PrimaryComboCost2 => "imoexf_author41_42_primary_combo_cost2",
            Self::RiAuthor41_42PrimaryComboCost2 => "ri_author41_42_primary_combo_cost2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Author41Mr,
    Author42Bo,
    Combo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowSide {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapDecision {
    Accepted,
    DroppedMrOverlap,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowRuntimeMode {
    ReplayOnly,
    ShadowOnly,
}

impl ShadowRuntimeMode {
    pub fn can_emit_orders(self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelProfile {
    pub profile_id: ProfileId,
    pub instrument: Instrument,
    pub timeframe: &'static str,
    pub session_policy: RegularSessionPolicy,
    pub author41_variant: &'static str,
    pub author42_variant: &'static str,
    pub combo_variant: &'static str,
    pub author42_k: f64,
    pub author42_cost_points: f64,
    pub no_overlap_enforced: bool,
    pub source_package: &'static str,
}

impl ModelProfile {
    pub fn ri_shadow_10m() -> Self {
        Self {
            profile_id: ProfileId::RiAuthor41_42PrimaryComboCost2,
            instrument: Instrument::Ri,
            timeframe: "10m",
            session_policy: RegularSessionPolicy::moex_10m(),
            author41_variant: "dual_no_overlap_plateau",
            author42_variant: "grid_k0.42_both",
            combo_variant: "ri_41dual_42best_cost2_nooverlap",
            author42_k: 0.42,
            author42_cost_points: 2.0,
            no_overlap_enforced: true,
            source_package: "analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04",
        }
    }

    pub fn imoexf_passive_shadow_10m() -> Self {
        Self {
            profile_id: ProfileId::ImoexfAuthor41_42PrimaryComboCost2,
            instrument: Instrument::Imoexf,
            timeframe: "10m",
            session_policy: RegularSessionPolicy::moex_10m(),
            author41_variant: "author41_boundary_short",
            author42_variant: "grid_k0.44_both",
            combo_variant: "imoexf_41short_42best_cost2_nooverlap",
            author42_k: 0.44,
            author42_cost_points: 2.0,
            no_overlap_enforced: true,
            source_package: "analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegularSessionPolicy {
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub weekdays_only: bool,
}

impl RegularSessionPolicy {
    pub fn moex_10m() -> Self {
        Self {
            start: NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN),
            end: NaiveTime::from_hms_opt(23, 49, 59)
                .unwrap_or_else(|| NaiveTime::from_hms_opt(23, 59, 59).unwrap_or(NaiveTime::MIN)),
            weekdays_only: true,
        }
    }

    pub fn is_model_bar(self, dt_local: NaiveDateTime) -> bool {
        if self.weekdays_only && dt_local.weekday().number_from_monday() > 5 {
            return false;
        }
        let t = dt_local.time();
        self.start <= t && t <= self.end
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowJournalRecord {
    pub instrument: Instrument,
    pub profile_id: ProfileId,
    pub component: Component,
    pub model_variant_id: String,
    pub bar_ts_local: NaiveDateTime,
    pub timeframe: String,
    pub prev_regular_date: Option<NaiveDate>,
    pub prev_close: Option<f64>,
    pub prev_high: Option<f64>,
    pub prev_low: Option<f64>,
    pub prev_range: Option<f64>,
    pub trigger_long: Option<f64>,
    pub trigger_short: Option<f64>,
    pub condition_values: Vec<(String, f64)>,
    pub side: Option<ShadowSide>,
    pub skip_reason: Option<String>,
    pub scheduled_entry_ts_local: Option<NaiveDateTime>,
    pub scheduled_entry_price: Option<f64>,
    pub scheduled_exit_ts_local: Option<NaiveDateTime>,
    pub exit_reason: Option<String>,
    pub overlap_decision: OverlapDecision,
    pub shadow_pnl_points: Option<f64>,
    pub feed_quality_flags: Vec<String>,
}

impl ShadowJournalRecord {
    pub fn skipped(
        profile: &ModelProfile,
        component: Component,
        bar_ts_local: NaiveDateTime,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            instrument: profile.instrument,
            profile_id: profile.profile_id,
            component,
            model_variant_id: match component {
                Component::Author41Mr => profile.author41_variant,
                Component::Author42Bo => profile.author42_variant,
                Component::Combo => profile.combo_variant,
            }
            .to_string(),
            bar_ts_local,
            timeframe: profile.timeframe.to_string(),
            prev_regular_date: None,
            prev_close: None,
            prev_high: None,
            prev_low: None,
            prev_range: None,
            trigger_long: None,
            trigger_short: None,
            condition_values: Vec::new(),
            side: None,
            skip_reason: Some(reason.into()),
            scheduled_entry_ts_local: None,
            scheduled_entry_price: None,
            scheduled_exit_ts_local: None,
            exit_reason: None,
            overlap_decision: OverlapDecision::NotApplicable,
            shadow_pnl_points: None,
            feed_quality_flags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceTrade {
    pub fixed_model_id: String,
    pub side: ShadowSide,
    pub entry_ts: NaiveDateTime,
    pub exit_ts: NaiveDateTime,
    pub entry_price: f64,
    pub exit_price: f64,
    pub points: f64,
    pub exit_reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceDaily {
    pub fixed_model_id: String,
    pub date: NaiveDate,
    pub pnl_points: f64,
    pub author41_pnl: Option<f64>,
    pub author42_pnl: Option<f64>,
    pub trades: Option<f64>,
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Author41Config {
    pub side_mode: Author41SideMode,
    pub k: f64,
    pub k2: f64,
    pub stop_k: f64,
    pub min_range: f64,
    pub max_range: f64,
    pub max_entries_per_day: u32,
    pub entry_end: NaiveTime,
    pub time_exit: NaiveTime,
    pub breakeven_after_bars: u32,
    pub roundtrip_cost_points: f64,
}

impl Author41Config {
    pub fn ri_dual_no_overlap_plateau() -> Self {
        Self {
            side_mode: Author41SideMode::Dual,
            k: 0.07,
            k2: 0.005,
            stop_k: 0.58,
            min_range: 0.016,
            max_range: 0.045,
            max_entries_per_day: 2,
            entry_end: NaiveTime::from_hms_opt(12, 0, 0).unwrap_or(NaiveTime::MIN),
            time_exit: NaiveTime::from_hms_opt(20, 0, 0).unwrap_or(NaiveTime::MIN),
            breakeven_after_bars: 20,
            roundtrip_cost_points: 2.0,
        }
    }

    pub fn ri_plateau_short_source() -> Self {
        Self {
            side_mode: Author41SideMode::Short,
            k: 0.20,
            k2: 0.020,
            stop_k: 0.75,
            min_range: 0.005,
            max_range: 0.100,
            max_entries_per_day: 2,
            entry_end: NaiveTime::from_hms_opt(12, 0, 0).unwrap_or(NaiveTime::MIN),
            time_exit: NaiveTime::from_hms_opt(20, 0, 0).unwrap_or(NaiveTime::MIN),
            breakeven_after_bars: 20,
            roundtrip_cost_points: 2.0,
        }
    }

    pub fn ri_plateau_long_source() -> Self {
        Self {
            side_mode: Author41SideMode::Long,
            k: 0.11,
            k2: 0.005,
            stop_k: 1.00,
            min_range: 0.005,
            max_range: 0.100,
            max_entries_per_day: 2,
            entry_end: NaiveTime::from_hms_opt(12, 0, 0).unwrap_or(NaiveTime::MIN),
            time_exit: NaiveTime::from_hms_opt(20, 0, 0).unwrap_or(NaiveTime::MIN),
            breakeven_after_bars: 20,
            roundtrip_cost_points: 2.0,
        }
    }

    pub fn imoexf_boundary_short() -> Self {
        Self {
            side_mode: Author41SideMode::Short,
            k: 0.16,
            k2: 0.020,
            stop_k: 0.58,
            min_range: 0.005,
            max_range: 0.075,
            max_entries_per_day: 2,
            entry_end: NaiveTime::from_hms_opt(12, 0, 0).unwrap_or(NaiveTime::MIN),
            time_exit: NaiveTime::from_hms_opt(20, 0, 0).unwrap_or(NaiveTime::MIN),
            breakeven_after_bars: 20,
            roundtrip_cost_points: 0.1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Author42Config {
    pub k: f64,
    pub stop_hour_k: f64,
    pub stop_k: f64,
    pub side_mode: Author42SideMode,
    pub min_prev_hl_ratio: f64,
    pub prev_extreme_move: f64,
    pub first_hour_extreme_k: f64,
    pub use_first_hour_extreme_filter: bool,
    pub exclude_friday: bool,
    pub exclude_june_window: bool,
    pub allow_reentry_on_day_extreme: bool,
    pub roundtrip_cost_points: f64,
    pub exit_time: NaiveTime,
}

impl Author42Config {
    pub fn ri_grid_k042_both() -> Self {
        Self {
            k: 0.42,
            stop_hour_k: 0.50,
            stop_k: 0.18,
            side_mode: Author42SideMode::Both,
            min_prev_hl_ratio: 1.01,
            prev_extreme_move: 0.025,
            first_hour_extreme_k: 1.50,
            use_first_hour_extreme_filter: true,
            exclude_friday: true,
            exclude_june_window: true,
            allow_reentry_on_day_extreme: true,
            roundtrip_cost_points: 0.0,
            exit_time: NaiveTime::from_hms_opt(23, 0, 0).unwrap_or(NaiveTime::MIN),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author42SideMode {
    Long,
    Short,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Author42DailyContext {
    prev_close: f64,
    prev2_close: f64,
    prev_range: f64,
    prev_hl_ratio: f64,
    prev_ret: f64,
}

pub fn replay_ri_author41_dual_no_overlap_source(
    bars: &[ModelBar],
    session_policy: RegularSessionPolicy,
) -> Author41ReplayResult {
    let mut filtered: Vec<ModelBar> = bars
        .iter()
        .copied()
        .filter(|bar| session_policy.is_model_bar(bar.ts_local))
        .collect();
    filtered.sort_by_key(|bar| bar.ts_local);

    let short = replay_author41(
        &filtered,
        Author41Config::ri_plateau_short_source(),
        session_policy,
    );
    let long = replay_author41(
        &filtered,
        Author41Config::ri_plateau_long_source(),
        session_policy,
    );

    let mut candidates = short.trades;
    candidates.extend(long.trades);
    candidates.sort_by_key(|trade| trade.entry_ts);

    let mut accepted = Vec::new();
    let mut last_exit = NaiveDateTime::MIN;
    for trade in candidates {
        if trade.entry_ts <= last_exit {
            continue;
        }
        last_exit = trade.exit_ts;
        accepted.push(trade);
    }

    let mut daily_map: BTreeMap<NaiveDate, Author41DailyPnl> = filtered
        .iter()
        .map(|bar| {
            (
                bar.ts_local.date(),
                Author41DailyPnl {
                    date: bar.ts_local.date(),
                    pnl_points: 0.0,
                    trades: 0,
                },
            )
        })
        .collect();
    for trade in &accepted {
        let row = daily_map
            .entry(trade.exit_ts.date())
            .or_insert(Author41DailyPnl {
                date: trade.exit_ts.date(),
                pnl_points: 0.0,
                trades: 0,
            });
        row.pnl_points += trade.net_points;
        row.trades += 1;
    }

    Author41ReplayResult {
        trades: accepted,
        daily: daily_map.into_values().collect(),
    }
}

pub fn replay_author42(
    bars: &[ModelBar],
    config: Author42Config,
    session_policy: RegularSessionPolicy,
) -> Author42ReplayResult {
    let mut filtered: Vec<ModelBar> = bars
        .iter()
        .copied()
        .filter(|bar| session_policy.is_model_bar(bar.ts_local))
        .collect();
    filtered.sort_by_key(|bar| bar.ts_local);

    let contexts = build_author42_daily_context(&filtered);
    let mut by_day: BTreeMap<NaiveDate, Vec<ModelBar>> = BTreeMap::new();
    for bar in filtered {
        by_day.entry(bar.ts_local.date()).or_default().push(bar);
    }

    let mut trades = Vec::new();
    let mut daily = Vec::new();
    for (date, day) in by_day {
        let Some(ctx) = contexts.get(&date).copied() else {
            daily.push(Author42DailyPnl {
                date,
                pnl_points: 0.0,
                trades: 0,
                skipped: "missing_context".to_string(),
            });
            continue;
        };
        let (day_trades, day_row) = run_author42_day(date, &day, ctx, config);
        trades.extend(day_trades);
        daily.push(day_row);
    }
    Author42ReplayResult { trades, daily }
}

pub fn replay_ri_author41_42_combo_cost2(
    bars: &[ModelBar],
    session_policy: RegularSessionPolicy,
) -> ComboReplayResult {
    let profile = ModelProfile::ri_shadow_10m();
    let author41 = replay_ri_author41_dual_no_overlap_source(bars, session_policy);
    let author42 = replay_author42(bars, Author42Config::ri_grid_k042_both(), session_policy);
    replay_combo_from_components(&profile, &author41, &author42)
}

pub fn build_ri_author41_42_combo_shadow_journal(
    bars: &[ModelBar],
    session_policy: RegularSessionPolicy,
) -> Vec<ShadowJournalRecord> {
    let profile = ModelProfile::ri_shadow_10m();
    let author41 = replay_ri_author41_dual_no_overlap_source(bars, session_policy);
    let author42 = replay_author42(bars, Author42Config::ri_grid_k042_both(), session_policy);

    let mut records = Vec::new();
    for trade in &author41.trades {
        records.push(author41_trade_journal_record(&profile, trade));
    }
    for trade in &author42.trades {
        let dropped = author41.trades.iter().any(|mr| {
            trade.entry_ts.date() == mr.entry_ts.date()
                && trade.entry_ts.max(mr.entry_ts) < trade.exit_ts.min(mr.exit_ts)
        });
        records.push(author42_trade_journal_record(
            &profile,
            trade,
            if dropped {
                OverlapDecision::DroppedMrOverlap
            } else {
                OverlapDecision::Accepted
            },
        ));
    }

    records.sort_by(|left, right| {
        (
            left.bar_ts_local,
            component_sort_key(left.component),
            left.scheduled_exit_ts_local,
        )
            .cmp(&(
                right.bar_ts_local,
                component_sort_key(right.component),
                right.scheduled_exit_ts_local,
            ))
    });
    records
}

fn author41_trade_journal_record(
    profile: &ModelProfile,
    trade: &Author41Trade,
) -> ShadowJournalRecord {
    ShadowJournalRecord {
        instrument: profile.instrument,
        profile_id: profile.profile_id,
        component: Component::Author41Mr,
        model_variant_id: profile.author41_variant.to_string(),
        bar_ts_local: trade.entry_ts,
        timeframe: profile.timeframe.to_string(),
        prev_regular_date: None,
        prev_close: None,
        prev_high: None,
        prev_low: None,
        prev_range: None,
        trigger_long: None,
        trigger_short: None,
        condition_values: Vec::new(),
        side: Some(trade.side),
        skip_reason: None,
        scheduled_entry_ts_local: Some(trade.entry_ts),
        scheduled_entry_price: Some(trade.entry_price),
        scheduled_exit_ts_local: Some(trade.exit_ts),
        exit_reason: Some(trade.exit_reason.clone()),
        overlap_decision: OverlapDecision::Accepted,
        shadow_pnl_points: Some(trade.net_points),
        feed_quality_flags: Vec::new(),
    }
}

fn author42_trade_journal_record(
    profile: &ModelProfile,
    trade: &Author42Trade,
    overlap_decision: OverlapDecision,
) -> ShadowJournalRecord {
    let accepted = overlap_decision == OverlapDecision::Accepted;
    ShadowJournalRecord {
        instrument: profile.instrument,
        profile_id: profile.profile_id,
        component: Component::Author42Bo,
        model_variant_id: profile.author42_variant.to_string(),
        bar_ts_local: trade.entry_ts,
        timeframe: profile.timeframe.to_string(),
        prev_regular_date: None,
        prev_close: None,
        prev_high: None,
        prev_low: None,
        prev_range: None,
        trigger_long: None,
        trigger_short: None,
        condition_values: Vec::new(),
        side: Some(trade.side),
        skip_reason: (!accepted).then(|| "mr_interval_overlap".to_string()),
        scheduled_entry_ts_local: Some(trade.entry_ts),
        scheduled_entry_price: Some(trade.entry_price),
        scheduled_exit_ts_local: Some(trade.exit_ts),
        exit_reason: Some(trade.exit_reason.clone()),
        overlap_decision,
        shadow_pnl_points: accepted.then_some(trade.gross_points - profile.author42_cost_points),
        feed_quality_flags: Vec::new(),
    }
}

fn component_sort_key(component: Component) -> u8 {
    match component {
        Component::Author41Mr => 0,
        Component::Author42Bo => 1,
        Component::Combo => 2,
    }
}

pub fn replay_combo_from_components(
    profile: &ModelProfile,
    author41: &Author41ReplayResult,
    author42: &Author42ReplayResult,
) -> ComboReplayResult {
    let accepted_author42 = filter_author42_non_overlapping(&author42.trades, &author41.trades);

    let mut rows: BTreeMap<NaiveDate, ComboDailyPnl> = BTreeMap::new();
    for row in &author41.daily {
        rows.entry(row.date).or_insert_with(|| ComboDailyPnl {
            date: row.date,
            author41_pnl: 0.0,
            author42_pnl: 0.0,
            author41_trades: 0,
            author42_trades: 0,
            pnl_points: 0.0,
            trades: 0,
        });
    }
    for row in &author42.daily {
        rows.entry(row.date).or_insert_with(|| ComboDailyPnl {
            date: row.date,
            author41_pnl: 0.0,
            author42_pnl: 0.0,
            author41_trades: 0,
            author42_trades: 0,
            pnl_points: 0.0,
            trades: 0,
        });
    }

    for trade in &author41.trades {
        let row = rows
            .entry(trade.exit_ts.date())
            .or_insert_with(|| ComboDailyPnl {
                date: trade.exit_ts.date(),
                author41_pnl: 0.0,
                author42_pnl: 0.0,
                author41_trades: 0,
                author42_trades: 0,
                pnl_points: 0.0,
                trades: 0,
            });
        row.author41_pnl += trade.net_points;
    }
    for trade in &accepted_author42 {
        let row = rows
            .entry(trade.exit_ts.date())
            .or_insert_with(|| ComboDailyPnl {
                date: trade.exit_ts.date(),
                author41_pnl: 0.0,
                author42_pnl: 0.0,
                author41_trades: 0,
                author42_trades: 0,
                pnl_points: 0.0,
                trades: 0,
            });
        row.author42_pnl += trade.gross_points - profile.author42_cost_points;
        row.author42_trades += 1;
    }

    let mut daily: Vec<ComboDailyPnl> = rows
        .into_values()
        .map(|mut row| {
            // The frozen combo artifact stores Author41 "trades" as active-day
            // count, while the detailed trade log above keeps true trade count.
            row.author41_trades = u32::from(row.author41_pnl.abs() > 0.0);
            row.pnl_points = row.author41_pnl + row.author42_pnl;
            row.trades = row.author41_trades + row.author42_trades;
            row
        })
        .collect();
    daily.sort_by_key(|row| row.date);

    ComboReplayResult {
        author41_trades: author41.trades.clone(),
        author42_trades: accepted_author42,
        daily,
    }
}

fn filter_author42_non_overlapping(
    candidate: &[Author42Trade],
    blocker: &[Author41Trade],
) -> Vec<Author42Trade> {
    candidate
        .iter()
        .filter(|trade| {
            !blocker.iter().any(|mr| {
                trade.entry_ts.date() == mr.entry_ts.date()
                    && trade.entry_ts.max(mr.entry_ts) < trade.exit_ts.min(mr.exit_ts)
            })
        })
        .cloned()
        .collect()
}

fn run_author42_day(
    date: NaiveDate,
    day: &[ModelBar],
    ctx: Author42DailyContext,
    config: Author42Config,
) -> (Vec<Author42Trade>, Author42DailyPnl) {
    if config.exclude_friday && date.weekday().number_from_monday() == 5 {
        return (
            Vec::new(),
            Author42DailyPnl {
                date,
                pnl_points: 0.0,
                trades: 0,
                skipped: "friday".to_string(),
            },
        );
    }
    if config.exclude_june_window && in_author_june_window(date) {
        return (
            Vec::new(),
            Author42DailyPnl {
                date,
                pnl_points: 0.0,
                trades: 0,
                skipped: "june_window".to_string(),
            },
        );
    }
    if !all_finite(&[
        ctx.prev_close,
        ctx.prev2_close,
        ctx.prev_range,
        ctx.prev_hl_ratio,
        ctx.prev_ret,
    ]) {
        return (
            Vec::new(),
            Author42DailyPnl {
                date,
                pnl_points: 0.0,
                trades: 0,
                skipped: "missing_context".to_string(),
            },
        );
    }
    if ctx.prev_range <= 0.0 || ctx.prev_close <= 0.0 {
        return (
            Vec::new(),
            Author42DailyPnl {
                date,
                pnl_points: 0.0,
                trades: 0,
                skipped: "bad_context".to_string(),
            },
        );
    }

    let range_ok = ctx.prev_hl_ratio > config.min_prev_hl_ratio;
    let mut buy_trig = range_ok && ctx.prev_ret > -config.prev_extreme_move;
    let mut short_trig = range_ok && ctx.prev_ret < config.prev_extreme_move;
    match config.side_mode {
        Author42SideMode::Long => short_trig = false,
        Author42SideMode::Short => buy_trig = false,
        Author42SideMode::Both => {}
    }

    let start_ts = day[0].ts_local;
    let first_idx = (day.len() >= 6).then_some(5);
    let mut long_level = None;
    let mut short_level = None;
    let mut trade_allowed = true;
    let mut was_long_today = false;
    let mut was_short_today = false;
    let mut pos: Option<OpenAuthor42Position> = None;
    let mut pending: Option<Author42Pending> = None;
    let mut day_hh = f64::NEG_INFINITY;
    let mut day_ll = f64::INFINITY;
    let mut pnl = 0.0;
    let mut trades = Vec::new();

    for (i, bar) in day.iter().copied().enumerate() {
        if let Some(pending_action) = pending.take() {
            match pending_action {
                Author42Pending::Exit(reason) => {
                    if let Some(open) = pos.take() {
                        let trade = close_author42_position(
                            open,
                            bar.ts_local,
                            bar.open,
                            reason,
                            config.roundtrip_cost_points,
                        );
                        pnl += trade.pnl_points;
                        trades.push(trade);
                    }
                }
                Author42Pending::Entry(side) => {
                    if pos.is_none() {
                        pos = Some(OpenAuthor42Position {
                            side,
                            entry_ts: bar.ts_local,
                            entry_price: bar.open,
                            bars_held: 0,
                        });
                        match side {
                            ShadowSide::Long => was_long_today = true,
                            ShadowSide::Short => was_short_today = true,
                        }
                    }
                }
            }
        }

        day_hh = day_hh.max(bar.high);
        day_ll = day_ll.min(bar.low);

        if let Some(open) = pos.as_mut() {
            open.bars_held += 1;
            if bar.ts_local.time() >= config.exit_time {
                let trade = close_author42_position(
                    *open,
                    bar.ts_local,
                    bar.close,
                    "time_exit_same_bar_close",
                    config.roundtrip_cost_points,
                );
                pnl += trade.pnl_points;
                trades.push(trade);
                pos = None;
                continue;
            }

            match open.side {
                ShadowSide::Long => {
                    if bar.close < ctx.prev_close + config.stop_k * ctx.prev_range {
                        pending = Some(Author42Pending::Exit("stop_emergency_long_next_open"));
                    } else if is_author42_hour_check(bar.ts_local, start_ts)
                        && bar.close < ctx.prev_close + config.stop_hour_k * ctx.prev_range
                    {
                        pending = Some(Author42Pending::Exit("stop_hour_long_next_open"));
                    }
                }
                ShadowSide::Short => {
                    if bar.close > ctx.prev_close - config.stop_k * ctx.prev_range {
                        pending = Some(Author42Pending::Exit("stop_emergency_short_next_open"));
                    } else if is_author42_hour_check(bar.ts_local, start_ts)
                        && bar.close > ctx.prev_close - config.stop_hour_k * ctx.prev_range
                    {
                        pending = Some(Author42Pending::Exit("stop_hour_short_next_open"));
                    }
                }
            }
        }

        if Some(i) == first_idx {
            let first_bar = day[0];
            long_level = Some(
                (ctx.prev_close + config.k * ctx.prev_range)
                    .max(bar.close)
                    .max(first_bar.high),
            );
            short_level = Some(
                (ctx.prev_close - config.k * ctx.prev_range)
                    .min(bar.close)
                    .min(first_bar.low),
            );
            if config.use_first_hour_extreme_filter
                && (bar.close - ctx.prev_close).abs() > config.first_hour_extreme_k * ctx.prev_range
            {
                trade_allowed = false;
            }
        }

        if pending.is_some() || pos.is_some() || !trade_allowed {
            continue;
        }
        let (Some(long_level), Some(short_level)) = (long_level, short_level) else {
            continue;
        };
        if bar.ts_local.time() >= config.exit_time || i + 1 >= day.len() {
            continue;
        }

        if config.allow_reentry_on_day_extreme {
            if was_long_today && bar.high >= day_hh && bar.ts_local.time() < config.exit_time {
                pending = Some(Author42Pending::Entry(ShadowSide::Long));
                continue;
            }
            if was_short_today && bar.low <= day_ll && bar.ts_local.time() < config.exit_time {
                pending = Some(Author42Pending::Entry(ShadowSide::Short));
                continue;
            }
        }

        if is_author42_hour_check(bar.ts_local, start_ts) {
            if buy_trig && bar.close > long_level {
                pending = Some(Author42Pending::Entry(ShadowSide::Long));
            } else if short_trig && bar.close < short_level {
                pending = Some(Author42Pending::Entry(ShadowSide::Short));
            }
        }
    }

    if let Some(open) = pos.take() {
        let last = day[day.len() - 1];
        let trade = close_author42_position(
            open,
            last.ts_local,
            last.close,
            "forced_last_bar_close",
            config.roundtrip_cost_points,
        );
        pnl += trade.pnl_points;
        trades.push(trade);
    }

    (
        trades.clone(),
        Author42DailyPnl {
            date,
            pnl_points: pnl,
            trades: trades.len() as u32,
            skipped: String::new(),
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OpenAuthor42Position {
    side: ShadowSide,
    entry_ts: NaiveDateTime,
    entry_price: f64,
    bars_held: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Author42Pending {
    Entry(ShadowSide),
    Exit(&'static str),
}

fn close_author42_position(
    pos: OpenAuthor42Position,
    exit_ts: NaiveDateTime,
    exit_price: f64,
    reason: &'static str,
    cost_points: f64,
) -> Author42Trade {
    let gross_points = match pos.side {
        ShadowSide::Long => exit_price - pos.entry_price,
        ShadowSide::Short => pos.entry_price - exit_price,
    };
    Author42Trade {
        side: pos.side,
        entry_ts: pos.entry_ts,
        exit_ts,
        entry_price: pos.entry_price,
        exit_price,
        gross_points,
        pnl_points: gross_points - cost_points,
        exit_reason: reason.to_string(),
        bars_held: pos.bars_held,
    }
}

fn build_author42_daily_context(bars: &[ModelBar]) -> BTreeMap<NaiveDate, Author42DailyContext> {
    #[derive(Debug, Clone, Copy)]
    struct DayStats {
        high: f64,
        low: f64,
        close: f64,
    }

    let mut by_day: BTreeMap<NaiveDate, DayStats> = BTreeMap::new();
    for bar in bars {
        by_day
            .entry(bar.ts_local.date())
            .and_modify(|stats| {
                stats.high = stats.high.max(bar.high);
                stats.low = stats.low.min(bar.low);
                stats.close = bar.close;
            })
            .or_insert(DayStats {
                high: bar.high,
                low: bar.low,
                close: bar.close,
            });
    }

    let mut contexts = BTreeMap::new();
    let mut prev2_close: Option<f64> = None;
    let mut prev: Option<DayStats> = None;
    for (date, stats) in by_day {
        if let (Some(prev), Some(prev2_close)) = (prev, prev2_close) {
            let prev_range = prev.high - prev.low;
            contexts.insert(
                date,
                Author42DailyContext {
                    prev_close: prev.close,
                    prev2_close,
                    prev_range,
                    prev_hl_ratio: prev.high / prev.low,
                    prev_ret: prev.close / prev2_close - 1.0,
                },
            );
        }
        prev2_close = prev.map(|stats: DayStats| stats.close);
        prev = Some(stats);
    }
    contexts
}

fn all_finite(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite())
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

fn is_author42_hour_check(ts: NaiveDateTime, start_ts: NaiveDateTime) -> bool {
    ts.time().minute() == 50 && ts > start_ts + chrono::Duration::minutes(50)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author41SideMode {
    Long,
    Short,
    Dual,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelBar {
    pub ts_local: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DailyAnchor {
    pub prev_close: f64,
    pub prev_low: f64,
    pub prev_range: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author41Trade {
    pub side: ShadowSide,
    pub entry_ts: NaiveDateTime,
    pub exit_ts: NaiveDateTime,
    pub entry_price: f64,
    pub exit_price: f64,
    pub gross_points: f64,
    pub net_points: f64,
    pub exit_reason: String,
    pub bars_held: u32,
    pub entry_index_for_day: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author41DailyPnl {
    pub date: NaiveDate,
    pub pnl_points: f64,
    pub trades: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author41ReplayResult {
    pub trades: Vec<Author41Trade>,
    pub daily: Vec<Author41DailyPnl>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Author41ReplayComparison {
    pub source_trades: usize,
    pub actual_trades: usize,
    pub trade_key_matches: usize,
    pub trade_exact_matches: usize,
    pub trade_point_mismatches: usize,
    pub missing_source_trades: usize,
    pub extra_actual_trades: usize,
    pub source_daily_rows: usize,
    pub actual_daily_rows: usize,
    pub daily_exact_matches: usize,
    pub daily_pnl_mismatches: usize,
    pub missing_source_daily: usize,
    pub extra_actual_daily: usize,
    pub source_total_pnl: f64,
    pub actual_total_pnl: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author42Trade {
    pub side: ShadowSide,
    pub entry_ts: NaiveDateTime,
    pub exit_ts: NaiveDateTime,
    pub entry_price: f64,
    pub exit_price: f64,
    pub gross_points: f64,
    pub pnl_points: f64,
    pub exit_reason: String,
    pub bars_held: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author42DailyPnl {
    pub date: NaiveDate,
    pub pnl_points: f64,
    pub trades: u32,
    pub skipped: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author42ReplayResult {
    pub trades: Vec<Author42Trade>,
    pub daily: Vec<Author42DailyPnl>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Author42ReplayComparison {
    pub source_trades: usize,
    pub actual_trades: usize,
    pub trade_key_matches: usize,
    pub trade_exact_matches: usize,
    pub trade_point_mismatches: usize,
    pub missing_source_trades: usize,
    pub extra_actual_trades: usize,
    pub source_daily_rows: usize,
    pub actual_daily_rows: usize,
    pub daily_exact_matches: usize,
    pub daily_pnl_mismatches: usize,
    pub missing_source_daily: usize,
    pub extra_actual_daily: usize,
    pub source_total_pnl: f64,
    pub actual_total_pnl: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComboDailyPnl {
    pub date: NaiveDate,
    pub author41_pnl: f64,
    pub author42_pnl: f64,
    pub author41_trades: u32,
    pub author42_trades: u32,
    pub pnl_points: f64,
    pub trades: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComboReplayResult {
    pub author41_trades: Vec<Author41Trade>,
    pub author42_trades: Vec<Author42Trade>,
    pub daily: Vec<ComboDailyPnl>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComboReplayComparison {
    pub source_daily_rows: usize,
    pub actual_daily_rows: usize,
    pub daily_exact_matches: usize,
    pub daily_pnl_mismatches: usize,
    pub component_pnl_mismatches: usize,
    pub trade_count_mismatches: usize,
    pub missing_source_daily: usize,
    pub extra_actual_daily: usize,
    pub source_total_pnl: f64,
    pub actual_total_pnl: f64,
    pub source_author41_total_pnl: f64,
    pub actual_author41_total_pnl: f64,
    pub source_author42_total_pnl: f64,
    pub actual_author42_total_pnl: f64,
    pub source_total_trades: f64,
    pub actual_total_trades: u32,
    pub actual_author41_trades: usize,
    pub actual_author42_trades: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct OpenAuthor41Position {
    side: ShadowSide,
    entry_ts: NaiveDateTime,
    entry_price: f64,
    prev_close: f64,
    prev_range: f64,
    bars_held: u32,
    entry_index_for_day: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author41Engine {
    config: Author41Config,
    current_date: Option<NaiveDate>,
    day_entries: u32,
    position: Option<OpenAuthor41Position>,
}

impl Author41Engine {
    pub fn new(config: Author41Config) -> Self {
        Self {
            config,
            current_date: None,
            day_entries: 0,
            position: None,
        }
    }

    pub fn on_bar(&mut self, bar: ModelBar, anchor: Option<DailyAnchor>) -> Vec<Author41Trade> {
        if self.current_date != Some(bar.ts_local.date()) {
            self.current_date = Some(bar.ts_local.date());
            self.day_entries = 0;
            self.position = None;
        }

        let mut trades = Vec::new();
        if let Some(pos) = self.position.as_mut() {
            pos.bars_held += 1;
            if let Some((exit_price, reason)) = Self::exit_signal(self.config, pos, bar) {
                let closed =
                    Self::close_position(self.config, pos, bar.ts_local, exit_price, reason);
                trades.push(closed);
                self.position = None;
            }
        }

        if self.position.is_some() {
            return trades;
        }
        if self.day_entries >= self.config.max_entries_per_day {
            return trades;
        }
        if bar.ts_local.time() > self.config.entry_end {
            return trades;
        }

        let Some(anchor) = anchor else {
            return trades;
        };
        if !anchor.prev_close.is_finite()
            || !anchor.prev_range.is_finite()
            || !anchor.prev_low.is_finite()
            || anchor.prev_range <= 0.0
            || anchor.prev_low <= 0.0
        {
            return trades;
        }
        let rel_range = anchor.prev_range / anchor.prev_low;
        if !rel_range.is_finite()
            || !(self.config.min_range < rel_range && rel_range < self.config.max_range)
        {
            return trades;
        }

        let side = self.entry_side(bar.close, anchor);
        if let Some(side) = side {
            self.day_entries += 1;
            self.position = Some(OpenAuthor41Position {
                side,
                entry_ts: bar.ts_local,
                entry_price: bar.close,
                prev_close: anchor.prev_close,
                prev_range: anchor.prev_range,
                bars_held: 0,
                entry_index_for_day: self.day_entries,
            });
        }

        trades
    }

    pub fn force_close_at_last_bar(&mut self, bar: ModelBar) -> Option<Author41Trade> {
        let pos = self.position.take()?;
        Some(Self::close_position(
            self.config,
            &pos,
            bar.ts_local,
            bar.close,
            "forced_last_bar_close",
        ))
    }

    fn entry_side(&self, close: f64, anchor: DailyAnchor) -> Option<ShadowSide> {
        let short_signal = close > anchor.prev_close
            && close < anchor.prev_close + self.config.k * anchor.prev_range;
        let long_signal = close < anchor.prev_close
            && close > anchor.prev_close - self.config.k * anchor.prev_range;

        match self.config.side_mode {
            Author41SideMode::Short if short_signal => Some(ShadowSide::Short),
            Author41SideMode::Long if long_signal => Some(ShadowSide::Long),
            Author41SideMode::Dual if short_signal => Some(ShadowSide::Short),
            Author41SideMode::Dual if long_signal => Some(ShadowSide::Long),
            _ => None,
        }
    }

    fn exit_signal(
        config: Author41Config,
        pos: &OpenAuthor41Position,
        bar: ModelBar,
    ) -> Option<(f64, &'static str)> {
        match pos.side {
            ShadowSide::Short => {
                let stop_price = pos.prev_close + config.stop_k * pos.prev_range;
                let take_price = pos.prev_close - config.k2 * pos.prev_range;
                if bar.high >= stop_price {
                    Some((stop_price, "stop"))
                } else if bar.close < take_price {
                    Some((bar.close, "take_author_close"))
                } else if bar.ts_local.time() >= config.time_exit {
                    Some((bar.close, "time_exit"))
                } else if pos.bars_held > config.breakeven_after_bars && bar.low <= pos.entry_price
                {
                    Some((pos.entry_price, "breakeven_limit"))
                } else {
                    None
                }
            }
            ShadowSide::Long => {
                let stop_price = pos.prev_close - config.stop_k * pos.prev_range;
                let take_price = pos.prev_close + config.k2 * pos.prev_range;
                if bar.low <= stop_price {
                    Some((stop_price, "stop"))
                } else if bar.close > take_price {
                    Some((bar.close, "take_author_close"))
                } else if bar.ts_local.time() >= config.time_exit {
                    Some((bar.close, "time_exit"))
                } else if pos.bars_held > config.breakeven_after_bars && bar.high >= pos.entry_price
                {
                    Some((pos.entry_price, "breakeven_limit"))
                } else {
                    None
                }
            }
        }
    }

    fn close_position(
        config: Author41Config,
        pos: &OpenAuthor41Position,
        exit_ts: NaiveDateTime,
        exit_price: f64,
        reason: &'static str,
    ) -> Author41Trade {
        let gross_points = match pos.side {
            ShadowSide::Short => pos.entry_price - exit_price,
            ShadowSide::Long => exit_price - pos.entry_price,
        };
        Author41Trade {
            side: pos.side,
            entry_ts: pos.entry_ts,
            exit_ts,
            entry_price: pos.entry_price,
            exit_price,
            gross_points,
            net_points: gross_points - config.roundtrip_cost_points,
            exit_reason: reason.to_string(),
            bars_held: pos.bars_held,
            entry_index_for_day: pos.entry_index_for_day,
        }
    }
}

pub fn replay_author41(
    bars: &[ModelBar],
    config: Author41Config,
    session_policy: RegularSessionPolicy,
) -> Author41ReplayResult {
    let mut filtered: Vec<ModelBar> = bars
        .iter()
        .copied()
        .filter(|bar| session_policy.is_model_bar(bar.ts_local))
        .collect();
    filtered.sort_by_key(|bar| bar.ts_local);

    let anchors = build_daily_anchors(&filtered);
    let mut engine = Author41Engine::new(config);
    let mut trades = Vec::new();
    let mut daily_map: BTreeMap<NaiveDate, Author41DailyPnl> = filtered
        .iter()
        .map(|bar| {
            (
                bar.ts_local.date(),
                Author41DailyPnl {
                    date: bar.ts_local.date(),
                    pnl_points: 0.0,
                    trades: 0,
                },
            )
        })
        .collect();

    for bar in filtered {
        let date = bar.ts_local.date();
        let anchor = anchors.get(&date).copied();
        for trade in engine.on_bar(bar, anchor) {
            let row = daily_map
                .entry(trade.exit_ts.date())
                .or_insert(Author41DailyPnl {
                    date: trade.exit_ts.date(),
                    pnl_points: 0.0,
                    trades: 0,
                });
            row.pnl_points += trade.net_points;
            row.trades += 1;
            trades.push(trade);
        }
    }

    Author41ReplayResult {
        trades,
        daily: daily_map.into_values().collect(),
    }
}

fn build_daily_anchors(bars: &[ModelBar]) -> BTreeMap<NaiveDate, DailyAnchor> {
    #[derive(Debug, Clone, Copy)]
    struct DayStats {
        high: f64,
        low: f64,
        close: f64,
    }

    let mut by_day: BTreeMap<NaiveDate, DayStats> = BTreeMap::new();
    for bar in bars {
        by_day
            .entry(bar.ts_local.date())
            .and_modify(|stats| {
                stats.high = stats.high.max(bar.high);
                stats.low = stats.low.min(bar.low);
                stats.close = bar.close;
            })
            .or_insert(DayStats {
                high: bar.high,
                low: bar.low,
                close: bar.close,
            });
    }

    let mut anchors = BTreeMap::new();
    let mut previous: Option<DayStats> = None;
    for (date, stats) in by_day {
        if let Some(prev) = previous {
            anchors.insert(
                date,
                DailyAnchor {
                    prev_close: prev.close,
                    prev_low: prev.low,
                    prev_range: prev.high - prev.low,
                },
            );
        }
        previous = Some(stats);
    }
    anchors
}

pub fn load_model_bars<R: Read>(reader: R) -> Result<Vec<ModelBar>> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv.headers().context("read model bar headers")?.clone();
    let mut bars = Vec::new();
    for record in csv.records() {
        let record = record.context("read model bar row")?;
        bars.push(ModelBar {
            ts_local: parse_ts(first_required_field(
                &headers,
                &record,
                &["ts_local", "datetime", "timestamp", "ts", "dt"],
            )?)?,
            open: parse_f64(required_field(&headers, &record, "open")?, "open")?,
            high: parse_f64(required_field(&headers, &record, "high")?, "high")?,
            low: parse_f64(required_field(&headers, &record, "low")?, "low")?,
            close: parse_f64(required_field(&headers, &record, "close")?, "close")?,
            volume: first_f64(&headers, &record, &["volume", "vol"]).unwrap_or(0.0),
        });
    }
    bars.sort_by_key(|bar| bar.ts_local);
    Ok(bars)
}

pub fn compare_author41_replay(
    actual: &Author41ReplayResult,
    source_trades: &[SourceTrade],
    source_daily: &[SourceDaily],
    tolerance: f64,
) -> Author41ReplayComparison {
    let mut actual_by_key: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for trade in &actual.trades {
        actual_by_key
            .entry(trade_key(trade.side, trade.entry_ts, trade.exit_ts))
            .or_default()
            .push(trade.net_points);
    }

    let mut trade_key_matches = 0;
    let mut trade_exact_matches = 0;
    let mut trade_point_mismatches = 0;
    let mut missing_source_trades = 0;

    for source in source_trades {
        let key = trade_key(source.side, source.entry_ts, source.exit_ts);
        let Some(points) = actual_by_key.get_mut(&key) else {
            missing_source_trades += 1;
            continue;
        };
        trade_key_matches += 1;
        if let Some(idx) = points
            .iter()
            .position(|actual_points| close_enough(*actual_points, source.points, tolerance))
        {
            trade_exact_matches += 1;
            points.remove(idx);
        } else {
            trade_point_mismatches += 1;
            points.pop();
        }
    }

    let extra_actual_trades = actual_by_key.values().map(Vec::len).sum();

    let source_daily_by_date: BTreeMap<NaiveDate, f64> = source_daily
        .iter()
        .map(|row| (row.date, row.pnl_points))
        .collect();
    let actual_daily_by_date: BTreeMap<NaiveDate, f64> = actual
        .daily
        .iter()
        .map(|row| (row.date, row.pnl_points))
        .collect();

    let mut daily_exact_matches = 0;
    let mut daily_pnl_mismatches = 0;
    let mut missing_source_daily = 0;
    for (date, source_pnl) in &source_daily_by_date {
        match actual_daily_by_date.get(date) {
            Some(actual_pnl) if close_enough(*actual_pnl, *source_pnl, tolerance) => {
                daily_exact_matches += 1;
            }
            Some(_) => daily_pnl_mismatches += 1,
            None => missing_source_daily += 1,
        }
    }
    let extra_actual_daily = actual_daily_by_date
        .keys()
        .filter(|date| !source_daily_by_date.contains_key(date))
        .count();

    Author41ReplayComparison {
        source_trades: source_trades.len(),
        actual_trades: actual.trades.len(),
        trade_key_matches,
        trade_exact_matches,
        trade_point_mismatches,
        missing_source_trades,
        extra_actual_trades,
        source_daily_rows: source_daily.len(),
        actual_daily_rows: actual.daily.len(),
        daily_exact_matches,
        daily_pnl_mismatches,
        missing_source_daily,
        extra_actual_daily,
        source_total_pnl: source_daily.iter().map(|row| row.pnl_points).sum(),
        actual_total_pnl: actual.daily.iter().map(|row| row.pnl_points).sum(),
    }
}

pub fn compare_author42_replay(
    actual: &Author42ReplayResult,
    source_trades: &[SourceTrade],
    source_daily: &[SourceDaily],
    tolerance: f64,
) -> Author42ReplayComparison {
    let mut actual_by_key: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for trade in &actual.trades {
        actual_by_key
            .entry(trade_key(trade.side, trade.entry_ts, trade.exit_ts))
            .or_default()
            .push(trade.pnl_points);
    }

    let mut trade_key_matches = 0;
    let mut trade_exact_matches = 0;
    let mut trade_point_mismatches = 0;
    let mut missing_source_trades = 0;

    for source in source_trades {
        let key = trade_key(source.side, source.entry_ts, source.exit_ts);
        let Some(points) = actual_by_key.get_mut(&key) else {
            missing_source_trades += 1;
            continue;
        };
        trade_key_matches += 1;
        if let Some(idx) = points
            .iter()
            .position(|actual_points| close_enough(*actual_points, source.points, tolerance))
        {
            trade_exact_matches += 1;
            points.remove(idx);
        } else {
            trade_point_mismatches += 1;
            points.pop();
        }
    }

    let extra_actual_trades = actual_by_key.values().map(Vec::len).sum();

    let source_daily_by_date: BTreeMap<NaiveDate, f64> = source_daily
        .iter()
        .map(|row| (row.date, row.pnl_points))
        .collect();
    let actual_daily_by_date: BTreeMap<NaiveDate, f64> = actual
        .daily
        .iter()
        .map(|row| (row.date, row.pnl_points))
        .collect();

    let mut daily_exact_matches = 0;
    let mut daily_pnl_mismatches = 0;
    let mut missing_source_daily = 0;
    for (date, source_pnl) in &source_daily_by_date {
        match actual_daily_by_date.get(date) {
            Some(actual_pnl) if close_enough(*actual_pnl, *source_pnl, tolerance) => {
                daily_exact_matches += 1;
            }
            Some(_) => daily_pnl_mismatches += 1,
            None => missing_source_daily += 1,
        }
    }
    let extra_actual_daily = actual_daily_by_date
        .keys()
        .filter(|date| !source_daily_by_date.contains_key(date))
        .count();

    Author42ReplayComparison {
        source_trades: source_trades.len(),
        actual_trades: actual.trades.len(),
        trade_key_matches,
        trade_exact_matches,
        trade_point_mismatches,
        missing_source_trades,
        extra_actual_trades,
        source_daily_rows: source_daily.len(),
        actual_daily_rows: actual.daily.len(),
        daily_exact_matches,
        daily_pnl_mismatches,
        missing_source_daily,
        extra_actual_daily,
        source_total_pnl: source_daily.iter().map(|row| row.pnl_points).sum(),
        actual_total_pnl: actual.daily.iter().map(|row| row.pnl_points).sum(),
    }
}

pub fn compare_combo_replay(
    actual: &ComboReplayResult,
    source_daily: &[SourceDaily],
    tolerance: f64,
) -> ComboReplayComparison {
    let source_daily_by_date: BTreeMap<NaiveDate, &SourceDaily> =
        source_daily.iter().map(|row| (row.date, row)).collect();
    let actual_daily_by_date: BTreeMap<NaiveDate, &ComboDailyPnl> =
        actual.daily.iter().map(|row| (row.date, row)).collect();

    let mut daily_exact_matches = 0;
    let mut daily_pnl_mismatches = 0;
    let mut component_pnl_mismatches = 0;
    let mut trade_count_mismatches = 0;
    let mut missing_source_daily = 0;

    for (date, source) in &source_daily_by_date {
        let Some(actual_row) = actual_daily_by_date.get(date) else {
            missing_source_daily += 1;
            continue;
        };

        let pnl_ok = close_enough(actual_row.pnl_points, source.pnl_points, tolerance);
        let author41_ok = source
            .author41_pnl
            .map(|pnl| close_enough(actual_row.author41_pnl, pnl, tolerance))
            .unwrap_or(true);
        let author42_ok = source
            .author42_pnl
            .map(|pnl| close_enough(actual_row.author42_pnl, pnl, tolerance))
            .unwrap_or(true);
        let trades_ok = source
            .trades
            .map(|trades| close_enough(actual_row.trades as f64, trades, tolerance))
            .unwrap_or(true);

        if pnl_ok && author41_ok && author42_ok && trades_ok {
            daily_exact_matches += 1;
        } else {
            if !pnl_ok {
                daily_pnl_mismatches += 1;
            }
            if !author41_ok || !author42_ok {
                component_pnl_mismatches += 1;
            }
            if !trades_ok {
                trade_count_mismatches += 1;
            }
        }
    }

    let extra_actual_daily = actual_daily_by_date
        .keys()
        .filter(|date| !source_daily_by_date.contains_key(date))
        .count();

    ComboReplayComparison {
        source_daily_rows: source_daily.len(),
        actual_daily_rows: actual.daily.len(),
        daily_exact_matches,
        daily_pnl_mismatches,
        component_pnl_mismatches,
        trade_count_mismatches,
        missing_source_daily,
        extra_actual_daily,
        source_total_pnl: source_daily.iter().map(|row| row.pnl_points).sum(),
        actual_total_pnl: actual.daily.iter().map(|row| row.pnl_points).sum(),
        source_author41_total_pnl: source_daily.iter().filter_map(|row| row.author41_pnl).sum(),
        actual_author41_total_pnl: actual.daily.iter().map(|row| row.author41_pnl).sum(),
        source_author42_total_pnl: source_daily.iter().filter_map(|row| row.author42_pnl).sum(),
        actual_author42_total_pnl: actual.daily.iter().map(|row| row.author42_pnl).sum(),
        source_total_trades: source_daily.iter().filter_map(|row| row.trades).sum(),
        actual_total_trades: actual.daily.iter().map(|row| row.trades).sum(),
        actual_author41_trades: actual.author41_trades.len(),
        actual_author42_trades: actual.author42_trades.len(),
    }
}

pub fn load_source_trades<R: Read>(reader: R, fixed_model_id: &str) -> Result<Vec<SourceTrade>> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv
        .headers()
        .context("read trade artifact headers")?
        .clone();
    let mut rows = Vec::new();
    for record in csv.records() {
        let record = record.context("read trade artifact row")?;
        if field(&headers, &record, "fixed_model_id") != Some(fixed_model_id) {
            continue;
        }
        rows.push(SourceTrade {
            fixed_model_id: fixed_model_id.to_string(),
            side: parse_side(required_field(&headers, &record, "side")?)?,
            entry_ts: parse_ts(required_field(&headers, &record, "entry_ts")?)?,
            exit_ts: parse_ts(required_field(&headers, &record, "exit_ts")?)?,
            entry_price: parse_f64(
                required_field(&headers, &record, "entry_price")?,
                "entry_price",
            )?,
            exit_price: parse_f64(
                required_field(&headers, &record, "exit_price")?,
                "exit_price",
            )?,
            points: first_f64(
                &headers,
                &record,
                &["net_points", "points_pnl", "pnl_points"],
            )
            .ok_or_else(|| anyhow!("missing trade points column for {fixed_model_id}"))?,
            exit_reason: required_field(&headers, &record, "exit_reason")?.to_string(),
        });
    }
    Ok(rows)
}

pub fn load_source_daily<R: Read>(reader: R, fixed_model_id: &str) -> Result<Vec<SourceDaily>> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv
        .headers()
        .context("read daily artifact headers")?
        .clone();
    let mut rows = Vec::new();
    for record in csv.records() {
        let record = record.context("read daily artifact row")?;
        if field(&headers, &record, "fixed_model_id") != Some(fixed_model_id) {
            continue;
        }
        rows.push(SourceDaily {
            fixed_model_id: fixed_model_id.to_string(),
            date: parse_date(required_field(&headers, &record, "date")?)?,
            pnl_points: first_f64(&headers, &record, &["pnl", "pnl_points"])
                .ok_or_else(|| anyhow!("missing daily pnl column for {fixed_model_id}"))?,
            author41_pnl: first_f64(&headers, &record, &["author41_pnl"]),
            author42_pnl: first_f64(&headers, &record, &["author42_pnl"]),
            trades: first_f64(&headers, &record, &["trades"]),
            skipped: field(&headers, &record, "skipped").map(str::to_string),
        });
    }
    Ok(rows)
}

fn trade_key(side: ShadowSide, entry_ts: NaiveDateTime, exit_ts: NaiveDateTime) -> String {
    let side = match side {
        ShadowSide::Long => "long",
        ShadowSide::Short => "short",
    };
    format!("{side}|{entry_ts}|{exit_ts}")
}

fn close_enough(left: f64, right: f64, tolerance: f64) -> bool {
    (left - right).abs() <= tolerance
}

fn first_required_field<'a>(
    headers: &csv::StringRecord,
    record: &'a csv::StringRecord,
    names: &[&str],
) -> Result<&'a str> {
    names
        .iter()
        .find_map(|name| field(headers, record, name))
        .ok_or_else(|| anyhow!("missing required field; tried {}", names.join("|")))
}

fn required_field<'a>(
    headers: &csv::StringRecord,
    record: &'a csv::StringRecord,
    name: &str,
) -> Result<&'a str> {
    field(headers, record, name).ok_or_else(|| anyhow!("missing required field {name}"))
}

fn field<'a>(
    headers: &csv::StringRecord,
    record: &'a csv::StringRecord,
    name: &str,
) -> Option<&'a str> {
    let idx = headers.iter().position(|header| header == name)?;
    let value = record.get(idx)?.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("nan") {
        None
    } else {
        Some(value)
    }
}

fn first_f64(
    headers: &csv::StringRecord,
    record: &csv::StringRecord,
    names: &[&str],
) -> Option<f64> {
    names
        .iter()
        .filter_map(|name| field(headers, record, name).and_then(|value| value.parse().ok()))
        .next()
}

fn parse_f64(value: &str, field_name: &str) -> Result<f64> {
    value
        .parse::<f64>()
        .with_context(|| format!("parse {field_name}={value:?}"))
}

fn parse_ts(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .with_context(|| format!("parse timestamp {value:?}"))
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").with_context(|| format!("parse date {value:?}"))
}

fn parse_side(value: &str) -> Result<ShadowSide> {
    match value {
        "long" => Ok(ShadowSide::Long),
        "short" => Ok(ShadowSide::Short),
        other => Err(anyhow!("unsupported side {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn dt(date: (i32, u32, u32), time: (u32, u32, u32)) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(date.0, date.1, date.2)
            .unwrap_or(NaiveDate::MIN)
            .and_hms_opt(time.0, time.1, time.2)
            .unwrap_or(NaiveDateTime::MIN)
    }

    #[test]
    fn frozen_profiles_match_handoff_ids() {
        let ri = ModelProfile::ri_shadow_10m();
        assert_eq!(ri.instrument.as_str(), "RI");
        assert_eq!(ri.profile_id.as_str(), "ri_author41_42_primary_combo_cost2");
        assert_eq!(ri.author41_variant, "dual_no_overlap_plateau");
        assert_eq!(ri.author42_variant, "grid_k0.42_both");
        assert_eq!(ri.author42_k, 0.42);
        assert_eq!(ri.author42_cost_points, 2.0);
        assert!(ri.no_overlap_enforced);

        let imoexf = ModelProfile::imoexf_passive_shadow_10m();
        assert_eq!(imoexf.instrument.as_str(), "IMOEXF");
        assert_eq!(
            imoexf.profile_id.as_str(),
            "imoexf_author41_42_primary_combo_cost2"
        );
        assert_eq!(imoexf.author41_variant, "author41_boundary_short");
        assert_eq!(imoexf.author42_variant, "grid_k0.44_both");
        assert_eq!(imoexf.author42_k, 0.44);
    }

    #[test]
    fn feed_guard_accepts_only_regular_weekday_model_bars() {
        let guard = RegularSessionPolicy::moex_10m();

        assert!(guard.is_model_bar(dt((2026, 4, 28), (9, 0, 0))));
        assert!(guard.is_model_bar(dt((2026, 4, 28), (23, 49, 0))));
        assert!(!guard.is_model_bar(dt((2026, 4, 28), (8, 59, 0))));
        assert!(!guard.is_model_bar(dt((2026, 4, 28), (23, 50, 0))));
        assert!(!guard.is_model_bar(dt((2026, 5, 2), (10, 0, 0))));
    }

    #[test]
    fn shadow_modes_cannot_emit_orders() {
        assert!(!ShadowRuntimeMode::ReplayOnly.can_emit_orders());
        assert!(!ShadowRuntimeMode::ShadowOnly.can_emit_orders());
    }

    #[test]
    fn skipped_journal_record_keeps_component_context() {
        let profile = ModelProfile::ri_shadow_10m();
        let record = ShadowJournalRecord::skipped(
            &profile,
            Component::Author42Bo,
            dt((2026, 4, 28), (12, 50, 0)),
            "friday_filter",
        );

        assert_eq!(record.instrument, Instrument::Ri);
        assert_eq!(record.profile_id, ProfileId::RiAuthor41_42PrimaryComboCost2);
        assert_eq!(record.model_variant_id, "grid_k0.42_both");
        assert_eq!(record.skip_reason.as_deref(), Some("friday_filter"));
        assert_eq!(record.overlap_decision, OverlapDecision::NotApplicable);

        let json = serde_json::to_string(&record).expect("serialize shadow record");
        assert!(json.contains("author42_bo"));
        assert!(!json.contains("request_id"));
    }

    #[test]
    fn ri_author41_short_entry_and_stop_match_source_contract() {
        let mut engine = Author41Engine::new(Author41Config::ri_dual_no_overlap_plateau());
        let anchor = DailyAnchor {
            prev_close: 109_800.0,
            prev_low: 100_000.0,
            prev_range: 3_525.862_068_965_517,
        };

        let trades = engine.on_bar(
            ModelBar {
                ts_local: dt((2019, 1, 4), (10, 0, 0)),
                open: 109_900.0,
                high: 109_950.0,
                low: 109_800.0,
                close: 109_920.0,
                volume: 1.0,
            },
            Some(anchor),
        );
        assert!(trades.is_empty());

        let trades = engine.on_bar(
            ModelBar {
                ts_local: dt((2019, 1, 4), (18, 40, 0)),
                open: 111_000.0,
                high: 111_900.0,
                low: 110_900.0,
                close: 111_800.0,
                volume: 1.0,
            },
            Some(anchor),
        );

        assert_eq!(trades.len(), 1);
        let trade = &trades[0];
        assert_eq!(trade.side, ShadowSide::Short);
        assert_eq!(trade.entry_ts, dt((2019, 1, 4), (10, 0, 0)));
        assert_eq!(trade.exit_ts, dt((2019, 1, 4), (18, 40, 0)));
        assert!((trade.exit_price - 111_845.0).abs() < 1e-9);
        assert_eq!(trade.exit_reason, "stop");
        assert!((trade.net_points + 1_927.0).abs() < 1e-9);
    }

    #[test]
    fn ri_source_plateau_side_configs_match_fixed_artifact_components() {
        let short = Author41Config::ri_plateau_short_source();
        assert_eq!(short.side_mode, Author41SideMode::Short);
        assert_eq!(short.k, 0.20);
        assert_eq!(short.k2, 0.020);
        assert_eq!(short.stop_k, 0.75);
        assert_eq!(short.min_range, 0.005);
        assert_eq!(short.max_range, 0.100);

        let long = Author41Config::ri_plateau_long_source();
        assert_eq!(long.side_mode, Author41SideMode::Long);
        assert_eq!(long.k, 0.11);
        assert_eq!(long.k2, 0.005);
        assert_eq!(long.stop_k, 1.00);
        assert_eq!(long.min_range, 0.005);
        assert_eq!(long.max_range, 0.100);
    }

    #[test]
    fn ri_author41_dual_prefers_short_when_both_would_not_apply() {
        let mut engine = Author41Engine::new(Author41Config::ri_dual_no_overlap_plateau());
        let anchor = DailyAnchor {
            prev_close: 100.0,
            prev_low: 90.0,
            prev_range: 3.0,
        };

        engine.on_bar(
            ModelBar {
                ts_local: dt((2026, 4, 28), (9, 0, 0)),
                open: 100.0,
                high: 100.2,
                low: 99.9,
                close: 100.1,
                volume: 1.0,
            },
            Some(anchor),
        );
        let trade = engine
            .force_close_at_last_bar(ModelBar {
                ts_local: dt((2026, 4, 28), (23, 40, 0)),
                open: 99.0,
                high: 99.0,
                low: 99.0,
                close: 99.0,
                volume: 1.0,
            })
            .expect("position should be open");

        assert_eq!(trade.side, ShadowSide::Short);
        assert_eq!(trade.exit_reason, "forced_last_bar_close");
    }

    #[test]
    fn author41_blocks_out_of_contract_range_and_after_entry_window() {
        let mut engine = Author41Engine::new(Author41Config::ri_dual_no_overlap_plateau());
        let too_small_range = DailyAnchor {
            prev_close: 100.0,
            prev_low: 100.0,
            prev_range: 1.0,
        };
        let valid_anchor = DailyAnchor {
            prev_close: 100.0,
            prev_low: 90.0,
            prev_range: 3.0,
        };

        engine.on_bar(
            ModelBar {
                ts_local: dt((2026, 4, 28), (10, 0, 0)),
                open: 100.0,
                high: 100.1,
                low: 100.0,
                close: 100.05,
                volume: 1.0,
            },
            Some(too_small_range),
        );
        assert!(engine.position.is_none());

        engine.on_bar(
            ModelBar {
                ts_local: dt((2026, 4, 28), (12, 10, 0)),
                open: 100.0,
                high: 100.1,
                low: 100.0,
                close: 100.05,
                volume: 1.0,
            },
            Some(valid_anchor),
        );
        assert!(engine.position.is_none());
    }

    #[test]
    fn replay_author41_builds_previous_regular_day_anchor_and_daily_pnl() {
        let bars = vec![
            ModelBar {
                ts_local: dt((2026, 4, 27), (9, 0, 0)),
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 4, 27), (23, 40, 0)),
                open: 100.0,
                high: 102.0,
                low: 98.0,
                close: 100.0,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 4, 28), (8, 50, 0)),
                open: 200.0,
                high: 250.0,
                low: 50.0,
                close: 200.0,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 4, 28), (10, 0, 0)),
                open: 100.0,
                high: 100.2,
                low: 99.9,
                close: 100.1,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 4, 28), (10, 10, 0)),
                open: 99.5,
                high: 99.5,
                low: 99.0,
                close: 99.0,
                volume: 1.0,
            },
        ];

        let result = replay_author41(
            &bars,
            Author41Config::ri_dual_no_overlap_plateau(),
            RegularSessionPolicy::moex_10m(),
        );

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].entry_ts, dt((2026, 4, 28), (10, 0, 0)));
        assert_eq!(result.trades[0].exit_ts, dt((2026, 4, 28), (10, 10, 0)));
        assert_eq!(result.trades[0].exit_reason, "take_author_close");
        assert!((result.trades[0].net_points + 0.9).abs() < 1e-9);

        assert_eq!(result.daily.len(), 2);
        assert_eq!(
            result.daily[0].date,
            NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()
        );
        assert_eq!(result.daily[0].pnl_points, 0.0);
        assert_eq!(
            result.daily[1].date,
            NaiveDate::from_ymd_opt(2026, 4, 28).unwrap()
        );
        assert!((result.daily[1].pnl_points + 0.9).abs() < 1e-9);
        assert_eq!(result.daily[1].trades, 1);
    }

    #[test]
    fn replay_author41_uses_previous_regular_day_not_weekend_gap() {
        let bars = vec![
            ModelBar {
                ts_local: dt((2026, 5, 1), (23, 40, 0)),
                open: 100.0,
                high: 102.0,
                low: 98.0,
                close: 100.0,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 5, 2), (10, 0, 0)),
                open: 500.0,
                high: 600.0,
                low: 400.0,
                close: 500.0,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 5, 4), (10, 0, 0)),
                open: 100.0,
                high: 100.2,
                low: 99.9,
                close: 100.1,
                volume: 1.0,
            },
            ModelBar {
                ts_local: dt((2026, 5, 4), (10, 10, 0)),
                open: 99.5,
                high: 99.5,
                low: 99.0,
                close: 99.0,
                volume: 1.0,
            },
        ];

        let result = replay_author41(
            &bars,
            Author41Config::ri_dual_no_overlap_plateau(),
            RegularSessionPolicy::moex_10m(),
        );

        assert_eq!(result.daily.len(), 2);
        assert_eq!(result.trades.len(), 1);
        assert_eq!(
            result.trades[0].entry_ts.date(),
            NaiveDate::from_ymd_opt(2026, 5, 4).unwrap()
        );
    }

    #[test]
    fn loads_prepared_model_bars_with_common_timestamp_columns() {
        let csv = "\
datetime,open,high,low,close,vol
2026-04-28 10:10:00,99.5,99.5,99.0,99.0,11
2026-04-28 10:00:00,100.0,100.2,99.9,100.1,10
";

        let bars = load_model_bars(csv.as_bytes()).expect("load prepared bars");
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].ts_local, dt((2026, 4, 28), (10, 0, 0)));
        assert_eq!(bars[0].volume, 10.0);
        assert_eq!(bars[1].ts_local, dt((2026, 4, 28), (10, 10, 0)));
    }

    #[test]
    fn loaded_model_bars_can_feed_author41_replay() {
        let csv = "\
ts_local,open,high,low,close,volume
2026-04-27 09:00:00,100.0,101.0,99.0,100.0,1
2026-04-27 23:40:00,100.0,102.0,98.0,100.0,1
2026-04-28 08:50:00,200.0,250.0,50.0,200.0,1
2026-04-28 10:00:00,100.0,100.2,99.9,100.1,1
2026-04-28 10:10:00,99.5,99.5,99.0,99.0,1
";

        let bars = load_model_bars(csv.as_bytes()).expect("load prepared bars");
        let result = replay_author41(
            &bars,
            Author41Config::ri_dual_no_overlap_plateau(),
            RegularSessionPolicy::moex_10m(),
        );

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].entry_ts, dt((2026, 4, 28), (10, 0, 0)));
        assert_eq!(result.daily.len(), 2);
    }

    #[test]
    fn compares_author41_replay_against_source_artifacts() {
        let actual = Author41ReplayResult {
            trades: vec![Author41Trade {
                side: ShadowSide::Short,
                entry_ts: dt((2026, 4, 28), (10, 0, 0)),
                exit_ts: dt((2026, 4, 28), (10, 10, 0)),
                entry_price: 100.1,
                exit_price: 99.0,
                gross_points: 1.1,
                net_points: -0.9,
                exit_reason: "take_author_close".to_string(),
                bars_held: 1,
                entry_index_for_day: 1,
            }],
            daily: vec![
                Author41DailyPnl {
                    date: NaiveDate::from_ymd_opt(2026, 4, 27).unwrap(),
                    pnl_points: 0.0,
                    trades: 0,
                },
                Author41DailyPnl {
                    date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                    pnl_points: -0.9,
                    trades: 1,
                },
            ],
        };
        let source_trades = vec![SourceTrade {
            fixed_model_id: "ri_author41_mr_primary".to_string(),
            side: ShadowSide::Short,
            entry_ts: dt((2026, 4, 28), (10, 0, 0)),
            exit_ts: dt((2026, 4, 28), (10, 10, 0)),
            entry_price: 100.1,
            exit_price: 99.0,
            points: -0.9,
            exit_reason: "take_author_close".to_string(),
        }];
        let source_daily = vec![
            SourceDaily {
                fixed_model_id: "ri_author41_mr_primary".to_string(),
                date: NaiveDate::from_ymd_opt(2026, 4, 27).unwrap(),
                pnl_points: 0.0,
                author41_pnl: None,
                author42_pnl: None,
                trades: Some(0.0),
                skipped: None,
            },
            SourceDaily {
                fixed_model_id: "ri_author41_mr_primary".to_string(),
                date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                pnl_points: -0.9,
                author41_pnl: None,
                author42_pnl: None,
                trades: Some(1.0),
                skipped: None,
            },
        ];

        let comparison = compare_author41_replay(&actual, &source_trades, &source_daily, 1e-9);
        assert_eq!(comparison.source_trades, 1);
        assert_eq!(comparison.actual_trades, 1);
        assert_eq!(comparison.trade_key_matches, 1);
        assert_eq!(comparison.trade_exact_matches, 1);
        assert_eq!(comparison.trade_point_mismatches, 0);
        assert_eq!(comparison.missing_source_trades, 0);
        assert_eq!(comparison.extra_actual_trades, 0);
        assert_eq!(comparison.daily_exact_matches, 2);
        assert_eq!(comparison.daily_pnl_mismatches, 0);
        assert_eq!(comparison.missing_source_daily, 0);
        assert_eq!(comparison.extra_actual_daily, 0);
        assert_eq!(comparison.source_total_pnl, -0.9);
        assert_eq!(comparison.actual_total_pnl, -0.9);
    }

    #[test]
    fn comparison_separates_missing_extra_and_point_drift() {
        let actual = Author41ReplayResult {
            trades: vec![
                Author41Trade {
                    side: ShadowSide::Short,
                    entry_ts: dt((2026, 4, 28), (10, 0, 0)),
                    exit_ts: dt((2026, 4, 28), (10, 10, 0)),
                    entry_price: 100.1,
                    exit_price: 99.0,
                    gross_points: 1.1,
                    net_points: -1.0,
                    exit_reason: "take_author_close".to_string(),
                    bars_held: 1,
                    entry_index_for_day: 1,
                },
                Author41Trade {
                    side: ShadowSide::Long,
                    entry_ts: dt((2026, 4, 29), (10, 0, 0)),
                    exit_ts: dt((2026, 4, 29), (10, 10, 0)),
                    entry_price: 99.0,
                    exit_price: 100.0,
                    gross_points: 1.0,
                    net_points: -1.0,
                    exit_reason: "extra".to_string(),
                    bars_held: 1,
                    entry_index_for_day: 1,
                },
            ],
            daily: vec![Author41DailyPnl {
                date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                pnl_points: -1.0,
                trades: 1,
            }],
        };
        let source_trades = vec![
            SourceTrade {
                fixed_model_id: "ri_author41_mr_primary".to_string(),
                side: ShadowSide::Short,
                entry_ts: dt((2026, 4, 28), (10, 0, 0)),
                exit_ts: dt((2026, 4, 28), (10, 10, 0)),
                entry_price: 100.1,
                exit_price: 99.0,
                points: -0.9,
                exit_reason: "take_author_close".to_string(),
            },
            SourceTrade {
                fixed_model_id: "ri_author41_mr_primary".to_string(),
                side: ShadowSide::Short,
                entry_ts: dt((2026, 4, 30), (10, 0, 0)),
                exit_ts: dt((2026, 4, 30), (10, 10, 0)),
                entry_price: 100.0,
                exit_price: 101.0,
                points: -3.0,
                exit_reason: "missing".to_string(),
            },
        ];
        let source_daily = vec![
            SourceDaily {
                fixed_model_id: "ri_author41_mr_primary".to_string(),
                date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                pnl_points: -0.9,
                author41_pnl: None,
                author42_pnl: None,
                trades: None,
                skipped: None,
            },
            SourceDaily {
                fixed_model_id: "ri_author41_mr_primary".to_string(),
                date: NaiveDate::from_ymd_opt(2026, 4, 30).unwrap(),
                pnl_points: -3.0,
                author41_pnl: None,
                author42_pnl: None,
                trades: None,
                skipped: None,
            },
        ];

        let comparison = compare_author41_replay(&actual, &source_trades, &source_daily, 1e-9);
        assert_eq!(comparison.trade_key_matches, 1);
        assert_eq!(comparison.trade_exact_matches, 0);
        assert_eq!(comparison.trade_point_mismatches, 1);
        assert_eq!(comparison.missing_source_trades, 1);
        assert_eq!(comparison.extra_actual_trades, 1);
        assert_eq!(comparison.daily_exact_matches, 0);
        assert_eq!(comparison.daily_pnl_mismatches, 1);
        assert_eq!(comparison.missing_source_daily, 1);
        assert_eq!(comparison.extra_actual_daily, 0);
    }

    #[test]
    fn loads_source_trades_across_component_column_variants() {
        let csv = "\
fixed_model_id,side,entry_ts,exit_ts,entry_price,exit_price,net_points,points_pnl,pnl_points,exit_reason\n\
imoexf_author41_mr_primary,short,2023-11-15 09:00:00,2023-11-15 09:10:00,3205.0,3195.0,9.9,,,take_author_close\n\
ri_author41_mr_primary,short,2019-01-04 10:00:00,2019-01-04 18:40:00,109920.0,111845.0,, -1927.0,,stop\n\
ri_author42_bo_primary,long,2019-01-09 18:00:00,2019-01-09 23:00:00,113920.0,114470.0,,,550.0,time_exit_same_bar_close\n";

        let imoexf = load_source_trades(csv.as_bytes(), "imoexf_author41_mr_primary")
            .expect("load imoexf trades");
        assert_eq!(imoexf.len(), 1);
        assert_eq!(imoexf[0].side, ShadowSide::Short);
        assert_eq!(imoexf[0].points, 9.9);

        let ri = load_source_trades(csv.as_bytes(), "ri_author41_mr_primary")
            .expect("load ri mr trades");
        assert_eq!(ri.len(), 1);
        assert_eq!(ri[0].points, -1927.0);

        let bo = load_source_trades(csv.as_bytes(), "ri_author42_bo_primary")
            .expect("load ri bo trades");
        assert_eq!(bo.len(), 1);
        assert_eq!(bo[0].side, ShadowSide::Long);
        assert_eq!(bo[0].points, 550.0);
    }

    #[test]
    fn loads_combo_daily_from_pnl_column() {
        let csv = "\
fixed_model_id,date,pnl_points,trades,skipped,author41_pnl,author42_pnl,pnl\n\
ri_author41_42_primary_combo_cost2,2019-01-03,,0,,0.0,0.0,0.0\n\
ri_author41_42_primary_combo_cost2,2019-01-04,,1,,-1927.0,0.0,-1927.0\n\
ri_author42_bo_primary,2019-01-04,0.0,0,friday,,, \n";

        let combo = load_source_daily(csv.as_bytes(), "ri_author41_42_primary_combo_cost2")
            .expect("load combo daily");
        assert_eq!(combo.len(), 2);
        assert_eq!(combo[1].date, NaiveDate::from_ymd_opt(2019, 1, 4).unwrap());
        assert_eq!(combo[1].pnl_points, -1927.0);
        assert_eq!(combo[1].author41_pnl, Some(-1927.0));
        assert_eq!(combo[1].author42_pnl, Some(0.0));
        assert_eq!(combo[1].trades, Some(1.0));

        let bo =
            load_source_daily(csv.as_bytes(), "ri_author42_bo_primary").expect("load bo daily");
        assert_eq!(bo.len(), 1);
        assert_eq!(bo[0].pnl_points, 0.0);
        assert_eq!(bo[0].skipped.as_deref(), Some("friday"));
    }

    #[test]
    fn combo_drops_bo_interval_overlap_and_applies_author42_cost2() {
        let profile = ModelProfile::ri_shadow_10m();
        let author41 = Author41ReplayResult {
            trades: vec![Author41Trade {
                side: ShadowSide::Short,
                entry_ts: dt((2026, 4, 28), (10, 0, 0)),
                exit_ts: dt((2026, 4, 28), (11, 0, 0)),
                entry_price: 100.0,
                exit_price: 95.0,
                gross_points: 5.0,
                net_points: 3.0,
                exit_reason: "take_author_close".to_string(),
                bars_held: 6,
                entry_index_for_day: 1,
            }],
            daily: vec![Author41DailyPnl {
                date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                pnl_points: 3.0,
                trades: 1,
            }],
        };
        let author42 = Author42ReplayResult {
            trades: vec![
                Author42Trade {
                    side: ShadowSide::Long,
                    entry_ts: dt((2026, 4, 28), (10, 30, 0)),
                    exit_ts: dt((2026, 4, 28), (12, 0, 0)),
                    entry_price: 100.0,
                    exit_price: 110.0,
                    gross_points: 10.0,
                    pnl_points: 10.0,
                    exit_reason: "time_exit_same_bar_close".to_string(),
                    bars_held: 9,
                },
                Author42Trade {
                    side: ShadowSide::Short,
                    entry_ts: dt((2026, 4, 28), (12, 10, 0)),
                    exit_ts: dt((2026, 4, 28), (13, 0, 0)),
                    entry_price: 120.0,
                    exit_price: 115.0,
                    gross_points: 5.0,
                    pnl_points: 5.0,
                    exit_reason: "time_exit_same_bar_close".to_string(),
                    bars_held: 5,
                },
            ],
            daily: vec![Author42DailyPnl {
                date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                pnl_points: 15.0,
                trades: 2,
                skipped: String::new(),
            }],
        };

        let combo = replay_combo_from_components(&profile, &author41, &author42);

        assert_eq!(combo.author42_trades.len(), 1);
        assert_eq!(
            combo.author42_trades[0].entry_ts,
            dt((2026, 4, 28), (12, 10, 0))
        );
        assert_eq!(combo.daily.len(), 1);
        assert_eq!(combo.daily[0].author41_pnl, 3.0);
        assert_eq!(combo.daily[0].author42_pnl, 3.0);
        assert_eq!(combo.daily[0].pnl_points, 6.0);
        assert_eq!(combo.daily[0].trades, 2);
    }

    #[test]
    fn compares_combo_components_and_total_daily_pnl() {
        let actual = ComboReplayResult {
            author41_trades: Vec::new(),
            author42_trades: Vec::new(),
            daily: vec![ComboDailyPnl {
                date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
                author41_pnl: 3.0,
                author42_pnl: 4.0,
                author41_trades: 1,
                author42_trades: 1,
                pnl_points: 7.0,
                trades: 2,
            }],
        };
        let source = vec![SourceDaily {
            fixed_model_id: "ri_author41_42_primary_combo_cost2".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 4, 28).unwrap(),
            pnl_points: 7.0,
            author41_pnl: Some(3.0),
            author42_pnl: Some(4.0),
            trades: Some(2.0),
            skipped: None,
        }];

        let comparison = compare_combo_replay(&actual, &source, 1e-9);

        assert_eq!(comparison.daily_exact_matches, 1);
        assert_eq!(comparison.daily_pnl_mismatches, 0);
        assert_eq!(comparison.component_pnl_mismatches, 0);
        assert_eq!(comparison.trade_count_mismatches, 0);
        assert_eq!(comparison.source_total_pnl, 7.0);
        assert_eq!(comparison.actual_total_pnl, 7.0);
    }

    #[test]
    fn combo_shadow_journal_records_accepted_and_dropped_bo() {
        let profile = ModelProfile::ri_shadow_10m();
        let mr = Author41Trade {
            side: ShadowSide::Short,
            entry_ts: dt((2026, 4, 28), (10, 0, 0)),
            exit_ts: dt((2026, 4, 28), (11, 0, 0)),
            entry_price: 100.0,
            exit_price: 95.0,
            gross_points: 5.0,
            net_points: 3.0,
            exit_reason: "take_author_close".to_string(),
            bars_held: 6,
            entry_index_for_day: 1,
        };
        let dropped_bo = Author42Trade {
            side: ShadowSide::Long,
            entry_ts: dt((2026, 4, 28), (10, 30, 0)),
            exit_ts: dt((2026, 4, 28), (12, 0, 0)),
            entry_price: 100.0,
            exit_price: 110.0,
            gross_points: 10.0,
            pnl_points: 10.0,
            exit_reason: "time_exit_same_bar_close".to_string(),
            bars_held: 9,
        };
        let accepted_bo = Author42Trade {
            side: ShadowSide::Short,
            entry_ts: dt((2026, 4, 28), (12, 10, 0)),
            exit_ts: dt((2026, 4, 28), (13, 0, 0)),
            entry_price: 120.0,
            exit_price: 115.0,
            gross_points: 5.0,
            pnl_points: 5.0,
            exit_reason: "time_exit_same_bar_close".to_string(),
            bars_held: 5,
        };

        let mr_record = author41_trade_journal_record(&profile, &mr);
        let dropped_record =
            author42_trade_journal_record(&profile, &dropped_bo, OverlapDecision::DroppedMrOverlap);
        let accepted_record =
            author42_trade_journal_record(&profile, &accepted_bo, OverlapDecision::Accepted);

        assert_eq!(mr_record.component, Component::Author41Mr);
        assert_eq!(mr_record.overlap_decision, OverlapDecision::Accepted);
        assert_eq!(mr_record.shadow_pnl_points, Some(3.0));
        assert_eq!(
            dropped_record.overlap_decision,
            OverlapDecision::DroppedMrOverlap
        );
        assert_eq!(
            dropped_record.skip_reason.as_deref(),
            Some("mr_interval_overlap")
        );
        assert_eq!(dropped_record.shadow_pnl_points, None);
        assert_eq!(accepted_record.overlap_decision, OverlapDecision::Accepted);
        assert_eq!(accepted_record.shadow_pnl_points, Some(3.0));
    }
}
