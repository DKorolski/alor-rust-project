# Interim Report: Instrumented `create:limit` / `delete:limit` Probe

Date: 2026-03-25

Related documents:

- `docs/create-limit-and-sessiongap-review-ready-2026-03-23.md`
- `docs/create-limit-topology-diagnostic-runbook.md`
- `docs/create-limit-and-sessiongap-specialist-handoff-2026-03-23.md`

## 1. Purpose

This document records the first focused live result after the new transport-ordering instrumentation rollout.

The goal of this phase was narrower than the earlier comparative matrix:

- deploy `P0` gateway instrumentation from the engineering task;
- run a controlled narrow passive-limit probe instead of a broad live rerun;
- verify whether the residual incident class changes when the stacks use different broker refresh tokens;
- capture fresh evidence around:
  - `create:limit`
  - `delete:limit`
  - `response` / `order event` correlation
  - transport resets on the limit-order control path.

This document is an interim engineering result.

It is intended for:

- project-management status review;
- engineering root-cause analysis;
- follow-on planning for the next narrow validation step.

It is not intended as closure.

## 2. Scope Executed From The Task

The following parts of the engineering task were completed in this phase.

### 2.1 Gateway instrumentation

Implemented and deployed:

- single-socket `send_seq`
- single-socket `recv_seq`
- inbound message classification:
  - `RequestResponse`
  - `DomainEvent`
  - `Transport`
  - `Unknown`
- stricter pending completion:
  - pending request is no longer closed by any frame carrying `guid` / `requestGuid`
  - pending completion is limited to explicit response-class messages
- trace propagation into command processing logs
- supervisor visibility for:
  - `event_request_id`
  - `state_request_id`
  - `request_map_hit`

Key gateway commit:

- `0c74996` `feat(gateway): trace cws limit response ordering`

Operational helper commits:

- `a3f75f3` `chore(scripts): add limit diagnostic helper`
- `60f3c64` `fix(scripts): trace post capture files`

### 2.2 Live deployment status

Deployed on VPS:

- diagnostic gateway image on both live stacks
- `sessiongap` runtime left on existing reviewed runtime image
- `hybrid` runtime kept on existing reviewed runtime image

An unrelated runtime-container misconfiguration was found during rollout:

- `hybrid` runtime container had been created from the wrong compose working directory
- it was recreated from `/opt/trading-hybrid`
- after correction it was healthy again

This environment fix was operationally necessary before collecting comparable live evidence.

### 2.3 Broker auth-context discriminator

The stacks were reconfigured to use different broker refresh tokens.

Observed via readiness:

- `sessiongap` `auth_principal_fingerprint = sha256:2dcaa06a8677f87a`
- `hybrid` `auth_principal_fingerprint = sha256:c5a5ef042fbb04da`

Interpretation:

- the fresh narrow probe was executed under different auth principals
- this materially weakens "`shared token alone`" as the main explanation for the residual incident class
- it does not by itself prove that all auth-context interference is impossible.

## 3. How The Probe Was Executed

### 3.1 Preflight

Before the focused probe:

- both gateways were restarted cleanly after token separation;
- both gateways returned to:
  - `readiness = true`
  - `gateway_phase = LiveReady`;
- both runtimes were healthy;
- artifact capture directories were created on VPS under:
  - `/opt/diag-captures/...`

### 3.2 Probe method

The narrow live probe used the helper script:

- `scripts/limit_diag.sh`
- deployed on VPS as `/opt/limit_diag.sh`

The intended controlled scenario was:

1. capture `pre`
2. submit one passive `sessiongap` `create:limit`
3. wait for ack / order lifecycle
4. if order remains passive, submit `delete:limit`
5. capture `post`

### 3.3 Important execution note

The first fresh `buy limit` probe after script rollout used a buy price above the live market.

Observed market context:

- market price was around `80.80`
- submitted buy limit was `81.71`

Therefore that first command was marketable, not passive.

That first attempt is useful as an operational sanity check, but it is not the valid passive `L2` artifact for this phase.

## 4. Observed Live Results

## 4.1 Token-separation baseline

Confirmed:

- both stacks were healthy after token update;
- both stacks used different `auth_principal_fingerprint`;
- no immediate new failure was introduced by token separation itself.

Interpretation:

- the environment was suitable for a fresh focused probe;
- the new evidence is not a same-refresh-token-only artifact.

## 4.2 Invalid passive probe due to marketable price

Fresh command:

- `request_id = 104bc338-83ec-4f2c-897e-7fe7bc91cc33`
- `price = 81.71`
- side:
  - `buy`

