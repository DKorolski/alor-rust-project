# VPS Live Observations

Date: 2026-04-10

## Scope

Observed stacks:

- `sessiongap`
- `trading-hybrid`
- `alorusdrubf`

Observation window covered:

- 2026-04-10 session close
- overnight transitions from 2026-04-09 into 2026-04-10

## Summary

The day ended operationally clean on final outcomes:

- all three observed strategies finished flat,
- no residual open position remained in the latest observed runtime state,
- all three returned through the expected overnight sync path and traded again during the session.

The day still contained non-trivial anomalies:

- `trading-hybrid` had repeated protective-order and repair-path problems in the morning,
- `trading-hybrid` also emitted another `orphan_trade`,
- `alorusdrubf` again reproduced the `create:market` transport-reset pattern on both entry and exit,
- both stacks still converged to flat by end of session.

## Findings

### 1. `sessiongap` was clean again

Observed day path:

- overnight `SyncingGap -> SyncingHistory -> ALLOWED`,
- 14:00 UTC entry emitted, accepted, and filled,
- 20:30 UTC exit emitted, accepted, and filled,
- runtime returned to `Flat`.

Final runtime state confirmed:

- `phase="Flat"`
- `traded_session=true`
- no residual pending state

Operational reading:

- `sessiongap` again looked normal.

### 2. `trading-hybrid` finished flat, but its intraday control path was noisy

Observed runtime path:

- overnight sync completed and runtime returned to `ALLOWED`,
- 06:01 UTC first entry was emitted, accepted, and filled,
- immediate protective follow-up failed:
  - one request failed with `cws_error: protocol_reset_without_close_handshake`,
  - another failed with `cws response timeout`,
- runtime temporarily went `ALLOWED -> BLOCKED -> ALLOWED`,
- repair path then continued:
  - 06:03 UTC another protective order was accepted,
  - 06:53 UTC `delete_stop_limit` cleanup succeeded,
  - 07:02 UTC another order sequence opened/finalized a new path,
  - 07:04 UTC a further order was accepted.

Later in the session:

- at 08:23 UTC runtime logged another `orphan_trade`,
- the latest end-of-session state still showed the strategy flat and cleaned up.

Additional operational note:

- around 20:09 UTC the runtime process appears to have restarted and reloaded config,
- startup guard blocked trading until the next fresh live bar,
- runtime then returned to `ALLOWED`.

Final runtime state confirmed:

- `last_position_qty=0.0`
- `active_cycle_id=null`
- `tp_order_id=null`
- `sl_stop_order_id=null`
- no pending request ids

Operational reading:

- final outcome was correct,
- but `trading-hybrid` had the noisiest internal day so far,
- especially around protective repair / stop cleanup / event ordering.

### 3. `alorusdrubf` repeated the same transport-reset pattern again

Observed runtime path:

- overnight sync completed and runtime returned to `ALLOWED`,
- 08:06 UTC first entry attempt failed with `cws_error` / `protocol_reset_without_close_handshake`,
- strategy deferred entry to the following bar,
- 08:07 UTC retry succeeded and opened a short position,
- 08:51 UTC first exit attempt failed with the same transport reset,
- 09:51 UTC next exit attempt again failed with the same transport reset,
- 20:33 UTC EOD exit attempt again failed with the same transport reset,
- each time runtime temporarily moved through `BLOCKED` during gateway recovery,
- by the end of the day the latest runtime state was flat.

Observed final state:

- `hybrid_state="flat"`
- `open_position_qty=0.0`
- `pending_request_ids=[]`
- `tracked_order_ids=[]`

Operational reading:

- the same core transport fragility remains reproducible for this stack,
- but deferred retry plus gateway reconnect/reauthorize still converged to the correct end-of-day outcome.

## Runtime State Check

Latest observed end-of-session runtime states:

- `sessiongap`: flat
- `trading-hybrid`: flat
- `alorusdrubf`: flat

No open-position residue remained in the latest observed runtime payloads.

## Operational Reading

For 2026-04-10 the day should be classified as:

- `sessiongap`: normal
- `trading-hybrid`: successful final outcome with significant runtime/control-path noise
- `alorusdrubf`: successful final outcome with repeated known transport-reset pattern

So the day was again operationally successful on final result, but it does not reduce concern about:

1. `alorusdrubf` transport instability on `create:market`
2. `trading-hybrid` protective-order repair / event-ordering instability

## Follow-Up

The next soak observations should continue to watch:

1. whether `trading-hybrid` repeats `orphan_trade` and protective repair noise,
2. whether `trading-hybrid` runtime restarts recur during trading hours,
3. whether `alorusdrubf` continues to hit `protocol_reset_without_close_handshake` on both entry and exit,
4. whether both noisy stacks still always converge to flat without manual intervention.
