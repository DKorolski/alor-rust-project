# Alor USDRUBF 7502T0U Smoke Checklist

Date: 2026-04-06

## Scope

This checklist is for controlled rollout of `alor_usdrubf_hybrid_v1` on `USDRUBF` with portfolio `7502T0U`.

## Related Docs

- `docs/alor-usdrubf-development-observations-2026-04-06.md`
- `docs/alor-usdrubf-local-bringup-report-2026-04-06.md`
- `docs/alor-usdrubf-live-hardening-tz-2026-04-06.md`

## Preflight (Before Smoke/Soak)

- run a clean diagnostic cycle first:
  - use `reset_state_on_start = true`
  - use a fresh `consumer_group` for runtime
  - use a fresh `streams.runtime_state` key for the run
  - use isolated stream namespace for bars/orders/trades/positions/acks/commands/health
- confirm no stale-tail symptoms in logs:
  - no startup stale entry attempts
  - no repeated `intent_dropped_*` + immediate state revert pattern
  - no unexpected duplicate request churn from historical tails
- proceed to smoke/canary/soak only after preflight is clean

### Supported startup profile (current)

- supported: clean-start profile only (flat account, isolated namespace, fresh consumer group and runtime-state stream),
- not yet proven: restart with open position and/or working orders/stop orders.

### Mandatory evidence to capture in next run

- bootstrap summary line,
- replay guard armed line,
- replay guard cleared line,
- first fresh live-origin bar marker,
- first `live_guard=ALLOWED`,
- first allowed entry,
- first broker-truth position transition.

Guard semantics to verify in logs:

- `live_ready` is cleared only by fresh `DataOrigin::Live` bar,
- fresh `history/history_gap/replay` bars must stay suppressed during startup gate.

## Config Files

- `configs/gateway.alor_usdrubf.live.7502T0U.toml`
- `configs/runtime.alor_usdrubf.paper.7502T0U.toml`
- `configs/runtime.alor_usdrubf.live.7502T0U.toml`

## Safety Invariants

- quantity semantics in runtime: `size` means contracts count
- micro-soak default: `use_fixed_live_size = true`, `live_fixed_units = 1.0`
- no implicit multiplication by contract lot in order quantity mapping
- `tick_size = 0.01` for live MOEX routing

## Stage 1: Paper Smoke

- run gateway with `gateway.alor_usdrubf.live.7502T0U.toml`
- run runtime with `runtime.alor_usdrubf.paper.7502T0U.toml`
- verify:
  - health endpoint is ready
  - bars stream is consumed
  - strategy emits intents only in paper mode
  - no runtime errors around config parsing or state restore

## Stage 2: Live Micro-Soak

- switch runtime to `runtime.alor_usdrubf.live.7502T0U.toml`
- confirm before start:
  - `allow_live_orders = true`
  - `strategy_id = alor_usdrubf_hybrid_v1`
  - `strategy_kind = alor_usdrubf_hybrid`
  - `live_fixed_units = 1.0`
- verify in early session:
  - create/ack/order/position flow is healthy
  - requested live quantity is exactly `1`
  - no oversize incidents and no unexpected quantity transformations

## Rollback

- stop runtime process first
- keep gateway running for diagnostics if needed
- switch runtime back to paper config
- preserve logs and runtime state key for incident analysis
