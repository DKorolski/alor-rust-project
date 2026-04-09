# VPS Live Observations

Date: 2026-04-09

## Scope

Observed stacks:

- `sessiongap`
- `trading-hybrid`
- `alorusdrubf`

Observation window covered:

- 2026-04-09 session
- overnight transitions from 2026-04-08 into 2026-04-09

## Summary

The second extended soak day also ended operationally clean:

- all three observed strategies finished flat,
- all three returned to `LiveReady / ALLOWED` after the overnight sync path,
- no unresolved open position remained in the observed runtime state.

Compared with 2026-04-08:

- `sessiongap` again looked clean,
- `trading-hybrid` still completed the day correctly, but showed one noteworthy `orphan_trade` log during the exit path,
- `alorusdrubf` again showed transport fragility around `create:market`, but deferred retry still converged to flat.

## Findings

### 1. `sessiongap` was clean end to end

Observed day path:

- 06:00 UTC session rollover completed normally,
- runtime returned to `ALLOWED`,
- 09:00 UTC entry order was emitted, accepted, and filled,
- 14:03 UTC exit order was emitted, accepted, and filled,
- strategy returned to `Flat`.

Gateway logs for the same day showed a clean action-scoped lifecycle on both entry and exit:

- forced token refresh before authorize,
- `authorize` ok,
- `create:limit` ok,
- clean action-scope close.

Final runtime state confirmed:

- `phase="Flat"`
- `traded_session=true`

### 2. `trading-hybrid` completed correctly, with one timing anomaly in logs

Observed day path:

- overnight `SyncingGap -> SyncingHistory -> ALLOWED`,
- 11:44 UTC entry order was emitted, accepted, and filled,
- 20:31 UTC exit order was emitted and accepted,
- runtime then logged `order filled awaiting execution`,
- final runtime state was flat.

Important nuance:

- runtime logged `orphan_trade` for trade `2033126127949603102` before the matching `command accepted` line for order `2033126127949854197`.

Operational reading:

- the strategy still ended correctly in flat,
- but the event ordering in the runtime log suggests trade/order arrival race or visibility lag on the exit path.

Final runtime state confirmed:

- `last_position_qty=0.0`
- no pending entry / exit / TP / SL request ids
- no open protective orders
- strategy payload remained flat and clean

### 3. `alorusdrubf` repeated the transport-fragility pattern, but recovered again

Observed day path:

- overnight startup path returned the strategy to `ALLOWED`,
- 08:01 UTC first live entry attempt failed with `cws_error` / `protocol_reset_without_close_handshake`,
- strategy deferred entry to the following bar,
- 08:02 UTC second entry attempt succeeded and opened a short position,
- 20:31 UTC EOD exit attempt failed with the same transport reset,
- strategy deferred exit to the next bar,
- reconnect / reauthorize succeeded,
- 20:32 UTC retry succeeded,
- broker position transitioned `open_to_flat`.

Unlike 2026-04-08:

- no `401 Invalid JWT token!` was observed today,
- but the core `create:market` transport reset pattern remained.

Gateway logs confirmed the same sequence:

- pending `create:market` failed on transport reset,
- gateway failed the pending request,
- gateway refreshed token and re-authorized,
- next send succeeded.

Final runtime state confirmed:

- `hybrid_state="flat"`
- `open_position_qty=0.0`
- `pending_request_ids=[]`
- `tracked_order_ids=[]`
- `exit_intent_inflight=false`

## Runtime State Check

Latest observed state after the close:

- `sessiongap`: flat
- `trading-hybrid`: flat
- `alorusdrubf`: flat

No residual pending control requests were visible in the latest observed runtime payloads for `trading-hybrid` and `alorusdrubf`.

## Operational Reading

For 2026-04-09 the day should be classified as:

- `sessiongap`: normal
- `trading-hybrid`: normal final outcome, with one event-ordering anomaly worth watching
- `alorusdrubf`: successful final outcome, but repeated exit-path transport instability persists

So the second day of extended soak was again successful on final outcomes, but not fully free of anomalies.

## Follow-Up

The next soak days should continue to focus on:

1. whether `alorusdrubf` keeps reproducing `protocol_reset_without_close_handshake` specifically on `create:market`,
2. whether `trading-hybrid` repeats the `orphan_trade` / late order-ack ordering pattern,
3. whether both systems still always converge to flat before end of session,
4. whether any anomaly starts leaking into unresolved runtime state rather than staying self-healed.
