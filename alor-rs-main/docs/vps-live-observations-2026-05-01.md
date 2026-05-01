# VPS Live Observations (2026-05-01)

## Checkpoint

Observation time:

```text
2026-05-01 10:22 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
hybrid           IMOEXF riskgate-shadow / 7502SN6
alor-usdrubf     USDRUBF hybrid / 7502T0U
RI shadow        RIM6 feed -> RI Author41/42 shadow journal
```

## Stack Status

All checked containers were up:

```text
trading-sessiongap-*     up 8 days, healthy
trading-hybrid-*         up 2-3 days, healthy
trading-alor-usdrubf-*   up 4-8 days, healthy
trading-ri-shadow-*      up 2 days
```

VPS resources at checkpoint:

```text
RAM         2.5G used / 7.7G total (5.2G available)
Swap        54M used / 3.9G
Disk /      39G used / 79G (52%)
```

Redis memory:

```text
sessiongap redis      289.58M
hybrid redis          498.71M
alor-usdrubf redis    305.03M
RI shadow redis       329.73M / 512M
```

## Session Regime (Unusual Friday)

Observed bar tails:

```text
hybrid (IMOEXF):      live 10m bars on 2026-05-01
ri-shadow (RIM6):     live 10m bars on 2026-05-01
alor-usdrubf:         latest live activity ended on prior regular session;
                      startup replay contains history_gap bars
sessiongap:           no fresh live trading flow for this holiday-like regime
```

Interpretation: the day behaves like a split regime where IMOEXF/RI continue
trading, while USDRUBF is effectively inactive.

## Log Scan

Checked last ~30h for the three strategy runtimes and RI runner.

Findings:

```text
No panic or fatal runtime errors.
No new command reject storms.
No Redis xreadgroup broken-pipe/connection-refused incident in this window.
```

Seen as expected operational noise:

```text
live_guard_changed transitions during reconnect/sync phases:
  ALLOWED -> BLOCKED -> SyncingGap/SyncingHistory -> ALLOWED
```

This matches previously observed gateway transport behavior and recovered
normally.

## Position Safety

Latest broker position tails indicate flat state for active live stacks:

```text
sessiongap       no open USDRUBF futures position
hybrid           no open IMOEXF futures position
alor-usdrubf     no open USDRUBF futures position
```

RI shadow runner also remained active and continued writing journal updates.

## Verdict

```text
Overall state: штатно.
```

No new blocking incident was found. The main operational watchpoint remains
Redis growth (especially hybrid + RI shadow), but at this checkpoint it is still
within safe operating bounds.

Follow-up:

```text
1. Keep daily Redis memory checks while extended micro soak is active.
2. Keep logging split-regime days explicitly (USDRUBF inactive vs IMOEXF/RI active).
3. If hybrid redis approaches ~600M and growth accelerates, run safe trim in non-trading window.
```

## Redis Safe Trim Automation

The whitelist Redis trim was extended to include `trading-ri-shadow-redis-1`
and installed as a daily systemd timer:

```text
service = trading-redis-safe-trim.service
timer   = trading-redis-safe-trim.timer
time    = daily 03:10 MSK
log     = /var/log/trading-redis-safe-trim.log
script  = /opt/trading-maintenance/redis_safe_trim.sh --apply
```

The script remains whitelist-only:

```text
trimmed:
  events.health
  broker.snapshots.*
  broker.positions.*
  broker.orders.*
  broker.trades.*
  cmd.orders.*
  cmd.acks.*
  md.bars.*

protected:
  runtime.state.*
  runtime.riskgate.*
```

Manual apply result at `2026-05-01 10:35 MSK`:

```text
sessiongap redis      302.0M -> 65.63M
hybrid redis          510.3M -> 104.01M
alor-usdrubf redis    327.0M -> 70.31M
RI shadow redis       339.0M -> 94.32M
```

Post-trim check:

```text
all containers remained up/healthy
no fresh runtime errors, xreadgroup failures, NOGROUP, or BusyLoading signatures
next timer run = 2026-05-02 03:10 MSK
```

Verdict: safe trim automation is active and validated by one manual apply.

## RI Shadow Journal Watermark Fix

The initial RI shadow journal showed inflated counts:

```text
2026-04-28: 62 records, MR 1, BO 61, total shadow_pnl +59396
```

Root cause:

```text
The live shadow runner rebuilds replay state after every incoming bar.
An open BO candidate was provisionally closed at the current tail bar as
forced_last_bar_close. Because the provisional scheduled_exit_ts changed on
each new bar, the append-only journal treated each tail update as a new record.
```

The runner was patched so same-day `forced_last_bar_close` records are treated
as provisional and are not written until the record belongs to a previous
regular session. Non-forced same-day exits still write immediately.

Deployment:

```text
new image = ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-7c590e4-ri-shadow-watermark
service   = trading-ri-shadow-ri-shadow-runner-1
scope     = runner only; gateway/redis unchanged; no order-emission path
```

The pre-patch journal was archived and compacted:

```text
raw archive:
  /opt/trading-ri-shadow/volumes/reports/moex_author41_42_shadow_ri.pre_watermark_20260501-104001.jsonl

compacted view:
  /opt/trading-ri-shadow/volumes/reports/moex_author41_42_shadow_ri.pre_watermark_20260501-104001.compacted.jsonl
```

Compacted read:

```text
rows_before = 78
rows_after  = 6

2026-04-28: MR 1, BO 1, shadow_pnl +1816
2026-04-29: MR 1, BO 1, shadow_pnl -34
2026-04-30: MR 2, BO 0, shadow_pnl +356
```

The active journal was reset after archive:

```text
/opt/trading-ri-shadow/volumes/reports/moex_author41_42_shadow_ri.jsonl = 0 rows at restart
runner warmup_bars = 488
runner warmup_records = 6
write_warmup = false
consumer lag = 0
pending = 0
```

Verdict: the previous `BO 61` count was a journal-finalization artifact, not 61
independent RI BO trades. Future live journal rows should represent finalized
shadow decisions rather than moving tail snapshots.

## RI Shadow Rollout Gate

Operator decision after the watermark fix:

```text
Keep RI Author41/42 in shadow observation for another 3-5 trading sessions.
Do not promote validation status yet.
```

Rationale:

- the journal-finalization bug is now patched, but the active post-patch
  journal has not yet accumulated enough finalized records;
- the compacted pre-patch read is useful, but promotion should be based on
  clean post-watermark live rows;
- watchpoints for the next 3-5 trading sessions:
  - no duplicate same-day provisional BO records;
  - active journal appends only finalized decisions;
  - Redis consumer remains `lag=0`, `pending=0`;
  - MR/BO attribution remains plausible against the live RIM6 10m feed.

Status:

```text
RI_SHADOW_WATERMARK_PATCHED / OBSERVE_3_TO_5_MORE_TRADING_SESSIONS
```

## VPS Resource Maintenance Check

Observation time:

```text
2026-05-01 14:38 MSK
host = nektodk.ispvds.com
```

Pre-cleanup resource state:

```text
RAM available:       6.2G / 7.7G
Swap used:           55M / 3.9G
Disk /:              40G used / 79G (54%), 35G free
Docker reclaimable:  8.46G
systemd journal:     2.3G
```

Redis memory before targeted trim:

```text
sessiongap redis      83.48M
alor-usdrubf redis    88.70M
hybrid redis          129.40M
RI shadow redis       116.89M / 512M
```

Safe cleanup applied online:

```text
docker image prune -f
journalctl --vacuum-size=512M
XTRIM events.health MAXLEN ~ 5000 on all four Redis containers
```

Scope intentionally not touched:

```text
runtime.state.*
runtime.riskgate.*
broker.snapshots.*
broker.orders/trades/positions
md.bars.*
cmd.orders / cmd.acks
```

Post-cleanup resource state:

```text
RAM available:       6.3G / 7.7G
Disk /:              34G used / 79G (45%), 42G free
Docker images:       3.74G total, 3.68G still reclaimable tagged old images
systemd journal:     486.7M
```

Redis memory after health-stream trim:

```text
sessiongap redis      36.75M, events.health ~= 5006
alor-usdrubf redis    42.03M, events.health ~= 5006
hybrid redis          82.77M, events.health ~= 5006
RI shadow redis       70.29M, events.health ~= 5006
```

Container status after cleanup:

```text
all 12 containers remained Up
live strategy/runtime containers remained healthy where healthcheck exists
```

