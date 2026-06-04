# VPS Live Observations - 2026-05-19

## Morning health check

Collection window: 2026-05-19 08:55-08:58 MSK.

VPS resources:

- Uptime: 37 days, 17 hours.
- Load average: 0.62 / 0.44 / 0.35.
- RAM: 7.7 GiB total, 1.9 GiB used, 5.8 GiB available.
- Swap: 3.9 GiB total, 108 MiB used.
- Disk `/`: 79 GiB total, 32 GiB used, 44 GiB available, 42%.

Docker resource snapshot:

- Docker images: 9 total / 9 active, 316.8 MiB total, 152.6 MiB reclaimable.
- Containers: 15 total / 15 active.
- Docker volumes: none.
- All main live runtime, gateway, and Redis containers are running.
- Main live runtime/gateway containers report healthy status.

Redis memory snapshot:

- `sessiongap`: 159.25 MiB used, 624.86 MiB peak.
- `hybrid`: 150.70 MiB used, 648.18 MiB peak.
- `alor-USDRUBF`: 136.08 MiB used, 667.84 MiB peak.
- `RI micro`: 102.41 MiB used, 494.45 MiB peak, 512 MiB configured Redis maxmemory.
- `RI shadow`: 186.95 MiB used, 330.94 MiB peak, 512 MiB configured Redis maxmemory.

Follow-up CPU snapshot:

- A first one-shot `docker stats` sample showed a transient `hybrid-redis` CPU spike.
- A repeat sample shortly after showed `hybrid-redis` back at 0.77% CPU.
- Interpretation: no sustained Redis CPU pressure was observed.

## Current state before 2026-05-19 open

All main live contours are flat before the 2026-05-19 regular session open.

- `sessiongap`: runtime `phase=Flat`, `traded_session=true` for 2026-05-18, latest sampled `USDRUBF` broker qty is 0.0.
- `hybrid IMOEXF`: runtime `last_position_qty=0.0`, no current owner/side, no pending entry/exit/protective ids, latest sampled `IMOEXF` broker qty is 0.0.
- `alor-USDRUBF`: runtime `hybrid_state=flat`, `open_position_qty=0.0`, no pending request ids or tracked order ids, latest sampled `USDRUBF` broker qty is 0.0.
- `RI author41/42 micro`: runtime `phase=flat`, no current component/side/cycle, no pending entry/exit request ids, latest sampled `RTS-6.26` broker qty is 0.0.
- `RI shadow`: shadow gateway is running; separate shadow Redis has only market/gateway streams and no live runtime state source of truth.

## 2026-05-18 session summary

### Sessiongap

Observed lifecycle:

- 12:00 MSK: entered `USDRUBF` short, qty 1, fill 72.48.
- 15:00 MSK: exited via buy, qty 1, fill 72.32.
- Runtime transitioned `Flat -> PendingEntry -> InPosition -> PendingExit -> Flat`.
- No rejects, orphan trades, or pending-state residue observed.

Interpretation:

- Sessiongap trade lifecycle was clean.
- The trade was directionally favorable before commissions.

### Alor-USDRUBF

Observed lifecycle:

- 11:10 MSK: breakout short entry accepted, qty 1, fill 72.56.
- 23:40 MSK: `bo_eod_exit` buy accepted, qty 1, fill 71.93.
- Runtime transitioned back to `broker_position_flat`.
- `entry_intent_inflight=false`, `exit_intent_inflight=false`, no pending request ids after close.

Interpretation:

- Alor-USDRUBF worked through the action-scoped market path and finished flat.
- The trade was directionally favorable before commissions.

### Hybrid IMOEXF

Observed lifecycle:

- 09:10 MSK: riskgate finalized the 2026-05-15 session row.
- Riskgate advanced to `last_finalized_session_date=2026-05-15`, `ledger_rows_count=194`, `rolling_sum_lb120=180.00000000000017`.
- 18:00 MSK: BO long entry accepted, qty 2, fill 2661.5.
- 23:40 MSK: BO EOD exit accepted, qty 2, fill 2668.5.
- Runtime ended flat with no pending entry/exit/protective request ids.

Interpretation:

- The post-watch riskgate item from 2026-05-18 was resolved for the 2026-05-15 row.
- The first observed size-2 hybrid IMOEXF BO cycle was clean and directionally favorable before commissions.
- The 2026-05-18 riskgate session row has not yet been finalized before the 2026-05-19 open, which is expected for the bar-driven finalize path.

### RI author41/42 micro

Observed lifecycle:

- 09:10 MSK: first MR long intent was dropped before emit by trading-window guard; runtime reverted state.
- 09:20 MSK: MR short entry accepted, qty 1, fill 114010.
- 10:30 MSK: MR exit accepted, qty 1, fill 113870.
- 13:10 MSK: BO long entry was rejected with `cws_http_400`, message `Нехватка средств по лимитам клиента.`, and rolled back.
- 14:10 MSK: BO long intent was dropped before emit by trading-window guard.
- 14:50 MSK: BO long entry was rejected again with the same insufficient-funds message and rolled back.
- 15:10 MSK: BO long entry was accepted, qty 1, fill 115680. A trade event arrived before ack mapping and was logged as `orphan_trade`, then the command ack arrived for the same order id.
- 23:10 MSK: BO exit accepted, qty 1, fill 116930.
- Runtime ended `phase=flat`, no pending entry/exit request ids, latest `RTS-6.26` broker qty 0.0.

Interpretation:

- RI finished flat and did not leave a live position or pending request.
- The sequence is operationally important: two broker rejects for insufficient funds plus one fill-before-ack `orphan_trade`.
- This should remain a RI micro watch item before any promotion or size discussion.

## Error and warning review

Log window: 2026-05-18 07:40 MSK through 2026-05-19 08:58 MSK.

Runtime:

- `sessiongap`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `alor-USDRUBF`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `hybrid IMOEXF`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `RI micro`: `ERROR=0`, `WARN=3`, `command_rejected=2`, `orphan_trade=1`.
- `RI shadow runner`: `ERROR=0`, `WARN=0`.

Gateway:

- Gateways had the recurring off-session websocket/CWS reconnect warnings: TLS EOF and `protocol_reset_without_close_handshake`.
- All checked CWS transport failures had `pending_count=0`, no `request_id`, no `cws_guid`, and no `order_id` in flight.
- No `broken pipe`, `Connection refused`, panic, or live command timeout was observed.

Interpretation:

- For sessiongap, alor-USDRUBF, and hybrid IMOEXF, the 2026-05-18 session was clean.
- RI micro had a clean final state but a non-clean intraday operational path due to insufficient-funds rejects and one orphan trade event.

## Watch list

- Re-check hybrid riskgate after the first regular 2026-05-19 IMOEXF bars; expected next advancement is finalizing the 2026-05-18 row.
- Track RI insufficient-funds rejects. If repeated, add a pre-send cash/margin guard or reduce/disable conflicting RI BO retries.
- Track RI `orphan_trade` if it repeats; current case appears to be fill-before-ack ordering because the later ack mapped to the same broker order id and the system ended flat.
- Continue normal Redis memory checks; no immediate Redis cleanup is needed.
