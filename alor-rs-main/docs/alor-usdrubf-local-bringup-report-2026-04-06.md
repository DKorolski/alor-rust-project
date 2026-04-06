# Alor USDRUBF Local Bring-Up Report (2026-04-06)

## Scope

Pre-check before next smoke canary/soak cycle:
- local start of `alor-gateway` and `strategy-runtime`
- verification of probe/health signals and `runtime_state`
- verification of indicator warmup status

Related follow-up scope:
- `docs/alor-usdrubf-live-hardening-tz-2026-04-06.md`

## Commands Used

Gateway:

```bash
set -a && source ./.env.preprod && set +a && \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.alor_usdrubf.live.7502T0U.toml \
  --redis-url redis://127.0.0.1/
```

Runtime (paper):

```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- \
  --config ./configs/runtime.alor_usdrubf.paper.7502T0U.toml
```

Redis checks:

```bash
redis-cli -u redis://127.0.0.1/ XLEN runtime.state.alor_usdrubf_hybrid_v1.paper.usdrubf.7502T0U
redis-cli -u redis://127.0.0.1/ XREVRANGE runtime.state.alor_usdrubf_hybrid_v1.paper.usdrubf.7502T0U + - COUNT 1
redis-cli -u redis://127.0.0.1/ XLEN events.health
redis-cli -u redis://127.0.0.1/ XREVRANGE events.health + - COUNT 2
```

## Observations

### 0) Current clean-start diagnostic (gateway runner + runtime live)

Latest local run sequence:
- gateway started with `alor_gateway_runner` (not `alor_gateway_limit_cancel`),
- legacy working order tail was manually cancelled and restart repeated,
- runtime started with `runtime.alor_usdrubf.live.7502T0U.toml`.

Observed statuses:
- gateway probe:
  - `GET /liveness` -> HTTP `200`
  - `GET /readiness` -> `readiness=true`, `gateway_phase=LiveReady`, `ws_connected=true`, `cws_authorized=true`
- runtime probe:
  - `GET /liveness` -> `{"liveness":true,...}`
  - `GET /readiness` -> `readiness=false`, `live_guard=BLOCKED`, reasons:
    - `gateway_health_stale`
    - `bootstrap:not_ready`
    - `bootstrap:missing_live_bar`

Redis evidence for this run:
- latest `runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U` payload keeps restored historical tail context (February state markers),
- latest `events.health` entry remained from older gateway instance (`source=gateway-33589-...`, old timestamp), i.e. health stream tail was not refreshed for current runtime read path.

Operational interpretation:
- strategy process is up and warmup executes, but live readiness gate remains intentionally blocked due stale health/state context.
- this is consistent with hardening protocol: next run should use clean diagnostic isolation (fresh runtime-state stream and/or consumer group, optionally `reset_state_on_start=true`).

### 0.1) Fully isolated diagnostic rerun (v2) - PASS

A second diagnostic pass was executed with fully isolated stream namespace:
- gateway transport-only stream overrides:
  - `md.bars.7502T0U.1m.diag_20260406`
  - `broker.orders.7502T0U.diag_20260406`
  - `broker.trades.7502T0U.diag_20260406`
  - `broker.positions.7502T0U.diag_20260406`
  - `broker.snapshots.7502T0U.diag_20260406`
  - `cmd.orders.7502T0U.diag_20260406`
  - `cmd.acks.7502T0U.diag_20260406`
  - `events.health.alor_usdrubf_diag_20260406`
- runtime:
  - `configs/runtime.alor_usdrubf.live.7502T0U.diag.toml`
  - `reset_state_on_start = true`
  - `runtime_state = runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U.diag_20260406`

Observed result:
- gateway probe: readiness `true`, phase `LiveReady`, ws/cws healthy.
- runtime probe:
  - liveness `true`
  - readiness `true`
  - `live_guard = ALLOWED`
  - reasons `[]`
- runtime logs:
  - `startup replay guard cleared; live_ready=true`
  - `live_guard_changed ... to="ALLOWED"`
- isolated Redis tails:
  - `events.health.alor_usdrubf_diag_20260406` latest source is current diag transport runner,
  - `runtime.state...diag_20260406` shows same-day live state and no legacy `seen_trade_ids` tail.
