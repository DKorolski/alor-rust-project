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

The 2026-04-27 patched/parameter-updated live session and the 2026-04-28
pre-open check were operationally clean:

- `sessiongap` stayed flat with no trading activity.
- `hybrid IMOEXF` stayed flat at pre-open with risk-gate shadow state loaded
  and no stale pending/deferred state.
- `alor-USDRUBF` produced one BO short round-trip and returned to broker-flat
  with clean acknowledgements and fills.

The only recurring noise was external reconnect-class gateway WARN traffic. It
did not coincide with live command failures in this observation window.

The next observation point is the first fresh `10m` live bar on 2026-04-28.

## Intraday Incident: Hybrid IMOEXF MR Protective Double Exit

Status:

- incident date: `2026-04-28`
- stack: `trading-hybrid`
- portfolio: `7502SN6`
- symbol: `IMOEXF`
- cycle: `69f04ce000`
- strategy owner: `MR`
- runtime config: `runtime.hybrid.live.7502SN6.riskgate-shadow.toml`
- runtime action taken: `strategy-runtime` stopped
- broker action taken: manual flatten sell `1` IMOEXF
- final broker position: `IMOEXF qty = 0.0`

### What Happened

The MR branch entered short and attempted to install protective exits:

```text
ENTRY:
  request_id = 11ab20d7-161e-5c95-8ba3-4dd8ba6577b0
  action = place
  side = sell
  price = 2732.0
  broker_order_id = 2033126183784160153
  trade = sell 1 @ 2732.5

TP:
  request_id = c9cb04fd-3d99-5e7c-b177-990e4ea7cebf
  action = place
  side = buy
  price = 2732.25
  status = error
  error_code = cws_error
  error_msg = cws disconnected: protocol_reset_without_close_handshake

SL:
  request_id = 054c03f1-476a-5e80-ac87-5e001c02e76a
  action = create_stop_limit
  side = buy
  trigger_price = 2734.25
  price = 2734.75
  stop_order_id = 119698842
  generated exchange_order_id = 2033126183784162963
  trade = buy 1 @ 2734.0
```

The SL-generated exchange order was reported in broker streams with
`request_id = null`, while retaining the strategy comment:

```text
HYB|sid=hybrid_imoexf|c=69f04ce000|o=MR|r=SL
```

Runtime classified the SL fill as `orphan_trade` and did not retire the MR
cycle before emitting a normal exit:

```text
EXIT:
  request_id = 70c77fb0-ffc0-5c17-9d1c-661a1237aac2
  action = place
  side = buy
  price = 2733.5
  broker_order_id = 2033126183784162978
  trade = buy 1 @ 2733.5
```

Net broker flow:

```text
sell 1 @ 2732.5   # MR entry
buy  1 @ 2734.0   # SL protective fill
buy  1 @ 2733.5   # extra runtime exit
```

This left the broker long `IMOEXF +1` while runtime moved into
`safe_mode_close_only` with `recovered_position_owner_unknown`.

### Manual Flatten

The first manual flatten attempt used a malformed Redis payload and was ignored
by gateway:

```text
cmd.orders.7502SN6 payload = schema_version:1
```

The corrected manual flatten was sent as a normal aggressive sell limit:

```text
request_id = 4a3ebe82-9fa6-45a2-9892-60853686a493
action = place
side = sell
price = 2700.0
comment = MANUAL|flatten|hybrid_incident_20260428
ack = accepted
broker_order_id = 2033126183784165562
trade = sell 1 @ 2733.5
```

Final broker snapshot:

```text
IMOEXF qty = 0.0
avg_price = 0.0
```

The `trading-hybrid-strategy-runtime-1` container remains stopped. It should not
be restarted on the old runtime state.

### Classification

This incident is not signal logic drift and not a market-order behavior issue.
The observed strategy/broker actions were limit/protective-limit paths:

- entry was a normal limit sell;
- TP was a normal limit buy attempt and failed on `cws_error`;
- SL was a stop-limit buy, accepted and filled;
- the extra exit was a normal limit buy;
- manual flatten was an aggressive limit sell.

Root cause class:

```text
protective_stop_fill_lineage_gap + double_exit_race
```

