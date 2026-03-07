# Hybrid Stage-2 Contract Freeze (v1.1 addendum)

Status: `P0 decisions locked`  
Scope: `strategy-runtime + gateway integration`, without changes to hybrid core logic.

This addendum fixes ambiguous parts of Stage-2 ToR before implementation.

## 1. Time Source Policy (P0)

- Strategy/execution state decisions use **event time** from runtime events:
  - bars (`BarEvent.close_time_utc`),
  - positions/orders/stop-orders timestamps from transport payloads.
- `valid_until`, `cooldown_until`, `repair_deadline`, `sl_escalate_timeout` are stored and compared in event-time domain.
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
- Add stale-input protection for entries:
  - if bar silence exceeds configured threshold (`max_silence_bars_sec`), new entries are blocked.
  - exits/cancel/repair remain allowed.
- This gate is additional to warmup/day-aggregate readiness gate.

## 6. Protocol/Gateway Rollout Sequence (P0)

- Mandatory rollout order:
  1. Deploy gateway/protocol changes (`stopLimit` commands + stop-orders WS stream support).
  2. Verify gateway emits stop-order events/snapshots.
  3. Deploy runtime with `hybrid_intraday`.
- Runtime hybrid must not be enabled against gateway versions without stop-order support.

## 7. Entry-Only Gate Placement (P0)

- Selected variant for Stage-2: **Variant A**.
- Entry-only gate is implemented in `hybrid_intraday` wrapper:
  - blocks `ENTRY` in no-trade windows/weekends/warmup/not-ready/stale-input,
  - allows `EXIT`, `CANCEL`, `repair`, `OCO cleanup`.
- Runtime global guard remains generic and non-owner-specific.

## 8. Acceptance for Stage-2A

Stage-2A is complete when:

- this addendum is committed,
- implementation tasks reference this file as normative behavior for ambiguous cases,
- no unresolved P0 ambiguity remains before Stage-2B coding.
