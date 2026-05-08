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
