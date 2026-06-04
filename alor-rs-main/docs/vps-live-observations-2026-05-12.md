# VPS Live Observations - 2026-05-12

## Pre-Open Health Check

Collection time: 2026-05-12 08:18-08:20 MSK.

Scope:

```text
sessiongap
hybrid IMOEXF riskgate profile
alor-USDRUBF
RI author41/42 micro
```

VPS resources:

```text
VPS uptime: 30 days
load average: 0.14 / 0.26 / 0.26
RAM: 7.7Gi total, 1.8Gi used, 5.9Gi available
swap: 3.9Gi total, 81Mi used
disk /: 79G total, 38G used, 38G available, 51%

Redis memory:
  sessiongap:       133.0MiB / 1GiB
  hybrid IMOEXF:    139.0MiB / 1GiB
  alor-USDRUBF:     142.6MiB / 1GiB
  RI author41/42:    72.0MiB / 768MiB
```

Container status:

```text
sessiongap gateway/runtime/redis: healthy
hybrid gateway/runtime/redis: healthy
alor-USDRUBF gateway/runtime/redis: healthy
RI author41/42 gateway/runtime/redis: healthy
```

2026-05-12 pre-open log counters:

```text
sessiongap:
  runtime ERROR=0 WARN=0 command_rejected=0 intent_emitted=0 orphan_trade=0
  gateway WARN=9, command_received=0, pending_count_nonzero=0

hybrid IMOEXF:
  runtime ERROR=0 WARN=0 command_rejected=0 intent_emitted=0 orphan_trade=0
  gateway WARN=11, command_received=0, pending_count_nonzero=0

alor-USDRUBF:
  runtime ERROR=0 WARN=0 command_rejected=0 intent_emitted=0 orphan_trade=0
  gateway WARN observed only as reconnect noise, pending_count_nonzero=0

RI author41/42:
  runtime ERROR=0 WARN=0 command_rejected=0 intent_emitted=0 orphan_trade=0
  gateway WARN=7, command_received=0, pending_count_nonzero=0
```

Broker flat check:

```text
sessiongap portfolio 7502MIW:
  USDRUBF qty=0
  RTS-6.26 qty=0

hybrid portfolio 7502SN6:
  IMOEXF qty=0

alor-USDRUBF portfolio 7502T0U:
  USDRUBF qty=0

RI author41/42 portfolio 7502MIW:
  RTS-6.26 qty=0
  USDRUBF qty=0
```

Interpretation:

```text
The VPS resource state is healthy. All strategy containers are healthy and all
checked broker positions are flat before the 2026-05-12 regular session.

Gateway WARNs are the familiar overnight transport reconnects:
  eof / TLS close_notify missing / reset without close handshake.

No commands were in flight during those reconnects:
  command_received=0
  pending_count_nonzero=0
```

Current pre-open status:

```text
SESSIONGAP = HEALTHY / FLAT / NO_TODAY_INTENTS
HYBRID_IMOEXF = HEALTHY / FLAT / QTY2_READY / NO_TODAY_INTENTS
ALOR_USDRUBF = HEALTHY / FLAT / NO_TODAY_INTENTS
RI_MICRO = HEALTHY / FLAT / NO_TODAY_INTENTS

OVERALL = PRE_OPEN_OK
```

## Completed 2026-05-11 Trade Attribution

Collection time: 2026-05-12 08:20-08:25 MSK.

Important attribution note:

```text
7502MIW streams are shared by sessiongap and RI author41/42. RTS-6.26 trades in
the sessiongap Redis view are RI trades, not sessiongap strategy trades.
```

### Sessiongap

Observed cycle:

```text
strategy_id: session_gap_standalone
symbol: USDRUBF
qty: 1
direction: short

entry command: sell 1 @ 73.64
entry fill: 2026-05-11 16:00:33 MSK, sell 1 @ 73.64, commission=3.43
exit command: buy 1 @ 73.74
exit fill: 2026-05-11 16:10:02 MSK, buy 1 @ 73.71, commission=3.43

gross result: -0.07 price points before commission
observed commissions: 3.43 + 3.43 = 6.86
final broker position: USDRUBF qty=0
runtime phase: Flat
```

Transport/runtime:

```text
entry ack: accepted, cws_http_code=200
exit ack: accepted, cws_http_code=200
runtime execution_confirmed for entry and exit
orphan_trade=0 for this sessiongap cycle
```

Status:

```text
SESSIONGAP_2026_05_11 = CLEAN_SHORT_ENTRY_EXIT / FLAT
SESSIONGAP_ECONOMICS_2026_05_11 = -0.07_PRICE_POINTS_BEFORE_COMMISSION
```

### Hybrid IMOEXF

Observed cycle:

```text
strategy_id: hybrid_imoexf
profile: imoexf_primary_riskgate_high180_lb120
cycle_id: 6a019a9000
component: BO
qty: 2
direction: long

entry command: HYB|sid=hybrid_imoexf|c=6a019a9000|o=BO|r=ENTRY
entry fill: 2026-05-11 12:10:16 MSK, buy 2 @ 2655.0, commission=3.44

exit command: HYB|sid=hybrid_imoexf|c=6a019a9000|o=BO|r=EXIT
exit fill: 2026-05-11 23:40:11 MSK, sell 2 @ 2663.0, commission=3.44

gross result per contract: +8.0 points before commission
gross result at qty=2: +16.0 points before commission
observed commissions: 3.44 + 3.44 = 6.88
final broker position: IMOEXF qty=0
runtime last_position_qty: 0
```

