# VPS Live Observations - 2026-04-23

Date: 2026-04-23

Scope:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

Context:

- all three MOEX stacks had already been cut over to the new `10m` live contour in a controlled `from zero` rollout
- `sessiongap` then adopted a live broker position on startup and required manual flatten plus one more clean `from zero` restart
- this note summarizes the 2026-04-23 session after the `10m` cutover and clean-restoration work

## Executive Summary

The overall result for 2026-04-23 is operationally positive, with one important external transport/auth noise window and one smaller strategy-runtime follow-up signal.

- `trading-sessiongap` ended the day on a clean contour:
  - manual flatten completed successfully
  - second `from zero` restart completed successfully
  - stack later returned to `LiveReady / ALLOWED`
  - no repeated broker-position adoption after the second restart
- `trading-hybrid` remained quiet and operationally clean:
  - no meaningful runtime warnings in the observed slice
  - no visible recurrence of stale `pending_exit_active`
  - no visible new `orphan_trade`
- `trading-alor-usdrubf` remained functionally alive on the target `action-scoped create:market` path, but produced two `orphan_trade` warnings on accepted market exits
- the largest warning cluster of the day was external:
  - repeated Alor transport / auth instability
  - `Connection refused`
  - `401 Invalid JWT token`
  - `502 Bad Gateway` from `oauth.alor.ru/refresh`

Reading:

- the new `10m` contour itself did not show a broad internal regression
- the loudest failures were broker-side infrastructure / auth churn
- the most useful remaining internal follow-up is `alor-usdrubf orphan_trade on accepted action-scoped market flow`

## 1. `trading-sessiongap`

### Startup and manual flatten

After the first `10m from zero` cutover, `sessiongap` did not start truly flat.

Observed startup evidence:

- gateway positions subscription reported a real broker position:
  - `qty = 1`
  - `avgPrice = 75.17`
- runtime logged:
  - `state corrected by broker qty=1.0`
  - `live phase transition Flat -> InPosition`

Because that adopted position had no strategy-owned `tp/sl`, it was manually flattened through the normal live command path:

- manual `exit` command was emitted as `place`
- gateway accepted it
- broker order was filled
- runtime logged:
  - `command acknowledged outcome="accepted"`
  - `orphan_trade`
  - `live phase transition InPosition -> Flat`

This was followed by a second clean `from zero` restart of `sessiongap` only.

### Second clean restart

The second restart was operationally clean:

- Redis started with a fresh new AOF
- startup logs showed:
  - `bootstrap: snapshots loaded orders=0 positions=0`
- the previous `state corrected by broker qty=1.0` did not repeat
- startup guard progressed from:
  - `SyncingHistory`
  - to `LiveReady / ALLOWED`

Observed runtime signal:

- one startup-only warning:
  - `signal warmup incomplete`
- later:
  - `signal warmup complete`
  - `live_guard_changed ... to="ALLOWED"`

Reading:

- this warning was part of normal replay / indicator warmup and not a separate incident
- after the second restart, `sessiongap` finally matched the intended clean post-cutover state

## 2. `trading-hybrid`

Observed runtime slice for the 2026-04-23 session was quiet.

Visible lifecycle:

- standard startup guard transitions
- `signal warmup complete`
- `live_guard_changed ... to="ALLOWED"`

Not observed in the pulled slice:

- `pending_exit_active`
- `exit_suppressed`
- `orphan_trade`
- `command rejected`
- protective-path warning churn

Reading:

- `hybrid` looked operationally healthy on the new `10m` contour during the observed session
- no new obvious control-path regression was visible in the runtime warning/error slice

## 3. `trading-alor-usdrubf`

### Positive signal

The target action-scoped market path remained alive and visible in gateway logs.

Observed accepted flow:

- action-scope session open
- forced token refresh before authorize
- `authorize ok`
- `create:market`
- `http_code=Some(200)`
- clean session close

This confirms that the intended `action_scoped + force_token_refresh_before_authorize` contour is still active after the `10m` cutover.

### Follow-up signal: orphan trades

Two runtime warnings appeared:

- `2026-04-23T08:10:01Z`
  - `orphan_trade`
  - `side="sell"`
  - `qty=1.0`
  - `price=74.89`
- `2026-04-23T15:00:06Z`
  - `orphan_trade`
  - `side="sell"`
  - `qty=1.0`
  - `price=75.73`

These warnings happened alongside gateway-side accepted `action-scoped create:market` flows rather than inside a failed transport send.

Reading:

- this does not look like the old `create:market` transport-failure storm
- it does look like a remaining lifecycle / correlation issue between accepted market path and runtime trade matching
- this should be tracked separately as a smaller but still meaningful follow-up item

## 4. External Alor Incident Window

The heaviest warning cluster of the day affected `sessiongap` and `alor-usdrubf` simultaneously and looks external to our own strategy logic.

Observed classes:

- `peer closed connection without sending TLS close_notify`
- `Connection refused (os error 111)`
- `401 Invalid JWT token`
- `502 Bad Gateway` on `https://oauth.alor.ru/refresh`

Representative windows:

- around `12:46Z .. 12:54Z`
  - repeated socket / reconnect failures
  - repeated `Connection refused`
  - cached token invalidation and `401 Invalid JWT token`
- around `17:12Z .. 17:16Z`
  - repeated `502 Bad Gateway` from `oauth.alor.ru/refresh`

Reading:

- this was not isolated to one strategy stack
- this is best read as an upstream Alor transport / auth incident rather than a local VPS or strategy-runtime bug

## Final Verdict

2026-04-23 was overall a good operational day for the new `10m` contour, with one manual cleanup step and one clear external broker-side noise window.

- `sessiongap`
  - required manual flatten plus a second clean restart
  - then converged to the intended clean state
- `hybrid`
  - remained calm and operationally clean in the observed slice
- `alor-usdrubf`
  - retained the improved action-scoped market send path
  - but still showed `orphan_trade` warnings worth follow-up
- the loudest warnings of the day were external Alor connectivity / auth failures, not a new internal cross-stack regression

The next useful follow-up is:

- keep observing the `10m` contour for several more sessions
- separately track `alor-usdrubf orphan_trade on accepted action-scoped market flow`
- avoid over-interpreting the 2026-04-23 `Connection refused / 401 / 502` window as a strategy regression
