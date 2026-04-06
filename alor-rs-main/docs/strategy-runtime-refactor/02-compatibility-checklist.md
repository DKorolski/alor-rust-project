# Strategy Runtime Compatibility Checklist

Date: 2026-04-04

Use this as a frozen quick-check before merging later refactor PRs, especially
config/state-heavy changes.

## Checklist

- legacy config parse still works, or an explicit migration path exists
- when `strategy_kind` is overridden and `strategy_id` is omitted, `strategy_id`
  resolves to `strategy_kind.default_strategy_id()`
- split specific sections reject non-matching `strategy_kind` instead of being
  silently ignored
- state transition layer can deserialize both legacy enum-shaped `strategy_state`
  and envelope-shaped `strategy_state` payloads during migration
- runtime-state persistence writes envelope-shaped `strategy_state` while restore
  still accepts legacy snapshots
- runtime core no longer hardcodes hybrid-only fallback comment tags; comment tag
  fallback comes from strategy-owned hook
- runtime core no longer hardcodes `strategy_state` order id extraction; tracked
  order ids come from strategy-owned hook
- runtime core no longer hardcodes pending request extraction; pending request
  ids are sourced via strategy-owned hook
- runtime core no longer hardcodes session-gap exit risk projection; runtime
  health uses strategy-owned exit risk hook
- at least one concrete strategy (`SessionGapStandalone`) provides explicit
  overrides for pending/exit-risk hooks to reduce host default legacy knowledge
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
