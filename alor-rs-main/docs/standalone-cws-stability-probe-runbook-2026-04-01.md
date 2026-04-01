# Standalone CWS Stability Probe Runbook

Date: 2026-04-01

## Purpose

Use a truly standalone probe that does not depend on `alor-gateway` runtime logic.

The new tool lives in workspace package:

- `standalone-cws-stability-probe`

It performs only:

1. OAuth refresh
2. raw CWS connect
3. raw `authorize`
4. idle wait
5. one real `create:limit`
6. optional `delete:limit`
7. artifact bundle write

## Why This Exists

The existing `raw_cws_stability_probe` already proved a strong signal, but it still lived inside `alor-gateway`.

This standalone probe removes that dependency and gives a thinner harness for:

- quick smoke validation,
- `idle30` reproduction,
- reconnect-before-send comparison.

## Current Status

Observed on 2026-04-01:

- `standalone idle60` passed:
  - `create:limit` succeeded
  - `delete:limit` succeeded
- `standalone idle30` failed:
  - first `create:limit` after 30-minute idle hit
    `WebSocket protocol error: Connection reset without closing handshake`
- `window-scoped` two-send check passed:
  - first short-lived session succeeded,
  - CWS was fully closed,
  - after roughly 30 minutes with no open CWS,
  - second short-lived session also succeeded

This means the standalone tool itself is validated, and the next decisive experiment is:

- `standalone idle30 + reconnect-before-final-send`

## Build

From repo root:

```bash
cd alor_project/bybit_barter_test/alor-rs-main
cargo build --release -p standalone-cws-stability-probe
```

Linux amd64 build from macOS:

```bash
cd alor_project/bybit_barter_test
docker run --rm --platform linux/amd64 \
  -v "$PWD":/app \
  -w /app/alor-rs-main \
  -e CARGO_TARGET_DIR=/app/alor-rs-main/target-linux-amd64 \
  rust:1.76 \
  sh -c 'cargo build --release -p standalone-cws-stability-probe'
```

Produced binary:

- `target-linux-amd64/release/standalone-cws-stability-probe`

## Required Inputs

The probe needs:

- `ALOR_REFRESH_TOKEN`
- `portfolio`
- `cws_url`
- `oauth_url`
- `exchange`
- `instrument_group`
- `symbol`

Recommended source:

- existing gateway config file via `--config /path/to/gateway.sessiongap.live.7502MIW.toml`
- plus `ALOR_REFRESH_TOKEN` exported in shell

Minimal config subset expected from `--config`:

- `portfolio`
- `exchange`
- `instrument_group`
- `symbols`
- optional `cws_url`
- optional `oauth_url`
- optional `refresh_token`

## Safety Preconditions

Before every standalone probe run:

1. stop `sessiongap`
2. confirm no open position
3. confirm no active working orders
4. choose a price far enough from market to reduce fill probability
5. use `--cancel-final-order`

After every standalone probe run:

1. verify no active working orders remain
2. verify no position remains
3. restart `sessiongap`

## Recommended Order Of Experiments

### Step 1: `idle60` smoke

Purpose:

- validate the standalone tool itself with a short wait before burning 30 minutes

Command pattern:

```bash
export ALOR_REFRESH_TOKEN='...'

./standalone-cws-stability-probe \
  --live-confirm \
  --config /opt/trading-sessiongap/configs/gateway.sessiongap.live.7502MIW.toml \
  --symbol USDRUBF \
  --side buy \
  --qty 1 \
  --price 79.50 \
  --idle-seconds 60 \
  --cancel-final-order \
  --comment-prefix standalone_idle60 \
  --artifact-dir /opt/standalone-cws-probe-results/idle60
```

Expected use:

- if this fails before the send phase, fix tool/runtime environment first
- if it reaches send, the standalone harness is valid

### Step 2: `idle30`

```bash
export ALOR_REFRESH_TOKEN='...'

./standalone-cws-stability-probe \
  --live-confirm \
  --config /opt/trading-sessiongap/configs/gateway.sessiongap.live.7502MIW.toml \
  --symbol USDRUBF \
  --side buy \
  --qty 1 \
  --price 79.50 \
  --idle-seconds 1800 \
  --cancel-final-order \
  --comment-prefix standalone_idle30 \
  --artifact-dir /opt/standalone-cws-probe-results/idle30
```

### Step 3: `idle30 + reconnect-before-final-send`

```bash
export ALOR_REFRESH_TOKEN='...'

./standalone-cws-stability-probe \
  --live-confirm \
  --config /opt/trading-sessiongap/configs/gateway.sessiongap.live.7502MIW.toml \
  --symbol USDRUBF \
  --side buy \
  --qty 1 \
  --price 79.50 \
  --idle-seconds 1800 \
  --reconnect-before-final-send \
  --cancel-final-order \
  --comment-prefix standalone_idle30_reconnect \
  --artifact-dir /opt/standalone-cws-probe-results/idle30-reconnect
```

## Artifacts

Each run writes:

- `summary.txt`
- `result.json`
- `events.log`
- `frames.outbound.log`
- `frames.inbound.log`

Unlike the earlier in-gateway probe, this standalone version is intended to persist `summary.txt` and `result.json` even when final send fails via transport reset.

## What To Compare

For each run compare:

1. `connect_start`
2. `authorize_ok`
3. `idle_complete`
4. first outbound `create:limit`
5. final outcome:
   - success,
   - `httpCode != 200`,
   - transport reset,
   - timeout,
   - close frame

## Current Interpretation Guidance

If:

- `idle60` passes,
- `idle30` fails,

then the degradation is time-window sensitive.

If:

- `idle30` fails,
- `idle30 + reconnect-before-final-send` passes,

then reconnect may still be a valid mitigation.

If:

- both fail the same way,

then the deeper problem is likely not limited to stale old-session reuse.

Current reading after the runs already completed on 2026-04-01:

- `idle60` does pass,
- `idle30` does fail,
- two short-lived sessions separated by ~30 minutes with no open CWS in between do pass,

so the degradation is clearly time-window sensitive.

What still remains open:

- whether an explicit reconnect immediately before the final send is enough to recover the path in the thinner standalone harness.

What now looks materially stronger:

- a window-scoped lifecycle where CWS is opened only for a control action and closed immediately after order/cancel result.
