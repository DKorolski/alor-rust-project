# Strategy Runtime Current Contract

Date: 2026-04-04

## Goal

Freeze the current `strategy-runtime` contract before structural refactor work.

This document records what exists today, which surfaces are already depended on by
tests and live contour, and which invariants must survive the first refactor
passes.

## Current Operating Model

Current supported model is:

- one runtime instance = one strategy
- one runtime instance is configured for one `strategy_kind`
- runtime consumes symbol-level broker truth from shared Redis streams
- `alor-gateway` remains the broker/control plane

This is visible in the runtime wiring:

- `RuntimeConfig` hosts a single `strategy: StrategyConfig` in `strategy-runtime/src/lib.rs:292`
- `Runtime::new()` creates exactly one boxed strategy in `strategy-runtime/src/runtime.rs:337`
- snapshots and live events are filtered to `config.strategy.symbol` in `strategy-runtime/src/runtime.rs:906`

Not supported by current contour:

- multiple independent strategies sharing one runtime process
- strategy-level ownership netting on one portfolio + symbol
- splitting broker position truth into multiple strategy-owned subpositions

## Core Strategy API Today

The current strategy-facing contract lives directly in `strategy-runtime/src/lib.rs`.

### Intent Layer

`Intent` is the shared action language between strategy and runtime in
`strategy-runtime/src/lib.rs:33`.

Current intent variants:

- `Place`
- `Market`
- `Cancel`
- `Replace`
- `CreateStopLimit`
- `DeleteStopLimit`
- `Classified`

Important current behavior:

- strategies may emit explicit intent class via `Intent::with_class()`
- runtime may also infer class from raw intent via `resolve_intent_class()` in `strategy-runtime/src/runtime.rs:3191`

### Strategy Trait

`Strategy` lives in `strategy-runtime/src/lib.rs:99`.

Current lifecycle hooks:

- `on_bar`
- `on_ack`
- `on_order`
- `on_stop_order`
- `on_position`
- `on_bootstrap_snapshot`
- `on_runtime_state_restored`
- `warmup_from_history`
- `state`
- `set_state`

This is already close to a host/adapter contract, but it is not isolated into a
dedicated internal API module yet.

### Strategy Context

`StrategyCtx` lives in `strategy-runtime/src/lib.rs:129`.

Current fields show what runtime exposes to strategies:

- strategy identity: `strategy_id`
- broker/account context: `portfolio`, `exchange`, `symbol`
- execution context: `trade_mode`, `paper_execution_mode`, `allow_live_orders`
- market/control context: `gateway_phase`
- position hint: `position_qty`
- timing context: `event_ts_utc`, `now_ts_utc`, `last_bar_ts`

This matters because later refactor steps should preserve these semantics even if
the host surface is moved into `strategy_host` or similar internal module.

## Current Runtime Lifecycle

Bootstrap order is explicit in `strategy-runtime/src/runtime.rs:701`:

1. `load_snapshots()`
2. `load_runtime_state()`
3. `notify_bootstrap_snapshot()`
4. `notify_runtime_state_restored()`
5. `warmup_strategy_indicators_from_history()`
6. pending stream recovery for acks/orders/trades/positions/bars

This call order is part of the current contract and should be treated as frozen
until lifecycle hooks are deliberately standardized.

## Snapshot and Recovery Contract

### Bootstrap Snapshot Shape

`BootstrapSnapshot` lives in `strategy-runtime/src/lib.rs:357`.

It currently contains:

- `positions_strategy`
- `working_orders_strategy`
- `working_stop_orders_strategy`
- `snapshot_ts_utc`

Even though gateway snapshots are broader, runtime filters them to strategy symbol
before handing them to strategy code in `strategy-runtime/src/runtime.rs:906`.

### Runtime State Restore

`load_runtime_state()` lives in `strategy-runtime/src/runtime.rs:1058`.

Current restored fields:

- `last_processed_bar_ts`
- `strategy_state`
- `last_trade_ts`
- `last_trade_id`
- `seen_trade_ids`

After restore, runtime calls:

- `strategy.set_state(snapshot.strategy_state)` in `strategy-runtime/src/runtime.rs:1075`
- `restore_pending_requests()` in `strategy-runtime/src/runtime.rs:1076`

### Warmup From History

Warmup is done by reading recent bar history from Redis and calling
`strategy.warmup_from_history()` in `strategy-runtime/src/runtime.rs:743`.

Current warmup properties:

- uses runtime-side history scan from the bars stream
- filters bars by strategy symbol
- sets `ctx.allow_live_orders = false`
- updates persisted strategy state after warmup
- logs `"bootstrap: strategy warmup from history bars completed"` when bars were processed

## Current Config Contract

### RuntimeConfig

`RuntimeConfig` lives in `strategy-runtime/src/lib.rs:292`.

Today it is the top-level host config and includes:

- Redis transport and stream names
- trade mode and live/paper gates
- health/read/trim settings
- one nested `StrategyConfig`
- paper/backtest/replay settings
- `reset_state_on_start`

### StrategyConfig

`StrategyConfig` lives in `strategy-runtime/src/lib.rs:405`.

It is currently monolithic and mixes:

- generic runtime strategy fields
- market buy/close fields
- session timing fields
- session gap standalone fields
- one nested `hybrid_intraday: HybridIntradaySettings`

This file already shows the main config smell that motivates later refactor:

- generic config carries strategy-specific parameter groups for multiple kinds
- config object knows how to derive each concrete strategy config

Current conversion methods:

- `to_limit_cancel_config()` at `strategy-runtime/src/lib.rs:595`
- `to_market_buy_and_close_config()` at `strategy-runtime/src/lib.rs:606`
- `to_toy_session_timing_config()` at `strategy-runtime/src/lib.rs:622`
- `to_mock_live_probe_config()` at `strategy-runtime/src/lib.rs:637`
- `to_session_gap_standalone_config()` at `strategy-runtime/src/lib.rs:649`
- `to_hybrid_intraday_runtime_config()` at `strategy-runtime/src/lib.rs:679`

### StrategyKind

`StrategyKind` lives in `strategy-runtime/src/lib.rs:534`.

Existing kinds:

- `LimitCancel`
- `MarketBuyAndClose`
- `ToySessionTiming`
- `SessionGapStandalone`
- `MockLiveProbe`
- `HybridIntraday`

All of these are in scope for registry/factory migration. The refactor should not
optimize only for the two live-heavy strategies.

## Current State Contract

### StrategyState

`StrategyState` lives in `strategy-runtime/src/state.rs:106`.

It is currently a giant enum mixing:

- legacy/simple strategy states such as `Placed`, `CancelSent`, `Done`
- market buy/close live states
- generic `Blocked`
- full `SessionGapStandalone` payload at `strategy-runtime/src/state.rs:171`
- full `HybridIntradayRuntime` payload at `strategy-runtime/src/state.rs:207`

This is the main baseline for later state-envelope work.

### RuntimeState

`RuntimeState` lives in `strategy-runtime/src/state.rs:331`.

It currently combines:

- common runtime markers: `last_processed_bar_ts`
- strategy-owned state: `strategy_state`
- broker truth mirrors: `orders`, `stop_orders`, `positions`
- trade dedup/recovery metadata: `last_trade_ts`, `last_trade_id`, `seen_trade_ids`

For the safe refactor sequence, this means we should not over-generalize runtime
common state too early. Some pending/recovery metadata may remain adapter-owned
until the strategy host seams are cleaner.

## Existing Strategy Implementations

Current implementations under `strategy-runtime/src/strategies`:

- `limit_cancel.rs`
- `market_buy_and_close.rs`
- `toy_session_timing.rs`
- `session_gap_standalone.rs`
- `mock_live_probe.rs`
- `hybrid_intraday_runtime.rs`

Hybrid itself already has deeper submodules:

- `hybrid_intraday/orchestrator.rs`
- `hybrid_intraday/mean_reversion.rs`
- `hybrid_intraday/intraday_breakout.rs`
- `hybrid_intraday/types.rs`

This is useful for the target architecture because `HybridIntraday` already looks
more like a strategy core plus orchestration layer than some older strategies.

## Compatibility Surfaces Already Relied On

### Legacy State JSON Shape

Current tests rely on legacy enum-shaped JSON.

Examples:

- `strategy-runtime/src/state.rs:361` tests deserializing old `SessionGapStandalone`
- `strategy-runtime/src/state.rs:402` tests deserializing old `HybridIntradayRuntime`
- `strategy-runtime/tests/e2e_session_gap_restart.rs:805` reads
  `payload["strategy_state"]["SessionGapStandalone"]`

This means state refactor cannot only preserve semantic recovery. It must also
either preserve or explicitly migrate the JSON shape used by current tests.

### Existing Test Surface

Current runtime test surface includes:

- config parsing tests in `strategy-runtime/tests/config_tests.rs`
- live guard tests in `strategy-runtime/tests/live_guard_tests.rs`
- ledger/report tests in `strategy-runtime/tests/ledger_reports.rs`
- smoke and reconnect e2e tests in `strategy-runtime/tests/e2e_smoke.rs` and
  `strategy-runtime/tests/e2e_reconnect_blocks.rs`
- restart compatibility path in `strategy-runtime/tests/e2e_session_gap_restart.rs`
- hybrid replay parity in `strategy-runtime/tests/e2e_hybrid_golden.rs`

These tests collectively define the current operational contract better than the
type surface alone.

## Frozen Invariants For Next PRs

The following should be treated as baseline invariants during PR1-PR4:

- `alor-gateway` remains the broker/control plane
- runtime still hosts exactly one strategy instance
- bootstrap order remains unchanged unless explicitly documented
- runtime continues to restore old state before pending stream recovery
- state warmup still happens with live order emission disabled
- runtime continues filtering snapshot/event truth by configured strategy symbol
- current paper/live/replay entry points remain intact
- old state JSON forms remain readable

## Immediate Refactor Implications

This baseline supports the agreed safe sequence:

1. freeze contracts and special cases in docs
2. move current shared abstractions into an internal host/api module without semantic change
3. introduce registry/factory for all existing `StrategyKind`
4. only then start config/state structural split with compatibility tests in place
