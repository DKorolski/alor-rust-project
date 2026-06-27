# VPS live observations — 2026-06-27

Review window: post-rollout session on 2026-06-26 and morning check on 2026-06-27.
Timezone in this note: Moscow time unless explicitly stated otherwise.

## Executive read

- Runtime rollout is in place on all five live stacks: `manual-20260626-freeze-observability-0ac4dbf`.
- Containers were healthy on the 2026-06-27 morning check: runtime, gateway, and Redis were up/healthy on all stacks.
- No `intent_dropped`, `missed_runtime_not_ready`, `live_entry_missed`, strategy reject, panic, or readiness timeout was found after the rollout window.
- First post-rollout live-bar readiness behavior looked correct: runtime waited readiness, then allowed trading without dropping the first valid intent.
- RI strategy-owned command streams had no new RI orders after the rollout.
- IMOEXF MR activity on both relevant portfolios converged flat; current state has no active orders.
- Main watchlist item is not freeze-intent drift anymore, but broker/trade correlation noise and one residual-emergency flattening sequence around 17:36 on 2026-06-26.

## Runtime and resource state

Morning check on 2026-06-27:

- `trading-ri-author41-42-7502t0u`: runtime/gateway/redis healthy.
- `trading-ri-author41-42-7502miw`: runtime/gateway/redis healthy.
- `trading-alor-usdrubf`: runtime/gateway/redis healthy.
- `trading-hybrid-author41-7502t0u`: runtime/gateway/redis healthy.
- `trading-hybrid`: runtime/gateway/redis healthy.

Runtime image on all five stacks:

```text
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260626-freeze-observability-0ac4dbf
```

Gateway image stayed unchanged:

```text
ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-20260618-oauth-68d1cd1
```

Redis memory was not near exhaustion. Earlier checks showed low memory usage, no evictions, no rejected connections, and no concerning blocked-client pattern.

## Readiness / freeze-intent behavior

At the first live bar after rollout, all stacks reached:

```text
LiveReady / ALLOWED
```

Observed counters were consistent with the intended patch behavior:

- `readiness_wait_started_total=1`
- `readiness_wait_allowed_total=1`
- `readiness_wait_timeout_total=0`
- `intent_blocked_state_kept_total=0`
- `orphan_trade_total=0` initially after the open readiness check

The first live bar processed was:

```text
last_bar_ts_utc=1782453600
```

This corresponds to the 2026-06-26 09:00 MSK bar. Runtime waited for readiness and allowed execution instead of dropping the signal under `SyncingHistory/gateway_ready=false`.

Later checks still showed:

- no readiness timeouts;
- no blocked intents kept in state;
- no missed-runtime-not-ready records;
- no rejected/panicked strategy flow.

## Strategy-owned activity after rollout

### RI

No strategy-owned RI command activity was found after the rollout on either RI portfolio:

- `7502T0U`
- `7502MIW`

This means there was no observed post-patch RI entry to validate against live fill timing during this review window.

### USDRUBF / 7502MIW

Observed strategy-owned entry:

- 2026-06-26 11:10:00/11:10:01 — `market` entry buy `USDRUBF`, qty 1.
- Request id: `d63000fe-013c-5379-982a-d8bd55c51723`.
- Broker order id: `2023556210669896112`.
- Fill: buy qty 1 @ 76.94 around 11:10:01.

Later state/broker position was flat.

Watchlist:

- A later `orphan_trade` warning appeared for `USDRUBF` sell qty 1 @ 78.86 around 17:36.
- The command summary did not show a matching strategy-owned exit command in the same simple command-stream scan.
- Current broker/state is flat, so this is not an operational exposure, but it should remain in the correlation/replay watchlist.

### IMOEXF Author41 / 7502T0U

Observed MR bracket cycle:

- 2026-06-26 10:20:01 — short entry sell `IMOEXF`, qty 3 @ 2238.0.
- Request id: `a59acabb-5c5b-5dcb-98b7-aa9beff68c1e`.
- Broker order id: `2033126359877903421`.
- TP placed: buy qty 3 @ 2235.5, broker order id `2033126359877903457`.
- SL placed: stop id `121753643`.
- 2026-06-26 10:24:53 — TP filled buy qty 3 @ 2235.5.
- 2026-06-26 10:24:54 — SL delete/cancel accepted.

Later observed activity:

- 2026-06-26 15:10:01 — market entry buy `IMOEXF`, qty 3 @ 2315.0.
- Request id: `4ad6b279-f2e1-5d41-8b6d-e48249c0b648`.
- Broker order id: `2033126359878401193`.

Around 17:36 there was a broker residual flattening sequence:

- `orphan_trade`: sell `IMOEXF`, qty 1 @ 2279.0.
- `broker_residual_emergency_exit`: reason `broker_position_size_changed`, previous qty 3, broker qty 2, emergency sell qty 2.
- Another `broker_residual_emergency_exit`: reason `unexpected_broker_residual`, previous qty 0, broker qty -2, emergency buy qty 2.

