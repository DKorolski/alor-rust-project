use std::collections::HashMap;
use std::io::Read;

use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

pub const SHADOW_PNL_LB120_LOOKBACK_SESSIONS: usize = 120;
pub const SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS: usize = 60;
pub const RISK_GATE_STATE_GENERATION: &str = "runtime-ledger-v1";

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateSessionRow {
    pub session_date: NaiveDate,
    pub shadow_pnl_points: f64,
    pub shadow_trade_count: u32,
    pub rolling_sum_before_session: f64,
    pub mr_enabled_for_session: bool,
    pub source: RiskGateRowSource,
    pub status: RiskGateRowStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskGateRowSource {
    Seed,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskGateRowStatus {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskGateStartupMode {
    BootstrapFromSeed,
    NormalAppend,
    RebuildFromHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskGateStartupDecision {
    ImportSeed,
    UseExistingLedger,
    RebuildFromSeed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskGateProfileIdentity {
    pub strategy_id: String,
    pub profile_id: String,
    pub mr_variant: String,
    pub timeframe: String,
    pub session_policy: String,
    pub model_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateStartupArtifacts {
    pub decision: RiskGateStartupDecision,
    pub ledger_records: Vec<RiskGateLedgerRecord>,
    pub records_to_write: Vec<RiskGateLedgerRecord>,
    pub materialized_state: RiskGateMaterializedState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskGateRedisKeys {
    pub ledger_stream: String,
    pub state_key: String,
    pub finalized_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateLedgerRecord {
    pub row: RiskGateSessionRow,
    pub profile_id: String,
    pub mr_variant: String,
    pub timeframe: String,
    pub session_policy: String,
    pub rolling_sum_lb120: f64,
    pub mr_enabled_next_session: bool,
    pub model_version: String,
    pub finalized_at_utc: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateMaterializedState {
    pub last_finalized_session_date: Option<NaiveDate>,
    pub rolling_sum_lb120: Option<f64>,
    pub mr_enabled_current_session: Option<bool>,
    pub mr_enabled_next_session: Option<bool>,
    pub seed_loaded: bool,
    pub ledger_rows_count: usize,
    pub current_shadow_session_date: Option<NaiveDate>,
    pub current_shadow_pnl_points: f64,
    pub current_generation: String,
}

impl RiskGateRedisKeys {
    pub fn for_profile(strategy_id: &str, profile_id: &str, session_date: NaiveDate) -> Self {
        let strategy_id = redis_key_component(strategy_id);
        let profile_id = redis_key_component(profile_id);
        let session_date = session_date.format("%Y-%m-%d");
        Self {
            ledger_stream: format!("runtime.riskgate.sessions.{strategy_id}.{profile_id}"),
            state_key: format!("runtime.riskgate.state.{strategy_id}.{profile_id}"),
            finalized_key: format!(
                "runtime.riskgate.finalized.{strategy_id}.{profile_id}.{session_date}"
            ),
        }
    }
}

impl RiskGateLedgerRecord {
    pub fn redis_fields(&self) -> Vec<(String, String)> {
        vec![
            (
                "session_date".to_string(),
                self.row.session_date.format("%Y-%m-%d").to_string(),
            ),
            ("profile_id".to_string(), self.profile_id.clone()),
            ("mr_variant".to_string(), self.mr_variant.clone()),
            ("timeframe".to_string(), self.timeframe.clone()),
            (
                "shadow_pnl_points".to_string(),
                format_float(self.row.shadow_pnl_points),
            ),
            (
                "shadow_trade_count".to_string(),
                self.row.shadow_trade_count.to_string(),
            ),
            (
                "rolling_120_pnl_before_session".to_string(),
                format_float(self.row.rolling_sum_before_session),
            ),
            (
                "mr_enabled_for_session".to_string(),
                bool_field(self.row.mr_enabled_for_session).to_string(),
            ),
            (
                "rolling_sum_lb120".to_string(),
                format_float(self.rolling_sum_lb120),
            ),
            (
                "mr_enabled_next_session".to_string(),
                bool_field(self.mr_enabled_next_session).to_string(),
            ),
            ("source".to_string(), self.row.source.as_str().to_string()),
            ("status".to_string(), self.row.status.as_str().to_string()),
            ("session_policy".to_string(), self.session_policy.clone()),
            ("model_version".to_string(), self.model_version.clone()),
            (
                "finalized_at_utc".to_string(),
                self.finalized_at_utc.to_string(),
            ),
        ]
    }

    pub fn from_redis_fields(fields: &[(String, String)]) -> Result<Self, String> {
        let fields = field_map(fields);
        let session_date = parse_required_date(&fields, "session_date")?;
        let profile_id = required_field(&fields, "profile_id")?.to_string();
        let mr_variant = required_field(&fields, "mr_variant")?.to_string();
        let timeframe = required_field(&fields, "timeframe")?.to_string();
        let session_policy = required_field(&fields, "session_policy")?.to_string();
        let shadow_pnl_points = parse_required_f64(&fields, "shadow_pnl_points")?;
        let shadow_trade_count = parse_required_u32(&fields, "shadow_trade_count")?;
        let rolling_sum_before_session =
            parse_required_f64(&fields, "rolling_120_pnl_before_session")?;
        let mr_enabled_for_session = parse_required_bool(&fields, "mr_enabled_for_session")?;
        let source = parse_row_source(required_field(&fields, "source")?)?;
        let status = parse_row_status(required_field(&fields, "status")?)?;
        let rolling_sum_lb120 = parse_required_f64(&fields, "rolling_sum_lb120")?;
        let mr_enabled_next_session = parse_required_bool(&fields, "mr_enabled_next_session")?;
        let model_version = required_field(&fields, "model_version")?.to_string();
        let finalized_at_utc = parse_required_i64(&fields, "finalized_at_utc")?;

        Ok(Self {
            row: RiskGateSessionRow {
                session_date,
                shadow_pnl_points,
                shadow_trade_count,
                rolling_sum_before_session,
                mr_enabled_for_session,
                source,
                status,
            },
            profile_id,
            mr_variant,
            timeframe,
            session_policy,
            rolling_sum_lb120,
            mr_enabled_next_session,
            model_version,
            finalized_at_utc,
        })
    }
}

impl RiskGateMaterializedState {
    pub fn redis_fields(&self) -> Vec<(String, String)> {
        let mut fields = vec![
            (
                "seed_loaded".to_string(),
                bool_field(self.seed_loaded).to_string(),
            ),
            (
                "ledger_rows_count".to_string(),
                self.ledger_rows_count.to_string(),
            ),
            (
                "current_shadow_pnl_points".to_string(),
                format_float(self.current_shadow_pnl_points),
            ),
            (
                "current_generation".to_string(),
                self.current_generation.clone(),
            ),
        ];
        push_optional_date(
            &mut fields,
            "last_finalized_session_date",
            self.last_finalized_session_date,
        );
        push_optional_float(&mut fields, "rolling_sum_lb120", self.rolling_sum_lb120);
        push_optional_bool(
            &mut fields,
            "mr_enabled_current_session",
            self.mr_enabled_current_session,
        );
        push_optional_bool(
            &mut fields,
            "mr_enabled_next_session",
            self.mr_enabled_next_session,
        );
        push_optional_date(
            &mut fields,
            "current_shadow_session_date",
            self.current_shadow_session_date,
        );
        fields
    }

    pub fn from_redis_fields(fields: &[(String, String)]) -> Result<Self, String> {
        let fields = field_map(fields);
        Ok(Self {
            last_finalized_session_date: parse_optional_date(
                &fields,
                "last_finalized_session_date",
            )?,
            rolling_sum_lb120: parse_optional_f64(&fields, "rolling_sum_lb120")?,
            mr_enabled_current_session: parse_optional_bool(&fields, "mr_enabled_current_session")?,
            mr_enabled_next_session: parse_optional_bool(&fields, "mr_enabled_next_session")?,
            seed_loaded: parse_required_bool(&fields, "seed_loaded")?,
            ledger_rows_count: parse_required_usize(&fields, "ledger_rows_count")?,
            current_shadow_session_date: parse_optional_date(
                &fields,
                "current_shadow_session_date",
            )?,
            current_shadow_pnl_points: parse_required_f64(&fields, "current_shadow_pnl_points")?,
            current_generation: required_field(&fields, "current_generation")?.to_string(),
        })
    }
}

impl RiskGateRowSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RiskGateRowSource::Seed => "seed",
            RiskGateRowSource::Runtime => "runtime",
        }
    }
}

impl RiskGateRowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RiskGateRowStatus::Complete => "complete",
            RiskGateRowStatus::Incomplete => "incomplete",
        }
    }
}

#[derive(Debug, Deserialize)]
struct SeedCsvRow {
    date: String,
    shadow_pnl_points: f64,
    shadow_trade_count: u32,
    rolling_120_pnl_before_session: f64,
    mr_enabled_for_session: String,
    source: String,
    status: String,
}

fn redis_key_component(raw: &str) -> String {
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized.to_string()
    }
}

fn bool_field(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn format_float(value: f64) -> String {
    if value.fract().abs() <= f64::EPSILON {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn push_optional_date(fields: &mut Vec<(String, String)>, key: &str, value: Option<NaiveDate>) {
    if let Some(value) = value {
        fields.push((key.to_string(), value.format("%Y-%m-%d").to_string()));
    }
}

fn push_optional_float(fields: &mut Vec<(String, String)>, key: &str, value: Option<f64>) {
    if let Some(value) = value {
        fields.push((key.to_string(), format_float(value)));
    }
}

fn push_optional_bool(fields: &mut Vec<(String, String)>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        fields.push((key.to_string(), bool_field(value).to_string()));
    }
}

fn field_map(fields: &[(String, String)]) -> HashMap<&str, &str> {
    fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
}

fn required_field<'a>(fields: &'a HashMap<&str, &str>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .copied()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("risk gate redis field missing: {key}"))
}

fn parse_required_date(fields: &HashMap<&str, &str>, key: &str) -> Result<NaiveDate, String> {
    let raw = required_field(fields, key)?;
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map_err(|err| format!("invalid risk gate date field {key}={raw}: {err}"))
}

fn parse_optional_date(
    fields: &HashMap<&str, &str>,
    key: &str,
) -> Result<Option<NaiveDate>, String> {
    fields
        .get(key)
        .copied()
        .filter(|value| !value.trim().is_empty())
        .map(|raw| {
            NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
                .map_err(|err| format!("invalid risk gate date field {key}={raw}: {err}"))
        })
        .transpose()
}

fn parse_required_f64(fields: &HashMap<&str, &str>, key: &str) -> Result<f64, String> {
    let raw = required_field(fields, key)?;
    raw.parse::<f64>()
        .map_err(|err| format!("invalid risk gate f64 field {key}={raw}: {err}"))
}

fn parse_optional_f64(fields: &HashMap<&str, &str>, key: &str) -> Result<Option<f64>, String> {
    fields
        .get(key)
        .copied()
        .filter(|value| !value.trim().is_empty())
        .map(|raw| {
            raw.parse::<f64>()
                .map_err(|err| format!("invalid risk gate f64 field {key}={raw}: {err}"))
        })
        .transpose()
}

fn parse_required_i64(fields: &HashMap<&str, &str>, key: &str) -> Result<i64, String> {
    let raw = required_field(fields, key)?;
    raw.parse::<i64>()
        .map_err(|err| format!("invalid risk gate i64 field {key}={raw}: {err}"))
}

fn parse_required_u32(fields: &HashMap<&str, &str>, key: &str) -> Result<u32, String> {
    let raw = required_field(fields, key)?;
    raw.parse::<u32>()
        .map_err(|err| format!("invalid risk gate u32 field {key}={raw}: {err}"))
}

fn parse_required_usize(fields: &HashMap<&str, &str>, key: &str) -> Result<usize, String> {
    let raw = required_field(fields, key)?;
    raw.parse::<usize>()
        .map_err(|err| format!("invalid risk gate usize field {key}={raw}: {err}"))
}

fn parse_required_bool(fields: &HashMap<&str, &str>, key: &str) -> Result<bool, String> {
    parse_seed_bool(required_field(fields, key)?)
}

fn parse_optional_bool(fields: &HashMap<&str, &str>, key: &str) -> Result<Option<bool>, String> {
    fields
        .get(key)
        .copied()
        .filter(|value| !value.trim().is_empty())
        .map(parse_seed_bool)
        .transpose()
}

pub fn parse_seed_csv<R: Read>(reader: R) -> Result<Vec<RiskGateSessionRow>, String> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(reader);
    let mut rows = Vec::new();
    for result in csv_reader.deserialize::<SeedCsvRow>() {
        let raw = result.map_err(|err| format!("invalid risk gate seed csv row: {err}"))?;
        rows.push(RiskGateSessionRow {
            session_date: NaiveDate::parse_from_str(raw.date.trim(), "%Y-%m-%d")
                .map_err(|err| format!("invalid risk gate seed date {}: {err}", raw.date))?,
            shadow_pnl_points: raw.shadow_pnl_points,
            shadow_trade_count: raw.shadow_trade_count,
            rolling_sum_before_session: raw.rolling_120_pnl_before_session,
            mr_enabled_for_session: parse_seed_bool(&raw.mr_enabled_for_session)?,
            source: parse_row_source(&raw.source)?,
            status: parse_row_status(&raw.status)?,
        });
    }
    validate_regular_session_ledger(&rows)?;
    Ok(rows)
}

fn parse_seed_bool(raw: &str) -> Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        other => Err(format!("invalid risk gate bool: {other}")),
    }
}

