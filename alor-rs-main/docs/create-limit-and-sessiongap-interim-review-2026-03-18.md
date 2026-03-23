# Interim Review: `create:limit` Diagnostics And `session_gap` Recovery

Date: 2026-03-18

Deployment tag under current live review:

- `candidate-unify-a673741`

Related documents:

- `docs/live-incident-note-2026-03-17.md`
- `docs/market-buy-and-close-diagnostic-runbook.md`
- `docs/post-tz-followup-2026-03-18.md`

## 1. Executive Summary

This document is an interim technical review, not a closure report.

Two new technical specifications were executed only partially end-to-end:

1. shared `create:limit` / CWS transport diagnostics;
2. `session_gap_standalone` recovery semantics after transient `cws_error`.

Current high-confidence conclusions:

- the new gateway observability/telemetry patch is implemented and visible on the live stack;
- the `session_gap` recovery-semantics patch is implemented and locally test-complete;
- the baseline `create:market` path is stable on the current live line;
- the previously captured clean `marketable_limit` incident remains the strongest evidence that the residual failure is not `session_gap`-specific and is more likely attached to the shared `create:limit` / CWS path;
- the current live `marketable_limit` re-run on `candidate-unify-a673741` did not produce a clean transport verdict, because it exposed a separate runtime-side issue before the command reached the gateway limit-send path.

This is enough to justify specialist review now, while explicitly marking the remaining TZ scope as incomplete.

## 2. Scope Under Review

### 2.1 TZ1: shared `create:limit` / CWS transport diagnostics

Requested outcomes:

- add connection-instance identity into gateway logs;
- add transport counters/metrics;
- verify topology on VPS;
- run comparative live series:
  - `M` baseline `create:market`,
  - `L1` controlled `marketable_limit`,
  - `L2` passive limit create/cancel loop,
  - `T1/T2/T3` topology comparison;
- produce a diagnostic report with strengthened/weakened hypotheses.

### 2.2 TZ2: `session_gap` recovery semantics after transient `cws_error`

Requested outcomes:

- split transient transport failures from terminal business failures;
- avoid unconditional terminal `Blocked` on transient `cws_error`;
- add duplicate-risk guards;
- add tests and operator-facing notes.

## 3. Code Work Completed

## 3.1 Prior diagnostic strategy work already in place before this review

The following earlier work was already available and remained part of the evidence package:

- `a67d6c9` `feat(runtime): add marketable limit diagnostic mode`
- `39db59e` `fix(runtime): defer market buy entry until second bar`
- `6763484` `fix(runtime): use wall clock for live order timeouts`
- `840bf65` `fix(runtime): align live request ids with emitted commands`
- `9609e0a` `fix(runtime): align marketable limit state request ids`

That earlier work created the control strategy:

- `market_buy_and_close`
- `live_order_style = market | marketable_limit`

and enabled the original A/B live comparison.

## 3.2 TZ1 code scope completed on the current line

Current gateway patch:

- `340a24f` `feat(gateway): add cws transport diagnostics`

Implemented:

- `cws_connection_instance_id`
- `cws_connect_seq`
- `cws_reconnect_seq`
- `cws_connected_ts_utc`
- `cws_connect_total`
- `cws_reconnect_total`
- `cws_protocol_reset_total`
- `cws_limit_send_total`
- `cws_limit_error_total`
- `cws_pending_failed_total`

These fields were added to:

- gateway readiness;
- `cws_limit_send` logging;
- `cws_transport_failure` logging;
- `cws_fail_pending` logging.

## 3.3 TZ2 code scope completed on the current line

Current runtime patch:

- `a673741` `feat(session-gap): recover from transient cws transport errors`

Implemented:

- transient `cws_error` is no longer treated the same as terminal business rejection;
- new recoverable phase:
  - `EntryRecoveryVerificationPending`
- bounded verification path before returning to `Flat`;
- guards against duplicate-entry risk using late order/position evidence;
- strategy-side markers/logs for:
  - transport-transient failure,
  - recovery verification,
  - recovery to `Flat`,
  - terminal failure.

## 3.4 Local test status

Executed locally for the current line:

- `cargo test -p strategy-runtime --quiet`
- `cargo test -p alor-gateway --lib --quiet`
- `cargo test -p alor-gateway --test json_contract --quiet`
- `cargo test -p alor-gateway --test redis_transport --quiet`

Result:

- `strategy-runtime`: green
- `alor-gateway` targeted tests: green
- full `alor-gateway` suite was also confirmed green outside the sandboxed environment

