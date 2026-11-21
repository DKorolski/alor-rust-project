//! Opt-in dated calendar and dependency selection, with no Strategy or broker route.
//! Existing workers keep their original calendar until a separate integration.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "moex_shadow_calendar_v1";
const MAX_DAYS: i64 = 4096;
const MAX_REGISTRY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionKind {
    Regular,
    Dsvd,
    Closed,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPolicy {
    RegularOnly,
    RegularAndDsvd,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Break {
    pub start_minute: u16,
    pub end_minute: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub date: NaiveDate,
    pub exchange_trade_date: Option<NaiveDate>,
    pub kind: SessionKind,
    pub context_open_minute: Option<u16>,
    pub close_minute: Option<u16>,
    pub main_open_minute: Option<u16>,
    pub main_close_minute: Option<u16>,
    pub breaks: Vec<Break>,
    pub note: String,
}

impl Session {
    fn validate(&self) -> Result<()> {
        ensure!(self.note.len() <= 512, "calendar note bound");
        if matches!(self.kind, SessionKind::Closed | SessionKind::Unknown) {
            ensure!(
                self.context_open_minute.is_none()
                    && self.close_minute.is_none()
                    && self.main_open_minute.is_none()
                    && self.main_close_minute.is_none()
                    && self.breaks.is_empty(),
                "non-session must not invent time boundaries"
            );
            return Ok(());
        }
        let (open, end, main_open, main_end) = (
            self.context_open_minute.context("context open required")?,
            self.close_minute.context("session close required")?,
            self.main_open_minute.context("main open required")?,
            self.main_close_minute.context("main close required")?,
        );
        let midnight = self.date.and_hms_opt(0, 0, 0).unwrap();
        ensure!(
            midnight.checked_sub_signed(Duration::hours(3)).is_some()
                && midnight.checked_add_signed(Duration::days(1)).is_some(),
            "calendar UTC bounds overflow"
        );
        ensure!(
            open < end
                && end <= 1440
                && open % 10 == 0
                && end % 10 == 0
                && open <= main_open
                && main_open < main_end
                && main_end <= end,
            "invalid session bounds/grid"
        );
        if self.kind == SessionKind::Dsvd {
            ensure!(
                open == 590
                    && matches!(end, 1140 | 1430)
                    && main_open == 600
                    && main_end == 1140
                    && self.breaks.is_empty(),
                "unsupported frozen DSVD contract"
            );
        }
        ensure!(self.breaks.len() <= 8, "calendar break bound");
        let mut previous_end = open;
        for pause in &self.breaks {
            ensure!(
                previous_end <= pause.start_minute
                    && pause.start_minute < pause.end_minute
                    && pause.end_minute <= end,
                "invalid or overlapping calendar break"
            );
            previous_end = pause.end_minute;
        }
        ensure!(
            !self.labels().is_empty(),
            "active calendar session has no model bins"
        );
        Ok(())
    }

    fn labels(&self) -> Vec<u16> {
        let (Some(start), Some(end)) = (self.context_open_minute, self.close_minute) else {
            return vec![];
        };
        (start..end)
            .step_by(10)
            .filter(|start| {
                // Preserve the reference's partly tradable legacy bins; no fill is inferred.
                (*start..(*start + 10).min(end)).any(|minute| {
                    !self
                        .breaks
                        .iter()
                        .any(|pause| pause.start_minute <= minute && minute < pause.end_minute)
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub schema_version: String,
    pub calendar_version: String,
    pub reference_sha256: String,
    pub symbol: String,
    pub first_date: NaiveDate,
    pub last_date: NaiveDate,
    pub sessions: Vec<Session>,
}

/// Pin supplied by a reviewed caller, never derived from the file being loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPin {
    pub artifact_sha256: String,
    pub reference_sha256: String,
    pub calendar_version: String,
    pub symbol: String,
    pub first_date: NaiveDate,
    pub last_date: NaiveDate,
}

pub struct Calendar {
    definition: Definition,
    sessions: BTreeMap<NaiveDate, Session>,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Routing {
    pub kind: SessionKind,
    pub mr_context_update_allowed: bool,
    pub bo_context_update_allowed: bool,
    /// Calendar permission only; signal, ownership and executable next-open gates remain separate.
    pub new_entry_allowed: bool,
    /// Local risk/reconcile must continue; this never overrides broker trading windows.
    pub position_management_allowed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageReason {
    CompleteRegular,
    CompleteDsvd,
    MissingScheduledSession,
    IncompleteRegular,
    IncompleteDsvd,
    DuplicateLabels,
    Closed,
    UnknownCalendar,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Coverage {
    pub date: NaiveDate,
    pub calendar_sha256: String,
    pub accepted: bool,
    pub reason: CoverageReason,
    pub expected_bins: usize,
    pub observed_bins: usize,
    pub required_bins: usize,
    pub missing_labels_utc: Vec<i64>,
    pub ignored_labels: usize,
    pub first_label_utc: Option<i64>,
    pub last_label_utc: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Selection {
    pub policy: ContextPolicy,
    pub previous: Option<NaiveDate>,
    pub previous2: Option<NaiveDate>,
    pub ready: bool,
    pub unresolved_dates: Vec<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ComponentSelection {
    pub target_date: NaiveDate,
    pub symbol: String,
    pub calendar_sha256: String,
    pub mr: Selection,
    pub bo: Selection,
    pub combo_ready: bool,
}

impl Calendar {
    pub fn from_json(bytes: &[u8], pin: &ArtifactPin) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_REGISTRY_BYTES,
            "calendar artifact size bound"
        );
        ensure!(
            format!("{:x}", Sha256::digest(bytes)) == pin.artifact_sha256,
            "calendar artifact hash mismatch"
        );
        let definition: Definition = serde_json::from_slice(bytes).context("calendar JSON")?;
        ensure!(
            definition.reference_sha256 == pin.reference_sha256
                && definition.calendar_version == pin.calendar_version
                && definition.symbol == pin.symbol
                && definition.first_date == pin.first_date
                && definition.last_date == pin.last_date,
            "calendar identity or horizon mismatch"
        );
        Self::new(definition)
    }

    pub fn load(path: &Path, pin: &ArtifactPin) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_REGISTRY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        Self::from_json(&bytes, pin)
    }

    pub fn new(definition: Definition) -> Result<Self> {
        ensure!(
            definition.schema_version == SCHEMA,
            "unsupported calendar schema"
        );
        ensure!(
            !definition.calendar_version.is_empty()
                && definition.calendar_version.len() <= 128
                && !definition.symbol.is_empty()
                && definition.symbol.len() <= 32
                && definition.symbol.bytes().all(|b| b.is_ascii_alphanumeric()),
            "invalid calendar identity"
        );
        ensure!(
            definition.reference_sha256.len() == 64
                && definition
                    .reference_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "calendar reference hash required"
        );
        let days = (definition.last_date - definition.first_date).num_days();
        ensure!(
            (0..MAX_DAYS).contains(&days) && definition.sessions.len() <= MAX_DAYS as usize,
            "calendar horizon bound"
        );
        let mut sessions = BTreeMap::new();
        for session in &definition.sessions {
            session.validate()?;
            ensure!(
                definition.first_date <= session.date && session.date <= definition.last_date,
                "session outside declared calendar"
            );
            ensure!(
                sessions.insert(session.date, session.clone()).is_none(),
                "duplicate calendar date"
            );
        }
        let sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&definition)?));
        Ok(Self {
            definition,
            sessions,
            sha256,
        })
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn horizon(&self) -> (NaiveDate, NaiveDate) {
        (self.definition.first_date, self.definition.last_date)
    }

    /// Exact REGULAR dates, independent of A/B price coverage and DSVD context.
    pub fn prior_regular_dates(&self, target: NaiveDate) -> Result<(NaiveDate, NaiveDate)> {
        ensure!(
            self.session(target).kind == SessionKind::Regular
                && target.weekday().num_days_from_monday() < 5,
            "not a regular weekday decision"
        );
        let selected = self.select(target, ContextPolicy::RegularOnly, &BTreeMap::new());
        let previous = selected.previous.context("missing previous regular date")?;
        let previous2 = selected
            .previous2
            .context("missing previous2 regular date")?;
        ensure!(
            self.session(previous).kind == SessionKind::Regular
                && self.session(previous2).kind == SessionKind::Regular,
            "unknown calendar in regular source frontier"
        );
        Ok((previous, previous2))
    }

    pub fn session(&self, date: NaiveDate) -> Session {
        self.sessions
            .get(&date)
            .cloned()
            .unwrap_or_else(|| Session {
                date,
                exchange_trade_date: None,
                kind: SessionKind::Unknown,
                context_open_minute: None,
                close_minute: None,
                main_open_minute: None,
                main_close_minute: None,
                breaks: vec![],
                note: "unclassified_date_no_weekday_fallback".into(),
            })
    }

    fn epoch(date: NaiveDate, minute: u16) -> i64 {
        (date.and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(i64::from(minute))
            - Duration::hours(3))
        .and_utc()
        .timestamp()
    }

    pub fn expected_labels_utc(&self, date: NaiveDate) -> Vec<i64> {
        self.session(date)
            .labels()
            .into_iter()
            .map(|minute| Self::epoch(date, minute))
            .collect()
    }

    pub fn bounds_utc(&self, date: NaiveDate) -> Option<(i64, i64)> {
        let session = self.session(date);
        Some((
            Self::epoch(date, session.context_open_minute?),
            Self::epoch(date, session.close_minute?),
        ))
    }

    pub fn route(
        &self,
        date: NaiveDate,
        label_minute: u16,
        mr: ContextPolicy,
        bo: ContextPolicy,
    ) -> Routing {
        let session = self.session(date);
        let in_grid = session.labels().contains(&label_minute);
        let allowed = |policy| {
            in_grid
                && (session.kind == SessionKind::Regular
                    || (session.kind == SessionKind::Dsvd
                        && policy == ContextPolicy::RegularAndDsvd))
        };
        Routing {
            kind: session.kind,
            mr_context_update_allowed: allowed(mr),
            bo_context_update_allowed: allowed(bo),
            new_entry_allowed: in_grid
                && session.kind == SessionKind::Regular
                && date.weekday().num_days_from_monday() < 5,
            position_management_allowed: true,
        }
    }

    pub fn coverage(&self, date: NaiveDate, labels: &[i64]) -> Coverage {
        let session = self.session(date);
        let expected = self.expected_labels_utc(date);
        let expected_set: BTreeSet<_> = expected.iter().copied().collect();
        let filtered: Vec<_> = labels
            .iter()
            .copied()
            .filter(|ts| expected_set.contains(ts))
            .collect();
        let observed: BTreeSet<_> = filtered.iter().copied().collect();
        let required = if session.kind == SessionKind::Dsvd {
            if session.close_minute == Some(1430) {
                68
            } else {
                44
            }
        } else {
            expected.len()
        };
        let first = observed.first().copied();
        let last = observed.last().copied();
        let reason = if session.kind == SessionKind::Closed {
            CoverageReason::Closed
        } else if session.kind == SessionKind::Unknown {
            CoverageReason::UnknownCalendar
        } else if filtered.len() != observed.len() {
            CoverageReason::DuplicateLabels
        } else if observed.is_empty() {
            CoverageReason::MissingScheduledSession
        } else if session.kind == SessionKind::Regular {
            if observed == expected_set {
                CoverageReason::CompleteRegular
            } else {
                CoverageReason::IncompleteRegular
            }
        } else if observed.len() >= required
            && first.is_some_and(|ts| ts <= Self::epoch(date, 600))
            && last == expected.last().copied()
        {
            CoverageReason::CompleteDsvd
        } else {
            CoverageReason::IncompleteDsvd
        };
        Coverage {
            date,
            calendar_sha256: self.sha256.clone(),
            accepted: matches!(
                reason,
                CoverageReason::CompleteRegular | CoverageReason::CompleteDsvd
            ),
            reason,
            expected_bins: expected.len(),
            observed_bins: observed.len(),
            required_bins: required,
            missing_labels_utc: expected
                .into_iter()
                .filter(|ts| !observed.contains(ts))
                .collect(),
            ignored_labels: labels.len() - filtered.len(),
            first_label_utc: first,
            last_label_utc: last,
        }
    }

    fn select(
        &self,
        target: NaiveDate,
        policy: ContextPolicy,
        coverage: &BTreeMap<NaiveDate, Coverage>,
    ) -> Selection {
        let mut candidates = Vec::new();
        let mut date = self.definition.first_date;
        while date < target && date <= self.definition.last_date {
            let kind = self.session(date).kind;
            if matches!(kind, SessionKind::Regular | SessionKind::Unknown)
                || (kind == SessionKind::Dsvd && policy == ContextPolicy::RegularAndDsvd)
            {
                candidates.push(date);
            }
            date = date.succ_opt().unwrap();
        }
        let chosen: Vec<_> = candidates.iter().rev().take(2).copied().collect();
        let unresolved_dates: Vec<_> = chosen
            .iter()
            .copied()
            .filter(|date| {
                self.session(*date).kind == SessionKind::Unknown
                    || !coverage.get(date).is_some_and(|c| {
                        c.date == *date && c.calendar_sha256 == self.sha256 && c.accepted
                    })
            })
            .collect();
        Selection {
            policy,
            previous: chosen.first().copied(),
            previous2: chosen.get(1).copied(),
            ready: chosen.len() == 2
                && unresolved_dates.is_empty()
                && self.session(target).kind == SessionKind::Regular,
            unresolved_dates,
        }
    }

    pub fn select_components(
        &self,
        target: NaiveDate,
        mr: ContextPolicy,
        bo: ContextPolicy,
        coverage: &BTreeMap<NaiveDate, Coverage>,
    ) -> ComponentSelection {
        let mr = self.select(target, mr, coverage);
        let bo = self.select(target, bo, coverage);
        ComponentSelection {
            target_date: target,
            symbol: self.definition.symbol.clone(),
            calendar_sha256: self.sha256.clone(),
            combo_ready: mr.ready && bo.ready,
            mr,
            bo,
        }
    }
}

#[cfg(test)]
mod tests;
