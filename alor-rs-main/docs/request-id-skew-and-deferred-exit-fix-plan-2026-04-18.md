# Request-Id Skew And Deferred Exit Fix Plan

Date: 2026-04-18

Related incident:

- [live-incident-note-2026-04-17-trading-hybrid-bo-exit-break2-request-id-skew.md](./live-incident-note-2026-04-17-trading-hybrid-bo-exit-break2-request-id-skew.md)

Related broader plan:

- [intent-path-unification-fix-plan-2026-04-17.md](./intent-path-unification-fix-plan-2026-04-17.md)

## 1. Purpose

This document defines the follow-up fix line for the `trading_window_closed -> stale pending_exit_active` incident observed on `trading-hybrid` on 2026-04-17.

The fix is intentionally broader than one stack because the failure class is shared:

- strategy-owned pending state computes one request id
- runtime emits another request id after timestamp normalization
- ack reconciliation misses the strategy-owned pending branch

## 2. Agreed product decisions

The following decisions are fixed:

1. `BreakoutEodExit` for `hybrid` remains at the current effective behavior:
   - `23:30 MSK`
2. exits blocked by break / closed trading window should be deferred in `runtime` before emit, not by relying on gateway reject as the normal path
3. request-id parity is a mandatory core fix, not an optional improvement next to pre-emit deferral
4. the bug class must be treated as potentially shared across strategies
5. rollout planning must explicitly decide whether the patch should be validated with a clean restart / `from zero` state

## 3. Problem statement

Current flow allows this failure pattern:

1. strategy constructs pending exit state using `created_ts_utc = raw bar close ts`
2. runtime emits `OrderCommand` using `created_ts_utc = normalize_event_ts(raw ts)`
3. request ids diverge
4. gateway reject ack matches runtime-emitted id but not strategy-owned pending id
5. strategy never clears pending exit
6. later exits are suppressed as `pending_exit_active`

This is especially dangerous when the first exit attempt happens during:

- break window
- temporary trading window closure
- other non-open scheduler states

because the normal fallback path should be:

- defer and retry

not:

- stale pending forever

## 4. Scope

### Directly affected

- `strategy-runtime`
- `trading-hybrid`

### Must be audited for same class

- `sessiongap`
- `trading-alor-usdrubf`
- any other strategy that stores pending request ids inside strategy state before final command emission

### Possibly touched

- shared request-id generation helpers
- live/deferred entry and exit helpers
- tests around `trading_window_closed`

## 5. Work packages

## WP1. Eliminate request-id skew between strategy state and emitted command

### Objective

Ensure that strategy-owned pending request ids and runtime-emitted command request ids are identical.

This work package is the core fix for the incident, not a nice-to-have add-on.

Pre-emit deferral for closed-window exits does not replace request-id parity:

- the same skew can affect other live paths,
- the same skew can break safety-net reject handling,
- and the same skew can surface outside break-window exit logic.

### Target model

Preferred direction:

- final live request-id generation lives in one host/runtime source of truth
- strategy code does not precompute the final emitted live request id on its own
- runtime decides:
  - emit now or defer
  - final `created_ts_utc`
  - final `request_id`
- only after that decision does strategy-owned live pending state receive the exact emitted request id

Temporary fallback only if the preferred model is too invasive for the first patch:

- ensure strategy code uses the same normalized `created_ts_utc` as runtime command emission

This fallback is acceptable only as a transitional implementation detail, not as the desired long-term host contract.

### Acceptance criteria

- `pending_entry_request_id`
- `pending_exit_request_id`
- `pending_tp_request_id`
- `pending_sl_request_id`

must always equal the request id of the emitted `OrderCommand`.

Additionally:

- it must be impossible to construct a path where strategy-owned pending state waits on one request id while runtime emits another
- or such skew must be explicitly detected and repaired before stale pending state can survive

## WP2. Defer window-closed exits before emit in runtime

### Objective

Stop using gateway reject as the normal control path for exits during:

- breaks
- known closed windows

### Direction

When an exit intent is generated and runtime can already tell the trading window is closed:

- do not emit the command
- do not create live pending request state
- convert to strategy-owned deferred exit immediately
- persist state in deferred form

This should be the standard path for recoverable exit-window situations.

### Acceptance criteria

- exit in `Break1` / `Break2` or other closed window does not send a broker command
- no `pending_exit_request_id` is left behind from a blocked emit
- strategy records deferred exit and retries after window reopen
- this path must use the final host/runtime request-id semantics from WP1 rather than strategy-local precomputed ids

## WP3. Preserve explicit gateway reject handling as a safety net

### Objective

Even after pre-emit deferral, keep the reject path correct for any residual or race-case `trading_window_closed`.

### Acceptance criteria

- if gateway still returns `trading_window_closed`
- and ack belongs to an exit
- strategy clears live pending exit
- strategy enters deferred exit
- no stale `pending_exit_active` remains

This remains a safety-net path:

- normal recoverable closed-window exits should be handled before emit by WP2
- residual gateway `trading_window_closed` handling must still converge correctly

## WP4. Add operator-visible skew detection

### Objective

If a request-id skew or lineage mismatch ever appears again, make it obvious in logs instead of silently missing the intended strategy-owned pending branch.

### Direction