Action-scoped routing check:

```text
ENTRY Place + Entry:
  action-scoped create:limit

TP Place + ProtectiveRepair:
  legacy long-lived create:limit
  failure = protocol_reset_without_close_handshake

SL CreateStopLimit + ProtectiveRepair:
  action-scoped create:stopLimit

EXIT Place + Exit:
  action-scoped create:limit

CLEANUP Cancel:
  action-scoped delete:limit

MANUAL FLATTEN Place + Exit:
  action-scoped create:limit
```

So the stack did not fully regress to the legacy CWS path. The regression gap
was narrower and more specific: protective TP was encoded as a `Place` command
with `IntentClass::ProtectiveRepair`, while gateway action-scoped routing only
covered `Place + Entry`, `Place + Exit`, `CreateStopLimit + ProtectiveRepair`,
and cleanup actions.

The gateway/broker provided enough lineage through `stop_order_id`,
`exchange_order_id`, and the strategy comment, but runtime did not reconcile the
SL-generated fill as a valid protective exit for cycle `69f04ce000` before
allowing the additional normal exit.

### Required Patch Direction

Patch before restart:

- Route `Place + IntentClass::ProtectiveRepair` through the action-scoped
  `create:limit` path when `action_scope_enable_create_limit = true`.
- Treat filled stop-limit exchange orders linked by `stop_order_id`,
  `exchange_order_id`, or `HYB|...|c=<cycle>|...|r=SL/TP` comment as valid
  protective fills, not generic orphan trades.
- On a valid protective TP/SL fill, atomically retire the active owner/cycle,
  clear pending protective and pending exit state, and suppress further normal
  exits for that cycle.
- Make protective cleanup idempotent: `Order to cancel not found` is benign when
  broker state already shows the target order filled or the cycle flat.
- Add a regression test for: TP install fails on `cws_error`, SL stop-limit
  fills with missing `request_id`, runtime receives an exit signal, and no
  second buy is emitted.
- Restart the hybrid target stack only from a clean broker-flat state and a
  from-zero runtime state after the patch.

### Hotfix Rollout

Applied hotfix:

```text
alor-gateway/src/services/command_consumer.rs:
  Place + IntentClass::ProtectiveRepair -> ActionScoped
  when action_scope_enable_create_limit = true

test:
  cargo test -p alor-gateway \
    execution_path_respects_phase_flags_for_entry_exit_and_cancel
```

Local test result:

```text
PASS
```

VPS rollout:

```text
stack = trading-hybrid
previous gateway image = manual-5430299
new gateway image = manual-5430299-protplace-20260428
runtime image = manual-2d1803e-riskgate
gateway config = /configs/gateway.hybrid.live.7502SN6.action-scoped.toml
runtime config = /configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml
```

Build note:

```text
The patched gateway image was built locally on the VPS.
GHCR push was attempted but rejected due to token scope:
permission_denied: The token provided does not match expected scopes.
The VPS rollout uses the locally available Docker image tag.
```

From-zero runtime restart:

```text
deleted runtime state stream:
  runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6

reset consumer group to latest stream id:
  strategy-runtime-hybrid-riskgate-shadow-7502SN6

streams reset:
  md.bars.7502SN6.10m
  broker.orders.7502SN6
  broker.trades.7502SN6
  broker.positions.7502SN6
  cmd.acks.7502SN6
  cmd.orders.7502SN6
```

Risk-gate ledger was intentionally preserved:

```text
risk_gate_mode = NormalAppend
decision = UseExistingLedger
ledger_rows_count = 181
last_finalized_session_date = 2026-04-27
rolling_sum_lb120 = 154.5000000000001
mr_enabled_current_session = true
mr_enabled_next_session = true
```

Post-rollout status:

```text
trading-hybrid-alor-gateway-1       healthy
trading-hybrid-strategy-runtime-1   healthy
broker positions                    {}
broker stop_orders                  {}
live_guard                          BLOCKED waiting_for_next_bar_after_restart
```

The `waiting_for_next_bar_after_restart` block is expected after a clean
consumer-tail reset. Trading should become eligible only after the next fresh
`10m` live bar is consumed.

### Follow-up: MR Exit Before BO Window

