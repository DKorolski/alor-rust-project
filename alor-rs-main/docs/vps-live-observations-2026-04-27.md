# VPS Live Observations (2026-04-27)

## Scope

Stack:

- `trading-hybrid`
- `trading-alor-usdrubf`
- `trading-sessiongap`
- symbol: `IMOEXF`
- symbol: `USDRUBF`
- portfolio: `7502SN6`
- portfolio: `7502T0U`
- portfolio: `7502MIW`
- rollout target: IMOEXF High180 MR + BO `riskgate-shadow` validation
- rollout target: alor-USDRUBF `mr_k_short = 0.035` challenger validation
- rollout target: SessionGap challenger config staging only
- time window: pre-open, around `02:00 MSK`

## Rollout Summary

The `trading-hybrid` stack was rolled from the previous action-scoped baseline
runtime image to the IMOEXF risk-gate runtime image:

```text
previous runtime image: manual-084308c
new runtime image:      manual-2d1803e-riskgate
gateway image:          manual-5430299
```

Active post-rollout config:

```text
GATEWAY_CONFIG=/configs/gateway.hybrid.live.7502SN6.action-scoped.toml
RUNTIME_CONFIG=/configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml
```

The bootstrap sequence was intentionally two-step:

1. Start once with `runtime.hybrid.live.7502SN6.riskgate-bootstrap.toml`.
2. Import the checked seed into the runtime-owned risk-gate ledger.
3. Switch immediately to `runtime.hybrid.live.7502SN6.riskgate-shadow.toml`.

The seed was mounted through the config volume:

```text
/configs/riskgate_high180_lb120_seed_2026-04-26.csv
```

## Preflight

Before changing the stack, the broker snapshot confirmed:

```text
IMOEXF qty = 0.0
stop_orders = {}
orders = filled-only historical orders
```

No live working orders or stop orders were visible for the target stack.

## Risk-Gate Bootstrap Evidence

Bootstrap mode:

```text
risk_gate_mode = bootstrap_from_seed
decision = ImportSeed
records_attempted = 180
records_inserted = 180
state_refreshed = true
```

Applied state:

```text
profile_id = imoexf_primary_high180_lb120
mr_enabled_current_session = true
mr_enabled_next_session = true
rolling_sum_lb120 = 161.90000000000012
last_finalized_session_date = 2026-04-21
ledger_rows_count = 180
```

After bootstrap, the runtime was switched to steady shadow mode:

```text
risk_gate_mode = normal_append
decision = UseExistingLedger
existing_records_loaded = 180
records_attempted = 0
state_refreshed = true
```

## From-Zero Correction

The first post-bootstrap replay restored a stale position from the old
`broker.positions.7502SN6` stream:

```text
last_position_qty = -1.0
safe_mode_close_only = true
safe_mode_reason = recovered_position_owner_unknown
```

This did not match the broker snapshot, which was flat. To avoid carrying stale
position state into the validation session, the runtime was stopped and the
validation contour was reset:

```text
cleared:
runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6
broker.positions.7502SN6
cmd.orders.7502SN6
cmd.acks.7502SN6
broker.orders.7502SN6
broker.trades.7502SN6
```

The shadow consumer group was destroyed for the relevant streams so the runtime
could restart cleanly under the new validation contour.

Post-reset runtime state:

```text
last_position_qty = 0.0
current_owner = null
current_side = null
safe_mode_close_only = false
safe_mode_reason = null
pending_* = null
deferred_* = null
seen_trade_ids = []
```

Command and execution streams remained empty after restart:

```text
cmd.orders.7502SN6 = 0
cmd.acks.7502SN6 = 0
broker.orders.7502SN6 = 0
broker.trades.7502SN6 = 0
```

## Current State

The stack is healthy at container/liveness level:

```text
trading-hybrid-alor-gateway-1       healthy
trading-hybrid-strategy-runtime-1   healthy
trading-hybrid-redis-1              healthy
```

Runtime readiness is expected to remain `503` pre-open while waiting for a
fresh live bar:

