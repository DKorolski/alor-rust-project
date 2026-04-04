use anyhow::{anyhow, bail, Result};

use crate::strategies::hybrid_intraday_runtime::HybridIntradayRuntimeStrategy;
use crate::strategies::limit_cancel::LimitCancelStrategy;
use crate::strategies::market_buy_and_close::MarketBuyAndCloseStrategy;
use crate::strategies::mock_live_probe::MockLiveProbeStrategy;
use crate::strategies::session_gap_standalone::SessionGapStandaloneStrategy;
use crate::strategies::toy_session_timing::ToySessionTimingStrategy;
use crate::strategy_host::Strategy;
use crate::{StrategyConfig, StrategyKind};

pub(crate) type BoxedStrategy = Box<dyn Strategy + Send + Sync>;
type StrategyFactoryFn = fn(&StrategyConfig) -> BoxedStrategy;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StrategyCapabilities {
    pub uses_bootstrap_snapshot: bool,
    pub uses_runtime_state_restore: bool,
    pub uses_history_warmup: bool,
    pub uses_stop_orders: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StrategyDescriptor {
    pub kind: StrategyKind,
    pub display_name: &'static str,
    pub factory: StrategyFactoryFn,
    pub capabilities: StrategyCapabilities,
}

#[derive(Debug)]
pub(crate) struct StrategyRegistry {
    descriptors: Vec<StrategyDescriptor>,
}

impl StrategyRegistry {
    pub(crate) fn builtin() -> Result<Self> {
        Self::from_descriptors(vec![
            StrategyDescriptor {
                kind: StrategyKind::LimitCancel,
                display_name: "LimitCancel",
                factory: create_limit_cancel,
                capabilities: StrategyCapabilities::default(),
            },
            StrategyDescriptor {
                kind: StrategyKind::MarketBuyAndClose,
                display_name: "MarketBuyAndClose",
                factory: create_market_buy_and_close,
                capabilities: StrategyCapabilities::default(),
            },
            StrategyDescriptor {
                kind: StrategyKind::ToySessionTiming,
                display_name: "ToySessionTiming",
                factory: create_toy_session_timing,
                capabilities: StrategyCapabilities::default(),
            },
            StrategyDescriptor {
                kind: StrategyKind::SessionGapStandalone,
                display_name: "SessionGapStandalone",
                factory: create_session_gap_standalone,
                capabilities: StrategyCapabilities {
                    uses_bootstrap_snapshot: true,
                    uses_runtime_state_restore: true,
                    uses_history_warmup: true,
                    uses_stop_orders: false,
                },
            },
            StrategyDescriptor {
                kind: StrategyKind::MockLiveProbe,
                display_name: "MockLiveProbe",
                factory: create_mock_live_probe,
                capabilities: StrategyCapabilities::default(),
            },
            StrategyDescriptor {
                kind: StrategyKind::HybridIntraday,
                display_name: "HybridIntraday",
                factory: create_hybrid_intraday,
                capabilities: StrategyCapabilities {
                    uses_bootstrap_snapshot: true,
                    uses_runtime_state_restore: true,
                    uses_history_warmup: true,
                    uses_stop_orders: true,
                },
            },
        ])
    }

    pub(crate) fn from_descriptors(descriptors: Vec<StrategyDescriptor>) -> Result<Self> {
        for (idx, descriptor) in descriptors.iter().enumerate() {
            if descriptors
                .iter()
                .skip(idx + 1)
                .any(|other| other.kind == descriptor.kind)
            {
                bail!(
                    "duplicate strategy descriptor for kind {:?}",
                    descriptor.kind
                );
            }
        }
        Ok(Self { descriptors })
    }

    pub(crate) fn descriptor(&self, kind: StrategyKind) -> Option<&StrategyDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.kind == kind)
    }

    pub(crate) fn create(&self, config: &StrategyConfig) -> Result<BoxedStrategy> {
        let descriptor = self.descriptor(config.strategy_kind).ok_or_else(|| {
            let registered = self
                .descriptors
                .iter()
                .map(|descriptor| descriptor.display_name)
                .collect::<Vec<_>>()
                .join(", ");
            anyhow!(
                "no strategy descriptor registered for kind {:?}; registered descriptors: {}",
                config.strategy_kind,
                registered
            )
        })?;
        Ok((descriptor.factory)(config))
    }

    #[cfg(test)]
    fn registered_kinds(&self) -> Vec<StrategyKind> {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect()
    }
}

fn create_limit_cancel(config: &StrategyConfig) -> BoxedStrategy {
    Box::new(LimitCancelStrategy::new(config.to_limit_cancel_config()))
}

fn create_market_buy_and_close(config: &StrategyConfig) -> BoxedStrategy {
    Box::new(MarketBuyAndCloseStrategy::new(
        config.to_market_buy_and_close_config(),
    ))
}

fn create_toy_session_timing(config: &StrategyConfig) -> BoxedStrategy {
    Box::new(ToySessionTimingStrategy::new(
        config.to_toy_session_timing_config(),
    ))
}

fn create_session_gap_standalone(config: &StrategyConfig) -> BoxedStrategy {
    Box::new(SessionGapStandaloneStrategy::new(
        config.to_session_gap_standalone_config(),
    ))
}

fn create_mock_live_probe(config: &StrategyConfig) -> BoxedStrategy {
    Box::new(MockLiveProbeStrategy::new(
        config.to_mock_live_probe_config(),
    ))
}

