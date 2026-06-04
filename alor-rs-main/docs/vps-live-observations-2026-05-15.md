# VPS Live Observations - 2026-05-15

## Morning health check

Collection window: 2026-05-15 09:21-09:24 MSK.

VPS resources remain healthy:

- Uptime: 33 days, 17 hours.
- Load average: 0.43 / 0.38 / 0.30.
- RAM: 7.7 GiB total, 1.9 GiB used, 5.8 GiB available.
- Swap: 3.9 GiB total, 79 MiB used.
- Disk `/`: 79 GiB total, 38 GiB used, 37 GiB available, 51%.

Docker memory snapshot:

- `sessiongap-redis-1`: 155.2 MiB / 1 GiB.
- `hybrid-redis-1`: 161.0 MiB / 1 GiB.
- `alor-usdrubf-redis-1`: 145.2 MiB / 1 GiB.
- `ri-author41-42-redis-1`: 94.16 MiB / 768 MiB.
- `ri-shadow-redis-1`: 180.7 MiB / 768 MiB.
- Strategy runtimes and gateways are small; no memory/disk pressure observed.

All main containers are up. Live strategy runtimes are healthy.

## Current live status

- `sessiongap`: own `USDRUBF` state is flat; no own trade yet today. Shared `7502MIW` broker stream shows current `RTS-6.26` short, but this belongs to RI.
- `hybrid IMOEXF`: flat; no live trade yet today. Riskgate is current: `last_finalized_session_date=2026-05-14`, `ledger_rows_count=193`, `rolling_sum_lb120=182.9`, `mr_enabled_current_session=true`.
- `alor-USDRUBF`: flat; no live trade yet today.
- `RI author41/42`: active `author41_mr` short position is open.

Current RI position:

- Component: `author41_mr`.
- Side: short.
- Instrument: `RTS-6.26`.
- Quantity: 1.
- Entry: sell 1 @ 114540.0 at 2026-05-15 09:10:07 MSK.
- Commission: 11.04.
- Command request: `a354347b-eb76-5c02-ba84-19dd11c43645`.
- Broker response: accepted, CWS HTTP 200, broker order id `1925039844266888398`.
- Runtime state: `live_in_position`, `current_cycle_id=author41_mr:20260515090000`, no pending request ids stuck.

Observation: RI runtime logged one `orphan_trade` warning for this entry fill. Redis state and broker position both show the fill was absorbed into `live_in_position`; this is treated as a non-blocking event-order/observability race unless it repeats or blocks exit.

## 2026-05-14 completed session recap

Note: `sessiongap` and RI share portfolio/account `7502MIW`, so `USDRUBF` trades are sessiongap and `RTS-6.26` trades are RI.

### SessionGap USDRUBF

Observed cycle:

- Entry command: sell 1 `USDRUBF` limit @ 73.19, accepted through CWS HTTP 200.
- Entry fill: sell 1 @ 73.20 at 2026-05-14 15:00:00 MSK, commission 3.39.
- Exit command: buy 1 `USDRUBF` limit @ 73.20, accepted through CWS HTTP 200.
- Exit fill: buy 1 @ 73.20 at 2026-05-14 23:30:02 MSK, commission 3.39.
- Gross result: 0.00 price points before commission.
- End state: flat.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- Both command acknowledgements were accepted; no pending state remained.

### Hybrid IMOEXF

Observed cycle:

- BO entry: short 2 `IMOEXF` @ 2670.0 at 2026-05-14 15:20:04 MSK, commission 3.56.
- BO EOD exit: buy 2 `IMOEXF` @ 2653.5 at 2026-05-14 23:40:28 MSK, commission 3.56.
- Gross result: +16.5 points per contract, +33.0 total before commission.
- End state: flat, no pending request ids.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- Size-2 BO lifecycle was clean: entry accepted, EOD exit accepted, broker-flat confirmed.
- Riskgate session ledger finalized 2026-05-14 with `shadow_pnl_points=0.0`, `shadow_trade_count=0`, `rolling_sum_lb120=182.9`, `mr_enabled_next_session=true`.

### Alor-USDRUBF

Observed cycle:

- BO entry: market sell 1 `USDRUBF` @ 73.36 at 2026-05-14 11:10:14 MSK, commission 3.39.
- BO EOD exit: market buy 1 `USDRUBF` @ 73.19 at 2026-05-14 23:40:00 MSK, commission 3.39.
- Gross result: +0.17 price points before commission.
- End state: flat.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- Market entry/exit path stayed clean; no stuck pending state observed.

### RI author41/42

Observed cycles:

- Cycle 1: MR long, buy 1 `RTS-6.26` @ 115400.0 at 09:20:05 MSK, exit sell 1 @ 115580.0 at 10:30:00 MSK, gross +180 points before commission.
- Cycle 2: MR short, sell 1 `RTS-6.26` @ 115560.0 at 11:00:03 MSK, exit buy 1 @ 115330.0 at 12:40:03 MSK, gross +230 points before commission.
- Total gross result: +410 points before commission.
- End state: flat.

Runtime observations:

- `ERROR=0`, `command_rejected=0`.
- All RI commands were accepted through `action_scoped_only`.
- No stuck pending request ids observed after completed cycles.

## Warnings and anomalies

- Gateway warnings across contours were mostly CWS/WS reconnects (`protocol_reset_without_close_handshake`, `eof`, TLS close-notify EOF) with `pending_count=0`.
- RI gateway had transient `AckTimeout for 7502MIW (positions)` during early morning reconnect/sync; later live order was accepted and broker position stream was current.
- Only material runtime warning in the current window is the RI `orphan_trade` on the 2026-05-15 entry fill; current state confirms the position is tracked.

## Watch list

- Monitor current RI short until planned exit and broker-flat confirmation.
- If RI `orphan_trade` appears again on the exit or leaves state inconsistent, treat as an incident candidate; otherwise keep it in service-observability bucket.
- Continue hybrid IMOEXF size-2 observation for more clean sessions before any further lot-size discussion.
- Redis and disk are healthy; no cleanup required now.

## Follow-up captured on 2026-05-16

The active RI `author41_mr` short noted in the morning check closed normally:

- Exit: buy 1 `RTS-6.26` @ 114590.0 at 2026-05-15 15:50:17 MSK.
- Exit request: `8582a196-5b66-586e-8b5e-9351f7e84ac6`.
- Broker response: accepted, CWS HTTP 200, broker order id `1925039844267362233`.
- Gross result for the RI cycle: -50 points before commission.
- RI end-of-day and 2026-05-16 check state: flat, no pending request ids.

Full 2026-05-15 completed-session recap is recorded in `vps-live-observations-2026-05-16.md`.