Verdict:

```text
Resource state is safe. No Redis pressure or disk pressure remains after
online cleanup.
```

Operational note:

The current `trading-ri-shadow` deployment still appears to be the older RI
shadow contour:

```text
redis keys observed:
  md.bars.RI.10m
  broker.snapshots.7502SN6
  cmd.orders.7502SN6
```

This does not match the new prepared `7502MIW/RIM6` runbook contour:

```text
expected next target:
  md.bars.7502MIW.RIM6.10m
  cmd.orders.7502MIW.ri_author41_42.shadow
  cmd.acks.7502MIW.ri_author41_42.shadow
```

Do not treat the current RI shadow container as proof that the new `7502MIW`
contour is deployed. Next RI rollout should explicitly replace or create the
target `7502MIW/RIM6` shadow stack from the prepared configs.

## RI Author41/42 7502MIW Shadow Contour Deployment

Observation time:

```text
2026-05-01 15:05 MSK
```

Action:

```text
created separate stack:
  /opt/trading-ri-author41-42-7502miw
compose project:
  trading-ri-author41-42-7502miw
```

Reason for separate stack:

```text
the existing trading-ri-shadow stack is the older handoff contour and still
uses 7502SN6 / md.bars.RI.10m style streams; the new target must be isolated
as 7502MIW / RIM6 / 10m with no shared command stream.
```

Runtime image note:

```text
initial runtime attempt with manual-7c590e4-ri-shadow-watermark failed because
the image did not yet know strategy_kind=ri_author41_42.

fresh runtime image built on VPS:
  ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-d6d0e7e-ri7502miw-20260501
```

Container status after deployment:

```text
trading-ri-author41-42-7502miw-redis-1              healthy
trading-ri-author41-42-7502miw-alor-gateway-1       healthy
trading-ri-author41-42-7502miw-strategy-runtime-1   healthy
```

Gateway resolved contour:

```text
portfolio = 7502MIW
symbol = RIM6
tf_sec = 600
bars = md.bars.7502MIW.RIM6.10m
commands = cmd.orders.7502MIW.ri_author41_42.shadow
acks = cmd.acks.7502MIW.ri_author41_42.shadow
control_cws_mode = action_scoped
```

Runtime resolved contour:

```text
strategy_kind = ri_author41_42
strategy_id = ri_author41_42.shadow.7502MIW
trade_mode = Paper
allow_live_orders = false
mode = shadow
allow_order_emission = false
execution_path = action_scoped_only
decision_journal_path = /reports/ri_author41_42_7502MIW_decisions.jsonl
```

Runtime startup evidence:

```text
bootstrap warmup completed
bars_processed = 457
mode = shadow
allow_order_emission = false
live_adapter_enabled = false
```

Redis state after startup:

```text
md.bars.7502MIW.RIM6.10m                  stream 912
events.health.ri_author41_42.7502MIW      stream 192
cmd.orders.7502MIW.ri_author41_42.shadow  stream 0
cmd.acks.7502MIW.ri_author41_42.shadow    stream 0
runtime.state.ri_author41_42.shadow.7502MIW stream 2
broker.snapshots.7502MIW                  stream 99
broker.positions.7502MIW                  stream 0
```

Journal review:

```text
rows = 15
status = PASS
live emission evidence rows = 0
duplicate shadow_recorded decision keys = 0
unexpected adapter decisions = 0
unexpected execution paths = 0
```

Decision mix:

```text
adapter_decision:
  shadow_recorded = 5
  intent_suppressed = 10
component:
  author41_mr = 12
  author42_bo = 3
execution_path:
  action_scoped_only = 10
  not_applicable_pre_go = 5
```

Interpretation:

```text
action_scoped_only appears on command-capable entry/exit rows.
not_applicable_pre_go appears only on pure shadow_recorded observation rows.
No request_id or broker_order_id was observed.
```

Resource impact of the new stack:

```text
strategy-runtime   ~2.8 MiB / 768 MiB
alor-gateway       ~3.7 MiB / 768 MiB
redis              ~5.1 MiB / 768 MiB
disk /             34G used / 79G (46%), 41G free
```

Verdict:

```text
RI_AUTHOR41_42_7502MIW_SHADOW_CONTOUR_DEPLOYED
PRE_GO_SHADOW_ONLY_VALIDATION_PASS
CONTINUE_OBSERVATION_7_TO_14_TRADING_SESSIONS
```

## Hybrid IMOEXF BO Stale Cycle Follow-Up

Observation time:

```text
2026-05-01 15:30 MSK
```

Observed sequence:

```text
2026-05-01 12:50 MSK  BO short entry emitted, accepted, filled
2026-05-01 13:00 MSK  breakout_no_overnight_guard_exit fired
2026-05-01 13:00 MSK  BO exit emitted, accepted, filled
latest broker snapshot IMOEXF qty = 0
```

The execution path itself was healthy:

```text
entry request accepted
entry execution confirmed
exit request accepted
exit execution confirmed
no CWS reject/error path observed
```

The suspicious field was lifecycle state:

```text
active_cycle_day = 2026-04-28
cycle_id = 69f04ce000
dt_local = 2026-05-01 13:00:00
```

Interpretation:

```text
This is not a broker-flat failure and not a CWS/action-scoped regression.
The position was closed safely, but the BO no-overnight guard appears to have
used a stale historical cycle id restored from old HYB-tagged order events.
That can make a fresh same-day BO position look like an overnight carry.
```

Patch line prepared:

```text
terminal historical order events no longer seed active_cycle_id from HYB tags
working tagged orders/stop-orders still can restore active_cycle_id
bootstrap working-order recovery remains intact
```

Regression tests added:

```text
terminal_historical_order_does_not_seed_stale_cycle_for_new_bo_entry
working_tagged_order_can_restore_active_cycle
```

Local validation:

```text
cargo test -p strategy-runtime hybrid_intraday_runtime --lib
57 passed
```

Status:

```text
HYBRID_BO_STALE_CYCLE_ID_PATCH_PREPARED
REBUILD_AND_ROLLOUT_REQUIRED_BEFORE_NEXT_VALIDATION_READ
```

## Hybrid IMOEXF Stale-Cycle Patch Rollout

Rollout window:

```text
2026-05-01 15:41-15:51 MSK
```

Pre-rollout safety checks:

```text
IMOEXF broker position qty = 0
broker snapshot positions = {}
broker snapshot stop_orders = {}
no active working orders observed
no new strategy intents in the last 30 minutes before rollout
```

Rolled out image:

```text
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-512b8e1-hybrid-stalecycle-20260501
```

Runtime reset scope:

```text
Deleted:
runtime.state.hybrid_intraday.live.action_scoped.imoexf.7502SN6
runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6

Preserved:
runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120
runtime.riskgate.state.hybrid_imoexf.imoexf_primary_high180_lb120
runtime.riskgate.finalized.hybrid_imoexf.imoexf_primary_high180_lb120.*
broker snapshots / market data / command and ack streams
```

Post-restart checks:

```text
containers healthy: redis, alor-gateway, strategy-runtime
runtime live guard: ALLOWED
risk gate decision: UseExistingLedger
risk gate existing_records_loaded = 184
risk gate records_inserted = 0
risk gate records_duplicate = 0
mr_enabled_current_session = true
rolling_sum_lb120 = 192.5000000000002
last_finalized_session_date = 2026-04-30
broker snapshot positions = {}
broker snapshot stop_orders = {}
runtime state active_cycle_id = null
runtime state current_owner = null
runtime state last_position_qty = 0.0
risk_gate_shadow_session_date = 2026-05-01
```

Notes:

```text
The rollout intentionally reset only runtime-owned operational state.
The riskgate ledger was kept as the canonical long-lived risk memory.

After from-zero startup the live guard remained blocked until a fresh live
bar after the startup replay boundary. This was expected and prevented replay
tail execution immediately after restart. At 15:51:54 MSK the guard moved to
ALLOWED with reasons_after = [].

One orphan_trade warning was observed from the already closed 2026-05-01
trade path. It did not restore active_cycle_id and broker state remained flat.
```

Status:

```text
PATCHED_IMAGE_DEPLOYED
FROM_ZERO_RUNTIME_STATE_CLEAN
RISK_GATE_LEDGER_PRESERVED
LIVE_READY_CONFIRMED_AFTER_FRESH_BAR
```
