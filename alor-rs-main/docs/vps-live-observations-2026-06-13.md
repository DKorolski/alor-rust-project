# VPS Live Observations - 2026-06-13

## Context

Weekend maintenance and post-DSWD inspection were performed while both live
portfolios were broker-flat and had no working protective orders.

Active contours:

```text
RI Author41/42 RIU6, 7502MIW
Alor-USDRUBF hybrid, 7502MIW
IMOEXF primary hybrid, 7502MIW
IMOEXF author41-short hybrid, 7502T0U
```

The temporary RI contour on `7502T0U` remains intentionally stopped.

## Trading And Log Status

All active runtime, gateway, and Redis containers were healthy before and
after maintenance.

The recent runtime log scan found no new:

```text
WARN
ERROR
command rejected
order timeout
stale pending exit
```

The weekend gateways and runtimes remain in a blocked synchronization phase
outside an active regular session. Short `gateway_health_stale`,
`cws_authorized=false`, and `ws_connected=false` transitions during the
gateway-only restarts cleared immediately and did not produce commands.

All resolved gateway configs still report:

```text
control_cws_mode = action_scoped
action_scope_enable_market = true
action_scope_enable_exit = true
```

## Redis OOM Finding

Kernel logs identified a previously unrecorded Redis cgroup OOM:

```text
2026-06-12 02:16:51 MSK
container = trading-alor-usdrubf-redis-1
result = redis-server killed by memory cgroup OOM
```

This was not host-wide memory pressure. The VPS still had several GiB of
available RAM. The affected Redis reached its container memory boundary
because its old gateway retained noisy health and broker snapshot streams
using one large global retention limit.

Largest pre-maintenance keys included:

```text
Alor-USDRUBF events.health ~= 98 MB
Alor-USDRUBF broker.snapshots.7502MIW ~= 81 MB
Primary Hybrid events.health ~= 219 MB
RI 7502MIW broker.snapshots.7502MIW ~= 129 MB
RI 7502MIW health ~= 59 MB
```

## Safe Maintenance

Only explicit noisy streams were trimmed. Runtime state, risk-gate state and
ledger, commands, acknowledgements, orders, trades, and model bars were
preserved.

Applied bounds:

```text
health = 1500
broker snapshots = 2000
broker positions = 2000
```

Online `BGREWRITEAOF` completed successfully for the three affected Redis
instances:

```text
Primary Hybrid AOF: ~75 MB -> ~5.9 MB
Alor-USDRUBF AOF: ~235 MB -> ~4.1 MB
RI 7502MIW AOF: ~44 MB -> ~4.7 MB
```

Post-maintenance Redis memory:

```text
Primary Hybrid used_memory ~= 18 MB
Alor-USDRUBF used_memory ~= 14 MB
RI 7502MIW used_memory ~= 16 MB
Hybrid author41 7502T0U remained ~= 14 MB
```

The verified per-stream retention gateway image was then rolled out
gateway-only to:

```text
trading-alor-usdrubf
trading-hybrid
trading-ri-author41-42-7502miw
```

Image:

```text
manual-20260606-perstream-retention
```

Every gateway returned healthy and logged the expected resolved source-side
limits:

```text
bars = 3000
orders/trades/commands/acks = 5000
positions/snapshots = 2000
health = 1500
```

No runtime or Redis restart and no from-zero reset were required.

Docker dangling image cleanup reclaimed approximately `428 MB`. Named old
images were retained for rollback.

## Resources After Maintenance

```text
host RAM available ~= 6.3 GiB / 7.7 GiB
swap used ~= 52 MiB / 3.9 GiB
disk used ~= 20 GiB / 79 GiB (27%)
active Redis containers ~= 21-28 MiB RSS each
```

## Risk-Gate Check

The primary IMOEXF risk-gate state remained intact:

```text
seed_loaded = true
ledger_rows_count = 209
last_finalized_session_date = 2026-06-10
rolling_sum_lb120 = 165.9
mr_enabled_current_session = true
mr_enabled_next_session = true
current_shadow_session_date = 2026-06-12
```

The special `2026-06-12` DSWD session has not been promoted to a completed
regular-session ledger row. Verify rollover/finalization on the next regular
session.

## Weekend Decision

No further live changes are required during this maintenance window.

Deferred intentionally:

- do not start the temporary RI `7502T0U` contour until duplicate-emitter
  status on the corporate server is explicitly confirmed;
- do not introduce Redis `maxmemory` policy in the same rollout as the
  retention change;
- do not reset any runtime state or risk-gate ledger;
- verify Monday startup, stream convergence, and risk-gate rollover before
  considering another operational change.

