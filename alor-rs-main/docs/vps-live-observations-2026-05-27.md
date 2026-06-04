# VPS Live Observations - 2026-05-27

Observation timestamp: 2026-05-27 09:30-09:55 MSK.

## Scope

Checked active VPS live contours after the corporate handoff preparation:

- `alor-USDRUBF` / `USDRUBF` / portfolio `7502MIW`
- `RI author41/42 micro` / `RTS-6.26` order symbol, `RIM6` model feed / portfolio `7502MIW`

Inactive / intentionally stopped:

- `sessiongap` / `USDRUBF` / portfolio `7502T0U`
- `hybrid IMOEXF` / `IMOEXF` / portfolio `7502T0U`

Reason: `7502T0U` is being prepared for the corporate mirror rollout, so the old VPS live contours on that portfolio were stopped after broker-flat checks.

## VPS Health

VPS:

- Host: `nektodk.ispvds.com`
- Check time: `Wed May 27 09:30:41 MSK 2026`

Resources:

| Metric | Value | Read |
| --- | ---: | --- |
| RAM | 7.7 GiB total, 1.3 GiB used, 6.4 GiB available | Healthy |
| Swap | 3.9 GiB total, 63 MiB used | Low usage |
| Disk `/` | 79 GiB total, 32 GiB used, 44 GiB available, 43% used | Healthy |

Active containers:

- `trading-alor-usdrubf-strategy-runtime-1`: Up 25 hours, healthy.
- `trading-alor-usdrubf-alor-gateway-1`: Up 25 hours, healthy.
- `trading-alor-usdrubf-redis-1`: Up 25 hours, healthy.
- `trading-ri-author41-42-7502miw-strategy-runtime-1`: Up 2 weeks, healthy.
- `trading-ri-author41-42-7502miw-alor-gateway-1`: Up 2 weeks, healthy.
- `trading-ri-author41-42-7502miw-redis-1`: Up 3 weeks, healthy.

Redis/container memory snapshot:

| Container | Current memory | Redis used memory | Peak | Read |
| --- | ---: | ---: | ---: | --- |
| `trading-alor-usdrubf-redis-1` | 111.3 MiB / 1 GiB | 107.29 MiB | 107.30 MiB | Healthy |
| `trading-ri-author41-42-7502miw-redis-1` | 151.4 MiB / 768 MiB | 144.52 MiB | 494.45 MiB | Healthy now; keep watching peak near 512 MiB cap |

Interpretation:

- No CPU/RAM/disk pressure was observed.
- RI Redis current memory is safe after previous trim/retention work, but the recorded peak is close to the configured 512 MiB Redis maxmemory. Continue weekly Redis checks and safe trim.

## Current State Check

### Alor-USDRUBF

Runtime state:

- Portfolio: `7502MIW`.
- Strategy state: `hybrid_state="flat"`.
- `open_position_qty=0.0`.
- `pending_request_ids=[]`.
- `tracked_order_ids=[]`.
- `entry_intent_inflight=false`.
- `exit_intent_inflight=false`.
- Last processed live bars are from 2026-05-27 morning session.

Broker stream:

- Latest `USDRUBF` position event: `qty=0.0`.
- No active position or pending lifecycle tail was visible.

Morning readiness:

- Runtime reached `LiveReady / ALLOWED` at `2026-05-27 09:00:06 MSK`.
- No new live order was emitted after the 2026-05-27 open as of the checkpoint.

Interpretation:

- The system is flat and ready.
- No stale pending entry/exit state is visible.

### RI Author41/42 Micro

Runtime state:

- Portfolio: `7502MIW`.
- `phase="flat"`.
- `current_component=null`.
- `current_side=null`.
- `pending_entry_request_id=null`.
- `pending_exit_request_id=null`.
- `last_transition_reason="live_position_flat_confirmed"`.

Broker stream:

- Latest `RTS-6.26` position event: `qty=0.0`.
- No working RI position was visible.

Morning readiness:

- Runtime reached `LiveReady / ALLOWED` at `2026-05-27 09:00:08 MSK`.
- No new live order was emitted after the 2026-05-27 open as of the checkpoint.

Interpretation:

- The RI micro contour is flat and ready.
- No stale pending request or manual-intervention state was visible.

## 2026-05-26 Session Review

### Alor-USDRUBF

Observed cycle:

| Local time | Event | Details |
| --- | --- | --- |
| 11:00 MSK | Signal | `bo_short_signal`, owner `day_breakout_waitfix`, signal price `71.99` |
| 11:10 MSK | Entry intent | `market`, side `Sell`, qty `1`, request `628a387d-27ec-5401-add5-adcda051aa08` |
| 11:10 MSK | Entry fill | sell `1 USDRUBF` @ `72.02`, order id `2023556116180532001`, commission `3.31` |
| 23:40 MSK | Exit intent | `bo_eod_exit`, side `Buy`, qty `1`, request `443dbb40-ebc3-580c-ad8c-68649ece5147` |
| 23:40 MSK | Exit fill | buy `1 USDRUBF` @ `71.30`, order id `2023556116180907629`, commission `3.31` |

Execution path:

- Gateway config resolved `control_cws_mode="action_scoped"`.
- Both commands returned `status=Accepted`, HTTP `200`.
- No reject, timeout, insufficient-funds, or CWS error path was observed.

Economics:

- Short entry `72.02`, exit `71.30`.
- Gross: `+0.72` USDRUBF price points, i.e. `+72` ticks before commission.
- Total logged commission: `6.62`.

Interpretation:

- Cycle looks logically consistent: breakout short, broker-confirmed entry, no overnight carry, EOD flatten.
- Exit was logged at 23:40 MSK because the runtime acts on the 10-minute event stream; it still closed in the evening session and did not carry overnight.

### RI Author41/42 Micro

Observed cycles:

| Local time | Event | Details |
| --- | --- | --- |
| 09:10 MSK | Entry | `author41_mr` short, sell `1 RTS-6.26` @ `113230`, order id `1925039874331669090` |
| 11:10 MSK | Exit | buy `1 RTS-6.26` @ `112760`, order id `1925039874332015962` |
| 11:20 MSK | Entry | `author41_mr` long, buy `1 RTS-6.26` @ `112840`, order id `1925039874332049892` |
| 11:30 MSK | Exit | sell `1 RTS-6.26` @ `113030`, order id `1925039874332075366` |

Execution path:

- All emitted intents used `execution_path="action_scoped_only"`.
- All commands returned `status=Accepted`, HTTP `200`.
- No reject, timeout, insufficient-funds, manual-intervention, or deferred-exit anomaly was observed.

Economics:

- Cycle 1: short `113230 -> 112760`, gross `+470` RTS points.
- Cycle 2: long `112840 -> 113030`, gross `+190` RTS points.
- Total gross: `+660` RTS points before commission.
- Total logged commission for the four fills: `42.80`.

Interpretation:

- RI completed two clean micro-live MR cycles on 2026-05-26.
- The full path remained action-scoped and broker-confirmed.
- End state returned to flat.

## Warning And Error Review

Window: latest 24h and 2026-05-27 morning checkpoint.

Runtime:

- `alor-USDRUBF`: no WARN/ERROR in the checked window.
- `RI author41/42 micro`: no WARN/ERROR in the checked window.

Gateway:

- Observed overnight/off-session websocket/CWS reconnect warnings:
  - EOF / TLS `close_notify` absent.
  - `protocol_reset_without_close_handshake`.
- These warnings had `pending_count=0`, `request_id=None`, `cws_guid=None`, `order_id=None`.
- No command loss or broker-side reject was visible.

Interpretation:

- Warnings look like the already-known off-session transport churn.
- No live order failure, action-scoped regression, or uncontrolled position risk was observed.

## Current Read

| System | Current state | 2026-05-27 morning action | Read |
| --- | --- | --- | --- |
| `alor-USDRUBF` | Flat | No order emitted as of checkpoint | Normal |
| `RI author41/42 micro` | Flat | No order emitted as of checkpoint | Normal |
| `sessiongap` | Stopped intentionally | N/A | Expected |
| `hybrid IMOEXF` | Stopped intentionally | N/A | Expected |

## Watch List

- Continue watching RI Redis memory; current value is safe, but peak memory was close to the 512 MiB cap.
- Continue weekly Redis maintenance with whitelist trim only; no broad `FLUSHALL`.
- Continue checking that RI and alor-USDRUBF stay on action-scoped execution paths after any redeploy.
- Confirm corporate handoff branch review before re-enabling any `7502T0U` contour on another environment.

## 2026-05-27 Hybrid IMOEXF Re-Enable On 7502MIW

Context:

- Decision: re-enable `hybrid IMOEXF` on the main `7502MIW` portfolio to observe three-system portfolio behavior together with `RI author41/42 micro` and `alor-USDRUBF`.
- `sessiongap` remains stopped intentionally.
- `7502T0U` remains reserved for corporate mirror preparation.

Deployment:

- Runtime config: `/configs/runtime.hybrid.live.7502MIW.riskgate-shadow.toml`.
- Gateway config: `/configs/gateway.hybrid.live.7502MIW.action-scoped.toml`.
- Portfolio: `7502MIW`.
- Symbol: `IMOEXF`.
- Quantity: `2`.
- Gateway resolved `control_cws_mode="action_scoped"`.
- Runtime resolved profile `imoexf_primary_riskgate_high180_lb120`, MR variant `high180`, gate policy `shadow_pnl_lb120_positive`.

Riskgate ledger:

- Startup mode: `NormalAppend`.
- Decision: `UseExistingLedger`.
- Existing records loaded: `199`.
- Records inserted from seed on startup: `0`.
- `seed_loaded=true`.
- `last_finalized_session_date=2026-05-22`.
- `rolling_sum_lb120=151.80000000000015`.
- `mr_enabled_current_session=true`.
- `mr_enabled_next_session=true`.

Startup and health:

- Containers started healthy:
  - `trading-hybrid-alor-gateway-1`
  - `trading-hybrid-strategy-runtime-1`
  - `trading-hybrid-redis-1`
- Runtime bootstrap filtered positions/orders:
  - `positions_open_strategy=0`.
  - `orders_open_strategy=0`.
  - `stop_orders_open_strategy=0`.
- Runtime reached `LiveReady / ALLOWED` at `2026-05-27 11:00:07 MSK`.
- `cmd.orders.7502MIW` and `cmd.acks.7502MIW` in the hybrid Redis had no emitted commands at the checkpoint.
- No hybrid WARN/ERROR/reject was observed after startup.

Current hybrid state:

- Live position: flat.
- `last_position_qty=0`.
- `current_owner=null`.
- `pending_entry_request_id=null`.
- `pending_exit_request_id=null`.
- Riskgate shadow accounting is active for `2026-05-27`; this is virtual gate state, not a broker position.

Portfolio read after re-enable:

- `RI author41/42 micro`: in `author41_mr` short position, broker-confirmed; one `orphan_trade` ordering warning remains in watchlist because the trade event arrived before request-map/ack reconciliation.
- `alor-USDRUBF`: no open position; state has a pending BO-short signal, no live broker position at checkpoint.
- `hybrid IMOEXF`: flat and ready.

Resource snapshot:

- `trading-ri-author41-42-7502miw-redis-1`: about `164.8 MiB / 768 MiB`.
- `trading-alor-usdrubf-redis-1`: about `118.9 MiB / 1 GiB`.
- `trading-hybrid-redis-1`: about `89.1 MiB / 1 GiB`.
- Runtime/gateway CPU load was low across all three systems.

Interpretation:

- Re-enable looks technically clean.
- The existing riskgate ledger was reused and the seed did not overwrite live history.
- Hybrid did not adopt RI/USDRUBF broker state as its own strategy state.
- Continue observing first IMOEXF live intent, riskgate session finalization, and RI orphan-trade ordering behavior.

## Extended Live Micro Soak Portfolio Mode

Decision:

- Continue extended live micro soak in one-portfolio mode on `7502MIW`.
- Active systems:
  - `RI author41/42 micro` on `RTS-6.26`.
  - `alor-USDRUBF` on `USDRUBF`.
  - `hybrid IMOEXF` on `IMOEXF`.
- `sessiongap` remains stopped and outside this portfolio-mode soak.

Operating assumptions:

- Instruments are different, so strategy-level order streams and broker positions should not conflict when symbol filtering works correctly.
- All live order emission must remain on the hardened action-scoped/fresh-control execution paths.
- Portfolio-level risk is now shared, so insufficient-funds / margin messages must be interpreted across all three instruments together, not per strategy in isolation.

Watchlist for the combined soak:

- Confirm that every strategy continues to filter broker snapshots/orders/trades by its own symbol and does not adopt another strategy's position.
- Watch available funds around overlapping entries, especially if RI is already in position when USDRUBF or IMOEXF emits an entry.
- Track `orphan_trade` ordering warnings separately from real uncontrolled-position incidents.
- Confirm `hybrid IMOEXF` riskgate ledger finalizes the next session row correctly after re-enable on `7502MIW`.
- Keep weekly Redis/resource maintenance active; do not let the one-portfolio soak hide per-stack Redis growth.

Current status:

- One-portfolio soak is active from the 2026-05-27 mid-session re-enable checkpoint.
- Initial read is clean: `RI` broker-confirmed in position, `alor-USDRUBF` no open position, `hybrid IMOEXF` flat and ready.

## 2026-05-27 All-In-Position Portfolio Checkpoint

Checkpoint: about `11:43 MSK`.

Broker snapshot on `7502MIW`:

- `RTS-6.26`: `qty=-1`, avg price `113840.0`.
- `USDRUBF`: `qty=-1`, avg price `71.03`.
- `IMOEXF`: `qty=2`, avg price `2573.0`.
- `RUB`: about `98051.44`.

