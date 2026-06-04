# VPS Live Observations - 2026-05-22

Observation timestamp: 2026-05-22 09:46 MSK.

## Scope

Checked active VPS live/shadow contours:

- `sessiongap` / `USDRUBF` / portfolio `7502MIW`
- `alor-USDRUBF` / `USDRUBF` / portfolio `7502T0U`
- `hybrid IMOEXF` / `IMOEXF` / portfolio `7502SN6`
- `RI author41/42 micro` / `RTS-6.26` order symbol, `RIM6` model feed / portfolio `7502MIW`
- `RI shadow`

Note: `sessiongap` and `RI author41/42 micro` share portfolio `7502MIW`, so raw broker streams are portfolio-level. Attribution must be done by `symbol`, strategy comment, runtime state, and command stream, not only by `broker.trades.7502MIW`.

## VPS Health

VPS:

- Host: `nektodk.ispvds.com`
- Uptime: `40 days, 18:18`
- Load average: `0.19, 0.26, 0.26`

Resources:

| Metric | Value | Read |
| --- | ---: | --- |
| RAM | 7.7 GiB total, 2.1 GiB used, 5.6 GiB available | Healthy |
| Swap | 3.9 GiB total, 101 MiB used | Low usage |
| Disk `/` | 79 GiB total, 32 GiB used, 44 GiB available, 42% used | Healthy |

Container status:

- All active live containers are `Up` and runtime/gateway containers report `healthy`.
- `sessiongap` runtime has been up 4 weeks.
- `alor-USDRUBF` runtime has been up 3 weeks.
- `hybrid IMOEXF` runtime has been up 12 days.
- `RI author41/42 micro` runtime has been up 2 weeks.

Redis/container memory snapshot:

| Container | Memory |
| --- | ---: |
| `trading-sessiongap-redis-1` | 183 MiB / 1 GiB |
| `trading-alor-usdrubf-redis-1` | 187.8 MiB / 1 GiB |
| `trading-hybrid-redis-1` | 350.3 MiB / 1 GiB |
| `trading-ri-author41-42-7502miw-redis-1` | 137.2 MiB / 768 MiB |
| `trading-ri-shadow-redis-1` | 214.6 MiB / 768 MiB |

Interpretation:

- No resource pressure was observed.
- `hybrid` Redis remains the largest live Redis at roughly 350 MiB, but still safely below the 1 GiB limit.
- No immediate cleanup is required from this checkpoint.

## Current State Check

Latest runtime/broker state snapshots:

### SessionGap

- Runtime state: `phase="Flat"`, `traded_session=false`.
- Current session: `2026-05-22`.
- Latest broker stream on shared `7502MIW` shows `RTS-6.26 qty=0.0` and RUB cash rows. This is expected because the portfolio is shared with RI.
- No `USDRUBF` open position was observed in runtime state.
- No fills or warnings were logged for `sessiongap` since 2026-05-22 00:00 MSK.

Interpretation:

- `sessiongap` is idle/flat.
- No trade today is consistent with its low-frequency behavior.

### Alor-USDRUBF

- Runtime state: `hybrid_state="flat"`.
- `open_position_qty=0.0`.
- No pending request ids or tracked order ids.
- `bo_was_long_today=false`, `bo_was_short_today=false`.
- No fills or warnings were logged since 2026-05-22 00:00 MSK.

Interpretation:

- `alor-USDRUBF` is clean and flat at the checkpoint.

### Hybrid IMOEXF

- Runtime state: flat.
- `last_position_qty=0.0`.
- No `pending_entry_request_id`, `pending_exit_request_id`, `tp_order_id`, `sl_stop_order_id`, or `sl_exchange_order_id`.
- Riskgate state:
  - `risk_gate_mr_enabled_current_session=true`
  - `risk_gate_rolling_sum_lb120=155.50000000000014`
  - `risk_gate_last_finalized_session_date=2026-05-21`
  - `risk_gate_ledger_rows_count=198`
  - current 2026-05-22 shadow session PnL is `0.0`, trade count `0`
- No fills or warnings were logged since 2026-05-22 00:00 MSK.

Interpretation:

- Hybrid is flat and ready.
- Riskgate ledger finalized the 2026-05-21 row and advanced from 197 to 198 rows.
- No stale protective order ids are visible in runtime state.

### RI Author41/42 Micro

Runtime state:

- `phase="flat"`
- `current_component=null`, `current_side=null`, `current_cycle_id=null`
- no pending entry/exit request ids
- `last_transition_reason="live_position_flat_confirmed"`
- broker position stream shows `RTS-6.26 qty=0.0`

Today’s live cycle:

- 09:10:03 MSK: emitted `author41_mr` entry, model side short, order side sell, qty 1, `execution_path="action_scoped_only"`.
- 09:10:03 MSK: execution confirmed, sell 1 `RTS-6.26` @ `117570.0`, commission `11.06`.
- 09:40:36 MSK: emitted `author41_mr` exit, order side buy, qty 1, `execution_path="action_scoped_only"`.
- 09:40:37 MSK: execution confirmed, buy 1 `RTS-6.26` @ `117460.0`, commission `11.06`.
- Gross: short `+110` RTS points before commission.

Gateway path:

- Both entry and exit went through action-scoped CWS sessions.
- Both CWS commands returned HTTP 200 and broker order ids:
  - entry: `1925039865741727949`
  - exit: `1925039865741768489`
- No `orphan_trade`, command reject, timeout, or insufficient-funds event was observed for today’s RI cycle.

Interpretation:

- RI completed a clean full micro-live path today.
- The cycle is favorable before commission.
- This is another useful positive observation for the RI micro-live soak.

### RI Shadow

- No live fills by design.
- No warnings/errors were observed in the checked window.

## Gateway And Transport Review

Observed gateway warnings:

- Recurring overnight/off-session websocket reconnects:
  - TLS EOF / `peer closed connection without sending TLS close_notify`
  - `protocol_reset_without_close_handshake`
- These appeared across sessiongap, alor-USDRUBF, hybrid, RI micro, and RI shadow gateway containers.
- The CWS transport warnings had:
  - `pending_count=0`
  - `request_id=None`
  - `cws_guid=None`
  - `order_id=None`

Morning readiness:

- All live gateways reached supervisor readiness around 06:00 MSK:
  - `bars_live_seen=true`
  - `positions_synced=true`
  - `orders_synced=true`
  - `stop_orders_synced=true`
  - `cws_authorization=true`

Interpretation:

- The reconnect warnings are operational noise from off-session transport churn, not live order failures.
- No evidence of command loss, command reject, CWS path regression, or uncontrolled position risk was observed.

## Error And Warning Summary

Window: 2026-05-22 00:00 MSK through 09:46 MSK.

Runtime:

- `sessiongap`: no WARN/ERROR, no fills.
- `alor-USDRUBF`: no WARN/ERROR, no fills.
- `hybrid IMOEXF`: no WARN/ERROR, no fills.
- `RI author41/42 micro`: no WARN/ERROR; one clean entry/exit cycle.
- `RI shadow`: no WARN/ERROR.

Gateway:

- Off-session reconnect warnings only.
- No `broken pipe`, `Connection refused`, panic, command reject, insufficient-funds reject, action-scoped CWS error path, or order-cleanup error was observed.

## Economics Snapshot

Today through 09:46 MSK:

| System | Closed cycles | Gross before commission | Notes |
| --- | ---: | ---: | --- |
| `sessiongap` | 0 | `0` | No trade today. |
| `alor-USDRUBF` | 0 | `0` | No trade today. |
| `hybrid IMOEXF` | 0 | `0` | No trade today. |
| `RI author41/42 micro` | 1 | `+110` RTS points | Clean short MR cycle, commission `22.12` total logged. |

## Broker Ledger Reconciliation

Raw broker export was saved in `docs/broker-ledger-2026-05-22-raw.md`.

Derived reconciliation is in `docs/broker-ledger-reconciliation-2026-05-22.md`.

Key results:

- Today’s broker ledger contains only the RI `RTS-6.26` cycle on `7502MIW`.
- Broker ledger exactly matches runtime for RI:
  - 09:10 MSK sell 1 @ `117570`
  - 09:40 MSK buy 1 @ `117460`
  - gross `+110` RTS points
- No broker-side `USDRUBF`, `IMOEXF`, or `7502T0U` trade was found for 2026-05-22 at the checked export point.
- Broker FIFO pairing ends with no open lots across parsed portfolios.

Correction to the previous economics checkpoint:

- The 2026-05-21 runtime economics review was captured at 14:20 MSK and therefore did not include a later `hybrid IMOEXF` evening cycle.
- Broker ledger and runtime logs confirm an additional 2026-05-21 `hybrid IMOEXF` cycle:
  - 20:20 MSK buy 2 @ `2654.5`
  - 21:00 MSK sell 2 @ `2651.0`
  - gross `-7.0`
- Current broker-view `hybrid IMOEXF` qty2 contour result from 2026-05-09 through the broker export is therefore `+30.0` points/contracts, not `+37.0`.

## Watch List

- Continue watching RI micro for 3-5 more clean sessions before any status/lot-size discussion.
- Continue watching hybrid IMOEXF size-2 cleanup idempotency after future TP/SL cycles; no new cleanup issue appeared today.
- Continue differentiating `7502MIW` shared broker stream events by symbol:
  - `USDRUBF` belongs to `sessiongap`.
  - `RTS-6.26` belongs to `RI author41/42 micro`.
- Continue daily Redis memory checks. Current Redis usage is safe.
