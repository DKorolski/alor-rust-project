# VPS Live Observations - 2026-05-14

## Morning health check

Collection window: 2026-05-14 09:40-09:45 MSK.

VPS resources remain healthy:

- Uptime: 32 days.
- Load average: 0.09 / 0.24 / 0.26.
- RAM: 7.7 GiB total, 2.0 GiB used, 5.7 GiB available.
- Swap: 3.9 GiB total, 79 MiB used.
- Disk `/`: 79 GiB total, 38 GiB used, 37 GiB available, 51%.

Docker memory snapshot:

- `alor-usdrubf-redis-1`: 152.2 MiB / 1 GiB.
- `hybrid-redis-1`: 226.2 MiB / 1 GiB.
- `ri-redis-1`: 92.39 MiB / 768 MiB.
- `sessiongap-redis-1`: 160.6 MiB / 1 GiB.
- Strategy runtimes and gateways remain small; no resource-pressure signal observed.

## Current live status

All four live contours are running and healthy.

- `sessiongap`: flat for its own `USDRUBF` state; no own commands or trades emitted today. The shared `7502MIW` account stream contains an open `RTS-6.26` position, but this belongs to the RI contour, not sessiongap.
- `hybrid IMOEXF`: flat; no commands/trades today. Riskgate state is current with `last_finalized_session_date=2026-05-13`, `ledger_rows_count=192`, `rolling_sum_lb120=182.9`, `mr_enabled_current_session=true`.
- `alor-USDRUBF`: flat; no commands/trades today.
- `RI author41/42`: live MR long position is active. Entry was accepted through the action-scoped path.

Current RI position:

- Component: `author41_mr`.
- Side: long.
- Instrument: `RTS-6.26`.
- Quantity: 1.
- Entry: buy 1 @ 115400.0 at 2026-05-14 09:20:05 MSK.
- Commission: 11.20.
- Command request: `b96520a4-abde-594e-b98d-068cd0bd2a93`.
- Broker response: accepted, CWS HTTP 200, broker order id `1925039839971926333`.
- Runtime phase: `live_in_position`, no pending request ids stuck.

## 2026-05-13 completed session recap

Note: `sessiongap` and `RI` share portfolio/account `7502MIW`, so broker stream rows must be attributed by instrument and order comment. `USDRUBF` rows belong to sessiongap; `RTS-6.26` rows belong to RI.

### SessionGap USDRUBF

Observed cycle:

- Entry command: sell 1 `USDRUBF` limit @ 73.33, accepted through CWS HTTP 200.
- Entry fill: sell 1 @ 73.34 at 2026-05-13 12:00:00 MSK, commission 3.41.
- Exit command: buy 1 `USDRUBF` limit @ 73.34, accepted through CWS HTTP 200.
- Exit fill: buy 1 @ 73.33 at 2026-05-13 12:10:05 MSK, commission 3.41.
- Gross result: +0.01 price points before commission.
- End state: flat.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`.
- One `orphan_trade` was logged on the exit fill, but it transitioned `PendingExit -> Flat`; this looks like a non-blocking event-order race rather than an uncontrolled position.

### Hybrid IMOEXF

Observed cycle:

- MR entry: buy 2 `IMOEXF` @ 2691.0, commission 3.54.
- Protective TP was installed: sell 2 @ 2694.0, broker id `2033126226733838508`.
- Protective SL was installed: stop sell 2, trigger 2670.0, price 2669.5, broker id `120648054`.
- MR exit: sell 2 @ 2683.5, commission 3.54.
- Cleanup: TP cancel accepted; SL stop-limit delete accepted.
- Gross result: -15.0 points total before commission.
- End state: flat, no pending request ids.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- The increased size-2 contour completed the full entry/protection/exit/cleanup lifecycle.

### Alor-USDRUBF

Observed cycle:

- Entry: market sell 1 `USDRUBF` @ 73.56, commission 3.41.
- Exit: market buy 1 `USDRUBF` @ 73.66, commission 3.41.
- Gross result: -0.10 price points before commission.
- End state: flat.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- Command path stayed clean; no stuck pending state observed.

### RI author41/42

Observed cycle:

- At 09:10 MSK, one intended RI entry was dropped by `intent_dropped_bar_silence` because the latest model bar was stale.
- Entry was emitted on the next eligible path: buy 1 `RTS-6.26` @ 115310.0, commission 11.22.
- Exit: sell 1 `RTS-6.26` @ 115460.0, commission 11.22.
- Gross result: +150 points before commission.
- End state: flat.

Runtime observations:

- `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `dropped_silence=1` acted as a guardrail before a later valid entry; not treated as an incident.

## Watch list

- Monitor the active 2026-05-14 RI `author41_mr` long position until planned exit and broker-flat confirmation.
- Continue observing hybrid IMOEXF at size 2 for at least several clean sessions before any further size discussion.
- Redis usage is within configured limits, but hybrid/sessiongap streams should remain on the routine trim watch list.
- Keep distinguishing service-level observability events (`orphan_trade` event-order races, `dropped_bar_silence` guards) from actual uncontrolled-position incidents.

## Follow-up captured on 2026-05-15

The active RI `author41_mr` long noted in the morning check closed normally:

- Exit: sell 1 `RTS-6.26` @ 115580.0 at 2026-05-14 10:30:00 MSK.
- Gross result for the first RI cycle: +180 points before commission.
- A second RI MR cycle also completed on 2026-05-14: short 1 @ 115560.0, exit buy 1 @ 115330.0, gross +230 points before commission.
- RI end-of-day state: flat.

Full 2026-05-14 completed-session recap is recorded in `vps-live-observations-2026-05-15.md`.
