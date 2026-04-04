# Strategy Runtime Current Special Cases

Date: 2026-04-04

## Goal

Freeze the places where `runtime` currently contains strategy-specific knowledge.

This document is intentionally concrete. The first refactor passes should remove
or isolate these cases through descriptor, factory, and adapter seams instead of
simply moving the same branching into a new file.

## Highest-Value Current Special Cases

### 1. Manual Strategy Creation In Runtime::new

`Runtime::new()` contains a direct `match` over `StrategyKind` in
`strategy-runtime/src/runtime.rs:337`.

Current branches:

- `LimitCancel`
- `MarketBuyAndClose`
- `ToySessionTiming`
- `SessionGapStandalone`
- `MockLiveProbe`
- `HybridIntraday`

Why this matters:

- runtime owns construction knowledge for every strategy
- adding a new kind requires editing runtime core
- config-to-concrete-strategy adaptation is also coupled here

This is the first target for `StrategyRegistry` / `StrategyFactory`.

### 2. Monolithic StrategyConfig With Per-Strategy Adapters

`StrategyConfig` in `strategy-runtime/src/lib.rs:405` is a shared container for
generic and strategy-specific fields.

It also owns per-strategy conversion logic:

- `to_limit_cancel_config()`
- `to_market_buy_and_close_config()`
- `to_toy_session_timing_config()`
- `to_mock_live_probe_config()`
- `to_session_gap_standalone_config()`
- `to_hybrid_intraday_runtime_config()`

Why this matters:

- common config knows internal fields of all strategies
- config change for one strategy widens shared surface for all runtime code
- future third strategy would enlarge this already overloaded struct

### 3. Silent Fallback For Unknown strategy_kind

`parse_strategy_kind()` in `strategy-runtime/src/config.rs:1691` silently maps
unknown values to `StrategyKind::LimitCancel`.

Why this matters:

- invalid config can load into the wrong strategy
- runtime may start with a valid but unintended execution path
- future registry/factory behavior cannot be trustworthy while this fallback exists

This is an explicit fix item in the refactor plan.

## Runtime Core Special Cases

### 4. strategy_exit_risk_status() Knows SessionGapStandalone Internals

`strategy_exit_risk_status()` lives in `strategy-runtime/src/runtime.rs:479`.

Current behavior:

- reads `StrategyState::SessionGapStandalone`
- branches on `SessionGapLivePhase`
- derives health/readiness overrides from session-gap-specific phases

Why this matters:

- runtime readiness is not purely generic
- session gap close-only and exit-recovery semantics are hardcoded in runtime core
- any additional strategy with similar risk semantics would currently require more
  core branching

This should move to a strategy-owned risk/health hook or descriptor-provided view.

### 5. pending_request_ids() Knows Many Concrete State Variants

`pending_request_ids()` lives in `strategy-runtime/src/runtime.rs:1092`.

Current behavior:

- enumerates legacy `Placed`, `MarketBuyPending`, `MarketBuySent`, `MarketCloseSent`
- enumerates `MarketLivePendingEntry` and `MarketLivePendingExit`
- peeks inside `SessionGapStandalone.phase`
- returns nothing for `HybridIntradayRuntime`

Why this matters:

- pending request recovery is partially strategy-specific
- runtime restore path depends on matching enum variants correctly
- `HybridIntradayRuntime` already shows that adapter-owned metadata may be needed
  instead of forcing everything into a generic runtime map too early

This is a strong reason to avoid premature over-generalization of runtime-common
state. Some recovery metadata may remain strategy-owned and surfaced through hooks.

### 6. strategy_state_order_ids() Only Understands Older Flow Shapes

`strategy_state_order_ids()` lives in `strategy-runtime/src/runtime.rs:2770`.

Current behavior:

- only extracts order ids from `Placed` and `CancelSent`

Why this matters:

- tracked order id introspection is not strategy-neutral
- the helper is already incomplete for richer strategy states
- future state refactor should replace this with strategy-provided tracked ids

### 7. intent_comment_tag() Is Hardcoded For Hybrid

`intent_comment_tag()` lives in `strategy-runtime/src/runtime.rs:3167`.

