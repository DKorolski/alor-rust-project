//! Isolated, opt-in RI context experiment. No Strategy implementation is exposed
//! by the facade, no Intent leaves it, and existing runtime registration is unchanged.
use anyhow::{ensure, Context};
use sha2::{Digest, Sha256};

use super::*;
use crate::ri_context::{
    ContextIdentity, ContextInput, ContextPolicy, ContextRoute, FrozenContext, RiContextStore,
};

const VERSION: &str = "ri_frozen_prospective_shadow_v2";
const MAX_WITNESS_BYTES: usize = 256 * 1024;

fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

#[derive(Debug, Clone)]
pub(super) struct FrozenSession {
    frozen: FrozenContext,
    anchor_bars: Vec<ModelBar>,
}

impl FrozenSession {
    fn load(store: &RiContextStore, date: NaiveDate) -> Result<Self> {
        let frozen = store
            .frozen(date)
            .context("shadow context is not frozen")?
            .clone();
        let anchor_bars = if frozen.ready() {
            store
                .frozen_anchor_inputs(date)?
                .into_iter()
                .map(|input| model_bar(&input.bar))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        };
        Ok(Self {
            frozen,
            anchor_bars,
        })
    }

    pub(super) fn daily_stats(&self, date: NaiveDate) -> Vec<(NaiveDate, RiDailyStats)> {
        if date != self.frozen.session_date || !self.frozen.ready() {
            return vec![];
        }
        [
            self.frozen.previous2.as_ref().unwrap(),
            self.frozen.previous.as_ref().unwrap(),
        ]
        .into_iter()
        .map(|anchor| {
            (
                anchor.session_date,
                RiDailyStats {
                    high: anchor.high,
                    low: anchor.low,
                    close: anchor.close,
                    bars: anchor.distinct_bars,
                    first_bar: local(anchor.first_bar_utc)
                        .expect("validated frozen timestamp")
                        .time(),
                    last_bar: local(anchor.last_bar_utc)
                        .expect("validated frozen timestamp")
                        .time(),
                },
            )
        })
        .collect()
    }

    pub(super) fn diagnostic_bars(&self, date: NaiveDate, observed: &[ModelBar]) -> Vec<ModelBar> {
        if date != self.frozen.session_date || !self.frozen.ready() {
            return vec![];
        }
        self.anchor_bars
            .iter()
            .copied()
            .chain(
                observed
                    .iter()
                    .copied()
                    .filter(|b| b.ts_local.date() == date),
            )
            .collect()
    }
}

fn local(ts: i64) -> Result<NaiveDateTime> {
    chrono::DateTime::from_timestamp(ts, 0)
        .and_then(|d| d.naive_utc().checked_add_signed(chrono::Duration::hours(3)))
        .context("invalid RI shadow timestamp")
}

fn model_bar(bar: &BarEvent) -> Result<ModelBar> {
    Ok(ModelBar {
        ts_local: local(bar.close_time_utc)?,
        open: bar.o,
        high: bar.h,
        low: bar.l,
        close: bar.close,
        volume: bar.v,
    })
}

