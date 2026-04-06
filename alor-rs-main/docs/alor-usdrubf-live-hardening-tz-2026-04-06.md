# Alor USDRUBF Live Hardening TZ (2026-04-06)

## Review Follow-up TZ (Locked After Review)

This document is extended by a focused follow-up hardening scope for `AlorUsdrubfHybrid` inside the already refactored `strategy-runtime` host path.

### Context

- base runtime refactor is already completed (registry/adapters/host hooks path),
- `AlorUsdrubfHybrid` already passes clean-start and isolated diagnostic bring-up,
- readiness and guard behavior are proven for isolated clean-start,
- parity path (`golden/test/train`) remains green after hardening changes.

Remaining gap from review:
- strategy is not yet proven equivalent to mature strategies for non-flat restart and deep bootstrap adoption/reconcile scenarios.

### Supported startup profile (current explicit verdict)

Current formally supported profile is **Profile A (minimum)**:

- clean-start,
- fresh consumer group,
- fresh runtime-state stream,
- isolated stream namespace,
- flat account (no active position/working orders/stop orders at startup).

Not yet proven / unsupported as fully reliable production semantics:

- restart with already open position,
- restart with working orders and stop orders,
- full ownership/reconcile parity with mature runtime strategies.

### Follow-up objective

Reach controlled live-prep maturity without widening architecture scope:

- make `live_ready` semantics strict (`fresh + DataOrigin::Live` only),
- implement meaningful bootstrap adoption/reconcile for non-flat scenarios,
- align capability descriptor with real operational behavior,
- publish an explicit next-run operational protocol and honest readiness verdict.

### In scope (follow-up)

- `strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs`,
- related adapter/registry/state wiring,
- strategy unit/integration tests,
- strategy docs, bring-up docs, runbook/config instructions.

### Out of scope (follow-up)

- `alor-gateway` contract/behavior changes,
- Redis stream contract redesign,
- broker command schema changes,
- new execution contour,
- broad all-strategies rearchitecture.

### Required additional tests in this follow-up

- `T10` Bootstrap adoption with non-flat snapshot.
- `T11` Fresh recovered-origin bar does not clear `live_ready`.
- `T12` Restart with non-flat snapshot preserves conservative owner/reconcile semantics.
- `T13` Terminal reject after entry intent.
- `T14` Terminal reject after exit intent.

Recommended:

- capability descriptor consistency check in registry tests.

### Follow-up implementation progress

Implemented in current iteration:

- `PR1`: strict `live_ready` unlock now requires fresh `DataOrigin::Live` bar only.
  - fresh non-live origins (`history/history_gap/replay`) no longer clear startup guard.
  - verified by `fresh_recovered_origin_bar_does_not_clear_live_ready` (`T11`).
- `PR2`: bootstrap adoption/reconcile for non-flat snapshot added.
  - non-flat symbol position from bootstrap is adopted as broker truth into strategy open state.
  - pending entry is cleared on non-flat adoption to avoid blind duplicate entry.
  - symbol working orders are tracked at bootstrap and block unsafe blind entry until reconcile.
  - verified by `bootstrap_adoption_with_non_flat_snapshot_prevents_blind_entry` (`T10`).
- `PR2.1`: bootstrap owner refinement added.
  - when owner cannot be inferred confidently during bootstrap non-flat adoption, strategy marks lifecycle as conservative owner mode.
  - owner trust is restored only after first live `on_position` confirmation.
  - verified by `restart_with_non_flat_snapshot_keeps_owner_conservative_until_live_confirmation` (`T12`).
- `PR2.2`: reject policy hardening added.
  - entry reject policy: clear inflight + defer retry to next bar (`entry_reject_deferred_retry`),
  - exit reject policy: keep open risk state + clear inflight + defer retry (`exit_reject_deferred_retry`).
  - verified by:
    - `terminal_reject_after_entry_intent_clears_inflight_and_defers_retry` (`T13`),
    - `terminal_reject_after_exit_intent_preserves_open_risk_state` (`T14`).
- `PR3`: capability descriptor reviewed.
  - `uses_stop_orders` for `AlorUsdrubfHybrid` downgraded to `false` (callback existed but no mature stop-order ownership semantics yet).
  - registry capability expectation tests updated and green.

### Reconcile precedence policy (locked)

For startup and restart semantics, precedence is explicit:

1. live broker truth callbacks (`on_position` / `on_order` / `on_ack`) have highest precedence,
2. bootstrap snapshot adoption has second precedence,
3. restored runtime state is a lower-priority seed and may be overridden during bootstrap/live reconcile.

Operational interpretation:

- runtime state is used to restore continuity hints,
- bootstrap snapshot sets initial broker-facing startup picture for this run,
- live broker events remain final authority for execution-sensitive transitions.

### Practical PR order for this follow-up

