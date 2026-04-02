# Trading Window Closed -> Blocked Observation

Date: 2026-04-02

## Context

During live soak on `sessiongap`, an entry intent was emitted while the exchange window was closed.

Observed runtime/gateway chain:

1. runtime emitted live entry intent (`action="place"`),
2. gateway rejected command with `error_code="trading_window_closed"`,
3. strategy transitioned `PendingEntry -> Blocked`,
4. no automatic resume happened after the trading window reopened.

## Evidence (Runtime Logs)

Observed sequence:

- `2026-04-02T11:00:02.468173Z` `live phase transition` `Flat -> PendingEntry`
- `2026-04-02T11:00:02.468221Z` `intent_emitted action="place"`
- `2026-04-02T11:00:02.579358Z` `command rejected ... error_code=Some("trading_window_closed")`
- `2026-04-02T11:00:02.579437Z` `session gap entry failed terminally`
- `2026-04-02T11:00:02.579466Z` `live phase transition` `PendingEntry -> Blocked`

Timezone note:

- `11:00:02 UTC` is `14:00:02 MSK` on `2026-04-02`.

## Main Observation

For this failure class, current behavior is terminal at strategy phase level:

- a command-level validation reject (`trading_window_closed`) is treated as an entry terminal failure;
- strategy remains in `Blocked` and does not self-recover on later open-window bars.

Operationally this means a missed-entry condition can persist until manual intervention.

## Scope Clarification

This is a runtime recovery semantics issue, not a CWS transport stability issue.

It is independent from:

- `control_cws_mode` (`legacy_long_lived` vs `action_scoped`),
- `action_scope_force_token_refresh_before_authorize`.

Reason:

- rejection happens at gateway validation stage before any actionable broker order acceptance.

## Clean-Restart Clarification

A full clean restart can clear the immediate stuck state, but does not remove the underlying behavior.

What restart does:

1. clears current terminal `Blocked` tail in process memory/state stream,
2. lets strategy re-arm from `Flat` on fresh runtime boot.

What restart does not do:

1. it does not change transition logic for `trading_window_closed`,
2. the same `PendingEntry -> Blocked` can happen again on the next closed-window reject.

So restart is an operational workaround, not a semantic fix.

## FLUSHDB Side Effect Note

If `FLUSHDB` is done as part of clean-slate restart, expect temporary warmup regressions:

- `sessiongap`: possible `Blocked(reason="indicators_not_warmed")` until required session features are rebuilt;
- `hybrid`: `entry_ready=false` until `prev_day_range` is recomputed.

This is expected after wiping Redis and should not be confused with transport-path regressions.

## Operational Mitigation (Current)

Until runtime behavior is changed:

1. monitor rejects with `error_code="trading_window_closed"`,
2. monitor runtime phase tail for `PendingEntry -> Blocked`,
3. if strategy is flat and no working orders remain, perform controlled runtime recycle to re-arm.

## Follow-Up Engineering Direction

Recommended change:

1. classify `trading_window_closed` on entry as recoverable, not terminal,
2. move to a non-terminal deferred phase instead of terminal `Blocked`,
3. automatically re-issue only a fresh order command, not the original transport request,
4. allow re-issue only when runtime is again `ALLOWED` and gateway is `LiveReady`,
5. keep explicit logs for defer/reissue/expiry reason to preserve diagnostics.

Exit policy should be stricter than entry:

1. `exit` deferred on `trading_window_closed` should remain pending until the position becomes `Flat`,
2. this includes overnight carry if the position is still open,
3. re-issue still happens only under `ALLOWED + LiveReady`.

Entry policy should be narrower:

1. defer within the same strategy session,
2. do not replay a stale entry indefinitely after the session/entry window has already expired.

## Implementation Slice Status

Current implementation status on `2026-04-02`:

1. `sessiongap` runtime slice implemented:
   - `PendingEntry + trading_window_closed -> EntryDeferredWindowClosed`
   - `PendingExit/ExitRecoveryPending + trading_window_closed -> ExitDeferredWindowClosed`
   - deferred re-issue occurs only on later live bars when normal live execution gates are open again
2. `hybrid` runtime slice implemented:
   - `PendingEntry + trading_window_closed` no longer forces `safe_mode_close_only`
   - a deferred entry intent is stored and re-issued only when live execution gates reopen
   - `PendingExit + trading_window_closed` stores deferred close intent instead of losing the close obligation
   - deferred exit re-issue continues on later eligible live bars until the position becomes `Flat`
3. persistence/runtime-state coverage added for both contours:
   - deferred intent metadata is serialized into strategy state
   - restart does not need to rely on reconstituting a stale transport request
   - a fresh request is emitted on re-issue with a new `request_id`

## Implemented Policy Summary

The implemented policy is now:

1. classify `trading_window_closed` as a recoverable business reject for both `entry` and `exit`,
2. preserve the business intent, not the original transport request,
3. re-issue only on later live bars when runtime is effectively tradable again:
   - `allow_live_orders=true`
   - `gateway_phase=LiveReady`
   - live bar origin
4. `entry` does not replay indefinitely:
   - `sessiongap` entry is bounded by the strategy entry/session window
   - `hybrid` deferred entry expires across local-day rollover instead of carrying a stale intraday entry into the next day
5. `exit` remains sticky until the position becomes `Flat`, including overnight carry when needed.

## Clean-Slate Rollout Order (Recommended)

For controlled rollout after this observation:

1. finalize baseline docs and risk note (this file),
2. deploy target images/configs,
3. restart gateway+runtime cleanly for both contours,
4. avoid ad-hoc `FLUSHDB` during soak unless explicitly planned,
5. verify two gates before live observation:
   - infra gate: `readiness=true`, `live_guard=ALLOWED`, `LiveReady`,
   - strategy gate: no persistent warmup-block tail (`indicators_not_warmed` / `entry_ready=false` past expected warmup window),
6. monitor `trading_window_closed` rejects and immediately check phase transitions.

## Acceptance For Fix

Expected result after implementation:

1. emit entry intent during closed window,
2. receive `trading_window_closed` reject,
3. strategy does not remain terminally blocked,
4. on first open-window eligible live bar under `ALLOWED + LiveReady`, strategy can emit a fresh command without restart,
5. for open positions, a deferred close obligation remains active until `Flat`.

## Post-Fix Validation

On `2026-04-02`, a follow-up clean rollout with:

- runtime commit `0b605ac`,
- runtime image `dev-0b605ac-indwarm-20260402-213111`,
- full `restart from zero` for both `sessiongap` and `hybrid`,

validated that the adjacent warmup/state issues were also addressed.

Observed result:

1. both gateways reached `LiveReady` after cold backfill,
2. both runtimes moved from temporary bootstrap block to `ALLOWED`,
3. `sessiongap` runtime state contained reconstructed:
   - `prev_close`
   - `yesterday_range`
   - `pre_prev_close`
4. `hybrid` runtime state contained reconstructed:
   - `prev_day_close`
   - `prev_day_range`
   - `prev_day_return`
   - `entry_ready=true`

This matters because a clean restart is now a usable operational recovery tool:

- it no longer leaves both strategies stuck in signal-warmup-null state,
- and it preserves the deferred-intent policy baseline introduced for `trading_window_closed`.

Detailed evidence was recorded separately in:

- `docs/restart-from-zero-indicator-warmup-validation-2026-04-02.md`
