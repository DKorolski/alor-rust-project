# VPS Live Observations - 2026-05-09

## Weekend Health / Rollout Check

Collection time: 2026-05-09 14:18-14:25 MSK.

Scope:

```text
sessiongap
hybrid IMOEXF riskgate profile
alor-USDRUBF
RI author41/42 micro
```

VPS resources:

```text
VPS uptime: 27 days
RAM: 7.7Gi total, 2.0Gi used, 5.7Gi available
swap: 3.9Gi total, 85Mi used
disk /: 79G total, 38G used, 38G available, 50%

Redis memory:
  sessiongap:       147.79M / 1GiB
  hybrid IMOEXF:    202.90M / 1GiB
  alor-USDRUBF:     155.44M / 1GiB
  RI author41/42:    81.08M / 512MiB
```

Container status:

```text
sessiongap gateway/runtime/redis: healthy
hybrid gateway/runtime/redis: healthy
alor-USDRUBF gateway/runtime/redis: healthy
RI author41/42 gateway/runtime/redis: healthy
```

Broker flat check:

```text
sessiongap portfolio 7502MIW:
  USDRUBF qty=0
  RTS-6.26 qty=0

alor-USDRUBF portfolio 7502T0U:
  USDRUBF qty=0

hybrid portfolio 7502SN6:
  IMOEXF qty=0

RI author41/42 portfolio 7502MIW:
  RTS-6.26 qty=0
  USDRUBF qty=0
```

Runtime/gateway log read:

```text
hybrid IMOEXF:
  no runtime WARN/ERROR since the checked 2026-05-08 post-session window
  latest command/ack records are accepted MR/BO records from 2026-05-08
  no command_rejected observed

RI author41/42:
  no runtime WARN/ERROR in the checked window
  no new position remains open

sessiongap:
  one orphan_trade warning observed for a shared 7502MIW USDRUBF broker event
  no evidence of stuck sessiongap-owned position

alor-USDRUBF:
  one orphan_trade warning observed for a USDRUBF broker event
  broker position is flat after reconciliation

gateways:
  only transport reconnect/reset noise was observed
  reconnects had pending_count=0 where visible
```

Interpretation:

```text
The systems are operationally healthy and flat. Redis memory increased but is
still below configured limits after prior trimming work. Current resource state
does not suggest VPS pressure.

The orphan_trade warnings in sessiongap/alor-USDRUBF should remain on the watch
list as observability/reconciliation races, but they are not blocking this
weekend rollout because broker positions are flat and no live pending state is
visible in the target hybrid stack.
```

## Completed 2026-05-08 Hybrid IMOEXF Observation

Source:

```text
hybrid Redis cmd.orders/cmd.acks/broker.positions and runtime logs.
```

Observed completed cycles:

```text
MR cycle:
  entry: buy 1 IMOEXF @ 2609.5
  protective TP: sell 1 @ 2610.5
  protective SL: accepted, then canceled after TP fill
  gross result: +1.0 point before commission
  final position: flat

BO cycle:
  entry command: HYB|sid=hybrid_imoexf|c=69fe295007|o=BO|r=ENTRY
  entry: buy 1 IMOEXF @ 2626.5
  exit command: HYB|sid=hybrid_imoexf|c=69fe295007|o=BO|r=EXIT
  exit: sell 1 IMOEXF @ 2629.0
  gross result: +2.5 points before commission
  final position: flat
```

Transport notes:

```text
MR TP/SL protective path remained accepted.
BO entry/exit commands were accepted.
No hybrid command_rejected records were observed for the checked window.
```

Status:

```text
HYBRID_IMOEXF_2026_05_08 = CLEAN_MR_AND_BO_LIFECYCLE / FLAT
HYBRID_IMOEXF_ECONOMICS = +3.5_GROSS_POINTS_BEFORE_COMMISSION_AT_QTY1
```

## Hybrid IMOEXF Qty2 Rollout

Decision:

```text
Increase hybrid IMOEXF from qty=1 to qty=2 for a controlled observation period.
Keep the next decision gate at at least 5 clean sessions before considering any
further size increase.
```

Pre-rollout safety checks:

```text
broker IMOEXF position: qty=0
runtime last_position_qty: 0
pending_entry_request_id: null
pending_exit_request_id: null
pending_tp_order_request_id: null
pending_sl_order_request_id: null
tp_order_id: null
sl_stop_order_id: null
safe_mode_close_only: false
```

Riskgate state before restart:

```text
ledger key: runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120
last_finalized_session_date: 2026-05-07
rolling_sum_lb120: 218.70000000000022
ledger_rows_count: 189
mr_enabled_current_session: true
mr_enabled_next_session: true
```

Config change:

```text
file: configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml
old qty: 1.0
new qty: 2.0
```

Remote rollout steps:

```text
1. Backed up remote config:
   /opt/trading-hybrid/configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml.bak-20260509-qty1
2. Stopped only trading-hybrid strategy-runtime.
3. Copied updated qty=2 config to VPS.
4. Cleared hybrid runtime state stream.
5. Destroyed and recreated the hybrid runtime consumer group.
6. Preserved riskgate ledger/state.
7. Recreated strategy-runtime container.
```

Post-rollout verification:

```text
resolved runtime config:
  strategy_kind: HybridIntraday
  strategy_id: hybrid_imoexf
  symbol: IMOEXF
  qty: 2.0
  profile: imoexf_primary_riskgate_high180_lb120
  mr_gate_policy: shadow_pnl_lb120_positive
  risk_gate_mode: normal_append

riskgate startup:
  decision: UseExistingLedger
  existing_records_loaded: 189
  state_refreshed: true
  seed was not re-imported over the existing ledger

runtime state after restart:
  next_cycle_seq: 0
  last_position_qty: 0
  all pending request ids: null
  tp_order_id: null
  sl_stop_order_id: null
  safe_mode_close_only: false
```

Current status:

```text
HYBRID_IMOEXF_QTY2_ROLLOUT = DONE
HYBRID_IMOEXF_FROM_ZERO_RUNTIME = DONE
HYBRID_IMOEXF_RISKGATE_LEDGER = PRESERVED_AND_CURRENT
HYBRID_IMOEXF_POSITION = FLAT
HYBRID_IMOEXF_NEXT_PHASE = OBSERVE_5_CLEAN_SESSIONS_AT_QTY2
```

Weekend note:

```text
After restart the runtime may remain BLOCKED while waiting for a fresh eligible
regular-session bar / SyncingHistory completion. This is expected on a weekend
or outside tradable model time and is not a rollout failure.
```

