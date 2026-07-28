use anyhow::{bail, Result};
use chrono::{Duration, NaiveTime, Timelike};

use crate::strategies::alor_usdrubf_hybrid::{AlorUsdrubfHybridConfig, AlorUsdrubfHybridStrategy};
use crate::strategies::hybrid_intraday::{
    BreakoutEodMode, HybridOrchestratorConfig, IntradayBreakoutConfig, MeanReversionConfig,
    MinRangeMode,
};
use crate::strategies::hybrid_intraday_runtime::{
    HybridIntradayProfile, HybridIntradayRuntimeConfig, HybridIntradayRuntimeStrategy,
    MeanReversionVariant, MrGatePolicy, RiskGateMode,
};
use crate::strategies::ri_author41_42_live::{
    RiAuthor4142ExecutionPath, RiAuthor4142LiveConfig, RiAuthor4142LiveStrategy,
    RiAuthor4142RuntimeMode,
};
use crate::strategies::session_gap_standalone::{
    SessionGapStandaloneConfig, SessionGapStandaloneStrategy,
};
use crate::strategy_host::Strategy;
use crate::{StrategyConfig, StrategySpecificConfig};

pub(crate) type BoxedStrategy = Box<dyn Strategy + Send + Sync>;

pub(crate) struct SessionGapStandaloneAdapter;

impl SessionGapStandaloneAdapter {
    pub(crate) fn from_strategy_config(
        config: &StrategyConfig,
    ) -> Result<SessionGapStandaloneConfig> {
        let settings = match config.specific() {
            StrategySpecificConfig::SessionGapStandalone(settings) => settings,
            other => {
                bail!(
                    "strategy kind {:?} requires SessionGapStandalone payload, found {:?}",
                    config.strategy_kind,
                    other.kind()
                )
            }
        };
        Ok(SessionGapStandaloneConfig {
            symbol: config.symbol.clone(),
            timezone_offset_hours: config.timezone_offset_hours,
            place_offset_ticks: settings.place_offset_ticks,
            tick_size: config.tick_size,
            close_hour: settings.close_hour,
            close_minute: settings.close_minute,
            entry_ack_timeout_ms: settings.entry_ack_timeout_ms,
            entry_fill_timeout_ms: settings.entry_fill_timeout_ms,
            exit_ack_timeout_ms: settings.exit_ack_timeout_ms,
            exit_fill_timeout_ms: settings.exit_fill_timeout_ms,
            signal_minute: settings.signal_minute,
            k_long: settings.k_long,
            k_short: settings.k_short,
            wait_hours: settings.wait_hours,
            k_tp_long: settings.k_tp_long,
            k_sl_long: settings.k_sl_long,
            k_tp_short: settings.k_tp_short,
            k_sl_short: settings.k_sl_short,
            long_ex_pct: settings.long_ex_pct,
            short_ex_pct: settings.short_ex_pct,
            session_gap_min: settings.session_gap_min,
            exit_offset_min: settings.exit_offset_min,
            work_weekends: settings.work_weekends,
            cash_factor: settings.cash_factor,
            start_cash: settings.start_cash,
            max_entry_hour: settings.max_entry_hour,
        })
    }

    pub(crate) fn create(config: &StrategyConfig) -> Result<BoxedStrategy> {
        let strategy_config = Self::from_strategy_config(config)?;
        Ok(Box::new(SessionGapStandaloneStrategy::new(strategy_config)))
    }
}

pub(crate) struct HybridIntradayAdapter;

