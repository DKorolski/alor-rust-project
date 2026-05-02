# VPS Live Observations (2026-05-02)

## Checkpoint

Observation time:

```text
2026-05-02 10:46-10:48 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
alor-usdrubf     USDRUBF hybrid / 7502T0U
hybrid           IMOEXF riskgate-shadow / 7502SN6
RI shadow        RIM6 / 7502MIW Author41/42 shadow
legacy RI shadow old contour
```

## VPS Resources

Host state:

```text
RAM total        7.7Gi
RAM used         1.8Gi
RAM available    5.9Gi
Swap used        50Mi / 3.9Gi
Disk /           35G / 79G used, 41G free, 47%
```

Docker storage:

```text
Images           61 total, 9 active
Image size       5.077GB
Reclaimable      5.016GB
Containers       15 active
```

Interpretation:

```text
No VPS memory pressure.
No disk pressure.
Docker images are a cleanup candidate, but not an urgent blocker.
```

Container memory:

```text
sessiongap redis             111.4MiB / 1GiB
alor-usdrubf redis           126.7MiB / 1GiB
hybrid redis                 149.4MiB / 1GiB
ri-7502miw redis              92.05MiB / 768MiB
legacy ri-shadow redis       150.8MiB / 768MiB
```

Redis logical memory:

```text
sessiongap redis             used=99.27M, peak=624.86M
alor-usdrubf redis           used=105.30M, peak=667.84M
hybrid redis                 used=137.42M, peak=648.18M
ri-7502miw redis             used=88.01M, peak=88.02M, maxmemory=512M
legacy ri-shadow redis       used=142.56M, peak=330.94M, maxmemory=512M
```

Interpretation:

```text
The daily Redis safe-trim automation appears effective.
Memory is well below the prior danger zone.
runtime.state.* and runtime.riskgate.* remain protected.
```

## Stack Health

Checked compose stacks:

```text
trading-sessiongap-*                  healthy, up 9 days
trading-alor-usdrubf-*                healthy, up 5-9 days
trading-hybrid-*                      healthy, up 19 hours
trading-ri-author41-42-7502miw-*      healthy, up 20 hours
trading-ri-shadow-*                   running, no explicit healthcheck
```

No container restart loop or unhealthy state was observed.

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
sessiongap:       no WARN/ERROR signatures in the checked window
alor-usdrubf:     no WARN/ERROR signatures in the checked window
ri-7502miw:       no WARN/ERROR signatures in the checked window
legacy ri-shadow: no WARN/ERROR signatures in the checked window
hybrid:           one orphan_trade warning immediately after from-zero restart
```

Hybrid orphan note:

```text
2026-05-01 15:50 MSK restart path emitted one orphan_trade warning
for already closed trade_id=2033126196669059325.

The warning did not restore stale active_cycle_id.
The strategy remained broker-flat at restart and later traded with a fresh
same-day cycle id.
```

## Position Safety

Latest broker snapshots:

```text
sessiongap USDRUBF       qty = 0.0, stop_orders = {}
alor-usdrubf USDRUBF     qty = 0.0, stop_orders = {}
hybrid IMOEXF            qty = 0.0, stop_orders = {}
ri-7502miw               no futures position, orders = {}, stop_orders = {}
```

## Hybrid IMOEXF Post-Patch Read

Observed 2026-05-01 BO lifecycle after stale-cycle patch rollout:

```text
15:50 MSK  submit_entry owner=IntradayBreakout side=Short reason=BreakoutShort
16:00 MSK  command accepted / execution confirmed
23:30 MSK  submit_exit owner=IntradayBreakout reason=BreakoutEodExit
23:43 MSK  command accepted / execution confirmed
```

Execution details:

```text
entry request_id = 2b666ffb-df99-5d69-b715-9c304411e8ad
entry order_id   = 2033126196669085943
entry exec       = sell 1 IMOEXF @ 2649.0