Transport/runtime:

```text
entry ack: accepted, cws_http_code=200
exit ack: accepted, cws_http_code=200
runtime command_rejected=0

Runtime WARN:
  orphan_trade on the BO exit fill
  trade_id=2033126218143910011
  order_id=2033126218144121186
  side=sell, qty=2, price=2663.0

Interpretation:
  This is the familiar event-ordering / observability race. The exit ack was
  accepted and broker/runtime state converged to flat.
```

Riskgate state after the session:

```text
risk_gate_mr_enabled_current_session: true
risk_gate_rolling_sum_lb120: 218.70000000000022
risk_gate_last_finalized_session_date: 2026-05-07
risk_gate_ledger_rows_count: 189
```

Status:

```text
HYBRID_IMOEXF_2026_05_11 = FIRST_QTY2_BO_CYCLE_CLEAN_FLAT
HYBRID_IMOEXF_ECONOMICS_2026_05_11 = +16.0_GROSS_POINTS_BEFORE_COMMISSION_AT_QTY2
HYBRID_IMOEXF_WARNINGS_2026_05_11 = ONE_ORPHAN_TRADE_OBSERVABILITY_RACE / NON_BLOCKING
```

### Alor-USDRUBF

Observed cycle:

```text
strategy_id: alor_usdrubf_hybrid_v1
component/comment: day_breakout_waitfix -> bo_stop1_short
symbol: USDRUBF
qty: 1
direction: short

entry fill: 2026-05-11 11:20:04 MSK, sell 1 @ 73.93, commission=3.43
exit fill: 2026-05-11 12:00:02 MSK, buy 1 @ 74.00, commission=3.43

gross result: -0.07 price points before commission
observed commissions: 3.43 + 3.43 = 6.86
final broker position: USDRUBF qty=0
runtime lifecycle_stage: broker_position_flat
```

Transport/runtime:

```text
entry ack: accepted, cws_http_code=200
exit ack: accepted, cws_http_code=200
runtime command_rejected=0

Runtime WARN:
  orphan_trade on the exit fill
  trade_id=2023556068935737967
  order_id=2023556068935819886
  side=buy, qty=1, price=74.00

Interpretation:
  Non-blocking event-ordering / observability race. The strategy saw
  open_to_flat and persisted broker_position_flat state.
```

Status:

```text
ALOR_USDRUBF_2026_05_11 = CLEAN_BO_STOP1_SHORT_EXIT / FLAT
ALOR_USDRUBF_ECONOMICS_2026_05_11 = -0.07_PRICE_POINTS_BEFORE_COMMISSION
ALOR_USDRUBF_WARNINGS_2026_05_11 = ONE_ORPHAN_TRADE_OBSERVABILITY_RACE / NON_BLOCKING
```

### RI Author41/42 Micro

Observed cycles:

```text
Cycle 1:
  component: author41_mr
  direction: short
  entry: 2026-05-11 09:30:01 MSK, sell 1 RTS-6.26 @ 112590.0
  exit: 2026-05-11 13:00:04 MSK, buy 1 RTS-6.26 @ 113940.0
  gross result: -1350 points before commission

Cycle 2:
  component: author42_bo
  direction: long
  entry: 2026-05-11 15:10:15 MSK, buy 1 RTS-6.26 @ 113820.0
  exit: 2026-05-11 23:11:07 MSK, sell 1 RTS-6.26 @ 114280.0
  gross result: +460 points before commission

observed commissions:
  4 fills x 10.83 = 43.32

session gross result:
  -890 points before commission

final broker position:
  RTS-6.26 qty=0
runtime phase:
  flat
```

Transport/runtime:

```text
all four RI commands used action_scoped_only / create:market
all four command acks accepted with cws_http_code=200
runtime command_rejected=0

Runtime WARN:
  orphan_trade on the MR entry fill
  trade_id=1925039827087001978
  order_id=1925039827087043296
  side=sell, qty=1, price=112590.0

Interpretation:
  The gateway restored request_id through request_map and all later lifecycle
  transitions completed. The warning remains a non-blocking observability race.
```

Status:

```text
RI_MICRO_2026_05_11 = TWO_CYCLES_COMPLETED / FLAT
RI_MICRO_PATH_2026_05_11 = ACTION_SCOPED_CREATE_MARKET_OK
RI_MICRO_ECONOMICS_2026_05_11 = -890_GROSS_POINTS_BEFORE_COMMISSION
RI_MICRO_WARNINGS_2026_05_11 = ONE_ORPHAN_TRADE_OBSERVABILITY_RACE / NON_BLOCKING
```

## Follow-Up Watch List

```text
1. Continue observing hybrid IMOEXF at qty=2 for at least 5 clean sessions
   before considering the next size step.
2. Continue tracking orphan_trade frequency across RI/hybrid/alor-USDRUBF.
   Current evidence points to event-ordering / observability, not failed exits.
3. No resource cleanup is required from this check. Redis memory and disk usage
   are comfortably within the configured limits.
```