fn parse_row_source(raw: &str) -> Result<RiskGateRowSource, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "seed" => Ok(RiskGateRowSource::Seed),
        "runtime" => Ok(RiskGateRowSource::Runtime),
        other => Err(format!("invalid risk gate row source: {other}")),
    }
}

fn parse_row_status(raw: &str) -> Result<RiskGateRowStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "complete" => Ok(RiskGateRowStatus::Complete),
        "incomplete" => Ok(RiskGateRowStatus::Incomplete),
        other => Err(format!("invalid risk gate row status: {other}")),
    }
}

pub fn validate_regular_session_ledger(rows: &[RiskGateSessionRow]) -> Result<(), String> {
    let mut prev: Option<NaiveDate> = None;
    for row in rows {
        if matches!(
            row.session_date.weekday(),
            chrono::Weekday::Sat | chrono::Weekday::Sun
        ) {
            return Err(format!(
                "risk gate ledger contains weekend session: {}",
                row.session_date
            ));
        }
        if let Some(prev_date) = prev {
            if row.session_date <= prev_date {
                return Err(format!(
                    "risk gate ledger dates are not strictly increasing: {} <= {}",
                    row.session_date, prev_date
                ));
            }
        }
        prev = Some(row.session_date);
    }
    Ok(())
}

