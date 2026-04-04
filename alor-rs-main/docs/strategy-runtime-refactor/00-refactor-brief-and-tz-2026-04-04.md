# Strategy Runtime Refactor Brief And TZ

Date: 2026-04-04

## Goal

Refactor `strategy-runtime` into an extensible strategy host for Alor strategies while preserving:

- the current `alor-gateway` broker/control contour,
- current Redis protocol,
- current operational semantics,
- current live/paper/replay applicability,
- and the model `one runtime instance = one strategy`.

The immediate first-stage objective is to make the third Alor strategy connect symmetrically without growing runtime core special cases in:

- config,
- state,
- runtime lifecycle,
- recovery,
- tagging,
- tracked order ids,
- risk-status logic.

## Scope Constraint

Supported model for this task:

- one runtime instance serves one strategy,
- multiple runtime processes are allowed,
- broker/gateway contour may be shared.

Explicitly out of scope:

- multiple independent strategies on one `portfolio + symbol`,
- strategy-level netting on shared broker truth,
- splitting symbol-level broker position truth across strategies,
- building a new runtime/gateway transport contour.

## Current State Confirmed In Code

The current codebase already shows the need for refactor:

1. `StrategyConfig` in `src/lib.rs` is monolithic and mixes:
   - generic runtime fields,
   - strategy-specific fields,
   - `to_*_config()` adapters for individual strategies.
2. `StrategyState` in `src/state.rs` already contains large strategy-specific branches.
3. `Runtime::new()` still creates strategies through manual `match` branching by `StrategyKind`.
4. Runtime operational logic still contains hidden strategy-specific knowledge in places such as:
   - `pending_request_ids()`
   - `strategy_exit_risk_status()`
   - `strategy_state_order_ids()`
   - `intent_comment_tag()`
5. `parse_strategy_kind()` currently has a dangerous silent fallback path and must become strict.

## Existing Strategy Kinds That Must Be Covered

This refactor is not limited to only the two production strategies.

All current kinds must pass through the new registration path:

- `SessionGapStandalone`
- `HybridIntraday`
- `LimitCancel`
- `MarketBuyAndClose`
- `ToySessionTiming`
- `MockLiveProbe`

The end state must also prepare a fully wired skeleton for a future third Alor strategy.

## What Must Stay Stable

Do not change the architecture role of:

- `alor-gateway`
- current reconnect/recovery semantics owned by gateway
- current Redis command/ack/order/position protocol
- current operational live contour

Do not migrate Alor execution into a separate barter-like transport framework.

## Target End State

`strategy-runtime` should become:

1. a strategy host,
2. with a clear internal strategy API layer,
3. with symmetric strategy integration through registry/factory,
4. with common runtime config/state separated from strategy payloads,
5. with runtime core no longer hand-knowing strategy-specific tagging/order/risk internals.

Expected integration path for any strategy:

1. register `StrategyKind`,
2. register descriptor/factory,
3. register strategy-specific config payload,
4. register strategy-specific state payload,
5. connect adapter/hooks,
6. run through common lifecycle path.

## Required Architectural Direction

### 1. Module-first, crate-later

The first refactor pass should introduce an internal strategy API module inside `strategy-runtime`, not immediately a new crate.

Rationale:

- current dependencies are still tight,
- `StrategyCtx` is runtime-local,
- lifecycle hook types are still coupled to runtime types,
- config/state are not yet cleanly split.

So the preferred order is:

- first extract an internal `strategy_api` / `strategy_host` module,
- consider a crate split only after the internal seams become stable.

### 2. Registry/factory for all strategy kinds

The new registry/factory path must cover all existing strategy kinds, not only the production two.

This is a hard requirement because otherwise the refactor would only relocate special cases instead of removing them.

### 3. Minimal capabilities only

Capabilities should be introduced only where runtime genuinely branches on them.

Acceptable initial examples:

- `requires_bootstrap_snapshot`
- `requires_runtime_state_restore`
- `uses_stop_orders`
- `requires_position_truth`
- `requires_history_warmup`
- `supports_paper_sim`
- `requires_trading_periods`

Avoid decorative future-only meta-flags in the first pass.

### 4. Config split