impl HybridIntradayAdapter {
    fn parse_profile(raw: &str) -> Result<HybridIntradayProfile> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "baseline" | "baseline_runtime_hybrid" => {
                Ok(HybridIntradayProfile::BaselineRuntimeHybrid)
            }
            "imoexf_primary_riskgate"
            | "imoexf_primary_riskgate_high180_lb120"
            | "imoexf_primary_riskgate_k053"
            | "hybrid_mr_riskgate_high180_lb120__bo_new_k053" => {
                Ok(HybridIntradayProfile::ImoexfPrimaryRiskgateHigh180Lb120)
            }
            other => bail!("unsupported hybrid_intraday profile: {other}"),
        }
    }

    fn parse_mr_variant(raw: &str) -> Result<MeanReversionVariant> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "classic" | "classic_prev_day_range" | "primary" => {
                Ok(MeanReversionVariant::ClassicPrevDayRange)
            }
            "high180" => Ok(MeanReversionVariant::High180),
            "author41_boundary_short" | "author41_short" => {
                Ok(MeanReversionVariant::Author41BoundaryShort)
            }
            other => bail!("unsupported hybrid_intraday mr_variant: {other}"),
        }
    }

    fn parse_mr_gate_policy(raw: &str) -> Result<MrGatePolicy> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "disabled" | "none" => Ok(MrGatePolicy::Disabled),
            "shadow_pnl_lb120_positive" | "riskgate_high180_lb120" => {
                Ok(MrGatePolicy::ShadowPnlLb120Positive)
            }
            other => bail!("unsupported hybrid_intraday mr_gate_policy: {other}"),
        }
    }

    fn parse_risk_gate_mode(raw: &str) -> Result<RiskGateMode> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "disabled" | "none" => Ok(RiskGateMode::Disabled),
            "bootstrap_from_seed" => Ok(RiskGateMode::BootstrapFromSeed),
            "normal_append" => Ok(RiskGateMode::NormalAppend),
            "rebuild_from_history" => Ok(RiskGateMode::RebuildFromHistory),
            "shadow_only" => Ok(RiskGateMode::ShadowOnly),
            "enforced" => Ok(RiskGateMode::Enforced),
            other => bail!("unsupported hybrid_intraday risk_gate_mode: {other}"),
        }
    }

    fn parse_optional_time(raw: &str, field: &str) -> Result<Option<NaiveTime>> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        NaiveTime::parse_from_str(raw, "%H:%M:%S")
            .map(Some)
            .map_err(|err| anyhow::anyhow!("invalid {field}: {raw}: {err}"))
    }

    pub(crate) fn from_strategy_config(
        config: &StrategyConfig,
    ) -> Result<HybridIntradayRuntimeConfig> {
        let settings = match config.specific() {
            StrategySpecificConfig::HybridIntraday(settings) => settings,
            other => {
                bail!(
                    "strategy kind {:?} requires HybridIntraday payload, found {:?}",
                    config.strategy_kind,
                    other.kind()
                )
            }
        };
        let runtime_settings = &settings.strategy;
        let profile = Self::parse_profile(&runtime_settings.profile)?;
        let mr_variant = Self::parse_mr_variant(&runtime_settings.mr_variant)?;
        let mr_gate_policy = Self::parse_mr_gate_policy(&runtime_settings.mr_gate_policy)?;
        let risk_gate_mode = Self::parse_risk_gate_mode(&runtime_settings.risk_gate_mode)?;
        if mr_gate_policy == MrGatePolicy::Disabled && risk_gate_mode != RiskGateMode::Disabled {
            bail!(
                "hybrid_intraday risk_gate_mode {:?} requires non-disabled mr_gate_policy",
                risk_gate_mode
            );
        }
        let (session_close_hour, session_close_minute, weekends_off) = config
            .trading_periods
            .as_ref()
            .map(|p| (p.session_end.hour(), p.session_end.minute(), p.weekends_off))
            .unwrap_or((
                config.session_close_hour,
                config.session_close_minute,
                false,
            ));
        let mr_session_end_time =
            NaiveTime::parse_from_str(&runtime_settings.mr_session_end_time, "%H:%M:%S")
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(11, 59, 0).unwrap_or(NaiveTime::MIN));
        let bo_min_range_mode = match runtime_settings
            .bo_min_range_mode
            .to_ascii_lowercase()
            .as_str()
        {
            "disabled" => MinRangeMode::Disabled,
            "relative_prev_close" | "relativeprevclose" => MinRangeMode::RelativePrevClose,
            _ => MinRangeMode::Absolute,
        };
        let breakout_eod_mode = match runtime_settings
            .orchestrator_breakout_eod_mode
            .to_ascii_lowercase()
            .as_str()
        {
            "overnight" => BreakoutEodMode::Overnight,
            _ => BreakoutEodMode::SameDay,
        };
        let breakout_overnight_exit_time = NaiveTime::parse_from_str(
            &runtime_settings.orchestrator_breakout_overnight_exit_time,
            "%H:%M:%S",
        )
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(9, 30, 0).unwrap_or(NaiveTime::MIN));

        Ok(HybridIntradayRuntimeConfig {
            symbol: config.symbol.clone(),
            profile,
            mr_variant,
            mr_gate_policy,
            risk_gate_mode,
            risk_gate_seed_file: runtime_settings.risk_gate_seed_file.clone(),
            risk_gate_ledger_key: runtime_settings.risk_gate_ledger_key.clone(),
            model_session_start_time: Self::parse_optional_time(
                &runtime_settings.model_session_start_time,
                "model_session_start_time",
            )?,
            model_session_end_time: Self::parse_optional_time(
                &runtime_settings.model_session_end_time,
                "model_session_end_time",
            )?,
            qty: config.qty.max(1.0),
            live_order_style: settings.live_order_style,
            tick_size: config.tick_size,
            marketable_limit_offset_ticks: settings.marketable_limit_offset_ticks,
            timezone_offset_hours: config.timezone_offset_hours,
            session_close_hour,
            session_close_minute,
            weekends_off,
            stop_end_buffer_sec: runtime_settings.stop_end_buffer_sec.max(1),
            repair_deadline_sec: runtime_settings.repair_deadline_sec.max(1),
            sl_escalate_timeout_sec: runtime_settings.sl_escalate_timeout_sec.max(1),
            max_repair_retries: runtime_settings.max_repair_retries.max(1),
            repair_backoff_base_sec: runtime_settings.repair_backoff_base_sec.max(1),
            repair_backoff_max_sec: runtime_settings
                .repair_backoff_max_sec
                .max(runtime_settings.repair_backoff_base_sec.max(1)),
            pending_timeout_sec: runtime_settings.pending_timeout_sec.max(1),
            partial_entry_fill_timeout_ms: runtime_settings.partial_entry_fill_timeout_ms.max(1),
            mr_config: MeanReversionConfig {
                min_range_long: runtime_settings.mr_min_range_long,
                max_range_long: runtime_settings.mr_max_range_long,
                k_long: runtime_settings.mr_k_long,
                take_k_long: runtime_settings.mr_take_k_long,
                stop_k_long: runtime_settings.mr_stop_k_long,
                min_range_short: runtime_settings.mr_min_range_short,
                max_range_short: runtime_settings.mr_max_range_short,
                k_short: runtime_settings.mr_k_short,
                take_k_short: runtime_settings.mr_take_k_short,
                stop_k_short: runtime_settings.mr_stop_k_short,
                tick_size: config.tick_size,
                session_end_time: mr_session_end_time,
                exit_offset: Duration::minutes(runtime_settings.mr_exit_offset_min.max(0)),
            },
            breakout_config: IntradayBreakoutConfig {
                k: runtime_settings.bo_k,
                stop1_range: runtime_settings.bo_stop1_range,
                stop2_range: runtime_settings.bo_stop2_range,
                big_move_threshold: runtime_settings.bo_big_move_threshold,
                min_range: runtime_settings.bo_min_range,
                min_range_mode: bo_min_range_mode,
                exclude_weekends: runtime_settings.bo_exclude_weekends,
                wait_hours: runtime_settings.bo_wait_hours,
            },
            orchestrator_config: HybridOrchestratorConfig {
                breakout_eod_mode,
                breakout_overnight_exit_time,
            },
        })
    }

    pub(crate) fn create(config: &StrategyConfig) -> Result<BoxedStrategy> {
        let strategy_config = Self::from_strategy_config(config)?;
        Ok(Box::new(HybridIntradayRuntimeStrategy::new(
            strategy_config,
        )))
    }
}

