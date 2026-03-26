# TZ 1.6 Review Bundle

This bundle contains the curated artifacts for the final `TZ 1.6` validation runs.

Scenarios:

- `idle30-baseline`
  - fresh restart, `30m` idle, one passive `create:limit -> delete:limit`
  - expected comparison case for the residual idle failure path
- `idle30-keepalive-10m`
  - fresh restart, safe keepalive at `10m` and `20m`, main probe at `30m`
- `idle30-keepalive-5m`
  - fresh restart, safe keepalive at `5/10/15/20/25m`, main probe at `30m`
- `idle30-reconnect-before-order`
  - fresh restart, `30m` idle, controlled gateway recycle, then one passive `create:limit -> delete:limit`

Each scenario directory contains the core review subset:

- scenario summary file
- preflight summary
- preflight `readiness.json`
- preflight `cws.debug.json`
- post `readiness.json`
- post `cws.debug.json`
- `gateway.post.log`

The reconnect scenario additionally includes:

- `before_reconnect` preflight
- `after_reconnect` preflight

Purpose of the bundle:

- make review easy without opening raw VPS capture directories;
- keep the comparison focused on the four decisive `TZ 1.6` scenarios;
- preserve the exact artifacts that support the final `baseline vs cadence vs reconnect` conclusion.
