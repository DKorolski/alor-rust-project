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
