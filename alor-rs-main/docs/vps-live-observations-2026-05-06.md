# VPS Live Observations (2026-05-06)

## Post-Session Review

Reviewed session:

```text
session_date = 2026-05-05
review_time  = 2026-05-06 08:07 MSK
host         = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
alor-usdrubf     USDRUBF hybrid / 7502T0U
hybrid           IMOEXF riskgate-shadow / 7502SN6
RI shadow        RIM6 / 7502MIW Author41/42 shadow
legacy RI shadow old contour
VPS resources and Redis state
```

## VPS Resources

Host state at review:

```text
RAM total        7.7Gi
RAM used         2.2Gi
RAM available    5.5Gi
Swap used        382Mi / 3.9Gi
Disk /           29G / 79G used, 46G free, 39%
```

Container memory:

```text
sessiongap redis             95.91MiB / 1GiB
alor-usdrubf redis           127MiB / 1GiB
hybrid redis                 203.2MiB / 1GiB
ri-7502miw redis             498.1MiB / 768MiB
legacy ri-shadow redis       145.4MiB / 768MiB
```

Interpretation:

```text
No host RAM pressure.
No disk pressure.
RI 7502MIW Redis is still the largest Redis container and should remain on the
watchlist, but it is not at maxmemory during this review.
```

## RI Redis Maintenance

Maintenance time:

```text
2026-05-06 09:18-09:20 MSK
```

Reason:

```text
trading-ri-author41-42-7502miw-redis-1 approached maxmemory:
used_memory_human = 494.30M
maxmemory_human   = 512.00M
```

Key-level finding:

```text
events.health.ri_author41_42.7502MIW  82484 rows, ~465MB
broker.snapshots.7502MIW              12217 rows, ~5.3MB
md.bars.7502MIW.RIM6.10m               1212 rows, ~0.38MB
runtime.state.ri_author41_42...          302 rows, ~0.35MB
```

Action:

```text
XTRIM events.health.ri_author41_42.7502MIW MAXLEN 2000
XTRIM broker.snapshots.7502MIW MAXLEN 3000
MEMORY PURGE
```

Preserved:

```text
md.bars.7502MIW.RIM6.10m
runtime.state.ri_author41_42.shadow.7502MIW
broker.positions.7502MIW
broker.orders.7502MIW
broker.trades.7502MIW
cmd.orders.7502MIW.ri_author41_42.shadow
cmd.acks.7502MIW.ri_author41_42.shadow
```

Result:

```text
used_memory_human before  494.30M
used_memory_human after    15.51M
docker stats after         27.31MiB / 768MiB
events.health rows          2001
broker.snapshots rows       3000
```

Post-check:

```text
RI runtime continued normally after trim.
Latest post-trim log: ri_model_bar_observed dt_local=2026-05-06 09:10:00
No post-trim runtime ERROR/WARN observed.
```

## Stack Health

Active stacks:

```text
trading-sessiongap-*                  healthy, up 12 days
trading-alor-usdrubf-*                healthy, up 9-12 days
trading-hybrid-*                      healthy, up 4-12 days
trading-ri-author41-42-7502miw-*      healthy, up 4 days
trading-ri-shadow-*                   running, no explicit healthcheck
```

Images of note:

```text
hybrid runtime      manual-512b8e1-hybrid-stalecycle-20260501
hybrid gateway      manual-5430299-protplace-20260428
ri-7502miw runtime  manual-d6d0e7e-ri7502miw-20260501
```

## Log Scan

Window:

```text
2026-05-05 00:00:00 MSK .. 2026-05-06 00:00:00 MSK
```

Runtime findings:

```text
sessiongap runtime       0 WARN/ERROR
alor-usdrubf runtime     0 WARN/ERROR
hybrid runtime           2 WARN, 1 rejected cleanup command
ri-7502miw runtime       0 WARN/ERROR
legacy ri-shadow runner  0 WARN/ERROR
```

Gateway findings:

```text
sessiongap gateway       12 WARN lines
alor-usdrubf gateway      8 WARN lines
hybrid gateway           16 WARN lines, 1 rejected cleanup ack
ri-7502miw gateway       10 WARN lines
legacy ri-shadow gateway  8 WARN lines
```

The gateway WARNs were reconnect/session-noise class:

```text
cws disconnected: eof
cws disconnected: protocol_reset_without_close_handshake
ws hub error; reconnecting
AckTimeout for positions
pending_count=0 on transport failures
```

