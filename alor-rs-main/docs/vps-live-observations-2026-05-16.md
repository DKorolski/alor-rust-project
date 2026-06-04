# VPS Live Observations - 2026-05-16

## Weekend health check

Collection window: 2026-05-16 13:19-13:21 MSK.

VPS resources remain healthy:

- Uptime: 34 days, 21 hours.
- Load average: 0.16 / 0.23 / 0.24.
- RAM: 7.7 GiB total, 2.1 GiB used, 5.7 GiB available.
- Swap: 3.9 GiB total, 79 MiB used.
- Disk `/`: 79 GiB total, 38 GiB used, 37 GiB available, 51%.

Docker memory snapshot:

- `sessiongap-redis-1`: 173.1 MiB / 1 GiB.
- `hybrid-redis-1`: 185.8 MiB / 1 GiB.
- `alor-usdrubf-redis-1`: 174.7 MiB / 1 GiB.
- `ri-author41-42-redis-1`: 122.0 MiB / 768 MiB.
- `ri-shadow-redis-1`: 216.1 MiB / 768 MiB.
- Strategy runtimes and gateways remain small; no memory/disk pressure observed.

All main containers are running. Live strategy runtimes are healthy.

## Current status

All live strategy contours are flat as of the 2026-05-16 Saturday check.

- `sessiongap`: flat, no own commands or trades since 2026-05-15 start.
- `hybrid IMOEXF`: flat, no pending entry/exit/protective orders, no live trade today.
- `alor-USDRUBF`: flat, no pending request ids, no live trade today.
- `RI author41/42`: flat; the 2026-05-15 short cycle closed normally.
- `RI shadow`: runtime has no errors; gateway shows weekend/no-bar reconnect and bar-silence noise.

Latest broker/runtime alignment:

- `IMOEXF`: broker qty 0.0, runtime flat.
- `USDRUBF` on `7502T0U`: broker qty 0.0, runtime flat.
- `RTS-6.26`: broker qty 0.0, RI runtime `phase=flat`.
- `sessiongap` own `USDRUBF`: runtime `phase=Flat`, no Friday trade.

## 2026-05-15 completed session recap

### SessionGap USDRUBF

No own trade on 2026-05-15.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- End state: flat.

### Hybrid IMOEXF

Observed cycle:

- BO entry: short 2 `IMOEXF` @ 2631.0 at 2026-05-15 18:20:06 MSK, commission 3.52.
- BO exit: buy 2 `IMOEXF` @ 2636.0 at 2026-05-15 19:10:04 MSK, commission 3.52.
- Exit reason: `BreakoutStop1Short`.
- Gross result: -5.0 points per contract, -10.0 total before commission.
- End state: flat, no pending request ids.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- Size-2 BO lifecycle was clean: entry accepted, exit accepted, broker-flat confirmed.

Riskgate:

- 2026-05-14 was finalized at 2026-05-15 09:10 MSK with `shadow_pnl_points=0.0`, `shadow_trade_count=0`.
- Current riskgate state: `last_finalized_session_date=2026-05-14`, `ledger_rows_count=193`, `rolling_sum_lb120=182.9`, `mr_enabled_current_session=true`, `mr_enabled_next_session=true`.
- 2026-05-15 riskgate ledger row was not finalized by the Saturday check. This is likely consistent with the current bar-driven/no-timer finalize design, but should be verified on the next regular session.

### Alor-USDRUBF

Observed cycle:

- BO entry: market sell 1 `USDRUBF` @ 73.13 at 2026-05-15 12:40:03 MSK, commission 3.38.
- BO EOD exit: market buy 1 `USDRUBF` @ 73.07 at 2026-05-15 23:40:34 MSK, commission 3.38.
- Gross result: +0.06 price points before commission.
- End state: flat, no pending request ids.

Runtime observations:

- `ERROR=0`, `command_rejected=0`.
- One `orphan_trade` warning was logged on the EOD exit fill. Broker position and runtime state converged to flat, so this is classified as non-blocking event-order/observability noise unless it repeats with stale state.

### RI author41/42

Observed cycle:

- MR entry: short 1 `RTS-6.26` @ 114540.0 at 2026-05-15 09:10:07 MSK, commission 11.04.
- MR exit: buy 1 `RTS-6.26` @ 114590.0 at 2026-05-15 15:50:17 MSK, commission 11.04.
- Exit reason in model decision: `breakeven_limit`.
- Gross result: -50 points before commission.
- End state: flat, no pending request ids.

Runtime observations:

- `ERROR=0`, `command_rejected=0`.
- Entry and exit were accepted through `action_scoped_only`.
- One `orphan_trade` warning was logged on the entry fill, but runtime absorbed the trade into `live_in_position`; exit later executed and broker-flat was confirmed.

## 2026-05-16 weekend observations

- No live strategy commands or executions observed today.
- Gateway warnings are mostly expected reconnect/sync noise: `protocol_reset_without_close_handshake`, `eof`, TLS close-notify EOF, and occasional `AckTimeout` during sync.
- All CWS transport warnings seen today had no active command rejection and no stuck live pending state in runtime.
- `RI shadow` gateway shows repeated `bar silence detected; resubscribing` / `ws subscribe retry exceeded` during the weekend. Runtime itself has `ERROR=0`, `WARN=0`; treat as weekend/no-live-bar observability noise unless it continues into a regular session.

## Watch list

- Verify on the next regular session that hybrid riskgate finalizes the 2026-05-15 session row or intentionally rolls it forward via the bar-driven finalize path.
- Continue watching `orphan_trade` warnings in `alor-USDRUBF` and `RI`; both latest cases converged correctly, but repeated event-order warnings are worth keeping visible.
- Redis usage increased modestly but remains well within limits. No cleanup required right now.
- Continue hybrid IMOEXF size-2 observation before any further lot-size discussion.