Current behavior:

- only applies when `strategy_kind == HybridIntraday`
- generates `HYB|sid=...|c=...|r=...`

Why this matters:

- comment tagging is currently a runtime-owned hybrid special case
- future strategy-specific order tagging needs a symmetric hook path

This is one of the clearest examples of logic that must become descriptor or
adapter owned instead of remaining in runtime core.

## State Shape Special Cases

### 8. Giant StrategyState Mixes Old Runtime Modes And Rich Strategy Payloads

`StrategyState` in `strategy-runtime/src/state.rs:106` currently contains:

- legacy simple runtime modes
- market buy/close runtime states
- generic `Blocked`
- full `SessionGapStandalone` payload
- full `HybridIntradayRuntime` payload

Why this matters:

- runtime and strategy concerns are mixed in one enum
- adding third strategy payload directly here would worsen coupling
- helper functions in runtime are encouraged to branch on state internals

The refactor target is not "another bigger enum". It is runtime-common state plus
strategy-specific envelope/payload with compatibility coverage.

### 9. Legacy JSON Shape Is Already A Compatibility Contract

Compatibility tests in `strategy-runtime/src/state.rs:361` and
`strategy-runtime/src/state.rs:402` already guarantee old deserialization paths.

Restart e2e also relies on enum-shaped JSON access in
`strategy-runtime/tests/e2e_session_gap_restart.rs:805`.

Why this matters:

- state refactor must preserve legacy restore semantics
- it must also preserve or consciously migrate current JSON test shape
- backward compatibility is broader than serde success alone

## Operational Special Cases That Define Current Scope

### 10. Symbol-Level Truth Is Assumed Everywhere

Runtime filters snapshots and live events to `config.strategy.symbol`.

Examples:

- snapshot filtering in `strategy-runtime/src/runtime.rs:906`
- bar warmup filtering in `strategy-runtime/src/runtime.rs:774`
- order filtering in `strategy-runtime/src/runtime.rs:1491`
- stop-order filtering in `strategy-runtime/src/runtime.rs:1523`
- trade filtering in `strategy-runtime/src/runtime.rs:1556`
- position filtering in `strategy-runtime/src/runtime.rs:1679`

Why this matters:

- current runtime assumes one symbol-level truth per hosted strategy
- this reinforces that multi-strategy ownership on one symbol remains out of scope

### 11. Trading Window Drop And State Revert Are Runtime-Owned

Current live path in `apply_intents()` at `strategy-runtime/src/runtime.rs:2870`
does the following:

- resolve intent class
- drop entry intents by trading window via `trading_window_allows_order()`
- if all intents are dropped, revert strategy state via
  `restore_strategy_state_after_dropped_intents()`
- do a second guard filter and possibly revert state again

Why this matters:

- runtime currently owns the contract for "emit vs suppress vs revert"
- strategy code is not the final authority on whether emitted intent will survive
- later host abstraction must preserve this operational semantic even if wiring changes

This area should be treated carefully because it affects restart recovery, pending
state, and live observability.

### 12. Warmup And Restore Semantics Are Host-Owned, Not Strategy-Owned

Current order in `bootstrap()`:

1. snapshots
2. runtime state
3. bootstrap callback
4. runtime-state-restored callback
5. warmup from history
6. pending recovery

Why this matters:

- strategies already depend on this ordering indirectly
- future lifecycle standardization must document it instead of letting it remain implicit

## What Should Not Be Done In Early PRs

To keep the safe sequence intact, the next steps should avoid:

- moving special cases into a new file without abstraction improvement
- inventing broad capability taxonomy before runtime actually branches on it
- forcing all recovery metadata into runtime-common state too early
- touching broker/gateway contour to compensate for runtime architecture issues

## Recommended Extraction Order

The current code suggests the following practical order:

1. isolate current host-facing types into an internal strategy host/api module
2. introduce registry/factory for all existing strategy kinds
3. add descriptor or adapter hooks for:
   - construction
   - pending request ids
   - risk/health projection
   - tracked order ids
   - comment tagging
4. only then split config and state around those seams

This keeps the refactor additive and reviewable while preserving current live and
restart semantics.