Later in the same session, the patched hybrid stack entered a new MR long:

```text
dt_local = 2026-04-28 10:20:00
owner = MeanReversion
side = Long
entry = buy 1 IMOEXF
tp = sell limit @ 2733.5
sl = sell stop-limit @ 2701.5
cycle = 69f05fa000
```

At `11:57 MSK`, runtime state still showed the MR long open:

```text
current_owner = mean_reversion
current_side = long
last_position_qty = 1.0
tp_order_id = 2033126183784193232
sl_stop_order_id = 119710160
last_processed_bar_ts = 2026-04-28 11:40:00 MSK
```

The configured MR cutoff was:

```text
mr_session_end_time = 11:59:00
mr_exit_offset_min = 5
```

On a `10m` feed, this means the effective cutoff is `11:54`, so the first bar
that can satisfy `dt >= 11:54` is the `12:00` model bar. That is too late for
the intended engineering contract: MR must be flat before BO becomes eligible at
`12:00 MSK`.

Manual intervention:

```text
runtime stopped
TP limit canceled:
  order_id = 2033126183784193232
  request_id = 6b687dec-e6ca-4625-a82c-5f105a7b6daf
  ack = accepted

SL stop-limit deleted:
  stop_order_id = 119710160
  request_id = 6baeaae0-d14b-4724-8866-a5b8b5ddaf64
  ack = accepted

manual flatten sell:
  request_id = d2b278e3-7a55-4290-b78f-e9091ffccf03
  order_id = 2033126183784245382
  comment = MANUAL|flatten|hybrid_mr_before_bo_20260428
  ack = accepted
  fill = sell 1
```

Broker state after manual intervention:

```text
IMOEXF qty = 0.0
TP status = canceled
SL status = canceled
```

Config patch:

```text
mr_session_end_time = 11:59:00
mr_exit_offset_min = 10
```

With a `10m` model feed, this makes the effective MR cutoff `11:49`, so the
exit should be emitted on the `11:50` model bar, before BO becomes eligible at
`12:00`.

Restart:

```text
runtime state stream deleted:
  runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6

consumer group reset to latest stream ids:
  strategy-runtime-hybrid-riskgate-shadow-7502SN6

risk-gate ledger preserved:
  decision = UseExistingLedger
  ledger_rows_count = 181
  last_finalized_session_date = 2026-04-27
  rolling_sum_lb120 = 154.5000000000001
```

Post-restart status:

```text
runtime config loaded with mr_exit_offset_min = 10
broker flat before runtime start
runtime healthy
live_guard = BLOCKED waiting_for_next_bar_after_restart
```

This is an intended from-zero clean restart after manual flatten. Runtime should
only become live-eligible after the next fresh `10m` bar.

## Evening Checkpoint: All Systems + RI Shadow

Observation time:

```text
2026-04-28 18:35 MSK
host = nektodk.ispvds.com
```

### VPS Resources

The VPS remained healthy after the RI shadow rollout and the three live stacks
continued running:

```text
load average = 0.24, 0.33, 0.43
RAM          = 7.7Gi total, 2.9Gi used, 4.8Gi available
swap         = 52Mi / 3.9Gi
disk /       = 42G / 79G, 56% used
```

Container memory:

```text
sessiongap redis      = 579.7MiB / 1GiB
alor-usdrubf redis    = 646.0MiB / 1GiB
hybrid redis          = 605.1MiB / 1GiB
RI shadow redis       = 8.7MiB / 768MiB
RI shadow gateway     = 3.4MiB
RI shadow runner      = 1.3MiB
```

No resource-pressure signature was observed.

### Shared Transport Event

All three live gateways saw the same transport reset cluster around
`2026-04-28 12:20 MSK`:

```text
sessiongap    cws/ws reset without close handshake, pending_count=0
hybrid        cws/ws reset without close handshake, pending_count=0
alor-usdrubf  cws/ws reset without close handshake, pending_count=0
```

Recovery was successful:

```text
sessiongap    live_guard ALLOWED again at 12:30:07 MSK
hybrid        live_guard ALLOWED again at 12:30:08 MSK
alor-usdrubf  live_guard ALLOWED again at 12:30:03 MSK
```

