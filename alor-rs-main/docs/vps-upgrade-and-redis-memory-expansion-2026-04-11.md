# VPS Upgrade And Redis Memory Expansion

Date: 2026-04-11

## Context

During the Saturday review of the extended soak, the main anomaly was not a weekend intent leak but repeated Redis availability failures.

Observed runtime symptoms included:

- `broken pipe`
- `Connection refused (os error 111)`
- `Temporary failure in name resolution`
- `BusyLoadingError: Redis is loading the dataset in memory`

Kernel logs on the VPS confirmed repeated `memory cgroup out of memory` events killing `redis-server`.

Affected containers:

- `trading-hybrid-redis-1`
- `trading-sessiongap-redis-1`

At the same time, all three Redis containers were operating very close to their configured memory cap.

## Pre-Change State

Original VPS plan:

- `2 vCPU`
- `4 GB RAM`
- `50 GB disk`

Original Redis container limits:

- `REDIS_MEM_LIMIT=512m`
- `memswap=1 GiB`

Observed Redis memory pressure before the change:

- `trading-sessiongap-redis-1`: about `509 MiB / 512 MiB`
- `trading-hybrid-redis-1`: about `412-509 MiB / 512 MiB`, depending on restart point
- `trading-alor-usdrubf-redis-1`: about `506-509 MiB / 512 MiB`

Observed Redis internal memory:

- `trading-hybrid-redis-1`: `used_memory_human` above `600 MB`
- `trading-sessiongap-redis-1`: `used_memory_human` above `525 MB`
- `trading-alor-usdrubf-redis-1`: `used_memory_human` around `501 MB`

This made the container-level memory cap structurally too small for the live state footprint.

## Infrastructure Change

The VPS tariff was upgraded to:

- `4 vCPU`
- `8 GB RAM`
- `80 GB disk`

Post-upgrade host checks confirmed:

- memory available increased substantially;
- swap usage dropped to `0`;
- root filesystem moved from roughly `50 GB` to roughly `80 GB`.

Observed post-upgrade host state:

- RAM total: about `7.7 GiB`
- available RAM: about `7.2 GiB`
- disk available on `/`: about `39 GiB`

## Redis Memory Limit Change

After the VPS upgrade, Redis limits were expanded in all three live stack `.env` files:

- `/opt/trading-sessiongap/.env`
- `/opt/trading-hybrid/.env`
- `/opt/trading-alor-usdrubf/.env`

Change applied:

- from `REDIS_MEM_LIMIT=512m`
- to `REDIS_MEM_LIMIT=1024m`

Safety backups were created before editing:

- `/opt/trading-sessiongap/.env.bak.pre-redis-mem-20260411-1`
- `/opt/trading-hybrid/.env.bak.pre-redis-mem-20260411-1`
- `/opt/trading-alor-usdrubf/.env.bak.pre-redis-mem-20260411-1`

Each stack was then recreated in a controlled way using Docker Compose.

## Verification

New container limits confirmed via `docker inspect`:

- `trading-sessiongap-redis-1 mem=1073741824`
- `trading-hybrid-redis-1 mem=1073741824`
- `trading-alor-usdrubf-redis-1 mem=1073741824`

Post-change Redis memory usage:

- `trading-sessiongap-redis-1`: `544.5 MiB / 1 GiB` (`53.17%`)
- `trading-hybrid-redis-1`: `620.9 MiB / 1 GiB` (`60.64%`)
- `trading-alor-usdrubf-redis-1`: `515.8 MiB / 1 GiB` (`50.37%`)

This materially reduced the immediate container-level OOM risk compared with the previous `~99%` utilization under a `512 MiB` cap.

## Runtime State After Recreate

After the controlled recreate:

- all three stacks came back up;
- all containers were healthy;
- runtimes were blocked only on normal startup gates.

Observed runtime status immediately after the change:

- `trading-sessiongap`: `SyncingHistory / BLOCKED`
- `trading-hybrid`: `LiveReady / BLOCKED`
- `trading-alor-usdrubf`: `SyncingHistory / BLOCKED`

This was expected because the systems had just been restarted and still needed the normal live-bar/startup progression.

## Reading

The best current reading is:

- the Saturday anomaly was an infra memory incident, not a weekend trading logic failure;
- the VPS upgrade addressed the broader host-capacity problem;
- the Redis memory-limit increase addressed the narrower container-cgroup bottleneck;
- further soak observation is still needed, but the most acute Redis OOM risk has been reduced.

## Follow-Up

Useful next steps:

1. monitor the next trading sessions for any repeated Redis restart waves;
2. confirm that Redis restart counters stop growing abnormally;
3. consider stream retention / trim policies as a later optimization;
4. optionally prune unused Docker images for disk hygiene, but treat that as secondary to the RAM/OOM fix.
