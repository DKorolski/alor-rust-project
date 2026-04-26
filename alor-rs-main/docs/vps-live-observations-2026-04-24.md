# VPS Live Observations - 2026-04-24

Date: 2026-04-24

Scope:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

Context:

- all three MOEX stacks are already running on the new `10m` live contour
- `sessiongap` had been manually flattened and restarted cleanly on 2026-04-23
- this note captures:
  - post-cutover live status on 2026-04-24
  - review of the previous session's `WARN/ERROR` slice
  - explicit confirmation that `hybrid` evening exit still converged correctly on `10m`

## Executive Summary

The cross-stack picture remains broadly healthy.

- all three stacks stayed `healthy`
- all three runtimes were observed in `LiveReady / ALLOWED`
- `sessiongap` remained clean after the second `from zero` restart
- `hybrid` did carry an open `IntradayBreakout` short late in the evening, but it later exited normally
- `alor-usdrubf` remained operational, with the same smaller follow-up signal as the prior day:
  - accepted action-scoped `create:market`
  - but runtime-side `orphan_trade` observations on some fills

The largest warning clusters still read as external Alor connectivity / auth churn rather than a new shared internal regression.

## 1. Cross-Stack Live Status

At the observed late-evening checkpoint:

- all 9 containers were `healthy`
- all three readiness endpoints returned:
  - `readiness=true`
  - `runtime_phase="LiveReady"`
  - `live_guard="ALLOWED"`
  - `scheduler_state="Open"`

This means the new `10m` contour remained alive across the entire stack set:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

## 2. Review of WARN / ERROR Slice

### `trading-sessiongap`

Observed runtime warning:

- startup-only `signal warmup incomplete`

Observed lifecycle:

- later `signal warmup complete`
- then `live_guard_changed ... to="ALLOWED"`

Reading:

- this remained a normal replay / indicator warmup artifact after clean startup
- no repeated broker-position adoption appeared
- no Redis / OOM / `BusyLoadingError` pattern returned

### `trading-hybrid`

The runtime warning/error slice stayed quiet.

Not observed:

- `pending_exit_active`
- `exit_suppressed`
- `orphan_trade`
- `command rejected`
- protective repair warning churn

Reading:

- `hybrid` continued to look clean on the runtime side
- the earlier request-id-skew / stale-pending class did not reappear in the observed slice

### `trading-alor-usdrubf`

The already-known smaller follow-up signal remained the main runtime-side anomaly:

- `orphan_trade` on accepted market flow

This still reads as:

- not a `create:market` send-path collapse
- more likely a remaining execution / trade-correlation issue after broker acceptance

## 3. External Alor Transport / Auth Noise

The main warning clusters continued to look external rather than strategy-local.

Observed classes across the reviewed slice:

- `peer closed connection without TLS close_notify`
- `Connection refused (os error 111)`
- `401 Invalid JWT token`
- `502 Bad Gateway` from `oauth.alor.ru/refresh`

Reading:

- the same pattern affected more than one stack
- this is best interpreted as upstream Alor transport / auth instability
- it should not be treated as a new internal regression in the `10m` contour itself

## 4. `hybrid` Evening Exit Observation

Late in the evening, `hybrid` was inspected while still holding an open short position.

Observed runtime state at that checkpoint:

- `last_position_qty = -1.0`
- `current_owner = "intraday_breakout"`
- `current_side = "short"`
- no `pending_exit_request_id`
- no deferred exit tail

This looked suspicious at first glance because wall-clock time in Moscow had already passed the expected evening-exit area.

However, the important distinction was:

- wall-clock time was later
- but the strategy's last processed `10m` bar was still behind that wall-clock boundary

So the correct operational reading was:

- the position was still legitimately live on the latest processed `10m` bar
- this was not yet evidence of a stuck exit

Later observation confirmed that `hybrid` did exit.

Reading:

- `hybrid` did not get stuck
- the evening exit completed on the expected `10m` cadence
- this is a useful positive signal that the `10m` contour still respects the intended EOD path

## Final Verdict

The 2026-04-24 soak observation remains encouraging.

- `sessiongap`
  - stayed clean after the prior-day manual flatten and second clean restart
- `hybrid`
  - remained operationally quiet
  - carried a late open short temporarily
  - then exited normally
- `alor-usdrubf`
  - stayed functionally alive on the intended action-scoped market path
  - still deserves follow-up on `orphan_trade` correlation semantics

The most important practical conclusion from this day is:

- the new `10m` contour still looks operationally viable
- `hybrid` evening exit behavior did not regress into a stuck-position incident
- the loudest warnings continue to point outward to Alor infrastructure rather than inward to a new shared runtime failure
