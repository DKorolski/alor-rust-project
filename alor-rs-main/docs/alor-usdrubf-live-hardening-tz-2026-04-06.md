# Alor USDRUBF Live Hardening TZ (2026-04-06)

## Status and Engineering Verdict

- Replay-core migration is usable and integrated into the new host path.
- Live-ready strategy hardening is not complete.
- Current no-go for live soak; go for controlled replay/paper hardening.
- Main risk is startup/live operational semantics, not only signal math.

Observed risk signatures in logs:
- startup accepted/rejected commands and orphan trades from historical tails,
- `intent_dropped_bar_silence`,
- `intent_dropped_by_trading_window`,
- `strategy_state_transition_reverted ... reason="intent_dropped_before_emit"`,
- restored state/time context from prior sessions (for example, February state in April runtime session).

## Scope and Non-Goals

### In scope

- hardening `alor_usdrubf_hybrid` to live-aware behavior in existing `strategy-runtime` architecture,
- preserving replay parity while improving live semantics,
- improving startup/restore behavior, dedupe, broker-truth transitions, and strategy-level observability.

### Out of scope

- changing `alor-gateway`,
- changing Redis protocol/contracts (`command/ack/order/position`),
- introducing multi-strategy ownership on one symbol,
- changing one-runtime-instance-per-strategy model,
- reworking already accepted host architecture.

## Primary Objective

Ensure the strategy:
- does not emit entry path on startup stale bars,
- restores full operational state (not only display-level state),
- does not finalize live execution-sensitive transitions only from `on_bar`,
- exposes strategy-level operational logs/hooks comparable to mature strategies,
- preserves replay parity (`golden/test/train`).

## P0: Mandatory Before Next Live Run

### P0.1 Bar dedupe

- Add strict dedupe/monotonic processing for bars.
- If a bar is not newer than the last processed working bar, it must not:
  - mutate state,
  - emit intents.

### P0.2 Startup replay guard

- Introduce startup suppression for replay/backlog tail.
- Add explicit lifecycle gate: `bootstrapping -> replay_tail -> live_ready`.
- Entry intents are forbidden before `live_ready`.

### P0.3 Full state round-trip

- Extend strategy payload and `set_state()` restore to include operational internals:
  - pending entry metadata,
  - open position metadata (`owner/reason/entry_ts/entry_price/size/stops/takes`),
  - session/day markers and per-day flags affecting decisions,
  - cash/ledger fields if strategy owns them,
  - request-tracking metadata needed for reconciliation.

### P0.4 Broker-truth-driven live transitions

- In live mode, do not treat transitions as final based only on `on_bar` intent path.
- Execution-sensitive transitions must be finalized through broker truth callbacks:
  - `on_ack`,
  - `on_order`,
  - `on_position`,
  - `on_stop_order`.

### P0.5 Stale-bar suppression for entry/open path

- Explicitly prevent stale/recovered bars from:
  - creating pending entry,
  - promoting pending -> open,
  - triggering session reset + immediate entry path.

### P0.6 Strategy-owned operational hooks

- Implement strategy hooks where needed for live semantics:
  - `pending_request_ids()`,
  - `tracked_order_ids()`,
  - `intent_comment_tag()`,
  - `exit_risk_status()`.

Do not rely only on neutral host defaults for strategy-specific behavior.

### P0.7 Strategy-level observability

- Add strategy-level service logs for:
  - bootstrap summary,
  - runtime-state restore summary,
  - replay guard armed/cleared,
  - stale/not-ready suppression reasons,
  - restore details (pending/open/session),
  - broker-truth reconciliation results.

## P1: Next Layer

### P1.1 Warmup decision (enable or justify no-warmup)

- If strategy depends on session/day aggregates, enable and validate history warmup, or
- document deterministic alternative proving warmup is not needed.

### P1.2 Split research core vs live adapter

- Move toward explicit separation:
  - research core: deterministic signal/replay logic,
  - live adapter: startup/recovery suppression, broker truth, emission safety.

### P1.3 Capability descriptor review

- Re-check strategy capability flags after hardening so descriptor reflects real operational behavior.

### P1.4 Strategy-level readiness/risk projection

- Expose strategy transitional risk via `exit_risk_status()` where applicable.

## P2: Desirable Improvements

- richer startup reconciliation diagnostics for orphan/foreign tails,
- additional strategy audit events:
  - `replay_tail_bar_ignored`,
  - `startup_entry_suppressed`,
  - `restore_incomplete_reconciled`,
  - `broker_truth_open_confirmed`,
  - `broker_truth_close_confirmed`,
- one-shot startup diagnostic summary event/log.

## Required Tests

### Existing required tests

- `T1` startup stale-bar suppression,
- `T2` restore consistency for pending/open,
- `T3` warmup no-live-orders behavior,
- `T4` runtime guard interaction,
- `T5` broker-truth reconciliation under event ordering variance,
- `T6` replay parity regression (`golden/test/train` with actions/trades/summary).

### Additional mandatory tests

- `T7` duplicate/replayed bar dedupe:
  - no second state mutation,
  - no duplicate intent/request.
- `T8` state round-trip equivalence:
  - behavior after restore is functionally equivalent for pending/open contexts.
- `T9` clean-start vs dirty-start:
  - dirty-start does not trade recovery tail as live,
  - clean-start has no false suppress/revert artifacts.

## Implementation Order

- Stage A: `P0.1-P0.3`
- Stage B: `P0.4-P0.5`
- Stage C: `P0.6-P0.7` + `P1.1`
- Stage D: `P1.2-P1.4`
- Stage E: `T1-T9` full regression

## Acceptance Criteria

All must hold:

1. Startup stale bars cannot trigger premature entry path.
2. `set_state()` restores working state, not only state label.
3. Live execution-sensitive transitions are not finalized only from bar path.
4. Logs explain why strategy restored/suppressed/entered `live_ready`.
5. Replay parity remains non-regressed.
6. Paper/runtime run no longer shows:
   - stale entry on startup,
   - repeated revert symptoms from startup tail,
   - incoherent pending/open after restore.

## Clean Diagnostic Run Protocol (Before Next Soak Attempt)

Use a dedicated clean run to isolate strategy behavior from historical stream tails:

1. Runtime config:
   - `reset_state_on_start = true`
   - new `consumer_group` for diagnostic run
   - fresh `runtime_state` stream name (or cleared old one)
2. First pass mode:
   - paper, or live with `allow_live_orders = false`, or close-only guard
3. Capture mandatory evidence:
   - bootstrap/restore summary,
   - replay guard armed/cleared,
   - first live-ready bar marker,
   - first allowed entry (if any).

## Redis Tail Hygiene Notes

Potential cause for noisy startup: stale data from prior test cycles in existing streams/groups.

Recommended sequence:

1. Prefer new `consumer_group` and fresh `runtime_state` stream for each diagnostic cycle.
2. Snapshot current stream tails before cleanup.
3. If cleanup is required, perform controlled stream/group reset only for target strategy streams and only in dedicated diagnostic window.

This avoids mixing fresh strategy behavior with historical pending/recovery tails.
