# VPS Live Observations - 2026-05-17

## Weekend health check

Collection window: 2026-05-17 10:29-10:31 MSK.

VPS resources:

- Uptime: 35 days, 19 hours.
- Load average: 0.24 / 0.36 / 0.32.
- RAM: 7.7 GiB total, 1.9 GiB used, 5.9 GiB available.
- Swap: 3.9 GiB total, 245 MiB used.
- Disk `/`: 79 GiB total, 52 GiB used, 24 GiB available, 69%.

Docker memory snapshot:

- `sessiongap-redis-1`: 156.1 MiB / 1 GiB.
- `hybrid-redis-1`: 155.9 MiB / 1 GiB.
- `alor-usdrubf-redis-1`: 138.2 MiB / 1 GiB.
- `ri-author41-42-redis-1`: 98.43 MiB / 768 MiB.
- `ri-shadow-redis-1`: 206.0 MiB / 768 MiB.
- Strategy runtimes and gateways remain small; no memory pressure observed.

All main containers are running. Live strategy runtimes are healthy.

## Disk usage watch

Disk usage increased materially versus the prior check: from roughly 38 GiB used / 51% to 52 GiB used / 69%.

Main contributors:

- `/opt`: 33 GiB total.
- `/opt/htx_barter`: 15 GiB, mostly `/opt/htx_barter/target/debug`.
- `/opt/bybit_barter_eth_bo_v2`: 11 GiB, mostly `/opt/bybit_barter_eth_bo_v2/target/debug`.
- `/var/lib/docker`: 12 GiB.
- Docker image cache: 11.6 GiB total, 11.54 GiB reclaimable according to `docker system df`.
- `/var/log/journal`: about 1.5 GiB.

Interpretation:

- The disk increase is not caused by the live Redis contours.
- The largest recent growth appears to be Rust build artifacts under `/opt/htx_barter/target/debug`, including large incremental objects and a large debug example binary created on 2026-05-16.
- No cleanup was performed during this observation pass.

Recommended follow-up:

- Prepare a separate safe cleanup pass for old build artifacts and unused Docker images.
- Treat `/opt/htx_barter/target/debug`, `/opt/bybit_barter_eth_bo_v2/target/debug`, and unused Docker images as primary candidates.
- Do cleanup separately from live observation and re-check services after cleanup.

## Post-cleanup check

Collection window: 2026-05-17 10:50 MSK.

Cleanup outcome:

- Disk `/`: 79 GiB total, 42 GiB used, 33 GiB available, 57%.
- `/opt`: 23 GiB, down from 33 GiB before cleanup.
- `/opt/bybit_barter_eth_bo_v2`: 1.2 GiB, down from about 11 GiB before cleanup.
- `/opt/htx_barter`: still 15 GiB and remains the largest `/opt` directory.
- Docker image cache remained unchanged: 11.6 GiB total, 11.54 GiB reclaimable according to `docker system df`.

Redis memory after cleanup:

- `sessiongap`: 148.36 MiB used, 624.86 MiB peak.
- `hybrid`: 149.55 MiB used, 648.18 MiB peak.
- `alor-USDRUBF`: 132.17 MiB used, 667.84 MiB peak.
- `RI micro`: 93.83 MiB used, 512 MiB configured Redis maxmemory.
- `RI shadow`: 200.72 MiB used, 512 MiB configured Redis maxmemory.

Service check:

- All main containers remained up and healthy after cleanup.
- No cleanup side effects were observed.
- Disk pressure is materially lower, but `/opt/htx_barter` build artifacts and unused Docker images remain safe-cleanup candidates for a separate pass.

## Docker image cache cleanup

Collection window: 2026-05-17 10:58-10:59 MSK.

Action:

- Ran `docker image prune -a -f`.
- Containers, volumes, networks, and Redis data were not pruned.
- The command removed only images not associated with any existing container.

Result:

- Docker reclaimed 11.28 GiB.
- Docker images changed from 65 total / 9 active / 11.54 GiB reclaimable to 9 total / 9 active / 152.6 MiB reclaimable.
- Disk `/` improved to 79 GiB total, 31 GiB used, 45 GiB available, 41%.
- All 15 containers remained running after cleanup.
- Live runtime/gateway containers still reported healthy status after cleanup.

Operational note:

- The cleanup removed old rollback/build images from local disk.
- If an old image tag is needed later, it will need to be pulled again from the registry or rebuilt.
- No live trading state was removed.

## Current status

All live strategy contours are flat as of the 2026-05-17 Sunday check.

- `sessiongap`: flat, no commands/trades since 2026-05-16 start.
- `hybrid IMOEXF`: flat, no pending entry/exit/protective orders, no commands/trades since 2026-05-16 start.
- `alor-USDRUBF`: flat, no pending request ids, no commands/trades since 2026-05-16 start.
- `RI author41/42`: flat, no pending request ids, no commands/trades since 2026-05-16 start.
- `RI shadow`: runtime has no errors; gateway still has weekend reconnect/no-bar noise.

Latest broker/runtime alignment:

- `IMOEXF`: broker qty 0.0, runtime flat.
- `USDRUBF` on `7502T0U`: broker qty 0.0, runtime flat.
- `RTS-6.26`: broker qty 0.0, RI runtime `phase=flat`.
- `sessiongap` own `USDRUBF`: runtime `phase=Flat`.

## 2026-05-16 and 2026-05-17 activity

No live strategy commands, acknowledgements, or live trades were observed for the main live contours on 2026-05-16 or by the 2026-05-17 morning check.

Runtime counters since 2026-05-17 00:00 MSK:

- `sessiongap`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `hybrid`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `alor-USDRUBF`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `RI micro`: `ERROR=0`, `WARN=0`, `command_rejected=0`, `orphan_trade=0`.
- `RI shadow runtime`: `ERROR=0`, `WARN=0`.

Gateway warnings:

- Main live gateways show expected weekend transport/sync noise: `eof`, `protocol_reset_without_close_handshake`, TLS close-notify EOF.
- RI gateway had two `AckTimeout for RIM6 (bars)` events after reconnect, with no active command and no position impact.
- `RI shadow` gateway had one `AckTimeout for RIM6 (bars)` during the 2026-05-17 window.

## Hybrid riskgate watch

Riskgate state remained unchanged during the weekend:

- `last_finalized_session_date=2026-05-14`.
- `ledger_rows_count=193`.
- `rolling_sum_lb120=182.9`.
- `mr_enabled_current_session=true`.
- `mr_enabled_next_session=true`.

Interpretation:

- The 2026-05-15 riskgate ledger row has still not been finalized by the Sunday check.
- This is not treated as an incident yet because the implementation is bar-driven and there has been no next regular trading session event.
- The check remains open for the next regular session.

## Watch list

- On the next regular session, verify that hybrid riskgate finalizes the 2026-05-15 row and advances `last_finalized_session_date` / `ledger_rows_count`.
- Track disk usage closely; post-Docker-cleanup 41% is comfortable again, but `/opt/htx_barter` build artifacts remain the main local cleanup candidate.
- Keep `RI shadow` weekend bar-silence / subscribe retry warnings in observability bucket unless they continue into a regular session.
- No trading/runtime incident is open from the 2026-05-17 check.