- isolated stream lengths confirm clean command/ack/trade tails:
  - `XLEN cmd.orders...diag = 0`
  - `XLEN cmd.acks...diag = 0`
  - `XLEN broker.trades...diag = 0`
  - `XLEN broker.orders...diag = 1` (snapshot canceled order tail, no working live order).

### 1) Gateway startup and probes

Gateway was started successfully after setting `ALOR_REFRESH_TOKEN` in the run shell.

Observed in logs:
- config resolved from `./configs/gateway.alor_usdrubf.live.7502T0U.toml`
- transport runner started and subscribed to `bars/positions/orders/stop_orders/trades`
- phase transition reached `LiveReady`

Probe/health confirmation:
- `GET http://127.0.0.1:8081/liveness` returned HTTP success
- `GET http://127.0.0.1:8081/readiness` returned payload with:
  - `readiness = true`
  - `gateway_phase = "LiveReady"`
  - `ws_connected = true`
  - `cws_authorized = true`

### 2) Runtime startup (paper) and probes

`strategy-runtime` with `runtime.alor_usdrubf.paper.7502T0U.toml` started and executed bootstrap path.

Observed in logs:
- `bootstrap: snapshots loaded orders=0 positions=0`
- `runtime_state_snapshot_loaded`
- `bootstrap_processed`
- `runtime_state_restored`
- `pending_recovery_started/finished` for `acks/orders/trades/positions/bars`
- health server bind: `health server listening addr=127.0.0.1:8091`

Probe/health confirmation (short controlled run):
- `GET http://127.0.0.1:8091/liveness` returned:
  - `{"liveness": true, ...}`
- `GET http://127.0.0.1:8091/readiness` returned:
  - `readiness = false` (expected in paper mode)
  - reasons include `trade_mode=Paper`, `allow_live_orders=false`, `bootstrap:not_ready`, `bootstrap:missing_live_bar`
  - gateway section shows `gateway_ready = true`, `ws_connected = true`, `cws_authorized = true`

### 3) Runtime state stream

`runtime.state.alor_usdrubf_hybrid_v1.paper.usdrubf.7502T0U` contains fresh payload for `AlorUsdrubfHybrid`.

Latest observed payload fields:
- `lifecycle_stage = "live"`
- `bootstrap_seen = true`
- `runtime_state_restored = true`
- `hybrid_state = "flat"`
- `current_date_local = "2026-02-19"`
- `cash = 100000.5438764`

### 4) Health stream (`events.health`)

Latest `events.health` entry is from current gateway instance and confirms:
- `source = "gateway-..."`
- `gateway_phase = "LiveReady"`
- `readiness = true`
- `ws_connected = true`
- `cws_authorized = true`
- `active_subscriptions_count = 5`
- `scheduler_state = "Open"`

## Indicator Warmup Status

For `StrategyKind::AlorUsdrubfHybrid`, history warmup capability is enabled:
- `uses_history_warmup: true` in `strategy-runtime/src/strategy_registry.rs`

Operational consequence:
- `warmup_from_history(...)` is executed at startup when strategy state allows it,
- indicators/session aggregates are pre-warmed from historical bars before live path.

## Pre-Smoke Canary/Soak Gate

Current status before next smoke canary/soak cycle:

1. **Gateway bring-up:** PASS (`LiveReady`, readiness=true).
2. **Runtime bring-up (paper):** PASS (`liveness` up, bootstrap/recovery path executed, runtime state persisted).
3. **Probe evidence:** PASS (fresh `events.health` + `runtime.state` tails captured).
4. **Indicator warmup policy:** OPEN ITEM
   - current strategy already has `uses_history_warmup = true`
   - remaining gate is operational stream/state cleanliness (`events.health` freshness + runtime-state tail isolation) before canary/soak.

## Next-Run Operational Protocol (Locked)

Before the next controlled micro run:

1. use a new runtime `consumer_group`,
2. use fresh `runtime_state` stream key,
3. use isolated stream namespace (gateway + runtime),
4. start in paper or `allow_live_orders=false` until startup profile for target scenario is proven.

For the next report, capture and attach:

- bootstrap summary,
- replay guard armed,
- replay guard cleared,
- first fresh `DataOrigin::Live` bar observed,
- first `live_guard=ALLOWED`,
- first allowed entry,
- first broker-truth position transition.

Startup support declaration for current revision:

- supported profile: clean-start only,
- non-flat restart/working-orders/stop-orders scenarios: not yet proven and treated as follow-up hardening scope.