/// Exact config identity, not a looser profile label. No implicit defaults for
/// breaks, clock, completeness or BO risk filters in the matched context pair.
pub fn context_identity(
    config: &RiAuthor4142LiveConfig,
    receipt_stream: String,
    policy: ContextPolicy,
) -> Result<ContextIdentity> {
    config.validate()?;
    ensure!(
        config.mode == RiAuthor4142RuntimeMode::ProspectiveShadow && !config.allow_order_emission,
        "frozen context requires prospective shadow, never live"
    );
    ensure!(
        config.model_feed_policy == Some(RiAuthor4142ModelFeedPolicy::MicroRegularV1),
        "explicit micro_regular_v1 required"
    );
    ensure!(
        config.timezone_offset_hours == 3
            && config.session_start_time == "07:00:00"
            && config.session_end_time == "23:49:59",
        "unsupported frozen shadow session clock"
    );
    ensure!(
        config.author41_entry_end_time == "10:00:00"
            && config.author41_time_exit == "20:00:00"
            && config.author42_exit_time == "23:00:00",
        "frozen shadow preserves canonical07 windows"
    );
    ensure!(
        config.author42_last_entry_time.is_none() && config.author42_max_entries_per_day.is_none(),
        "context pair must not inherit BO max2/last-entry filters"
    );
    ensure!(
        config.min_anchor_bars == 92
            && config.anchor_first_bar_at_or_before == "07:10:00"
            && config.anchor_last_bar_at_or_after == "23:30:00",
        "canonical anchor rule mismatch"
    );
    ensure!(
        config.anchor_transition_date.as_deref() == Some("2026-07-14")
            && config.pre_transition_min_anchor_bars == Some(80)
            && config
                .pre_transition_anchor_first_bar_at_or_before
                .as_deref()
                == Some("09:10:00")
            && config.pre_transition_anchor_last_bar_at_or_after.as_deref() == Some("23:30:00"),
        "transition anchor rule mismatch"
    );
    ensure!(
        config.qty.is_finite() && config.qty > 0.,
        "invalid shadow quantity"
    );
    let identity = ContextIdentity {
        profile_id: config.profile_id.clone(),
        symbol: config.symbol.clone(),
        numerical_config_sha256: digest(&(
            VERSION,
            config,
            format!("{:?}", Author41Config::ri_plateau_short_source()),
            format!("{:?}", Author41Config::ri_plateau_long_source()),
            format!("{:?}", Author42Config::ri_grid_k042_both()),
        ))?,
        model_feed_policy: "micro_regular_v1".into(),
        receipt_stream,
        policy,
        model_open_hour: 7,
        excluded_dates: config
            .excluded_model_dates
            .iter()
            .map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d"))
            .collect::<std::result::Result<_, _>>()?,
    };
    identity.validate()?;
    Ok(identity)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowStep {
    pub sequence: u64,
    pub bar_ts_utc: i64,
    pub received_at_utc: Option<i64>,
    pub durable_at_utc: Option<i64>,
    pub origin: DataOrigin,
    pub disposition: String,
    pub frozen_context_sha256: Option<String>,
    pub records: Vec<RiAuthor4142JournalRecord>,
    pub state_trace: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShadowGeneration {
    pub id: String,
    /// Collect earlier sessions as context only, without retrospectively trading
    /// warmup. The first decision session still requires a durable frozen view.
    pub first_decision_session: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RecoveryWitness {
    version: String,
    generation: ShadowGeneration,
    identity: ContextIdentity,
    sequence: u64,
    ingress_sha256: String,
    used_contexts: BTreeMap<NaiveDate, String>,
    decision_chain_sha256: String,
    state_sha256: String,
}

#[derive(Debug, Clone)]
pub struct RiFrozenShadowAdapter {
    strategy: RiAuthor4142LiveStrategy,
    identity: ContextIdentity,
    generation: ShadowGeneration,
    sequence: u64,
    last_model_ts: Option<i64>,
    used_contexts: BTreeMap<NaiveDate, String>,
    decision_chain: String,
}

impl RiFrozenShadowAdapter {
    pub fn new(
        config: RiAuthor4142LiveConfig,
        identity: ContextIdentity,
        generation: ShadowGeneration,
    ) -> Result<Self> {
        ensure!(
            context_identity(&config, identity.receipt_stream.clone(), identity.policy)?
                == identity,
            "shadow config/context identity mismatch"
        );
        ensure!(
            !generation.id.trim().is_empty()
                && generation.id.len() <= 512
                && generation
                    .first_decision_session
                    .weekday()
                    .num_days_from_monday()
                    < 5
                && !identity
                    .excluded_dates
                    .contains(&generation.first_decision_session),
            "invalid shadow generation"
        );
        let mut strategy = RiAuthor4142LiveStrategy::new(config)?;
        strategy.shadow_branches = Some(Vec::new());
        Ok(Self {
            strategy,
            identity,
            generation,
            sequence: 0,
            last_model_ts: None,
            used_contexts: BTreeMap::new(),
            decision_chain: digest(&VERSION)?,
        })
    }

    pub fn process(&mut self, store: &RiContextStore, input: &ContextInput) -> Result<ShadowStep> {
        // No externally visible action/state is released on failed validation.
        let mut next = self.clone();
        let result = next.process_inner(store, input)?;
        *self = next;
        Ok(result)
    }

    fn process_inner(
        &mut self,
        store: &RiContextStore,
        input: &ContextInput,
    ) -> Result<ShadowStep> {
        ensure!(
            store.identity() == &self.identity,
            "wrong shadow context store"
        );
        let route = store.verified_route(input)?;
        ensure!(
            self.sequence < 10_000,
            "shadow receipt limit; explicit rotation required"
        );
        ensure!(
            input.sequence == self.sequence + 1,
            "shadow input must advance exactly once in receipt order"
        );
        if let Some(branches) = &mut self.strategy.shadow_branches {
            branches.clear();
        }
        let mut disposition = format!("context_only:{route:?}");
        let mut context_sha = None;
        if route == ContextRoute::ContextOnly
            && input.bar.origin == DataOrigin::Live
            && local(input.bar.close_time_utc)?.date() >= self.generation.first_decision_session
        {
            let dt = local(input.bar.close_time_utc)?;
            let received_date = local(input.received_at_utc.context("missing receipt")?)?.date();
            if dt.date() == received_date {
                ensure!(
                    self.last_model_ts
                        .is_none_or(|last| input.bar.close_time_utc > last),
                    "non-monotonic prospective bar; do not reorder received data"
                );
                let view = FrozenSession::load(store, dt.date())?;
                let sha = view.frozen.sha256()?;
                if let Some(old) = self.used_contexts.get(&dt.date()) {
                    ensure!(old == &sha, "shadow context changed within session");
                }
                if self
                    .last_model_ts
                    .is_some_and(|last| local(last).map(|d| d.date() != dt.date()).unwrap_or(true))
                {
                    ensure!(
                        self.strategy.live_mr.position.is_none()
                            && self.strategy.live_bo.position.is_none()
                            && self.strategy.live_bo.pending.is_none(),
                        "shadow unresolved position/pending across session boundary"
                    );
                }
                self.used_contexts.insert(dt.date(), sha.clone());
                context_sha = Some(sha);
                disposition = if view.frozen.ready() {
                    "prospective_observed"
                } else {
                    "context_not_ready"
                }
                .into();
                self.strategy.shadow_context = Some(view);
                let ctx = StrategyCtx {
                    strategy_id: self.generation.id.clone(),
                    portfolio: String::new(),
                    exchange: "MOEX".into(),
                    symbol: self.identity.symbol.clone(),
                    tick_size: 10.,
                    trade_mode: crate::TradeMode::Paper,
                    paper_execution_mode: crate::PaperExecutionMode::LiveOnly,
                    allow_live_orders: false,
                    gateway_phase: crate::live_guard::GatewayPhase::LiveReady,
                    position_qty: None,
                    event_ts_utc: input.bar.close_time_utc,
                    now_ts_utc: input.durable_at_utc.context("missing durable receipt")?,
                    last_bar_ts: self.last_model_ts,
                };
                let intents = self.strategy.on_bar(&ctx, &input.bar);
                ensure!(intents.is_empty(), "shadow attempted outbound Intent");
                self.last_model_ts = Some(input.bar.close_time_utc);
            } else {
                disposition = "previous_session_live_receipt_context_only".into();
            }
        }
        self.sequence = input.sequence;
        let records = std::mem::take(&mut self.strategy.journal_records);
        let result = ShadowStep {
            sequence: self.sequence,
            bar_ts_utc: input.bar.close_time_utc,
            received_at_utc: input.received_at_utc,
            durable_at_utc: input.durable_at_utc,
            origin: input.bar.origin.clone(),
            disposition,
            frozen_context_sha256: context_sha,
            records,
            state_trace: self.state_trace(),
        };
        self.decision_chain = digest(&(&self.decision_chain, &result))?;
        Ok(result)
    }

    pub fn state_trace(&self) -> serde_json::Value {
        let s = &self.strategy;
        serde_json::json!({
            "sequence": self.sequence, "last_model_ts": self.last_model_ts,
            "frozen_context": s.shadow_context.as_ref().map(|view| &view.frozen),
            "strategy_state": s.state,
            "branches": s.shadow_branches,
            "mr_entries_today_total": s.live_mr.entries_today,
            "mr_position": s.live_mr.position.as_ref().map(|p| serde_json::json!({
                "side": p.side, "entry_ts": p.entry_ts_local, "entry_price": p.entry_price,
                "prev_close": p.prev_close, "prev_range": p.prev_range, "bars_held": p.bars_held,
                "decision_key": p.decision_key,
            })),
            "bo_entries_today": s.live_bo.entries_today,
            "bo_observation_index": s.live_bo.bar_index,
            "bo_first_bar": s.live_bo.first_bar.map(|b| b.ts_local),
            "bo_context": s.live_bo.context.map(|c| serde_json::json!({
                "prev_close": c.prev_close, "prev2_close": c.prev2_close, "prev_range": c.prev_range,
                "prev_hl_ratio": c.prev_hl_ratio, "prev_ret": c.prev_ret,
            })),
            "bo_long_level": s.live_bo.long_level, "bo_short_level": s.live_bo.short_level,
            "bo_trade_allowed": s.live_bo.trade_allowed,
            "bo_pending": s.live_bo.pending.map(|p| match p {
                RiAuthor4142LiveBoPending::Entry(side) => serde_json::json!({"kind": "entry", "side": side}),
                RiAuthor4142LiveBoPending::Exit(reason) => serde_json::json!({"kind": "exit", "reason": reason}),
            }),
            "bo_position": s.live_bo.position.as_ref().map(|p| serde_json::json!({
                "side": p.side, "entry_ts": p.entry_ts_local, "entry_price": p.entry_price,
                "bars_held": p.bars_held, "decision_key": p.decision_key,
            })),
            "bo_day_high": s.live_bo.day_hh.is_finite().then_some(s.live_bo.day_hh),
            "bo_day_low": s.live_bo.day_ll.is_finite().then_some(s.live_bo.day_ll),
            "bo_was_long_today": s.live_bo.was_long_today,
            "bo_was_short_today": s.live_bo.was_short_today,
        })
    }

    fn witness(&self, store: &RiContextStore) -> Result<RecoveryWitness> {
        ensure!(
            store.identity() == &self.identity,
            "wrong witness context identity"
        );
        for (date, sha) in &self.used_contexts {
            ensure!(
                store
                    .frozen(*date)
                    .context("missing used frozen context")?
                    .sha256()?
                    == *sha,
                "witness context drift"
            );
        }
        let s = &self.strategy;
        let bars = s
            .model_bars
            .iter()
            .map(|b| (b.ts_local, b.open, b.high, b.low, b.close, b.volume))
            .collect::<Vec<_>>();
        let emitted: BTreeSet<_> = s.emitted_decision_keys.iter().collect();
        let state_sha256 = digest(&(
            self.state_trace(),
            bars,
            emitted,
            &s.shadow_path_keys_by_date,
            &s.shadow_decisions_by_key,
            format!("{:?}", s.live_mr),
            format!("{:?}", s.live_bo),
        ))?;
        Ok(RecoveryWitness {
            version: VERSION.into(),
            generation: self.generation.clone(),
            identity: self.identity.clone(),
            sequence: self.sequence,
            ingress_sha256: store.input_prefix_sha256(self.sequence)?,
            used_contexts: self.used_contexts.clone(),
            decision_chain_sha256: self.decision_chain.clone(),
            state_sha256,
        })
    }

    /// A replay witness, not a standalone operational snapshot. Persist alongside
    /// the selected frozen-store checkpoint and the complete durable input prefix.
    pub fn recovery_witness(&self, store: &RiContextStore) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(&self.witness(store)?)?;
        ensure!(bytes.len() <= MAX_WITNESS_BYTES, "oversized shadow witness");
        Ok(bytes)
    }

    /// Reconstruct ALL prospective state using original inputs and per-session
    /// frozen context. Replayed records are discarded, never republished.
    pub fn restore(
        config: RiAuthor4142LiveConfig,
        identity: ContextIdentity,
        generation: ShadowGeneration,
        store: &RiContextStore,
        inputs: &[ContextInput],
        witness: &[u8],
    ) -> Result<Self> {
        ensure!(
            witness.len() <= MAX_WITNESS_BYTES,
            "oversized shadow witness"
        );
        let expected: RecoveryWitness = serde_json::from_slice(witness)?;
        ensure!(
            expected.version == VERSION
                && expected.identity == identity
                && expected.generation == generation,
            "shadow recovery identity/version mismatch"
        );
        ensure!(
            inputs.len() as u64 == expected.sequence && inputs.len() <= 10_000,
            "incomplete or oversized shadow replay prefix"
        );
        let mut adapter = Self::new(config, identity, generation)?;
        tracing::subscriber::with_default(
            tracing::subscriber::NoSubscriber::default(),
            || -> Result<()> {
                for input in inputs {
                    adapter.process(store, input)?;
                }
                Ok(())
            },
        )?;
        ensure!(
            adapter.witness(store)? == expected,
            "shadow replay recovery mismatch; no fallback to flat"
        );
        Ok(adapter)
    }
}

#[cfg(test)]
mod tests;