Observed result:

- command ack:
  - `accepted`
- `broker_order_id = 2023555931497043528`
- broker order path:
  - `working`
  - then `filled`
- resulting position:
  - `USDRUBF qty = 1.0`
  - `avg_price = 80.81`

Interpretation:

- this was not a valid passive `L2` probe because the limit price was marketable;
- however it confirmed that the instrumented path can still carry a clean accepted `create:limit` round-trip under separate tokens;
- it also reconfirmed that:
  - direct ack and lifecycle event are distinct artifacts
  - `working` can appear before a later fill event on the same order path.

The position was later flattened manually for safety before the next passive probe.

## 4.3 Valid passive `create:limit` probe

Fresh passive create:

- `request_id = f8d63638-5239-49bf-84bd-0f6fe985912f`
- price:
  - `80.2`
- side:
  - `buy`

Gateway/control-path observations:

- `command received`
- `cws_limit_send`
- `cws send opcode="create:limit"`
- `cws response matched pending request`
- `command ack published`

Ack result:

- `status = accepted`
- `broker_order_id = 2023555931497048623`
- `cws_message = "An order '2023555931497048623' has been created."`

Broker order evidence:

- order reached:
  - `status = working`
- `filled = 0.0`
- `price = 80.2`

Position evidence:

- position remained flat during this passive order:
  - no fill on the passive probe itself

Interpretation:

- under the new instrumented build and under separate broker auth principals, passive `create:limit` can still succeed cleanly;
- the strongest residual failure in this fresh run is therefore not a blanket `create:limit` failure.

## 4.4 Correlation / ordering observation on the passive create

Fresh supervisor-side observation for the passive create:

- the order event initially arrived with:
  - `event_request_id = null`
- gateway then logged:
  - `order event request_id updated from state`
- final supervisor log showed:
  - `request_map_hit = true`
  - request id restored from state/request map

Interpretation:

- this is a direct fresh example that order lifecycle events and direct command responses do not naturally arrive in the same correlation shape;
- request identity for the lifecycle event can require local state/request-map recovery;
- this strongly supports the engineering task focus on:
  - response/event separation
  - request ownership
  - state-machine assumptions around ordering.

## 4.5 First `delete:limit` reproduction on the passive order

First cancel attempt:

- `request_id = 1fb09326-b31c-429d-a4c4-f6091110d6c6`
- target order:
  - `2023555931497048623`

Observed result:

- `status = error`
- `error_code = cws_error`
- `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`

Gateway path showed:

- `cws send opcode="delete:limit"`
- immediate `cws_transport_failure`
- `cws_fail_pending`
- `command ack published`

Interpretation:

- the fresh passive probe reproduced the residual incident class specifically on `delete:limit`;
- this moves the current strongest live failure focus further toward the limit-order cancel/control path.

## 4.6 Second `delete:limit` reproduction on the same order

Second cancel attempt:

- `request_id = 43f28f4d-cbee-4f39-bc3d-9d5af60c5419`
- target order unchanged:
  - `2023555931497048623`

Observed result:

- again:
  - `status = error`
  - `error_code = cws_error`
  - `protocol_reset_without_close_handshake`

At the same time:

- `broker.orders` still showed the passive order as `working`
- `broker.positions` remained flat

Interpretation:

- this was not a one-off single cancel anomaly;
- repeated `delete:limit` on the same passive working order reproduced the same transport/control failure class again.

## 4.7 Manual broker-terminal cancellation and terminal state

Because the passive order remained live after repeated gateway-side cancel failures, it was manually canceled by the operator in the broker terminal.

After that manual action, broker order stream showed:

- `order_id = 2023555931497048623`
- terminal:
  - `status = canceled`
- `filled = 0.0`
- position remained:
  - `qty = 0.0`

Important correlation detail:

- the terminal `canceled` order event carried:
  - `request_id = 43f28f4d-cbee-4f39-bc3d-9d5af60c5419`

Interpretation:

- the order did eventually reach a clean terminal state;
- however the exact causality of the final `canceled` event remains ambiguous:
  - it may reflect the broker-terminal manual action,
  - or a delayed broker-side completion/correlation of the second `delete:limit` request after transport reset,
  - or a combination of both.

This ambiguity is itself relevant to the engineering task, because it reinforces that:

- direct command ack and eventual order lifecycle should not be treated as the same source of truth;
- transport failure on the direct response path does not prove that no downstream business action occurred.

## 4.8 Post-failure CWS auth recovery observation

