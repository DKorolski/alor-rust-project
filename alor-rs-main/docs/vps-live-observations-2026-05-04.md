# VPS Live Observations (2026-05-04)

## Checkpoint

Observation time:

```text
2026-05-04 09:39-09:41 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
alor-usdrubf     USDRUBF hybrid / 7502T0U
hybrid           IMOEXF riskgate-shadow / 7502SN6
RI shadow        RIM6 / 7502MIW Author41/42 shadow
legacy RI shadow old contour
VPS resources, Redis safe-trim, and disk cleanup state
```

## VPS Resources

Host state:

```text
RAM total        7.7Gi
RAM used         2.0Gi
RAM available    5.8Gi
Swap used        320Mi / 3.9Gi
Disk /           20G / 79G used, 56G free, 26%
```

Docker state:

```text
Images           56 total, 9 active
Docker size      3.759GB
Reclaimable      3.699GB
Containers       15 active
```

Container memory:

```text
sessiongap redis             106MiB / 1GiB
alor-usdrubf redis           105.9MiB / 1GiB
hybrid redis                 109.5MiB / 1GiB
ri-7502miw redis             293.3MiB / 768MiB
legacy ri-shadow redis       144.7MiB / 768MiB
```

Interpretation:

```text
No RAM pressure.
Disk pressure resolved compared with the 2026-05-03 77% checkpoint.
The previously identified Rust build-artifact footprint under /opt is now much
smaller, though target directories still exist.
```

Build-artifact state:

```text
/opt/barter-rs               1.6G
/opt/bybit_barter_eth_bo_v2  1.2G
/opt/barter-rs-hybrid-smoke  1.4G
```

## Stack Health

Checked compose stacks:

```text
trading-sessiongap-*                  healthy, up 11 days
trading-alor-usdrubf-*                healthy, up 7-11 days
trading-hybrid-*                      healthy, up 2 days
trading-ri-author41-42-7502miw-*      healthy, up 2 days
trading-ri-shadow-*                   running, no explicit healthcheck
```

No restart loop or unhealthy state was observed.

## Log Scan

Checked last 30h for:

```text
WARN
ERROR
panic
command rejected
Connection refused
broken pipe
OOM / out of memory
orphan trade
```

Findings:

```text
sessiongap:       no WARN/ERROR signatures
alor-usdrubf:     no WARN/ERROR signatures
hybrid:           no WARN/ERROR signatures
ri-7502miw:       no WARN/ERROR signatures
legacy ri-shadow: no WARN/ERROR signatures
```

Observed operational noise:

```text
Night/weekend guard transitions:
  gateway_ready=false
  ws_connected=false
  cws_authorized=false
  SyncingGap / SyncingHistory
```

Interpretation:

```text
This matches normal reconnect/session-boundary behavior. No command reject
storm, Redis reader failure, or CWS error path was found in this checkpoint.
```

## Position Safety

Latest broker snapshots:

```text
sessiongap USDRUBF       qty = 0.0, stop_orders = {}
alor-usdrubf USDRUBF     qty = 0.0, stop_orders = {}
hybrid IMOEXF            qty = 0.0, stop_orders = {}
ri-7502miw               no futures position, orders = {}, stop_orders = {}
```

Runtime states:

```text
sessiongap phase = Flat, current session_date = 2026-05-04
alor-usdrubf lifecycle_stage = live, hybrid_state = flat
hybrid active_cycle_id = null, last_position_qty = 0.0
ri-7502miw phase = flat, mode = shadow
```

## SessionGap

State:

```text
session_date = 2026-05-04
traded_session = false
phase = Flat
prev_close = 75.22
first_min_high = 75.17
first_min_low = 75.17
first_hour_price = null
last_bar_ts = 2026-05-04 09:30:00 MSK
```

Interpretation:

```text
SessionGap is live and flat. No entry was expected at this early checkpoint
before the relevant signal window is complete.
```

## Alor-USDRUBF

State:

```text
lifecycle_stage = live
hybrid_state = flat
current_date_local = 2026-05-04
open_position_qty = 0.0
pending_request_ids = []
tracked_order_ids = []
entry_intent_inflight = false
exit_intent_inflight = false
```

Interpretation:

```text
Alor-USDRUBF is live, synchronized, and flat.
No stale pending lifecycle state was observed.
```

## Hybrid IMOEXF

Runtime state:

```text
active_cycle_id = null
last_position_qty = 0.0
current_owner = null
pending_entry_request_id = null
pending_exit_request_id = null
safe_mode_close_only = false
last_day_local = 2026-05-04
today_start_local = 2026-05-04T09:00:00
```

Riskgate state:

```text
ledger_rows_count = 185
last_finalized_session_date = 2026-05-01
rolling_sum_lb120 = 192.5000000000002
mr_enabled_current_session = true
mr_enabled_next_session = true
risk_gate_shadow_session_date = 2026-05-04
```

Resolved watchpoint:

```text
The prior riskgate watchpoint is resolved.
At 2026-05-04 09:10:07 MSK, session 2026-05-01 was finalized:

action = risk_gate_shadow_session_finalized
session_date = 2026-05-01
shadow_pnl_points = 0.0
shadow_trade_count = 0
ledger_rows_count = 185
```

Interpretation:

```text
Hybrid is flat and the riskgate ledger caught up after the next regular
session cycle. No patch is required for this item.
```

## RI 7502MIW Shadow

RI runtime state:

```text
mode = shadow
profile_id = ri_author41_42_primary_combo_cost2
timeframe = 10m
allow_order_emission = false
execution_path = action_scoped_only
live_adapter_enabled = false
model_bars_seen = 434
suppressed_service_bars = 148
model_decisions_seen = 6
phase = flat
last_transition_reason = dry_run_exit:take_author_close
```

Streams:

```text
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow   = 0
runtime.state.ri_author41_42.shadow.7502MIW = 127
md.bars.7502MIW.RIM6.10m = 1037
broker.snapshots.7502MIW = 12340
```

Timing read:

```text
last_model_bar_ts = 2026-05-04 09:30:00 MSK
last_bar_ts       = 2026-05-04 09:30:00 MSK
```

Interpretation:

```text
RI shadow progressed into the regular Monday session and remains pre-GO safe.
No live command stream writes were observed.
```

## Redis Safe Trim

Timer/service status:

```text
trading-redis-safe-trim.service last run = 2026-05-04 03:10 MSK
status = SUCCESS
next timer = 2026-05-05 03:10 MSK
```

Important 2026-05-04 trim evidence:

```text
sessiongap broker.snapshots.7502MIW len_before=16309 len_after=10000
hybrid broker.snapshots.7502SN6 len_before=16306 len_after=10000
alor-usdrubf broker.snapshots.7502T0U len_before=16306 len_after=10000
ri-7502miw broker.snapshots.7502MIW len_before=16306 len_after=10000
legacy-ri broker.snapshots.7502SN6 len_before=16309 len_after=10000
```

Redis logical memory after checkpoint:

```text
sessiongap redis             94.30M
alor-usdrubf redis           100.18M
hybrid redis                 99.44M
ri-7502miw redis             288.73M / 512M maxmemory
legacy ri-shadow redis       139.41M / 512M maxmemory
```

Observation:

```text
The RI 7502MIW container is included in the timer and snapshots are trimmed.
Its Redis memory remains higher than the other contours, likely allocator and
snapshot churn behavior. Continue watching, but it is below maxmemory.
```

## Verdict

```text
OVERALL_STATUS = ШТАТНО
RESOURCE_STATUS = SAFE
SESSIONGAP = FLAT / NO_ERRORS
ALOR_USDRUBF = LIVE_FLAT / NO_ERRORS
HYBRID_IMOEXF = FLAT / RISK_LEDGER_WATCHPOINT_RESOLVED
RI_7502MIW_SHADOW = NO_LIVE_COMMANDS / SHADOW_SAFE
REDIS_SAFE_TRIM = ACTIVE_AND_COVERING_RI_7502MIW
PATCH_REQUIRED = NO
```

Follow-up:

```text
1. Continue daily checks during extended micro soak.
2. Watch RI 7502MIW Redis memory and broker.snapshots length before/after timer.
3. Keep RI shadow in observation; still too early for GO/NO-GO promotion.
4. No code patch is required from this checkpoint.
```