pub(crate) struct RiAuthor4142Adapter;

impl RiAuthor4142Adapter {
    pub(crate) fn from_strategy_config(config: &StrategyConfig) -> Result<RiAuthor4142LiveConfig> {
        let settings = match config.specific() {
            StrategySpecificConfig::RiAuthor4142(settings) => settings,
            other => {
                bail!(
                    "strategy kind {:?} requires RiAuthor4142 payload, found {:?}",
                    config.strategy_kind,
                    other.kind()
                )
            }
        };

        Ok(RiAuthor4142LiveConfig {
            symbol: config.symbol.clone(),
            profile_id: settings.profile_id.clone(),
            timeframe: settings.timeframe.clone(),
            mode: RiAuthor4142RuntimeMode::parse(&settings.mode)?,
            allow_order_emission: settings.allow_order_emission,
            execution_path: RiAuthor4142ExecutionPath::parse(&settings.execution_path)?,
            order_symbol: settings
                .order_symbol
                .as_ref()
                .map(|symbol| symbol.trim())
                .filter(|symbol| !symbol.is_empty())
                .map(ToString::to_string),
            excluded_model_dates: settings.excluded_model_dates.clone(),
            min_anchor_bars: settings.min_anchor_bars,
            anchor_first_bar_at_or_before: settings.anchor_first_bar_at_or_before.clone(),
            anchor_last_bar_at_or_after: settings.anchor_last_bar_at_or_after.clone(),
            anchor_transition_date: settings.anchor_transition_date.clone(),
            pre_transition_min_anchor_bars: settings.pre_transition_min_anchor_bars,
            pre_transition_anchor_first_bar_at_or_before: settings
                .pre_transition_anchor_first_bar_at_or_before
                .clone(),
            pre_transition_anchor_last_bar_at_or_after: settings
                .pre_transition_anchor_last_bar_at_or_after
                .clone(),
            actual_expiry_date: settings.actual_expiry_date.clone(),
            roll_target_sessions_before: settings.roll_target_sessions_before,
            roll_fallback_sessions_before: settings.roll_fallback_sessions_before,
            qty: config.qty.max(1.0),
            timezone_offset_hours: config.timezone_offset_hours,
        })
    }