After later incident-driven reconnect attempts on the same diagnostic line, both gateways entered repeated CWS authorization failures:

- `httpCode = 401`
- `message = "Invalid JWT token!"`
- `cws_authorized = false`

Important operational observation:

- the configured `refresh_token` values were not changed;
- restarting only `alor-gateway` with the same `refresh_token` restored both stacks to:
  - `readiness = true`
  - `gateway_phase = LiveReady`
  - `cws_authorized = true`

Interpretation:

- this does not look like a permanently invalid `refresh_token`;
- it is more consistent with a stale cached `access_token` remaining in memory across the failing reconnect loop;
- restart likely recovered the stacks by forcing a fresh OAuth access-token refresh from the same unchanged `refresh_token`.

Code follow-up implemented after this observation:

- cached `access_token` is now invalidated on explicit CWS authorize `401`;
- this allows the next reconnect attempt to fetch a fresh token without requiring a process restart.

## 5. What Was Verified Against The Task

## 5.1 Completed in this phase

1. `P0` transport-ordering instrumentation was implemented and deployed.

2. Separate-principal probe mode became available and was executed live.

3. A fresh valid passive `create:limit` was observed to complete cleanly through:
   - send
   - accepted ack
   - `working` order state

4. Fresh repeated `delete:limit` attempts on that same passive working order reproduced:
   - `cws_error`
   - `protocol_reset_without_close_handshake`

5. Fresh correlation evidence confirmed that:
   - lifecycle `order` event may initially lack `request_id`
   - gateway recovers correlation from state/request-map

6. Final operational state was restored to safe baseline:
   - order terminal `canceled`
   - position `0`

## 5.2 Not yet completed in this phase

1. A final `PASS` / `REPRO` memo with side-by-side reconstructed chronology in the exact task template.

2. A proof of whether the final terminal `canceled` event was caused by:
   - manual broker-terminal action,
   - delayed broker-side processing of the failed `delete:limit`,
   - or both.

3. Diagnostic single-writer lock experiment.

4. Final proof whether the residual failure is:
   - primarily client-side ordering/correlation,
   - primarily broker/CWS transport,
   - or combined.
5. Final confirmation whether the new auth-cache invalidation removes the repeated post-failure `401 Invalid JWT token!` loop in future incidents.

## 6. What This Interim Result Strengthens

The fresh result materially strengthens the following readout:

- the residual incident class is no longer best described as shared-token-only behavior;
- passive `create:limit` can still succeed cleanly under separate principals;
- the strongest fresh reproduced failing path is now:
  - `delete:limit`
  - on an already accepted passive working order;
- direct response path and lifecycle path still show non-trivial correlation behavior;
- a combined explanation remains plausible:
  - transport/control instability exists,
  - and client-side correlation/ownership assumptions still matter operationally.

## 7. What This Interim Result Weakens

This phase further weakens:

- "`shared refresh token alone`" as the primary explanation;
- "`every create:limit always fails`" as the current leading framing;
- a simplistic interpretation that a transport-side cancel error necessarily means "no downstream cancel semantics happened".

## 8. Current Best Interim Conclusion

Fresh instrumented live evidence under separate broker auth principals shows:

- passive `create:limit` can pass;
- repeated `delete:limit` can still reproduce the same `cws_error` / protocol-reset incident class;
- order-event correlation still requires local state/request-map recovery;
- final terminal state can arrive later and with ambiguous causality after direct cancel-path transport failure.

The strongest current fresh framing is therefore:

- the residual incident class remains a shared CWS limit-order control-path issue;
- in the fresh instrumented run, the most reproducible failing step is `delete:limit`;
- separate tokens do not eliminate the problem class;
- response/event correlation and transport failure handling remain central engineering review targets.
- repeated post-failure `401 Invalid JWT token!` does not currently point to a dead refresh token; it is more plausibly a cached-access-token recovery problem.

## 9. Recommended Next Step

Do not broaden live scope immediately.

Use this run as the primary fresh interim artifact for engineering review, then choose one narrow next action:

1. reconstruct a formal chronology for this exact run:
   - place accepted
   - working observed
   - cancel repro
   - cancel repro again
   - terminal canceled observed later
2. review whether the final `canceled` event after repeated cancel reset implies:
   - delayed downstream success after transport error,
   - or merely correlation overlap with the manual terminal action
3. only then decide whether the next step should be:
   - a diagnostic single-writer lock,
   - additional handler-level ordering logs,
   - or a broker-facing escalation focused specifically on `delete:limit`.
