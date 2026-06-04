# VPS Live Observations - 2026-05-08

## Pre-Session Health Check

Collection time: 2026-05-08 08:04-08:10 MSK.

Scope:

```text
sessiongap
hybrid IMOEXF
alor-USDRUBF
RI author41/42 micro
RI shadow
```

Container health:

```text
sessiongap gateway/runtime/redis: healthy
hybrid gateway/runtime/redis: healthy
alor-USDRUBF gateway/runtime/redis: healthy
RI micro gateway/runtime/redis: healthy
RI shadow gateway/runner/redis: running
```

VPS resources:

```text
RAM: 7.7Gi total, 1.7Gi used, 6.0Gi available
swap: 3.9Gi total, 88Mi used
disk /: 79G total, 37G used, 38G available, 50%

Redis memory:
  sessiongap: 102.2MiB / 1GiB
  hybrid: 157.1MiB / 1GiB
  alor-USDRUBF: 135.5MiB / 1GiB
  RI micro: 50.8MiB / 768MiB
  RI shadow: 150.5MiB / 768MiB
```

Runtime log summary over last 14h:

```text
sessiongap:
  WARN=0
  ERROR=0
  intent_emitted=0
  command_rejected=0
  manual_intervention=0

hybrid IMOEXF:
  WARN=0
  ERROR=0
  intent_emitted=0
  command_rejected=0
  manual_intervention=0

alor-USDRUBF:
  WARN=0
  ERROR=0
  intent_emitted=0
  command_rejected=0
  manual_intervention=0

RI author41/42 micro:
  WARN=0
  ERROR=0
  intent_emitted=0
  command_rejected=0
  manual_intervention=0

RI shadow:
  WARN=0
  ERROR=0
```

Gateway log summary over last 14h:

```text
sessiongap gateway:
  WARN=11
  ERROR=0
  command_received=0
  CWS authorization successful after reconnects

hybrid gateway:
  WARN=10
  ERROR=0
  command_received=0
  CWS authorization successful after reconnects

alor-USDRUBF gateway:
  WARN=11
  ERROR=0
  command_received=0
  CWS authorization successful after reconnects

RI micro gateway:
  WARN=10
  ERROR=0
  command_received=0
  CWS authorization successful after reconnects

RI shadow gateway:
  WARN=8
  ERROR=0
  command_received=0
  CWS authorization successful after reconnects
```

Interpretation:

```text
Gateway WARNs are overnight CWS/WS transport reconnects:
  eof / TLS close_notify missing / reset without close handshake.

All observed gateway reconnects had pending_count=0 and no command_received
events. There were no order-path rejects, no action-scoped send failures, and
no runtime intent emissions during the checked window.

The runtime guard transitions around 23:40-00:04 and 03:24-03:33 MSK are
consistent with overnight MOEX transport/session transitions:
  gateway_ready=false
  phase=SyncingGap
  phase=SyncingHistory
  occasional ws_connected=false / cws_authorized=false
```

Broker snapshots / flat check:

```text
sessiongap Redis broker.snapshots.7502MIW:
  USDRUBF qty=0
  RTS-6.26 qty=0
  RUB cash present

hybrid Redis broker.snapshots.7502SN6:
  IMOEXF qty=0
  RUB cash present

alor-USDRUBF Redis broker.snapshots.7502T0U:
  USDRUBF qty=0
  RUB cash present

RI micro Redis broker.snapshots.7502MIW:
  RTS-6.26 qty=0
  RUB cash present

RI shadow Redis broker.snapshots.7502SN6:
  IMOEXF qty=0
  RUB cash present
```

Redis stream check:

```text
sessiongap:
  md.bars.7502MIW.10m XLEN=1168
  cmd.orders.7502MIW XLEN=6
  cmd.acks.7502MIW XLEN=6
  runtime.state.session_gap_standalone.live.7502MIW XLEN=500

hybrid:
  md.bars.7502SN6.10m XLEN=2736
  cmd.orders.7502SN6 XLEN=76
  cmd.acks.7502SN6 XLEN=75
  runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6 XLEN=500

alor-USDRUBF:
  md.bars.7502T0U.10m XLEN=1347
  cmd.orders.7502T0U XLEN=14
  cmd.acks.7502T0U XLEN=14
  runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U XLEN=501

RI micro:
  md.bars.7502MIW.RIM6.10m XLEN=3000
  cmd.orders.7502MIW.ri_author41_42.micro XLEN=0
  cmd.acks.7502MIW.ri_author41_42.micro XLEN=0
  runtime.state.ri_author41_42.micro.7502MIW XLEN=77

RI shadow:
  md.bars.RI.10m XLEN=993
  cmd.orders.7502SN6 XLEN=0
```