pub fn reconcile_seed_with_ledger(
    mode: RiskGateStartupMode,
    existing_rows: &[RiskGateSessionRow],
    seed_rows: &[RiskGateSessionRow],
) -> Result<RiskGateStartupDecision, String> {
    validate_regular_session_ledger(existing_rows)?;
    validate_regular_session_ledger(seed_rows)?;
    let existing_last = existing_rows.last().map(|row| row.session_date);
    let seed_last = seed_rows.last().map(|row| row.session_date);

    match (mode, existing_last, seed_last) {
        (RiskGateStartupMode::BootstrapFromSeed, None, Some(_)) => {
            Ok(RiskGateStartupDecision::ImportSeed)
        }
        (RiskGateStartupMode::BootstrapFromSeed, Some(_), _) => Err(
            "risk gate bootstrap_from_seed refused because runtime ledger already exists"
                .to_string(),
        ),
        (RiskGateStartupMode::BootstrapFromSeed, None, None) => {
            Err("risk gate bootstrap_from_seed has no seed rows".to_string())
        }
        (RiskGateStartupMode::NormalAppend, Some(existing), Some(seed)) if existing >= seed => {
            Ok(RiskGateStartupDecision::UseExistingLedger)
        }
        (RiskGateStartupMode::NormalAppend, Some(_), None) => {
            Ok(RiskGateStartupDecision::UseExistingLedger)
        }
        (RiskGateStartupMode::NormalAppend, Some(existing), Some(seed)) => Err(format!(
            "risk gate normal_append refused because ledger is behind seed: ledger_last={existing}, seed_last={seed}"
        )),
        (RiskGateStartupMode::NormalAppend, None, _) => {
            Err("risk gate normal_append refused because runtime ledger is empty".to_string())
        }
        (RiskGateStartupMode::RebuildFromHistory, _, Some(_)) => {
            Ok(RiskGateStartupDecision::RebuildFromSeed)
        }
        (RiskGateStartupMode::RebuildFromHistory, _, None) => {
            Err("risk gate rebuild_from_history has no rebuilt rows".to_string())
        }
    }
}

