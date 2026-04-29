# VPS Live Observations (2026-04-29)

## Morning Checkpoint

Observation time:

```text
2026-04-29 09:25 MSK
host = nektodk.ispvds.com
```

Scope:

```text
sessiongap       USDRUBF / 7502MIW
hybrid           IMOEXF riskgate-shadow / 7502SN6
alor-usdrubf     USDRUBF hybrid / 7502T0U
RI shadow        RIM6 feed -> RI Author41/42 shadow journal
```

## VPS Resources

All containers were up and the VPS had no resource-pressure signature:

```text
uptime        = 17 days, 17:58
load average  = 0.37, 0.36, 0.34
RAM           = 7.7Gi total, 3.0Gi used, 4.7Gi available
swap          = 54Mi / 3.9Gi
disk /        = 43G / 79G, 57% used
```

Container memory:

```text
sessiongap redis      = 637.5MiB / 1GiB
alor-usdrubf redis    = 685.5MiB / 1GiB
hybrid redis          = 659.5MiB / 1GiB
RI shadow redis       = 76.7MiB / 768MiB
RI shadow gateway     = 4.1MiB
RI shadow runner      = 1.4MiB
```

The three live Redis instances grew overnight but remained below the configured
limit. Continue watching them during the reduced-retention soak.

Stream lengths:

```text
sessiongap md.bars.7502MIW.10m                 = 632
hybrid md.bars.7502SN6.10m                     = 1656
alor-usdrubf md.bars.7502T0U.10m               = 811
RI shadow md.bars.RI.10m                       = 309
hybrid riskgate ledger rows                    = 182
RI shadow journal rows                         = 63
events.health on live stacks                   = 100000 each
```

## Overnight Transport Events

The overnight logs show several Alor WS/CWS reconnects across the stacks:

```text
2026-04-29 02:40 MSK  ws EOF / TLS close_notify warnings
2026-04-29 02:50 MSK  CWS eof reconnects
2026-04-29 06:24-06:35 MSK  reset / SyncingGap / SyncingHistory sequence
2026-04-29 08:25 MSK  CWS reset, recovered
```

Important details:

```text
pending_count = 0 in sampled CWS transport failures
no command rejected / panic / Connection refused found in the checked window
all three live runtimes returned to LiveReady around 09:00 MSK
```

Interpretation: network/provider transport noise, not a VPS resource incident.

## Hybrid IMOEXF

The 2026-04-28 BO short watchpoint closed cleanly at EOD:

```text
2026-04-28 23:30 MSK
action = submit_exit owner=IntradayBreakout reason=BreakoutEodExit
request_id = 10760e20-1421-5b1b-824f-68f6ea3edb90
path = action-scoped create:limit
fill = buy 1 IMOEXF @ 2698.5
entry reference = sell 1 IMOEXF @ 2712.0
```

Broker snapshot at the morning check showed no open IMOEXF position in the
latest position tail; only RUB cash entries were present.

Risk-gate ledger advanced correctly:

```text
session_date = 2026-04-28
source = runtime
shadow_pnl_points = 0.0
shadow_trade_count = 0
ledger_rows_count = 182
last_finalized_session_date = 2026-04-28
rolling_sum_lb120 = 175.1000000000002
mr_enabled_current_session = true
mr_enabled_next_session = true
```

Verdict: штатно. The BO EOD exit worked, account is flat, and the riskgate
ledger updated from runtime rather than reimporting seed.

Watchpoint: the `current_shadow_session_date` materialized hash still displays
`2026-04-28` immediately after finalization. This may simply roll forward on
the first active 2026-04-29 shadow/MR event, but it is worth checking again
after the first fresh session bars.

## SessionGap USDRUBF

No new trade lifecycle appeared after the prior clean 2026-04-28 entry/exit.

Latest broker snapshot tail:

```text
USDRUBF position = no open futures position in latest tail
latest futures order = previous filled sell exit
latest futures trade = previous sell fill @ 75.08
```

The runtime returned to `LiveReady` at:

```text
2026-04-29 09:00:07 MSK
```

Verdict: штатно / flat before the new session.