RI micro post-hardening note:

```text
RI micro remains on runtime image:
  manual-fcc1dab-ri-service-20260507

No new RI micro command/ack records were created overnight after yesterday's
from-zero service-hardening rollout. The latest snapshot is flat.
```

Current pre-session status:

```text
SESSIONGAP = RUNNING / HEALTHY / FLAT / NO_NEW_INTENTS
HYBRID_IMOEXF = RUNNING / HEALTHY / FLAT / NO_NEW_INTENTS
ALOR_USDRUBF = RUNNING / HEALTHY / FLAT / NO_NEW_INTENTS
RI_MICRO = RUNNING / HEALTHY / FLAT / SERVICE_HARDENED / NO_NEW_INTENTS
RI_SHADOW = RUNNING / HEALTHY / FLAT / OBSERVATION_ONLY

OVERALL = PRE_SESSION_OK
```

Next checks after open:

```text
1. Confirm guards return from SyncingHistory/SyncingGap to LiveReady/ALLOWED
   after fresh regular-session bars arrive.
2. If RI emits a genuine live entry/exit, verify exact request_id tracking and
   action_scoped_only transport.
3. Continue watching gateway reconnect WARN frequency; current reconnects are
   benign because there were no commands in flight.
```

## Completed 2026-05-07 Trade Result Attribution

Collection time: 2026-05-08 08:09-08:20 MSK.

Source:

```text
Redis broker.trades / broker.orders streams across live VPS contours.
Date filter: broker event date = 2026-05-07 MSK.
```

Important attribution note:

```text
Some broker streams are shared by portfolio, not by strategy.

7502MIW broker stream is visible in both sessiongap and RI micro Redis. It can
contain RI and USDRUBF broker activity even when sessiongap itself emitted no
intent.

7502SN6 broker stream is visible in both hybrid and RI shadow Redis. RI shadow
is observation-only and did not emit commands; IMOEXF broker trades in that
stream are hybrid trades.

Existing=true records observed around 02:40-02:50 MSK are broker replay /
historical snapshot records after reconnect and are excluded from new-trade
economics below.
```

Strategy-attributed results:

```text
sessiongap:
  strategy intents: 0
  strategy trades: 0
  note: broker stream contains shared portfolio records, but no sessiongap
        intent/emission was observed for the session.

hybrid IMOEXF:
  component: MR
  entry: 2026-05-07 09:20:07 MSK, sell 1 IMOEXF @ 2623.5
  protective orders: TP buy @ 2620.0, SL buy @ 2651.0
  exit: 2026-05-07 10:11:43 MSK, buy 1 IMOEXF @ 2620.0
  SL cleanup: canceled after TP fill
  gross result: +3.5 points before commission
  broker commissions observed: 1.74 + 1.74 = 3.48
  final broker position: IMOEXF qty=0
  interpretation: logical MR short lifecycle; entry, TP fill, SL cleanup, flat.

alor-USDRUBF:
  component/comment: day_breakout_waitfix -> bo_stop1_long
  entry: 2026-05-07 11:30:04 MSK, buy 1 USDRUBF @ 74.81
  exit: 2026-05-07 12:00:04 MSK, sell 1 USDRUBF @ 74.78
  gross result: -0.03 price points before commission
  broker commissions observed: 3.48 + 3.48 = 6.96
  final broker position: USDRUBF qty=0
  interpretation: regular long breakout stopped by Stop1; flat after exit.

RI author41/42 micro:
  2026-05-07 10:10:03 MSK, buy 1 RTS-6.26 @ 110890.0
  2026-05-07 10:22:09 MSK, sell 1 RTS-6.26 @ 110630.0
  gross result if treated as a pair: -260 points before commission
  broker commissions observed: 11.10 + 11.10 = 22.20
  attribution: operational remediation before the 12:38 MSK service-hardening
               rollout, not a clean post-hardening RI soak trade.
  final broker position: RTS-6.26 qty=0
  interpretation: exclude from post-hardening RI acceptance economics; keep in
                  incident/remediation accounting.

RI shadow:
  live commands: 0
  shadow commands: 0
  note: IMOEXF broker trades visible in RI shadow Redis belong to the shared
        7502SN6 portfolio and are attributed to hybrid, not RI shadow.
```

Order/transport observations:

```text
hybrid IMOEXF:
  broker order comments:
    HYB|sid=hybrid_imoexf|...|o=MR|r=ENTRY
    HYB|sid=hybrid_imoexf|...|o=MR|r=TP
    HYB|sid=hybrid_imoexf|...|o=MR|r=SL
  no runtime command_rejected observed in the 14h pre-session check.

alor-USDRUBF:
  broker order comments:
    USDRUBF|entry|day_breakout_waitfix
    USDRUBF|exit|bo_stop1_long
  no runtime command_rejected observed in the 14h pre-session check.

RI micro:
  service-hardening rollout happened after these remediation fills.
  After the rollout, cmd.orders.7502MIW.ri_author41_42.micro remains 0 and
  cmd.acks.7502MIW.ri_author41_42.micro remains 0 as of the pre-session check.
```

Completed-session status:

```text
HYBRID_IMOEXF_2026_05_07 = CLEAN_MR_TP_EXIT / FLAT
ALOR_USDRUBF_2026_05_07 = CLEAN_BO_STOP_EXIT / FLAT
SESSIONGAP_2026_05_07 = NO_STRATEGY_TRADE
RI_MICRO_2026_05_07 = REMEDIATION_PAIR_BEFORE_SERVICE_HARDENING / FLAT
RI_SHADOW_2026_05_07 = OBSERVATION_ONLY / NO_LIVE_COMMANDS
```

## RI Author41/42 Micro Full-Path Observation

Collection time: 2026-05-08 11:51-12:05 MSK.

Scope:

```text
stack: trading-ri-author41-42-7502miw
runtime image: ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-fcc1dab-ri-service-20260507
gateway image: ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-5430299-protplace-20260428
instrument: RTS-6.26
mode: micro_live
execution_path: action_scoped_only
```

Runtime/gateway health:

```text
strategy-runtime: Up 23h, healthy
alor-gateway:     Up 23h, healthy
Redis memory:     63.47M / 512M, fragmentation 1.16
cmd.orders.7502MIW.ri_author41_42.micro XLEN=4
cmd.acks.7502MIW.ri_author41_42.micro   XLEN=4
```

Live guard:

```text
2026-05-08 09:00:07 MSK: LiveReady / ALLOWED
```

Observed RI MR cycles:

```text
Cycle 1:
  component: author41_mr
  model signal: 2026-05-08 09:00:00 MSK, short
  entry intent: sell 1 RTS-6.26, action=market, request_id=ae2ce75b-...
  entry ack: accepted, broker_order_id=1925039822792044716
  entry fill: sell 1 @ 110290.0, commission=10.91
  model exit: take_author_close, scheduled_exit=09:20 MSK, shadow_pnl_points=+88
  exit intent: buy 1 RTS-6.26, action=market, request_id=dbe9499d-...
  exit ack: accepted, broker_order_id=1925039822792062398
  exit fill: buy 1 @ 110170.0, commission=10.91
  gross result: +120 points before commission
  final broker position after cycle: flat

Cycle 2:
  component: author41_mr
  model signal: 2026-05-08 09:30:00 MSK, short
  entry intent: sell 1 RTS-6.26, action=market, request_id=cde911b0-...
  entry ack: accepted, broker_order_id=1925039822792069695
  entry fill: sell 1 @ 110360.0, commission=10.91
  model exit: take_author_close, scheduled_exit=10:00 MSK, shadow_pnl_points=+158
  exit intent: buy 1 RTS-6.26, action=market, request_id=9a3e0297-...
  exit ack: accepted, broker_order_id=1925039822792099620
  exit fill: buy 1 @ 110200.0, commission=10.91
  gross result: +160 points before commission
  final broker position after cycle: flat
```

Transport/readiness observations:

```text
All four RI micro commands used action-scoped CWS sessions.
Each action-scoped session performed fresh authorize before create:market.
All four CWS responses were httpCode=200 and ack status=accepted.
No command_rejected observed.
No live pending request remained after the cycles.
```

Broker/runtime flat checks:

```text
broker.positions.7502MIW latest RTS-6.26 row:
  qty=0.0, avg_price=0.0, ts_utc=1778224201

runtime.state.ri_author41_42.micro.7502MIW latest:
  phase="flat"
  current_component=null
  current_side=null
  pending_entry_request_id=null
  pending_exit_request_id=null
  last_transition_reason="live_position_flat_confirmed"
  last_trade_id="1925039822792034455"
```

Warnings/anomalies:

