use std::fs::File;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{FixedOffset, NaiveDate, TimeZone};

use crate::redis_transport::{RedisRuntimeTransport, RedisStreamFieldMessage};
use crate::strategies::hybrid_intraday::{
    build_ledger_records_from_rows, build_runtime_session_row, parse_seed_csv,
    plan_risk_gate_startup, rebuild_materialized_state_from_ledger_records,
    rows_from_ledger_records, validate_ledger_record_identity, RiskGateLedgerRecord,
    RiskGateMaterializedState, RiskGateProfileIdentity, RiskGateRedisKeys, RiskGateRowSource,
    RiskGateSessionRow, RiskGateStartupArtifacts, RiskGateStartupMode,
};
use crate::strategy_host::RiskGateSessionFinalization;
use crate::{StrategyConfig, StrategySpecificConfig};

pub const RISK_GATE_LEDGER_RECOVERY_SCAN_COUNT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskGateWriteSummary {
    pub attempted_records: usize,
    pub inserted_records: usize,
    pub duplicate_records: usize,
    pub state_refreshed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskGateStartupStoreConfig {
    pub identity: RiskGateProfileIdentity,
    pub mode: RiskGateStartupMode,
    pub seed_file: Option<PathBuf>,
    pub current_shadow_session_date: Option<NaiveDate>,
    pub finalized_at_utc: i64,
    pub ledger_scan_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateStartupStoreResult {
    pub existing_records_loaded: usize,
    pub previous_state: Option<RiskGateMaterializedState>,
    pub artifacts: RiskGateStartupArtifacts,
    pub write_summary: RiskGateWriteSummary,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateRuntimeAppendResult {
    pub write_summary: RiskGateWriteSummary,
    pub materialized_state: Option<RiskGateMaterializedState>,
}

pub fn startup_store_config_from_strategy_config(
    strategy: &StrategyConfig,
    finalized_at_utc: i64,
) -> Result<Option<RiskGateStartupStoreConfig>> {
    let StrategySpecificConfig::HybridIntraday(settings) = strategy.specific() else {
        return Ok(None);
    };
    let runtime_settings = &settings.strategy;
    let mode = match parse_startup_mode(&runtime_settings.risk_gate_mode)? {
        Some(mode) => mode,
        None => return Ok(None),
    };
    if !mr_gate_policy_enabled(&runtime_settings.mr_gate_policy)? {
        bail!(
            "risk gate startup mode {:?} requires non-disabled mr_gate_policy",
            mode
        );
    }

    let profile_id = parse_profile_id(&runtime_settings.profile)?;
    let mr_variant = parse_mr_variant(&runtime_settings.mr_variant)?;
    let mut strategy_id = strategy.common.strategy_id.clone();
    if let Some(raw_ledger_key) = runtime_settings.risk_gate_ledger_key.as_deref() {
        let (ledger_strategy_id, ledger_profile_id) = parse_ledger_stream_identity(raw_ledger_key)?;
        if ledger_strategy_id != strategy_id {
            bail!(
                "risk gate ledger stream strategy_id mismatch: config={} ledger_key={}",
                strategy_id,
                ledger_strategy_id
            );
        }
        if ledger_profile_id != profile_id {
            bail!(
                "risk gate ledger stream profile_id mismatch: profile={} ledger_key={}",
                profile_id,
                ledger_profile_id
            );
        }
        strategy_id = ledger_strategy_id;
    }

    let seed_file = runtime_settings
        .risk_gate_seed_file
        .as_ref()
        .map(PathBuf::from);
    if matches!(
        mode,
        RiskGateStartupMode::BootstrapFromSeed | RiskGateStartupMode::RebuildFromHistory
    ) && seed_file.is_none()
    {
        bail!("risk gate mode {:?} requires risk_gate_seed_file", mode);
    }

    let identity = RiskGateProfileIdentity {
        strategy_id,
        profile_id,
        mr_variant,
        timeframe: "10m".to_string(),
        session_policy: session_policy_from_config(
            &runtime_settings.model_session_start_time,
            &runtime_settings.model_session_end_time,
        )?,
        model_version: model_version_from_seed_file(seed_file.as_ref()),
    };

    Ok(Some(RiskGateStartupStoreConfig {
        identity,
        mode,
        seed_file,
        current_shadow_session_date: local_session_date(
            finalized_at_utc,
            strategy.common.timezone_offset_hours,
        ),
        finalized_at_utc,
        ledger_scan_count: RISK_GATE_LEDGER_RECOVERY_SCAN_COUNT,
    }))
}

pub async fn load_risk_gate_ledger_records(
    transport: &RedisRuntimeTransport,
    ledger_stream: &str,
    count: usize,
) -> Result<Vec<RiskGateLedgerRecord>> {
    let messages = transport
        .xrevrange_last_n_fields(ledger_stream, count)
        .await
        .with_context(|| format!("risk gate ledger read failed: {ledger_stream}"))?;
    parse_risk_gate_ledger_messages(&messages)
}

pub async fn load_risk_gate_materialized_state(
    transport: &RedisRuntimeTransport,
    state_key: &str,
) -> Result<Option<RiskGateMaterializedState>> {
    let fields = transport
        .hgetall(state_key)
        .await
        .with_context(|| format!("risk gate state read failed: {state_key}"))?;
    if fields.is_empty() {
        return Ok(None);
    }
    RiskGateMaterializedState::from_redis_fields(&fields)
        .map(Some)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("risk gate state parse failed: {state_key}"))
}

pub async fn run_risk_gate_startup_store(
    transport: &RedisRuntimeTransport,
    config: &RiskGateStartupStoreConfig,
) -> Result<RiskGateStartupStoreResult> {
    let keys = RiskGateRedisKeys::for_profile(
        &config.identity.strategy_id,
        &config.identity.profile_id,
        config.current_shadow_session_date.unwrap_or(NaiveDate::MIN),
    );
    let existing_records = load_risk_gate_ledger_records(
        transport,
        &keys.ledger_stream,
        config.ledger_scan_count.max(1),
    )
    .await?;
    let previous_state = load_risk_gate_materialized_state(transport, &keys.state_key).await?;
    let seed_rows = load_seed_rows(config.seed_file.as_ref())?;
    let artifacts = plan_risk_gate_startup(
        config.mode,
        &existing_records,
        &seed_rows,
        &config.identity,
        config.current_shadow_session_date,
        config.finalized_at_utc,
    )
    .map_err(anyhow::Error::msg)
    .context("risk gate startup plan failed")?;
    let write_summary =
        persist_risk_gate_startup_artifacts(transport, &config.identity, &artifacts).await?;

    Ok(RiskGateStartupStoreResult {
        existing_records_loaded: existing_records.len(),
        previous_state,
        artifacts,
        write_summary,
    })
}

pub async fn persist_risk_gate_startup_artifacts(
    transport: &RedisRuntimeTransport,
    identity: &RiskGateProfileIdentity,
    artifacts: &RiskGateStartupArtifacts,
) -> Result<RiskGateWriteSummary> {
    validate_ledger_record_identity(&artifacts.records_to_write, identity)
        .map_err(anyhow::Error::msg)
        .context("risk gate startup artifact identity validation failed")?;
    let base_keys = RiskGateRedisKeys::for_profile(
        &identity.strategy_id,
        &identity.profile_id,
        artifacts
            .materialized_state
            .last_finalized_session_date
            .unwrap_or(chrono::NaiveDate::MIN),
    );
    let mut inserted_records = 0;
    let mut duplicate_records = 0;
    for record in &artifacts.records_to_write {
        let keys = RiskGateRedisKeys::for_profile(
            &identity.strategy_id,
            &identity.profile_id,
            record.row.session_date,
        );
        let inserted = transport
            .write_risk_gate_session_if_new(
                &keys.finalized_key,
                &keys.ledger_stream,
                &keys.state_key,
                &record.redis_fields(),
                &[],
            )
            .await
            .with_context(|| {
                format!(
                    "risk gate ledger write failed: session_date={}",
                    record.row.session_date
                )
            })?;
        if inserted {
            inserted_records += 1;
        } else {
            duplicate_records += 1;
        }
    }

    transport
        .hset_fields(
            &base_keys.state_key,
            &artifacts.materialized_state.redis_fields(),
        )
        .await
        .with_context(|| format!("risk gate state refresh failed: {}", base_keys.state_key))?;

    Ok(RiskGateWriteSummary {
        attempted_records: artifacts.records_to_write.len(),
        inserted_records,
        duplicate_records,
        state_refreshed: true,
    })
}

pub async fn append_risk_gate_runtime_session(
    transport: &RedisRuntimeTransport,
    identity: &RiskGateProfileIdentity,
    finalization: &RiskGateSessionFinalization,
    finalized_at_utc: i64,
    ledger_scan_count: usize,
) -> Result<RiskGateRuntimeAppendResult> {
    let keys = RiskGateRedisKeys::for_profile(
        &identity.strategy_id,
        &identity.profile_id,
        finalization.session_date,
    );
    let existing_records =
        load_risk_gate_ledger_records(transport, &keys.ledger_stream, ledger_scan_count.max(1))
            .await?;
    validate_ledger_record_identity(&existing_records, identity)
        .map_err(anyhow::Error::msg)
        .context("risk gate runtime append identity validation failed")?;
    let mut rows = rows_from_ledger_records(&existing_records)
        .map_err(anyhow::Error::msg)
        .context("risk gate runtime append ledger rows invalid")?;

    if rows
        .iter()
        .any(|row| row.session_date == finalization.session_date)
    {
        return Ok(RiskGateRuntimeAppendResult {
            write_summary: RiskGateWriteSummary {
                attempted_records: 1,
                inserted_records: 0,
                duplicate_records: 1,
                state_refreshed: false,
            },
            materialized_state: None,
        });
    }

    let runtime_row = build_runtime_session_row(
        &rows,
        finalization.session_date,
        finalization.shadow_pnl_points,
        finalization.shadow_trade_count,
    )
    .map_err(anyhow::Error::msg)
    .context("risk gate runtime row build failed")?;
    rows.push(runtime_row);
    let records = build_ledger_records_from_rows(&rows, identity, finalized_at_utc)
        .map_err(anyhow::Error::msg)
        .context("risk gate runtime record build failed")?;
    let record = records
        .last()
        .cloned()
        .context("risk gate runtime append produced no record")?;
    let seed_loaded = records
        .iter()
        .any(|record| record.row.source == RiskGateRowSource::Seed);
    let materialized_state =
        rebuild_materialized_state_from_ledger_records(&records, None, 0.0, seed_loaded)
            .map_err(anyhow::Error::msg)
            .context("risk gate runtime state rebuild failed")?;

    let inserted = transport
        .write_risk_gate_session_if_new(
            &keys.finalized_key,
            &keys.ledger_stream,
            &keys.state_key,
            &record.redis_fields(),
            &materialized_state.redis_fields(),
        )
        .await
        .with_context(|| {
            format!(
                "risk gate runtime append write failed: session_date={}",
                finalization.session_date
            )
        })?;

    Ok(RiskGateRuntimeAppendResult {
        write_summary: RiskGateWriteSummary {
            attempted_records: 1,
            inserted_records: usize::from(inserted),
            duplicate_records: usize::from(!inserted),
            state_refreshed: inserted,
        },
        materialized_state: inserted.then_some(materialized_state),
    })
}

pub fn parse_risk_gate_ledger_messages(
    messages: &[RedisStreamFieldMessage],
) -> Result<Vec<RiskGateLedgerRecord>> {
    messages
        .iter()
        .map(|message| {
            RiskGateLedgerRecord::from_redis_fields(&message.fields)
                .map_err(anyhow::Error::msg)
                .with_context(|| {
                    format!(
                        "risk gate ledger record parse failed: stream={} id={}",
                        message.stream, message.id
                    )
                })
        })
        .collect()
}

pub fn load_seed_rows(seed_file: Option<&PathBuf>) -> Result<Vec<RiskGateSessionRow>> {
    let Some(seed_file) = seed_file else {
        return Ok(Vec::new());
    };
    let file = File::open(seed_file)
        .with_context(|| format!("risk gate seed open failed: {}", seed_file.display()))?;
    parse_seed_csv(file)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("risk gate seed parse failed: {}", seed_file.display()))
}

fn parse_startup_mode(raw: &str) -> Result<Option<RiskGateStartupMode>> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "disabled" | "none" | "shadow_only" => Ok(None),
        "bootstrap_from_seed" => Ok(Some(RiskGateStartupMode::BootstrapFromSeed)),
        "normal_append" => Ok(Some(RiskGateStartupMode::NormalAppend)),
        "rebuild_from_history" => Ok(Some(RiskGateStartupMode::RebuildFromHistory)),
        "enforced" => Ok(Some(RiskGateStartupMode::NormalAppend)),
        other => bail!("unsupported risk gate startup mode: {other}"),
    }
}

