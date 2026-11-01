//! Offline reaction model. No runtime host, Redis, gateway or order-command types.
use anyhow::{ensure, Result};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Bar {
    #[serde(deserialize_with = "parse_label")]
    pub ts_local: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

fn parse_label<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<NaiveDateTime, D::Error> {
    let value = String::deserialize(deserializer)?;
    NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M:%S").map_err(serde::de::Error::custom)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    PreviousDay,
    Vwap,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Self::PreviousDay => "previous_day",
            Self::Vwap => "vwap",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub model_id: String,
    pub family: Family,
    pub context_hour: u32,
    pub clock_hour: u32,
    pub mr_horizon: u32,
    pub bo_wait: u32,
    pub mr_start_delay: u32,
}

pub fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).expect("validated model time")
}

pub fn opening(date: NaiveDate, hour: u32) -> u32 {
    if date < NaiveDate::from_ymd_opt(2026, 7, 14).unwrap() {
        9
    } else {
        hour
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.context_hour, 7 | 9),
            "unsupported context hour"
        );
        ensure!(
            self.clock_hour == self.context_hour,
            "clock/context mismatch"
        );
        ensure!(matches!(self.mr_horizon, 3 | 4), "unsupported MR horizon");
        ensure!(matches!(self.bo_wait, 2 | 4), "unsupported BO wait");
        ensure!(self.mr_start_delay == 0, "unsupported MR delay");
        Ok(())
    }

    pub fn eligible(&self, bar: &Bar) -> bool {
        bar.ts_local.weekday().num_days_from_monday() < 5
            && bar.ts_local.time() >= time(opening(bar.ts_local.date(), self.context_hour), 0)
            && bar.ts_local.time() < time(23, 50)
    }
}

pub fn validate_raw(bars: &[Bar]) -> Result<()> {
    ensure!(!bars.is_empty(), "empty input");
    for (index, bar) in bars.iter().enumerate() {
        ensure!(
            index == 0 || bars[index - 1].ts_local < bar.ts_local,
            "unordered/duplicate input"
        );
        ensure!(
            bar.ts_local.minute().is_multiple_of(10)
                && bar.ts_local.second() == 0
                && bar.ts_local.nanosecond() == 0,
            "non-M10 label"
        );
        ensure!(
            [bar.open, bar.high, bar.low, bar.close, bar.volume]
                .iter()
                .all(|x| x.is_finite()),
            "nonfinite OHLCV"
        );
        ensure!(
            bar.low > 0.0
                && bar.low <= bar.open
                && bar.open <= bar.high
                && bar.low <= bar.close
                && bar.close <= bar.high,
            "invalid OHLC"
        );
    }
    Ok(())
}

