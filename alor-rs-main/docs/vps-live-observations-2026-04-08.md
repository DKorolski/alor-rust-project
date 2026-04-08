# VPS Live Observations

Date: 2026-04-08

## Scope

Observed stacks:

- `sessiongap`
- `trading-hybrid`
- `alorusdrubf`

Observation window covered:

- 2026-04-07 session close
- 2026-04-08 session
- post-reboot recovery on 2026-04-08

## Summary

The day ended operationally clean:

- all observed strategies finished flat,
- no unresolved open position remained in the observed runtime state,
- post-reboot startup returned all three stacks to `LiveReady / ALLOWED`.

The main anomaly of the day was concentrated in `alorusdrubf`:

- repeated `create:market` exit sends hit `protocol_reset_without_close_handshake`,
- one reconnect cycle also hit `401 Invalid JWT token!` on CWS authorize,
- gateway recovery and strategy deferred retry logic still completed the exit and returned the strategy to flat.

## Findings

### 1. `sessiongap` closed cleanly

At 2026-04-07 20:30 UTC the strategy completed a normal exit lifecycle:

- `InPosition -> PendingExit`
- `intent_emitted action="place"`
- `command accepted`
- `execution confirmed`
- `PendingExit -> Flat`

The gateway side for the same action showed a clean action-scoped control session:

- forced token refresh before authorize,
- `authorize` `httpCode=200`,
- `create:limit` `httpCode=200`,
- clean session close.

Observed overnight transport noise existed on the gateway side, including websocket resets, but it happened without a pending control request and recovered normally.

### 2. `trading-hybrid` looked operationally normal

Observed behavior was consistent with expected lifecycle:

- overnight `ALLOWED -> BLOCKED -> SyncingGap/SyncingHistory`,
- daily rollover recalculated day features,
- after reboot the runtime restored state, replayed history warmup, and returned to `ALLOWED`,
- on 2026-04-08 it emitted one entry and one exit, and both were accepted and filled.

Observed runtime trade path:

- 2026-04-08 09:00 UTC entry accepted and filled,
- 2026-04-08 20:31 UTC exit accepted and filled.

No anomaly surfaced in the collected hybrid logs for this day.

### 3. `alorusdrubf` had repeated exit transport failures but recovered to flat

`alorusdrubf` recovered correctly after reboot:

- bootstrap snapshot loaded,
- runtime state restored,
- replay guard armed,
- first live-origin bar cleared the replay guard,
- runtime returned to `LiveReady / ALLOWED`.

Entry path:

- first live entry attempt at 2026-04-08 09:37 UTC failed with `cws_error` / `protocol_reset_without_close_handshake`,
- strategy deferred entry to the next bar,
- second entry attempt at 2026-04-08 09:38 UTC succeeded and opened a short position.

Exit path:

- 2026-04-08 15:52 UTC exit attempt was rejected with `trading_window_closed`,
- 2026-04-08 16:52 UTC exit attempt failed with `cws_error` / `protocol_reset_without_close_handshake`,
- 2026-04-08 17:51 UTC exit attempt failed with the same transport error,
- 2026-04-08 19:52 UTC exit attempt failed with the same transport error,
- 2026-04-08 20:31 UTC EOD exit attempt failed again with transport reset,
- during reconnect after that failure, CWS authorize returned `401 Invalid JWT token!`,
- gateway invalidated the cached token, refreshed it, re-authorized successfully,
- 2026-04-08 20:32 UTC the next retry succeeded,
- broker position transitioned `open_to_flat`.

This is the key operational reading for the day:

- recovery logic worked,
- deferred retry behavior worked,
- token invalidation and refresh on `401` worked,
- but the control path still showed repeated transport fragility on live exits for this stack.

## Runtime State Check

Latest observed `alorusdrubf` runtime state confirmed final flat outcome:

- `live_ready=true`
- `hybrid_state="flat"`
- `open_position_qty=0.0`
- `pending_request_ids=[]`
- `tracked_order_ids=[]`
- `exit_intent_inflight=false`

## Operational Reading

For 2026-04-08 the systems behaved as follows:

- `sessiongap`: normal
- `trading-hybrid`: normal
- `alorusdrubf`: recovered successfully, but with notable repeated exit-path transport instability

So the day can be considered operationally successful, with one important caveat:

- `alorusdrubf` reached the correct final outcome,
- but not via a clean single-pass execution path.

## Follow-Up

The next live-soak checks should continue to focus on `alorusdrubf`:

1. whether repeated `protocol_reset_without_close_handshake` continues to cluster around `create:market` exit sends,
2. whether `401 Invalid JWT token!` remains rare and self-healing after forced refresh,
3. whether every deferred exit still converges to flat before the end of session,
4. whether this pattern is isolated to the current strategy / stack contour or becomes visible elsewhere.
