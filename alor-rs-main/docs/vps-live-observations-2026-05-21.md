# VPS Live Observations - 2026-05-21

## Health check

Collection window: 2026-05-21 10:06 MSK.

VPS resources:

- Uptime: 39 days, 18 hours.
- Load average: 0.23 / 0.30 / 0.28.
- RAM: 7.7 GiB total, 2.0 GiB used, 5.7 GiB available.
- Swap: 3.9 GiB total, 103 MiB used.
- Disk `/`: 79 GiB total, 32 GiB used, 44 GiB available, 42%.

Docker resource snapshot:

- Docker images: 9 total / 9 active, 316.8 MiB total, 152.6 MiB reclaimable.
- Containers: 15 total / 15 active.
- Docker volumes/build cache: none.
- All main runtime, gateway, and Redis containers are running.
- Main live runtime/gateway containers report healthy status.

Redis memory snapshot:

- `sessiongap`: 176.99 MiB used, 624.86 MiB peak.
- `hybrid`: 177.18 MiB used, 648.18 MiB peak.
- `alor-USDRUBF`: 148.55 MiB used, 667.84 MiB peak.
- `RI micro`: 119.22 MiB used, 494.45 MiB peak, 512 MiB configured Redis maxmemory.
- `RI shadow`: 195.78 MiB used, 330.94 MiB peak, 512 MiB configured Redis maxmemory.

Interpretation:

- Resource pressure is low.
- Redis memory is materially below the earlier high-water marks after the cleanup work.
- `RI micro` still has a historical peak close to its 512 MiB maxmemory, but current usage is safe.

## Riskgate follow-up

The 2026-05-20 watch item was to confirm that hybrid riskgate advances after the next regular IMOEXF session events.

Current riskgate state:

- `last_finalized_session_date=2026-05-20`.
- `ledger_rows_count=197`.
- `rolling_sum_lb120=171.60000000000014`.
- `mr_enabled_current_session=true`.
- `mr_enabled_next_session=true`.

Recent ledger rows:

- 2026-05-20: finalized with `shadow_pnl_points=0.0`, `shadow_trade_count=0`.
- 2026-05-19: finalized with `shadow_pnl_points=6.8`, `shadow_trade_count=2`.
- 2026-05-18: finalized with `shadow_pnl_points=0.0`, `shadow_trade_count=0`.

Interpretation:

- The delayed bar-driven finalization path worked as expected.
- The 2026-05-19 and 2026-05-20 rows are now in the canonical Redis ledger.
- MR remains enabled for the current and next session.

## Current state

At 10:06 MSK, all main live contours are flat.

- `sessiongap`: runtime `phase=Flat`, `traded_session=false` for 2026-05-21, latest sampled broker futures qty is 0.0.
- `alor-USDRUBF`: runtime `hybrid_state=flat`, `open_position_qty=0.0`, no pending request ids or tracked order ids, latest sampled `USDRUBF` broker qty is 0.0.
- `hybrid IMOEXF`: runtime `last_position_qty=0.0`, no current owner/side, no pending entry/exit/protective ids, latest sampled `IMOEXF` broker qty is 0.0, `broker.stop_orders.7502SN6` latest query is empty.
- `RI author41/42 micro`: runtime `phase=flat`, no current component/side/cycle, no pending entry/exit request ids, latest sampled `RTS-6.26` broker qty is 0.0.
- `RI shadow`: runner has no WARN/ERROR signatures in the checked window; gateway has only reconnect noise.

Note: the `7502MIW` broker order/position streams are shared at portfolio level, so RI orders can be visible in the sessiongap Redis broker streams. Attribution should be based on runtime state and order comments, not only the raw broker stream key.

## 2026-05-20 session summary

### Sessiongap

Observed lifecycle:

- 14:00 MSK: entered `USDRUBF` long, qty 1, fill 71.53.
- 23:30 MSK: exited via sell, qty 1, fill 71.44.
- Runtime transitioned back to `Flat`.
- No runtime WARN/ERROR, command reject, or orphan trade was observed for this cycle.

Interpretation:

- Sessiongap finished flat.
- The trade was slightly adverse before commissions, but lifecycle was clean.

### Alor-USDRUBF

Observed lifecycle:

- 11:10 MSK: breakout long entry accepted, qty 1, fill 71.05.
- 23:40 MSK: `bo_eod_exit` sell accepted, qty 1, fill 71.40.
- Runtime transitioned back to `broker_position_flat`.
- No command reject or transport-path anomaly was observed.

Interpretation:

- Alor-USDRUBF finished flat and the trade was directionally favorable before commissions.
- The marketable/action-scoped execution path remained healthy.

### Hybrid IMOEXF

Observed lifecycle:

- 09:10 MSK: riskgate finalized the 2026-05-19 session row.
- 12:10 MSK: IntradayBreakout short entry accepted, qty 2, fill 2634.0.
- 23:40 MSK: Breakout EOD buy exit accepted, qty 2, fill 2631.5.
- Runtime finished flat with no pending entry/exit/protective request ids.

Interpretation:

- Hybrid size-2 BO path worked cleanly and ended flat.
- The trade was directionally favorable before commissions.
- No protective-order cleanup issue was observed for the 2026-05-20 BO cycle because this branch does not install TP/SL brackets.

### RI author41/42 micro

Observed lifecycle:

- 09:10 MSK: `author41_mr` short entry accepted, qty 1, fill 119910.
- 09:50 MSK: `author41_mr` buy exit accepted, qty 1, fill 119360.
- 11:10 MSK: `author42_bo` short entry accepted, qty 1, fill 117710.
- 23:10 MSK: `author42_bo` buy exit accepted, qty 1, fill 116370.
- Runtime ended `phase=flat`, no pending entry/exit request ids.
- One `orphan_trade` warning was logged on the BO exit before the later command ack mapped the same broker order id.

Interpretation:

- RI finished flat and both MR and BO cycles were directionally favorable before commissions.
- No repeat of the earlier insufficient-funds rejects was observed.
- The `orphan_trade` is classified as non-blocking fill-before-ack ordering noise because the later ack converged and broker position is flat.

## 2026-05-21 morning partial session

### Sessiongap

State at 10:06 MSK:

- No entry yet for 2026-05-21.
- Runtime remains `Flat`.

Interpretation:

- This is normal at this checkpoint; no position risk.

### Alor-USDRUBF

Observed lifecycle:

- 09:50 MSK: MR short entry accepted, qty 1, fill 71.59.
- 10:00 MSK: `mr_take` buy exit accepted, qty 1, fill 71.28.
- Runtime transitioned `open_to_flat`.
- One `orphan_trade` warning was logged on the exit fill before command ack.

Interpretation:

- The morning MR cycle is closed and favorable before commissions.
- The raw order record prices (`65.1`, `76.8`) are not fill prices; execution-confirmed `exec_price` should be used for economics.
- Current broker and runtime states are flat.

### Hybrid IMOEXF

Observed lifecycle:

- 09:10 MSK: riskgate finalized the 2026-05-20 session row.
- 09:10 MSK: MR long bracket entry accepted, qty 2, fill 2631.0.
- 09:24-09:26 MSK: TP filled in two partial fills, total qty 2, fill 2632.5; stop cleanup accepted.
- 09:40 MSK: MR long bracket entry accepted, qty 2, fill 2630.5.
- 09:41 MSK: protective SL path closed the position; runtime logged one `orphan_trade` for the sell fill.
- 09:50 MSK: MR long bracket entry accepted, qty 2, fill 2630.0.
- 10:00 MSK: TP sell exit accepted/filled, qty 2, fill 2631.0.
- 10:00 MSK: cleanup emitted both cancel/delete-stop style commands. The plain order cancel returned `cws_http_400` / `Order to cancel not found`, while `delete_stop_limit` was accepted.

Current state:

- Broker `IMOEXF` qty is 0.0.
- Runtime has no pending entry/exit/protective request ids.
- `broker.stop_orders.7502SN6` latest query is empty.

Interpretation:

