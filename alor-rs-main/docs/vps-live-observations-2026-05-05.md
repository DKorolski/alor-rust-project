# VPS Live Observations (2026-05-05)

## Post-Session Review

Reviewed session:

```text
session_date = 2026-05-04
review_time  = 2026-05-05 07:39 MSK
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

Host state:

```text
RAM total        7.7Gi
RAM used         1.9Gi
RAM available    5.8Gi
Swap used        409Mi / 3.9Gi
Disk /           29G / 79G used, 47G free, 39%
```

Container memory:

```text
sessiongap redis             95.17MiB / 1GiB
alor-usdrubf redis           104.2MiB / 1GiB
hybrid redis                 101.3MiB / 1GiB
ri-7502miw redis             390.1MiB / 768MiB
legacy ri-shadow redis       134.9MiB / 768MiB
```

Interpretation:

```text
No host RAM pressure.
No disk pressure.
RI 7502MIW Redis remains the largest active Redis container, but it is still
below maxmemory. Continue watching it during the next safe-trim cycle.
```

## Stack Health

Active stacks:

```text
trading-sessiongap-*                  healthy, up 11 days
trading-alor-usdrubf-*                healthy, up 8-11 days
trading-hybrid-*                      healthy, up 3 days
trading-ri-author41-42-7502miw-*      healthy, up 3 days
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
2026-05-04 00:00:00 MSK .. 2026-05-05 00:00:00 MSK
```

Runtime findings:

```text
sessiongap runtime       1 WARN: orphan_trade for accepted USDRUBF entry fill
alor-usdrubf runtime     1 WARN: orphan_trade for accepted USDRUBF entry fill
hybrid runtime           0 WARN/ERROR signatures
ri-7502miw runtime       0 WARN/ERROR signatures
legacy ri-shadow runner  0 WARN/ERROR signatures
```

Gateway findings:

```text
sessiongap gateway       24 WARN lines
alor-usdrubf gateway     22 WARN lines
hybrid gateway           26 WARN lines
ri-7502miw gateway       22 WARN lines
legacy ri-shadow gateway 23 WARN lines
```

The gateway WARNs were reconnect/session-noise class:

```text
cws disconnected: protocol_reset_without_close_handshake
ws hub error; reconnecting
HTTP error: 502 Bad Gateway
pending_count=0 on CWS transport failures
```

Interpretation:

```text
No command reject storm.
No Redis read failure.
No OOM / broken pipe / Connection refused.
No active command was in-flight during the observed CWS disconnects.
The two runtime orphan_trade lines are watch items, but both affected trades
were accepted broker fills and the runtimes converged to flat.
```

## Live Economy Summary

### SessionGap USDRUBF (`7502MIW`)

Fills:

```text
2026-05-04 13:00:04 MSK  buy  1 @ 75.65  intent_class=entry
2026-05-04 23:31:04 MSK  sell 1 @ 75.67  intent_class=exit
```

Result:

```text
closed_cycles        1
ack_status           accepted=2
ack_errors           0
gross_points         +0.02
commission_sum       6.92
latest_position      qty=0.0
working_orders       none
runtime_phase        Flat
traded_session       true
```

Interpretation:

```text
The session is economically tiny but logically correct: one entry, one EOD
exit, broker flat, no working orders.
```

### Alor-USDRUBF Hybrid (`7502T0U`)

Fills:

```text
2026-05-04 11:30:01 MSK  buy  1 @ 75.54  USDRUBF|entry|day_breakout_waitfix
2026-05-04 23:40:55 MSK  sell 1 @ 75.67  USDRUBF|exit|bo_eod_exit
```

Result:

```text
closed_cycles        1
ack_status           accepted=2
ack_errors           0
gross_points         +0.13
commission_sum       6.92
latest_position      qty=0.0
working_orders       none
lifecycle_stage      broker_position_flat
hybrid_state         flat
```

Interpretation:

```text
The BO path entered and exited at EOD as expected. No stale pending request or
tracked order state was present after the session.
```

### Hybrid IMOEXF (`7502SN6`)

Fills:

```text
2026-05-04 12:20:01 MSK  sell 1 @ 2635.0  HYB|sid=hybrid_imoexf|c=69f8626801|o=BO|r=ENTRY
2026-05-04 23:40:19 MSK  buy  1 @ 2624.0  HYB|sid=hybrid_imoexf|c=69f8626801|o=BO|r=EXIT
```

Result:

```text
closed_cycles        1
component            BO
ack_status           accepted=2
ack_errors           0
gross_points         +11.0
commission_sum       3.52
latest_position      qty=0.0
working_orders       none
runtime_state        flat
```

Runtime state after session:

```text
active_cycle_id                 null
last_position_qty               0.0
current_owner                   null
pending_entry_request_id        null
pending_exit_request_id         null
safe_mode_close_only            false
last_day_local                  2026-05-04
was_short_today                 true
last_trade_id                   2033126196669100480
```

Interpretation:

```text
This was the cleanest economic contributor of the day. The BO short was held
through the day and flattened near EOD without stale-cycle symptoms.
```

## Hybrid Riskgate

Runtime state:

```text
risk_gate_shadow_session_date        2026-05-04
risk_gate_shadow_pnl_points          0.0
risk_gate_shadow_trade_count         0
risk_gate_mr_enabled_current_session true
risk_gate_rolling_sum_lb120          192.5000000000002
risk_gate_last_finalized_session     2026-05-01
risk_gate_ledger_rows_count          185
```

Materialized Redis state:

```text
ledger_rows_count             185
last_finalized_session_date   2026-05-01
rolling_sum_lb120             192.5000000000002
mr_enabled_current_session    true
mr_enabled_next_session       true
```

Interpretation:

```text
The actual 2026-05-04 live trade was BO and is not part of the MR riskgate
shadow PnL. The 2026-05-04 riskgate session has not yet been finalized at this
07:39 MSK review point; this is expected before the next regular-session
startup event. Watch that it finalizes after the next eligible 10m bar.
```

## RI Author41/42 7502MIW Shadow

Journal totals after 2026-05-04:

```text
total_journal_rows       24
shadow_model_decisions   8
sessions_with_decisions  5
live_command_rows        0
```

Per-session decisions:

```text
2026-04-28  1
2026-04-29  2
2026-04-30  2
2026-05-01  1
2026-05-04  2
```

2026-05-04 decisions:

```text
09:00 MSK  author41_mr short  exit=11:00  reason=take_author_close       shadow_pnl_points=+328
12:00 MSK  author42_bo short  exit=23:00  reason=time_exit_same_bar_close shadow_pnl_points=+798
```

Shadow economics from runtime `ri_model_decision` logs:

```text
total_decisions      8
total_shadow_pnl     +2144 points
wins / losses        7 / 1

