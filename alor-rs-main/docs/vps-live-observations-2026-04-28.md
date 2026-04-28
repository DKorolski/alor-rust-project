# VPS Live Observations (2026-04-28)

## Scope

Stacks:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

Window:

- VPS check time: `2026-04-28 08:54 MSK`
- log window: last `24h`
- includes the `2026-04-27` regular session and the `2026-04-28` pre-open state

## Executive Summary

All three live stacks were healthy at the 2026-04-28 pre-open check.

No runtime `WARN` / `ERROR` / `rejected` / `orphan` / `safe_mode` events were
seen in the 24h window.

The main trading event in the window was the `trading-alor-usdrubf` BO short
round-trip on 2026-04-27:

```text
11:10 MSK: BO short signal accepted into pending entry
11:20 MSK: entry market intent emitted and accepted
11:20 MSK: sell fill, qty = 1, price = 74.99
13:00 MSK: bo_stop1_short exit intent emitted and accepted
13:00 MSK: buy fill, qty = 1, price = 75.06
post-exit: broker_position_flat, runtime flat
```

No active strategy positions or working orders were present at the check time.

## Active Configs

```text
sessiongap:
  GATEWAY_CONFIG=/configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml
  RUNTIME_CONFIG=/configs/runtime.sessiongap.live.7502MIW.toml

hybrid IMOEXF:
  GATEWAY_CONFIG=/configs/gateway.hybrid.live.7502SN6.action-scoped.toml
  RUNTIME_CONFIG=/configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml

alor-USDRUBF:
  GATEWAY_CONFIG=/configs/gateway.alor_usdrubf.live.7502T0U.toml
  RUNTIME_CONFIG=/configs/runtime.alor_usdrubf.live.7502T0U.challenger_mr035.toml
```

## Stack Status

All containers were healthy:

```text
trading-sessiongap-alor-gateway-1       healthy
trading-sessiongap-redis-1              healthy
trading-sessiongap-strategy-runtime-1   healthy

trading-hybrid-alor-gateway-1           healthy
trading-hybrid-redis-1                  healthy
trading-hybrid-strategy-runtime-1       healthy

trading-alor-usdrubf-alor-gateway-1     healthy
trading-alor-usdrubf-redis-1            healthy
trading-alor-usdrubf-strategy-runtime-1 healthy
```

## SessionGap (`7502MIW`, USDRUBF)

Runtime state remained flat:

```text
phase = Flat
traded_session = false
last_trade_ts = null
seen_trade_ids = []
```

Broker snapshot:

```text
USDRUBF qty = 0.0
orders = {}
stop_orders = {}
```

Streams:

```text
cmd.orders.7502MIW = 0
cmd.acks.7502MIW = 0
broker.orders.7502MIW = 0
broker.trades.7502MIW = 0
runtime.state.session_gap_standalone.live.7502MIW = 447
md.bars.7502MIW.10m = 539
```

24h runtime warning count:

```text
runtime_warn_count = 0
intent_count = 0
accepted_ack_count = 0
execution_count = 0
```

Gateway warning count:

```text
gateway_warn_count = 13
```

These were reconnect-class events (`protocol_reset_without_close_handshake`,
`unexpected_eof`) with no pending commands and no strategy impact observed.

## Hybrid IMOEXF (`7502SN6`, riskgate-shadow)

Runtime state remained flat:

```text
last_position_qty = 0.0
current_owner = null
current_side = null
pending_* = null
deferred_* = null
safe_mode_close_only = false
entry_ready = true
```

Risk-gate state:

```text
risk_gate_shadow_session_date = 2026-04-27
risk_gate_shadow_pnl_points = 0.0
risk_gate_shadow_trade_count = 0
risk_gate_mr_enabled_current_session = true
risk_gate_rolling_sum_lb120 = 161.90000000000012
risk_gate_ledger_rows_count = 180
```

Broker snapshot:

```text
no IMOEXF position
orders = {}
stop_orders = {}
```

Streams:

```text
cmd.orders.7502SN6 = 0
cmd.acks.7502SN6 = 0
broker.orders.7502SN6 = 0
broker.trades.7502SN6 = 0
runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6 = 92
md.bars.7502SN6.10m = 1276
```

24h runtime warning count:

```text
runtime_warn_count = 0
intent_count = 0
accepted_ack_count = 0
execution_count = 0
```

Gateway warning count:

```text
gateway_warn_count = 15
```

These were reconnect-class events without pending commands and without strategy
state impact.

## Alor-USDRUBF (`7502T0U`, challenger `mr_k_short=0.035`)

The stack completed one BO short round-trip on 2026-04-27.

Entry:

```text
2026-04-27 11:10 MSK:
  action = signal_generated
  owner = day_breakout_waitfix
  side = short
  reason = bo_short_signal
  signal_price = 75.02
  scale_at_signal = 0.37000000000000455

2026-04-27 11:20 MSK:
  action = intent_emitted
  intent_class = entry
  side = Sell
  qty = 1
  request_id = ffd07c59-89ca-5f7e-966a-b56953284bf5

2026-04-27 11:20 MSK:
  command acknowledged, status = Accepted
  broker_order_id = 2023556030281134256
  execution_confirmed, exec_price = 74.99
  broker position transition = initial_broker_sync_open
```

Exit:

```text
2026-04-27 13:00 MSK:
  action = intent_emitted
  intent_class = exit
  side = Buy
  qty = 1
  exit_reason = bo_stop1_short
  reference_price_from_signal = 75.06
  request_id = 562050bb-49fe-5259-9e75-47718361a23f

2026-04-27 13:00 MSK:
  command acknowledged, status = Accepted
  broker_order_id = 2023556030281199737
  execution_confirmed, exec_price = 75.06
  broker position transition = open_to_flat
```

Latest runtime state:

```text
lifecycle_stage = broker_position_flat
hybrid_state = flat
open_position_qty = 0.0
pending_entry_owner = null
pending_request_ids = []
tracked_order_ids = []
entry_intent_inflight = false
exit_intent_inflight = false
seen_trade_ids = [
  2023556030281031210,
  2023556030281035109
]
```

Latest broker snapshot:

```text
USDRUBF qty = 0.0
orders = filled historical entry/exit only
stop_orders = {}
```

Streams:

```text
cmd.orders.7502T0U = 2
cmd.acks.7502T0U = 2
broker.orders.7502T0U = 4
broker.trades.7502T0U = 2
runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U = 327
md.bars.7502T0U.10m = 718
```

24h runtime counts:

```text
runtime_warn_count = 0
intent_count = 8
accepted_ack_count = 2
execution_count = 2
position_transition_count = 2
```

`intent_count` is higher than two because the log filter also matched strategy
state transitions and intent-related runtime lines. The actual broker-side
command lifecycle was two intents: one entry and one exit.

Gateway warning count:

```text
gateway_warn_count = 13
```

These were reconnect-class events. No `command rejected`, `orphan_trade`,
`safe_mode`, or stale pending state was observed.

## Resource Snapshot

```text
load average = 0.58 / 0.26 / 0.20
RAM available = 5.0 GiB
swap used = 28 MiB
disk = 38G used / 79G total / 37G free / 51%
```

Container memory:

```text
sessiongap redis      530.3 MiB / 1 GiB
hybrid redis          550.8 MiB / 1 GiB
alor-usdrubf redis    599.0 MiB / 1 GiB
```

Redis memory remains below limit, but `alor-usdrubf` is the largest of the
three and should continue to be watched during the reduced-retention soak.

## Verdict

The 2026-04-27 patched/parameter-updated live session was operationally clean:

- `sessiongap` stayed flat with no trading activity.
- `hybrid IMOEXF` stayed flat with risk-gate shadow state loaded and no stale
  pending/deferred state.
- `alor-USDRUBF` produced one BO short round-trip and returned to broker-flat
  with clean acknowledgements and fills.

The only recurring noise was external reconnect-class gateway WARN traffic. It
did not coincide with live command failures in this observation window.

The next observation point is the first fresh `10m` live bar on 2026-04-28.
