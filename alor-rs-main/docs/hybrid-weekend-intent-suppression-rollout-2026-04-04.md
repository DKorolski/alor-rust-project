# Hybrid Weekend Intent Suppression Rollout

Date: 2026-04-04

## Context

`hybrid` on live soak was generating weekend `MeanReversion` entry actions, after which runtime dropped them by trading-window / weekend guards.

Observed runtime log pattern before the fix:

- `hybrid actions generated`
- `intent_dropped_market_closed state=Weekend`
- `intent_dropped_by_trading_window`
- `strategy_state_transition_reverted ... reason="intent_dropped_before_emit"`

This behavior was operationally safe, but noisy and not aligned with the original Python baseline.

## Baseline Parity Reading

Reference model:

- `pre_rust_handoff/hybrid_orchestrator_bt.py`

Key point:

- `trade_weekends=False`
- weekend bars still reach `next()`,
- but the strategy returns before orchestrator signal generation.

So the baseline expectation is:

- weekend bars may still be observed by the strategy,
- but no trading actions should be emitted on weekends.

## Runtime Patch

Commit:

- `b8165ca` `Suppress hybrid weekend signal emission`

Behavior after the patch:

- weekend bars still update `hybrid` internal state,
- warmup / day aggregates still progress,
- but the normal signal-generation path is suppressed before `orchestrator.on_bar(...)`,
- so no weekend entry / exit intents are emitted.

This uses the already existing runtime-level `weekends_off` setting from `trading_periods`.

## Rollout Method

Rollout target:

- `/opt/trading-hybrid`

Image built and pushed:

- `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-b8165ca-hybweekend-20260404-132019`

Rollout style:

- manual VPS rollout
- no `from zero`
- no Redis reset

Operational steps:

1. backup `/opt/trading-hybrid/.env`
2. switch only `RUNTIME_IMAGE_TAG`
3. `docker compose pull strategy-runtime`
4. `docker compose up -d --force-recreate strategy-runtime`

Note:

- during `docker compose up -d --force-recreate strategy-runtime`, compose also recreated `trading-hybrid-alor-gateway-1`
- Redis was not recreated
- rollout still remained a normal in-place restart, not a cold restart

## Post-Rollout Verification

Containers:

- `trading-hybrid-strategy-runtime-1` -> `dev-b8165ca-hybweekend-20260404-132019`
- `trading-hybrid-alor-gateway-1` -> `dev-71c09ac-actionscope2-20260402-131941`

Readiness after restart:

- gateway: `readiness=true`, `phase=LiveReady`
- runtime: initially `BLOCKED` for bootstrap
- runtime: later `readiness=true`, `live_guard=ALLOWED`

Important runtime log sequence after restart:

- bootstrap `BLOCKED` due to `bootstrap:missing_live_bar` / `bootstrap:not_ready`
- `signal warmup complete`
- `live_guard_changed ... to="ALLOWED"`

## Runtime State Check

Latest persisted runtime state after rollout showed:

- `entry_ready=true`
- `pending_entry_* = null`
- `pending_exit_* = null`
- `deferred_entry_* = null`
- `deferred_exit_* = null`
- `safe_mode_close_only=false`
- `prev_day_close=2760.5`
- `prev_day_range=33.5`
- `prev_day_return=-0.0026939655172413795`

This confirms:

- signal context stayed warmed,
- no stale pending/deferred intent remained,
- runtime resumed from a clean operational state.

## Main Result

In the fresh post-rollout log slice, the previous weekend noise pattern was no longer present:

- no `hybrid actions generated`
- no `intent_dropped_market_closed`
- no `intent_dropped_by_trading_window`
- no `strategy_state_transition_reverted`

This is the expected result of suppressing weekend action generation earlier in the runtime path.

## Residual Observation

There is still a separate config mismatch between runtime and gateway weekend semantics:

- runtime scheduler reports `Weekend`
- gateway readiness currently reports `scheduler_state="Open"`

This mismatch pre-existed the patch and was not part of the rollout scope.

## Decision

- accept `b8165ca` as the current `hybrid` weekend-behavior baseline
- keep this runtime in soak
- treat gateway/runtime weekend-state alignment as a separate follow-up task