Interpretation: this was a shared Alor/WebSocket transport reset, not a local
VPS resource issue. There were no pending CWS commands at the reset moment in
the checked logs.

### SessionGap USDRUBF (`7502MIW`)

SessionGap completed a clean one-shot lifecycle:

```text
13:00:01 MSK  entry emitted, buy 1 USDRUBF
13:00:02 MSK  entry fill @ 75.05
13:10:02 MSK  exit emitted, sell 1 USDRUBF
13:10:03 MSK  exit fill @ 75.08
```

Broker snapshot after the cycle:

```text
USDRUBF qty = 0.0
latest exit order = filled sell 1
```

Verdict: штатно. Action-scoped `create:limit` path was used and accepted.

### Hybrid IMOEXF (`7502SN6`, riskgate-shadow)

Risk-gate startup used the existing ledger, not seed reimport:

```text
decision = UseExistingLedger
ledger_rows_count = 181
last_finalized_session_date = 2026-04-27
rolling_sum_lb120 = 154.5000000000001
mr_enabled_current_session = true
mr_enabled_next_session = true
```

After the manual MR cleanup and from-zero restart, the runtime waited for a new
bar as expected. At `12:20 MSK`, while still in `SyncingGap`, it generated a BO
short signal but did not emit it:

```text
dt_local = 2026-04-28 12:20:00
owner = IntradayBreakout
side = Short
can_emit = false
can_execute = false
```

After the gateway recovered and the runtime returned to `LiveReady`, BO entry
was emitted and accepted:

```text
12:50:00 MSK  submit_entry owner=IntradayBreakout side=Short
12:50:00 MSK  intent_emitted request_id=7093a557-c13f-5556-b9d8-579f632bd2b8
12:50:01 MSK  fill sell 1 IMOEXF @ 2712.0
```

Current broker snapshot at the evening check:

```text
IMOEXF qty = -1.0
avg_price = 2712.0
owner/comment = BO entry
```

Verdict: BO short is live and expected to be managed by the hybrid BO lifecycle.
No protective TP/SL is expected for BO; protective TP/SL belongs to the MR
component only.

Watchpoint: verify same-day BO exit / EOD flatten later in the session.

### Alor-USDRUBF Hybrid (`7502T0U`)

No fresh USDRUBF trade lifecycle appeared in the checked `06:00 UTC+` window.
The stack experienced the shared transport reset and recovered:

```text
12:20 MSK  live_guard BLOCKED due gateway/ws/cws
12:30 MSK  live_guard ALLOWED
```

Broker position stream tail showed only RUB cash positions in the latest
entries, with no fresh open USDRUBF position in the sampled tail.

Verdict: штатно / idle in this observation window.

### RI Author41/42 Shadow

The new RI shadow contour remained isolated and healthy:

```text
compose project = trading-ri-shadow
gateway ticker  = RIM6
model stream    = md.bars.RI.10m
runner symbol   = RIM6
stream XLEN     = 275
consumer group  = moex-author41-42-shadow-ri
pending         = 0
lag             = 0
journal rows    = 34
```

The Alor subscription to generic `RI` had failed earlier with:

```text
Instrument with symbol RI was not found in exchange MOEX
```

The active `RIM6` subscription works and produces `10m` bars. The RI contour is
shadow-only; no RI order-emitting runtime is running.

Operational nuance: the RI shadow gateway uses the same `7502SN6` portfolio for
transport auth, so its isolated Redis receives portfolio position snapshots
including the live IMOEXF BO short. This does not mean RI shadow is trading; it
only reflects the account-level positions subscription.

### Evening Verdict

The systems are operationally healthy at the checkpoint:

```text
sessiongap       clean entry/exit, flat
hybrid           BO short open as expected after from-zero restart
alor-usdrubf     idle/flat in sampled tail
RI shadow        collecting RIM6 10m data and writing journal
VPS resources    normal
```

Known watchpoints:

```text
1. Confirm hybrid BO same-day/EOD exit later today.
2. Watch Redis memory, especially alor-usdrubf and hybrid, after the retention changes.
3. Review RI shadow journal after a full session for rolling-vs-finalized output semantics.
```
