use alor_protocol::CommandAck;

use crate::state::StrategyState;
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, Intent, OrderEvent, PositionEvent, RuntimeStateRestored, Strategy,
    StrategyCtx, StopOrderEvent,
};

#[derive(Debug, Clone)]
pub struct AlorSkeletonConfig {
    pub symbol: String,
}

#[derive(Debug)]
pub struct AlorSkeletonStrategy {
    config: AlorSkeletonConfig,
    state: StrategyState,
}

impl AlorSkeletonStrategy {
    pub fn new(config: AlorSkeletonConfig) -> Self {
        Self {
            config,
            state: StrategyState::AlorSkeleton {
                lifecycle_stage: "created".to_string(),
                last_bar_ts: None,
                bootstrap_seen: false,
                runtime_state_restored: false,
            },
        }
    }
}

impl Strategy for AlorSkeletonStrategy {
    fn on_bar(&mut self, _ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        if bar.symbol != self.config.symbol {
            return Vec::new();
        }
        if let StrategyState::AlorSkeleton {
            lifecycle_stage,
            last_bar_ts,
            ..
        } = &mut self.state
        {
            *lifecycle_stage = "live".to_string();
            *last_bar_ts = Some(bar.close_time_utc);
        }
        Vec::new()
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &CommandAck) -> Vec<Intent> {
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_stop_order(&mut self, _ctx: &StrategyCtx, _ord: &StopOrderEvent) -> Vec<Intent> {
        if let StrategyState::AlorSkeleton {
            lifecycle_stage, ..
        } = &mut self.state
        {
            *lifecycle_stage = "stop_order_observed".to_string();
        }
        Vec::new()
    }

    fn on_position(&mut self, _ctx: &StrategyCtx, _pos: &PositionEvent) -> Vec<Intent> {
        Vec::new()
    }

    fn on_bootstrap_snapshot(
        &mut self,
        _ctx: &StrategyCtx,
        _snapshot: &BootstrapSnapshot,
    ) -> Vec<Intent> {
        if let StrategyState::AlorSkeleton {
            lifecycle_stage,
            bootstrap_seen,
            ..
        } = &mut self.state
        {
            *lifecycle_stage = "bootstrapped".to_string();
            *bootstrap_seen = true;
        }
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        _state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        if let StrategyState::AlorSkeleton {
            lifecycle_stage,
            runtime_state_restored,
            ..
        } = &mut self.state
        {
            *lifecycle_stage = "runtime_state_restored".to_string();
            *runtime_state_restored = true;
        }
        Vec::new()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        self.state = state;
    }
}