pub fn rolling_shadow_pnl_before_next_session(
    rows: &[RiskGateSessionRow],
    lookback_sessions: usize,
    min_history_sessions: usize,
) -> Option<f64> {
    let complete_rows = rows
        .iter()
        .filter(|row| row.status == RiskGateRowStatus::Complete)
        .collect::<Vec<_>>();
    if complete_rows.len() < min_history_sessions {
        return None;
    }
    let start = complete_rows.len().saturating_sub(lookback_sessions);
    Some(
        complete_rows[start..]
            .iter()
            .map(|row| row.shadow_pnl_points)
            .sum(),
    )
}

pub fn mr_enabled_for_next_session(rows: &[RiskGateSessionRow]) -> Option<bool> {
    rolling_shadow_pnl_before_next_session(
        rows,
        SHADOW_PNL_LB120_LOOKBACK_SESSIONS,
        SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS,
    )
    .map(|pnl| pnl > 0.0)
}

pub fn rolling_shadow_pnl_before_session(
    rows: &[RiskGateSessionRow],
    session_date: NaiveDate,
    lookback_sessions: usize,
    min_history_sessions: usize,
) -> Option<f64> {
    let complete_rows = rows
        .iter()
        .filter(|row| row.status == RiskGateRowStatus::Complete && row.session_date < session_date)
        .collect::<Vec<_>>();
    if complete_rows.len() < min_history_sessions {
        return None;
    }
    let start = complete_rows.len().saturating_sub(lookback_sessions);
    Some(
        complete_rows[start..]
            .iter()
            .map(|row| row.shadow_pnl_points)
            .sum(),
    )
}

pub fn mr_enabled_for_session(
    rows: &[RiskGateSessionRow],
    session_date: NaiveDate,
) -> Option<bool> {
    rolling_shadow_pnl_before_session(
        rows,
        session_date,
        SHADOW_PNL_LB120_LOOKBACK_SESSIONS,
        SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS,
    )
    .map(|pnl| pnl > 0.0)
}

pub fn build_runtime_session_row(
    existing_rows: &[RiskGateSessionRow],
    session_date: NaiveDate,
    shadow_pnl_points: f64,
    shadow_trade_count: u32,
) -> Result<RiskGateSessionRow, String> {
    validate_regular_session_ledger(existing_rows)?;
    if matches!(
        session_date.weekday(),
        chrono::Weekday::Sat | chrono::Weekday::Sun
    ) {
        return Err(format!(
            "risk gate runtime row cannot use weekend session: {session_date}"
        ));
    }
    if existing_rows
        .last()
        .is_some_and(|row| session_date <= row.session_date)
    {
        return Err(format!(
            "risk gate runtime row is not after ledger tail: {session_date}"
        ));
    }
    let rolling_sum_before_session = rolling_shadow_pnl_before_session(
        existing_rows,
        session_date,
        SHADOW_PNL_LB120_LOOKBACK_SESSIONS,
        SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS,
    )
    .unwrap_or(0.0);
    Ok(RiskGateSessionRow {
        session_date,
        shadow_pnl_points,
        shadow_trade_count,
        rolling_sum_before_session,
        mr_enabled_for_session: rolling_sum_before_session > 0.0,
        source: RiskGateRowSource::Runtime,
        status: RiskGateRowStatus::Complete,
    })
}

pub fn build_ledger_records_from_rows(
    rows: &[RiskGateSessionRow],
    identity: &RiskGateProfileIdentity,
    finalized_at_utc: i64,
) -> Result<Vec<RiskGateLedgerRecord>, String> {
    validate_regular_session_ledger(rows)?;
    let mut records = Vec::with_capacity(rows.len());
    for idx in 0..rows.len() {
        let prefix = &rows[..=idx];
        let rolling_sum_lb120 =
            rolling_shadow_pnl_before_next_session(prefix, SHADOW_PNL_LB120_LOOKBACK_SESSIONS, 0)
                .unwrap_or(0.0);
        let mr_enabled_next_session = rolling_shadow_pnl_before_next_session(
            prefix,
            SHADOW_PNL_LB120_LOOKBACK_SESSIONS,
            SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS,
        )
        .map(|pnl| pnl > 0.0)
        .unwrap_or(false);
        records.push(RiskGateLedgerRecord {
            row: rows[idx].clone(),
            profile_id: identity.profile_id.clone(),
            mr_variant: identity.mr_variant.clone(),
            timeframe: identity.timeframe.clone(),
            session_policy: identity.session_policy.clone(),
            rolling_sum_lb120,
            mr_enabled_next_session,
            model_version: identity.model_version.clone(),
            finalized_at_utc,
        });
    }
    Ok(records)
}

