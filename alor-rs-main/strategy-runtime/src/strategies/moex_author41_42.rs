use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

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
}
