use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use super::intraday_breakout::IntradayBreakoutEngine;
use super::mean_reversion::MeanReversionEngine;
use super::types::{Action, EntrySignal, Owner, ReasonCode, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HybridState {
    Flat,
    Pending,
    Open,
}

#[derive(Debug, Clone, Copy)]
pub enum BreakoutEodMode {
    SameDay,
    Overnight,
}

#[derive(Debug, Clone, Copy)]
pub struct HybridOrchestratorConfig {
    pub breakout_eod_mode: BreakoutEodMode,
    pub breakout_overnight_exit_time: NaiveTime,
}

impl Default for HybridOrchestratorConfig {
    fn default() -> Self {
        Self {
            breakout_eod_mode: BreakoutEodMode::SameDay,
            breakout_overnight_exit_time: NaiveTime::from_hms_opt(9, 30, 0)
                .unwrap_or(NaiveTime::MIN),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HybridSnapshot {
    pub state: HybridState,
    pub current_owner: Option<Owner>,
    pub current_side: Option<Side>,
    pub has_pending_entry: bool,
    pub overnight_exit_armed_date: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct HybridOrchestrator {
    pub mean_reversion: MeanReversionEngine,
    pub intraday_breakout: IntradayBreakoutEngine,
    pub config: HybridOrchestratorConfig,
    state: HybridState,
    current_owner: Option<Owner>,
    current_side: Option<Side>,
    pending_entry: Option<EntrySignal>,
    overnight_exit_armed_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy)]
pub struct BarInput {
    pub dt: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub close_prev: f64,
    pub day_range_prev: f64,
    pub has_open_position: bool,
    pub has_live_orders: bool,
}

impl HybridOrchestrator {
    pub fn new(
        mean_reversion: MeanReversionEngine,
        intraday_breakout: IntradayBreakoutEngine,
        config: HybridOrchestratorConfig,
    ) -> Self {
        Self {
            mean_reversion,
            intraday_breakout,
            config,
            state: HybridState::Flat,
            current_owner: None,
            current_side: None,
            pending_entry: None,
            overnight_exit_armed_date: None,
        }
    }

    pub fn reset(&mut self) {
        self.state = HybridState::Flat;
        self.current_owner = None;
        self.current_side = None;
        self.pending_entry = None;
        self.overnight_exit_armed_date = None;
    }

    pub fn snapshot(&self) -> HybridSnapshot {
        HybridSnapshot {
            state: self.state,
            current_owner: self.current_owner,
            current_side: self.current_side,
            has_pending_entry: self.pending_entry.is_some(),
            overnight_exit_armed_date: self.overnight_exit_armed_date,
        }
    }

    pub fn restore(&mut self, snapshot: HybridSnapshot) {
        self.state = snapshot.state;
        self.current_owner = snapshot.current_owner;
        self.current_side = snapshot.current_side;
        self.pending_entry = None;
        self.overnight_exit_armed_date = snapshot.overnight_exit_armed_date;
    }

    pub fn on_bar(&mut self, bar: BarInput) -> Vec<Action> {
        let mr_exit_reason = if bar.has_open_position
            && self.current_owner == Some(Owner::MeanReversion)
            && self.mean_reversion.should_force_exit(bar.dt)
        {
            Some(ReasonCode::MeanRevTimeCutoff)
        } else {
            None
        };
        let mr_entry_signal = self.mean_reversion.evaluate_entry(
            bar.dt,
            bar.close,
            bar.close_prev,
            bar.day_range_prev,
        );
        self.on_bar_with_mr_override(bar, mr_entry_signal, mr_exit_reason)
    }

    pub fn on_bar_with_mr_override(
        &mut self,
        bar: BarInput,
        mr_entry_signal: Option<EntrySignal>,
        mr_exit_reason: Option<ReasonCode>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        self.intraday_breakout
            .on_bar(bar.dt, bar.open, bar.high, bar.low, bar.close);

        if bar.has_open_position
            && self.current_owner == Some(Owner::IntradayBreakout)
            && matches!(self.config.breakout_eod_mode, BreakoutEodMode::Overnight)
            && self.should_trigger_overnight_exit(bar.dt)
        {
            self.overnight_exit_armed_date = None;
            actions.push(Action::SubmitExit {
                owner: Owner::IntradayBreakout,
                reason: ReasonCode::WaitfixOvernightExit,
            });
            return actions;
        }

        if bar.has_open_position && self.current_owner == Some(Owner::MeanReversion) {
            let Some(reason) = mr_exit_reason else {
                return actions;
            };
            actions.push(Action::SubmitExit {
                owner: Owner::MeanReversion,
                reason,
            });
            return actions;
        }

        if bar.has_open_position && self.current_owner == Some(Owner::IntradayBreakout) {
            if let Some(side) = self.current_side {
                if let Some(exit_signal) = self
                    .intraday_breakout
                    .evaluate_exit(bar.dt, bar.close, side)
                {
                    if exit_signal.reason == ReasonCode::BreakoutEodExit
                        && matches!(self.config.breakout_eod_mode, BreakoutEodMode::Overnight)
                    {
                        self.overnight_exit_armed_date = Some(bar.dt.date());
                        actions.push(Action::ArmOvernightExit {
                            owner: Owner::IntradayBreakout,
                            reason: ReasonCode::BreakoutEodExit,
                            armed_date: bar.dt.date(),
                            exit_time: self.config.breakout_overnight_exit_time,
                        });
                        return actions;
                    }
                    actions.push(Action::SubmitExit {
                        owner: Owner::IntradayBreakout,
                        reason: exit_signal.reason,
                    });
                    return actions;
                }
            }
        }

        if bar.has_open_position || bar.has_live_orders || self.state == HybridState::Pending {
            return actions;
        }

        if let Some(signal) = mr_entry_signal {
            return self.emit_entry_action(signal);
        }

        if let Some(signal) = self.intraday_breakout.evaluate_entry(bar.dt, bar.close) {
            return self.emit_entry_action(signal);
        }

        actions
    }

    pub fn warm_bar(&mut self, bar: BarInput) {
        self.intraday_breakout
            .on_bar(bar.dt, bar.open, bar.high, bar.low, bar.close);
    }

    pub fn on_order_filled(&mut self, role: &str, owner: Owner, side: Option<Side>) {
        if role == "entry" {
            self.state = HybridState::Open;
            self.current_owner = Some(owner);
            self.current_side = side;
            self.pending_entry = None;
            if owner == Owner::IntradayBreakout {
                if let Some(entry_side) = side {
                    self.intraday_breakout.mark_entry(entry_side);
                }
            }
            return;
        }

        if matches!(role, "exit" | "stop" | "take") {
            self.state = HybridState::Flat;
            self.current_owner = None;
            self.current_side = None;
            self.pending_entry = None;
            self.overnight_exit_armed_date = None;
        }
    }

    pub fn on_order_rejected(&mut self, role: &str) {
        if role == "entry" {
            self.state = HybridState::Flat;
            self.current_owner = None;
            self.current_side = None;
            self.pending_entry = None;
        }
    }

    fn should_trigger_overnight_exit(&self, dt: NaiveDateTime) -> bool {
        let Some(armed_date) = self.overnight_exit_armed_date else {
            return false;
        };
        if dt.date() <= armed_date {
            return false;
        }
        let t = self.config.breakout_overnight_exit_time;
        dt.hour() > t.hour() || (dt.hour() == t.hour() && dt.minute() >= t.minute())
    }

    fn emit_entry_action(&mut self, signal: EntrySignal) -> Vec<Action> {
        self.state = HybridState::Pending;
        self.pending_entry = Some(signal.clone());
        self.current_owner = Some(signal.owner);
        self.current_side = Some(signal.side);
        vec![Action::SubmitEntry(signal)]
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};

    use super::*;
    use crate::strategies::hybrid_intraday::{
        IntradayBreakoutConfig, MeanReversionConfig, MinRangeMode,
    };

    fn dt(y: i32, mo: u32, d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap_or(NaiveDate::MIN)
            .and_hms_opt(h, m, 0)
            .unwrap_or(NaiveDateTime::MIN)
    }

    #[test]
    fn prefers_mean_reversion_signal_before_breakout() {
        let mr = MeanReversionEngine::new(MeanReversionConfig::default());
        let br = IntradayBreakoutEngine::new(IntradayBreakoutConfig {
            min_range_mode: MinRangeMode::Disabled,
            wait_hours: 0.0,
            ..IntradayBreakoutConfig::default()
        });
        let mut orch = HybridOrchestrator::new(mr, br, HybridOrchestratorConfig::default());
        let actions = orch.on_bar(BarInput {
            dt: dt(2026, 1, 5, 10, 0),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 101.95,
            close_prev: 102.0,
            day_range_prev: 2.0,
            has_open_position: false,
            has_live_orders: false,
        });
        assert!(!actions.is_empty());
        match &actions[0] {
            Action::SubmitEntry(entry) => assert_eq!(entry.owner, Owner::MeanReversion),
            _ => panic!("expected submit_entry"),
        }
    }

    #[test]
    fn overnight_mode_arms_exit_instead_of_immediate_exit() {
        let mr = MeanReversionEngine::new(MeanReversionConfig::default());
        let mut br = IntradayBreakoutEngine::new(IntradayBreakoutConfig::default());
        br.on_bar(dt(2026, 1, 5, 9, 0), 100.0, 101.0, 99.0, 100.0);
        br.on_bar(dt(2026, 1, 6, 9, 0), 100.0, 103.0, 98.0, 102.0);
        let mut orch = HybridOrchestrator::new(
            mr,
            br,
            HybridOrchestratorConfig {
                breakout_eod_mode: BreakoutEodMode::Overnight,
                breakout_overnight_exit_time: NaiveTime::from_hms_opt(9, 30, 0)
                    .unwrap_or(NaiveTime::MIN),
            },
        );
        orch.current_owner = Some(Owner::IntradayBreakout);
        orch.current_side = Some(Side::Long);
        orch.state = HybridState::Open;
        let actions = orch.on_bar(BarInput {
            dt: dt(2026, 1, 6, 23, 30),
            open: 101.0,
            high: 102.0,
            low: 100.0,
            close: 101.5,
            close_prev: 100.0,
            day_range_prev: 2.0,
            has_open_position: true,
            has_live_orders: false,
        });
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::ArmOvernightExit { reason, .. } => {
                assert_eq!(*reason, ReasonCode::BreakoutEodExit);
            }
            _ => panic!("expected arm overnight action"),
        }
    }
}