Current state after the sequence:

- flat;
- no active orders;
- no pending entry/exit request;
- no active TP/SL ids.

Assessment:

- Operationally the stack converged flat.
- The 17:36 sequence should stay on watchlist because it suggests fragmented/replayed broker position/trade updates or a correlation gap around residual handling.

### Hybrid IMOEXF / 7502MIW

Observed MR bracket cycle:

- 2026-06-26 10:20:01/10:20:02 — short entry sell `IMOEXF`, qty 3 @ 2238.0.
- Request id: `f19d70fd-6cc9-522d-ba03-f62daf4de4e1`.
- Broker order id: `2033126359877903459`.
- TP placed: buy qty 3 @ 2228.0, broker order id `2033126359877903485`.
- SL placed: stop id `121753644`.
- 2026-06-26 10:28:03 — TP filled buy qty 3 @ 2228.0.
- 2026-06-26 10:28:05 — SL delete/cancel accepted.

Later observed activity:

- 2026-06-26 15:10:00 — market entry buy `IMOEXF`, qty 3 @ 2315.5.
- Request id: `80cd11d8-0451-5c3d-a23a-dcc9ced700dd`.
- Broker order id: `2033126359878401038`.

Watchlist:

- `orphan_trade` appeared on the 10:20 entry fill even though command/ack/order/fill existed. This looks like fill-before-runtime-correlation noise rather than a true unmanaged trade.
- Another `orphan_trade` appeared around 17:35:49: sell `IMOEXF`, qty 3 @ 2279.5.

Current state after the sequence:

- flat;
- no active orders;
- no pending entry/exit request;
- no active TP/SL ids.

## Reconnect / gap-sync noise

Overnight logs showed gateway/live-guard transitions during reconnect and gap sync:

- `ALLOWED -> BLOCKED` while `cws_authorized=false`, `ws_connected=false`, `gateway_ready=false`;
- `SyncingGap` / `SyncingHistory`;
- return to normal readiness afterward.

There were also older RI `orphan_trade` warnings during reconnect/gap-sync replay. No matching new RI strategy commands were found after rollout, so these look like broker stream replay/correlation noise rather than fresh trading activity.

## Current operational state

Current read:

- all reviewed systems are flat;
- no active orders were found on the checked IMOEXF states;
- no pending entry/exit state was present;
- no readiness-timeout or dropped-intent pattern appeared after the patch;
- Redis/resources looked normal.

## Remaining watchlist

1. Investigate/categorize `orphan_trade` warnings after reconnect and around same-day fills.
   - Distinguish true unmanaged broker activity from replay/fill-before-correlation ordering.
2. Review the 2026-06-26 17:36 Author41 / 7502T0U residual-emergency sequence.
   - It converged flat, but the double residual correction deserves a precise cause.
3. Keep monitoring freeze-intent readiness counters at the next open.
   - Key expected values: no `readiness_wait_timeout_total`, no `intent_blocked_state_kept_total`, no `missed_runtime_not_ready`.
4. BO partial-fill logging/scope remains a cleanup candidate.
   - Confirm whether prior wording was only diagnostic text or an actual scope issue.
   - Partial-entry accumulation should remain bracket-MR scoped and should not leak into BO or simple market entry/exit semantics.

## Broker-risk migration stop

Later on 2026-06-27, after the Alor depositary-license risk review and funds-withdrawal decision, live order-emitting systems were stopped intentionally.

Pre-stop broker REST check:

- `7502MIW`: non-RUB open positions = 0.
- `7502T0U`: non-RUB open positions = 0.
- `7502SN6`: non-RUB open positions = 0.

Stopped on VPS:

- `/opt/trading-ri-author41-42-7502t0u`: `strategy-runtime`, `alor-gateway`.
- `/opt/trading-ri-author41-42-7502miw`: `strategy-runtime`, `alor-gateway`.
- `/opt/trading-alor-usdrubf`: `strategy-runtime`, `alor-gateway`.
- `/opt/trading-hybrid-author41-7502t0u`: `strategy-runtime`, `alor-gateway`.
- `/opt/trading-hybrid`: `strategy-runtime`, `alor-gateway`.

Redis containers were left running intentionally to preserve local operational history and allow follow-up reconciliation.

Post-stop status:

- all five `strategy-runtime` containers exited cleanly with code `0`;
- all five `alor-gateway` containers exited cleanly with code `0`;
- all five Redis containers remained up/healthy;
- broker REST re-check still showed non-RUB open positions = 0 for `7502MIW`, `7502T0U`, and `7502SN6`.

Operational read:

- systems are no longer order-emitting;
- broker positions are flat for traded instruments;
- next work should focus on funds withdrawal, broker-risk decision memo, and new Finam/T-Bank gateway preparation.
