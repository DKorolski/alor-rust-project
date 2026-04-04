# Strategy Runtime Compatibility Checklist

Date: 2026-04-04

Use this as a frozen quick-check before merging later refactor PRs, especially
config/state-heavy changes.

## Checklist

- legacy config parse still works, or an explicit migration path exists
- legacy snapshot load still works
- legacy runtime state load still works
- legacy JSON shape used in restart e2e is still readable
- bootstrap order is unchanged:
  - `load_snapshots()`
  - `load_runtime_state()`
  - `notify_bootstrap_snapshot()`
  - `notify_runtime_state_restored()`
  - `warmup_strategy_indicators_from_history()`
  - pending stream recovery
- warmup still runs with `allow_live_orders = false`
- symbol-level filtering is unchanged for snapshots, bars, orders, stop-orders,
  trades, and positions
- one runtime instance still hosts one strategy
- `alor-gateway` broker/control contour is unchanged

## When To Use

Review this checklist in every PR that touches:

- config parsing
- runtime state shape
- bootstrap/recovery order
- lifecycle hook wiring
- strategy registry/factory integration

If any item changes intentionally, the PR should say so explicitly and update the
freeze docs rather than drifting silently.