pub fn rows_from_ledger_records(
    records: &[RiskGateLedgerRecord],
) -> Result<Vec<RiskGateSessionRow>, String> {
    let mut rows = records
        .iter()
        .map(|record| record.row.clone())
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.session_date);
    validate_regular_session_ledger(&rows)?;
    Ok(rows)
}

pub fn validate_ledger_record_identity(
    records: &[RiskGateLedgerRecord],
    identity: &RiskGateProfileIdentity,
) -> Result<(), String> {
    for record in records {
        if record.profile_id != identity.profile_id {
            return Err(format!(
                "risk gate ledger profile_id mismatch: {} != {}",
                record.profile_id, identity.profile_id
            ));
        }
        if record.mr_variant != identity.mr_variant {
            return Err(format!(
                "risk gate ledger mr_variant mismatch: {} != {}",
                record.mr_variant, identity.mr_variant
            ));
        }
        if record.timeframe != identity.timeframe {
            return Err(format!(
                "risk gate ledger timeframe mismatch: {} != {}",
                record.timeframe, identity.timeframe
            ));
        }
        if record.session_policy != identity.session_policy {
            return Err(format!(
                "risk gate ledger session_policy mismatch: {} != {}",
                record.session_policy, identity.session_policy
            ));
        }
    }
    Ok(())
}

pub fn rebuild_materialized_state_from_ledger_records(
    records: &[RiskGateLedgerRecord],
    current_shadow_session_date: Option<NaiveDate>,
    current_shadow_pnl_points: f64,
    seed_loaded: bool,
) -> Result<RiskGateMaterializedState, String> {
    let rows = rows_from_ledger_records(records)?;
    let last_finalized_session_date = rows.last().map(|row| row.session_date);
    let rolling_sum_lb120 = rolling_shadow_pnl_before_next_session(
        &rows,
        SHADOW_PNL_LB120_LOOKBACK_SESSIONS,
        SHADOW_PNL_LB120_MIN_HISTORY_SESSIONS,
    );
    let mr_enabled = rolling_sum_lb120.map(|pnl| pnl > 0.0);
    Ok(RiskGateMaterializedState {
        last_finalized_session_date,
        rolling_sum_lb120,
        mr_enabled_current_session: mr_enabled,
        mr_enabled_next_session: mr_enabled,
        seed_loaded,
        ledger_rows_count: rows.len(),
        current_shadow_session_date,
        current_shadow_pnl_points,
        current_generation: RISK_GATE_STATE_GENERATION.to_string(),
    })
}

