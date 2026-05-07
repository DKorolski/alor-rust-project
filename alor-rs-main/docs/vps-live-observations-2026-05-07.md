# VPS Live Observations - 2026-05-07

## Pre-Session Check

Observed at:

```text
2026-05-07 08:07 MSK
```

Container status:

```text
sessiongap          runtime/gateway/redis up, healthy
alor-usdrubf        runtime/gateway/redis up, healthy
hybrid-imoexf       runtime/gateway/redis up, healthy
ri-author41/42      runtime/gateway/redis up, healthy
legacy ri-shadow    runner/gateway/redis up
```

Resource status:

```text
RAM       1.6 GiB used / 7.7 GiB total, 6.1 GiB available
swap      90 MiB used / 3.9 GiB total
disk /    31 GiB used / 79 GiB total, 45 GiB available, 41%

RI micro runtime    2.66 MiB / 768 MiB
RI micro gateway    4.24 MiB / 768 MiB
RI micro Redis      127.4 MiB / 768 MiB
```

Broker-state check:

```text
sessiongap       nonzero=0 working=0
alor-usdrubf     nonzero=0 working=0
hybrid-imoexf    nonzero=0 working=0
ri-author41/42   nonzero=0 working=0
```

RI Author41/42 micro state:

```text
mode                   micro_live
allow_order_emission   true
execution_path         action_scoped_only
live_adapter_enabled   true
phase                  flat
last_processed_bar_ts  RIM6=1778099400
last_bar_ts            1778100000
last_model_bar_ts      1778099400
model_bars_seen        697
suppressed_service_bars 148
model_decisions_seen   13
last_decision_key      ri_author41_42_primary_combo_cost2|author42_bo|2026-05-06 17:00:00|Some(Long)|Some(2026-05-06T23:00:00)|time_exit_same_bar_close
```

RI micro command safety:

```text
cmd.orders.7502MIW.ri_author41_42.micro  0
cmd.acks.7502MIW.ri_author41_42.micro    0
```

RI Redis memory:

```text
used_memory_human       116.44M
maxmemory_human         512.00M
mem_fragmentation_ratio 1.09
```

Interpretation:

```text
The RI micro rollout remained safe overnight. The runtime is active in
micro_live mode with live_adapter_enabled=true and action_scoped_only execution,
but no live commands were emitted before the next regular session. Historical
warmup/model state is populated, and the strategy remains flat.
```

## Log Review

RI Author41/42:

```text
runtime WARN/ERROR since midnight: none
gateway WARN/ERROR: reconnect/reset noise only
command stream: empty
ack stream: empty
```

Other systems:

```text
sessiongap       no new trades, flat; runtime live_guard blocked during night sync
alor-usdrubf     no new trades, flat; runtime live_guard blocked during night sync
hybrid-imoexf    no new trades, flat; runtime live_guard blocked during night sync
```

Observed gateway noise:

```text
CWS/WS EOF or reset reconnects occurred across several stacks during the night.
All relevant transport failures had pending_count=0.
Alor-USDRUBF had one positions subscribe AckTimeout during reconnect.
Hybrid gateway replayed an existing canceled IMOEXF stop order snapshot.
```

Interpretation:

```text
The overnight noise is consistent with the known Alor WS/CWS reconnect pattern
and did not coincide with live commands, open positions, or working orders. No
operator action is required before the session, but the first RI micro live
intent must still be watched closely for action-scoped CWS path and clean
broker ack/fill lifecycle.
```

## RI Redis Health Retention Trim

The first post-rollout RI Redis read showed that the footprint was dominated by
the diagnostic health stream, not by bars, commands, acks, or runtime state:

```text
events.health.ri_author41_42.7502MIW  18,527 rows, ~117.79 MiB
broker.snapshots.7502MIW              ~5.59 MiB
broker.positions.7502MIW              ~0.52 MiB
md.bars.7502MIW.RIM6.10m              ~0.51 MiB
runtime.state.ri_author41_42.shadow   ~0.42 MiB
runtime.state.ri_author41_42.micro    ~0 MiB
cmd.orders/acks micro                 0 rows
```

Safe online trim was applied only to the RI health stream:

```text
XTRIM events.health.ri_author41_42.7502MIW MAXLEN 2000
MEMORY PURGE
```

Result:

```text
before_len=18541
before_mem=123608428
trimmed=16541
after_len=2000
after_mem=13333980
used_memory_human=19.47M
maxmemory_human=512.00M
mem_fragmentation_ratio=1.56
cmd.orders.7502MIW.ri_author41_42.micro=0
cmd.acks.7502MIW.ri_author41_42.micro=0
```

Interpretation:

```text
This was safe hygiene cleanup. It did not touch model bars, broker snapshots,
positions, runtime state, command streams, or ack streams. The RI health trim
target should be reduced from 5000 to 2000 because this stream is diagnostic
heartbeat history, not trading state.
```

