# RI Author41/42 7502MIW Shadow Rollout Runbook

Date: 2026-05-01

Status: `PRE_GO_SHADOW_ONLY`

This runbook prepares the RI Author41/42 shadow contour on portfolio `7502MIW`.
It is not a micro-live rollout. Order emission must remain disabled.

Deployment status as of 2026-05-01 15:05 MSK:

```text
stack = trading-ri-author41-42-7502miw
host = nektodk.ispvds.com / 155.212.170.21
status = DEPLOYED_SHADOW_ONLY
runtime_image = ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-d6d0e7e-ri7502miw-20260501
gateway_image = ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-5430299-protplace-20260428
```

Initial validation:

```text
docker compose ps: redis/gateway/runtime healthy
runtime warmup: bars_processed=457
runtime mode: shadow
allow_live_orders=false
allow_order_emission=false
live_adapter_enabled=false
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow = 0
strict pre-GO journal review = PASS
```

## Files

Runtime config:

```text
configs/runtime.ri_author41_42.shadow.7502MIW.toml
```

Gateway config:

```text
configs/gateway.ri_author41_42.shadow.7502MIW.toml
```

## Safety Invariants

Required before start:

- `trade_mode = paper`;
- `allow_live_orders = false`;
- `allow_order_emission = false`;
- `mode = shadow`;
- `execution_path = action_scoped_only`;
- `symbol = RIM6`;
- no shared portfolio-wide bar stream for RI.

RI bar stream:

```text
md.bars.7502MIW.RIM6.10m
```

Do not point RI runtime at:

```text
md.bars.7502MIW.10m
```

That stream may contain `USDRUBF` bars used by sessiongap.

## Gateway Stream Overrides

When using `alor_gateway_transport_runner`, run the RI gateway process with
these overrides:

```text
STREAM_BARS=md.bars.7502MIW.RIM6.10m
STREAM_HEALTH=events.health.ri_author41_42.7502MIW
STREAM_COMMANDS=cmd.orders.7502MIW.ri_author41_42.shadow
STREAM_ACKS=cmd.acks.7502MIW.ri_author41_42.shadow
CONSUMER_GROUP=gateway-commands-ri-author41-42-shadow-7502MIW
```

The isolated command/ack streams are defensive. Runtime should not emit order
commands in shadow mode, but the streams must still not collide with sessiongap.

## Preflight

Check config parses locally:

```text
cargo test -p strategy-runtime loads_ri_author41_42_7502miw_shadow_config --test config_tests -- --nocapture
cargo test -p alor-gateway loads_ri_author41_42_7502miw_gateway_config -- --nocapture
```

Check the target VPS before start:

```text
docker ps
docker stats --no-stream
redis-cli XLEN md.bars.7502MIW.RIM6.10m
redis-cli XLEN runtime.state.ri_author41_42.shadow.7502MIW
redis-cli XLEN cmd.orders.7502MIW.ri_author41_42.shadow
redis-cli XLEN cmd.acks.7502MIW.ri_author41_42.shadow
```

Expected before first start:

- RI runtime state may be empty;
- RI command stream should be empty or inactive;
- RI ack stream should be empty or inactive;
- no requirement to clear sessiongap streams.

If an older RI shadow stack already exists on the VPS, verify it is not being
mistaken for the new `7502MIW/RIM6` target. The old contour may show streams
such as:

```text
md.bars.RI.10m
broker.snapshots.7502SN6
cmd.orders.7502SN6
```

Those streams do not satisfy this runbook. The target contour must use:

```text
md.bars.7502MIW.RIM6.10m
cmd.orders.7502MIW.ri_author41_42.shadow
cmd.acks.7502MIW.ri_author41_42.shadow
```

## Start Sequence

1. Copy the runtime and gateway configs to the VPS config directory.
2. Start the RI gateway shadow process with the stream overrides above.
3. Start the RI runtime shadow process with:

```text
RUNTIME_CONFIG=/configs/runtime.ri_author41_42.shadow.7502MIW.toml
```

4. Confirm runtime health endpoint is alive on `127.0.0.1:8094`.
5. Confirm gateway health endpoint is alive on `127.0.0.1:8084`.

## Log Checks

Runtime should show:

```text
ri_model_bar_observed
mode=shadow
allow_order_emission=false
live_adapter_enabled=false
```

When decisions appear, runtime may show:

```text
ri_model_decision
ri_candidate_intent_suppressed
```

It must not show live order command emission.

Gateway should show bar publication for `RIM6` and no RI command processing in
normal shadow mode.

## Redis Checks

RI bars:

```text
redis-cli XREVRANGE md.bars.7502MIW.RIM6.10m + - COUNT 3
```

RI decisions:

```text
tail -n 50 /opt/trading-ri-author41-42-7502miw/volumes/reports/ri_author41_42_7502MIW_decisions.jsonl
```

Structured journal review:

```text
python3 scripts/ri_author41_42_journal_review.py \
  /opt/trading-ri-author41-42-7502miw/volumes/reports/ri_author41_42_7502MIW_decisions.jsonl \
  --strict-pre-go \
  --tail 20 \
  --out-md /opt/trading-ri-author41-42-7502miw/volumes/reports/ri_author41_42_7502MIW_journal_review.md
```