- Position state is clean and there is no uncontrolled stop order visible after the cleanup.
- The `Order to cancel not found` appears to be duplicate/late cleanup against an already-filled/canceled TP-side order, not a position incident.
- Keep this as a service-hardening watch item because the runtime also logged `cleanup_ack_error_with_active_stop_while_flat`; current state confirms the residual stop was removed by the accepted `delete_stop_limit`.

Broker ledger reconciliation:

- 09:10 MSK: buy 2 `IMOEXF` at 2631.0.
- 09:24 MSK: sell 1 `IMOEXF` at 2632.5.
- 09:26 MSK: sell 1 `IMOEXF` at 2632.5.
- 09:40 MSK: buy 2 `IMOEXF` at 2630.5.
- 09:41 MSK: sell 2 `IMOEXF` at 2629.0.
- 09:50 MSK: buy 2 `IMOEXF` at 2630.0.
- 10:00 MSK: sell 2 `IMOEXF` at 2631.0.

Engineering note:

- This is the first clearly observed size-2 partial TP case in the current soak.
- The first 09:24 partial fill did not cause a premature flat transition; the second 09:26 fill completed the TP and only then the stop cleanup path completed.
- The 10:00 `Order to cancel not found` reject looks more like cleanup idempotency/stale live-order cleanup after a fully filled TP, not a partial-fill failure.
- Candidate hardening: treat cleanup-side `Order to cancel not found` as benign/idempotent when broker position is already flat and the paired `delete_stop_limit` succeeds, and make the log wording distinguish stale TP cancel from genuinely active stop cleanup failure.

### RI author41/42 micro

Observed lifecycle:

- 09:10 MSK: `author41_mr` long entry accepted, qty 1, fill 116390.
- 10:00 MSK: `author41_mr` sell exit accepted, qty 1, fill 116550.
- Runtime transitioned back to `phase=flat`.
- One `orphan_trade` warning was logged on the exit fill before command ack.

Interpretation:

- The morning MR cycle is closed and favorable before commissions.
- No insufficient-funds reject was observed.
- Current broker and runtime states are flat.

## Error and warning review

Log window: 2026-05-20 08:50 MSK through 2026-05-21 10:06 MSK.

Runtime:

- `sessiongap`: no WARN/ERROR, no command rejects, no orphan trades.
- `alor-USDRUBF`: no command rejects; one `orphan_trade` on the 2026-05-21 MR exit fill, later converged to flat.
- `hybrid IMOEXF`: one `orphan_trade` on the 2026-05-21 MR SL/protective exit; one cleanup reject `cws_http_400` / `Order to cancel not found` after a later TP exit, with state and broker confirmed flat afterward.
- `RI micro`: two `orphan_trade` warnings in the checked window, one on the 2026-05-20 BO exit and one on the 2026-05-21 MR exit; both later converged to accepted command/state flat.
- `RI shadow runner`: no WARN/ERROR.

Gateway:

- Gateways logged recurring off-session websocket/CWS reconnect warnings: TLS EOF and `protocol_reset_without_close_handshake`.
- Several position `AckTimeout` warnings appeared during the overnight reconnect/sync window.
- Checked CWS transport failures had no live `request_id`, `cws_guid`, or `order_id` in flight.
- No `broken pipe`, `Connection refused`, panic, live command timeout, or insufficient-funds reject was observed.

Interpretation:

- Trading state is clean across the live contours at the checkpoint.
- Main watch items are service-observability and cleanup semantics, not uncontrolled position risk.

## Economics snapshot

Economics refresh window: 2026-05-20 08:50 MSK through 2026-05-21 14:20 MSK.

Method:

- Use live runtime `execution_confirmed` fills plus `orphan_trade` fills where a trade arrived before ack and no matching `execution_confirmed` was emitted.
- The window starts and ends with the main contours flat, so cycle pairing is reliable for this slice.
- Gross result is reported as price points/contracts before fees.
- Runtime logged commissions are shown separately. For cycles with `orphan_trade` fills, logged commission is incomplete because orphan lines do not include commission.
- Do not read the gross points minus commission as final RUB PnL without applying the correct instrument multiplier.

