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

For `StrategyKind::AlorUsdrubfHybrid`, history warmup capability is currently disabled:
- `uses_history_warmup: false` in `strategy-runtime/src/strategy_registry.rs`

Operational consequence:
- `warmup_from_history(...)` path is not executed for this strategy.
- indicators are initialized from live/streamed bars only, not pre-warmed from historical bars at startup.

## Pre-Smoke Canary/Soak Gate

Current status before next smoke canary/soak cycle:

1. **Gateway bring-up:** PASS (`LiveReady`, readiness=true).
2. **Runtime bring-up (paper):** PASS (`liveness` up, bootstrap/recovery path executed, runtime state persisted).
3. **Probe evidence:** PASS (fresh `events.health` + `runtime.state` tails captured).
4. **Indicator warmup policy:** OPEN ITEM
   - current strategy has `uses_history_warmup = false`
   - if strict pre-warm is required before canary/soak, enable and validate warmup path first.