Config / maintenance follow-up:

```text
RI runtime health trim target: 5000 -> 2000
RI shadow health trim target:  5000 -> 2000
safe-trim script special case: events.health.ri_author41_42.* -> 2000
VPS active config updated: yes
VPS safe-trim dry-run after update: RI health would trim at limit=2000
final verification: health_len=2000, used_memory_human=19.49M
micro cmd/ack streams remained empty
```

## Watch Items

```text
1. Confirm RI moves from BLOCKED/SyncingHistory to LiveReady after the first
   eligible RIM6 10m live bar.
2. Confirm RI command stream remains empty until an actual same-day model
   intent is generated.
3. On first RI micro command, verify:
   - qty=1
   - stream=cmd.orders.7502MIW.ri_author41_42.micro
   - gateway action scope session open/send/result logs
   - no legacy CWS path
   - broker ack/fill is accepted and reconciled
4. Continue routine Redis memory watch after the health retention reduction and
   micro rollout.
```

## Verdict

```text
STATUS = CLEAN_PRE_SESSION_POST_RI_MICRO_ROLLOUT
RI_MICRO = ENABLED / FLAT / NO_COMMANDS / ACTION_SCOPED_ONLY
ALL_LIVE_CONTOURS = FLAT / NO_WORKING_ORDERS
WARN_ERROR = RECONNECT_NOISE_ONLY_PENDING_COUNT_0
PATCH_REQUIRED = NO
OPERATOR_ACTION_REQUIRED = NO
```

## Post-Open RI Micro Incident

Collection time: 2026-05-07 ~09:10 MSK.

All existing live contours moved cleanly from night sync into live readiness:

```text
sessiongap       BLOCKED -> ALLOWED, flat, no working orders
alor-usdrubf     BLOCKED -> ALLOWED, flat, no working orders
hybrid-imoexf    BLOCKED -> ALLOWED, flat, no working orders
```

Hybrid IMOEXF also finalized the risk-gate shadow ledger for 2026-05-06:

```text
risk_gate_shadow_session_finalized session_date=2026-05-06 shadow_pnl_points=11.4 shadow_trade_count=1
risk gate runtime session finalized inserted_records=1 duplicate_records=0 state_refreshed=true
```

RI Author41/42 generated the first real micro-live entry on the 09:00 model bar:

```text
component=author41_mr
role=entry
model_side=long
order_side=Buy
qty=1
order_style=market_p0
execution_path=action_scoped_only
decision_key=ri_author41_42_primary_combo_cost2|author41_mr|2026-05-07 09:00:00|Some(Long)|live_prospective
```

The transport path was correct. Gateway received the command and used
action-scoped CWS:

```text
command received strategy_id=ri_author41_42.micro.7502MIW symbol=RIM6 action=market
action_scope_send_start primary_opcode=create:market opcode=authorize
action_scope_send_start primary_opcode=create:market opcode=create:market
action_scope_send_result primary_opcode=create:market http_code=400 order_id=None
```

The broker rejected the command:

```text
status=Rejected
error_code=cws_http_400
error_msg="Неизвестный инструмент в заявке"
cws_http_code=400
```

Immediate safety action:

```text
trading-ri-author41-42-7502miw-strategy-runtime-1 stopped
gateway and redis left running
cmd.orders.7502MIW.ri_author41_42.micro = 1
cmd.acks.7502MIW.ri_author41_42.micro   = 1
broker position snapshot remains flat for RI
```

Read-only Alor reference-data check confirmed the symbol split:

```text
GET /md/v2/Securities/MOEX/RIM6
symbol=RTS-6.26
shortname=RIM6
board=RFUD
tradingStatusInfo="нормальный период торгов"

GET /md/v2/Securities/MOEX/RTS-6.26
symbol=RTS-6.26
shortname=RIM6
board=RFUD
```

Interpretation:

```text
This is not a regression to the legacy CWS path. The command used
action_scoped_only as expected. The failure is RI-specific symbol semantics:
market data bars arrive and are stored with shortname RIM6, but CWS order
placement appears to require the full Alor security symbol RTS-6.26.

A secondary safety issue was also found: after rejected entry, RI runtime state
had already moved to live_in_position even though broker stayed flat. This must
be patched before restarting RI micro.
```

Patch line:

```text
1. Keep model/warmup symbol = RIM6.
2. Add RI order_symbol = RTS-6.26 for CWS commands.
3. Route live order commands with order_symbol while preserving RIM6 bars.
4. Roll back RI live state to flat on rejected entry when broker position is flat.
5. Rebuild/redeploy only RI runtime after tests.
6. Restart RI from clean state; do not replay the stale live_in_position tail.
```
