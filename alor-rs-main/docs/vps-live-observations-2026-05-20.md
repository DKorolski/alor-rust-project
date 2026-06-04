# VPS Live Observations - 2026-05-20

## Morning health check

Collection window: 2026-05-20 08:51-08:52 MSK.

VPS resources:

- Uptime: 38 days, 17 hours.
- Load average: 0.18 / 0.22 / 0.28.
- RAM: 7.7 GiB total, 1.9 GiB used, 5.8 GiB available.
- Swap: 3.9 GiB total, 105 MiB used.
- Disk `/`: 79 GiB total, 32 GiB used, 44 GiB available, 42%.

Docker resource snapshot:

- Docker images: 9 total / 9 active, 316.8 MiB total, 152.6 MiB reclaimable.
- Containers: 15 total / 15 active.
- Docker volumes: none.
- All main live runtime, gateway, and Redis containers are running.
- Main live runtime/gateway containers report healthy status.

Redis memory snapshot:

- `sessiongap`: 163.77 MiB used, 624.86 MiB peak.
- `hybrid`: 243.03 MiB used, 648.18 MiB peak.
- `alor-USDRUBF`: 141.39 MiB used, 667.84 MiB peak.
- `RI micro`: 109.21 MiB used, 494.45 MiB peak, 512 MiB configured Redis maxmemory.
- `RI shadow`: 186.55 MiB used, 330.94 MiB peak, 512 MiB configured Redis maxmemory.

Interpretation:

- Resource pressure remains low.
- `hybrid` Redis grew versus the prior morning but is still well below its previous peak and the container limit.
- No Redis cleanup is needed from this check.

## Weekend watch follow-up

The weekend note asked us to verify that the bar-driven hybrid riskgate ledger finalizes delayed session rows after the next regular session event.

Current riskgate state:

- `last_finalized_session_date=2026-05-18`.
- `ledger_rows_count=195`.
- `rolling_sum_lb120=180.00000000000017`.
- `mr_enabled_current_session=true`.
- `mr_enabled_next_session=true`.

Recent ledger rows:

- 2026-05-18: finalized with `shadow_pnl_points=0.0`, `shadow_trade_count=0`.
- 2026-05-15: finalized with `shadow_pnl_points=0.0`, `shadow_trade_count=0`.
- 2026-05-14: finalized with `shadow_pnl_points=0.0`, `shadow_trade_count=0`.

Interpretation:

- The weekend watch item is resolved through the 2026-05-18 row.
- The 2026-05-19 row was not finalized yet at 08:52 MSK on 2026-05-20, which is expected because the check was before the first regular 09:00/09:10 session event.
- Re-check after the first 2026-05-20 regular IMOEXF bars that the 2026-05-19 row advances.

## Current state before 2026-05-20 open

All main live contours are flat before the 2026-05-20 regular session open.

- `sessiongap`: runtime `phase=Flat`, `traded_session=true` for 2026-05-19, latest sampled `USDRUBF` broker qty is 0.0.
- `hybrid IMOEXF`: runtime `last_position_qty=0.0`, no current owner/side, no pending entry/exit/protective ids, latest sampled `IMOEXF` broker qty is 0.0.
- `alor-USDRUBF`: runtime `hybrid_state=flat`, `open_position_qty=0.0`, no pending request ids or tracked order ids, latest sampled `USDRUBF` broker qty is 0.0.
- `RI author41/42 micro`: runtime `phase=flat`, no current component/side/cycle, no pending entry/exit request ids, latest sampled `RTS-6.26` broker qty is 0.0.
- `RI shadow`: runner has `ERROR=0`, `WARN=0`; gateway has only reconnect noise.

## 2026-05-19 session summary

### Sessiongap

Observed lifecycle:

- 12:00 MSK: entered `USDRUBF` short, qty 1, fill 71.26.
- 18:20 MSK: exited via buy, qty 1, fill 70.94.
- Runtime transitioned back to `Flat`.
- One `orphan_trade` warning was logged on the entry fill before the later command ack mapped the order.

Interpretation:

- Sessiongap finished flat and the trade was directionally favorable before commissions.
- The `orphan_trade` is classified as non-blocking fill-before-ack ordering noise unless it repeats with stale state.

### Alor-USDRUBF

Observed lifecycle:

- 11:10 MSK: breakout short entry accepted, qty 1, fill 71.35.
- 23:40 MSK: `bo_eod_exit` buy accepted, qty 1, fill 70.56.
- Runtime transitioned back to `broker_position_flat`.
- One `orphan_trade` warning was logged on the EOD exit fill; broker position and runtime converged to flat.

Interpretation:

- Alor-USDRUBF finished flat and the trade was directionally favorable before commissions.
- No command reject or transport-path anomaly was observed.

### Hybrid IMOEXF

Observed lifecycle:

- 09:10 MSK: riskgate finalized the 2026-05-18 session row.
- 09:10 MSK: MR long bracket entry accepted, qty 2, fill 2667.0.
- 11:08 MSK: TP exit filled, qty 2, fill 2669.5; stop order cleanup accepted.
- 11:10 MSK: MR short bracket entry accepted, qty 2, fill 2670.5.
- 12:20 MSK: TP exit filled, qty 2, fill 2666.0; stop order cleanup accepted.
- Runtime ended flat with no pending entry/exit/protective request ids.
- One `orphan_trade` warning was logged on the second MR entry before the later command ack.

Interpretation:

- Hybrid size-2 MR path worked cleanly from a lifecycle perspective and ended flat.
- Protective TP/SL install and stop cleanup paths were exercised successfully.
- The session was directionally favorable before commissions.

### RI author41/42 micro

Observed lifecycle:

- 09:10 MSK: MR long entry accepted, qty 1, fill 116990.
- 09:20 MSK: MR exit accepted, qty 1, fill 117290.
- 09:30 MSK: MR long entry accepted, qty 1, fill 116980.
- 09:50 MSK: MR exit accepted, qty 1, fill 117210.
- 18:10 MSK: BO long entry accepted, qty 1, fill 118830.
- 23:10 MSK: BO exit accepted, qty 1, fill 119470.
- Runtime ended `phase=flat`, no pending entry/exit request ids.
- One `orphan_trade` warning was logged on the BO exit before the later command ack.

Interpretation:

- No repeat of the 2026-05-18 insufficient-funds rejects was observed.
- RI finished flat and the 2026-05-19 trade path was directionally favorable before commissions.
- The repeated fill-before-ack `orphan_trade` pattern remains a service-observability watch item.

## Error and warning review

Log window: 2026-05-19 08:55 MSK through 2026-05-20 08:52 MSK.

Runtime:

- `sessiongap`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=1`.
- `alor-USDRUBF`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=1`.
- `hybrid IMOEXF`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=1`.
- `RI micro`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=1`.
- `RI shadow runner`: `ERROR=0`, `WARN=0`.

Gateway:

- Gateways had recurring off-session websocket/CWS reconnect warnings: TLS EOF and `protocol_reset_without_close_handshake`.
- Sessiongap and hybrid gateways also logged three positions `AckTimeout` warnings during the overnight reconnect/sync window.
- Checked CWS transport failures had `pending_count=0`, no `request_id`, no `cws_guid`, and no `order_id` in flight.
- No `broken pipe`, `Connection refused`, panic, live command timeout, command reject, or insufficient-funds reject was observed.

Interpretation:

- Trading state is clean across all live contours.
- Main open watch item is not position risk, but repeated fill-before-ack `orphan_trade` observability noise.
- Gateway reconnect/AckTimeout warnings remain classified as off-session sync noise because they happened without live command in flight.

## Watch list

- Re-check hybrid riskgate after the first regular 2026-05-20 IMOEXF bars; expected next advancement is finalizing the 2026-05-19 row.
- Continue tracking `orphan_trade` frequency across sessiongap, alor-USDRUBF, hybrid, and RI. Current cases converged to flat, but the recurrence suggests an observability hardening backlog item.
- Continue tracking RI insufficient-funds rejects. No repeat was observed on 2026-05-19.
- Continue regular Redis memory checks; no immediate Redis cleanup is needed.