Expected pre-GO review result:

- status is `PASS`;
- no live-emission evidence rows;
- no duplicate `shadow_recorded` decision keys;
- all command-capable `entry`/`exit` rows use `execution_path=action_scoped_only`;
- pure `shadow_recorded` observation rows may use
  `execution_path=not_applicable_pre_go`;
- adapter decisions are limited to `shadow_recorded`,
  `intent_suppressed`, and `manual_intervention_required`.

RI commands should remain empty:

```text
redis-cli XLEN cmd.orders.7502MIW.ri_author41_42.shadow
```

If command length increases unexpectedly, stop the RI runtime and investigate
before continuing.

## Contract Roll

RI is an expiring futures contract.

Simple roll policy:

- roll 7 calendar days before expiry;
- roll only between sessions;
- require broker-flat and no working orders;
- change symbol/feed config to the next contract;
- restart runtime/live state from zero;
- load warmup/history from the new contract;
- do not run intraday cross-contract transfer.

## Stop Conditions

Stop and investigate if any of the following appears:

- RI runtime consumes non-`RIM6` bars;
- RI command stream receives messages;
- `allow_live_orders=true`;
- `allow_order_emission=true`;
- `mode=micro_live`;
- legacy CWS path appears in RI command-path evidence;
- cross-day decision enters candidate lifecycle instead of
  `manual_intervention_required`;
- runtime state restore reports pending/known order tails.

## Observation 2026-05-02

Checkpoint:

```text
2026-05-02 10:47 MSK
```

Runtime state:

```text
mode = shadow
profile_id = ri_author41_42_primary_combo_cost2
timeframe = 10m
allow_order_emission = false
execution_path = action_scoped_only
live_adapter_enabled = false
phase = flat
model_bars_seen = 430
suppressed_service_bars = 76
model_decisions_seen = 6
last_transition_reason = dry_run_exit:take_author_close
```

Safety checks:

```text
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow = 0
broker positions = no futures position
broker orders = {}
broker stop_orders = {}
WARN/ERROR scan = clean
```

Timing read:

```text
last_model_bar_ts = 2026-05-01 23:30:00 MSK
last_bar_ts = 2026-05-02 10:20:00 MSK
```

Interpretation:

```text
The shadow contour remains pre-GO safe.
Newer bars are visible to the runtime, but model progression remains anchored
to the last regular model bar. This is consistent with weekend/service-bar
suppression and with the current shadow-only contract.
```

## Observation 2026-05-03

Checkpoint:

```text
2026-05-03 09:37-09:40 MSK
```

Runtime state:

```text
mode = shadow
profile_id = ri_author41_42_primary_combo_cost2
timeframe = 10m
allow_order_emission = false
execution_path = action_scoped_only
live_adapter_enabled = false
phase = flat
model_bars_seen = 430
suppressed_service_bars = 114
model_decisions_seen = 6
last_transition_reason = dry_run_exit:take_author_close
```

Safety checks:

```text
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow = 0
broker futures position = none
broker orders = {}
broker stop_orders = {}
WARN/ERROR scan = clean
```

Timing read:

```text
last_model_bar_ts = 2026-05-01 23:30:00 MSK
last_bar_ts = 2026-05-02 10:30:00 MSK
```

Maintenance note:

```text
The RI 7502MIW Redis container was added to /opt/trading-maintenance/redis_safe_trim.sh
so the daily whitelist trim now covers this new shadow contour.

Manual apply trimmed broker.snapshots.7502MIW:
len_before=15444
len_after=10000

Runtime state, command streams, and model bars were not trimmed.
```

Interpretation:

```text
The shadow contour remains pre-GO safe.
The weekend/service-bar suppression behavior remains consistent with the
current model contract.
The Redis retention gap for the new contour was corrected before becoming a
resource incident.
```

## Observation 2026-05-04

Checkpoint:

```text
2026-05-04 09:40 MSK
```

Runtime state:

```text
mode = shadow
profile_id = ri_author41_42_primary_combo_cost2
timeframe = 10m
allow_order_emission = false
execution_path = action_scoped_only
live_adapter_enabled = false
phase = flat
model_bars_seen = 434
suppressed_service_bars = 148
model_decisions_seen = 6
last_transition_reason = dry_run_exit:take_author_close
```

Safety checks:

```text
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow = 0
broker futures position = none
broker orders = {}
broker stop_orders = {}
WARN/ERROR scan = clean
```

Timing read:

```text
last_model_bar_ts = 2026-05-04 09:30:00 MSK
last_bar_ts = 2026-05-04 09:30:00 MSK
```

Redis retention:

```text
The 2026-05-04 03:10 MSK safe-trim timer included the RI 7502MIW contour.
broker.snapshots.7502MIW was trimmed from 16306 to 10000.

At the checkpoint:
broker.snapshots.7502MIW = 12340
md.bars.7502MIW.RIM6.10m = 1037
runtime.state.ri_author41_42.shadow.7502MIW = 127
```

Interpretation:

```text
The shadow contour progressed into the regular Monday session and remains
pre-GO safe. The safe-trim timer now covers the new RI contour, and no live
command emission was observed.
```
