# VPS Live Observations (2026-05-03)

## Checkpoint

Observation time:

```text
2026-05-03 09:36-09:40 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
alor-usdrubf     USDRUBF hybrid / 7502T0U
hybrid           IMOEXF riskgate-shadow / 7502SN6
RI shadow        RIM6 / 7502MIW Author41/42 shadow
legacy RI shadow old contour
VPS resources and safe-trim maintenance
```

## VPS Resources

Host state:

```text
RAM total        7.7Gi
RAM used         1.8Gi
RAM available    5.9Gi
Swap used        225Mi / 3.9Gi
Disk /           42G / 79G used, 33G free, 57%
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
sessiongap redis             101MiB / 1GiB
alor-usdrubf redis           110.2MiB / 1GiB
hybrid redis                 107.9MiB / 1GiB
ri-7502miw redis             195.2MiB / 768MiB
legacy ri-shadow redis       145.7MiB / 768MiB
```

Interpretation:

```text
No RAM pressure.
No container memory pressure.
Disk usage increased from the prior checkpoint, but remains below the danger
zone. The growth is not caused by live Docker JSON logs.
```

Disk investigation:

```text
/var/log                     675M
/var/cache                   876M
/var/lib                     5.2G
/opt/barter-rs               17G
/opt/bybit_barter_eth_bo_v2  9.3G
```

Largest identified files are old Rust build artifacts:

```text
/opt/barter-rs/target/debug/deps/libbarter_data-*.rlib                  ~4.9G
/opt/barter-rs/target/debug/examples/bybit_hybrid_smoke_probe*          ~2.5G each
/opt/bybit_barter_eth_bo_v2/target/debug/deps/barter_data-*             ~2.5G
/opt/barter-rs*/target/debug/incremental/*                              multiple 100M-900M objects
```

Conclusion:

```text
Disk growth is mainly old build artifacts under /opt, not current live soak
streams or container logs.
Do not clean them automatically in this observation pass; schedule a separate
safe build-artifact cleanup if needed.
```

## Stack Health

Checked compose stacks:

```text
trading-sessiongap-*                  healthy, up 10 days
trading-alor-usdrubf-*                healthy, up 6-10 days
trading-hybrid-*                      healthy, up 42 hours
trading-ri-author41-42-7502miw-*      healthy, up 43 hours
trading-ri-shadow-*                   running, no explicit healthcheck
```

No restart loop or unhealthy state was observed.

## Log Scan

Checked last 24h for:

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
Night/weekend live_guard_changed transitions:
  ALLOWED/BLOCKED
  gateway_ready=false
  ws_connected=false
  cws_authorized=false
  SyncingGap / SyncingHistory