## 4. Live Deployment And Topology

## 4.1 Current reviewed deployment

Live tag under review:

- `candidate-unify-a673741`

## 4.2 Topology actually observed on VPS

Observed topology during the current review was `T2`:

- `sessiongap` stack running;
- `hybrid` stack running simultaneously;
- each stack had its own:
  - gateway,
  - runtime,
  - Redis;
- both stacks remained active during the baseline `M` validation.

Observed `sessiongap` gateway readiness on the current line:

- `readiness = true`
- `gateway_phase = LiveReady`
- `cws_connection_instance_id` present
- `cws_connect_seq = 1`
- `cws_reconnect_seq = 0`
- `cws_connect_total = 1`
- `cws_reconnect_total = 0`
- `cws_protocol_reset_total = 0`
- `cws_limit_send_total = 0`
- `cws_limit_error_total = 0`
- `cws_pending_failed_total = 0`

Interpretation:

- the new transport-identity telemetry is live and usable;
- the reviewed deployment reached a clean post-restart gateway state before the controlled tests.

## 5. Confirmed Live Evidence

## 5.1 Historical clean `B` reproduction remains valid evidence

The strongest clean `create:limit` reproduction in the evidence package remains:

- timestamp:
  - `1773834181`
  - `2026-03-18 11:43:01 UTC`
  - `2026-03-18 14:43:01 MSK`
- strategy:
  - `market_buy_and_close_diag_marketable_limit`
- `request_id = 92925d49-5de7-5301-bb23-3b471cc2b7d0`
- action:
  - `place`