```text
Runtime WARN:
  orphan_trade on cycle 1 exit fill
  trade_id=1925039822792033470
  order_id=1925039822792062398
  side=buy, qty=1, price=110170.0

Interpretation:
  This appears to be an event-ordering / observability race: the trade event
  reached runtime before the ack mapping was visible there. Gateway request_map
  did resolve the same order_id/request_id, the command ack was accepted, and
  broker/runtime state later converged to flat.

Gateway WARN:
  overnight ws/cws reconnects at 03:24-03:31 UTC, pending_count=0.

Interpretation:
  Benign reconnects; no command was in flight.
```

Broker order-record caveat:

```text
The gateway broker.orders stream reports order_type=limit and reference prices
for these create:market actions. Runtime execution_confirmed correctly marks
exec_price as the fill and warns that reference_price_from_order_record is not
the execution price. Use broker.trades / execution_confirmed for economics.
```

Status:

```text
RI_MICRO_2026_05_08 = FIRST_CLEAN_POST_HARDENING_FULL_PATH_READ
RI_MICRO_PATH = ACTION_SCOPED_CREATE_MARKET_OK
RI_MICRO_POSITION = FLAT_CONFIRMED_BY_BROKER_AND_RUNTIME
RI_MICRO_ECONOMICS = TWO_MR_SHORT_CYCLES / +280_GROSS_POINTS_BEFORE_COMMISSION
RI_MICRO_WARNINGS = ONE_ORPHAN_TRADE_OBSERVABILITY_RACE / NON_BLOCKING
```

Follow-up:

```text
1. Keep RI at size=1 during the current micro soak.
2. Monitor whether orphan_trade repeats; if repeated, add a service-hardening
   task for earlier request_map/ack correlation or delayed orphan classification.
3. Record the analyst TP/SL verdict below: TP limit remains out of the primary
   RI micro contract; SL bracket remains a separate future safety discussion.
```

Analyst follow-up on TP/SL execution contract:

```text
TP bracket/limit should not be enabled as the primary live micro contract.
Keep RI MR TP as closed-bar `take_author_close` followed by marketable
action-scoped exit for parity.

If operational protection is strengthened later, discuss SL bracket/stop-limit
separately. The research stop condition is already level-touch based, while the
TP limit variant changes the contract and may improve win-rate appearance but
hurt expectancy.
```

Code note:

```text
The older `ri_dual_no_overlap_plateau()` helper still exists with summary
parameters K=0.07 / StopK=0.58, but it is not the active replay/live handoff
path. It is now marked as a legacy local/unit-test helper.
```

## Sessiongap 2026-05-08 Completed Trade Correction

Collection time: 2026-05-09 14:50 MSK.

Reason for correction:

```text
The pre-session 2026-05-08 note correctly said there were no new sessiongap
intents in the overnight check window, but the trading session later produced a
completed sessiongap USDRUBF cycle. Add the completed-session attribution here
so the daily journal does not undercount sessiongap activity.
```

Observed sessiongap cycle:

```text
strategy_id: session_gap_standalone
symbol: USDRUBF
qty: 1
direction: short

entry signal ts: 2026-05-08 12:50:00 MSK
entry intent emitted: 2026-05-08 13:00:01 MSK
entry order: sell 1 USDRUBF @ 74.43
entry ack: accepted, cws_http_code=200
entry fill: sell 1 @ 74.43, commission=3.45

exit signal ts: 2026-05-08 13:10:00 MSK
exit intent emitted: 2026-05-08 13:20:16 MSK
exit order: buy 1 USDRUBF @ 74.32
exit ack: accepted, cws_http_code=200
exit fill: buy 1 @ 74.31, commission=3.45

gross result: +0.12 price points before commission
observed commissions: 3.45 + 3.45 = 6.90
final broker position: USDRUBF qty=0
```

Runtime path:

```text
Flat -> PendingEntry -> InPosition -> PendingExit -> Flat
```

Warnings/anomalies:

```text
Runtime WARN:
  orphan_trade on the exit fill
  trade_id=2023556064640770579
  order_id=2023556064640894922
  side=buy, qty=1, price=74.31

Interpretation:
  The exit command ack was accepted by the gateway and the strategy transitioned
  PendingExit -> Flat. This looks like the same event-ordering / observability
  race seen in other live contours, not a failed exit or uncontrolled position.
```

Corrected status:

```text
SESSIONGAP_2026_05_08 = CLEAN_SHORT_ENTRY_EXIT / FLAT
SESSIONGAP_ECONOMICS_2026_05_08 = +0.12_PRICE_POINTS_BEFORE_COMMISSION
SESSIONGAP_WARNINGS_2026_05_08 = ONE_ORPHAN_TRADE_OBSERVABILITY_RACE / NON_BLOCKING
```
