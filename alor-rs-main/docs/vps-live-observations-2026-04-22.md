# VPS Live Observations - 2026-04-22

Date: 2026-04-22

Scope:

- `trading-hybrid`
- `trading-alor-usdrubf`

Context:

- `trading-sessiongap` had already been recovered on 2026-04-21 with reduced Redis trim and clean `from zero` Redis restart.
- `trading-hybrid` and `trading-alor-usdrubf` still carried the old large Redis persistence tails.
- The agreed operational plan was to wait for both stacks to be flat and then perform preventive reduced-trim rollout in a safe pre-open window.

## Executive Summary

The preventive maintenance was completed successfully before the market open.

- both stacks were confirmed `flat`
- both stacks were in `OutsideSession`
- reduced live trim baseline was applied on VPS
- both Redis instances were restarted `from zero`
- old Redis tails were intentionally discarded rather than preserved as backup
- both stacks returned healthy
- Redis memory footprint dropped from hundreds of MiB to about `5 MiB`

This was not an emergency recovery like `sessiongap`.
It was preventive maintenance intended to stop `hybrid` and `alor-usdrubf` from growing into the same Redis persistence problem.

## 1. Pre-Rollout Safety Check

At `2026-04-22 07:06 MSK` both stacks were in a safe state for maintenance.

### `trading-hybrid`

- `readiness=false`
- `runtime_phase="SyncingHistory"`
- `scheduler_state="OutsideSession"`
- runtime state showed:
  - `last_position_qty = 0.0`
  - `current_owner = null`
  - `pending_entry_* = null`
  - `pending_exit_* = null`
  - `deferred_* = null`

### `trading-alor-usdrubf`

- `readiness=false`
- `runtime_phase="SyncingHistory"`
- `scheduler_state="OutsideSession"`
- runtime state showed:
  - `hybrid_state = "flat"`
  - `open_position_qty = 0.0`
  - `pending_request_ids = []`
  - `tracked_order_ids = []`
  - `entry_intent_inflight = false`
  - `exit_intent_inflight = false`

Reading:

- no live position risk
- no pending or deferred tails
- no reason to postpone the preventive cleanup further

## 2. Applied Maintenance

For both stacks:

- runtime trim settings on VPS were reduced to:
  - `bars = 10000`
  - `orders = 5000`
  - `trades = 5000`
  - `positions = 20000`
  - `commands = 5000`
  - `acks = 5000`
  - `health = 5000`
  - `runtime_state = 500`
- stack was stopped with `docker compose down`
- `volumes/redis` was removed
- a fresh empty Redis directory was created
- stack was started again with `docker compose up -d`

Operational choice:

- Redis backup tails were not preserved
- this was intentional and aligned with the resource-control decision from the previous day

Small backup copies of the runtime config files were kept on VPS before editing.

## 3. Post-Rollout Validation

### Container health

Both stacks returned healthy immediately after recreate:

- `trading-hybrid-strategy-runtime-1`
- `trading-hybrid-alor-gateway-1`
- `trading-hybrid-redis-1`
- `trading-alor-usdrubf-strategy-runtime-1`
- `trading-alor-usdrubf-alor-gateway-1`
- `trading-alor-usdrubf-redis-1`

### Restart counts

Fresh start indicators:

- `hybrid_redis_restart = 0`
- `hybrid_runtime_restart = 0`
- `usdrubf_redis_restart = 0`
- `usdrubf_runtime_restart = 0`

### Redis startup evidence

Both Redis logs showed clean fresh startup:

- `Creating AOF base file ... on server start`
- `Creating AOF incr file ... on server start`

No old AOF/RDB replay tail was visible in the startup logs.

### Redis memory after cleanup

- `trading-hybrid-redis-1`
  - about `5.172 MiB / 1 GiB`
  - about `0.51%`
- `trading-alor-usdrubf-redis-1`
  - about `4.602 MiB / 1 GiB`
  - about `0.45%`

This is a major reduction from the pre-cleanup state:

- `hybrid` had been around `341 MiB`
- `alor-usdrubf` had been around `889 MiB`

## 4. Redis Tail Snapshot After Reset

Representative stream lengths after clean restart:

### `trading-hybrid`

- `md.bars.7502SN6.1m = 2520`
- `broker.snapshots.7502SN6 = 9`
- `events.health = 13`
- `runtime.state.hybrid_intraday.live.action_scoped.imoexf.7502SN6 = 501`

### `trading-alor-usdrubf`

- `md.bars.7502T0U.1m = 1744`
- `broker.snapshots.7502T0U = 6`
- `events.health = 11`
- `runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U = 501`

Reading:

- both stacks came back on short fresh tails
- the old heavy Redis persistence history is no longer in the live path

## 5. Immediate Runtime Status

Immediately after startup both stacks were still in the expected pre-open guarded state:

- `readiness=false`
- `runtime_phase="SyncingHistory"`
- `live_guard="BLOCKED"`
- `scheduler_state="OutsideSession"`

This was expected because:

- the market was still closed
- startup had not yet seen the fresh live bar path needed to transition to `ALLOWED`

## Final Verdict

The preventive pre-open Redis cleanup for `hybrid` and `alor-usdrubf` was successful.

- no position risk was interrupted
- no stale Redis persistence tail was carried forward
- both stacks restarted cleanly
- both Redis footprints dropped to near-zero operational levels

Current cross-stack status after the 2026-04-21 / 2026-04-22 maintenance line:

- `sessiongap`
  - recovered `from zero`
  - healthy
- `hybrid`
  - reduced trim baseline active
  - Redis restarted `from zero`
  - healthy
- `alor-usdrubf`
  - reduced trim baseline active
  - Redis restarted `from zero`
  - healthy

The next step is straightforward:

- observe normal startup progression into the open session
- keep watching Redis memory growth over the next several sessions
- use this period to judge whether the reduced trim baseline is sufficient or whether snapshot / age-based retention work is still required