- `PR1`: strict `live_ready` only on fresh live-origin + `T11` + docs sync.
- `PR2`: bootstrap adoption/reconcile (non-flat snapshot) + `T10` + precedence policy docs.
- `PR3`: capability descriptor review/cleanup + `T12` (or equivalent proof) + docs sync.
- `PR4`: next-run runbook finalization + controlled bring-up report update + final review memo.

### Definition of done for this follow-up

All must be true simultaneously:

1. supported startup profile is explicit and unambiguous in docs,
2. `live_ready` is cleared only by fresh `DataOrigin::Live` bar,
3. non-flat bootstrap adoption/reconcile is implemented at least at limited mature level and covered by test,
4. capability descriptor is aligned with actual strategy behavior,
5. `T10` and `T11` are green,
6. next-run protocol is reproducible and isolated from stale tails,
7. final readiness statement is honest: `Go` vs `Conditional-Go` vs `Not yet equivalent`.

## Final anti-regression gate status (legacy strategies)

Mandatory anti-regression suite for `session_gap_standalone` and `hybrid_intraday_runtime` is green:

- unit subsets for both strategies: PASS,
- required integration/e2e set:
  - `e2e_session_gap_restart`: PASS,
  - `e2e_reconnect_blocks`: PASS,
  - `e2e_hybrid_golden`: PASS,
  - `e2e_smoke`: PASS,
  - `live_guard_tests`: PASS,
  - `config_tests`: PASS,
  - `ledger_reports`: PASS,
- aggregate regression run:
  - `cargo test -p strategy-runtime`: PASS.

## Status and Engineering Verdict

- Replay-core migration is usable and integrated into the new host path.
- Live-ready strategy hardening is not complete.
- Current no-go for live soak; go for controlled replay/paper hardening.
- Main risk is startup/live operational semantics, not only signal math.

Progress note:
- Stage A (`P0.1-P0.3`) is completed in runtime code and covered by unit tests.
- Stage B is in progress; implemented parts:
  - live `on_bar` path emits intents without finalizing pending/open transitions,
  - transition finalization is moved to broker-truth callbacks (currently `on_position`),
  - in-flight guards are persisted in state payload for restart continuity.
  - non-live/recovery bar origins (`history/history_gap/replay`) are explicitly suppressed in live mode before session mutation and signal path.
- Stage C started; initial scope implemented:
  - strategy-owned hook overrides wired (`tracked_order_ids`, `pending_request_ids`, `intent_comment_tag`, `exit_risk_status`),
  - tracker fields now restored from `RuntimeStateRestored` and synced in strategy payload,
  - strategy-level service logs added on bootstrap/restore/suppression/broker-position reconciliation.
  - hook-path tests added for restore hydration, ack/order tracker updates, comment-tag format, and exit-risk projection.
- Stage D started:
  - strategy internals now separate deterministic research evaluation from live adapter orchestration through a dedicated `ResearchSnapshot` path,
  - no runtime protocol/API changes introduced; behavior locked by existing Stage A-C test suite.
  - replay parity regression gate (`T6`) re-run after Stage D split:
    - `golden`, `test`, `train` all pass in `--check` mode.
- Stage E test gate progress:
  - `T4` runtime guard interaction: live path stays blocked when `enable_live_execution=false`.
  - `T5` broker-truth reconciliation ordering variance: out-of-order late reject ACK does not break already confirmed open position.
  - `T9` dirty-start vs fresh-live transition: recovery tail is suppressed first, then fresh live bar resumes entry path.
  - parity (`T6`) re-run after Stage E additions: still green on `golden/test/train`.

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

### Current T1-T9 Coverage Matrix

- `T1` startup stale-bar suppression: covered by `stale_live_bars_are_suppressed_until_fresh_bar`.
- `T2` restore consistency for pending/open: covered by `set_state_restores_pending_entry_for_next_bar` and `set_state_restores_open_position_and_allows_exit_evaluation`.
- `T3` warmup no-live-orders behavior: covered by `warmup_keeps_trading_state_untouched_when_pending_or_open_exists`.
- `T4` runtime guard interaction: covered by `runtime_guard_blocks_live_path_when_live_execution_disabled`.
- `T5` broker-truth ordering variance: covered by `broker_truth_reconciliation_is_stable_with_out_of_order_ack`.
- `T6` replay parity regression: covered by `usdrubf_hybrid_replay --check` on `golden/test/train` (green).
- `T7` duplicate/replayed dedupe: covered by `duplicate_bar_is_ignored` and `recovered_origin_bar_is_suppressed_in_live_without_session_reset`.
- `T8` state round-trip equivalence: covered by restore tests for pending/open plus hook-state hydration (`runtime_restore_populates_pending_and_tracked_hooks`).
- `T9` clean-start vs dirty-start: covered by `dirty_start_suppresses_recovery_tail_then_allows_fresh_live_bar`.

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
