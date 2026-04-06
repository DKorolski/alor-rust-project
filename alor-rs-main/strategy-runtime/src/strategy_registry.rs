use anyhow::{anyhow, bail, Result};
use crate::strategy_adapters::{AlorUsdrubfHybridAdapter, HybridIntradayAdapter, SessionGapStandaloneAdapter};
use crate::strategies::limit_cancel::{LimitCancelConfig, LimitCancelStrategy};
use crate::strategies::market_buy_and_close::{
    MarketBuyAndCloseConfig, MarketBuyAndCloseStrategy,
};
use crate::strategies::mock_live_probe::{
    MockLiveProbeConfig, MockLiveProbeMode, MockLiveProbeStrategy,
};
use crate::strategies::toy_session_timing::{ToySessionTimingConfig, ToySessionTimingStrategy};
use crate::strategy_host::Strategy;
use crate::{StrategyConfig, StrategyKind, StrategySpecificConfig};

pub(crate) type BoxedStrategy = Box<dyn Strategy + Send + Sync>;
type StrategyFactoryFn = fn(&StrategyConfig) -> Result<BoxedStrategy>;

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
            StrategyDescriptor {
                kind: StrategyKind::AlorUsdrubfHybrid,
                display_name: "AlorUsdrubfHybrid",
                factory: create_alor_usdrubf_hybrid,
                capabilities: StrategyCapabilities {
                    uses_bootstrap_snapshot: true,
                    uses_runtime_state_restore: true,
                    uses_history_warmup: true,
                    uses_stop_orders: false,
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
        (descriptor.factory)(config)
    }

    #[cfg(test)]
    fn registered_kinds(&self) -> Vec<StrategyKind> {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect()
    }
}

fn create_limit_cancel(config: &StrategyConfig) -> Result<BoxedStrategy> {
    let settings = match config.specific() {
        StrategySpecificConfig::LimitCancel(settings) => settings,
        other => {
            bail!(
                "strategy kind {:?} requires LimitCancel payload, found {:?}",
                config.strategy_kind,
                other.kind()
            )
        }
    };
    Ok(Box::new(LimitCancelStrategy::new(LimitCancelConfig {
        symbol: config.symbol.clone(),
        tick_size: config.tick_size,
        offset_ticks: settings.place_offset_ticks,
        qty: config.qty,
        side: config.side,
        max_wait_bars_for_ack: settings.max_wait_bars_for_ack,
    })))
}

fn create_market_buy_and_close(config: &StrategyConfig) -> Result<BoxedStrategy> {
    let settings = match config.specific() {
        StrategySpecificConfig::MarketBuyAndClose(settings) => settings,
        other => {
            bail!(
                "strategy kind {:?} requires MarketBuyAndClose payload, found {:?}",
                config.strategy_kind,
                other.kind()
            )
        }
    };
    Ok(Box::new(MarketBuyAndCloseStrategy::new(
        MarketBuyAndCloseConfig {
            symbol: config.symbol.clone(),
            qty: config.qty,
            side: config.side,
            live_order_style: settings.live_order_style,
            tick_size: config.tick_size,
            marketable_limit_offset_ticks: settings.marketable_limit_offset_ticks,
            close_trigger: settings.close_trigger,
            entry_ack_timeout_ms: settings.entry_ack_timeout_ms,
            entry_fill_timeout_ms: settings.entry_fill_timeout_ms,
            exit_ack_timeout_ms: settings.exit_ack_timeout_ms,
            exit_fill_timeout_ms: settings.exit_fill_timeout_ms,
        },
    )))
}

fn create_toy_session_timing(config: &StrategyConfig) -> Result<BoxedStrategy> {
    match config.specific() {
        StrategySpecificConfig::ToySessionTiming(_) => {}
        other => {
            bail!(
                "strategy kind {:?} requires ToySessionTiming payload, found {:?}",
                config.strategy_kind,
                other.kind()
            )
        }
    }
    Ok(Box::new(ToySessionTimingStrategy::new(
        ToySessionTimingConfig {
            symbol: config.symbol.clone(),
            qty: config.qty,
            entry_side: config.side,
            session_open_hour: config.session_open_hour,
            session_open_minute: config.session_open_minute,
            session_close_hour: config.session_close_hour,
            session_close_minute: config.session_close_minute,
            entry_after_open_min: config.entry_after_open_min,
            exit_before_close_min: config.exit_before_close_min,
            timezone_offset_hours: config.timezone_offset_hours,
        },
    )))
}

fn create_session_gap_standalone(config: &StrategyConfig) -> Result<BoxedStrategy> {
    SessionGapStandaloneAdapter::create(config)
}

fn create_mock_live_probe(config: &StrategyConfig) -> Result<BoxedStrategy> {
    let settings = match config.specific() {
        StrategySpecificConfig::MockLiveProbe(settings) => settings,
        other => {
            bail!(
                "strategy kind {:?} requires MockLiveProbe payload, found {:?}",
                config.strategy_kind,
                other.kind()
            )
        }
    };
    Ok(Box::new(MockLiveProbeStrategy::new(MockLiveProbeConfig {
        symbol: config.symbol.clone(),
        qty: config.qty,
        side: config.side,
        tick_size: config.tick_size,
        offset_ticks: settings.place_offset_ticks,
        trigger_after_live_bars: settings.max_wait_bars_for_ack.max(1),
        mode: MockLiveProbeMode::parse(&config.strategy_id),
    })))
}

fn create_hybrid_intraday(config: &StrategyConfig) -> Result<BoxedStrategy> {
    HybridIntradayAdapter::create(config)
}

fn create_alor_usdrubf_hybrid(config: &StrategyConfig) -> Result<BoxedStrategy> {
    AlorUsdrubfHybridAdapter::create(config)
}

#[cfg(test)]
mod tests {
    use super::{StrategyCapabilities, StrategyDescriptor, StrategyRegistry};
    use crate::StrategyKind;
    use crate::StrategyConfig;

    fn sample_strategy_config(kind: StrategyKind) -> StrategyConfig {
        let mut config = StrategyConfig::defaults_for_kind(kind);
        config.strategy_id = "test-strategy".to_string();
        config.symbol = "SBER".to_string();
        config
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
                StrategyKind::AlorUsdrubfHybrid,
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
        .expect_err("duplicate descriptor must fail");

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
            StrategyKind::AlorUsdrubfHybrid,
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

        let alor = registry
            .descriptor(StrategyKind::AlorUsdrubfHybrid)
            .expect("alor skeleton descriptor");
        assert_eq!(
            alor.capabilities,
            StrategyCapabilities {
                uses_bootstrap_snapshot: true,
                uses_runtime_state_restore: true,
                uses_history_warmup: true,
                uses_stop_orders: false,
            }
        );
    }

    #[test]
    fn alor_usdrubf_capabilities_match_followup_hardening_profile() {
        let registry = StrategyRegistry::builtin().expect("builtin registry");
        let alor = registry
            .descriptor(StrategyKind::AlorUsdrubfHybrid)
            .expect("alor descriptor");

        assert!(alor.capabilities.uses_bootstrap_snapshot);
        assert!(alor.capabilities.uses_runtime_state_restore);
        assert!(alor.capabilities.uses_history_warmup);
        assert!(!alor.capabilities.uses_stop_orders);
    }
}