pub fn validate_sessions(bars: &[Bar], cfg: &Config) -> Result<()> {
    ensure!(!bars.is_empty(), "empty prepared input");
    let mut index = 0;
    while index < bars.len() {
        let date = bars[index].ts_local.date();
        let mut expected = date.and_time(time(opening(date, cfg.context_hour), 0));
        while index < bars.len() && bars[index].ts_local.date() == date {
            ensure!(
                bars[index].ts_local == expected,
                "incomplete model session {date}: expected {expected}"
            );
            expected += Duration::minutes(10);
            index += 1;
        }
        ensure!(
            expected == date.and_time(time(23, 50)),
            "incomplete model session {date}"
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Component {
    MR,
    BO,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn sign(self) -> f64 {
        if self == Self::Long {
            1.0
        } else {
            -1.0
        }
    }
    pub fn name(self) -> &'static str {
        if self == Self::Long {
            "long"
        } else {
            "short"
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub owner: Component,
    pub side: Side,
    pub signal: NaiveDateTime,
    pub action: NaiveDateTime,
    pub scale: f64,
    pub anchor: f64,
    pub stop1: f64,
    pub stop2: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Position {
    pub entry: Entry,
    pub price: f64,
    pub stop: Option<f64>,
    pub take: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Exit {
    pub signal: NaiveDateTime,
    pub action: NaiveDateTime,
    pub reason: String,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, Serialize)]
pub enum Pending {
    Entry(Entry),
    Exit(Exit),
}

#[derive(Clone, Serialize, Debug)]
pub struct Event {
    pub model_id: String,
    pub role: String,
    pub ts: NaiveDateTime,
    pub signal_ts: NaiveDateTime,
    pub component: Component,
    pub side: Side,
    pub price: Option<f64>,
    pub reason: String,
}

#[derive(Clone, Serialize, Debug)]
pub struct Trade {
    pub model_id: String,
    pub component: Component,
    pub side: Side,
    pub entry_signal_ts: NaiveDateTime,
    pub scale: f64,
    pub anchor: f64,
    pub stop1: f64,
    pub stop2: f64,
    pub entry_ts: NaiveDateTime,
    pub entry_price: f64,
    pub stop_price: Option<f64>,
    pub take_price: Option<f64>,
    pub exit_ts: NaiveDateTime,
    pub exit_signal_ts: NaiveDateTime,
    pub exit_price: f64,
    pub exit_reason: String,
    pub gross_points: f64,
    pub ambiguous_mr_bar: bool,
    pub gross_rub: f64,
    pub fees_rub: f64,
    pub net_rub: f64,
    pub tick_stress_net_rub: f64,
    pub cost_x2_net_rub: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Kahan {
    pub sum: f64,
    correction: f64,
}

impl Kahan {
    fn add(&mut self, value: f64) {
        // Match the frozen pandas grouped cumulative-sum arithmetic, not a rounded VWAP.
        let adjusted = value - self.correction;
        let next = self.sum + adjusted;
        self.correction = next - self.sum - adjusted;
        self.sum = next;
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Metrics {
    pub first: NaiveDateTime,
    pub last: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub count: usize,
    pub numerator: Kahan,
    pub volume: Kahan,
}

impl Metrics {
    fn new(bar: &Bar) -> Self {
        Self {
            first: bar.ts_local,
            last: bar.ts_local,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            count: 0,
            numerator: Kahan::default(),
            volume: Kahan::default(),
        }
    }
    fn update(&mut self, bar: &Bar) -> Result<()> {
        self.last = bar.ts_local;
        self.high = self.high.max(bar.high);
        self.low = self.low.min(bar.low);
        self.close = bar.close;
        self.count += 1;
        let volume = bar.volume.max(0.0);
        self.numerator
            .add(((bar.high + bar.low + bar.close) / 3.0) * volume);
        self.volume.add(volume);
        ensure!(
            self.numerator.sum.is_finite() && self.volume.sum.is_finite(),
            "feature overflow"
        );
        Ok(())
    }
    pub fn range(&self) -> f64 {
        self.high - self.low
    }
    pub fn vwap(&self) -> f64 {
        if self.volume.sum > 0.0 {
            self.numerator.sum / self.volume.sum
        } else {
            self.close
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Engine {
    pub cfg: Config,
    pub metrics: Option<Metrics>,
    pub previous: Option<Metrics>,
    pub previous2: Option<Metrics>,
    pub position: Option<Position>,
    pub pending: Option<Pending>,
    pub used_long: bool,
    pub used_short: bool,
    pub mr_scheduled: usize,
    pub branch: &'static str,
    pub trades: Vec<Trade>,
    pub events: Vec<Event>,
}

impl Engine {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            metrics: None,
            previous: None,
            previous2: None,
            position: None,
            pending: None,
            used_long: false,
            used_short: false,
            mr_scheduled: 0,
            branch: "not_evaluated",
            trades: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn prior_return(&self) -> Option<f64> {
        Some(self.previous.as_ref()?.close / self.previous2.as_ref()?.close - 1.0)
    }

    pub fn features(&self) -> (Option<f64>, Option<f64>, Option<f64>, f64) {
        let m = self.metrics.as_ref().expect("observed bar");
        match self.cfg.family {
            Family::PreviousDay => {
                let anchor = self.previous.as_ref().map(|p| p.close);
                (
                    anchor,
                    anchor,
                    self.previous.as_ref().map(Metrics::range),
                    self.prior_return().unwrap_or(0.0),
                )
            }
            Family::Vwap => (
                Some(m.vwap()),
                Some(m.open),
                Some(m.range()),
                (m.close - m.open) / m.open,
            ),
        }
    }

    pub fn setups(&self) -> (bool, bool, bool) {
        let m = self.metrics.as_ref().unwrap();
        match self.features() {
            (Some(mr), Some(bo), Some(r), g) if r > 0.0 && m.close > 0.0 => (
                0.006 < r / m.close
                    && r / m.close < 0.050
                    && 0.0 < m.close - mr
                    && m.close - mr < 0.035 * r,
                g >= -0.020 && m.close > bo + 0.45 * r,
                g <= 0.020 && m.close < bo - 0.45 * r,
            ),
            _ => (false, false, false),
        }
    }

    pub fn clock(&self) -> u32 {
        opening(
            self.metrics.as_ref().unwrap().first.date(),
            self.cfg.clock_hour,
        )
    }

    pub fn mr_force(&self) -> NaiveTime {
        time(self.clock() + self.cfg.mr_horizon - 1, 50)
    }

    fn observe(&mut self, bar: &Bar) -> Result<()> {
        if self
            .metrics
            .as_ref()
            .is_none_or(|m| m.first.date() != bar.ts_local.date())
        {
            ensure!(
                self.position.is_none() && self.pending.is_none(),
                "unresolved cross-day state"
            );
            self.previous2 = self.previous.take();
            self.previous = self.metrics.take();
            self.metrics = Some(Metrics::new(bar));
            self.used_long = false;
            self.used_short = false;
            self.mr_scheduled = 0;
        }
        self.metrics.as_mut().unwrap().update(bar)
    }

    fn apply_pending(&mut self, bar: &Bar) -> Result<bool> {
        let Some(pending) = self.pending.take() else {
            return Ok(false);
        };
        match pending {
            Pending::Entry(entry) => {
                ensure!(
                    entry.action == bar.ts_local && self.position.is_none(),
                    "stale or overlapping entry"
                );
                let (stop, take) = if entry.owner == Component::MR {
                    (
                        Some(round_tick(bar.open + 0.43 * entry.scale)),
                        Some(round_tick(bar.open - 0.16 * entry.scale)),
                    )
                } else {
                    if entry.side == Side::Long {
                        self.used_long = true;
                    } else {
                        self.used_short = true;
                    }
                    (None, None)
                };
                self.events.push(Event {
                    model_id: self.cfg.model_id.clone(),
                    role: "entry".into(),
                    ts: bar.ts_local,
                    signal_ts: entry.signal,
                    component: entry.owner,
                    side: entry.side,
                    price: Some(bar.open),
                    reason: if entry.owner == Component::MR {
                        "mr_entry"
                    } else {
                        "bo_entry"
                    }
                    .into(),
                });
                self.position = Some(Position {
                    entry,
                    price: bar.open,
                    stop,
                    take,
                });
            }
            Pending::Exit(exit) => {
                ensure!(exit.action == bar.ts_local, "stale exit");
                let p = self
                    .position
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("exit without position"))?;
                let points = p.entry.side.sign() * (bar.open - p.price);
                let gross = points * 1000.0;
                let fees = (p.price + bar.open) * 1000.0 * 0.0000462 * 2.0;
                self.trades.push(Trade {
                    model_id: self.cfg.model_id.clone(),
                    component: p.entry.owner,
                    side: p.entry.side,
                    entry_signal_ts: p.entry.signal,
                    scale: p.entry.scale,
                    anchor: p.entry.anchor,
                    stop1: p.entry.stop1,
                    stop2: p.entry.stop2,
                    entry_ts: p.entry.action,
                    entry_price: p.price,
                    stop_price: p.stop,
                    take_price: p.take,
                    exit_ts: bar.ts_local,
                    exit_signal_ts: exit.signal,
                    exit_price: bar.open,
                    exit_reason: exit.reason.clone(),
                    gross_points: points,
                    ambiguous_mr_bar: exit.ambiguous,
                    gross_rub: gross,
                    fees_rub: fees,
                    net_rub: gross - fees,
                    tick_stress_net_rub: gross - fees - 20.0,
                    cost_x2_net_rub: gross - 2.0 * fees,
                });
                self.events.push(Event {
                    model_id: self.cfg.model_id.clone(),
                    role: "exit".into(),
                    ts: bar.ts_local,
                    signal_ts: exit.signal,
                    component: p.entry.owner,
                    side: p.entry.side,
                    price: Some(bar.open),
                    reason: exit.reason,
                });
            }
        }
        Ok(true)
    }

    fn exit_reason(&self, bar: &Bar) -> Option<(String, bool)> {
        let p = self.position.as_ref()?;
        let (reason, ambiguous) = if p.entry.owner == Component::MR {
            let stopped = bar.high >= p.stop.unwrap();
            let taken = bar.low <= p.take.unwrap();
            (
                if stopped {
                    "mr_stop"
                } else if taken {
                    "mr_take"
                } else if bar.ts_local.time() >= self.mr_force() {
                    "mr_time_cutoff"
                } else {
                    return None;
                }
                .to_string(),
                stopped && taken,
            )
        } else {
            let touched = if p.entry.side == Side::Long {
                bar.low <= p.entry.stop2
            } else {
                bar.high >= p.entry.stop2
            };
            (
                if touched {
                    format!("bo_stop2_{}", p.entry.side.name())
                } else if bar.ts_local.minute() == 50
                    && (bar.close - p.entry.stop1) * p.entry.side.sign() < 0.0
                {
                    format!("bo_stop1_{}", p.entry.side.name())
                } else if bar.ts_local.time() >= time(23, 30) {
                    "bo_eod_exit".into()
                } else {
                    return None;
                },
                false,
            )
        };
        Some((reason, ambiguous))
    }

    fn decide(&mut self, bar: &Bar, next: Option<NaiveDateTime>) -> Result<()> {
        let boundary = bar.ts_local + Duration::minutes(10);
        if let Some((reason, ambiguous)) = self.exit_reason(bar) {
            ensure!(
                next == Some(boundary),
                "no consecutive execution bar for exit"
            );
            self.pending = Some(Pending::Exit(Exit {
                signal: bar.ts_local,
                action: boundary,
                reason,
                ambiguous,
            }));
        }
        if self.pending.is_some() || self.position.is_some() {
            self.branch = if self.pending.is_some() {
                "pending_active"
            } else {
                "position_active"
            };
            return Ok(());
        }
        let (mr_anchor, bo_anchor, scale, _) = self.features();
        let Some(scale) = scale.filter(|r| r.is_finite() && *r > 0.0) else {
            self.branch = "invalid_scale_or_price";
            return Ok(());
        };
        let (mr, long, short) = self.setups();
        let selected = if bar.ts_local.time() >= time(self.clock() + self.cfg.mr_start_delay, 0)
            && bar.ts_local.time() <= time(self.clock() + self.cfg.mr_horizon - 1, 40)
            && mr
        {
            Some((Component::MR, Side::Short))
        } else if bar.ts_local.time() >= time(self.clock() + self.cfg.bo_wait, 0) {
            if long && !self.used_long {
                Some((Component::BO, Side::Long))
            } else if short && !self.used_short {
                Some((Component::BO, Side::Short))
            } else {
                None
            }
        } else {
            None
        };
        let Some((owner, side)) = selected else {
            self.branch = "no_eligible_candidate";
            return Ok(());
        };
        let terminal = if owner == Component::MR {
            time(self.clock() + self.cfg.mr_horizon, 0)
        } else {
            time(23, 30)
        };
        if boundary.time() >= terminal || next.is_none() {
            self.events.push(Event {
                model_id: self.cfg.model_id.clone(),
                role: "entry_suppressed_terminal".into(),
                ts: boundary,
                signal_ts: bar.ts_local,
                component: owner,
                side,
                price: None,
                reason: "terminal_guard".into(),
            });
            self.branch = "entry_suppressed_terminal";
            return Ok(());
        }
        ensure!(
            next == Some(boundary),
            "no consecutive execution bar for entry"
        );
        self.pending = Some(Pending::Entry(Entry {
            owner,
            side,
            signal: bar.ts_local,
            action: boundary,
            scale,
            anchor: if owner == Component::MR {
                mr_anchor.unwrap()
            } else {
                bo_anchor.unwrap()
            },
            stop1: bo_anchor.unwrap() + side.sign() * 0.51 * scale,
            stop2: bo_anchor.unwrap() - side.sign() * 0.35 * scale,
        }));
        if owner == Component::MR {
            self.mr_scheduled += 1;
            self.branch = "mr_selected";
        } else {
            self.branch = "bo_selected";
        }
        Ok(())
    }

    pub(super) fn start_frozen(&mut self, previous: Metrics, previous2: Metrics) -> Result<()> {
        ensure!(
            self.metrics.is_none(),
            "frozen session requires a new engine"
        );
        ensure!(
            self.position.is_none() && self.pending.is_none(),
            "unresolved session"
        );
        ensure!(
            self.cfg.family == Family::PreviousDay,
            "frozen context requires Previous-day"
        );
        self.previous = Some(previous);
        self.previous2 = Some(previous2);
        Ok(())
    }

    pub(super) fn step_frozen(&mut self, bar: &Bar) -> Result<()> {
        // Do not call observe(): its daily roll would overwrite the frozen anchors.
        if self.metrics.is_none() {
            self.metrics = Some(Metrics::new(bar));
        }
        self.metrics.as_mut().unwrap().update(bar)?;
        self.branch = "not_evaluated";
        self.apply_pending(bar)?;
        let next =
            (bar.ts_local.time() < time(23, 40)).then_some(bar.ts_local + Duration::minutes(10));
        self.decide(bar, next)?;
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
        mut emit: impl FnMut(&str, &Self, &Bar) -> Result<()>,
    ) -> Result<()> {
        self.cfg.validate()?;
        validate_raw(bars)?;
        validate_sessions(bars, &self.cfg)?;
        ensure!(
            bars.iter().all(|b| self.cfg.eligible(b)),
            "unfiltered model bars"
        );
        for (index, bar) in bars.iter().enumerate() {
            self.observe(bar)?;
            self.branch = "not_evaluated";
            emit("bar_start", self, bar)?;
            if self.apply_pending(bar)? {
                emit("pending_applied", self, bar)?;
            }
            emit("observed", self, bar)?;
            let next = bars
                .get(index + 1)
                .filter(|b| b.ts_local.date() == bar.ts_local.date())
                .map(|b| b.ts_local);
            self.decide(bar, next)?;
            emit("bar_end", self, bar)?;
            if next.is_none() {
                ensure!(
                    self.position.is_none() && self.pending.is_none(),
                    "unresolved end-of-session state"
                );
            }
        }
        Ok(())
    }
}

pub fn round_tick(price: f64) -> f64 {
    (price / 0.01 + 0.5).floor() * 0.01
}