```text
phase = SyncingHistory
live_guard = BLOCKED
reasons include:
bootstrap:missing_live_bar
bootstrap:not_ready
waiting_for_first_bar
```

Warmup evidence in logs:

```text
bootstrap: snapshots filtered positions_open_strategy=0 orders_open_strategy=0 stop_orders_open_strategy=0
bootstrap: strategy warmup from history bars completed bars_processed=445
risk_gate_state_applied
```

## Watchpoints For 09:00 MSK

At the first fresh regular `10m` bar:

- runtime should leave `SyncingHistory` and move toward `LiveReady / ALLOWED`;
- no stale `intent_emitted` should appear before the fresh live bar;
- `cmd.orders.7502SN6` should remain empty unless a fresh live signal is valid;
- `last_position_qty` should remain `0.0` until a real broker execution occurs;
- risk-gate session state should remain `normal_append`, not `bootstrap_from_seed`;
- MR/BO attribution should be checked separately if a trade appears.

## Verdict

The IMOEXF risk-gate shadow-validation rollout is installed and clean at the
pre-open state level. The only open operational check is the first fresh live
bar transition on 2026-04-27.

## Alor-USDRUBF Challenger Rollout

The `trading-alor-usdrubf` stack was checked after the IMOEXF rollout because
the live work plan also contains a small USDRUBF Hybrid challenger:

```text
baseline mr_k_short = 0.045
challenger mr_k_short = 0.035
mr_force_exit_time = 11:50:00
bo_k = 0.45
bo_eod_exit_time = 23:30:00
```

Preflight broker snapshot was flat:

```text
USDRUBF qty = 0.0
```

The challenger config was copied to the VPS:

```text
/opt/trading-alor-usdrubf/configs/runtime.alor_usdrubf.live.7502T0U.challenger_mr035.toml
```

Active post-rollout config:

```text
GATEWAY_CONFIG=/configs/gateway.alor_usdrubf.live.7502T0U.toml
RUNTIME_CONFIG=/configs/runtime.alor_usdrubf.live.7502T0U.challenger_mr035.toml
GATEWAY_IMAGE_TAG=manual-5430299
RUNTIME_IMAGE_TAG=sha-4a0a266
```

The runtime was restarted from zero by clearing only the transient lifecycle
streams and runtime state:

```text
cleared:
runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U
cmd.orders.7502T0U
cmd.acks.7502T0U
broker.orders.7502T0U
broker.trades.7502T0U
broker.positions.7502T0U
```

Market-data bars and broker snapshots were not cleared.

Resolved runtime config confirmed the challenger:

```text
config_path = /configs/runtime.alor_usdrubf.live.7502T0U.challenger_mr035.toml
mr_k_short = 0.035
mr_force_exit_time = 11:50:00
bo_k = 0.45
```

Post-reset state:

```text
hybrid_state = flat
open_position_qty = 0.0
pending_request_ids = []
tracked_order_ids = []
entry_intent_inflight = false
exit_intent_inflight = false
seen_trade_ids = []
```

Post-reset stream lengths:

```text
cmd.orders.7502T0U = 0
cmd.acks.7502T0U = 0
broker.orders.7502T0U = 0
broker.trades.7502T0U = 0
broker.positions.7502T0U = 0
```

No runtime or gateway `WARN` / `ERROR` lines were observed after the restart in
the pre-open verification window.

## SessionGap Challenger Staging

`trading-sessiongap` was not switched away from the current live baseline. This
matches the work-plan rule that SessionGap TP-short challengers should be
validated separately, not promoted silently as production defaults.

Active config remains:

```text
RUNTIME_CONFIG=/configs/runtime.sessiongap.live.7502MIW.toml
signal_minute = 50
wait_hours = 3
k_tp_short = 0.28
k_sl_short = 0.65
k_tp_long = 0.28
k_sl_long = 0.68
max_entry_hour = 16
close_hour = 23
close_minute = 49
```

The challenger configs were staged on the VPS for future controlled validation:

```text
/opt/trading-sessiongap/configs/runtime.sessiongap.live.7502MIW.challenger_tp_short_050.toml
/opt/trading-sessiongap/configs/runtime.sessiongap.live.7502MIW.challenger_tp_short_060.toml
```

The stack remained healthy and flat in the broker snapshot:

```text
USDRUBF qty = 0.0
cmd.orders.7502MIW = 0
cmd.acks.7502MIW = 0
broker.orders.7502MIW = 0
broker.trades.7502MIW = 0
```

## Additional Watchpoints

- `trading-alor-usdrubf` should leave pre-open `SyncingHistory` only after the
  first fresh `10m` live bar on 2026-04-27.
- First alor-USDRUBF MR event should be checked specifically for the narrower
  `mr_k_short = 0.035` trigger behavior.
- SessionGap remains baseline; do not interpret staged challenger files as an
  active parameter change.

## Pre-Open Log Check

Time:

```text
2026-04-27 08:18-08:20 MSK
```

All three stacks were checked before the regular MOEX session open:

```text
trading-sessiongap      healthy, flat, no command/order/trade streams
trading-hybrid          healthy, flat, no command/order/trade streams
trading-alor-usdrubf    healthy, flat, no command/order/trade streams
```

Active configs:

```text
sessiongap:      /configs/runtime.sessiongap.live.7502MIW.toml
hybrid IMOEXF:   /configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml
alor-USDRUBF:    /configs/runtime.alor_usdrubf.live.7502T0U.challenger_mr035.toml
```

Broker snapshots showed no strategy positions:

```text
7502MIW: USDRUBF qty = 0.0
7502SN6: no IMOEXF position in snapshot
7502T0U: no USDRUBF position in snapshot
```

One pre-open operational issue was found on `trading-hybrid`: after the
from-zero Redis cleanup, the empty command stream no longer had the gateway
consumer group:

```text
cmd.orders.7502SN6 / gateway-commands -> NOGROUP
```

This was repaired without adding commands or restarting the stack:

```text
XGROUP CREATE cmd.orders.7502SN6 gateway-commands $ MKSTREAM
```

Post-repair state:

```text
cmd.orders.7502SN6 = 0
gateway-commands pending = 0
fresh gateway WARN/ERROR since repair = none
fresh runtime WARN/ERROR since repair = none
```

Verdict: the three live stacks are ready for the first fresh `10m` bar check.

## Safe Resource Cleanup

Time:

```text
2026-04-27 08:59 MSK
```

The VPS resource check before cleanup showed no active pressure:

```text
load average = 0.29 / 0.26 / 0.26
RAM available = 5.3 GiB
swap used = 28 MiB
disk before = 47G used / 79G total / 29G free / 63%
```

Redis memory stayed below the configured 1 GiB container limit:

```text
sessiongap redis      used_memory = 413.96M
hybrid redis          used_memory = 433.63M
alor-usdrubf redis    used_memory = 464.17M
```

Cleanup actions were limited to inactive artifacts:

```text
deleted old backup:
/opt/trading-hybrid/volumes/redis.bak.pre-from-zero-20260418-104553

docker image prune -f
```

The old backup was not part of the active Redis volume:

```text
active hybrid Redis volume = /opt/trading-hybrid/volumes/redis
deleted backup size = 3.0G
```

Docker prune was intentionally run without `-a`, so only dangling images were
removed and tagged rollback images were left in place.

Post-cleanup result:

```text
disk after = 37G used / 79G total / 38G free / 50%
docker dangling image space reclaimed = 6.316GB
hybrid active volume size = 300M
sessiongap active volume size = 322M
alor-usdrubf active volume size = 347M
```

All three trading stacks stayed healthy after cleanup:

```text
trading-sessiongap      healthy
trading-hybrid          healthy
trading-alor-usdrubf    healthy
```

Fresh logs after cleanup did not show new `WARN`, `ERROR`, `NOGROUP`,
`failed`, or `rejected` lines.
