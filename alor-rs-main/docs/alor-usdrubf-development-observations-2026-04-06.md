# Alor USDRUBF Development Observations (2026-04-06)

## Purpose

Consolidated record of what was implemented for `alor_usdrubf_hybrid_v1`, what was verified locally, and what was observed during paper/live runtime bring-up before next smoke canary/soak checks.

## Delivered Implementation

### Hardening progress snapshot

- Stage A (`P0.1-P0.3`) is implemented and validated:
  - bar dedupe (`last_processed_bar_ts` monotonic guard),
  - startup replay-tail suppression with `live_ready` gate,
  - full state round-trip payload for pending/open/session internals.
- Stage B started and partially delivered:
  - live `on_bar` no longer finalizes execution-sensitive transitions,
  - pending/open transitions in live are reconciled via broker callbacks (`on_position`),
  - in-flight intent flags added to strategy state (`entry_intent_inflight`, `exit_intent_inflight`),
  - rejected/expired/error ack clears in-flight flags,
  - live suppression added for non-live/recovery bar origins (`history`, `history_gap`, `replay`) before session reset/signal generation.
- Stage C started:
  - strategy-owned hooks implemented in `AlorUsdrubfHybrid`:
    - `tracked_order_ids()`,
    - `pending_request_ids()`,
    - `intent_comment_tag()` (format `AUS|sid=...|c=...|r=...`),
    - `exit_risk_status()` for exit-inflight risk projection.
  - request/order trackers are now hydrated from `on_runtime_state_restored` and updated via `on_ack`/`on_order`.
  - strategy-level observability logs added for bootstrap, runtime restore, stale/recovered suppression, and broker-truth position reconciliation.
- Stage D started:
  - deterministic signal/exit evaluation path is now explicitly wrapped as research-core methods via `ResearchSnapshot`:
    - `evaluate_signal_research(...)`,
    - `evaluate_exit_research(...)`.
  - live adapter/orchestration (`on_bar` + broker-truth callbacks) now consumes this research-core layer without changing runtime API.
  - post-refactor parity gate re-run:
    - `cargo run -p strategy-runtime --bin usdrubf_hybrid_replay -- --split golden --check`,
    - `cargo run -p strategy-runtime --bin usdrubf_hybrid_replay -- --split test --check`,
    - `cargo run -p strategy-runtime --bin usdrubf_hybrid_replay -- --split train --check`,
    - all splits passed without parity diffs.
- Stage E tests (in progress) added and validated:
  - runtime guard interaction in live when execution is disabled,
  - broker-truth reconciliation stability under out-of-order ack sequencing,
  - dirty-start recovery-tail suppression followed by fresh live resume path.
  - parity re-check after these additions remains green across `golden/test/train`.
  - additional formal coverage added for:
    - open-position restore + exit evaluation path,
    - warmup non-interference with pending/open trading state.

### Strategy integration path

- Strategy kind and payload wired as `AlorUsdrubfHybrid` in runtime config/model.
- Strategy module is `strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs`.
- Runtime adapter and registry factory are wired through:
  - `strategy-runtime/src/strategy_adapters.rs`
  - `strategy-runtime/src/strategy_registry.rs`
- Strategy state branch is wired in `strategy-runtime/src/state.rs`.
- Legacy alias compatibility remains:
  - `strategy_kind = "alor_skeleton"` still maps to `AlorUsdrubfHybrid`.
  - `[strategy.alor_skeleton]` still accepted as alias of `[strategy.alor_usdrubf_hybrid]`.

### Replay path

- Rust replay binary exists: `strategy-runtime/src/bin/usdrubf_hybrid_replay.rs`.
- Runtime strategy file naming is aligned with production naming (`alor_usdrubf_hybrid.rs`).

### Runtime/gateway configs

- Runtime paper/live configs are present:
  - `configs/runtime.alor_usdrubf.paper.7502T0U.toml`
  - `configs/runtime.alor_usdrubf.live.7502T0U.toml`
