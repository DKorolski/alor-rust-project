# Redis Retention And Cleanup Plan

Date: 2026-04-21

## Context

`trading-sessiongap` entered a Redis restart loop on 2026-04-21.

Observed evidence:

- runtime side:
  - `broken pipe`
  - `Connection refused`
  - `BusyLoadingError: Redis is loading the dataset in memory`
  - later `Temporary failure in name resolution`
- Redis side:
  - repeated AOF / RDB reload
  - repeated background AOF rewrite attempts
  - `RDB memory usage when created 1396.65 Mb`
- kernel side:
  - repeated `memory cgroup out of memory`
  - repeated `Killed process ... redis-server`

This is not a strategy logic problem.
It is a Redis state / retention / persistence growth problem.

## Current Retention Model

The current runtime retention is already bounded, but only by `MAXLEN`, not by time.

Original live trim settings before reduction:

- `bars = 200000`
- `orders = 100000`
- `trades = 100000`
- `positions = 50000`
- `commands = 50000`
- `acks = 100000`
- `health = 10000`
- `runtime_state = 2000`

Important limitation:

- this is count-based retention
- it does **not** mean "keep exactly 5-7 days"
- actual time horizon depends on event rate per stream

## Current Footprint Snapshot

Observed Redis memory before the `sessiongap` recovery:

- `trading-sessiongap-redis-1`
  - `used_memory_human: 1.61G`
  - runtime currently unstable / loading, so stream inspection was incomplete
- `trading-hybrid-redis-1`
  - `used_memory_human: 331.60M`
- `trading-alor-usdrubf-redis-1`
  - `used_memory_human: 876.17M`

Observed stream lengths for healthy stacks:

### `trading-hybrid`

- `md.bars.7502SN6.1m` -> `5199`
- `broker.snapshots.7502SN6` -> `26457`
- `broker.positions.7502SN6` -> `592`
- `broker.orders.7502SN6` -> `7`
- `broker.trades.7502SN6` -> `5`
- `cmd.orders.7502SN6` -> `2`
- `cmd.acks.7502SN6` -> `2`
- `events.health` -> `52912`
- `runtime.state.*` -> `2001`

### `trading-alor-usdrubf`

- `md.bars.7502T0U.1m` -> `28352`
- `broker.snapshots.7502T0U` -> `100000`
- `broker.positions.7502T0U` -> `18679`
- `broker.orders.7502T0U` -> `120`
- `broker.trades.7502T0U` -> `76`
- `cmd.orders.7502T0U` -> `103`
- `cmd.acks.7502T0U` -> `103`
- `events.health` -> `100000`
- `runtime.state.*` -> `2000`

Reading:

- the biggest growth drivers are not `orders`, `trades`, `cmd.orders`, or `cmd.acks`
- the likely pressure comes from:
  - `broker.snapshots.*`
  - `events.health`
  - `md.bars.*`
  - `broker.positions.*`
  - AOF persistence overhead on top of those streams

## Goal

Reduce Redis persistence footprint so that:

1. live recovery still works,
2. runtime state continuity is preserved,
3. recent operational evidence remains available,
4. Redis restart / AOF reload stays comfortably below the container memory limit,
5. the problem does not recur every few days.

## Retention Policy Target

### Practical target

For live stacks, keep only a short operational recovery window:

- approximately `5-7` trading days of recent Redis stream history
- latest runtime state tail
- latest broker snapshots tail

### Important precision note

With the current implementation, we can only enforce this approximately by count.

So the practical plan is:

1. **immediate remediation**
   - reduce `MAXLEN` values materially
2. **follow-up hardening**
   - add age-based retention using `XTRIM MINID` or equivalent periodic trim logic

## Recommended New Baseline Limits

These are the reduced live defaults selected for the next cleanup iteration.

They are intentionally much lower than current values, but still conservative enough for recovery:

- `bars = 10000`
- `orders = 5000`
- `trades = 5000`
- `positions = 20000`
- `commands = 5000`
- `acks = 5000`
- `health = 5000`
- `runtime_state = 500`
- `broker.snapshots`:
  - currently not separately configurable in runtime trim
  - should be treated as a special follow-up item

Rationale:

- `bars = 10000`
  - enough for about a week of recent 1m bars for these MOEX flows with safety headroom
- `orders/trades/cmd/acks = 5000`
  - more than enough for operational debugging and recovery
