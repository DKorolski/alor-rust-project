# Post-TZ Follow-Up: Live Incidents And Diagnostic Work

Date: 2026-03-18

Related documents:

- `docs/live-incident-note-2026-03-17.md`
- `docs/market-buy-and-close-diagnostic-runbook.md`

## 1. Executive Summary

Since the last technical specification, the work split into two tracks:

1. code changes needed to build a clean control strategy for limit-path diagnostics;
2. live validation needed to compare `create:market` against `create:limit`.

Current conclusion:

- the baseline market path is stable;
- the controlled `marketable_limit` path reproduced the same residual CWS transport reset that had previously been seen on `session_gap_standalone`;
- this materially shifts the primary hypothesis away from "`session_gap` only" and toward the shared `create:limit` / CWS transport path;
- the `hybrid` recovered-position ownership issue still looks separate and was not explained by this work.

## 2. Scope Since The Last TZ

The requested scope after the prior TZ was:

- extend `market_buy_and_close` with a live execution style switch;
- support both `market` and `marketable_limit`;
- add structured logging, config fields, and tests;
- use the strategy as a control path for live A/B comparison;
- compare:
  - `create:market`,
  - controlled `create:limit`,
  - native `session_gap` `create:limit`.

That scope was completed in code and exercised live.

## 3. Code Changes Completed

### 3.1 Diagnostic mode for `market_buy_and_close`

Implemented:

- `live_order_style = "market" | "marketable_limit"`
- `marketable_limit_offset_ticks`
- `Intent::Market` for baseline live market mode
- `Intent::Place` for controlled live marketable-limit mode
- structured logs with:
  - `live_order_style`
  - `request_id`
  - `side`
  - `qty`
  - `price` for `marketable_limit`
  - `reason = entry | flatten`

Main commit:

- `a67d6c9` `feat(runtime): add marketable limit diagnostic mode`

### 3.2 Follow-up runtime fixes found during live validation

The live experiments surfaced several secondary runtime issues. These were fixed incrementally:

1. defer first live entry until the second live bar
- reason: avoid immediate drop on first bar after restart
- commit: `39db59e`
- message: `fix(runtime): defer market buy entry until second bar`

2. use wall clock for live order timeouts
- reason: bar timestamps were producing false `entry_ack_timeout_ms` / `exit_ack_timeout_ms`
- applied to:
  - `market_buy_and_close`
  - `session_gap_standalone`
- commit: `6763484`
- message: `fix(runtime): use wall clock for live order timeouts`

3. align live request ids with emitted commands
- reason: strategy state and emitted command ids could diverge
- commit: `840bf65`
- message: `fix(runtime): align live request ids with emitted commands`

4. align `marketable_limit` state request ids with runtime `Intent::Place` serialization
- reason: `marketable_limit` still used a state-side request-id scheme that did not match the runtime-published command id
- commit: `9609e0a`
- message: `fix(runtime): align marketable limit state request ids`

### 3.3 Test status

After the latest follow-up fix:

- `cargo test -p strategy-runtime --quiet`
- result: `114 passed`

## 4. Live Validation Performed

## 4.1 Baseline reference already available before the A/B work

Before the dedicated A/B comparison, the stack already had:

- manual production-path `create:limit` validation;
- runtime-native B2 validation on `session_gap`;
- confirmed end-to-end success when transport remained healthy.

This remained the baseline for comparing later failures.

## 4.2 Test Group A: `market_buy_and_close` with `live_order_style = "market"`

Result: `PASS`

Clean live evidence on 2026-03-18:

- runtime config:
  - `/configs/runtime.market-buy-close.live.market.7502MIW.toml`
- readiness:
  - `readiness = true`
  - `live_guard = ALLOWED`

Successful cycle:

- entry request:
  - `request_id = 788bbfbc-4a4a-55d9-8c75-6250495fd117`
- flatten request:
  - `request_id = ad0836ba-6125-5490-97fe-19b947670d31`
- both commands were accepted;
- both orders reached `working -> filled`;
- broker position returned to `USDRUBF qty = 0.0`;
- runtime state reached `Done`.

Interpretation:

- the baseline `create:market` path is stable on the live stack;
- the follow-up timeout and request-id fixes were necessary to make this result clean and reproducible.

## 4.3 Test Group B: `market_buy_and_close` with `live_order_style = "marketable_limit"`

Result: `FAIL` in the exact target way

Clean target incident:

- timestamp:
  - `1773834181` -> `2026-03-18 11:43:01 UTC` / `2026-03-18 14:43:01 MSK`
- command:
  - `request_id = 92925d49-5de7-5301-bb23-3b471cc2b7d0`
  - `strategy_id = market_buy_and_close_diag_marketable_limit`
  - `action = place`
  - `price = 83.14`