Add explicit operator-visible diagnostics for cases where:

- strategy-owned pending request id exists
- an ack / reject arrives for a related live command path
- but the ids do not match the expected strategy-owned pending id

Recommended event shape:

- `pending_request_id_skew_detected`
- `strategy_pending_request_id=...`
- `emitted_request_id=...`
- `intent_class=...`
- `owner=...`
- `cycle_id=...`

### Acceptance criteria

- skew or lineage mismatch cannot remain silent
- operator logs make it clear whether the problem is:
  - impossible by construction after WP1
  - or detected explicitly as an invariant violation

## WP5. Audit all strategy-owned pending request paths

### Objective

Treat this as a shared class bug, not a hybrid-only one.

### Audit targets

- entry pending ids
- exit pending ids
- TP / SL pending ids
- deferred entry / deferred exit original request ids

### Systems

- `trading-hybrid`
- `trading-sessiongap`
- `trading-alor-usdrubf`
- any other live path using deterministic request ids inside strategy-owned state

### Acceptance criteria

- no strategy stores a request id that can diverge from emitted command request id

## WP6. Keep effective `hybrid` BO EOD at `23:30`

### Objective

Do not change the actual `BreakoutEodExit` behavior in this fix line.

### Notes

- the current breakout engine uses `23:30`
- session config may still contain a later close, but that is not the active breakout-EOD trigger

### Acceptance criteria

- patched behavior preserves `23:30` `BreakoutEodExit`
- no accidental drift to `23:49` appears in live behavior

## WP7. Validation and restart policy

### Objective

Decide explicitly whether patched validation should be:

- normal controlled rollout
- or controlled rollout with clean runtime state / `from zero`

### Recommendation

For this class of pending-state bug, clean validation is safer if we want confidence that:

- no stale persisted pending ids survive
- deferred exit behavior is observed from a known clean state

So preferred validation path is:

- rebuild patched images
- controlled restart
- clean validation / `from zero` for the primary target stack

Target-stack requirement:

- `trading-hybrid` should be validated with clean runtime state / `from zero`
- account must be flat
- no working orders
- no stop orders

Broader stack policy:

- all affected stacks should be restarted in a controlled way if the patch touches shared runtime logic materially
- but `from zero` is mandatory at least for the primary validation target stack
- wider `from zero` rollout for every stack is optional and should be decided operationally

### Acceptance criteria

- rollout runbook explicitly states whether validation uses preserved state or clean state
- operators know how to interpret observed behavior under that choice

## 6. Test plan

### Required unit / integration tests

1. `hybrid` exit during break:
   - generate `BreakoutStop1Short`
   - runtime must defer before emit
   - no live pending exit id left behind

2. `hybrid` residual gateway reject safety net:
   - simulate `trading_window_closed`
   - pending exit must clear
   - deferred exit must appear

3. request-id parity:
   - strategy-owned pending request id must equal emitted `OrderCommand.request_id`

4. skew-specific invariant test:
   - simulate a path where strategy-owned pending request state and emitted command request id would previously diverge
   - verify that:
     - such divergence is impossible after the fix
     - or it is explicitly detected and repaired
   - verify that the strategy does not remain in stale `pending_exit_active`

5. anti-regression:
   - normal open-market exits still emit immediately
   - `BreakoutEodExit` remains `23:30`

6. shared-class audit coverage:
   - at least one non-hybrid strategy path checked for request-id skew absence

## 7. Rollout recommendation

Recommended order:

1. implement patch
2. run local tests
3. review
4. rebuild images
5. perform controlled live rollout
6. run clean validation / `from zero` for `trading-hybrid`
7. run patched micro soak
8. only then expand confidence further

## 8. Patched micro soak focus

Main validation targets:

1. `trading-hybrid`

- `BreakoutStop1Short` or analogous exit during break
- confirm runtime defers before emit
- confirm no stale `pending_exit_active`
- confirm later reissue works after break reopen
- confirm `BreakoutEodExit` still works at `23:30`

2. `trading-sessiongap`

- verify no regression in one-shot exit lifecycle

3. `trading-alor-usdrubf`

- verify no regression in request-id / deferred handling on market path

## 9. Practical rollout recommendation

Recommended staged rollout:

### Stage 1

Implement:

- request-id parity core fix
- pre-emit defer for closed-window exits
- residual reject safety net
- skew-specific tests
- operator-visible skew detection

### Stage 2

Perform clean validation for `trading-hybrid`:

- fresh runtime state
- fresh consumer position if needed by the runbook
- flat account
- no working orders
- no stop orders

### Stage 3

Run patched micro soak for `3-5` sessions focused on:

- break-window exits
- deferred exit reissue
- absence of stale `pending_exit_active`
- preserving `BreakoutEodExit = 23:30`

### Stage 4

If clean:

- expand confidence to the broader shared class
- then decide whether further normalization work is needed

## 10. Success condition

This fix line is successful if all of the following are true:

- no strategy can retain stale pending exit solely because of request-id skew
- exit during break enters deferred state before emit
- fallback reject handling still clears pending state correctly
- `hybrid` no longer suppresses valid later exits after one break-window reject
- `hybrid` keeps `BreakoutEodExit` at `23:30`
- patched rollout validation is run with an explicitly documented restart / `from zero` decision
