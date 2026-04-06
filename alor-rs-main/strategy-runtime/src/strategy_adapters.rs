use anyhow::{bail, Result};
use chrono::{Duration, NaiveTime, Timelike};

use crate::strategies::hybrid_intraday::{
    BreakoutEodMode, HybridOrchestratorConfig, IntradayBreakoutConfig, MeanReversionConfig,
    MinRangeMode,
};
use crate::strategies::hybrid_intraday_runtime::{
    HybridIntradayRuntimeConfig, HybridIntradayRuntimeStrategy,
};
use crate::strategies::alor_skeleton::{AlorSkeletonConfig, AlorSkeletonStrategy};
use crate::strategies::session_gap_standalone::{
    SessionGapStandaloneConfig, SessionGapStandaloneStrategy,
};
use crate::strategy_host::Strategy;
use crate::{StrategyConfig, StrategySpecificConfig};

pub(crate) type BoxedStrategy = Box<dyn Strategy + Send + Sync>;

pub(crate) struct SessionGapStandaloneAdapter;

impl SessionGapStandaloneAdapter {
    pub(crate) fn from_strategy_config(config: &StrategyConfig) -> Result<SessionGapStandaloneConfig> {
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
    pub(crate) fn from_strategy_config(config: &StrategyConfig) -> Result<HybridIntradayRuntimeConfig> {
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
        let (session_close_hour, session_close_minute, weekends_off) = config
            .trading_periods
            .as_ref()
            .map(|p| (p.session_end.hour(), p.session_end.minute(), p.weekends_off))
            .unwrap_or((config.session_close_hour, config.session_close_minute, false));
        let mr_session_end_time =
            NaiveTime::parse_from_str(&runtime_settings.mr_session_end_time, "%H:%M:%S")
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(11, 59, 0).unwrap_or(NaiveTime::MIN));
        let bo_min_range_mode = match runtime_settings.bo_min_range_mode.to_ascii_lowercase().as_str() {
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
        Ok(Box::new(HybridIntradayRuntimeStrategy::new(strategy_config)))
    }
}

pub(crate) struct AlorSkeletonAdapter;

impl AlorSkeletonAdapter {
    pub(crate) fn from_strategy_config(config: &StrategyConfig) -> Result<AlorSkeletonConfig> {
        match config.specific() {
            StrategySpecificConfig::AlorSkeleton(_) => Ok(AlorSkeletonConfig {
                symbol: config.symbol.clone(),
            }),
            other => bail!(
                "strategy kind {:?} requires AlorSkeleton payload, found {:?}",
                config.strategy_kind,
                other.kind()
            ),
        }
    }

    pub(crate) fn create(config: &StrategyConfig) -> Result<BoxedStrategy> {
        let strategy_config = Self::from_strategy_config(config)?;
        Ok(Box::new(AlorSkeletonStrategy::new(strategy_config)))
    }
}

#[cfg(test)]
mod tests {
    use super::{AlorSkeletonAdapter, HybridIntradayAdapter, SessionGapStandaloneAdapter};
    use crate::{StrategyConfig, StrategyKind};

    #[test]
    fn session_gap_adapter_rejects_non_matching_payload() {
        let config = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        let err = SessionGapStandaloneAdapter::from_strategy_config(&config)
            .err()
            .expect("expected mismatch error");
        assert!(err
            .to_string()
            .contains("requires SessionGapStandalone payload"));
    }

    #[test]
    fn hybrid_adapter_rejects_non_matching_payload() {
        let config = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        let err = HybridIntradayAdapter::from_strategy_config(&config)
            .err()
            .expect("expected mismatch error");
        assert!(err
            .to_string()
            .contains("requires HybridIntraday payload"));
    }

    #[test]
    fn alor_skeleton_adapter_rejects_non_matching_payload() {
        let config = StrategyConfig::defaults_for_kind(StrategyKind::LimitCancel);
        let err = AlorSkeletonAdapter::from_strategy_config(&config)
            .err()
            .expect("expected mismatch error");
        assert!(err
            .to_string()
            .contains("requires AlorSkeleton payload"));
    }
}
