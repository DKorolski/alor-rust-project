use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

use anyhow::{Context, Result, anyhow};

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
}