```

Interpretation:

```text
These transitions match previously observed reconnect/session-boundary behavior
and recovered normally. No command reject storm or Redis read failure was found.
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
sessiongap phase = Flat, last session = 2026-04-30
alor-usdrubf lifecycle_stage = broker_position_flat
hybrid active_cycle_id = null, last_position_qty = 0.0
ri-7502miw phase = flat, mode = shadow
```

## Hybrid IMOEXF Watchpoint

Hybrid riskgate state:

```text
ledger_rows_count = 184
last_finalized_session_date = 2026-04-30
rolling_sum_lb120 = 192.5000000000002
mr_enabled_current_session = true
mr_enabled_next_session = true
current_shadow_session_date = 2026-05-01
current_shadow_pnl_points = 0.0
```

Observation:

```text
2026-05-01 is still not finalized as a riskgate ledger row at this checkpoint.
This remains a watchpoint. It is not an execution incident because broker
position, runtime lifecycle state, and stop orders are flat/clean.
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
model_bars_seen = 430
suppressed_service_bars = 114
model_decisions_seen = 6
phase = flat
last_transition_reason = dry_run_exit:take_author_close
```

Streams:

```text
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow   = 0
runtime.state.ri_author41_42.shadow.7502MIW = 89
md.bars.7502MIW.RIM6.10m = 999
```

Timing read:

```text
last_model_bar_ts = 2026-05-01 23:30:00 MSK
last_bar_ts       = 2026-05-02 10:30:00 MSK
```

Interpretation:

```text
RI shadow remains pre-GO safe.
No live command stream writes were observed.
Service/weekend bars continue to be suppressed from model progression.
```

## Redis Safe Trim Maintenance Patch

Finding:

```text
The daily Redis safe-trim timer ran successfully at 2026-05-03 03:10 MSK,
but the new RI 7502MIW Redis container was not included in the CONTAINERS
whitelist.
```

Evidence:

```text
trading-ri-author41-42-7502miw-redis-1 was absent from the trim log.
ri-7502miw redis used_memory_human grew to ~186.85M.
broker.snapshots.7502MIW length was 15444 before manual apply.
```

Patch applied on VPS:

```text
script = /opt/trading-maintenance/redis_safe_trim.sh
backup = /opt/trading-maintenance/redis_safe_trim.sh.bak_20260503_093826_add_ri7502miw
added container = trading-ri-author41-42-7502miw-redis-1
bash -n = pass
```

Updated whitelist:

```text
trading-sessiongap-redis-1
trading-hybrid-redis-1
trading-alor-usdrubf-redis-1
trading-ri-author41-42-7502miw-redis-1
trading-ri-shadow-redis-1
```

Dry-run result for RI 7502MIW:

```text
broker.snapshots.7502MIW len=15438 limit=10000 action=would_trim
md.bars.7502MIW.RIM6.10m len=999 limit=3000 action=skip
cmd.orders.7502MIW.ri_author41_42.shadow len=0 action=skip
cmd.acks.7502MIW.ri_author41_42.shadow len=0 action=skip
```

Apply result:

```text
broker.snapshots.7502MIW len_before=15444 len_after=10000 limit=10000 action=trimmed
broker.positions.7502MIW len=8 action=skip
broker.orders.7502MIW len=0 action=skip
broker.trades.7502MIW len=0 action=skip
md.bars.7502MIW.RIM6.10m len=999 action=skip
cmd.orders.7502MIW.ri_author41_42.shadow len=0 action=skip
cmd.acks.7502MIW.ri_author41_42.shadow len=0 action=skip
```

Post-apply memory:

```text
sessiongap redis             65.63M
alor-usdrubf redis           70.63M
hybrid redis                 70.25M
ri-7502miw redis             186.07M
legacy ri-shadow redis       102.18M
```

Note:

```text
RI 7502MIW used_memory did not drop materially immediately after trimming the
snapshot stream. This is likely allocator/fragmentation behavior. The important
control is now in place: snapshot stream length is capped and future growth
will be bounded by the daily timer.
```

Post-maintenance health:

```text
all main containers healthy/running
no fresh WARN/ERROR signatures in the checked 10m post-maintenance window
sessiongap/alor-usdrubf/hybrid remain flat
ri-7502miw command streams remain 0/0
```

## Verdict

```text
OVERALL_STATUS = ШТАТНО
RESOURCE_STATUS = SAFE_BUT_DISK_WATCH
SESSIONGAP = FLAT / NO_ERRORS
ALOR_USDRUBF = FLAT / NO_ERRORS
HYBRID_IMOEXF = FLAT / NO_ERRORS / RISK_LEDGER_WATCHPOINT
RI_7502MIW_SHADOW = NO_LIVE_COMMANDS / SHADOW_SAFE
MAINTENANCE_PATCH = REDIS_SAFE_TRIM_INCLUDE_RI_7502MIW
```

Follow-up:

```text
1. Re-check hybrid riskgate 2026-05-01 finalization after the next regular cycle.
2. Confirm the 2026-05-04 03:10 MSK safe-trim timer includes ri-7502miw.
3. Plan separate cleanup for old /opt Rust target/debug artifacts if disk keeps growing.
4. Continue RI shadow observation; still too early for promotion.
```
