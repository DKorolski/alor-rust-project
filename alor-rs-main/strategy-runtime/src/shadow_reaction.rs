//! Numerical frozen-context integration only. Not a Strategy or an operational
//! checkpoint/collector: no Intent, broker, Redis, filesystem or runtime route.
//! Replaying a saved prefix reconstructs pending state, never re-emits orders.
#[path = "bin/imoexf_shadow_replay/model.rs"]
pub mod imoexf;
#[path = "bin/usdrubf_shadow_replay/model.rs"]
pub mod usdrubf;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::shadow_session_context::{Anchor, Frozen, Identity, Profile, Store};
use crate::{BarEvent, DataOrigin};

const VERSION: &str = "frozen_reaction_prefix_v1";
const MAX_BYTES: usize = 40 * 1024 * 1024;
const MAX_BARS: usize = 101;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "engine", rename_all = "snake_case", deny_unknown_fields)]
pub enum Config {
    Imoexf {
        sleeve: Sleeve,
        costs: imoexf::Costs,
    },
    Usdrubf {
        settings: usdrubf::Config,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sleeve {
    Mr,
    Bo,
    Combo,
}
impl Config {
    pub fn fingerprint(&self) -> Result<String> {
        hash(&(VERSION, self))
    }
    pub(crate) fn profile(&self) -> Result<Profile> {
        match self {
            Self::Imoexf { costs, .. } => {
                ensure!(
                    [
                        costs.point_value_rub,
                        costs.tick_value_rub,
                        costs.rate_per_component_per_side
                    ]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0)
                        && costs.point_value_rub > 0.0
                        && costs.components > 0
                        && costs.components <= 16,
                    "invalid numerical cost policy"
                );
                Ok(Profile::ImoexfReaction07)
            }
            Self::Usdrubf { settings } => {
                settings.validate()?;
                ensure!(
                    settings.family == usdrubf::Family::PreviousDay,
                    "not a Previous-day profile"
                );
                ensure!(
                    !settings.model_id.is_empty() && settings.model_id.len() <= 256,
                    "invalid model id"
                );
                Ok(if settings.context_hour == 7 {
                    Profile::UsdrubfPreviousDay07
                } else {
                    Profile::UsdrubfPreviousDay09
                })
            }
        }
    }
}

fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn local(ts: i64) -> Result<NaiveDateTime> {
    DateTime::from_timestamp(ts, 0)
        .and_then(|dt| dt.naive_utc().checked_add_signed(Duration::hours(3)))
        .context("invalid model label")
}
fn imo_anchor(a: &Anchor) -> Result<imoexf::Observation> {
    Ok(imoexf::Observation {
        first: local(a.first_bar_utc)?,
        last: local(a.last_bar_utc)?,
        high: a.high,
        low: a.low,
        close: a.close,
        count: a.distinct_bars,
    })
}
fn usd_anchor(a: &Anchor) -> Result<usdrubf::Metrics> {
    Ok(usdrubf::Metrics {
        first: local(a.first_bar_utc)?,
        last: local(a.last_bar_utc)?,
        open: a.open,
        high: a.high,
        low: a.low,
        close: a.close,
        count: a.distinct_bars,
        // Previous-day models never consume a prior-session VWAP accumulator.
        numerator: usdrubf::Kahan::default(),
        volume: usdrubf::Kahan::default(),
    })
}

#[derive(Clone, Serialize)]
#[serde(tag = "engine", content = "state", rename_all = "snake_case")]
enum Engine {
    Imoexf(Box<imoexf::Engine>),
    Usdrubf(Box<usdrubf::Engine>),
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
    context_checkpoint: String,
    config: Config,
    day: NaiveDate,
    inputs: Vec<Input>,
    clock_floor: i64,
    witness: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: Payload,
    sha256: String,
}

/// One numerical session. Only origin=Replay is accepted, so this facade cannot
/// silently become a live adapter. Original receipt attestation belongs elsewhere.
#[derive(Clone)]
pub struct NumericalSession {
    identity: Identity,
    context_checkpoint: String,
    config: Config,
    frozen: Frozen,
    engine: Engine,
    inputs: Vec<Input>,
    clock_floor: i64,
}
impl NumericalSession {
    pub fn new(context: &Store, day: NaiveDate, config: Config) -> Result<Self> {
        let identity = context.identity().clone();
        ensure!(
            config.profile()? == identity.profile,
            "context/engine profile mismatch"
        );
        ensure!(
            config.fingerprint()? == identity.model_config_sha256,
            "model config mismatch"
        );
        let frozen = context
            .frozen(day)
            .context("session context not frozen")?
            .clone();
        ensure!(frozen.ready(), "two complete prior anchors required");
        let previous = frozen.previous.as_ref().unwrap();
        let previous2 = frozen.previous2.as_ref().unwrap();
        let engine = match &config {
            Config::Imoexf { sleeve, costs } => {
                let mut e = imoexf::Engine::new(
                    match sleeve {
                        Sleeve::Mr => "mr",
                        Sleeve::Bo => "bo",
                        Sleeve::Combo => "combo",
                    },
                    costs.clone(),
                )?;
                e.start_frozen(
                    local(frozen.cutoff_utc)?,
                    imo_anchor(previous)?,
                    imo_anchor(previous2)?,
                )?;
                Engine::Imoexf(Box::new(e))
            }
            Config::Usdrubf { settings } => {
                let mut e = usdrubf::Engine::new(settings.clone());
                e.start_frozen(usd_anchor(previous)?, usd_anchor(previous2)?)?;
                Engine::Usdrubf(Box::new(e))
            }
        };
        let clock_floor = frozen.frozen_at_utc;
        Ok(Self {
            identity,
            context_checkpoint: String::from_utf8(context.checkpoint_bytes()?)?,
            config,
            frozen,
            engine,
            inputs: vec![],
            clock_floor,
        })
    }