The target config model should become:

- common runtime/strategy host config,
- plus a strategy-specific payload per strategy.

Also required:

- unknown `strategy_kind` must become a hard config error,
- no silent fallback is allowed.

### 5. State envelope

The target state model should become:

- runtime-common state,
- plus strategy state envelope:
  - `strategy_kind`
  - `state_version`
  - `payload`

Backward compatibility must explicitly cover:

- legacy config loading or migration path,
- legacy runtime state loading,
- legacy JSON shape already expected in tests.

### 6. Remove hidden strategy knowledge from runtime core

The refactor is not done until strategy-specific behavior is moved out of runtime core in:

- pending request bookkeeping,
- exit risk status,
- tracked order ids,
- comment tagging.

Comment tagging is especially important because it is currently effectively special-cased for `HybridIntraday`.

### 7. Standardized lifecycle hooks

The lifecycle path must be explicit and documented for all strategies:

- `on_bootstrap_snapshot`
- `on_runtime_state_restored`
- `on_bar`
- `on_ack`
- `on_order`
- `on_stop_order`
- `on_position`

The runtime should minimize accidental strategy-specific branching outside these common hooks.

### 8. Strategy adapter pattern

Every strategy should end up with a clear structure:

- strategy core,
- strategy adapter,
- strategy config payload,
- strategy state payload.

At minimum, `SessionGapStandalone` and `HybridIntraday` must be brought to the same integration pattern.

### 9. Structured strategy audit logging

First pass must add structured audit logging without forcing a new Redis audit stream.

Minimum desirable events:

- signal generated,
- intent emitted,
- intent blocked,
- bootstrap processed,
- runtime state restored,
- pending recovery started/finished,
- order/position acknowledgement on strategy side.

### 10. Wired third-strategy skeleton

The third strategy skeleton must be a real wired-in component, not a dead placeholder file set.

Expected wiring:

- strategy kind,
- registry/factory,
- config parsing,
- state envelope,
- capability descriptor,
- lifecycle wiring.

Business logic may remain skeletal in this first phase, but infrastructure wiring must be complete.

## Recommended PR Order

To minimize live contour risk, the recommended sequence is:

1. docs, inventory, invariant freeze, current special-case map
2. internal `strategy_api` / `strategy_host` module extraction
3. registry/factory for all current strategy kinds
4. minimal capabilities
5. config split + strict `strategy_kind` parsing + compatibility path
6. state envelope + legacy compatibility + legacy JSON shape tests
7. removal of runtime special cases:
   - pending request ids
   - risk status
   - tracked order ids
   - comment tagging
8. lifecycle standardization + strategy adapters
9. structured strategy audit logging
10. fully wired third strategy skeleton + regression pass

## Definition Of Done

The work is done only when all of the following are true:

- broker/gateway contour is unchanged in meaning,
- `strategy-runtime` still supports `one runtime instance = one strategy`,
- shared-symbol multi-strategy ownership remains explicitly out of scope,
- all existing strategy kinds go through one registry/factory path,
- config is no longer monolithic,
- unknown `strategy_kind` no longer silently falls back,
- state is split into runtime-common plus strategy envelope,
- legacy config and state remain readable or have an explicit migration path,
- legacy JSON shape compatibility is tested,
- runtime special-case knowledge is removed from the listed core helpers,
- `SessionGapStandalone` and `HybridIntraday` are integrated symmetrically,
- structured strategy audit logging exists,
- third strategy skeleton is fully wired,
- tests remain green.

## Initial Discussion Notes

Current reading of the task suggests the following implementation principles:

1. Start with documentation and contract freeze before code moves.
2. Keep the first real code PR narrow and mechanical: introduce internal API module without semantic change.
3. Treat registry/factory and config strictness as the first high-leverage architectural cut.
4. Delay aggressive state-model surgery until compatibility tests are in place.
5. Move runtime special cases only after adapter/descriptor surfaces exist, otherwise the logic just relocates into a new giant helper.
6. Keep the third strategy skeleton infrastructure-first; do not mix it with real trading logic too early.

## Working Branch

Branch created for this stream of work:

- `feature/runtime-third-strategy-refactor`
