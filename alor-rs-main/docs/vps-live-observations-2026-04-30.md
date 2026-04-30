# VPS Live Observations (2026-04-30)

## Morning Checkpoint

Observation time:

```text
2026-04-30 10:14 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
hybrid           IMOEXF riskgate-shadow / 7502SN6
alor-usdrubf     USDRUBF hybrid / 7502T0U
RI shadow        RIM6 feed -> RI Author41/42 shadow journal
```

## Stack Status

All checked containers were up. The three trading runtimes and gateways were
healthy:

```text
trading-sessiongap-strategy-runtime-1       up 7 days, healthy
trading-sessiongap-alor-gateway-1           up 7 days, healthy
trading-sessiongap-redis-1                  up 7 days, healthy

trading-hybrid-strategy-runtime-1           up 46 hours, healthy
trading-hybrid-alor-gateway-1               up 2 days, healthy
trading-hybrid-redis-1                      up 7 days, healthy

trading-alor-usdrubf-strategy-runtime-1     up 3 days, healthy
trading-alor-usdrubf-alor-gateway-1         up 3 days, healthy
trading-alor-usdrubf-redis-1                up 7 days, healthy

trading-ri-shadow-ri-shadow-runner-1        up 40 hours
trading-ri-shadow-alor-gateway-1            up 40 hours
trading-ri-shadow-redis-1                   up 40 hours
```

Redis memory after the previous safe trim continued to grow from live traffic
but remained controlled:

```text
sessiongap redis      183.03M
hybrid redis          301.11M
alor-usdrubf redis    187.77M
RI shadow redis       197.03M / 512M
```

## Log Scan

No runtime `ERROR`, panic, command reject, Redis connection refusal, `NOGROUP`,
or `BUSYGROUP` signature was found in the 24h scan.

Observed transport noise:

```text
2026-04-29 15:59 MSK    CWS/WS EOF reconnects across stacks
2026-04-29 23:40-23:50  CWS/WS EOF reconnects across stacks
2026-04-30 06:24-06:35  protocol reset / reconnect sequence
```

Important details:

```text
sampled CWS transport failures had pending_count = 0
sessiongap had one AckTimeout for positions during the reconnect window
RI shadow had one AckTimeout for positions during the reconnect window
no command reject or stuck command lifecycle was seen
```

Interpretation: normal Alor transport/reconnect noise for this soak line, not a
VPS resource incident.

## Current Broker State

Latest position tails:

```text
sessiongap       no USDRUBF futures position; RUB cash only
hybrid           IMOEXF qty = 0.0; RUB cash only
alor-usdrubf     USDRUBF qty = 0.0; RUB cash only
```

Verdict: all three live trading stacks were flat at the checkpoint.

## SessionGap USDRUBF

SessionGap has been quiet after the clean 2026-04-28 round-trip.

Latest trade/order streams:

```text
broker.orders.7502MIW = 5
broker.trades.7502MIW = 3
latest entry          = buy 1 USDRUBF @ 75.05 on 2026-04-28
latest exit           = sell 1 USDRUBF @ 75.08 on 2026-04-28
latest position tail  = RUB cash only
```

Latest runtime state:

```text
session_date       = 2026-04-30
phase              = Flat
traded_session     = false
last_trade_ts      = 2026-04-28 exit
last_bar_ts        = live 2026-04-30 bars
prev_close         = 74.93
yesterday_range    = 1.04
first_hour_price   = 75.05
session_high/low   = 75.28 / 74.87
```

Config context:

```text
feed              = md.bars.7502MIW.10m
signal_minute     = 50
wait_hours        = 3
max_entry_hour    = 16
session_gap_min   = 60.0
k_tp_long/short   = 0.28 / 0.28
k_sl_long/short   = 0.68 / 0.65
```

Interpretation: several days without fresh SessionGap trades is acceptable and
consistent with the strategy profile. The feed is live, the runtime state is
flat and updating, and no readiness/blocking error was observed. Given the
strict session-gap trigger and `max_entry_hour=16`, the absence of a trade is a
valid no-signal outcome rather than evidence of a broken stack.

## Hybrid IMOEXF

Hybrid riskgate state is current:

```text
risk_gate_shadow_session_date       = 2026-04-30
risk_gate_last_finalized_session    = 2026-04-29
risk_gate_ledger_rows_count         = 183
risk_gate_rolling_sum_lb120         = 191.6000000000002
risk_gate_mr_enabled_current        = true
current broker position             = flat
```

The 2026-04-29 session contained multiple MR cycles and one BO short. The BO
short was closed by the live no-overnight guard:

```text
2026-04-29 15:20 MSK    BO short entry accepted/fill @ 2673.5
2026-04-29 15:30 MSK    breakout_no_overnight_guard_exit WARN
2026-04-29 15:30 MSK    BreakoutEodExit accepted/fill buy @ 2670.5
```

Verdict: position safety worked and the account is flat. The `WARN` is
operator-visible by design, but it should remain a watchpoint because it means
the rescue/no-overnight overlay, not the ordinary same-day EOD contour, closed
the BO position.

## Alor-USDRUBF Hybrid

The 2026-04-29 BO short lifecycle completed:

```text
2026-04-29 12:30 MSK    bo_short_signal accepted into pending entry
2026-04-29 12:40 MSK    market sell intent accepted
2026-04-29 12:40 MSK    sell fill @ 75.05
2026-04-29 23:40 MSK    bo_eod_exit buy intent accepted
2026-04-29 23:40 MSK    buy fill @ 74.92
post-exit               broker position open_to_flat
```

Latest runtime state:

```text
hybrid_state             = flat
open_position_qty        = 0.0
pending_request_ids      = []
entry_intent_inflight    = false
exit_intent_inflight     = false
last_trade_ts            = 2026-04-29 EOD exit
```

Verdict: штатно. The strategy entered and exited through the expected BO/EOD
market lifecycle, and current broker/runtime state is flat.

## RI Shadow

RI shadow remained up and continued collecting bars/journal rows. No runner
errors were found in the checked window. Redis usage remains below its
configured 512M cap.

Operational nuance remains unchanged: the RI shadow gateway uses the `7502SN6`
account transport, so account-level IMOEXF stop-order snapshots may appear in
the RI shadow gateway logs. This is not RI trading.

## Morning Verdict

```text
sessiongap       flat, live feed/state updating, no-signal quiet period is logical
hybrid           flat, riskgate ledger current, BO rescue exit watchpoint noted
alor-usdrubf     flat after successful BO short + EOD exit
RI shadow        healthy, collecting shadow data
Redis            controlled after trim, monitor growth but no immediate action
```

Follow-up:

```text
1. Keep watching hybrid BO exits: ordinary EOD contour vs no-overnight rescue should be separated in future reports.
2. Continue daily Redis checks; do not enable automatic trim until at least one more stable checkpoint.
3. SessionGap quiet period requires no action unless bars stop updating, phase leaves Flat unexpectedly, or command streams show rejected/stuck intents.
```