- `positions = 20000`
  - positions stream can be noisier; keep more headroom
- `health = 5000`
  - enough for short operational history without letting health dominate memory
- `runtime_state = 500`
  - enough for recent state tail and debugging, without storing thousands of stale state snapshots

## Incident Recovery Plan For `trading-sessiongap`

### Phase 1. Stabilize first

Before any retention rollout:

1. confirm account is flat
2. confirm no working orders
3. confirm no stop orders
4. stop the `trading-sessiongap` stack
5. back up current Redis volume before destructive cleanup

### Phase 2. Emergency recovery

Preferred recovery for `sessiongap`:

- restart `from zero`
- do **not** try to preserve the huge Redis persistence tail as the live baseline

Reason:

- the current persisted Redis dataset is already too large for safe restart
- carrying it forward keeps the restart-loop risk alive
- `sessiongap` can re-bootstrap from broker snapshots and fresh live flow

### Phase 3. Bring up with reduced retention

For the recovered `sessiongap` stack:

- lower trim settings before restart
- clear the old Redis state / persistence
- start fresh with the reduced limits

### Status update

This recovery has now been executed for `trading-sessiongap`:

- trim on VPS was reduced to the baseline above
- `trading-sessiongap` was restarted `from zero`
- the old Redis persistence tail was removed from the live path
- the stack returned to `LiveReady / ALLOWED`

Operational note:

- the temporary Redis backup created during recovery was later removed intentionally to reclaim disk space
- this was treated as acceptable because the backup was not expected to be needed operationally

## Rollout Plan For All Stacks

### Stage A. Emergency only for `sessiongap`

1. stabilize `sessiongap` with clean Redis
2. lower retention settings there first
3. observe restart behavior and memory profile

### Stage B. Prevent recurrence in healthy stacks

After `sessiongap` is stable:

1. apply the same lower trim settings to:
   - `trading-hybrid`
   - `trading-alor-usdrubf`
2. do controlled recreates
3. verify:
   - no regression in live startup
   - no loss of essential recovery semantics

Reason:

- `hybrid` and `alor-usdrubf` are still healthy, but they are also accumulating large Redis tails
- this is preventive maintenance, not only a `sessiongap` fix

Current operational decision:

- do not touch `trading-alor-usdrubf` while it is in position
- perform preventive rollout for `trading-hybrid` and `trading-alor-usdrubf` in the next safe pre-open window

## Required Follow-Up In Code / Config

### 1. Lower live trim defaults

Update live configs and, if desired, runtime defaults so future stacks do not inherit the oversized limits.

### 2. Add explicit snapshot retention control

Current pressure strongly suggests that `broker.snapshots.*` needs its own bounded policy.

This should become a first-class configurable retention target rather than an uncontrolled long tail.

### 3. Add age-based retention

If we truly want "5-7 days", count-based `MAXLEN` is only approximate.

Better long-term solution:

- periodic `XTRIM MINID` for old entries
- or an equivalent background cleanup job

This is especially valuable for:

- `md.bars.*`
- `broker.snapshots.*`
- `events.health`
- `broker.positions.*`

## Validation Checklist

After cleanup / rollout, confirm:

1. Redis restarts stop growing abnormally
2. Redis loads its dataset without `BusyLoadingError` loops
3. runtime containers stop restart-looping
4. live guards return to normal:
   - `SyncingGap -> SyncingHistory -> ALLOWED`
5. memory usage stays comfortably below limit after several sessions
6. AOF rewrite no longer triggers repeated OOM kills

## Recommended Order

1. Observe the recovered `sessiongap` for one or two sessions.
2. In the next safe pre-open window, roll the same lower retention baseline to `hybrid`.
3. In the same or next safe pre-open window, roll the same lower retention baseline to `alor-usdrubf` once it is safely flat.
4. Then open a follow-up work item for:
   - snapshot retention control
   - age-based trim

## Bottom Line

Yes, old Redis data should be cleaned aggressively enough to avoid repeat OOM.

The right near-term approach is:

- `from zero` recovery for `sessiongap` with much smaller stream `MAXLEN`
- then apply the same reduced retention baseline to `hybrid` and `alor-usdrubf` in a safe pre-open window

The right longer-term approach is:

- move from rough count-based retention toward real age-based retention for `5-7` days.