fn create_hybrid_intraday(config: &StrategyConfig) -> BoxedStrategy {
    Box::new(HybridIntradayRuntimeStrategy::new(
        config.to_hybrid_intraday_runtime_config(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{StrategyCapabilities, StrategyDescriptor, StrategyRegistry};
    use crate::strategies::market_buy_and_close::MarketBuyAndCloseLiveOrderStyle;
    use crate::{CloseTrigger, HybridIntradaySettings, StrategyConfig, StrategyKind};

    fn sample_strategy_config(kind: StrategyKind) -> StrategyConfig {
        StrategyConfig {
            strategy_id: "test-strategy".to_string(),
            strategy_kind: kind,
            symbol: "SBER".to_string(),
            qty: 1.0,
            side: alor_protocol::Side::Buy,
            live_order_style: MarketBuyAndCloseLiveOrderStyle::Market,
            marketable_limit_offset_ticks: 0,
            place_offset_ticks: 1,
            tick_size: 0.01,
            max_wait_bars_for_ack: 3,
            close_trigger: CloseTrigger::NextBar,
            entry_ack_timeout_ms: 15_000,
            entry_fill_timeout_ms: 60_000,
            exit_ack_timeout_ms: 15_000,
            exit_fill_timeout_ms: 60_000,
            session_open_hour: 10,
            session_open_minute: 0,
            session_close_hour: 23,
            session_close_minute: 50,
            entry_after_open_min: 59,
            exit_before_close_min: 20,
            timezone_offset_hours: 3,
            trading_periods: None,
            max_silence_bars_sec: 0,
            session_gap_k_long: 0.5,
            session_gap_k_short: 0.46,
            session_gap_wait_hours: 2,
            session_gap_k_tp_long: 0.28,
            session_gap_k_sl_long: 0.68,
            session_gap_k_tp_short: 0.28,
            session_gap_k_sl_short: 0.65,
            session_gap_long_ex_pct: 2.2,
            session_gap_short_ex_pct: 2.2,
            session_gap_start_cash: 30_000.0,
            session_gap_cash_factor: 0.9,
            session_gap_max_entry_hour: 19,
            session_gap_close_hour: 23,
            session_gap_close_minute: 49,
            session_gap_min: 60.0,
            session_gap_exit_offset_min: 20,
            session_gap_work_weekends: false,
            hybrid_intraday: HybridIntradaySettings::default(),
        }
    }

    #[test]
    fn builtin_registry_covers_all_existing_strategy_kinds() {
        let registry = StrategyRegistry::builtin().expect("builtin registry");
        assert_eq!(
            registry.registered_kinds(),
            vec![
                StrategyKind::LimitCancel,
                StrategyKind::MarketBuyAndClose,
                StrategyKind::ToySessionTiming,
                StrategyKind::SessionGapStandalone,
                StrategyKind::MockLiveProbe,
                StrategyKind::HybridIntraday,
            ]
        );
    }

    #[test]
    fn registry_rejects_duplicate_strategy_kind_descriptors() {
        let err = StrategyRegistry::from_descriptors(vec![
            StrategyDescriptor {
                kind: StrategyKind::LimitCancel,
                display_name: "LimitCancel",
                factory: super::create_limit_cancel,
                capabilities: StrategyCapabilities::default(),
            },
            StrategyDescriptor {
                kind: StrategyKind::LimitCancel,
                display_name: "LimitCancelDuplicate",
                factory: super::create_limit_cancel,
                capabilities: StrategyCapabilities::default(),
            },
        ])
        .err()
        .expect("duplicate descriptor must fail");

        assert!(err
            .to_string()
            .contains("duplicate strategy descriptor for kind"));
    }

    #[test]
    fn registry_create_errors_when_kind_is_not_registered() {
        let registry = StrategyRegistry::from_descriptors(vec![StrategyDescriptor {
            kind: StrategyKind::LimitCancel,
            display_name: "LimitCancel",
            factory: super::create_limit_cancel,
            capabilities: StrategyCapabilities::default(),
        }])
        .expect("registry");

        let err = registry
            .create(&sample_strategy_config(StrategyKind::HybridIntraday))
            .err()
            .expect("missing descriptor must fail");

        assert!(err
            .to_string()
            .contains("no strategy descriptor registered for kind"));
    }

    #[test]
    fn builtin_registry_creates_every_existing_strategy_kind() {
        let registry = StrategyRegistry::builtin().expect("builtin registry");

        for kind in [
            StrategyKind::LimitCancel,
            StrategyKind::MarketBuyAndClose,
            StrategyKind::ToySessionTiming,
            StrategyKind::SessionGapStandalone,
            StrategyKind::MockLiveProbe,
            StrategyKind::HybridIntraday,
        ] {
            let _strategy = registry
                .create(&sample_strategy_config(kind))
                .expect("strategy must be created");
        }
    }

    #[test]
    fn builtin_registry_exposes_minimal_runtime_capabilities() {
        let registry = StrategyRegistry::builtin().expect("builtin registry");

        let limit_cancel = registry
            .descriptor(StrategyKind::LimitCancel)
            .expect("limit cancel descriptor");
        assert_eq!(limit_cancel.capabilities, StrategyCapabilities::default());

        let session_gap = registry
            .descriptor(StrategyKind::SessionGapStandalone)
            .expect("session gap descriptor");
        assert_eq!(
            session_gap.capabilities,
            StrategyCapabilities {
                uses_bootstrap_snapshot: true,
                uses_runtime_state_restore: true,
                uses_history_warmup: true,
                uses_stop_orders: false,
            }
        );

        let hybrid = registry
            .descriptor(StrategyKind::HybridIntraday)
            .expect("hybrid descriptor");
        assert_eq!(
            hybrid.capabilities,
            StrategyCapabilities {
                uses_bootstrap_snapshot: true,
                uses_runtime_state_restore: true,
                uses_history_warmup: true,
                uses_stop_orders: true,
            }
        );
    }
}
