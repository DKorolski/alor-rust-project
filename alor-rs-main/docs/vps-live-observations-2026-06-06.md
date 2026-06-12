# VPS Live Observations - 2026-06-06

## Review Scope

Context:

- VPS: `155.212.170.21`, host `nektodk.ispvds.com`.
- Review time: `2026-06-06 09:13-09:16 MSK`.
- Scope: Friday `2026-06-05` execution review, current broker-flat check,
  runtime/gateway warning scan, risk-gate ledger check, and VPS resource
  maintenance.

Current health:

- All checked runtime, gateway, and Redis containers were `healthy`.
- Fresh runtime scan found no `WARN`, `ERROR`, `rejected`, `failed`,
  `orphan_trade`, or `intent_dropped` events.
- Fresh broker position snapshots showed:
  - `7502MIW`: `IMOEXF qty = 0.0`, `USDRUBF qty = 0.0`.
  - `7502T0U`: `IMOEXF qty = 0.0`.
- No uncontrolled position or protective-order tail was identified.

## Friday Execution Review

### Primary `IMOEXF hybrid riskgate` on `7502MIW`

Observed BO-short cycle:

- Entry signal/action: `2026-06-05 18:10 MSK`, `IntradayBreakout`,
  `BreakoutShort`.
- Entry execution: sell `4` at `2561.0`.
- Exit signal/action: `2026-06-05 20:50 MSK`, `BreakoutStop1Short`.
- Exit execution: buy `4` at `2566.5`.
- Broker position later converged to `IMOEXF qty = 0.0`.

Interpretation:

- Entry and exit commands were accepted and executed without reject/failure.
- This was a BO cycle, so no MR protective TP/SL bracket was expected.
- The cycle was economically negative by `5.5` index points before fees, but
  operationally clean.

### `IMOEXF hybrid author41-short` on `7502T0U`

Observed BO-short cycle:

- Entry signal/action: `2026-06-05 18:10 MSK`, `IntradayBreakout`,
  `BreakoutShort`.
- Entry execution: sell `1` at `2561.0`.
- Exit signal/action: `2026-06-05 20:50 MSK`, `BreakoutStop1Short`.
- Exit execution: buy `1` at `2566.5`.
- Broker position later converged to `IMOEXF qty = 0.0`.

Interpretation:

- The author41-short contour and primary contour behaved symmetrically for the
  shared BO component, with only quantity differing.
- No stale order, partial-fill anomaly, or uncontrolled position tail was
  observed.

### `Alor-USDRUBF` on `7502MIW`

Observed BO-long cycle:

- Signal: `2026-06-05 11:10 MSK`, owner `day_breakout_waitfix`,
  reason `bo_long_signal`.
- Entry: buy `1` at `73.92` around `11:20 MSK`.
- Exit: sell `1` at `73.62` around `12:00 MSK`,
  reason `bo_stop1_long`.
- Broker position later converged to `USDRUBF qty = 0.0`.

Interpretation:

- This was a BO cycle, not an MR cycle. Therefore the absence of MR TP/SL
  bracket orders was expected under the current contract.
- Both entry and exit used the validated action-scoped control path and were
  accepted by the broker.
- The cycle was economically negative by `0.30` price points before fees, but
  operationally clean.

### `RI author41/42` on `7502MIW`

- No fresh RI decision, entry, exit, reject, or incident was found in the
  checked Friday interval.
- Current broker position snapshot did not show an open RI position.

## Warning Review

Gateway warnings:

- The gateways logged intermittent market-data websocket reconnect warnings:
  - connection reset without closing handshake;
  - peer reset / unexpected EOF;
  - isolated ping timeout;
  - a short `ws subscribe retry exceeded` sequence on Alor-USDRUBF.
- No command reject, action-scoped send failure, or execution failure followed
  these reconnect events.
- All gateways and runtimes remained healthy and broker state converged flat.

Kernel/system:

- No kernel OOM or killed-process event was found in the checked `30h` window.

## Risk-Gate Ledger Check

Primary IMOEXF risk-gate state:

- Profile: `imoexf_primary_high180_lb120`.
- `seed_loaded = true`.
- `ledger_rows_count = 205`.
- `rolling_sum_lb120 = 165.40000000000018`.
- `mr_enabled_current_session = true`.
- `mr_enabled_next_session = true`.
- Latest finalized row: `2026-06-04`, `shadow_pnl_points = 0.0`,
  `shadow_trade_count = 0`.

Pending check:

- No finalized risk-gate ledger row for Friday `2026-06-05` was present during
  the Saturday morning check.
- Re-check on the first eligible regular-session event. The current bar-driven
  contract may finalize the previous session when the next regular weekday
  session begins.

## VPS Resources And Safe Trim

Pre-maintenance resource snapshot:

- Host memory: `7.7 GiB` total, about `4.2 GiB` available.
- Swap: about `105 MiB / 3.9 GiB` used.
- Disk `/`: `38G / 79G`, about `51%`.
- Docker images: about `1.9 GB`, with about `1.85 GB` reclaimable.
- After the per-stream gateway canary build, temporary build context and
  dangling image layers were removed. Docker reclaimed about `1.679 GB`;
  the active canary and previous rollback gateway images were preserved.

Redis before trim:

- `trading-hybrid-author41-7502t0u-redis-1`: about `286 MiB / 512 MiB`.
- `trading-hybrid-redis-1`: about `232 MiB / 1 GiB`.
- `trading-alor-usdrubf-redis-1`: about `232 MiB / 1 GiB`.
- `trading-ri-author41-42-7502miw-redis-1`: about `251 MiB / 768 MiB`.

Author41 growth source:

- `events.health.hybrid_author41_short.7502T0U`: about `20.5k` entries.
- `broker.snapshots.7502T0U`: about `10.8k` entries.
- `broker.positions.7502T0U`: about `2.7k` entries.
- Orders/trades/runtime state remained small or protected.

Safe maintenance:

- All checked instruments were broker-flat before maintenance.
- Ran `/opt/maintenance/redis_safe_trim.sh --apply`.
- Protected `runtime.state.*`, `runtime.riskgate.*`, command, order, and trade
  history was not broadly deleted.
- Trading services were not stopped.

Redis after trim:

- `trading-hybrid-author41-7502t0u-redis-1`: about `64 MiB / 512 MiB`.
- `trading-hybrid-redis-1`: about `187 MiB / 1 GiB`.
- `trading-alor-usdrubf-redis-1`: about `183 MiB / 1 GiB`.
- `trading-ri-author41-42-7502miw-redis-1`: about `226 MiB / 768 MiB`.

Post-maintenance:

- All containers remained `healthy`.
- No fresh runtime warning/error appeared after trim.
- Existing timer remains active, with the next scheduled run at
  `2026-06-08 08:10 MSK`.

## Current Read

- Extended micro soak remains operationally acceptable.
- Friday execution paths converged flat without command rejects or stale
  protective-order tails.
- The absence of a bracket on Friday Alor-USDRUBF was expected because the
  trade belonged to BO, while the new bracket contract applies to MR.
- Transient websocket reconnects remain a noisy but convergent gateway class.
- Author41 Redis growth remains an active operations watch item; the current
  Monday-Friday timer leaves a weekend maintenance gap.
- Re-check Friday risk-gate ledger finalization at the next regular weekday
  session start.

## Author41 Source-Side Retention Canary

The author41 contour was broker-flat before the change:

- `IMOEXF = 0`;
- `USDRUBF = 0`;
- `RTS-6.26 = 0`.

Deployed gateway-only image:

- `manual-20260606-perstream-retention`.

Applied source-side stream limits:

- bars: `3000`;
- orders/trades/commands/acks: `5000`;
- positions/snapshots: `2000`;
- health: `1500`.

Validation result:

- gateway, runtime, and Redis remained healthy;
- CWS authorization completed successfully;
- runtime briefly reported stale gateway health during the gateway restart,
  then cleared it after the first fresh heartbeat;
- no runtime from-zero reset was performed;
- health/snapshot/position streams converged to `1500/2000/2000`;
- Redis memory was about `30.92 MiB / 512 MiB` after convergence.

Interpretation:

- retain periodic health heartbeat because runtime freshness checks depend on
  it;
- source-side per-stream retention solves the observed growth without losing
  the independently retained command/order/trade history;
- continue the author41 canary before rolling the new gateway image to other
  systems.

## `7502T0U` RI Restart And Author41 Qty Step-Up

Pre-change broker truth from the active `7502T0U` gateway:

- positions: empty;
- regular orders: empty;
- stop orders: empty.

Changes:

- restarted the separate `trading-ri-author41-42-7502t0u` micro-live contour
  from zero;
- retained RI order size at `qty = 1`;
- changed `IMOEXF hybrid author41-short` from `qty = 1` to `qty = 2`;
- restarted only the author41 strategy runtime; its gateway and Redis state
  were preserved.

RI startup validation:

- gateway config resolved `RIM6`, 10-minute feed, and
  `control_cws_mode = action_scoped`;
- runtime resolved `mode = micro_live`, `allow_order_emission = true`, and
  `order_symbol = RTS-6.26`;
- broker bootstrap reconciled flat with no open orders or stops;
- history warmup processed `487` bars;
- historical `ri_model_decision` events were diagnostic replay output only;
  no historical `intent_emitted` was generated;
- RI Redis started clean at approximately `1.39 MiB / 512 MiB`.

Author41 validation:

- resolved runtime config reports `qty = 2`;
- startup replay guard was armed;
- no stale order/position tail appeared after restart.

Current state:

- both runtimes are healthy;
- both are safely blocked waiting for the next eligible live regular-session
  bar after restart;
- this is expected on Saturday with `weekends_off = true`.
