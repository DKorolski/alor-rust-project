# VPS Live Observations - 2026-06-05

## Pre-Open Log Review

Context:

- VPS: `155.212.170.21`, host `nektodk.ispvds.com`.
- Review time: `2026-06-05 08:47 MSK`, before the regular session open.
- Focus: compare `IMOEXF` behavior on `7502MIW` and `7502T0U`, check logs/resources, and keep extended micro soak journals current.

Health/resource snapshot:

- All checked trading containers were `healthy`:
  - `trading-hybrid-*`
  - `trading-alor-usdrubf-*`
  - `trading-ri-author41-42-7502miw-*`
  - `trading-hybrid-author41-7502t0u-*`
- Disk after previous cleanup remained acceptable:
  - `/`: `37G used / 79G`, about `50%`.
- Redis memory:
  - `trading-hybrid-redis-1`: about `204MiB / 1GiB`.
  - `trading-alor-usdrubf-redis-1`: about `218MiB / 1GiB`.
  - `trading-ri-author41-42-7502miw-redis-1`: about `245MiB / 768MiB`.
  - `trading-hybrid-author41-7502t0u-redis-1`: about `399MiB / 512MiB`.

Interpretation:

- Main `7502MIW` Redis instances are within the current envelope.
- `trading-hybrid-author41-7502t0u-redis-1` is again the tightest Redis contour and should remain on the Redis maintenance watchlist.

## IMOEXF Comparison

### `7502MIW` - primary `IMOEXF hybrid riskgate`

Observed:

- No fresh `IMOEXF` orders were found in `broker.orders.7502MIW` from `2026-06-04 00:00 UTC` through the pre-open check.
- No fresh `IMOEXF` position events were found in the latest sampled `broker.positions.7502MIW` window.
- Runtime warning scan for the primary hybrid contour showed only the already-known post-restart/pre-open warning sequence from `2026-06-04`:
  - `bars stream has data but runtime reads none`
  - stream `md.bars.7502MIW.10m`
  - consumer group `strategy-runtime-hybrid-riskgate-shadow-7502MIW`
- No new `mr_entry_suppressed`, `orphan_trade`, reject, or failed protective path events were seen for `7502MIW` `IMOEXF` in the checked interval.

Interpretation:

- Primary `7502MIW` `IMOEXF hybrid` was quiet over the checked fresh interval.
- The near-zero MR bracket churn patch did not receive a fresh live-trigger validation in this interval because no new `IMOEXF` primary hybrid trade was observed.
- No evidence of fresh uncontrolled `IMOEXF` state on `7502MIW`.

### `7502T0U` - `IMOEXF hybrid author41-short`

Observed MR cycle:

- Entry:
  - `2026-06-04 09:30:06 MSK`
  - `MR short`, `qty = 1`
  - entry order: sell limit `2595.5`
  - comment: `HYB|sid=hybrid_imoexf|c=6a21191007|o=MR|r=ENTRY`
- Protective orders:
  - TP: buy limit `2588.5`, `qty = 1`
  - SL: buy stop-limit `stop_price = 2615.5`, `price = 2616.0`, `qty = 1`
- Exit:
  - TP filled at `2026-06-04 16:49:17 MSK`
  - TP price: `2588.5`
  - paired SL canceled at `2026-06-04 16:49:18 MSK`
- Position stream later showed `IMOEXF qty = 0.0`, so broker state converged flat.

Warnings:

- Runtime logged one `orphan_trade` at entry fill:
  - symbol `IMOEXF`
  - side `sell`
  - qty `1`
  - price `2596.0`
  - order id `2033126295453316925`
- Broker order stream still identified the same cycle via the strategy-owned MR entry comment, and the cycle later closed via TP with SL cleanup.

Interpretation:

- The `7502T0U` author41 contour executed a clean MR bracket lifecycle at the broker level:
  - entry filled
  - TP installed
  - SL installed
  - TP filled
  - SL canceled
  - broker position flat
- The `orphan_trade` remains classified as the known fill-before-ack/order-correlation warning class, not as an uncontrolled position incident.
- Continue watching this class because the entry order had `request_id = null` in broker snapshots, even though the comments and final broker state converged.