    pub fn observe(&mut self, bar: BarEvent, observed_at_utc: i64) -> Result<()> {
        ensure!(
            bar.origin == DataOrigin::Replay,
            "only explicit numerical replay inputs allowed"
        );
        ensure!(
            bar.symbol == self.identity.symbol,
            "execution instrument mismatch"
        );
        ensure!(
            self.inputs.len() < MAX_BARS,
            "numerical prefix bound reached"
        );
        let expected = self.frozen.cutoff_utc + self.inputs.len() as i64 * 600;
        let label = local(bar.close_time_utc)?;
        ensure!(
            bar.close_time_utc == expected
                && label.date() == self.frozen.session_date
                && label.time() <= imoexf::time(23, 40),
            "missing, duplicate or out-of-session execution bar"
        );
        ensure!(
            observed_at_utc >= bar.close_time_utc + 600
                && observed_at_utc >= self.clock_floor
                && self
                    .inputs
                    .last()
                    .is_none_or(|i| observed_at_utc >= i.observed_at_utc),
            "bar not closed or numerical clock regressed"
        );
        ensure!(
            bar.v.is_finite() && bar.v >= 0.0,
            "invalid execution volume"
        );
        // Failed pending application, feature overflow or EOD cannot partially
        // consume a decision. Commit only after the entire closed-bar step passes.
        let mut staged = self.engine.clone();
        match &mut staged {
            Engine::Imoexf(e) => {
                let b = imoexf::Bar {
                    ts_local: label,
                    open: bar.o,
                    high: bar.h,
                    low: bar.l,
                    close: bar.close,
                    volume: bar.v,
                };
                imoexf::validate_bars(&[b])?;
                e.step_frozen(&b)?;
            }
            Engine::Usdrubf(e) => {
                let b = usdrubf::Bar {
                    ts_local: label,
                    open: bar.o,
                    high: bar.h,
                    low: bar.l,
                    close: bar.close,
                    volume: bar.v,
                };
                usdrubf::validate_raw(&[b])?;
                ensure!(e.cfg.eligible(&b), "nontradable model input");
                e.step_frozen(&b)?;
            }
        }
        self.engine = staged;
        self.inputs.push(Input {
            bar,
            observed_at_utc,
        });
        self.clock_floor = observed_at_utc;
        Ok(())
    }

    pub fn state(&self) -> Result<Value> {
        Ok(
            json!({"version":VERSION,"evidence":"numerical_prefix_not_operational_receipts",
            "identity":self.identity,"frozen":self.frozen,"model":self.engine,
            "bars_processed":self.inputs.len(),"order_emission_allowed":false}),
        )
    }

    pub fn complete(&self) -> Result<()> {
        ensure!(
            self.inputs.last().is_some_and(
                |i| local(i.bar.close_time_utc).is_ok_and(|t| t.time() == imoexf::time(23, 40))
            ),
            "incomplete session; do not infer flat or fabricate EOD fill"
        );
        let flat = match &self.engine {
            Engine::Imoexf(e) => e.position.is_none() && e.pending.is_none(),
            Engine::Usdrubf(e) => e.position.is_none() && e.pending.is_none(),
        };
        ensure!(flat, "unresolved position/pending at session end");
        Ok(())
    }

    /// Bytes for an offline artifact, not a durable production write protocol.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>> {
        let payload = Payload {
            version: VERSION.into(),
            context_checkpoint: self.context_checkpoint.clone(),
            config: self.config.clone(),
            day: self.frozen.session_date,
            inputs: self.inputs.clone(),
            clock_floor: self.clock_floor,
            witness: self.state()?,
        };
        let sha256 = hash(&payload)?;
        let bytes = serde_json::to_vec(&Envelope { payload, sha256 })?;
        ensure!(bytes.len() <= MAX_BYTES, "checkpoint too large");
        Ok(bytes)
    }

    pub fn restore(expected: &Identity, config: &Config, bytes: &[u8], now: i64) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "checkpoint too large");
        let saved: Envelope = serde_json::from_slice(bytes)?;
        let p = saved.payload;
        ensure!(
            p.version == VERSION && hash(&p)? == saved.sha256,
            "checkpoint version/checksum mismatch"
        );
        ensure!(p.inputs.len() <= MAX_BARS, "oversized numerical prefix");
        ensure!(
            p.config.fingerprint()? == config.fingerprint()?,
            "restart model changed"
        );
        ensure!(
            p.clock_floor <= now
                && p.inputs
                    .last()
                    .is_none_or(|i| i.observed_at_utc <= p.clock_floor),
            "restart clock regressed"
        );
        let context = Store::restore(expected, p.context_checkpoint.as_bytes(), now)?;
        let mut session = Self::new(&context, p.day, config.clone())?;
        for input in p.inputs {
            session.observe(input.bar, input.observed_at_utc)?;
        }
        ensure!(
            session.state()? == p.witness,
            "reconstructed decision state differs"
        );
        ensure!(
            p.clock_floor >= session.clock_floor,
            "invalid saved processing clock"
        );
        session.clock_floor = now;
        Ok(session)
    }
}

#[cfg(test)]
#[path = "shadow_reaction/tests.rs"]
mod tests;
