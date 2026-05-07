# VPS Live Observations - 2026-05-07

## Pre-Session Check

Observed at:

```text
2026-05-07 08:07 MSK
```

Container status:

```text
sessiongap          runtime/gateway/redis up, healthy
alor-usdrubf        runtime/gateway/redis up, healthy
hybrid-imoexf       runtime/gateway/redis up, healthy
ri-author41/42      runtime/gateway/redis up, healthy
legacy ri-shadow    runner/gateway/redis up
```

Resource status:

```text
RAM       1.6 GiB used / 7.7 GiB total, 6.1 GiB available
swap      90 MiB used / 3.9 GiB total
disk /    31 GiB used / 79 GiB total, 45 GiB available, 41%

RI micro runtime    2.66 MiB / 768 MiB
RI micro gateway    4.24 MiB / 768 MiB
RI micro Redis      127.4 MiB / 768 MiB
```

Broker-state check:

```text
sessiongap       nonzero=0 working=0
alor-usdrubf     nonzero=0 working=0
hybrid-imoexf    nonzero=0 working=0
ri-author41/42   nonzero=0 working=0
```

RI Author41/42 micro state:

```text
mode                   micro_live
allow_order_emission   true
execution_path         action_scoped_only
live_adapter_enabled   true
phase                  flat
last_processed_bar_ts  RIM6=1778099400
last_bar_ts            1778100000
last_model_bar_ts      1778099400
model_bars_seen        697
suppressed_service_bars 148
model_decisions_seen   13
last_decision_key      ri_author41_42_primary_combo_cost2|author42_bo|2026-05-06 17:00:00|Some(Long)|Some(2026-05-06T23:00:00)|time_exit_same_bar_close
```

RI micro command safety:

```text
cmd.orders.7502MIW.ri_author41_42.micro  0
cmd.acks.7502MIW.ri_author41_42.micro    0
```

RI Redis memory:

```text
used_memory_human       116.44M
maxmemory_human         512.00M
mem_fragmentation_ratio 1.09
```

Interpretation:

```text
The RI micro rollout remained safe overnight. The runtime is active in
micro_live mode with live_adapter_enabled=true and action_scoped_only execution,
but no live commands were emitted before the next regular session. Historical
warmup/model state is populated, and the strategy remains flat.
```

## Log Review

RI Author41/42:

```text
runtime WARN/ERROR since midnight: none
gateway WARN/ERROR: reconnect/reset noise only
command stream: empty
ack stream: empty
```

Other systems:

```text
sessiongap       no new trades, flat; runtime live_guard blocked during night sync
alor-usdrubf     no new trades, flat; runtime live_guard blocked during night sync
hybrid-imoexf    no new trades, flat; runtime live_guard blocked during night sync
```

Observed gateway noise:

```text
CWS/WS EOF or reset reconnects occurred across several stacks during the night.
All relevant transport failures had pending_count=0.
Alor-USDRUBF had one positions subscribe AckTimeout during reconnect.
Hybrid gateway replayed an existing canceled IMOEXF stop order snapshot.
```

Interpretation:

```text
The overnight noise is consistent with the known Alor WS/CWS reconnect pattern
and did not coincide with live commands, open positions, or working orders. No
operator action is required before the session, but the first RI micro live
intent must still be watched closely for action-scoped CWS path and clean
broker ack/fill lifecycle.
```

## Watch Items

```text
1. Confirm RI moves from BLOCKED/SyncingHistory to LiveReady after the first
   eligible RIM6 10m live bar.
2. Confirm RI command stream remains empty until an actual same-day model
   intent is generated.
3. On first RI micro command, verify:
   - qty=1
   - stream=cmd.orders.7502MIW.ri_author41_42.micro
   - gateway action scope session open/send/result logs
   - no legacy CWS path
   - broker ack/fill is accepted and reconciled
4. Continue routine Redis memory watch after the recent trim and micro rollout.
```

## Verdict

```text
STATUS = CLEAN_PRE_SESSION_POST_RI_MICRO_ROLLOUT
RI_MICRO = ENABLED / FLAT / NO_COMMANDS / ACTION_SCOPED_ONLY
ALL_LIVE_CONTOURS = FLAT / NO_WORKING_ORDERS
WARN_ERROR = RECONNECT_NOISE_ONLY_PENDING_COUNT_0
PATCH_REQUIRED = NO
OPERATOR_ACTION_REQUIRED = NO
```
