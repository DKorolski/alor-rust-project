# VPS Live Observations - 2026-05-13

## Morning Health Check

Collection time: 2026-05-13 10:03-10:08 MSK.

Scope:

```text
sessiongap
hybrid IMOEXF riskgate profile
alor-USDRUBF
RI author41/42 micro
```

VPS resources:

```text
VPS uptime: 31 days
load average: 0.42 / 0.33 / 0.33
RAM: 7.7Gi total, 2.0Gi used, 5.8Gi available
swap: 3.9Gi total, 80Mi used
disk /: 79G total, 38G used, 37G available, 51%

Redis memory:
  sessiongap:       134.1MiB / 1GiB
  hybrid IMOEXF:    239.3MiB / 1GiB
  alor-USDRUBF:     153.2MiB / 1GiB
  RI author41/42:    94.5MiB / 768MiB
```

Container status:

```text
sessiongap gateway/runtime/redis: healthy
hybrid gateway/runtime/redis: healthy
alor-USDRUBF gateway/runtime/redis: healthy
RI author41/42 gateway/runtime/redis: healthy
```

2026-05-13 morning log read:

```text
sessiongap:
  runtime ERROR=0 command_rejected=0 intent_emitted=0 orphan_trade=0
  no sessiongap-owned trade today
  shared 7502MIW position stream shows RI RTS position, not sessiongap position

hybrid IMOEXF:
  runtime ERROR=0 command_rejected=0
  current live position: IMOEXF long qty=2
  MR protective TP/SL accepted

alor-USDRUBF:
  runtime ERROR=0 command_rejected=0 intent_emitted=0 orphan_trade=0
  current live position: flat

RI author41/42:
  runtime ERROR=0 command_rejected=0
  current live position: RTS-6.26 long qty=1
  one `intent_dropped_bar_silence` at 09:10 MSK, then entry emitted after a
  fresh eligible bar and accepted by CWS
```

Gateway read:

```text
Gateway WARNs remain the familiar overnight transport reconnect noise:
  eof / TLS close_notify missing / reset without close handshake.

No pending_count_nonzero events were observed in gateway logs.
No command rejects were observed.
```

Current broker positions:

```text
sessiongap portfolio 7502MIW:
  USDRUBF qty=0
  RTS-6.26 qty=1  # shared portfolio position from RI, not sessiongap-owned

hybrid portfolio 7502SN6:
  IMOEXF qty=2, avg_price=2691.0

alor-USDRUBF portfolio 7502T0U:
  USDRUBF qty=0

RI author41/42 portfolio 7502MIW:
  RTS-6.26 qty=1, avg_price=115310.0
```

Current morning status:

```text
SESSIONGAP = HEALTHY / NO_OWN_POSITION / NO_TODAY_INTENTS
HYBRID_IMOEXF = HEALTHY / LIVE_MR_LONG_QTY2 / PROTECTIVE_TP_SL_ACCEPTED
ALOR_USDRUBF = HEALTHY / FLAT / NO_TODAY_INTENTS
RI_MICRO = HEALTHY / LIVE_MR_LONG_QTY1 / ACTION_SCOPED_ENTRY_ACCEPTED

OVERALL = HEALTHY_WITH_TWO_EXPECTED_OPEN_POSITIONS
```

## Current 2026-05-13 Open Positions

### Hybrid IMOEXF

Observed state:

```text
cycle_id: 6a0415b802
component: mean_reversion
side: long
qty: 2
entry fill: 2026-05-13 09:20:02 MSK, buy 2 IMOEXF @ 2691.0
commission: 3.54

protective TP:
  sell 2 @ 2694.0
  broker_order_id=2033126226733838508
  ack accepted, cws_http_code=200

protective SL:
  stop sell 2, trigger=2670.0, price=2669.5
  broker_order_id=120648054
  ack accepted, cws_http_code=200
```

Runtime state:

```text
last_position_qty: 2
current_owner: mean_reversion
current_side: long
tp_order_id: 2033126226733838508
sl_stop_order_id: 120648054
safe_mode_close_only: false
pending_entry_request_id: null
pending_exit_request_id: null
```

Riskgate state:

```text
risk_gate_mr_enabled_current_session: true
risk_gate_rolling_sum_lb120: 189.80000000000015
risk_gate_last_finalized_session_date: 2026-05-12
risk_gate_ledger_rows_count: 191
```

Status:

```text
HYBRID_IMOEXF_2026_05_13_CURRENT = LIVE_MR_LONG_QTY2 / TP_SL_INSTALLED
```

### RI Author41/42 Micro

Observed state:

```text
component: author41_mr
side: long
qty: 1
entry: 2026-05-13 09:20:17 MSK, buy 1 RTS-6.26 @ 115310.0
commission: 11.22
execution_path: action_scoped_only
ack: accepted, cws_http_code=200
```

Runtime state:

```text
phase: live_in_position
current_component: author41_mr
current_side: long
current_cycle_id: author41_mr:20260513091000
current_entry_ts_local: 2026-05-13 09:10:00
pending_entry_request_id: null
pending_exit_request_id: null
```

Observation:

```text
At 09:10 MSK the runtime logged `intent_dropped_bar_silence` because the latest
model bar was still stale relative to the entry decision. At the next eligible
bar the entry was emitted and accepted. This behaved like a conservative guard,
not an execution failure.
```

Status:

```text
RI_MICRO_2026_05_13_CURRENT = LIVE_MR_LONG_QTY1 / ACTION_SCOPED_ENTRY_ACCEPTED
```

