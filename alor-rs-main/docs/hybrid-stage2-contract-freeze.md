# Hybrid Stage-2 Contract Freeze (v1.1 addendum)

Status: `P0 decisions locked`  
Scope: `strategy-runtime + gateway integration`, without changes to hybrid core logic.

This addendum fixes ambiguous parts of Stage-2 ToR before implementation.

## 1. Time Source Policy (P0)

- Strategy/execution state decisions use **event time** from runtime events:
  - bars (`BarEvent.close_time_utc`),
  - positions/orders/stop-orders timestamps from transport payloads.
- `valid_until`, `cooldown_until`, `repair_deadline`, `sl_escalate_timeout` are stored and compared in event-time domain.
- Runtime keeps monotonic strategy time:
  - `strategy_now_ts_utc = max(last_now_ts_utc, event_ts_utc)`,
  - if `event_ts_utc <= 0`: set `event_ts_utc = last_now_ts_utc` (no time advance) and continue.
- Timeout comparisons must use saturating arithmetic to avoid regressions on out-of-order events.
- Wall clock is allowed only for infrastructure watchdogs/telemetry, not for strategy transitions.

## 2. Position Cardinality & Ambiguous Snapshot (P0)

- Hybrid runtime instance is single-symbol/single-position by contract.
- If bootstrap snapshot contains:
  - more than one non-zero position for target symbol, or
  - conflicting symbol/portfolio mapping for active position,
  then strategy enters `SafeMode(close_only)` and blocks new entries.
- Recovery action in this state is manual operator intervention or forced flatten path from runbook.

## 3. Partial Fill Baseline Policy (P0 baseline, P1 extension)

- Stage-2 P0 policy: state transitions are driven by `PositionEvent qty transitions` only.
- Partial fills are accepted as informational events and logged, but do not trigger bracket resize/replace logic in P0.
- P1 extension may add dynamic qty-aware TP/SL updates with rate limiting.

## 4. Cancel/Repair Retry Boundaries (P0)

- For cancel and repair actions: bounded retry only.
- Required config defaults:
  - `max_cancel_retries = 3`
  - `max_repair_retries = 3`
  - exponential backoff with capped delay (`repair_backoff_max_sec`).
- After retries exhausted:
  - no tight loop,
  - enter or stay in `SafeMode(close_only)`,
  - emit explicit reason in logs/health.

## 5. FeatureBuilder Silence Gate (P0)

- Keep `next available bar` semantics for progression.
- Silence is computed as:
  - `bar_gap_sec = cur_bar.close_time_utc.saturating_sub(prev_bar.close_time_utc)`.
- Add stale-input protection for entries:
  - if `bar_gap_sec > max_silence_bars_sec`, block ENTRY on current bar.
  - exits/cancel/repair remain allowed.
- If `prev_bar.close_time_utc <= 0` or `cur_bar.close_time_utc <= 0`, silence gate is not applied.
- ENTRY is blocked only on the first bar where large gap is detected (the current bar of that gap).
- This gate is additional to warmup/day-aggregate readiness gate.

## 6. Protocol/Gateway Rollout Sequence (P0)

- Mandatory rollout order:
  1. Deploy gateway/protocol changes (`stopLimit` commands + stop-orders WS stream support).
  2. Verify gateway emits stop-order events/snapshots.
  3. Deploy runtime with `hybrid_intraday`.
- Runtime hybrid must not be enabled against gateway versions without stop-order support.

## 7. Entry-Only Gate Placement and Runtime Guard (P0)

- Selected Stage-2 policy: **intent-aware gating** (runtime + wrapper compatible).
- Entry-only gate remains in `hybrid_intraday` wrapper:
  - blocks `ENTRY` in no-trade windows/weekends/warmup/not-ready/stale-input,
  - allows `EXIT`, `CANCEL`, `repair`, `OCO cleanup`.
- Runtime guard must classify intents and never drop non-entry intents:
  - `IntentClass::Entry`,
  - `IntentClass::Exit`,
  - `IntentClass::CancelCleanup`,
  - `IntentClass::ProtectiveRepair`.
- Normative mapping for classification:
  - `Entry`: `MarketEntry`, `PlaceEntryLimit/MarketableLimit`, any intent increasing exposure.
  - `Exit`: `MarketExit/PlaceExit` (if present), intents reducing exposure toward flat.
  - `CancelCleanup`: `Cancel(order_id)`, `CancelAll`, `DeleteStopLimit(stop_id)`, OCO cleanup actions.
  - `ProtectiveRepair`: `PlaceTP/ReplaceTP`, `CreateStopLimitSL/ReplaceStopLimitSL` (or delete+create), any protection repair for an already open position.
- Closed/Break/silence restrictions apply to `Entry` only.
- Runtime guard must not drop `Exit`, `CancelCleanup`, or `ProtectiveRepair`.
- At Closed/Break, adapter handles `ProtectiveRepair` with defer/backoff (`next_repair_at_ts`) and bounded retries; no tight loop. On next Open, repair is retried.
- This resolves current `drop whole batch` behavior conflict in `runtime.rs` and is mandatory before Stage-2B integration tests.

## 8. Backward Compatibility for Intent Class (P0)

- Intent payload uses backward-compatible field:
  - `intent_class: Option<IntentClass>` with `#[serde(default)]`.
- Legacy messages with `intent_class = None` are treated as `Entry` by default.
- `hybrid_intraday` must always set explicit `intent_class`.

## 9. Acceptance for Stage-2A

Stage-2A is complete when:

- this addendum is committed,
- implementation tasks reference this file as normative behavior for ambiguous cases,
- no unresolved P0 ambiguity remains before Stage-2B coding.