fn mr_gate_policy_enabled(raw: &str) -> Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "disabled" | "none" => Ok(false),
        "shadow_pnl_lb120_positive" | "riskgate_high180_lb120" => Ok(true),
        other => bail!("unsupported mr_gate_policy for risk gate startup: {other}"),
    }
}

fn parse_profile_id(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "imoexf_primary_riskgate"
        | "imoexf_primary_riskgate_high180_lb120"
        | "imoexf_primary_riskgate_k053"
        | "hybrid_mr_riskgate_high180_lb120__bo_new_k053" => {
            Ok("imoexf_primary_high180_lb120".to_string())
        }
        other => bail!("unsupported risk gate profile: {other}"),
    }
}

fn parse_mr_variant(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "high180" => Ok("high180".to_string()),
        other => bail!("unsupported risk gate mr_variant for startup: {other}"),
    }
}

fn parse_ledger_stream_identity(raw: &str) -> Result<(String, String)> {
    let prefix = "runtime.riskgate.sessions.";
    let trimmed = raw.trim();
    let suffix = trimmed
        .strip_prefix(prefix)
        .ok_or_else(|| anyhow!("invalid risk gate ledger stream key: {trimmed}"))?;
    let mut parts = suffix.splitn(2, '.');
    let strategy_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("invalid risk gate ledger stream key: {trimmed}"))?;
    let profile_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("invalid risk gate ledger stream key: {trimmed}"))?;
    Ok((strategy_id.to_string(), profile_id.to_string()))
}

