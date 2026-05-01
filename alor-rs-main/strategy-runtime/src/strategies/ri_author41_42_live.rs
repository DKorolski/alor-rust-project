use anyhow::{bail, Result};
use chrono::{NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state::StrategyState;
use crate::strategies::moex_author41_42::{ModelProfile, RegularSessionPolicy};
use crate::strategy_host::{
    BarEvent, BootstrapSnapshot, DataOrigin, Intent, OrderEvent, PositionEvent,
    RuntimeStateRestored, Strategy, StrategyCtx,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142RuntimeMode {
    Shadow,
    DryRun,
    MicroLive,
}

impl RiAuthor4142RuntimeMode {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "shadow" | "shadow_only" => Ok(Self::Shadow),
            "dry_run" | "dryrun" => Ok(Self::DryRun),
            "micro_live" | "live_micro" => Ok(Self::MicroLive),
            other => bail!("unsupported ri_author41_42 mode: {other}"),
        }
    }

    pub fn can_emit_orders(self) -> bool {
        matches!(self, Self::MicroLive)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::DryRun => "dry_run",
            Self::MicroLive => "micro_live",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiAuthor4142ExecutionPath {
    ActionScopedOnly,
}

impl RiAuthor4142ExecutionPath {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "action_scoped_only" | "action-scoped-only" => Ok(Self::ActionScopedOnly),
            other => bail!("unsupported ri_author41_42 execution_path: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActionScopedOnly => "action_scoped_only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiAuthor4142LiveConfig {
    pub symbol: String,
    pub profile_id: String,
    pub timeframe: String,
    pub mode: RiAuthor4142RuntimeMode,
    pub allow_order_emission: bool,
    pub execution_path: RiAuthor4142ExecutionPath,
    pub qty: f64,
    pub timezone_offset_hours: i32,
}

impl RiAuthor4142LiveConfig {
    pub fn validate_pre_go(&self) -> Result<()> {
        if self.execution_path != RiAuthor4142ExecutionPath::ActionScopedOnly {
            bail!("ri_author41_42 requires action_scoped_only execution");
        }
        if self.allow_order_emission {
            bail!("ri_author41_42 live order emission is blocked until GO/NO-GO is approved");
        }
        if self.mode.can_emit_orders() {
            bail!("ri_author41_42 micro_live mode is blocked until GO/NO-GO is approved");
        }
        if self.timeframe.trim() != "10m" {
            bail!(
                "ri_author41_42 frozen model requires 10m timeframe, got {}",
                self.timeframe
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RiAuthor4142LiveStrategy {
    config: RiAuthor4142LiveConfig,
    session_policy: RegularSessionPolicy,
    state: StrategyState,
}

impl RiAuthor4142LiveStrategy {
    pub fn new(config: RiAuthor4142LiveConfig) -> Result<Self> {
        config.validate_pre_go()?;
        let profile = ModelProfile::ri_shadow_10m();
        let session_policy = profile.session_policy;
        Ok(Self {
            state: StrategyState::RiAuthor4142Live {
                mode: config.mode.as_str().to_string(),
                profile_id: config.profile_id.clone(),
                timeframe: config.timeframe.clone(),
                allow_order_emission: config.allow_order_emission,
                execution_path: config.execution_path.as_str().to_string(),
                last_bar_ts: None,
                last_model_bar_ts: None,
                model_bars_seen: 0,
                suppressed_service_bars: 0,
                live_adapter_enabled: false,
            },
            config,
            session_policy,
        })
    }

    fn local_dt(&self, ts_utc: i64) -> Option<NaiveDateTime> {
        chrono::DateTime::from_timestamp(ts_utc, 0).map(|dt| {
            dt.naive_utc() + chrono::Duration::hours(self.config.timezone_offset_hours.into())
        })
    }

    fn update_bar_state(&mut self, bar: &BarEvent) {
        let Some(dt_local) = self.local_dt(bar.close_time_utc) else {
            return;
        };
        let is_model_bar =
            self.session_policy.is_model_bar(dt_local) && bar.origin != DataOrigin::HistoryGap;

        if let StrategyState::RiAuthor4142Live {
            last_bar_ts,
            last_model_bar_ts,
            model_bars_seen,
            suppressed_service_bars,
            ..
        } = &mut self.state
        {
            *last_bar_ts = Some(bar.close_time_utc);
            if is_model_bar {
                *last_model_bar_ts = Some(bar.close_time_utc);
                *model_bars_seen = model_bars_seen.saturating_add(1);
            } else {
                *suppressed_service_bars = suppressed_service_bars.saturating_add(1);
            }
        }

        info!(
            target: "strategy_runtime::ri_author41_42_live",
            action = "ri_model_bar_observed",
            ts_utc = bar.close_time_utc,
            dt_local = %dt_local,
            hour = dt_local.time().hour(),
            minute = dt_local.time().minute(),
            is_model_bar,
            mode = self.config.mode.as_str(),
            allow_order_emission = self.config.allow_order_emission,
            live_adapter_enabled = false,
        );
    }

    pub fn can_emit_orders(&self) -> bool {
        self.config.mode.can_emit_orders() && self.config.allow_order_emission
    }
}

impl Strategy for RiAuthor4142LiveStrategy {
    fn on_bar(&mut self, _ctx: &StrategyCtx, bar: &BarEvent) -> Vec<Intent> {
        self.update_bar_state(bar);
        Vec::new()
    }

    fn on_ack(&mut self, _ctx: &StrategyCtx, _ack: &alor_protocol::CommandAck) -> Vec<Intent> {
        Vec::new()
    }

    fn on_order(&mut self, _ctx: &StrategyCtx, _ord: &OrderEvent) -> Vec<Intent> {
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
        Vec::new()
    }

    fn on_runtime_state_restored(
        &mut self,
        _ctx: &StrategyCtx,
        state: &RuntimeStateRestored,
    ) -> Vec<Intent> {
        let _ = state;
        Vec::new()
    }

    fn warmup_from_history(&mut self, _ctx: &StrategyCtx, bars: &[BarEvent]) -> usize {
        for bar in bars {
            self.update_bar_state(bar);
        }
        bars.len()
    }

    fn state(&self) -> &StrategyState {
        &self.state
    }

    fn set_state(&mut self, state: StrategyState) {
        if let StrategyState::RiAuthor4142Live { .. } = state {
            self.state = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RiAuthor4142ExecutionPath, RiAuthor4142LiveConfig, RiAuthor4142LiveStrategy,
        RiAuthor4142RuntimeMode,
    };

    fn default_config() -> RiAuthor4142LiveConfig {
        RiAuthor4142LiveConfig {
            symbol: "RIM6".to_string(),
            profile_id: "ri_author41_42_primary_combo_cost2".to_string(),
            timeframe: "10m".to_string(),
            mode: RiAuthor4142RuntimeMode::Shadow,
            allow_order_emission: false,
            execution_path: RiAuthor4142ExecutionPath::ActionScopedOnly,
            qty: 1.0,
            timezone_offset_hours: 3,
        }
    }

    #[test]
    fn shadow_mode_scaffold_cannot_emit_orders() {
        let strategy = RiAuthor4142LiveStrategy::new(default_config()).expect("strategy");
        assert!(!strategy.can_emit_orders());
    }

    #[test]
    fn allow_order_emission_is_blocked_until_go() {
        let mut config = default_config();
        config.allow_order_emission = true;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("order emission must remain blocked")
            .to_string();
        assert!(err.contains("blocked until GO/NO-GO"));
    }

    #[test]
    fn micro_live_mode_is_blocked_until_go() {
        let mut config = default_config();
        config.mode = RiAuthor4142RuntimeMode::MicroLive;

        let err = RiAuthor4142LiveStrategy::new(config)
            .expect_err("micro_live must remain blocked")
            .to_string();
        assert!(err.contains("micro_live mode is blocked"));
    }
}
