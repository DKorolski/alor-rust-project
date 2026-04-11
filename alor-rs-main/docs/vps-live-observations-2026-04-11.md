# VPS Live Observations 2026-04-11

Date: 2026-04-11

## Summary

Saturday review focused on the `trading-hybrid` stack because only `IMOEXF` remained active and the runtime had recently received a weekend intent-suppression patch.

The main conclusion is:

- no evidence was found that the strategy leaked weekend trading intents;
- the dominant anomaly was infrastructure-side Redis instability caused by repeated memory cgroup OOM kills;
- the same pressure pattern also touched `sessiongap-redis`, while `alorusdrubf-redis` remained alive but was also near its memory ceiling.

## Hybrid Runtime Behavior

Observed `trading-hybrid-strategy-runtime-1` behavior:

- runtime repeatedly restored state, completed warmup, and returned to `LiveReady / ALLOWED`;
- Saturday bars continued to populate strategy state correctly;
- no clear Saturday order-intent leak was observed in the inspected logs;
- the runtime remained operationally flat when inspected between restart waves.

Important restart windows observed in runtime logs:

- around `2026-04-11 14:25 MSK`
- around `2026-04-11 14:31 MSK`
- around `2026-04-11 14:37 MSK`

Runtime symptoms during those windows:

- `broken pipe`
- `Connection refused (os error 111)`
- `Temporary failure in name resolution`
- `BusyLoadingError: Redis is loading the dataset in memory`

This pattern is consistent with Redis becoming unavailable, restarting, reloading its dataset, and only then allowing the runtime to resume.

## Redis / VPS Findings

Host memory snapshot after the incident:

- total RAM: `3.7 GiB`
- used RAM: `2.6 GiB`
- free RAM: `186 MiB`
- available RAM: `1.1 GiB`
- swap used: `1.1 GiB / 1.9 GiB`

Redis container limits:

- all three Redis containers are capped at `512 MiB`
- all three currently sit near the cap

Observed live memory usage:

- `trading-hybrid-redis-1`: `509.4 MiB / 512 MiB`
- `sessiongap-redis-1`: `509.8 MiB / 512 MiB`
- `alorusdrubf-redis-1`: `507.7 MiB / 512 MiB`

Observed Redis internal memory:

- `trading-hybrid-redis-1`: `used_memory_human=601.34M`
- `sessiongap-redis-1`: `used_memory_human=525.48M`
- `alorusdrubf-redis-1`: `used_memory_human=501.29M`

Observed restart counts:

- `trading-hybrid-redis-1`: `restart=143`
- `sessiongap-redis-1`: `restart=5`
- `alorusdrubf-redis-1`: `restart=0`

Kernel `dmesg` confirms repeated Redis OOM kills:

- `2026-04-11 14:37:15 MSK`
- `2026-04-11 14:38:29 MSK`
- `2026-04-11 14:43:24 MSK`
- `2026-04-11 14:45:08 MSK`
- `2026-04-11 14:49:33 MSK`
- `2026-04-11 14:51:42 MSK`
- `2026-04-11 14:55:41 MSK`
- `2026-04-11 14:58:18 MSK`
- `2026-04-11 15:01:48 MSK`

Affected cgroups map to:

- `trading-hybrid-redis-1`
- `sessiongap-redis-1`

## Disk / Docker Notes

Disk pressure does not appear to be the direct incident cause:

- root filesystem usage: `37G / 50G` (`78%`)
- inode usage: `29%`

However, Docker disk footprint is large:

- `docker system df` reports `21.24 GB` of images
- `21.17 GB` is reclaimable
- `/var/lib/docker/overlay2` uses about `24 GB`

This is worth cleaning for hygiene, but it does not explain the Redis OOM behavior directly because the observed failure mode was memory-cgroup pressure, not disk exhaustion.

## Reading

Current best reading for 2026-04-11 is:

- the Saturday anomaly should be classified as an infra incident, not a strategy logic failure;
- weekend intent suppression does not currently show evidence of leaking actual trading actions;
- the urgent risk is Redis memory headroom, not the weekend signal path.

## Recommended Actions

Operationally, the next actions should be:

1. increase Redis memory limits above `512 MiB`;
2. review Redis retention / stream trimming so memory growth is bounded;
3. keep observing whether `sessiongap-redis` continues to restart under the same pressure;
4. optionally prune unused Docker images to free disk space, but treat that as hygiene rather than the primary fix.