- result:
  - `status = error`
  - `error_code = cws_error`
  - `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
  - `broker_order_id = null`

Gateway path for that clean event showed:

- `command received`
- `cws_limit_send`
- `opcode = create:limit`
- immediate `cws_transport_failure`
- `cws_fail_pending`
- preserved correlation through `cws_request_guid`

Interpretation:

- this is still the cleanest reproduction of the target residual class;
- it materially weakens the hypothesis that the problem is unique to `session_gap_standalone`.

## 5.2 Current-line clean `M` pass under `T2`

After clearing stale diagnostic state and returning the broker position to flat, a clean `M` cycle was executed on the current line.

Entry:

- `request_id = a2f12657-d8de-5094-96df-48117e4abca1`
- command timestamp:
  - `1773859962`
  - `2026-03-18 18:52:42 UTC`
  - `2026-03-18 21:52:42 MSK`
- command action:
  - `market`
- gateway ack:
  - `status = accepted`
  - `broker_order_id = 2023555914316948892`
- resulting position:
  - `USDRUBF qty = 1.0`
  - `avg_price = 83.45`

Flatten:

- `request_id = 178cc38b-1972-5893-a9da-2e0e0b7adf6a`
- command timestamp:
  - `1773859981`
  - `2026-03-18 18:53:01 UTC`
  - `2026-03-18 21:53:01 MSK`
- command action:
  - `market`
- gateway ack:
  - `status = accepted`
  - `broker_order_id = 2023555914316948907`
- resulting position:
  - `USDRUBF qty = 0.0`

Runtime path:

- first live entry was deferred until the second bar, as designed;
- `PendingEntry -> InPosition -> PendingExit -> Done`
- final runtime state:
  - `Done`
  - `last_processed_bar_ts = 1773859920`

Gateway counters after the clean `M` cycle:

- `cws_protocol_reset_total = 0`
- `cws_reconnect_total = 0`
- `cws_limit_send_total = 0`

Interpretation:

- the baseline market path is currently stable under `T2`;
- no evidence was produced that the residual issue affects all live order types;
- this strengthens the case for a narrower `create:limit` / CWS path problem.

Note:

- `cmd.orders` confirmed the runtime emitted `action = market`;
- `broker.orders` still showed broker-side orders normalized as `order_type = limit`, which is consistent with earlier market-path evidence and does not invalidate the command-path classification.

## 6. Invalid Or Excluded Runs

## 6.1 Contaminated `M` attempt due to stale diagnostic state

An earlier `M` attempt on the current line was invalid and must be excluded from transport interpretation.

What happened:

- the diagnostic runtime restored stale in-position state instead of starting from clean `Flat`;
- the strategy emitted only a flatten sell without a fresh entry;
- that sell became a real live short.

Key timestamp:

- flatten order timestamp:
  - `1773857843`
  - `2026-03-18 18:17:23 UTC`
  - `2026-03-18 21:17:23 MSK`

Operational consequence:

- a real unintended short position was created;
- it later had to be flattened manually;
- the broker position returned to zero at:
  - `1773859558`
  - `2026-03-18 18:45:58 UTC`
  - `2026-03-18 21:45:58 MSK`

Interpretation:

- this run is not evidence about the market transport path;
- it is evidence that diagnostic runs must begin from:
  - flat broker position,
  - flat gateway snapshot,
  - empty diagnostic runtime state.

## 7. New Runtime-Side Finding From Current `L1`

## 7.1 Clean setup before `L1`

Before the current `marketable_limit` re-run:

- broker position was verified flat:
  - `USDRUBF qty = 0.0`
- gateway snapshot was verified flat:
  - `USDRUBF qty = 0.0`
- diagnostic state cleanup was executed:
  - `runtime.state.market_buy_and_close_diag.market.live.7502MIW`
  - `runtime.state.market_buy_and_close_diag.marketable_limit.live.7502MIW`

## 7.2 What happened on the current `L1` start

Current `market-b` runtime start:

- config:
  - `/configs/runtime.market-buy-close.live.marketable-limit.7502MIW.toml`

Readiness remained blocked waiting for live bootstrap:

- `bootstrap:not_ready`
- `bootstrap:missing_live_bar`

Later runtime-state evidence showed:

- `strategy_state = MarketLivePendingEntry`
- `request_guid = 1a7671cc-2f48-5510-baca-81b3a7e45314`
- `acked = false`
- runtime state timestamps around:
  - `1773860332`
  - `2026-03-18 18:58:52 UTC`
  - `2026-03-18 21:58:52 MSK`

## 7.3 Why this is important

At the same time, there was no corresponding fresh evidence of the command reaching the gateway limit path:

- no fresh `command received` for `market_buy_and_close_diag_marketable_limit`
- no fresh `cws_limit_send`
- no fresh limit-side `command_ack`
- gateway counters still showed:
  - `cws_limit_send_total = 0`
  - `cws_limit_error_total = 0`
  - `cws_protocol_reset_total = 0`

Interpretation:

- the current-line `L1` attempt did not yet test the gateway/CWS limit-send path;
- instead, it exposed a separate runtime-side issue:
  - `marketable_limit` diagnostic startup can enter `PendingEntry` before command emission is externally visible;
- this blocks clean live interpretation of the current `L1` result.

This finding should be treated separately from the already-confirmed historical clean `create:limit` reset reproduction.

## 8. Status Against The Current TZs

## 8.1 TZ1: shared `create:limit` / CWS transport diagnostics

### Completed

- connection-instance logging: done
- transport counters: done
- topology note for observed `T2`: done
- baseline `M` series:
  - clean pass demonstrated

### Partially completed

- `L1`:
  - historical clean reproduction exists
  - current-line re-run is not yet conclusive because of the runtime-side blocker above

### Not completed

- `L2` passive limit create/cancel loop
- `T1` isolation run
- `T3` expanded topology run
- final comparative diagnostic report with all planned series completed

## 8.2 TZ2: `session_gap` recovery semantics after transient `cws_error`

### Completed

- code implementation: done
- tests: done
- behavior split between transient transport and terminal errors: implemented

### Still pending operationally

- short operator note / runbook update
- broader live confidence beyond the already available incident evidence

## 9. Current Working Conclusions

## 9.1 What is already strong enough to rely on

1. The gateway observability patch is live and useful.

2. The `session_gap` recovery patch is code-complete and tested.

3. The live `create:market` path is stable on the reviewed line.

4. The previously captured clean `marketable_limit` failure still supports the shared-path hypothesis:

- not only `session_gap`,
- more likely shared `create:limit` / CWS transport behavior.

## 9.2 What is not yet strong enough to claim

1. We cannot yet claim that the current line has a fresh clean `L1` verdict.

2. We cannot yet claim topology sensitivity across `T1/T2/T3`.

3. We cannot yet close TZ1 as fully complete.

## 10. Specialist Review Questions

The review request should focus on these points:

1. Is the current evidence package already strong enough to continue treating the residual problem primarily as a shared `create:limit` / CWS path issue rather than a `session_gap`-specific issue?

2. Should the next step be:
- first fix the runtime-side `marketable_limit` startup/pending-entry anomaly,
- or first execute `L2` using a passive command-path loop that bypasses this runtime-side blocker?

3. Is it worth prioritizing `T1` topology isolation before returning to another full live `L1` re-run?

4. Is the current live evidence sufficient to review TZ2 as implemented, while leaving TZ1 diagnostics explicitly partial?

## 11. Recommended Interim Position

Recommended status to communicate externally:

- `TZ1 observability implementation`: complete
- `TZ1 live diagnostics`: partial
- `TZ2 recovery-semantics implementation`: complete
- `Overall status`: ready for specialist review, not ready for closure

## 12. Bottom Line

The current cycle produced real progress:

- new gateway transport diagnostics are deployed and visible live;
- the `session_gap` transient-error recovery policy was upgraded;
- a clean market baseline was reproduced on the current line;
- the strongest clean `create:limit` reproduction still points toward the shared limit/CWS path.

At the same time, the current cycle also uncovered a new blocker:

- the latest `marketable_limit` live re-run did not reach a clean gateway/CWS verdict because the runtime entered `PendingEntry` without a corresponding visible command reaching the gateway limit-send path.

That is exactly the point where specialist review is justified:

- there is enough evidence to review direction and prioritization;
- there is not yet enough evidence to call the full TZ set complete.

## 13. Specialist Review Outcome

An external technical review was received after the interim document was circulated.

Summary of the review outcome:

- the current document status is correct:
  - `ready for specialist review, not ready for closure`
- `TZ1` observability implementation was accepted as sufficiently complete in code;
- `TZ2` recovery-semantics implementation was accepted as sufficiently complete in code;
- the clean `M` baseline was accepted as a valid live pass on the current line;
- the historical clean `B` reproduction remains the main argument that the residual issue is not `session_gap`-specific and is more likely attached to the shared `create:limit` / CWS path;
- the fresh current-line `L1` re-run was explicitly not accepted as a valid gateway/CWS verdict because it hit the runtime-side blocker before a clean limit-send path could be observed.

Reviewer's key interpretation:

- the current working hypothesis should remain:
  - shared `create:limit` / CWS path issue
  - not a purely `session_gap`-specific issue
- the newly exposed runtime-side `marketable_limit` startup anomaly should be treated as a separate blocker and resolved independently from the shared transport-path investigation.

The review also confirmed that `TZ1` is still incomplete because the following are still missing:

- clean current-line `L1` verdict
- `L2` passive limit create/cancel loop
- `T1` isolation run
- `T3` expanded topology run
- final comparative diagnostic report

## 14. Agreed Next Execution Plan

The review produced a clear execution order.

### 14.1 First priority

Fix the runtime-side blocker in the `marketable_limit` diagnostic path.

Problem to isolate:

- the strategy enters `MarketLivePendingEntry`
- but there is no matching externally visible:
  - `cmd.orders`
  - `command received`
  - `cws_limit_send`
  - `command_ack`

What must be checked first:

- whether state can transition into `MarketLivePendingEntry` before actual command publication;
- whether `request_guid` generation diverges from command-emission flow in `marketable_limit` mode;
- whether bootstrap/live-guard gating can leave strategy state in pending mode without a real emitted command;
- whether `marketable_limit` startup can replay/restore into a pseudo-pending path without passing through the normal runtime-to-gateway boundary.

Acceptance criterion for this blocker fix:

- clean `L1` re-run with visible, ordered evidence:
  - `intent_emitted`
  - `cmd.orders`
  - `command received`
  - `cws_limit_send`
  - then either:
    - accepted/fill/flatten
    - or clean transport failure

### 14.2 Second priority

Repeat clean `L1` after the runtime-side blocker fix.

Goal:

- obtain a fresh current-line verdict on the `marketable_limit` path.

### 14.3 Fallback if `L1` remains noisy

If the runtime-side diagnostic mode continues to produce ambiguous startup behavior, do not spend a long live cycle on it.

Instead:

- move directly to `L2`
- use a passive limit create/cancel loop through the production command path

Reason:

- `L2` isolates the shared `create:limit` / CWS path with less dependence on the diagnostic strategy lifecycle.

### 14.4 Topology isolation after that

Run `T1` before broader topology expansion.

Priority order:

1. `T1` isolated run
2. `T2` comparison
3. `T3` expanded topology

Reason:

- `T1` answers the most important topology question first:
  - does the problem remain even in clean isolated topology?

## 15. Updated External Status

After specialist review, the recommended external status remains:

- `TZ1 observability implementation`: complete
- `TZ1 live diagnostics`: partial
- `TZ2 recovery-semantics implementation`: complete
- `TZ2 live confidence`: partial but sufficient for code review
- `Overall`: ready for review and next-step decision, not ready for closure
