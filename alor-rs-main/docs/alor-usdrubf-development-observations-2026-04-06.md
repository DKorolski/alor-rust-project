# Alor USDRUBF Development Observations (2026-04-06)

## Purpose

Consolidated record of what was implemented for `alor_usdrubf_hybrid_v1`, what was verified locally, and what was observed during paper/live runtime bring-up before next smoke canary/soak checks.

## Delivered Implementation

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

- `AlorUsdrubfHybrid` capability currently has `uses_history_warmup = false`.
- Consequence: no startup pre-warm from history bars for this strategy; state is built from streamed events.

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

## Follow-up TZ

Formal follow-up hardening scope is fixed in:

- `docs/alor-usdrubf-live-hardening-tz-2026-04-06.md`

