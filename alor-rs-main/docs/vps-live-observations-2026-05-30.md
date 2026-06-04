# VPS Live Observations - 2026-05-30

## Weekend Health / Resource Check

Checkpoint:

- Time checked: `2026-05-30 08:45 MSK`.
- Context: Saturday/weekend monitoring after `7502T0U` RI handoff preparation.

Active containers:

- `trading-ri-author41-42-7502miw-*`: healthy.
- `trading-alor-usdrubf-*`: healthy.
- `trading-hybrid-*`: healthy.
- `trading-hybrid-author41-7502t0u-*`: healthy.
- `trading-ri-author41-42-7502t0u-*`: not running, expected after stopping temporary VPS RI on `7502T0U`.

Resource snapshot:

- RAM: `7.7 GiB` total, about `5.8 GiB` available.
- Disk `/`: `79G` total, `36G` used, `40G` available, `48%`.
- Redis memory:
  - `trading-ri-author41-42-7502miw-redis-1`: about `172M / 512M`.
  - `trading-alor-usdrubf-redis-1`: about `176M`.
  - `trading-hybrid-redis-1`: about `150M`.
  - `trading-hybrid-author41-7502t0u-redis-1`: about `280M / 512M`.

Log read:

- Overnight CWS/WS reconnects were observed across gateways.
- Reconnects had no command in flight:
  - `opcode_in_flight=None`.
  - `request_id=None`.
  - `pending_count=0`.
- Runtime guards moved from `ALLOWED` to expected overnight/weekend blocked states:
  - `phase=SyncingGap`.
  - `phase=SyncingHistory`.
  - temporary `gateway_ready=false`, `ws_connected=false`, `cws_authorized=false`.
- Repeated `trade event received existing=true` rows appeared after reconnect/resubscribe.
- These rows are interpreted as broker replay/resubscribe events, not new strategy emissions.
- No `command rejected` or panic/error path was observed in the checked window.

Broker-state read:

- Latest broker position snapshots show flat for traded instruments:
  - `RTS-6.26 qty=0`.
  - `USDRUBF qty=0`.
  - `IMOEXF qty=0`.
- No active stop-order tail was visible in the checked Redis streams.

Interpretation:

- Weekend state is operationally normal.
- All live VPS contours that should remain running are healthy.
- `7502T0U` temporary RI remains stopped as intended.
- The remaining `7502T0U` VPS contour is the trial `IMOEXF hybrid author41-short`.
- Redis usage is acceptable, but `trading-hybrid-author41-7502t0u-redis-1` is the closest to its cap and should stay on the resource watchlist.

Watchlist:

- Monitor `trading-hybrid-author41-7502t0u-redis-1` memory; current level is around `280M / 512M`.
- Continue weekly Redis/resource checks and safe trim/cleanup procedure before memory pressure becomes operational.
- Continue `IMOEXF hybrid` watchlist for partial fills and near-zero MR bracket churn before any larger scale-up decision.

## Safe Redis Trim - Hybrid Author41 7502T0U

Context:

- Redis: `trading-hybrid-author41-7502t0u-redis-1`.
- Reason: memory grew quickly to about `281M / 512M` over a few days.
- Diagnosis:
  - Main growth source was `events.health.hybrid_author41_short.7502T0U`.
  - Secondary source was `broker.snapshots.7502T0U`.
  - Bars/trades/orders were small and were not the cause.

Before trim:

- `events.health.hybrid_author41_short.7502T0U`: `33913` rows, about `219M`.
- `broker.snapshots.7502T0U`: `16959` rows, about `26M`.
- `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U`: `500` rows, about `1.6M`.
- Total Redis memory: about `281M`.

Action:

- Exact trim was used because approximate `MAXLEN ~ N` was rejected by this Redis CLI invocation.
- Commands applied:
  - `XTRIM events.health.hybrid_author41_short.7502T0U MAXLEN 2000`.
  - `XTRIM broker.snapshots.7502T0U MAXLEN 2000`.
  - `XTRIM runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U MAXLEN 200`.
- Trading streams were not trimmed:
  - `md.bars.7502T0U.10m`.
  - `broker.orders.7502T0U`.
  - `broker.trades.7502T0U`.
  - `cmd.orders.7502T0U.hybrid_author41_short`.
  - `cmd.acks.7502T0U.hybrid_author41_short`.

After trim:

- `events.health.hybrid_author41_short.7502T0U`: `2000` rows, about `13M`.
- `broker.snapshots.7502T0U`: `2000` rows, about `2.8M`.
- `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U`: `200` rows, about `0.6M`.
- Total Redis memory: about `16M`.
- Containers remained healthy after trim.

Follow-up:

- Add this contour to regular Redis trim/watch procedure.
- Consider reducing health event retention or emission frequency if the contour remains active for longer soak.

## Safe Redis Trim - All Active VPS Contours

Context:

- Follow-up check showed the same class of service-stream growth in the main contours too.
- The main growth sources were `events.health*` and `broker.snapshots.*`.
- Trading streams were intentionally not trimmed.

Actions:

- `trading-ri-author41-42-7502miw-redis-1`:
  - Before: about `174M`.
  - After: about `51M`.
  - Trimmed `events.health.ri_author41_42.7502MIW` to `2000`.
  - Trimmed `broker.snapshots.7502MIW` to `3000`.
- `trading-alor-usdrubf-redis-1`:
  - Before: about `178M`.
  - After: about `35M`.
  - Trimmed `events.health` to about `3000`.
  - Trimmed `broker.snapshots.7502MIW` to `3000`.
- `trading-hybrid-redis-1`:
  - Before: about `151M`.
  - After: about `37M`.
  - Trimmed `events.health` to about `3000`.
  - Trimmed `broker.snapshots.7502MIW` to `3000`.
  - Trimmed `broker.snapshots.7502T0U` to `3000`.
- `trading-hybrid-author41-7502t0u-redis-1`:
  - Before this second pass: about `17M`.
  - After: about `13M`.
  - Trimmed `events.health.hybrid_author41_short.7502T0U` to `1500`.
  - Trimmed `broker.snapshots.7502T0U` to `1500`.
  - Kept runtime state at `200`.

Config change:

- Reduced `Hybrid author41 7502T0U` runtime trim health retention:
  - file: `runtime.hybrid.live.7502T0U.author41-short.toml`;
  - `health = 3000` -> `health = 1500`.
- The same file was updated on VPS under `/opt/trading-hybrid-author41-7502t0u/configs/`.
- Backup on VPS:
  - `/opt/trading-hybrid-author41-7502t0u/configs/runtime.hybrid.live.7502T0U.author41-short.toml.bak.pre-health-trim-20260530`.

Result:

- All checked containers remained healthy after trim.
- Main finding: existing `[trim]` sections help but do not fully prevent service stream growth, especially for gateway-produced `broker.snapshots.*`.
- Recommendation: add a regular maintenance trim job for `events.health*`, `broker.snapshots.*`, and `runtime.state.*` across live Redis instances.