## Other System Notes

`Alor-USDRUBF` on `7502MIW`:

- Order stream showed `USDRUBF|exit|bo_stop1_long` filled at `2026-06-04 16:00:10 MSK`.
- Latest sampled position stream entries were older than that filled exit and still showed `USDRUBF qty = 1.0`.
- Therefore the position stream was stale relative to the later order event during this pre-open check.
- Re-check broker positions after fresh broker snapshots resume; do not infer an active USDRUBF overnight carry from the stale position stream alone.

`RI author41/42` on `7502MIW`:

- Container was healthy and resource usage was low.
- No new RI-specific incident was identified in this pre-open pass.

## Watchlist Updates

- Keep `trading-hybrid-author41-7502t0u-redis-1` on Redis maintenance watch:
  - current memory about `399MiB / 512MiB`.
  - this contour grows faster than the primary `7502MIW` hybrid Redis.
- Keep `orphan_trade` / fill-before-ack correlation on watch:
  - latest `7502T0U` author41 MR entry produced one `orphan_trade`.
  - broker lifecycle still converged cleanly.
- Re-check `7502MIW` USDRUBF broker position after the next fresh broker position snapshot because the latest sampled position stream lagged behind the filled BO exit order.

## Redis Maintenance Action

Context:

- `trading-hybrid-author41-7502t0u-redis-1` was the tightest Redis contour before the session open.
- Top stream lengths before trim:
  - `events.health.hybrid_author41_short.7502T0U`: `36800`.
  - `broker.snapshots.7502T0U`: `19400`.
  - `broker.positions.7502T0U`: `7854`.
  - `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U`: `500`.
  - `broker.orders.7502T0U`: `82`.
  - `broker.trades.7502T0U`: `48`.

Safe trim applied:

- `events.health.hybrid_author41_short.7502T0U`: trimmed to `3000`.
- `broker.snapshots.7502T0U`: trimmed to `2000`.
- `broker.positions.7502T0U`: trimmed to `2000`.
- Protected / not trimmed:
  - `runtime.state.*`.
  - `runtime.riskgate.*`.
  - command streams.
  - order/trade streams, because they were already small.

Result:

- `trading-hybrid-author41-7502t0u-redis-1` memory decreased from about `393MiB / 512MiB` to about `39MiB` reported by Redis, about `48MiB / 512MiB` in Docker stats.
- `BGREWRITEAOF` completed successfully for the author41 Redis:
  - AOF current size decreased to about `12MiB`.
- The shared safe trim script was updated and deployed to:
  - local repo: `alor-rs-main/scripts/redis_safe_trim.sh`.
  - VPS: `/opt/maintenance/redis_safe_trim.sh`.
- A systemd timer was installed because `cron.service` is not present on the VPS:
  - service: `redis-safe-trim-live-soak.service`.
  - timer: `redis-safe-trim-live-soak.timer`.
  - schedule: `Mon..Fri 08:10:00 MSK`.
  - next scheduled run after installation: `2026-06-08 08:10:00 MSK`.

Post-maintenance checks:

- Trading containers remained `healthy`.
- Recent runtime logs showed no fresh `WARN` / `ERROR` after maintenance.
- Redis memory after script apply:
  - `trading-hybrid-author41-7502t0u-redis-1`: about `48MiB / 512MiB`.
  - `trading-hybrid-redis-1`: about `165MiB / 1GiB`.
  - `trading-alor-usdrubf-redis-1`: about `170MiB / 1GiB`.
  - `trading-ri-author41-42-7502miw-redis-1`: about `183MiB / 768MiB`.

## Current Read

- Extended micro soak remains operationally acceptable.
- `7502MIW` primary `IMOEXF` was quiet in the checked interval.
- `7502T0U` author41 `IMOEXF` showed a successful MR bracket lifecycle and flat convergence, with one known-class `orphan_trade` warning.
- No fresh evidence of CWS legacy-path regression, uncontrolled position tail, or stale protective order tail was found in this pass.
- Redis maintenance is now automated through a systemd timer rather than ad-hoc manual trims.