fn session_policy_from_config(start: &str, end: &str) -> Result<String> {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() && end.is_empty() {
        return Ok("Mon-Fri 09:00..23:49".to_string());
    }
    let start = if start.is_empty() { "09:00:00" } else { start };
    let end = if end.is_empty() { "23:49:59" } else { end };
    let start = chrono::NaiveTime::parse_from_str(start, "%H:%M:%S")
        .map_err(|err| anyhow!("invalid model_session_start_time: {start}: {err}"))?;
    let end = chrono::NaiveTime::parse_from_str(end, "%H:%M:%S")
        .map_err(|err| anyhow!("invalid model_session_end_time: {end}: {err}"))?;
    Ok(format!(
        "Mon-Fri {}..{}",
        start.format("%H:%M"),
        end.format("%H:%M")
    ))
}

fn model_version_from_seed_file(seed_file: Option<&PathBuf>) -> String {
    seed_file
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "runtime-ledger-v1".to_string())
}

fn local_session_date(finalized_at_utc: i64, timezone_offset_hours: i32) -> Option<NaiveDate> {
    let offset = FixedOffset::east_opt(timezone_offset_hours.saturating_mul(3600))?;
    offset
        .timestamp_opt(finalized_at_utc, 0)
        .single()
        .map(|dt| dt.naive_local().date())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    use chrono::NaiveDate;
    use tempfile::NamedTempFile;

    use crate::redis_transport::RedisStreamFieldMessage;
    use crate::strategies::hybrid_intraday::{
        build_ledger_records_from_rows, RiskGateProfileIdentity, RiskGateRowSource,
        RiskGateRowStatus, RiskGateSessionRow,
    };
    use crate::{StrategyConfig, StrategyKind};

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

    fn row(date: &str, pnl: f64) -> RiskGateSessionRow {
        RiskGateSessionRow {
            session_date: NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("date"),
            shadow_pnl_points: pnl,
            shadow_trade_count: u32::from(pnl != 0.0),
            rolling_sum_before_session: 0.0,
            mr_enabled_for_session: false,
            source: RiskGateRowSource::Seed,
            status: RiskGateRowStatus::Complete,
        }
    }

    #[test]
    fn parses_field_stream_messages_into_ledger_records() {
        let records = build_ledger_records_from_rows(
            &[row("2026-04-23", 1.0), row("2026-04-24", -2.5)],
            &identity(),
            1_776_990_000,
        )
        .expect("records");
        let messages = records
            .iter()
            .enumerate()
            .map(|(idx, record)| RedisStreamFieldMessage {
                stream: "runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120"
                    .to_string(),
                id: format!("{idx}-0"),
                fields: record.redis_fields(),
            })
            .collect::<Vec<_>>();

        let parsed = parse_risk_gate_ledger_messages(&messages).expect("parsed messages");

        assert_eq!(parsed, records);
    }

    #[test]
    fn parse_errors_include_stream_and_id() {
        let messages = [RedisStreamFieldMessage {
            stream: "riskgate.stream".to_string(),
            id: "42-0".to_string(),
            fields: vec![("session_date".to_string(), "2026-04-24".to_string())],
        }];

        let err = parse_risk_gate_ledger_messages(&messages).expect_err("parse fails");
        let err = format!("{err:#}");

        assert!(err.contains("riskgate.stream"));
        assert!(err.contains("42-0"));
        assert!(err.contains("profile_id"));
    }

    #[test]
    fn load_seed_rows_returns_empty_without_seed_file() {
        let rows = load_seed_rows(None).expect("empty seed");

        assert!(rows.is_empty());
    }

    #[test]
    fn load_seed_rows_reads_seed_csv() {
        let mut file = NamedTempFile::new().expect("temp file");
        writeln!(
            file,
            "date,shadow_pnl_points,shadow_trade_count,rolling_120_pnl_before_session,mr_enabled_for_session,source,status"
        )
        .expect("write header");
        writeln!(file, "2026-04-23,1.5,1,0.0,false,seed,complete").expect("write row");

        let rows = load_seed_rows(Some(&file.path().to_path_buf())).expect("seed rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_date, row("2026-04-23", 1.5).session_date);
        assert_eq!(rows[0].shadow_pnl_points, 1.5);
    }

    #[test]
    fn startup_config_from_strategy_builds_identity_and_mode() {
        let mut strategy = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        strategy.common.strategy_id = "hybrid_imoexf".to_string();
        strategy.common.timezone_offset_hours = 3;
        let settings = strategy.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.profile = "imoexf_primary_riskgate_k053".to_string();
        settings.strategy.mr_variant = "high180".to_string();
        settings.strategy.mr_gate_policy = "shadow_pnl_lb120_positive".to_string();
        settings.strategy.risk_gate_mode = "bootstrap_from_seed".to_string();
        settings.strategy.risk_gate_seed_file =
            Some("docs/seeds/riskgate_lb120_seed_2026_04_26.csv".to_string());
        settings.strategy.model_session_start_time = "09:00:00".to_string();
        settings.strategy.model_session_end_time = "23:49:59".to_string();

        let config = startup_store_config_from_strategy_config(&strategy, 1_777_000_000)
            .expect("startup config")
            .expect("enabled");

        assert_eq!(config.mode, RiskGateStartupMode::BootstrapFromSeed);
        assert_eq!(config.identity.strategy_id, "hybrid_imoexf");
        assert_eq!(config.identity.profile_id, "imoexf_primary_high180_lb120");
        assert_eq!(config.identity.mr_variant, "high180");
        assert_eq!(config.identity.timeframe, "10m");
        assert_eq!(config.identity.session_policy, "Mon-Fri 09:00..23:49");
        assert_eq!(
            config.identity.model_version,
            "riskgate_lb120_seed_2026_04_26"
        );
        assert_eq!(
            config.ledger_scan_count,
            RISK_GATE_LEDGER_RECOVERY_SCAN_COUNT
        );
        assert!(config.seed_file.is_some());
        assert!(config.current_shadow_session_date.is_some());
    }

    #[test]
    fn startup_config_from_strategy_returns_none_for_disabled_mode() {
        let strategy = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);

        let config = startup_store_config_from_strategy_config(&strategy, 1_777_000_000)
            .expect("startup config");

        assert!(config.is_none());
    }

    #[test]
    fn startup_config_from_strategy_maps_enforced_to_existing_ledger_mode() {
        let mut strategy = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = strategy.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.profile = "imoexf_primary_riskgate".to_string();
        settings.strategy.mr_variant = "high180".to_string();
        settings.strategy.mr_gate_policy = "shadow_pnl_lb120_positive".to_string();
        settings.strategy.risk_gate_mode = "enforced".to_string();

        let config = startup_store_config_from_strategy_config(&strategy, 1_777_000_000)
            .expect("startup config")
            .expect("enabled");

        assert_eq!(config.mode, RiskGateStartupMode::NormalAppend);
    }

    #[test]
    fn startup_config_from_strategy_requires_seed_for_bootstrap() {
        let mut strategy = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = strategy.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.profile = "imoexf_primary_riskgate".to_string();
        settings.strategy.mr_variant = "high180".to_string();
        settings.strategy.mr_gate_policy = "shadow_pnl_lb120_positive".to_string();
        settings.strategy.risk_gate_mode = "bootstrap_from_seed".to_string();
        settings.strategy.risk_gate_seed_file = None;

        let err =
            startup_store_config_from_strategy_config(&strategy, 1_777_000_000).expect_err("err");
        assert!(err.to_string().contains("requires risk_gate_seed_file"));
    }

    #[test]
    fn startup_config_from_strategy_validates_ledger_key_identity() {
        let mut strategy = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        strategy.common.strategy_id = "hybrid_imoexf".to_string();
        let settings = strategy.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.profile = "imoexf_primary_riskgate".to_string();
        settings.strategy.mr_variant = "high180".to_string();
        settings.strategy.mr_gate_policy = "shadow_pnl_lb120_positive".to_string();
        settings.strategy.risk_gate_mode = "normal_append".to_string();
        settings.strategy.risk_gate_ledger_key =
            Some("runtime.riskgate.sessions.hybrid_imoexf.some_other_profile".to_string());

        let err =
            startup_store_config_from_strategy_config(&strategy, 1_777_000_000).expect_err("err");
        assert!(err.to_string().contains("profile_id mismatch"));
    }
}