Summary:

- `sessiongap`: 1 closed cycle, gross `-0.09` USDRUBF price-points/contracts, logged commission `6.58`.
- `alor-USDRUBF`: 3 closed cycles, gross `+0.74` USDRUBF price-points/contracts, logged commission `16.39` with one orphan exit fill.
- `hybrid IMOEXF`: 4 closed cycles, gross `+7.0` IMOEXF points/contracts, logged commission `24.44` with one orphan SL/protective exit fill.
- `RI author41/42 micro`: 4 closed cycles, gross `+2320` RTS points/contracts, logged commission `66.36` with two orphan exit fills.

Cycle detail:

- `sessiongap`: 2026-05-20 14:00 -> 23:30 MSK, long `USDRUBF` 1, 71.53 -> 71.44, gross `-0.09`.
- `alor-USDRUBF`: 2026-05-20 11:10 -> 23:40 MSK, long 1, 71.05 -> 71.40, gross `+0.35`.
- `alor-USDRUBF`: 2026-05-21 09:50 -> 10:00 MSK, short 1, 71.59 -> 71.28, gross `+0.31`; exit fill was `orphan_trade`.
- `alor-USDRUBF`: 2026-05-21 11:10 -> 12:00 MSK, short 1, 71.03 -> 70.95, gross `+0.08`.
- `hybrid IMOEXF`: 2026-05-20 12:10 -> 23:40 MSK, short 2, 2634.0 -> 2631.5, gross `+5.0`.
- `hybrid IMOEXF`: 2026-05-21 09:10 -> 09:26 MSK, long 2, 2631.0 -> 2632.5, gross `+3.0`; TP filled as two partial fills.
- `hybrid IMOEXF`: 2026-05-21 09:40 -> 09:41 MSK, long 2, 2630.5 -> 2629.0, gross `-3.0`; exit fill was `orphan_trade`.
- `hybrid IMOEXF`: 2026-05-21 09:50 -> 10:00 MSK, long 2, 2630.0 -> 2631.0, gross `+2.0`.
- `RI micro`: 2026-05-20 09:10 -> 09:50 MSK, short 1, 119910 -> 119360, gross `+550`.
- `RI micro`: 2026-05-20 11:10 -> 23:10 MSK, short 1, 117710 -> 116370, gross `+1340`; exit fill was `orphan_trade`.
- `RI micro`: 2026-05-21 09:10 -> 10:00 MSK, long 1, 116390 -> 116550, gross `+160`; exit fill was `orphan_trade`.
- `RI micro`: 2026-05-21 10:10 -> 10:20 MSK, short 1, 116620 -> 116350, gross `+270`.

Interpretation:

- The strongest recent economics are RI micro and hybrid IMOEXF.
- Hybrid IMOEXF remains positive in this slice even with one SL/protective loss; size-2 behavior is economically useful but should stay at qty 2 until cleanup idempotency is watched or patched.
- Alor-USDRUBF is positive but smaller in absolute price movement.
- Sessiongap had one slightly negative long cycle in this slice; no conclusion from one cycle.

## Watch list

- Continue tracking hybrid cleanup behavior after size-2 bracket TP/SL cycles. Current state is clean, but `cleanup_ack_error_with_active_stop_while_flat` should not become noisy or mask a real stale stop.
- Hybrid IMOEXF service-hardening candidate: size-2 partial TP worked correctly, but flat cleanup is not yet perfectly idempotent. Watch for repeated `Order to cancel not found` after TP fills, delayed/failed `delete_stop_limit`, `stop_order_active_while_flat`, or any non-empty `broker.stop_orders.7502SN6` after runtime/broker flat. Do not increase hybrid above qty 2 until this has either stayed clean for several sessions or is patched.
- Continue tracking fill-before-ack `orphan_trade` frequency across alor-USDRUBF, hybrid, and RI. Latest cases converged correctly.
- Continue monitoring RI insufficient-funds rejects; no repeat was observed in this window.
- Continue daily Redis memory checks. Current memory is safe; no immediate cleanup is required.
