//! Numerical-only Author41/BO053 A/A and A/B reaction kernel. No Strategy,
//! broker/Redis/HTTP route, brackets or operational admission capability.
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::shadow_calendar::ContextPolicy;
use crate::shadow_component_context::{Arm, Evidence, FrozenComponents, LevelInputs};
use crate::{BarEvent, DataOrigin};

const VERSION: &str = "imoexf_component_reaction_v2";
const MAX_CHECKPOINT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoGate {
    Off,
    /// Synthetic fixed-day diagnostic, never a real PIT certificate or grant.
    FrozenNumerical {
        allow_new_bo: bool,
    },
    Observed {
        binding: ObservedGate,
    },
}

/// Only the crate's verified session bridge mints this binding. It is never
/// deserialized from a numerical checkpoint as a permission token.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ObservedGate {
    context_sha256: String,
    decision_sha256: String,
    allow_new_bo: bool,
}
impl BoGate {
    fn allows(&self) -> bool {
        match self {
            Self::Off => true,
            Self::FrozenNumerical { allow_new_bo } => *allow_new_bo,
            Self::Observed { binding } => binding.allow_new_bo,
        }
    }

    pub(crate) fn observed(
        context: &FrozenComponents,
        allow_new_bo: bool,
        decision_sha256: String,
    ) -> Result<Self> {
        ensure!(
            context.request.evidence == Evidence::DeclaredAsObserved
                && context.request.arm == Arm::ImoAbOi
                && decision_sha256.len() == 64
                && decision_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "observed gate identity"
        );
        Ok(Self::Observed {
            binding: ObservedGate {
                context_sha256: hash(context)?,
                decision_sha256,
                allow_new_bo,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Owner {
    Mr,
    Bo,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Long,
    Short,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Action {
    pub owner: Owner,
    pub side: Side,
    pub signal_ts_utc: i64,
    pub at_utc: i64,
    pub exit_reason: Option<&'static str>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Position {
    pub entry: Action,
    pub price: f64,
    pub bars_held: usize,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Event {
    pub role: &'static str,
    pub ts_utc: i64,
    pub component: Owner,
    pub side: Side,
    pub signal_ts_utc: Option<i64>,
    pub price: Option<f64>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Trade {
    pub component: Owner,
    pub side: Side,
    pub entry_ts_utc: i64,
    pub entry_signal_ts_utc: i64,
    pub entry_price: f64,
    pub bars_held: usize,
    pub exit_ts_utc: i64,
    pub exit_signal_ts_utc: i64,
    pub exit_price: f64,
    pub exit_reason: &'static str,
    pub gross_points: f64,
    pub friction_rub: f64,
    pub net_x1_rub: f64,
    pub net_x2_rub: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct State {
    pub observed_bars: usize,
    pub last_label_utc: Option<i64>,
    pub mr_entries: usize,
    pub used_long: bool,
    pub used_short: bool,
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    pub first_high: Option<f64>,
    pub first_low: Option<f64>,
    pub bo_long_level: Option<f64>,
    pub bo_short_level: Option<f64>,
    pub author_allowed: bool,
    pub position: Option<Position>,
    pub pending: Option<Action>,
    pub decision_site: &'static str,
    pub mr_predicate: bool,
    pub bo_long_predicate: bool,
    pub bo_short_predicate: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    bar: BarEvent,
    observed_at_utc: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: String,
    context_sha256: String,
    gate: Value,
    inputs: Vec<Input>,
    witness: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: Payload,
    sha256: String,
}

#[derive(Clone)]
pub struct Engine {
    context: FrozenComponents,
    context_sha256: String,
    gate: BoGate,
    state: State,
    trades: Vec<Trade>,
    events: Vec<Event>,
    inputs: Vec<Input>,
}

fn local(ts: i64) -> Result<NaiveDateTime> {
    DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(Duration::hours(3)))
        .context("invalid numerical timestamp")
}
fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn valid(c: &LevelInputs) -> bool {
    (c.previous_close + c.previous_range + c.previous_low).is_finite()
        && c.previous_close > 0.
        && c.previous_range > 0.
        && c.previous_low > 0.
}
fn leg_cost(price: f64) -> f64 {
    1. + price * 10. * 0.000066 + 0.5 * 10.
}

impl Engine {
    pub fn new(context: FrozenComponents, gate: BoGate) -> Result<Self> {
        ensure!(
            context.dependency_ready && context.mr.selection.ready && context.bo.selection.ready,
            "both component dependencies required"
        );
        let mr = context.mr.inputs.as_ref().context("missing MR features")?;
        let bo = context.bo.inputs.as_ref().context("missing BO features")?;
        ensure!(
            [mr, bo].iter().all(|c| [
                c.previous_close,
                c.previous_high,
                c.previous_low,
                c.previous_range,
                c.previous2_close,
                c.previous_return,
                c.entry_scale,
                c.stop_scale
            ]
            .iter()
            .all(|v| v.is_finite())
                && c.previous_close > 0.
                && c.previous_low > 0.
                && c.previous2_close > 0.
                && c.previous_range >= 0.),
            "invalid component features"
        );
        let start = local(context.cutoff_utc)?;
        let close = local(context.session_close_utc)?;
        ensure!(
            context.symbol == "IMOEXF"
                && start.date() == context.session_date
                && start.date().weekday().num_days_from_monday() < 5
                && matches!(start.hour(), 7 | 9)
                && start.minute() == 0
                && start.second() == 0,
            "unsupported numerical session"
        );
        ensure!(
            close.date() == context.session_date
                && (close.hour(), close.minute(), close.second()) == (23, 50, 0),
            "shortened/break execution requires a separately matched contract"
        );
        let expected: Vec<_> = (context.cutoff_utc..context.session_close_utc)
            .step_by(600)
            .collect();
        ensure!(
            expected == context.expected_execution_labels_utc,
            "shortened/break execution requires a separately matched contract"
        );
        ensure!(
            context.mr.selection.policy == ContextPolicy::RegularOnly
                && context.bo.selection.policy
                    == if context.request.arm == Arm::ImoAa {
                        ContextPolicy::RegularOnly
                    } else {
                        ContextPolicy::RegularAndDsvd
                    },
            "arm/component policy mismatch"
        );
        match &gate {
            BoGate::Off => ensure!(
                context.request.evidence != Evidence::DeclaredAsObserved
                    || context.request.arm != Arm::ImoAbOi,
                "observed OI arm requires a frozen decision"
            ),
            BoGate::FrozenNumerical { .. } => ensure!(
                context.request.arm == Arm::ImoAbOi
                    && context.request.evidence == Evidence::NumericalFixture,
                "numerical OI gate is not operational evidence"
            ),
            BoGate::Observed { binding } => ensure!(
                context.request.arm == Arm::ImoAbOi
                    && context.request.evidence == Evidence::DeclaredAsObserved
                    && binding.context_sha256 == hash(&context)?,
                "observed OI gate/context mismatch"
            ),
        }
        let author_allowed = valid(bo)
            && bo.previous_return.is_finite()
            && bo.previous_high / bo.previous_low > 1.01
            && start.weekday().num_days_from_monday() != 4
            && !((start.month() == 5 && start.day() >= 21)
                || start.month() == 6
                || (start.month() == 7 && start.day() == 1));
        Ok(Self {
            context_sha256: hash(&context)?,
            context,
            gate,
            state: State {
                observed_bars: 0,
                last_label_utc: None,
                mr_entries: 0,
                used_long: false,
                used_short: false,
                day_high: None,
                day_low: None,
                first_high: None,
                first_low: None,
                bo_long_level: None,
                bo_short_level: None,
                author_allowed,
                position: None,
                pending: None,
                decision_site: "not_evaluated",
                mr_predicate: false,
                bo_long_predicate: false,
                bo_short_predicate: false,
            },
            trades: vec![],
            events: vec![],
            inputs: vec![],
        })
    }

    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn trades(&self) -> &[Trade] {
        &self.trades
    }
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Numerical completion only, not an operational clean-EOD certificate.
    pub fn is_complete(&self) -> bool {
        self.state.observed_bars == self.context.expected_execution_labels_utc.len()
            && self.state.last_label_utc == Some(self.context.session_close_utc - 600)
            && self.state.position.is_none()
            && self.state.pending.is_none()
    }

    pub fn observe(&mut self, bar: BarEvent, observed_at_utc: i64) -> Result<()> {
        let label = local(bar.close_time_utc)?;
        ensure!(
            bar.origin == DataOrigin::Replay && bar.symbol == "IMOEXF",
            "only explicit IMOEXF numerical replay inputs"
        );
        ensure!(
            bar.close_time_utc == self.context.cutoff_utc + self.inputs.len() as i64 * 600
                && label.date() == self.context.session_date
                && (label.hour(), label.minute()) <= (23, 40),
            "nonconsecutive, duplicate or out-of-session bar"
        );
        ensure!(
            observed_at_utc >= bar.close_time_utc + 600
                && observed_at_utc >= self.context.frozen_at_utc
                && self
                    .inputs
                    .last()
                    .is_none_or(|i| observed_at_utc >= i.observed_at_utc),
            "numerical clock regression/unclosed bar"
        );
        ensure!(
            [bar.o, bar.h, bar.l, bar.close, bar.v]
                .iter()
                .all(|v| v.is_finite())
                && bar.l > 0.
                && bar.v >= 0.
                && bar.l <= bar.o
                && bar.o <= bar.h
                && bar.l <= bar.close
                && bar.close <= bar.h,
            "invalid numerical OHLCV"
        );
        // Missing execution data must never partially apply a pending decision.
        let mut staged = self.clone();
        staged.step(&bar)?;
        staged.inputs.push(Input {
            bar,
            observed_at_utc,
        });
        *self = staged;
        Ok(())
    }

    fn apply(&mut self, bar: &BarEvent) -> Result<()> {
        let Some(a) = self.state.pending.take() else {
            return Ok(());
        };
        ensure!(a.at_utc == bar.close_time_utc, "stale pending action");
        if let Some(reason) = a.exit_reason {
            let p = self
                .state
                .position
                .take()
                .context("exit without position")?;
            ensure!(
                p.entry.owner == a.owner && p.entry.side == a.side,
                "exit ownership mismatch"
            );
            let points = (bar.o - p.price) * if a.side == Side::Long { 1. } else { -1. };
            let cost = leg_cost(p.price) + leg_cost(bar.o);
            self.trades.push(Trade {
                component: a.owner,
                side: a.side,
                entry_ts_utc: p.entry.at_utc,
                entry_signal_ts_utc: p.entry.signal_ts_utc,
                entry_price: p.price,
                bars_held: p.bars_held,
                exit_ts_utc: a.at_utc,
                exit_signal_ts_utc: a.signal_ts_utc,
                exit_price: bar.o,
                exit_reason: reason,
                gross_points: points,
                friction_rub: cost,
                net_x1_rub: points * 10. - cost,
                net_x2_rub: points * 10. - 2. * cost,
            });
        } else {
            ensure!(self.state.position.is_none(), "overlapping entry");
            match (a.owner, a.side) {
                (Owner::Mr, _) => self.state.mr_entries += 1,
                (Owner::Bo, Side::Long) => self.state.used_long = true,
                (Owner::Bo, Side::Short) => self.state.used_short = true,
            }
            self.state.position = Some(Position {
                entry: a.clone(),
                price: bar.o,
                bars_held: 0,
            });
        }
        self.events.push(Event {
            role: if a.exit_reason.is_some() {
                "exit"
            } else {
                "entry"
            },
            ts_utc: a.at_utc,
            component: a.owner,
            side: a.side,
            signal_ts_utc: Some(a.signal_ts_utc),
            price: Some(bar.o),
        });
        Ok(())
    }

    fn schedule(
        &mut self,
        owner: Owner,
        side: Side,
        signal: i64,
        exit_reason: Option<&'static str>,
    ) -> Result<()> {
        ensure!(
            self.state.pending.is_none()
                && (local(signal)?.hour(), local(signal)?.minute()) < (23, 40),
            "no consecutive same-day execution bar"
        );
        self.state.pending = Some(Action {
            owner,
            side,
            signal_ts_utc: signal,
            at_utc: signal + 600,
            exit_reason,
        });
        Ok(())
    }

    fn exit_reason(&self, b: &BarEvent) -> Result<Option<&'static str>> {
        let Some(p) = &self.state.position else {
            return Ok(None);
        };
        let t = local(b.close_time_utc)?;
        let c = if p.entry.owner == Owner::Mr {
            self.context.mr.inputs.as_ref().unwrap()
        } else {
            self.context.bo.inputs.as_ref().unwrap()
        };
        let (pc, pr) = (c.previous_close, c.previous_range);
        Ok(match p.entry.owner {
            Owner::Mr => {
                if b.h >= pc + 0.58 * pr {
                    Some("stop")
                } else if b.close < pc - 0.020 * pr {
                    Some("take_author_close")
                } else if t.hour() >= 20 {
                    Some("time_exit")
                } else if p.bars_held > 20 && b.l <= p.price {
                    Some("breakeven_limit")
                } else {
                    None
                }
            }
            Owner::Bo => {
                if (t.hour(), t.minute()) == (23, 30) {
                    Some("bo_eod")
                } else if p.entry.side == Side::Long {
                    if b.close < pc - 0.35 * pr {
                        Some("bo_stop2_long")
                    } else if t.minute() == 50 && b.close < pc + 0.51 * pr {
                        Some("bo_stop1_long")
                    } else {
                        None
                    }
                } else if b.close > pc + 0.35 * pr {
                    Some("bo_stop2_short")
                } else if t.minute() == 50 && b.close > pc - 0.51 * pr {
                    Some("bo_stop1_short")
                } else {
                    None
                }
            }
        })
    }

    fn predicates(&self, b: &BarEvent) -> Result<(bool, bool, bool)> {
        let m = self.context.mr.inputs.as_ref().unwrap();
        let c = self.context.bo.inputs.as_ref().unwrap();
        let t = local(b.close_time_utc)?;
        let cutoff_hour = local(self.context.cutoff_utc)?.hour() + 3;
        Ok((
            valid(m)
                && self.state.mr_entries < 2
                && (t.hour(), t.minute()) <= (cutoff_hour, 0)
                && 0.005 < m.previous_range / m.previous_low
                && m.previous_range / m.previous_low < 0.075
                && m.previous_close < b.close
                && b.close < m.previous_close + 0.16 * m.previous_range
                && m.previous_range.is_finite()
                && m.previous_close + 0.58 * m.previous_range > b.close,
            valid(c)
                && !(c.previous_return.is_finite() && c.previous_return < -0.025)
                && !self.state.used_long
                && b.close > c.previous_close + 0.53 * c.previous_range,
            valid(c)
                && !(c.previous_return.is_finite() && c.previous_return > 0.025)
                && !self.state.used_short
                && b.close < c.previous_close - 0.53 * c.previous_range,
        ))
    }

    fn step(&mut self, b: &BarEvent) -> Result<()> {
        self.apply(b)?;
        let s = &mut self.state;
        s.day_high = Some(s.day_high.map_or(b.h, |h| h.max(b.h)));
        s.day_low = Some(s.day_low.map_or(b.l, |l| l.min(b.l)));
        if s.observed_bars == 0 {
            s.first_high = Some(b.h);
            s.first_low = Some(b.l);
        }
        let bo = self.context.bo.inputs.as_ref().unwrap();
        // Observation precedes all MR, ownership and OI early returns.
        if s.observed_bars == 5 && valid(bo) {
            s.bo_long_level = Some(
                (bo.previous_close + 0.53 * bo.previous_range)
                    .max(b.close)
                    .max(s.first_high.unwrap()),
            );
            s.bo_short_level = Some(
                (bo.previous_close - 0.53 * bo.previous_range)
                    .min(b.close)
                    .min(s.first_low.unwrap()),
            );
            if (b.close - bo.previous_close).abs() > 1.5 * bo.previous_range {
                s.author_allowed = false;
            }
        }
        s.observed_bars += 1;
        s.last_label_utc = Some(b.close_time_utc);
        if let Some(p) = &mut s.position {
            p.bars_held += 1;
        }
        if let Some(reason) = self.exit_reason(b)? {
            let p = self.state.position.as_ref().unwrap();
            self.schedule(p.entry.owner, p.entry.side, b.close_time_utc, Some(reason))?;
        }
        let (mr, long, short) = self.predicates(b)?;
        self.state.mr_predicate = mr;
        self.state.bo_long_predicate = long;
        self.state.bo_short_predicate = short;
        let m = self.context.mr.inputs.as_ref().unwrap();
        let c = self.context.bo.inputs.as_ref().unwrap();
        self.state.decision_site = if self.state.position.is_some()
            || self.state.pending.is_some()
            || !(valid(m) || valid(c))
        {
            "busy_or_invalid"
        } else if mr {
            self.schedule(Owner::Mr, Side::Short, b.close_time_utc, None)?;
            "mr_selected"
        } else if !valid(c) || !self.gate.allows() {
            "bo_disabled_or_gate"
        } else if b.close_time_utc < self.context.cutoff_utc + 3 * 3600 || c.previous_range < 1.01 {
            "bo_wait_or_range"
        } else if !long && !short {
            "bo_no_eligible_side"
        } else {
            let side = if long { Side::Long } else { Side::Short };
            let t = local(b.close_time_utc)?;
            if (t.hour(), t.minute()) >= (23, 30) {
                "bo_terminal"
            } else if (
                local(b.close_time_utc + 600)?.hour(),
                local(b.close_time_utc + 600)?.minute(),
            ) >= (23, 30)
            {
                self.events.push(Event {
                    role: "entry_suppressed_eod",
                    ts_utc: b.close_time_utc + 600,
                    component: Owner::Bo,
                    side,
                    signal_ts_utc: None,
                    price: None,
                });
                "bo_next_open_eod"
            } else {
                self.schedule(Owner::Bo, side, b.close_time_utc, None)?;
                "bo_selected"
            }
        };
        let t = local(b.close_time_utc)?;
        if (t.hour(), t.minute()) == (23, 40) {
            ensure!(
                self.state.pending.is_none() && self.state.position.is_none(),
                "unclosed numerical session; no invented flatten"
            );
        }
        Ok(())
    }

    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>> {
        let payload = Payload {
            version: VERSION.into(),
            context_sha256: self.context_sha256.clone(),
            gate: serde_json::to_value(&self.gate)?,
            inputs: self.inputs.clone(),
            witness: serde_json::to_value((&self.state, &self.events, &self.trades))?,
        };
        let sha256 = hash(&payload)?;
        let bytes = serde_json::to_vec(&Envelope { payload, sha256 })?;
        ensure!(
            bytes.len() <= MAX_CHECKPOINT_BYTES,
            "numerical checkpoint bound"
        );
        Ok(bytes)
    }

    pub fn restore(context: FrozenComponents, gate: BoGate, bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_CHECKPOINT_BYTES,
            "numerical checkpoint bound"
        );
        let e: Envelope = serde_json::from_slice(bytes)?;
        ensure!(
            e.payload.version == VERSION
                && hash(&e.payload)? == e.sha256
                && e.payload.context_sha256 == hash(&context)?
                && e.payload.gate == serde_json::to_value(&gate)?
                && e.payload.inputs.len() <= 101,
            "numerical checkpoint identity/hash mismatch"
        );
        let mut result = Self::new(context, gate)?;
        for input in e.payload.inputs {
            result.observe(input.bar, input.observed_at_utc)?;
        }
        ensure!(
            serde_json::to_value((&result.state, &result.events, &result.trades))?
                == e.payload.witness,
            "numerical witness differs from replay"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