author41_mr          6 decisions, +1608 points
author42_bo          2 decisions,  +536 points

long                 3 decisions,  +584 points
short                5 decisions, +1560 points
```

Per-session shadow PnL:

```text
2026-04-28  +588
2026-04-29   -34
2026-04-30  +356
2026-05-01  +108
2026-05-04 +1126
```

Runtime state:

```text
mode                   shadow
allow_order_emission   false
execution_path         action_scoped_only
live_adapter_enabled   false
model_bars_seen        518
model_decisions_seen   8
phase                  flat
last_transition_reason dry_run_exit:time_exit_same_bar_close
last_trade_id          null
```

Interpretation:

```text
RI shadow remains pre-GO safe. The 2026-05-04 session added useful evidence:
one MR decision and one BO decision, both ending flat in shadow lifecycle with
no command emission.
```

## Verdict

```text
OVERALL_STATUS = CLEAN_POST_SESSION
SESSION_DATE = 2026-05-04
SESSIONGAP = FLAT / 1_CYCLE / +0.02_GROSS_POINTS
ALOR_USDRUBF = FLAT / 1_CYCLE / +0.13_GROSS_POINTS
HYBRID_IMOEXF = FLAT / 1_BO_CYCLE / +11.0_GROSS_POINTS
RI_7502MIW_SHADOW = 2_NEW_MODEL_DECISIONS / NO_LIVE_COMMANDS
ACK_ERRORS = NONE_ON_LIVE_CONTOURS
GATEWAY_NOISE = RECONNECT_ONLY_WITH_PENDING_COUNT_0
RESOURCE_STATUS = SAFE
PATCH_REQUIRED = NO
```

Follow-up:

```text
1. Confirm hybrid riskgate finalizes 2026-05-04 after the next regular-session
   startup bar.
2. Continue RI shadow observation; evidence improved to 8 total model
   decisions, but keep size-1 micro promotion gated by controlled rollout.
3. Keep hybrid IMOEXF at size 1 for now; the post-update result is positive,
   but still based on a small number of closed cycles.
4. Watch isolated orphan_trade warnings on sessiongap/alor-usdrubf. They did
   not prevent flat convergence, but should not grow into a repeated pattern.
```