Hybrid cleanup WARN:

```text
2026-05-05 10:50:12 MSK
request_id=455dc6ab-e262-5526-8e45-b4833c8c197b
error_code=cws_http_400
message="Order to cancel not found"
context=flat cleanup after MR TP fill
```

Interpretation:

```text
The hybrid cleanup reject was non-fatal. The TP limit was already filled, the
stop-limit cleanup was accepted separately, and the broker state later showed
no IMOEXF position and no working orders.
```

## Live Economy Summary

### SessionGap USDRUBF (`7502MIW`)

Result:

```text
traded_session       false
fills                none
runtime_warn_error   0
latest_position      flat
working_orders       none
```

Interpretation:

```text
No sessiongap trade was generated on 2026-05-05. This is acceptable for the
strategy: no missed exit or stale order state was observed.
```

### Alor-USDRUBF Hybrid (`7502T0U`)

Fills:

```text
2026-05-05 11:10:01 MSK  sell 1 @ 75.38
2026-05-05 12:00:01 MSK  buy  1 @ 75.55
```

Result:

```text
closed_cycles        1
gross_points         -0.17
commission_sum       6.96
latest_position      flat
working_orders       none
runtime_warn_error   0
```

Interpretation:

```text
The strategy entered a USDRUBF short and exited within the session. The result
was negative, but the lifecycle was clean: no rejected command, no stale pending
state, and broker flat after exit.
```

### Hybrid IMOEXF (`7502SN6`)

Fills:

```text
2026-05-05 09:10:02 MSK  buy  1 @ 2621.5  MR long entry
2026-05-05 09:37:09 MSK  sell 1 @ 2623.0  MR long TP exit
2026-05-05 10:30:06 MSK  sell 1 @ 2627.5  MR short entry
2026-05-05 10:50:11 MSK  buy  1 @ 2622.0  MR short TP exit
```

Result:

```text
closed_cycles        2
component            MeanReversion bracket
gross_points         +7.0
commission_sum       6.92
latest_position      flat
working_orders       none
runtime_warn_error   2 WARN from cleanup race
```

Action-scoped/protective path observations:

```text
MR long entry         place accepted
MR long TP            place accepted
MR long SL            create_stop_limit accepted
MR long SL cleanup    delete_stop_limit accepted

MR short entry        place accepted
MR short TP           place accepted and filled
MR short SL           create_stop_limit accepted
MR short TP cleanup   cancel rejected: Order to cancel not found
MR short SL cleanup   delete_stop_limit accepted
```

Interpretation:

```text
The day was economically positive and ended cleanly. The only anomaly was a
benign cleanup race on the already-filled TP order; the protective stop was
deleted and no broker-side working order remained.
```

## Hybrid Riskgate

Runtime / materialized state observed after the session:

```text
seed_loaded                    true
ledger_rows_count              186
last_finalized_session_date    2026-05-04
rolling_sum_lb120              185.2000000000002
mr_enabled_current_session     true
mr_enabled_next_session        true
current_shadow_pnl_points      0.0
current_generation             runtime-ledger-v1
```

Session event:

```text
2026-05-05 09:10:02 MSK
risk_gate_shadow_session_finalized
session_date=2026-05-04
shadow_pnl_points=0.0
shadow_trade_count=0
```

Interpretation:

```text
The previous watch item resolved: 2026-05-04 was finalized on the next
regular-session startup bar. The 2026-05-05 MR trades were allowed by the gate.
Continue watching that 2026-05-05 finalizes on the next regular-session startup.
```

## RI Author41/42 7502MIW Shadow

2026-05-05 decisions:

```text
09:00 MSK  author41_mr long  exit=10:00  reason=take_author_close  shadow_pnl_points=+528
10:50 MSK  author41_mr long  exit=12:30  reason=take_author_close  shadow_pnl_points=+218
```

Shadow economics after 2026-05-05:

```text
total_decisions      10
total_shadow_pnl     +2890 points
wins / losses        9 / 1

author41_mr          8 decisions, +2354 points
author42_bo          2 decisions,  +536 points

2026-05-05           2 decisions, +746 points
```

Runtime state:

```text
mode                   shadow
allow_order_emission   false
execution_path         action_scoped_only
live_adapter_enabled   false
model_bars_seen        606
model_decisions_seen   10
phase                  flat
last_transition_reason dry_run_exit:take_author_close
last_trade_id          null
```

Interpretation:

```text
RI shadow remains pre-GO safe: no command emission, no live broker orders, and
flat dry-run lifecycle. The economic read continues to improve, but the sample
is still small, especially for Author42 BO.
```

## Pre-Session Check (2026-05-06)

Observed at:

```text
2026-05-06 08:07 MSK
```

Current broker state:

```text
sessiongap       non-cash positions none, working orders none, today trades 0
alor-usdrubf     non-cash positions none, working orders none, today trades 0
hybrid-imoexf    non-cash positions none, working orders none, today trades 0
ri-author41/42   non-cash positions none, working orders none, today trades 0
legacy ri        broker streams empty
```

Runtime pre-session logs:

```text
sessiongap runtime       0 WARN/ERROR
alor-usdrubf runtime     0 WARN/ERROR
hybrid runtime           0 WARN/ERROR since midnight
ri-7502miw runtime       0 WARN/ERROR since midnight
legacy ri-shadow runner  0 WARN/ERROR since midnight
```

Interpretation:

```text
The system starts 2026-05-06 flat and clean before the regular trading window.
The only observed pre-session noise is gateway reconnect/reset noise with no
in-flight command exposure.
```

## Verdict

```text
OVERALL_STATUS = CLEAN_POST_SESSION_WITH_ONE_BENIGN_HYBRID_CLEANUP_WARN
SESSION_DATE = 2026-05-05
SESSIONGAP = FLAT / NO_TRADE
ALOR_USDRUBF = FLAT / 1_CYCLE / -0.17_GROSS_POINTS
HYBRID_IMOEXF = FLAT / 2_MR_CYCLES / +7.0_GROSS_POINTS
RI_7502MIW_SHADOW = 2_NEW_MODEL_DECISIONS / +746_POINTS / NO_LIVE_COMMANDS
ACK_ERRORS = ONE_HYBRID_CLEANUP_CANCEL_REJECT_ONLY
GATEWAY_NOISE = RECONNECT_ONLY_WITH_PENDING_COUNT_0
RESOURCE_STATUS = SAFE
PATCH_REQUIRED = NO_IMMEDIATE_PATCH
```

Follow-up:

```text
1. Keep watching hybrid cleanup races; current one was benign, but repeated
   cancel-after-fill rejects should remain visible in observations.
2. Confirm hybrid riskgate finalizes 2026-05-05 on the next eligible regular
   session startup bar.
3. Continue RI shadow observation for at least 3-5 additional trading sessions
   before any controlled micro GO decision.
4. Keep RI Redis on the resource watchlist; it is the largest active Redis
   container but remains below maxmemory.
```

## Post-Session Flat Gate (2026-05-06)

Observed at:

```text
2026-05-06 23:40 MSK
```

Broker-state check:

```text
sessiongap       non-cash positions none, working orders none
alor-usdrubf     non-cash positions none, working orders none
hybrid-imoexf    non-cash positions none, working orders none
ri-author41/42   non-cash positions none, working orders none
```

Important lifecycle events:

```text
sessiongap       13:00 short USDRUBF, 13:10 buy exit, broker-flat
alor-usdrubf     12:20 short USDRUBF, 23:40 BO EOD buy exit, broker-flat
hybrid-imoexf    11:30 MR short IMOEXF, 12:00 TP buy exit, broker-flat
ri-author41/42   shadow only, no live command emission
```

Alor-USDRUBF EOD exit path:

```text
request_id=4866ec64-8006-5ef1-a9c8-145c106d990f
action=create:market
control_cws_mode=action_scoped
status=Accepted
broker_order_id=2023556056051328601
```

Resources:

```text
RAM      2.0 GiB used / 7.7 GiB total, 5.7 GiB available
disk /   29 GiB used / 79 GiB total, 46 GiB available, 39%
RI Redis 90.68 MiB / 768 MiB after 2026-05-06 trim
```

Interpretation:

```text
The 2026-05-06 session ended in a clean flat gate across all live/shadow
contours. No working or stop orders were detected in the checked broker streams.
The alor-usdrubf EOD exit used the expected action-scoped CWS path; no legacy
CWS regression was observed in the final exit path.
```

RI micro readiness note:

```text
This is a valid operational window for a controlled RI micro rollout only after
an explicit final GO decision. The rollout must still use the pending micro
configs, clear only the micro runtime state/command/ack streams, preserve the
10m RIM6 bar stream, and verify live_adapter_enabled=true plus
execution_path=action_scoped_only after restart.
```