- Gateway config is present:
  - `configs/gateway.alor_usdrubf.live.7502T0U.toml`
- Live micro-size safety is configured:
  - `use_fixed_live_size = true`
  - `live_fixed_units = 1.0`
  - `tick_size = 0.01`

## Local Validation Summary

### Build and config validation

- `cargo check -p strategy-runtime`: PASS.
- `cargo test -p strategy-runtime --test config_tests`: PASS.
- Runtime config load in both paper/live modes: PASS.

### Bring-up validation (gateway + runtime)

- Gateway with `gateway.alor_usdrubf.live.7502T0U.toml`: started and reached `LiveReady`.
- Runtime with paper config: started, loaded snapshots, restored runtime state, health server bound.
- Runtime with live config: started, initially `BLOCKED` while waiting for first bar, then switched to `ALLOWED`.

### Probes and streams

- Gateway probe:
  - readiness payload shows `readiness=true`, `gateway_phase=LiveReady`, `ws_connected=true`, `cws_authorized=true`.
- Runtime probe:
  - `liveness=true`.
  - in paper mode `readiness=false` is expected (`trade_mode=Paper`, `allow_live_orders=false`, bootstrap guard reasons).
- Streams:
  - `events.health` has fresh entries from current gateway instance.
  - `runtime.state.alor_usdrubf_hybrid_v1.{paper|live}.usdrubf.7502T0U` has fresh state payloads.

## Key Live Observations

During live runtime session, logs show:

- quantity semantics are correct for micro mode (`qty=1.0` in emitted/fill-related lines).
- idempotency protection works:
  - `command duplicate ... status=Duplicate` for repeated `request_id`.
- multiple rejects from broker/CWS (`cws_http_400`) are present, including:
  - insufficient limits/funds,
  - BOC immediate-cross rejection,
  - price outside limit,
  - cancel-not-found.
- orphan trade warnings and `intent_dropped_bar_silence` events were observed.

Interpretation:
- runtime live path is functioning and guarded;
- current stream/session context is not fully clean for soak-quality signal extraction (historical tail/pending artifacts are present).

## Indicator Warmup Status

- `AlorUsdrubfHybrid` capability now has `uses_history_warmup = true`.
- `warmup_from_history` is implemented for session/day aggregates and does not mutate active pending/open trading state.
- Warmup test coverage is present in strategy unit tests.

## Logging Notes

To run runtime without audit spam:

```bash
RUST_LOG=info,strategy_runtime::audit=off cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.alor_usdrubf.live.7502T0U.toml
```

Useful runtime-only grep:

```bash
rg "live_guard_changed|intent_emitted|command accepted|command rejected|orphan_trade|intent_dropped" /tmp/strategy-runtime.log
```

## Pre-Canary/Soak Gate Decision

Status now:

1. Integration wiring: COMPLETE.
2. Local bring-up and probes: COMPLETE.
3. Live micro quantity semantics (`qty=1.0`): CONFIRMED.
4. Operational cleanliness for soak: NOT YET (due to duplicate/reject/orphan noise from active stream context).
5. Indicator pre-warm policy: OPEN (if mandatory for soak, enable and validate history warmup first).

Update after isolated diagnostic rerun (`diag_20260406` stream namespace):

1. Gateway probe/readiness: PASS (`LiveReady`, ws/cws healthy).
2. Runtime liveness/readiness: PASS (`live_guard=ALLOWED`, reasons empty).
3. Isolated Redis tails: PASS (no stale command/ack/trade tails in diagnostic streams).
4. Operational cleanliness for staged smoke/canary: PASS in isolated mode.
5. Indicator warmup: PASS (`uses_history_warmup=true`, startup warmup executed in logs).

## Follow-up TZ

Formal follow-up hardening scope is fixed in:

- `docs/alor-usdrubf-live-hardening-tz-2026-04-06.md`