Observed gateway path:

- `command received`
- `action="cws_limit_send"`
- `opcode="create:limit"`
- `cws_guid="92925d49-5de7-5301-bb23-3b471cc2b7d0"`
- immediate transport failure:
  - `disconnect_kind="protocol_reset_without_close_handshake"`
- pending request failure logged through:
  - `cws_fail_pending`
- error ack published with preserved correlation:
  - `request_id = 92925d49-5de7-5301-bb23-3b471cc2b7d0`
  - `status = error`
  - `error_code = cws_error`
  - `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
  - `cws_request_guid = 92925d49-5de7-5301-bb23-3b471cc2b7d0`
  - `broker_order_id = null`

Interpretation:

- this reproduces the same residual failure class outside `session_gap`;
- the issue is therefore not isolated to `session_gap` timing/state alone;
- the evidence is consistent with a shared `create:limit` / CWS transport problem.

## 4.4 Secondary issue found during `B`

The first clean `B` reproduction also exposed a secondary strategy-side correlation issue:

- strategy state held one `request_guid`;
- the emitted `cmd.orders` / `cmd.acks` path used another request id;
- because of that mismatch, the strategy later blocked on a timeout instead of treating the transport error ack as belonging to the pending entry.

This did not invalidate the main conclusion from `B`.

What it meant:

- the transport reset itself was real and already proven;
- the later `Blocked` state was partly a diagnostic-strategy bug, not the root transport symptom.

That bug was fixed in:

- `9609e0a` `fix(runtime): align marketable limit state request ids`

## 4.5 Later `B` re-smoke after `9609e0a`

A later live re-smoke on the `9609e0a` line did not produce a clean standalone verification artifact.

What happened:

- the stack later showed fresh `session_gap_standalone` activity;
- a new live `session_gap` entry appeared in the shared streams;
- the diagnostic runtime no longer represented a clean flat-start `market_buy_and_close_diag_marketable_limit` control cycle.

Interpretation:

- `9609e0a` is code-complete and test-complete;
- a clean live re-check of that narrow follow-up fix is still desirable;
- this does not change the already confirmed main finding from the earlier clean `B` incident.

## 5. Incident Interpretation

The results now match Scenario 2 from the diagnostic runbook:

- `A = stable`
- `B = also shows residual resets`

Most likely interpretation:

- the remaining issue is closer to:
  - `create:limit`,
  - CWS transport/session behavior around live limit sends,
  - broker-side limit handling or session interference around that path;
- not only to `session_gap`-specific signal timing or state transitions.

At the same time:

- `session_gap_standalone` still needs recovery-semantics work after transient `cws_error`;
- the gateway observability fix has already proven effective on live failures;
- the `hybrid` ownership/reconciliation issue remains separate.

## 6. Status Against The Requested Deliverables

### 6.1 Code deliverables

Status: `DONE`

- `market_buy_and_close` supports `live_order_style`
- `marketable_limit` uses `Intent::Place`
- flatten path is supported
- old market mode remains available

### 6.2 Test deliverables

Status: `DONE`

- config parsing and defaults covered
- market mode regression covered
- marketable-limit entry and flatten covered
- latest `strategy-runtime` tests pass

### 6.3 Operational diagnostic deliverable

Status: `DONE`

We now have a reproducible diagnostic control strategy that can compare:

- baseline `create:market`
- controlled `create:limit`
- native `session_gap` `create:limit`

### 6.4 Incident-level conclusion

Status: `CONFIRMED`

The residual reset is reproducible on the controlled `marketable_limit` path, so the main investigation should now focus on the shared limit/CWS path rather than treating the issue as `session_gap`-only.

## 7. Open Items

1. perform one clean flat-start live re-smoke of `9609e0a`
- goal: confirm that `marketable_limit` request-id correlation is now clean end-to-end

2. continue root-cause investigation around `create:limit`
- focus:
  - CWS transport lifecycle around send
  - session resets
  - reconnect proximity
  - broker-side handling during limit order creation

3. separately investigate `session_gap` recovery semantics
- specifically:
  - how to recover from transient `cws_error`
  - whether `Blocked` should remain terminal in this class of failure

4. keep `hybrid` as a separate incident track
- current evidence does not support merging it with the `sessiongap` / `create:limit` issue

## 8. Bottom Line

The period since the last TZ produced a usable control strategy, a validated baseline market path, and a controlled reproduction of the residual live `create:limit` transport reset.

That is enough to change the working diagnosis:

- the unresolved issue is no longer best described as a `session_gap`-only anomaly;
- it is now better treated as a shared limit-path transport problem with a separate strategy-recovery follow-up.
