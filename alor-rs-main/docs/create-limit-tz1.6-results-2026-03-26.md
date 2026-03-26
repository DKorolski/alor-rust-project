# TZ 1.6 Results: Final Validation Before Hardening

Date: 2026-03-26

Related documents:

- `docs/create-limit-tz1.5-results-2026-03-26.md`
- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-review-submission-2026-03-25.md`

## 1. Purpose

`TZ 1.6` is the final analytic gate before hardening.

The practical question for this phase is:

- does regular cadence control activity keep the path healthy after a long quiet window;
- or is a proactive reconnect/recycle before the first limit command the more reliable workaround.

## 2. Baseline Entering TZ 1.6

The following should now be treated as established:

- `idle20 -> PASS`
- `idle30 -> REPRO`
- `idle30 + single keepalive@15m -> REPRO`
- safe active `create:limit -> delete:limit` loops do not themselves poison the path
- readiness and heartbeat can still look healthy before fail
- reconnect can return the path to a healthy immediate post-recovery state

## 3. Tooling Prepared

`scripts/limit_diag.sh` has been extended with `TZ 1.6` orchestration helpers:

- `wait-ready`
- `restart-gateway`
- `tz16-baseline`
- `tz16-cadence`
- `tz16-reconnect`

These wrappers are intended to keep:

- restart handling
- preflight naming
- summary formatting
- postflight capture

consistent across all `TZ 1.6` runs.

## 4. Planned Experiments

## 4.1 Experiment A: Baseline Idle 30m

Goal:

- confirm the `idle30` baseline remains comparable with `TZ 1.5`

Command:

```bash
/opt/limit_diag.sh tz16-baseline sessiongap 79.00 1800
```

Expected scenario:

- fresh gateway restart
- `30m` idle
- one passive `create:limit -> delete:limit`

## 4.2 Experiment B: Cadence Keepalive Every 10m

Goal:

- test whether a `10m` safe control cadence keeps the path alive through the `30m` window

Command:

```bash
/opt/limit_diag.sh tz16-cadence sessiongap 79.00 600 1800
```

Expected scenario:

- fresh gateway restart
- keepalive at `10m`
- keepalive at `20m`
- main probe at `30m`

## 4.3 Experiment C: Cadence Keepalive Every 5m

Goal:

- test whether a denser `5m` cadence materially improves outcomes versus the single-keepalive and `10m` cadence cases

Command:

```bash
/opt/limit_diag.sh tz16-cadence sessiongap 79.00 300 1800
```

Expected scenario:

- fresh gateway restart
- keepalive at `5m / 10m / 15m / 20m / 25m`
- main probe at `30m`

## 4.4 Experiment D: Reconnect Before First Order

Goal:

- test whether a controlled gateway recycle immediately before the first live limit command is a reliable operational workaround

Command:

```bash
/opt/limit_diag.sh tz16-reconnect sessiongap 79.00 1800
```

Expected scenario:

- fresh gateway restart
- `30m` idle
- controlled gateway recycle
- one passive `create:limit -> delete:limit`

## 5. Artifact Expectations

For each run, retain at minimum:

- preflight summary
- preflight readiness JSON
- preflight `/debug/cws`
- post readiness JSON
- post `/debug/cws`
- gateway post log
- scenario summary file

Target review bundle layout after execution:

- `idle30-baseline`
- `idle30-keepalive-10m`
- `idle30-keepalive-5m`
- `idle30-reconnect-before-order`

## 6. Current Status

At the time of this file:

- `TZ 1.6` orchestration support is prepared locally in `scripts/limit_diag.sh`
- live `TZ 1.6` runs are still pending

## 7. Expected Closing Decision

The analytic phase can be treated as complete if `TZ 1.6` lets us honestly choose one of these operational conclusions:

1. cadence keepalive materially keeps the path healthy; or
2. cadence is insufficient, but reconnect/recycle before the first limit command is reliable.