pub fn plan_risk_gate_startup(
    mode: RiskGateStartupMode,
    existing_records: &[RiskGateLedgerRecord],
    seed_rows: &[RiskGateSessionRow],
    identity: &RiskGateProfileIdentity,
    current_shadow_session_date: Option<NaiveDate>,
    finalized_at_utc: i64,
) -> Result<RiskGateStartupArtifacts, String> {
    validate_ledger_record_identity(existing_records, identity)?;
    let existing_rows = rows_from_ledger_records(existing_records)?;
    let decision = reconcile_seed_with_ledger(mode, &existing_rows, seed_rows)?;
    let (ledger_records, records_to_write) = match decision {
        RiskGateStartupDecision::UseExistingLedger => {
            let mut records = existing_records.to_vec();
            records.sort_by_key(|record| record.row.session_date);
            (records, Vec::new())
        }
        RiskGateStartupDecision::ImportSeed | RiskGateStartupDecision::RebuildFromSeed => {
            let records = build_ledger_records_from_rows(seed_rows, identity, finalized_at_utc)?;
            (records.clone(), records)
        }
    };
    let seed_loaded = ledger_records
        .iter()
        .any(|record| record.row.source == RiskGateRowSource::Seed);
    let materialized_state = rebuild_materialized_state_from_ledger_records(
        &ledger_records,
        current_shadow_session_date,
        0.0,
        seed_loaded,
    )?;
    Ok(RiskGateStartupArtifacts {
        decision,
        ledger_records,
        records_to_write,
        materialized_state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: u32, pnl: f64) -> RiskGateSessionRow {
        RiskGateSessionRow {
            session_date: NaiveDate::from_ymd_opt(2026, 1, day).unwrap_or(NaiveDate::MIN),
            shadow_pnl_points: pnl,
            shadow_trade_count: u32::from(pnl != 0.0),
            rolling_sum_before_session: 0.0,
            mr_enabled_for_session: false,
            source: RiskGateRowSource::Seed,
            status: RiskGateRowStatus::Complete,
        }
    }

    fn regular_session_date(offset: i64) -> NaiveDate {
        let mut date = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap_or(NaiveDate::MIN);
        let mut remaining = offset;
        while remaining > 0 {
            date += chrono::Duration::days(1);
            if !matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
                remaining -= 1;
            }
        }
        date
    }

    fn identity() -> RiskGateProfileIdentity {
        RiskGateProfileIdentity {
            strategy_id: "hybrid_imoexf".to_string(),
            profile_id: "imoexf_primary_high180_lb120".to_string(),
            mr_variant: "high180".to_string(),
            timeframe: "10m".to_string(),
            session_policy: "Mon-Fri 09:00..23:49".to_string(),
            model_version: "2026-04-26".to_string(),
        }
    }

    #[test]
    fn rejects_weekend_rows() {
        let rows = [RiskGateSessionRow {
            session_date: NaiveDate::from_ymd_opt(2026, 1, 3).unwrap_or(NaiveDate::MIN),
            shadow_pnl_points: 0.0,
            shadow_trade_count: 0,
            rolling_sum_before_session: 0.0,
            mr_enabled_for_session: false,
            source: RiskGateRowSource::Seed,
            status: RiskGateRowStatus::Complete,
        }];

        assert!(validate_regular_session_ledger(&rows).is_err());
    }

    #[test]
    fn rejects_non_monotonic_rows() {
        let rows = [row(5, 1.0), row(5, 2.0)];

        assert!(validate_regular_session_ledger(&rows).is_err());
    }

    #[test]
    fn next_session_gate_uses_complete_history_only() {
        let rows = (0..60)
            .map(|idx| RiskGateSessionRow {
                session_date: regular_session_date(idx),
                shadow_pnl_points: 1.0,
                shadow_trade_count: 1,
                rolling_sum_before_session: 0.0,
                mr_enabled_for_session: false,
                source: RiskGateRowSource::Seed,
                status: RiskGateRowStatus::Complete,
            })
            .collect::<Vec<_>>();

        assert_eq!(mr_enabled_for_next_session(&rows), Some(true));
    }

    #[test]
    fn parses_seed_csv_and_validates_contract() {
        let csv = b"date,shadow_pnl_points,shadow_trade_count,rolling_120_pnl_before_session,mr_enabled_for_session,source,status\n2026-01-05,1.5,1,0.0,False,seed,complete\n2026-01-06,-0.5,1,1.5,True,seed,complete\n";

        let rows = parse_seed_csv(&csv[..]).expect("seed csv parses");

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].session_date,
            NaiveDate::from_ymd_opt(2026, 1, 5).unwrap_or(NaiveDate::MIN)
        );
        assert_eq!(rows[0].source, RiskGateRowSource::Seed);
        assert_eq!(rows[0].status, RiskGateRowStatus::Complete);
        assert!(!rows[0].mr_enabled_for_session);
        assert!(rows[1].mr_enabled_for_session);
    }

    #[test]
    fn bootstrap_import_requires_empty_ledger() {
        let seed_rows = [row(5, 1.0)];

        assert_eq!(
            reconcile_seed_with_ledger(RiskGateStartupMode::BootstrapFromSeed, &[], &seed_rows,),
            Ok(RiskGateStartupDecision::ImportSeed)
        );
        assert!(reconcile_seed_with_ledger(
            RiskGateStartupMode::BootstrapFromSeed,
            &seed_rows,
            &seed_rows,
        )
        .is_err());
    }

    #[test]
    fn normal_append_refuses_ledger_behind_seed() {
        let existing_rows = [row(5, 1.0)];
        let seed_rows = [row(5, 1.0), row(6, 2.0)];

        let err = reconcile_seed_with_ledger(
            RiskGateStartupMode::NormalAppend,
            &existing_rows,
            &seed_rows,
        )
        .expect_err("stale ledger must be refused");

        assert!(err.contains("ledger is behind seed"));
    }

    #[test]
    fn gate_for_session_excludes_current_session_pnl() {
        let rows = (0..61)
            .map(|idx| RiskGateSessionRow {
                session_date: regular_session_date(idx),
                shadow_pnl_points: if idx == 60 { 100.0 } else { -1.0 },
                shadow_trade_count: 1,
                rolling_sum_before_session: 0.0,
                mr_enabled_for_session: false,
                source: RiskGateRowSource::Seed,
                status: RiskGateRowStatus::Complete,
            })
            .collect::<Vec<_>>();
        let current_session = regular_session_date(60);
        let next_session = regular_session_date(61);

        assert_eq!(mr_enabled_for_session(&rows, current_session), Some(false));
        assert_eq!(mr_enabled_for_session(&rows, next_session), Some(true));
    }

    #[test]
    fn build_runtime_row_uses_prior_complete_sessions() {
        let rows = (0..60)
            .map(|idx| RiskGateSessionRow {
                session_date: regular_session_date(idx),
                shadow_pnl_points: 1.0,
                shadow_trade_count: 1,
                rolling_sum_before_session: 0.0,
                mr_enabled_for_session: false,
                source: RiskGateRowSource::Seed,
                status: RiskGateRowStatus::Complete,
            })
            .collect::<Vec<_>>();

        let row = build_runtime_session_row(&rows, regular_session_date(60), -2.0, 1)
            .expect("runtime row");

        assert_eq!(row.source, RiskGateRowSource::Runtime);
        assert_eq!(row.rolling_sum_before_session, 60.0);
        assert!(row.mr_enabled_for_session);
    }

    #[test]
    fn redis_keys_follow_runtime_owned_ledger_contract() {
        let keys = RiskGateRedisKeys::for_profile(
            "Hybrid IMOEXF",
            "imoexf_primary_high180_lb120",
            NaiveDate::from_ymd_opt(2026, 4, 24).unwrap_or(NaiveDate::MIN),
        );

        assert_eq!(
            keys.ledger_stream,
            "runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120"
        );
        assert_eq!(
            keys.state_key,
            "runtime.riskgate.state.hybrid_imoexf.imoexf_primary_high180_lb120"
        );
        assert_eq!(
            keys.finalized_key,
            "runtime.riskgate.finalized.hybrid_imoexf.imoexf_primary_high180_lb120.2026-04-24"
        );
    }

    #[test]
    fn ledger_record_fields_separate_current_and_next_session_gate() {
        let record = RiskGateLedgerRecord {
            row: RiskGateSessionRow {
                session_date: NaiveDate::from_ymd_opt(2026, 4, 24).unwrap_or(NaiveDate::MIN),
                shadow_pnl_points: -2.5,
                shadow_trade_count: 2,
                rolling_sum_before_session: 11.0,
                mr_enabled_for_session: true,
                source: RiskGateRowSource::Runtime,
                status: RiskGateRowStatus::Complete,
            },
            profile_id: "imoexf_primary_high180_lb120".to_string(),
            mr_variant: "high180".to_string(),
            timeframe: "10m".to_string(),
            session_policy: "Mon-Fri 09:00..23:49".to_string(),
            rolling_sum_lb120: 8.5,
            mr_enabled_next_session: true,
            model_version: "2026-04-26".to_string(),
            finalized_at_utc: 1_776_990_000,
        };
        let fields = record.redis_fields();

        assert!(fields.contains(&("session_date".to_string(), "2026-04-24".to_string())));
        assert!(fields.contains(&("timeframe".to_string(), "10m".to_string())));
        assert!(fields.contains(&("shadow_pnl_points".to_string(), "-2.5".to_string())));
        assert!(fields.contains(&(
            "rolling_120_pnl_before_session".to_string(),
            "11.0".to_string()
        )));
        assert!(fields.contains(&("mr_enabled_for_session".to_string(), "true".to_string())));
        assert!(fields.contains(&("rolling_sum_lb120".to_string(), "8.5".to_string())));
        assert!(fields.contains(&("mr_enabled_next_session".to_string(), "true".to_string())));
        assert!(fields.contains(&(
            "session_policy".to_string(),
            "Mon-Fri 09:00..23:49".to_string()
        )));

        let parsed = RiskGateLedgerRecord::from_redis_fields(&fields)
            .expect("ledger record round-trips through redis fields");

        assert_eq!(parsed, record);
    }

    #[test]
    fn materialized_state_fields_are_fast_state_only() {
        let state = RiskGateMaterializedState {
            last_finalized_session_date: Some(
                NaiveDate::from_ymd_opt(2026, 4, 24).unwrap_or(NaiveDate::MIN),
            ),
            rolling_sum_lb120: Some(8.5),
            mr_enabled_current_session: Some(true),
            mr_enabled_next_session: Some(false),
            seed_loaded: true,
            ledger_rows_count: 181,
            current_shadow_session_date: Some(
                NaiveDate::from_ymd_opt(2026, 4, 27).unwrap_or(NaiveDate::MIN),
            ),
            current_shadow_pnl_points: 0.0,
            current_generation: "runtime-ledger-v1".to_string(),
        };
        let fields = state.redis_fields();

        assert!(fields.contains(&("seed_loaded".to_string(), "true".to_string())));
        assert!(fields.contains(&("ledger_rows_count".to_string(), "181".to_string())));
        assert!(fields.contains(&(
            "last_finalized_session_date".to_string(),
            "2026-04-24".to_string()
        )));
        assert!(fields.contains(&("rolling_sum_lb120".to_string(), "8.5".to_string())));
        assert!(fields.contains(&(
            "current_shadow_session_date".to_string(),
            "2026-04-27".to_string()
        )));
        assert!(!fields.iter().any(|(key, _)| key == "ledger_rows_json"));

        let parsed = RiskGateMaterializedState::from_redis_fields(&fields)
            .expect("materialized state round-trips through redis fields");

        assert_eq!(parsed, state);
    }

    #[test]
    fn materialized_state_rebuilds_from_ledger_records_not_cache() {
        let records = (0..60)
            .rev()
            .map(|idx| {
                let session_date = regular_session_date(idx);
                RiskGateLedgerRecord {
                    row: RiskGateSessionRow {
                        session_date,
                        shadow_pnl_points: 1.0,
                        shadow_trade_count: 1,
                        rolling_sum_before_session: 0.0,
                        mr_enabled_for_session: false,
                        source: RiskGateRowSource::Seed,
                        status: RiskGateRowStatus::Complete,
                    },
                    profile_id: "imoexf_primary_high180_lb120".to_string(),
                    mr_variant: "high180".to_string(),
                    timeframe: "10m".to_string(),
                    session_policy: "Mon-Fri 09:00..23:49".to_string(),
                    rolling_sum_lb120: 0.0,
                    mr_enabled_next_session: false,
                    model_version: "2026-04-26".to_string(),
                    finalized_at_utc: 1_776_990_000 + idx,
                }
            })
            .collect::<Vec<_>>();

        let state = rebuild_materialized_state_from_ledger_records(
            &records,
            Some(regular_session_date(60)),
            0.0,
            true,
        )
        .expect("state rebuilds from reverse-chronological stream read");

        assert_eq!(
            state.last_finalized_session_date,
            Some(regular_session_date(59))
        );
        assert_eq!(state.rolling_sum_lb120, Some(60.0));
        assert_eq!(state.mr_enabled_current_session, Some(true));
        assert_eq!(state.mr_enabled_next_session, Some(true));
        assert_eq!(state.ledger_rows_count, 60);
        assert_eq!(state.current_generation, RISK_GATE_STATE_GENERATION);
    }

    #[test]
    fn redis_field_parser_rejects_incomplete_ledger_record() {
        let fields = vec![("session_date".to_string(), "2026-04-24".to_string())];

        let err = RiskGateLedgerRecord::from_redis_fields(&fields)
            .expect_err("missing fields are rejected");

        assert!(err.contains("profile_id"));
    }

    #[test]
    fn ledger_records_from_seed_rows_include_next_session_gate() {
        let rows = (0..60)
            .map(|idx| RiskGateSessionRow {
                session_date: regular_session_date(idx),
                shadow_pnl_points: 1.0,
                shadow_trade_count: 1,
                rolling_sum_before_session: 0.0,
                mr_enabled_for_session: false,
                source: RiskGateRowSource::Seed,
                status: RiskGateRowStatus::Complete,
            })
            .collect::<Vec<_>>();

        let records =
            build_ledger_records_from_rows(&rows, &identity(), 1_776_990_000).expect("records");

        assert_eq!(records.len(), 60);
        assert_eq!(records[0].rolling_sum_lb120, 1.0);
        assert!(!records[58].mr_enabled_next_session);
        assert!(records[59].mr_enabled_next_session);
        assert_eq!(records[59].rolling_sum_lb120, 60.0);
    }

    #[test]
    fn startup_plan_imports_seed_when_ledger_is_empty() {
        let seed_rows = (0..60)
            .map(|idx| RiskGateSessionRow {
                session_date: regular_session_date(idx),
                shadow_pnl_points: 1.0,
                shadow_trade_count: 1,
                rolling_sum_before_session: 0.0,
                mr_enabled_for_session: false,
                source: RiskGateRowSource::Seed,
                status: RiskGateRowStatus::Complete,
            })
            .collect::<Vec<_>>();

        let plan = plan_risk_gate_startup(
            RiskGateStartupMode::BootstrapFromSeed,
            &[],
            &seed_rows,
            &identity(),
            Some(regular_session_date(60)),
            1_776_990_000,
        )
        .expect("bootstrap plan");

        assert_eq!(plan.decision, RiskGateStartupDecision::ImportSeed);
        assert_eq!(plan.ledger_records.len(), 60);
        assert_eq!(plan.records_to_write.len(), 60);
        assert_eq!(plan.materialized_state.ledger_rows_count, 60);
        assert_eq!(plan.materialized_state.rolling_sum_lb120, Some(60.0));
        assert_eq!(
            plan.materialized_state.mr_enabled_current_session,
            Some(true)
        );
        assert!(plan.materialized_state.seed_loaded);
    }

    #[test]
    fn startup_plan_uses_existing_ledger_in_normal_append() {
        let seed_rows = (0..60)
            .map(|idx| row(idx as u32 + 1, 1.0))
            .collect::<Vec<_>>();
        let mut seed_rows = seed_rows;
        for (idx, row) in seed_rows.iter_mut().enumerate() {
            row.session_date = regular_session_date(idx as i64);
        }
        let existing_records =
            build_ledger_records_from_rows(&seed_rows, &identity(), 1_776_990_000)
                .expect("existing records");

        let plan = plan_risk_gate_startup(
            RiskGateStartupMode::NormalAppend,
            &existing_records,
            &seed_rows,
            &identity(),
            Some(regular_session_date(60)),
            1_776_990_000,
        )
        .expect("normal append plan");

        assert_eq!(plan.decision, RiskGateStartupDecision::UseExistingLedger);
        assert!(plan.records_to_write.is_empty());
        assert_eq!(plan.ledger_records.len(), existing_records.len());
        assert_eq!(
            plan.materialized_state.ledger_rows_count,
            existing_records.len()
        );
    }

    #[test]
    fn startup_plan_rejects_identity_mismatch() {
        let rows = [RiskGateSessionRow {
            session_date: regular_session_date(0),
            shadow_pnl_points: 1.0,
            shadow_trade_count: 1,
            rolling_sum_before_session: 0.0,
            mr_enabled_for_session: false,
            source: RiskGateRowSource::Seed,
            status: RiskGateRowStatus::Complete,
        }];
        let mut wrong_identity = identity();
        wrong_identity.profile_id = "other_profile".to_string();
        let existing_records = build_ledger_records_from_rows(&rows, &identity(), 1_776_990_000)
            .expect("existing records");

        let err = plan_risk_gate_startup(
            RiskGateStartupMode::NormalAppend,
            &existing_records,
            &rows,
            &wrong_identity,
            Some(regular_session_date(1)),
            1_776_990_000,
        )
        .expect_err("identity mismatch must fail");

        assert!(err.contains("profile_id mismatch"));
    }
}
