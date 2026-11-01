//! Independent frozen Author41-short / Hybrid BO053 reaction model, not a live adapter.
use anyhow::{ensure, Result};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Deserializer, Serialize};

pub fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
}

pub fn transition() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 14).unwrap()
}

fn parse_timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
    let text = String::deserialize(d)?;
    NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S").map_err(serde::de::Error::custom)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bar {
    #[serde(deserialize_with = "parse_timestamp")]
    pub ts_local: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

pub fn validate_bars(bars: &[Bar]) -> Result<()> {
    ensure!(!bars.is_empty(), "empty model input");
    let mut previous = None;
    for b in bars {
        let ts = b.ts_local;
        ensure!(previous.is_none_or(|p| p < ts), "unordered/duplicate input");
        previous = Some(ts);
        let start = if ts.date() < transition() { 9 } else { 7 };
        ensure!(
            ts.weekday().num_days_from_monday() < 5
                && ts.time() >= time(start, 0)
                && ts.time() <= time(23, 49)
                && ts.minute().is_multiple_of(10)
                && ts.second() == 0
                && ts.nanosecond() == 0,
            "input violates frozen weekday M10 session: {ts}"
        );
        ensure!(
            [b.open, b.high, b.low, b.close, b.volume]
                .iter()
                .all(|v| v.is_finite())
                && b.low > 0.0
                && b.volume >= 0.0
                && b.low <= b.open
                && b.open <= b.high
                && b.low <= b.close
                && b.close <= b.high,
            "invalid OHLCV: {ts}"
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Owner {
    MR,
    BO,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Long,
    Short,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Action {
    pub owner: Owner,
    pub side: Side,
    pub signal: NaiveDateTime,
    pub at: NaiveDateTime,
    pub exit_reason: Option<&'static str>,
}

impl Action {
    pub fn role(&self) -> &'static str {
        if self.exit_reason.is_some() {
            "exit"
        } else {
            "entry"
        }
    }
    pub fn reason(&self) -> &'static str {
        self.exit_reason.unwrap_or(match self.owner {
            Owner::MR => "mr_entry",
            Owner::BO => "bo_entry",
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Position {
    pub entry: Action,
    pub price: f64,
    pub bars_held: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub first: NaiveDateTime,
    pub last: NaiveDateTime,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub count: usize,
}

impl Observation {
    fn empty(first: NaiveDateTime) -> Self {
        Self {
            first,
            last: first,
            high: f64::NEG_INFINITY,
            low: f64::INFINITY,
            close: f64::NAN,
            count: 0,
        }
    }
    fn observe(&mut self, b: &Bar) {
        self.last = b.ts_local;
        self.high = self.high.max(b.high);
        self.low = self.low.min(b.low);
        self.close = b.close;
        self.count += 1;
    }
    pub fn range(&self) -> f64 {
        self.high - self.low
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Costs {
    pub point_value_rub: f64,
    pub tick_value_rub: f64,
    pub rate_per_component_per_side: f64,
    pub components: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trade {
    pub model_id: String,
    pub contract: &'static str,
    pub sleeve: &'static str,
    pub component: Owner,
    pub side: Side,
    pub entry_ts: NaiveDateTime,
    pub entry_event: NaiveDateTime,
    pub entry_signal_ts: NaiveDateTime,
    pub entry_price: f64,
    pub bars_held: usize,
    pub exit_ts: NaiveDateTime,
    pub exit_event: NaiveDateTime,
    pub exit_signal_ts: NaiveDateTime,
    pub exit_price: f64,
    pub exit_reason: &'static str,
    pub gross_points: f64,
    pub gross_rub: f64,
    pub fees_rub: f64,
    pub net_rub: f64,
    pub one_tick_net_rub: f64,
    pub cost_x2_net_rub: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub sleeve: &'static str,
    pub role: &'static str,
    pub ts: NaiveDateTime,
    pub component: Owner,
    pub side: Side,
    pub signal_ts: Option<NaiveDateTime>,
    pub price: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Engine {
    pub sleeve: &'static str,
    pub previous: Option<Observation>,
    pub previous2: Option<Observation>,
    pub observation: Option<Observation>,
    pub position: Option<Position>,
    pub pending: Option<Action>,
    pub mr_entries: usize,
    pub used_long: bool,
    pub used_short: bool,
    pub branch: &'static str,
    pub trades: Vec<Trade>,
    pub events: Vec<Event>,
    costs: Costs,
}

impl Engine {
    pub fn new(sleeve: &'static str, costs: Costs) -> Result<Self> {
        ensure!(
            matches!(sleeve, "mr" | "bo" | "combo"),
            "unsupported frozen sleeve"
        );
        Ok(Self {
            sleeve,
            previous: None,
            previous2: None,
            observation: None,
            position: None,
            pending: None,
            mr_entries: 0,
            used_long: false,
            used_short: false,
            branch: "not_evaluated",
            trades: vec![],
            events: vec![],
            costs,
        })
    }

    pub fn context(&self) -> (f64, f64, f64, f64) {
        let Some(p) = &self.previous else {
            return (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
        };
        let ret = self
            .previous2
            .as_ref()
            .map_or(f64::NAN, |p2| (p.close - p2.close) / p2.close);
        (p.close, p.range(), p.low, ret)
    }

    pub fn valid(&self) -> bool {
        let (pc, pr, pl, _) = self.context();
        (pc + pr + pl).is_finite() && pc > 0.0 && pr > 0.0 && pl > 0.0
    }

    pub fn setups(&self, b: &Bar) -> (bool, bool, bool) {
        let (pc, pr, pl, ret) = self.context();
        let valid = self.valid();
        (
            valid && 0.005 < pr / pl && pr / pl < 0.075 && pc < b.close && b.close < pc + 0.16 * pr,
            valid && !(ret.is_finite() && ret < -0.025) && b.close > pc + 0.53 * pr,
            valid && !(ret.is_finite() && ret > 0.025) && b.close < pc - 0.53 * pr,
        )
    }

    fn apply(&mut self, b: &Bar) -> Result<bool> {
        let Some(a) = self.pending else {
            return Ok(false);
        };
        ensure!(
            a.at == b.ts_local,
            "execution bar missing; refusing stale decision"
        );
        if let Some(reason) = a.exit_reason {
            let p = self
                .position
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("exit without position"))?;
            ensure!(
                p.entry.owner == a.owner && p.entry.side == a.side,
                "exit ownership mismatch"
            );
            let gross_points = (b.open - p.price) * if a.side == Side::Long { 1.0 } else { -1.0 };
            let gross_rub = gross_points * self.costs.point_value_rub;
            let fees_rub = (p.price + b.open)
                * self.costs.point_value_rub
                * self.costs.rate_per_component_per_side
                * self.costs.components as f64;
            self.trades.push(Trade {
                model_id: if self.sleeve == "bo" {
                    "bo053_reaction07_control".into()
                } else {
                    format!("k016_t020_s058_{}", self.sleeve)
                },
                contract: "reaction_next_open",
                sleeve: self.sleeve,
                component: a.owner,
                side: a.side,
                entry_ts: p.entry.at,
                entry_event: p.entry.at,
                entry_signal_ts: p.entry.signal,
                entry_price: p.price,
                bars_held: p.bars_held,
                exit_ts: a.at,
                exit_event: a.at,
                exit_signal_ts: a.signal,
                exit_price: b.open,
                exit_reason: reason,
                gross_points,
                gross_rub,
                fees_rub,
                net_rub: gross_rub - fees_rub,
                one_tick_net_rub: gross_rub - fees_rub - 2.0 * self.costs.tick_value_rub,
                cost_x2_net_rub: gross_rub - 2.0 * fees_rub,
            });
            self.position = None;
        } else {
            ensure!(self.position.is_none(), "overlapping entry");
            self.position = Some(Position {
                entry: a,
                price: b.open,
                bars_held: 0,
            });
            match (a.owner, a.side) {
                (Owner::MR, _) => self.mr_entries += 1,
                (Owner::BO, Side::Long) => self.used_long = true,
                (Owner::BO, Side::Short) => self.used_short = true,
            }
        }
        self.events.push(Event {
            sleeve: self.sleeve,
            role: a.role(),
            ts: a.at,
            component: a.owner,
            side: a.side,
            signal_ts: Some(a.signal),
            price: Some(b.open),
        });
        self.pending = None;
        Ok(true)
    }

    fn schedule(&mut self, a: Action, next: Option<NaiveDateTime>) -> Result<()> {
        ensure!(self.pending.is_none(), "pending decision already exists");
        ensure!(
            next.is_some_and(|ts| ts == a.at && ts.date() == a.signal.date()),
            "no consecutive same-session execution bar for {}",
            a.signal
        );
        self.pending = Some(a);
        Ok(())
    }

    fn exit_reason(&self, b: &Bar) -> Option<&'static str> {
        let p = self.position.as_ref()?;
        let (pc, pr, _, _) = self.context();
        match p.entry.owner {
            Owner::MR => {
                if b.high >= pc + 0.58 * pr {
                    Some("stop")
                } else if b.close < pc - 0.020 * pr {
                    Some("take_author_close")
                } else if b.ts_local.time() >= time(20, 0) {
                    Some("time_exit")
                } else if p.bars_held > 20 && b.low <= p.price {
                    Some("breakeven_limit")
                } else {
                    None
                }
            }
            Owner::BO => {
                if b.ts_local.time() == time(23, 30) {
                    Some("bo_eod")
                } else if p.entry.side == Side::Long {
                    if b.close < pc - 0.35 * pr {
                        Some("bo_stop2_long")
                    } else if b.ts_local.minute() == 50 && b.close < pc + 0.51 * pr {
                        Some("bo_stop1_long")
                    } else {
                        None
                    }
                } else if b.close > pc + 0.35 * pr {
                    Some("bo_stop2_short")
                } else if b.ts_local.minute() == 50 && b.close > pc - 0.51 * pr {
                    Some("bo_stop1_short")
                } else {
                    None
                }
            }
        }
    }

    fn decide(&mut self, b: &Bar, next: Option<NaiveDateTime>) -> Result<()> {
        let boundary = b.ts_local + Duration::minutes(10);
        if let Some(p) = &mut self.position {
            p.bars_held += 1;
        }
        if let Some(reason) = self.exit_reason(b) {
            let p = self.position.as_ref().unwrap();
            self.schedule(
                Action {
                    owner: p.entry.owner,
                    side: p.entry.side,
                    signal: b.ts_local,
                    at: boundary,
                    exit_reason: Some(reason),
                },
                next,
            )?;
        }
        self.branch = if !self.valid() {
            "context_not_ready"
        } else if self.pending.is_some() {
            "pending_active"
        } else if self.position.is_some() {
            "position_active"
        } else {
            "not_evaluated"
        };
        if self.branch != "not_evaluated" {
            return Ok(());
        }
        let (mr, long, short) = self.setups(b);
        if self.sleeve != "bo" && mr && self.mr_entries < 2 && b.ts_local.time() <= time(10, 0) {
            self.schedule(
                Action {
                    owner: Owner::MR,
                    side: Side::Short,
                    signal: b.ts_local,
                    at: boundary,
                    exit_reason: None,
                },
                next,
            )?;
            self.branch = "mr_selected";
            return Ok(());
        }
        if self.sleeve == "mr" {
            self.branch = "bo_disabled";
            return Ok(());
        }
        if b.ts_local < self.observation.as_ref().unwrap().first + Duration::hours(3) {
            self.branch = "bo_wait";
            return Ok(());
        }
        if self.context().1 < 1.01 {
            self.branch = "bo_range_too_small";
            return Ok(());
        }
        let side = if long && !self.used_long {
            Some(Side::Long)
        } else if short && !self.used_short {
            Some(Side::Short)
        } else {
            None
        };
        let Some(side) = side else {
            self.branch = "bo_no_eligible_side";
            return Ok(());
        };
        if b.ts_local.time() >= time(23, 30) || next.is_none() {
            self.branch = "bo_terminal_or_missing_next_bar";
            return Ok(());
        }
        if boundary.time() >= time(23, 30) {
            self.events.push(Event {
                sleeve: self.sleeve,
                role: "entry_suppressed_eod",
                ts: boundary,
                component: Owner::BO,
                side,
                signal_ts: None,
                price: None,
            });
            self.branch = "bo_next_open_at_eod";
            return Ok(());
        }
        self.schedule(
            Action {
                owner: Owner::BO,
                side,
                signal: b.ts_local,
                at: boundary,
                exit_reason: None,
            },
            next,
        )?;
        self.branch = "bo_selected";
        Ok(())
    }

    pub(super) fn start_frozen(
        &mut self,
        first: NaiveDateTime,
        previous: Observation,
        previous2: Observation,
    ) -> Result<()> {
        ensure!(
            self.observation.is_none(),
            "frozen session requires a new engine"
        );
        ensure!(
            self.position.is_none() && self.pending.is_none(),
            "unresolved session"
        );
        self.previous = Some(previous);
        self.previous2 = Some(previous2);
        self.observation = Some(Observation::empty(first));
        Ok(())
    }

    pub(super) fn step_frozen(&mut self, b: &Bar) -> Result<()> {
        // The facade validates a consecutive session prefix and stages mutations.
        // Only the expected next label is used; no future price is supplied.
        self.branch = "not_evaluated";
        self.apply(b)?;
        self.observation.as_mut().unwrap().observe(b);
        let next = (b.ts_local.time() < time(23, 40)).then_some(b.ts_local + Duration::minutes(10));
        self.decide(b, next)?;
        if next.is_none() {
            ensure!(
                self.position.is_none() && self.pending.is_none(),
                "unclosed frozen session"
            );
        }
        Ok(())
    }

    pub fn replay(
        &mut self,
        bars: &[Bar],
        mut observe: impl FnMut(&str, &Self, &Bar) -> Result<()>,
    ) -> Result<()> {
        validate_bars(bars)?;
        ensure!(self.observation.is_none(), "replay requires a new engine");
        let mut offset = 0;
        while offset < bars.len() {
            ensure!(
                self.position.is_none() && self.pending.is_none(),
                "unresolved prior session"
            );
            let count = bars[offset..]
                .iter()
                .take_while(|b| b.ts_local.date() == bars[offset].ts_local.date())
                .count();
            let day = &bars[offset..offset + count];
            self.previous2 = self.previous.take();
            self.previous = self.observation.take();
            self.observation = Some(Observation::empty(day[0].ts_local));
            self.mr_entries = 0;
            self.used_long = false;
            self.used_short = false;
            for (i, b) in day.iter().enumerate() {
                self.branch = "not_evaluated";
                observe("bar_start", self, b)?;
                if self.apply(b)? {
                    observe("pending_applied", self, b)?;
                }
                self.observation.as_mut().unwrap().observe(b);
                observe("observed", self, b)?;
                self.decide(b, day.get(i + 1).map(|next| next.ts_local))?;
                observe("bar_end", self, b)?;
            }
            // Never hide missing EOD/execution data by clearing positions or inventing a flatten.
            ensure!(
                self.position.is_none() && self.pending.is_none(),
                "unclosed session {}",
                day[0].ts_local.date()
            );
            offset += count;
        }
        Ok(())
    }
}