Runtime state:

- `RI author41/42 micro`: `live_in_position`, component `author41_mr`, side `short`, current cycle `author41_mr:20260527113000`, no pending entry/exit request.
- `alor-USDRUBF`: `broker_position_open`, owner `day_breakout_waitfix`, side `short`, qty `1`, entry price `71.03`, no exit intent in flight.
- `hybrid IMOEXF`: active cycle `6a16ab8800`, owner `mean_reversion`, side `long`, qty `2`, no pending entry/exit request.

Execution path and broker confirmations:

- `RI`:
  - First short entry at `10:40 MSK`: sell `1 RTS-6.26` @ `113980.0`, accepted via action-scoped path.
  - Exit at `11:30 MSK`: buy `1 RTS-6.26` @ `113600.0`, accepted.
  - New short entry at `11:40 MSK`: sell `1 RTS-6.26` @ `113840.0`, accepted via action-scoped path.
- `alor-USDRUBF`:
  - Short entry at `11:10 MSK`: sell `1 USDRUBF` @ `71.03`, accepted.
- `hybrid IMOEXF`:
  - MR long entry at `11:40 MSK`: buy `2 IMOEXF` @ `2573.0`, accepted.
  - TP working order: sell `2 IMOEXF` limit @ `2575.5`, order id `2033126269683596942`.
  - SL working stop order: sell `2 IMOEXF`, stop price `2553.5`, limit price `2553.0`, stop order id `121558304`.

Warnings/errors:

- No `Rejected`, insufficient-funds, timeout, or gateway-level error was observed for the simultaneous-position checkpoint.
- The only WARN in the checked window was the known RI `orphan_trade` ordering event on the earlier `10:40 MSK` entry; the command ack arrived accepted and the broker position was later confirmed.

Interpretation:

- This is the first observed one-portfolio checkpoint where all three live micro systems are simultaneously in broker positions.
- Strategy-level symbol filtering still looks correct: each runtime owns only its own instrument state.
- `hybrid IMOEXF` MR position is protected by broker-visible TP and SL orders.
- Continue watching portfolio-level free funds/margin while all three systems are open at the same time.

## 2026-05-27 Portfolio Flat After All-In-Position Checkpoint

Checkpoint: about `12:01 MSK`.

Broker snapshot:

- `RTS-6.26`: `qty=0`, avg price `0`.
- `USDRUBF`: `qty=0`, avg price `0`.
- `IMOEXF`: `qty=0`, avg price `0`.
- `RUB`: about `98084.45`.

Runtime state:

- `RI author41/42 micro`: `phase="flat"`, `last_transition_reason="live_position_flat_confirmed"`, no pending entry/exit request.
- `alor-USDRUBF`: `hybrid_state="flat"`, `lifecycle_stage="broker_position_flat"`, no pending entry/exit request.
- `hybrid IMOEXF`: `last_position_qty=0`, `active_cycle_id=null`, `current_owner=null`, no pending entry/exit/TP/SL request.

Observed exits:

- `RI`:
  - Exit intent emitted at `11:50 MSK` for the `11:30 MSK` MR short.
  - Buy `1 RTS-6.26` accepted and filled @ `113780.0`.
  - Result for this cycle: short `113840.0 -> 113780.0`, gross `+60` RTS points before commission.
- `alor-USDRUBF`:
  - Exit intent emitted at `12:00 MSK`, reason `bo_stop1_short`.
  - Buy `1 USDRUBF` accepted and filled @ `71.37`.
  - Result for this cycle: short `71.03 -> 71.37`, gross `-0.34` USDRUBF price points before commission.
- `hybrid IMOEXF`:
  - MR TP order filled: sell `2 IMOEXF` @ `2575.5`.
  - SL stop order `121558304` canceled after TP fill.
  - Result for this cycle: long `2573.0 -> 2575.5`, gross `+2.5` IMOEXF price points per contract before commission.

Warnings/errors:

- No rejected command, insufficient-funds error, uncontrolled position, or stale protective order was observed.
- `USDRUBF` produced one `orphan_trade` warning on the exit fill before the ack/order map caught up. The command ack arrived `Accepted`, and broker flat was confirmed.
- The earlier RI `orphan_trade` ordering warning remains in the same watch class.

Interpretation:

- The first all-in-position one-portfolio cycle completed and returned to flat across all three instruments.
- `hybrid IMOEXF` bracket behavior was clean: entry filled, TP filled, SL canceled.
- The main watch item is now not position control, but ordering/noise class: `orphan_trade` can appear when trade events arrive before command ack reconciliation.
