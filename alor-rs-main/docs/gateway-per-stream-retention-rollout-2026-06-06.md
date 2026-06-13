# Gateway Per-Stream Redis Retention Rollout

Date: 2026-06-06

## Problem

The gateway previously applied one global `TRIM_MAXLEN` value to every Redis
stream. Noisy health, broker snapshot, and position streams therefore retained
far more data than needed for live operation. The
`trading-hybrid-author41-7502t0u` Redis instance repeatedly grew toward its
`512 MiB` limit within days.

Changing health publication to status-change-only is unsafe under the current
runtime contract: the live guard requires fresh gateway health and treats it
as stale after approximately 20 seconds.

## Implemented Contract

The gateway still publishes periodic health heartbeat events, but supports an
independent source-side `XADD MAXLEN` for each stream class:

- `TRIM_MAXLEN_BARS`
- `TRIM_MAXLEN_ORDERS`
- `TRIM_MAXLEN_TRADES`
- `TRIM_MAXLEN_POSITIONS`
- `TRIM_MAXLEN_SNAPSHOTS`
- `TRIM_MAXLEN_COMMANDS`
- `TRIM_MAXLEN_ACKS`
- `TRIM_MAXLEN_HEALTH`

`TRIM_MAXLEN` remains the backward-compatible fallback for every value that is
not explicitly set.

Recommended author41 canary values:

```dotenv
TRIM_MAXLEN_BARS=3000
TRIM_MAXLEN_ORDERS=5000
TRIM_MAXLEN_TRADES=5000
TRIM_MAXLEN_POSITIONS=2000
TRIM_MAXLEN_SNAPSHOTS=2000
TRIM_MAXLEN_COMMANDS=5000
TRIM_MAXLEN_ACKS=5000
TRIM_MAXLEN_HEALTH=1500
```

## Canary Rollout

Apply the new gateway image only to `trading-hybrid-author41-7502t0u` first.
The strategy runtime and Redis remain running; no from-zero reset is required.

Acceptance checks:

1. The gateway startup log reports the expected resolved per-stream limits.
2. Gateway and runtime return to healthy state after the gateway-only restart.
3. Runtime health does not become persistently stale.
4. `events.health*`, `broker.snapshots.*`, and `broker.positions.*` converge to
   their configured bounds.
5. No action-scoped command-path regression or new reject class appears.

Rollback:

1. Restore the previous gateway image tag.
2. Remove the per-stream environment variables if necessary.
3. Restart only `alor-gateway`.

## Follow-Up

Keep the whitelist safe-trim timer as a safety net during the canary. After a
clean observation period, roll the same source-side retention profile to the
other live contours. A compact change-event stream may be added later for
operator diagnostics, but it must not replace the heartbeat used by the live
guard.

## Canary Result

The author41 canary was deployed on `2026-06-06` with gateway image:

`manual-20260606-perstream-retention`

Post-deploy checks:

- gateway-only restart completed; Redis and strategy runtime were not reset;
- gateway and runtime returned to `healthy`;
- CWS authorization completed successfully;
- transient `gateway_health_stale` cleared after the first fresh heartbeat;
- gateway startup log reported all expected per-stream limits;
- stream lengths converged immediately to:
  - health: `1500`;
  - snapshots: `2000`;
  - positions: `2000`;
- Redis memory was approximately `30.92 MiB / 512 MiB` after convergence.

Canary status: deployed and healthy. Continue observation before applying the
new gateway image to the remaining live contours.

## Remaining Live Contour Rollout

On `2026-06-13`, after a Redis cgroup OOM was confirmed for
`trading-alor-usdrubf-redis-1`, the same verified image and per-stream limits
were rolled out gateway-only to:

- `trading-alor-usdrubf`;
- `trading-hybrid`;
- `trading-ri-author41-42-7502miw`.

Before rollout, noisy health, snapshot, and position streams were safely
trimmed and each affected Redis completed an online `BGREWRITEAOF`. Runtime
state, command/ack streams, model bars, and the IMOEXF risk-gate ledger were
preserved.

All three gateways returned healthy, reported the expected resolved limits,
and retained their action-scoped execution configuration. No runtime or Redis
restart and no from-zero reset were required.

Current rollout status: enabled on all active VPS live contours.