## Alor-USDRUBF Hybrid

No fresh USDRUBF orders or trades appeared in the checked window. Latest
position tail showed only RUB cash entries.

The runtime returned to `LiveReady` at:

```text
2026-04-29 09:00:09 MSK
```

Verdict: штатно / idle-flat at the morning checkpoint.

## RI Author41/42 Shadow

RI shadow remained healthy:

```text
gateway ticker = RIM6
model stream = md.bars.RI.10m
stream XLEN = 309
journal rows = 63
runner errors = none in checked window
```

The shadow journal continued to update:

```text
2026-04-28 23:00 MSK  author42_bo time_exit_same_bar_close
2026-04-29 09:00 MSK  author41_mr accepted long, shadow_pnl_points=228.0
```

Verdict: shadow-only contour is collecting data. The journal currently contains
rolling/intraday records, not a finalized-only daily report.

Operational nuance remains unchanged: the RI shadow gateway uses the `7502SN6`
account transport, so its isolated Redis sees account-level IMOEXF snapshots.
This is not RI trading.

## Morning Verdict

```text
sessiongap       flat, LiveReady
hybrid           BO EOD exit confirmed, flat, riskgate ledger updated
alor-usdrubf     flat/idle, LiveReady
RI shadow        healthy, writing journal
VPS resources    normal
```

Follow-up checks:

```text
1. Check hybrid after first fresh 2026-04-29 bars: riskgate current session should roll forward.
2. Continue watching Redis memory; live Redis is now ~62-67% of 1GiB.
3. Review RI shadow after a full day to decide whether finalized-only output is needed.
```

## Safe Redis Trim

Observation / operation time:

```text
2026-04-29 09:39-09:43 MSK
```

Precondition:

```text
sessiongap       flat; latest futures position sample USDRUBF qty=0.0
hybrid           flat; latest futures position sample IMOEXF qty=0.0
alor-usdrubf     flat/idle; latest position tail only RUB cash
```

The first maintenance trim was run online, without stopping containers.

Script:

```text
/opt/trading-maintenance/redis_safe_trim.sh
```

Guardrails:

```text
mode supports --dry-run and --apply
only explicit stream whitelist is trimmed
runtime.state.* is protected
runtime.riskgate.* is protected
no FLUSHDB / key deletion
```

Whitelist limits:

```text
events.health       -> 10000
broker.snapshots.*  -> 10000
broker.positions.*  -> 5000
broker.orders.*     -> 5000
broker.trades.*     -> 5000
cmd.orders.*        -> 5000
cmd.acks.*          -> 5000
md.bars.*           -> 3000
```

Dry-run showed that only these oversized streams required trimming:

```text
events.health
broker.snapshots.*
```

The first apply used approximate `XTRIM MAXLEN ~`, which was too soft and only
trimmed in large chunks. The script was then corrected to exact
`XTRIM MAXLEN =` and re-run.

Final memory after exact trim:

```text
sessiongap redis      637.6MiB -> 88.9MiB
hybrid redis          659.6MiB -> 94.1MiB
alor-usdrubf redis    685.2MiB -> 87.9MiB
```

Final stream lengths:

```text
sessiongap events.health          ~10004
sessiongap broker.snapshots       ~10003
hybrid events.health              ~10002
hybrid broker.snapshots           ~10003
alor-usdrubf events.health        ~10002
alor-usdrubf broker.snapshots     10000
```

Protected keys remained present:

```text
runtime.state.session_gap_standalone.live.7502MIW
runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502SN6
runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U
runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120 = 182 rows
```

Post-trim checks:

```text
all live containers remained healthy
no recent ERROR / panic / Connection refused / NOGROUP / xreadgroup failed
bars/orders/trades/positions streams preserved
```

Verdict: online safe trim succeeded. It materially reduced Redis memory without
touching runtime state or riskgate history.

Follow-up:

```text
1. Keep the script as manual maintenance until one more observation confirms no side effects.
2. Then add a nightly timer/cron for the same whitelist trim, preferably outside active trading hours.
3. Keep weekend cleanup as a separate manual audit, not an automatic broad prune.
```