exit request_id  = f1fdb670-d5ef-51a1-8ac2-c38e56ff3677
exit order_id    = 2033126196669106563
exit exec        = buy 1 IMOEXF @ 2645.0
```

Runtime state after EOD exit:

```text
active_cycle_id = null
last_position_qty = 0.0
current_owner = null
pending_entry_request_id = null
pending_exit_request_id = null
safe_mode_close_only = false
was_short_today = true
last_trade_ts = 1777668236
```

Interpretation:

```text
Patch behavior looks correct in the first live read.
The fresh BO cycle used same-day cycle_id=69f4a17800.
No stale-cycle overnight guard regression repeated.
BO EOD exit fired and flattened the account.
No command reject or CWS error path was observed.
```

## Riskgate Ledger

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

Latest ledger rows:

```text
2026-04-30 source=runtime shadow_pnl_points=0.9 shadow_trade_count=1 rolling_sum_lb120=192.5
2026-04-29 source=runtime shadow_pnl_points=6.4 shadow_trade_count=1 rolling_sum_lb120=191.6
2026-04-28 source=runtime shadow_pnl_points=0.0 shadow_trade_count=0 rolling_sum_lb120=175.1
```

Observation:

```text
2026-05-01 is present as current_shadow_session_date but not yet finalized
as a ledger row at this checkpoint. This is a follow-up watchpoint, not an
execution incident, because runtime state and broker state are flat.
```

## RI 7502MIW Shadow

RI shadow state:

```text
mode = shadow
allow_order_emission = false
execution_path = action_scoped_only
live_adapter_enabled = false
model_bars_seen = 430
suppressed_service_bars = 76
model_decisions_seen = 6
phase = flat
last_transition_reason = dry_run_exit:take_author_close
```

Streams:

```text
cmd.orders.7502MIW.ri_author41_42.shadow = 0
cmd.acks.7502MIW.ri_author41_42.shadow   = 0
```

Timing read:

```text
last_model_bar_ts = 2026-05-01 23:30:00 MSK
last_bar_ts       = 2026-05-02 10:20:00 MSK
```

Interpretation:

```text
RI shadow continues safely in pre-GO mode.
The runtime sees newer bars, but the canonical model layer has not advanced
past the last regular model bar. This is consistent with weekend/service-bar
suppression.
```

## Verdict

```text
OVERALL_STATUS = ШТАТНО
RESOURCE_STATUS = SAFE
SESSIONGAP = FLAT / NO_ERRORS
ALOR_USDRUBF = FLAT / NO_ERRORS
HYBRID_IMOEXF = BO_EOD_EXIT_CONFIRMED / FLAT / PATCH_READ_POSITIVE
RI_7502MIW_SHADOW = NO_LIVE_COMMANDS / SHADOW_SAFE
WATCHPOINT = HYBRID_RISKGATE_2026_05_01_LEDGER_FINALIZATION
```

Follow-up:

```text
1. Re-check hybrid riskgate ledger after next regular finalization point.
2. Keep RI shadow observation for 7-14 sessions before any GO/NO-GO promotion.
3. Plan Docker image cleanup separately; current disk pressure is low.
```

## Safe Cleanup

Cleanup time:

```text
2026-05-02 10:51-10:52 MSK
```

Pre-cleanup state:

```text
Disk /           35G / 79G used, 41G free, 47%
Docker images    61 total, 9 active
Docker size      5.077GB
Docker reclaim   5.016GB
Dangling images  present
```

Action:

```text
docker image prune -f
```

Scope:

```text
Removed only dangling Docker images/layers.
Did not run docker image prune -a.
Did not remove tagged rollback images.
Did not remove Redis data, runtime state, riskgate ledger, or compose stacks.
Did not remove /opt rollout/build directories.
```

Result:

```text
Reclaimed        1.318GB
Disk /           34G / 79G used, 42G free, 45%
Docker images    56 total, 9 active
Docker size      3.759GB
Dangling images  none
```

Post-cleanup health:

```text
sessiongap containers healthy
alor-usdrubf containers healthy
hybrid containers healthy
ri-7502miw containers healthy
legacy ri-shadow containers running
```

Post-cleanup log and safety check:

```text
no fresh WARN/ERROR signatures in the checked 10m window
sessiongap USDRUBF qty = 0.0, stop_orders = {}
alor-usdrubf USDRUBF qty = 0.0, stop_orders = {}
hybrid IMOEXF qty = 0.0, stop_orders = {}
ri-7502miw cmd.orders = 0
ri-7502miw cmd.acks = 0
```

Cleanup verdict:

```text
SAFE_CLEANUP_COMPLETED
NO_SERVICE_REGRESSION_OBSERVED
NO_PATCH_REQUIRED_FROM_THIS_CHECKPOINT
```