    pub(crate) fn create(config: &StrategyConfig) -> Result<BoxedStrategy> {
        let strategy_config = Self::from_strategy_config(config)?;
        Ok(Box::new(RiAuthor4142LiveStrategy::new(strategy_config)?))
    }
}

pub(crate) struct AlorUsdrubfHybridAdapter;

impl AlorUsdrubfHybridAdapter {
    pub(crate) fn from_strategy_config(config: &StrategyConfig) -> Result<AlorUsdrubfHybridConfig> {
        match config.specific() {
            StrategySpecificConfig::AlorUsdrubfHybrid(settings) => Ok(AlorUsdrubfHybridConfig {
                symbol: config.symbol.clone(),
                timezone_offset_hours: config.timezone_offset_hours,
                model_session_start_time: NaiveTime::parse_from_str(
                    &settings.model_session_start_time,
                    "%H:%M:%S",
                )
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN)),
                model_session_end_time: NaiveTime::parse_from_str(
                    &settings.model_session_end_time,
                    "%H:%M:%S",
                )
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(23, 49, 59).unwrap_or(NaiveTime::MIN)),
                mr_min_rel_range: settings.mr_min_rel_range,
                mr_max_rel_range: settings.mr_max_rel_range,
                mr_k_short: settings.mr_k_short,
                mr_take_k_short: settings.mr_take_k_short,
                mr_stop_k_short: settings.mr_stop_k_short,
                mr_last_entry_time: NaiveTime::parse_from_str(
                    &settings.mr_last_entry_time,
                    "%H:%M:%S",
                )
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(11, 40, 0).unwrap_or(NaiveTime::MIN)),
                mr_force_exit_time: NaiveTime::parse_from_str(
                    &settings.mr_force_exit_time,
                    "%H:%M:%S",
                )
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(11, 50, 0).unwrap_or(NaiveTime::MIN)),
                bo_k: settings.bo_k,
                bo_stop1_range: settings.bo_stop1_range,
                bo_stop2_range: settings.bo_stop2_range,
                bo_big_move_threshold: settings.bo_big_move_threshold,
                bo_wait_hours: settings.bo_wait_hours,
                bo_eod_exit_time: NaiveTime::parse_from_str(&settings.bo_eod_exit_time, "%H:%M:%S")
                    .unwrap_or_else(|_| {
                        NaiveTime::from_hms_opt(23, 30, 0).unwrap_or(NaiveTime::MIN)
                    }),
                commission_pct_per_side: settings.commission_pct_per_side,
                position_size_fraction: settings.position_size_fraction,
                initial_cash: settings.initial_cash,
                enable_live_execution: settings.enable_live_execution,
                use_fixed_live_size: settings.use_fixed_live_size,
                live_fixed_units: settings.live_fixed_units,
                max_silence_bars_sec: config.max_silence_bars_sec,
                tick_size: config.tick_size,
            }),
            other => bail!(
                "strategy kind {:?} requires AlorUsdrubfHybrid payload, found {:?}",
                config.strategy_kind,
                other.kind()
            ),
        }
    }

    pub(crate) fn create(config: &StrategyConfig) -> Result<BoxedStrategy> {
        let strategy_config = Self::from_strategy_config(config)?;
        Ok(Box::new(AlorUsdrubfHybridStrategy::new(strategy_config)))
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveTime;

    use super::{AlorUsdrubfHybridAdapter, HybridIntradayAdapter, SessionGapStandaloneAdapter};
    use crate::strategies::hybrid_intraday_runtime::{
        HybridIntradayProfile, MeanReversionVariant, MrGatePolicy, RiskGateMode,
    };
    use crate::{StrategyConfig, StrategyKind};

    #[test]
    fn session_gap_adapter_rejects_non_matching_payload() {
        let config = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        let err = SessionGapStandaloneAdapter::from_strategy_config(&config)
            .expect_err("expected mismatch error");
        assert!(err
            .to_string()
            .contains("requires SessionGapStandalone payload"));
    }

    #[test]
    fn hybrid_adapter_rejects_non_matching_payload() {
        let config = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        let err = HybridIntradayAdapter::from_strategy_config(&config)
            .expect_err("expected mismatch error");
        assert!(err.to_string().contains("requires HybridIntraday payload"));
    }

    #[test]
    fn hybrid_adapter_parses_profile_and_model_session_guard() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = config.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.profile = "imoexf_primary_riskgate_k053".to_string();
        settings.strategy.model_session_start_time = "09:00:00".to_string();
        settings.strategy.model_session_end_time = "23:49:59".to_string();

        let runtime_config =
            HybridIntradayAdapter::from_strategy_config(&config).expect("hybrid runtime config");

        assert_eq!(
            runtime_config.profile,
            HybridIntradayProfile::ImoexfPrimaryRiskgateHigh180Lb120
        );
        assert_eq!(
            runtime_config.mr_variant,
            MeanReversionVariant::ClassicPrevDayRange
        );
        assert_eq!(runtime_config.mr_gate_policy, MrGatePolicy::Disabled);
        assert_eq!(runtime_config.risk_gate_mode, RiskGateMode::Disabled);
        assert_eq!(
            runtime_config.model_session_start_time,
            Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN))
        );
        assert_eq!(
            runtime_config.model_session_end_time,
            Some(NaiveTime::from_hms_opt(23, 49, 59).unwrap_or(NaiveTime::MIN))
        );
    }

    #[test]
    fn hybrid_adapter_accepts_high180_without_active_risk_gate() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = config.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.mr_variant = "high180".to_string();

        let runtime_config =
            HybridIntradayAdapter::from_strategy_config(&config).expect("hybrid runtime config");

        assert_eq!(runtime_config.mr_variant, MeanReversionVariant::High180);
    }

    #[test]
    fn hybrid_adapter_accepts_author41_boundary_short_variant() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = config.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.mr_variant = "author41_boundary_short".to_string();

        let runtime_config =
            HybridIntradayAdapter::from_strategy_config(&config).expect("hybrid runtime config");

        assert_eq!(
            runtime_config.mr_variant,
            MeanReversionVariant::Author41BoundaryShort
        );
    }

    #[test]
    fn hybrid_adapter_allows_shadow_risk_gate_bootstrap_modes() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = config.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.mr_gate_policy = "shadow_pnl_lb120_positive".to_string();
        settings.strategy.risk_gate_mode = "bootstrap_from_seed".to_string();
        settings.strategy.mr_variant = "high180".to_string();
        settings.strategy.profile = "imoexf_primary_riskgate".to_string();
        settings.strategy.risk_gate_seed_file = Some("docs/risk_gate_seed.csv".to_string());

        let runtime_config =
            HybridIntradayAdapter::from_strategy_config(&config).expect("runtime config");
        assert_eq!(
            runtime_config.mr_gate_policy,
            MrGatePolicy::ShadowPnlLb120Positive
        );
        assert_eq!(
            runtime_config.risk_gate_mode,
            RiskGateMode::BootstrapFromSeed
        );
    }

    #[test]
    fn hybrid_adapter_allows_enforced_risk_gate_mode() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = config.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.profile = "imoexf_primary_riskgate".to_string();
        settings.strategy.mr_variant = "high180".to_string();
        settings.strategy.mr_gate_policy = "shadow_pnl_lb120_positive".to_string();
        settings.strategy.risk_gate_mode = "enforced".to_string();

        let runtime_config =
            HybridIntradayAdapter::from_strategy_config(&config).expect("runtime config");
        assert_eq!(runtime_config.risk_gate_mode, RiskGateMode::Enforced);
    }

    #[test]
    fn hybrid_adapter_rejects_risk_gate_mode_without_policy() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::HybridIntraday);
        let settings = config.hybrid_intraday_mut().expect("hybrid settings");
        settings.strategy.mr_gate_policy = "disabled".to_string();
        settings.strategy.risk_gate_mode = "normal_append".to_string();

        let err = HybridIntradayAdapter::from_strategy_config(&config).expect_err("invalid combo");
        assert!(err
            .to_string()
            .contains("requires non-disabled mr_gate_policy"));
    }

    #[test]
    fn alor_usdrubf_hybrid_adapter_rejects_non_matching_payload() {
        let config = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        let err = AlorUsdrubfHybridAdapter::from_strategy_config(&config)
            .expect_err("expected mismatch error");
        assert!(err
            .to_string()
            .contains("requires AlorUsdrubfHybrid payload"));
    }

    #[test]
    fn alor_usdrubf_hybrid_adapter_propagates_model_session_window() {
        let mut config = StrategyConfig::defaults_for_kind(StrategyKind::AlorUsdrubfHybrid);
        let settings = config
            .alor_usdrubf_hybrid_mut()
            .expect("alor-usdrubf hybrid settings");
        settings.model_session_start_time = "07:00:00".to_string();
        settings.model_session_end_time = "23:49:59".to_string();

        let runtime_config =
            AlorUsdrubfHybridAdapter::from_strategy_config(&config).expect("runtime config");
        assert_eq!(
            runtime_config.model_session_start_time,
            NaiveTime::from_hms_opt(7, 0, 0).unwrap_or(NaiveTime::MIN)
        );
        assert_eq!(
            runtime_config.model_session_end_time,
            NaiveTime::from_hms_opt(23, 49, 59).unwrap_or(NaiveTime::MIN)
        );
    }
}
