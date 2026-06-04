# VPS Live Observations - 2026-05-18

## Pre-open health check

Collection window: 2026-05-18 07:42 MSK.

VPS resources:

- Uptime: 36 days, 16 hours.
- Load average: 0.29 / 0.26 / 0.27.
- RAM: 7.7 GiB total, 1.8 GiB used, 5.9 GiB available.
- Swap: 3.9 GiB total, 120 MiB used.
- Disk `/`: 79 GiB total, 31 GiB used, 45 GiB available, 41%.

Docker resource snapshot:

- Docker images: 9 total / 9 active, 316.8 MiB total, 152.6 MiB reclaimable.
- Containers: 15 total / 15 active.
- Docker volumes: none.
- All main runtime, gateway, and Redis containers are running.
- Main live runtime/gateway containers report healthy status.

Redis memory snapshot:

- `sessiongap`: 130.38 MiB used, 624.86 MiB peak.
- `hybrid`: 131.66 MiB used, 648.18 MiB peak.
- `alor-USDRUBF`: 115.65 MiB used, 667.84 MiB peak.
- `RI micro`: 76.58 MiB used, 494.45 MiB peak, 512 MiB configured Redis maxmemory.
- `RI shadow`: 178.11 MiB used, 330.94 MiB peak, 512 MiB configured Redis maxmemory.

Interpretation:

- Resource pressure is low after the 2026-05-17 local build artifact and Docker image cache cleanup.
- Disk pressure remains comfortable at 41%.
- Redis memory is materially below recent peaks.

## Runtime and broker-state check

All live strategy contours are flat before the 2026-05-18 regular session open.

- `sessiongap`: runtime state `phase=Flat`; latest state is still 2026-05-15, no new session bar yet.
- `hybrid IMOEXF`: runtime flat, `last_position_qty=0.0`, all pending entry/exit/protective request ids are null, TP/SL ids are null.
- `alor-USDRUBF`: runtime `hybrid_state=flat`, `open_position_qty=0.0`, no pending request ids, no tracked order ids.
- `RI author41/42 micro`: runtime `phase=flat`, no current component/side/cycle, `pending_entry_request_id=null`, `pending_exit_request_id=null`.
- `RI shadow`: runtime is flat, dry-run only, no live adapter enabled.

Latest broker position streams observed only RUB cash position updates in the pre-open reconnect window. No futures position was present in the latest sampled position payloads for the live contours.

## Log review

Log window: since 2026-05-18 00:00 MSK.

Runtime counters:

- `sessiongap` runtime: `ERROR=0`, `WARN=0`, `command_rejected=0`, no live commands/trades.
- `hybrid` runtime: `ERROR=0`, `WARN=0`, `command_rejected=0`, no live commands/trades.
- `alor-USDRUBF` runtime: `ERROR=0`, `WARN=0`, `command_rejected=0`, no live commands/trades.
- `RI micro` runtime: `ERROR=0`, `WARN=0`, `command_rejected=0`, no live commands/trades.
- `RI shadow` runtime: `ERROR=0`, `WARN=0`.

Gateway observations:

- Gateways had overnight/pre-open reconnect noise: TLS EOF, `protocol_reset_without_close_handshake`, and `Connection reset by peer`.
- CWS transport failures had `pending_count=0`, no `request_id`, no `cws_guid`, and no `order_id` in flight.
- No `AckTimeout`, command reject, orphan trade, panic, broken pipe, or connection refused was observed in the checked window.

Interpretation:

- The warning pattern is consistent with idle/off-session Alor websocket/CWS reconnect behavior.
- No trading command was in flight during the reconnects.
- No live-path anomaly is open from the pre-open check.

## Hybrid riskgate watch

Riskgate state at 2026-05-18 07:42 MSK:

- `seed_loaded=true`.
- `ledger_rows_count=193`.
- `last_finalized_session_date=2026-05-14`.
- `current_shadow_session_date=2026-05-09`.
- `rolling_sum_lb120=182.90000000000015`.
- `mr_enabled_current_session=true`.
- `mr_enabled_next_session=true`.

Recent ledger rows:

- 2026-05-14: `shadow_pnl_points=0.0`, `shadow_trade_count=0`, `rolling_sum_lb120=182.9`, complete.
- 2026-05-13: `shadow_pnl_points=-7.6`, `shadow_trade_count=1`, complete.
- 2026-05-12: `shadow_pnl_points=-19.1`, `shadow_trade_count=1`, complete.

Interpretation:

- The 2026-05-15 hybrid riskgate ledger row has still not finalized at this pre-open check.
- This remains a watch item, not an incident, because the check was taken before the first regular 2026-05-18 session event.
- Verify after the first regular IMOEXF bars that the bar-driven finalize path advances the ledger from 2026-05-14.

## Watch list

- Re-check hybrid riskgate after the 2026-05-18 regular session starts; expected outcome is ledger advancement beyond 2026-05-14.
- Continue monitoring gateway reconnect warnings during the session; they are benign only while no request is in flight.
- Continue regular Redis memory checks, especially `RI shadow`, but no memory-pressure action is needed at this point.
