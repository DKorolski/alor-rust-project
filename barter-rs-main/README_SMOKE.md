# Smoke Runbook (alor-gateway + strategy-runtime)

This runbook is a short checklist to verify live behavior and reconnect safety.

## Prerequisites
- Redis running and reachable.
- `alor-gateway` configured to publish:
  - `broker.snapshots.<portfolio>` stream with `SnapshotOrders` and `SnapshotPositions`.
  - `events.health` stream with `gateway_phase`.
- `strategy-runtime` configured with `streams.snapshots` and `trade_mode=live`.

## Start order
1. Start `alor-gateway` and wait until `events.health` reports `gateway_phase=LiveReady`.
2. Start `strategy-runtime`.
3. Wait for the first **Live** bar to arrive (see note below).

> **Note:** if the bars timeframe is 60s, the first live bar can take up to one bar interval.

## Success criteria
- `strategy-runtime` logs `live_guard_transition ... to=ALLOWED` after:
  - snapshots are loaded,
  - first live bar is seen,
  - gateway phase is `LiveReady`.
- When blocked, `strategy-runtime` logs `intent_dropped_by_guard`.
- When allowed, `strategy-runtime` logs `intent_emitted`.

## Redis stream checks
Use Redis CLI to inspect streams:

```bash
XINFO STREAM broker.snapshots.<portfolio>
XREVRANGE broker.snapshots.<portfolio> + - COUNT 5

XINFO STREAM cmd.orders.<portfolio>
XREVRANGE cmd.orders.<portfolio> + - COUNT 5

XINFO STREAM cmd.acks.<portfolio>
XREVRANGE cmd.acks.<portfolio> + - COUNT 5
```

## Reconnect scenario (manual)
1. Ensure there is a working order in `broker.orders.<portfolio>` or a recent `cmd.orders.<portfolio>` entry.
2. Simulate reconnect:
   - restart `alor-gateway` **or**
   - drop network temporarily.
3. Expect `strategy-runtime` to log:
   - `live_guard_transition ... to=BLOCKED` with `phase=Reconnecting` or `phase=SyncingGap`.
4. When `events.health` returns to `gateway_phase=LiveReady`:
   - runtime logs `live_guard_transition ... to=ALLOWED`,
   - strategy can emit intents again.

## Troubleshooting
- If trading is blocked with `waiting_for_next_bar_after_restart`, wait for the next **Live** bar.
- If snapshots are missing, runtime will fail-fast in live mode.