## Completed 2026-05-12 Trade Attribution

Collection time: 2026-05-13 10:04-10:08 MSK.

Important attribution note:

```text
7502MIW streams are shared by sessiongap and RI author41/42. RTS-6.26 trades
visible in sessiongap Redis are RI trades, not sessiongap strategy trades.
```

### Sessiongap

Observed result:

```text
strategy_id: session_gap_standalone
own commands on 2026-05-12: none
own trades on 2026-05-12: none
runtime phase: Flat
```

Status:

```text
SESSIONGAP_2026_05_12 = NO_STRATEGY_TRADE / FLAT
```

### Hybrid IMOEXF

Observed cycles:

```text
MR cycle:
  side: short
  qty: 2
  entry: 2026-05-12 10:10:02 MSK, sell 2 @ 2666.0
  protective TP: buy 2 @ 2659.0, accepted
  protective SL: stop buy 2, trigger=2719.0, accepted
  exit: 2026-05-12 10:20:03 MSK, buy 2 @ 2671.5
  TP cleanup: accepted
  SL cleanup: accepted
  gross result per contract: -5.5 points
  gross result at qty=2: -11.0 points

BO cycle:
  side: long
  qty: 2
  entry: 2026-05-12 13:10:12/13:10:15 MSK, buy 2 @ 2686.0
  exit: 2026-05-12 15:00:10 MSK, sell 2 @ 2680.5
  gross result per contract: -5.5 points
  gross result at qty=2: -11.0 points

session gross result:
  -22.0 points before commission

observed commissions:
  MR: 3.52 + 3.52 = 7.04
  BO: 1.76 + 1.76 + 3.52 = 7.04
  total: 14.08

final broker position:
  IMOEXF qty=0
```

Transport/runtime:

```text
all hybrid commands were accepted with cws_http_code=200
command_rejected=0

Runtime WARN:
  orphan_trade on MR exit fill
  orphan_trade on BO exit fill

Interpretation:
  Both are the familiar event-ordering / observability race. The strategy
  completed cleanup and converged to flat.
```

Status:

```text
HYBRID_IMOEXF_2026_05_12 = TWO_QTY2_CYCLES_COMPLETED / FLAT
HYBRID_IMOEXF_ECONOMICS_2026_05_12 = -22.0_GROSS_POINTS_BEFORE_COMMISSION_AT_QTY2
HYBRID_IMOEXF_WARNINGS_2026_05_12 = TWO_ORPHAN_TRADE_OBSERVABILITY_RACES / NON_BLOCKING
```

### Alor-USDRUBF

Observed cycle:

```text
strategy_id: alor_usdrubf_hybrid_v1
component/comment: day_breakout_waitfix -> bo_stop1_long
symbol: USDRUBF
qty: 1
direction: long

entry: 2026-05-12 11:10:09 MSK, buy 1 @ 74.00
exit: 2026-05-12 12:00:24 MSK, sell 1 @ 73.86

gross result: -0.14 price points before commission
observed commissions: 3.43 + 3.43 = 6.86
final broker position: USDRUBF qty=0
```

Transport/runtime:

```text
entry ack: accepted, cws_http_code=200
exit ack: accepted, cws_http_code=200
command_rejected=0
orphan_trade=0
runtime lifecycle converged to flat
```

Status:

```text
ALOR_USDRUBF_2026_05_12 = CLEAN_BO_STOP1_LONG_EXIT / FLAT
ALOR_USDRUBF_ECONOMICS_2026_05_12 = -0.14_PRICE_POINTS_BEFORE_COMMISSION
```

### RI Author41/42 Micro

Observed cycles:

```text
Cycle 1:
  component: author41_mr
  side: long
  entry: 2026-05-12 10:10:01 MSK, buy 1 @ 114050.0
  exit: 2026-05-12 10:40:01 MSK, sell 1 @ 114730.0
  gross result: +680 points

Cycle 2:
  component: author41_mr
  side: short
  entry: 2026-05-12 10:50:01 MSK, sell 1 @ 114580.0
  exit: 2026-05-12 14:20:29 MSK, buy 1 @ 114690.0
  gross result: -110 points

Cycle 3:
  component: author42_bo
  side: long
  entry: 2026-05-12 22:10:28 MSK, buy 1 @ 115350.0
  exit: 2026-05-12 23:10:48 MSK, sell 1 @ 115470.0
  gross result: +120 points

session gross result:
  +690 points before commission

observed commissions:
  6 fills x 11.20 = 67.20

final broker position:
  RTS-6.26 qty=0
```

Transport/runtime:

```text
all RI commands used action_scoped_only / create:market
all command acks accepted with cws_http_code=200
command_rejected=0
orphan_trade=0
runtime phase converged to flat
```

Status:

```text
RI_MICRO_2026_05_12 = THREE_CYCLES_COMPLETED / FLAT
RI_MICRO_PATH_2026_05_12 = ACTION_SCOPED_CREATE_MARKET_OK
RI_MICRO_ECONOMICS_2026_05_12 = +690_GROSS_POINTS_BEFORE_COMMISSION
```

## Follow-Up Watch List

```text
1. Continue observing current 2026-05-13 open hybrid MR long until TP/SL/exit
   and cleanup converge to flat.
2. Continue observing current 2026-05-13 RI MR long until scheduled/model exit.
3. Hybrid Redis memory increased to ~239MiB after recent activity but remains
   well below the 1GiB limit. Keep it on the regular trim watch list.
4. Continue tracking orphan_trade frequency. Current evidence still points to
   event-ordering / observability races, not failed execution.
```

