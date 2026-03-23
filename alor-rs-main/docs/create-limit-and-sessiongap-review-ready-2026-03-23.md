# Review-Ready Report: `create:limit` Diagnostics And `session_gap` Recovery

Date: 2026-03-23

Related documents:

- `docs/live-incident-note-2026-03-17.md`
- `docs/create-limit-and-sessiongap-interim-review-2026-03-18.md`
- `docs/post-tz-followup-2026-03-18.md`
- `docs/market-buy-and-close-diagnostic-runbook.md`

## 1. Executive Summary

This document is the current review-ready status report for two connected workstreams:

1. shared `create:limit` / CWS transport diagnostics;
2. `session_gap_standalone` recovery semantics after transient `cws_error`.

Current conclusion:

- `TZ1` observability implementation is complete in code and verified live;
- `TZ2` recovery-semantics implementation is complete in code and locally test-complete;
- the baseline live `create:market` path remains stable on the reviewed line;
- the earlier runtime-side `marketable_limit` blocker was real, was fixed, and no longer masks the live result;
- after that fix, the first comparative live matrix produced:
  - fresh `L1 @ T2` reproduction,
  - clean `L2 @ T2` reproduction,
  - clean `L2 @ T1` pass,
  - clean `L2 @ T3` reproduction;
- late discriminator probes then showed:
  - clean gateway-only coexistence passes,
  - `sessiongap runtime ON / hybrid runtime OFF` mixed result:
    - clean `place`
    - first `delete:limit` reproduction
    - clean retry after gateway recovery
  - clean `hybrid runtime ON / sessiongap runtime OFF` pass,
  - clean `both runtimes ON` pass;
- the residual failure is therefore not `session_gap`-specific and is not explained by a simple deterministic topology switch;
- the strongest current hypothesis is now:
  - intermittent shared CWS limit-order control-plane instability
  - with topology/runtime/timing sensitivity.

This is sufficient for specialist review now.

It is not sufficient for closure because root-cause narrowing is still incomplete:

- the exact runtime/timing trigger is still unproven;
- no remedial fix has yet been validated against the narrowed hypothesis;
- `TZ2` still has only partial live confidence outside the entry-side transient scenario.

## 2. Scope Under Review

## 2.1 TZ1: shared `create:limit` / CWS transport diagnostics

Requested outcomes:

- add connection-instance identity into gateway logs;
- add transport counters and readiness visibility;
- verify topology on VPS;
- run comparative live series:
  - `M` baseline `create:market`
  - `L1` controlled `marketable_limit`
  - `L2` passive limit create/cancel loop
  - `T1/T2/T3` topology comparison
- produce a strengthened diagnostic hypothesis.

## 2.2 TZ2: `session_gap_standalone` recovery semantics after transient `cws_error`

Requested outcomes:

- split transient transport failures from terminal business failures;
- avoid unconditional terminal `Blocked` on transient `cws_error`;
- add duplicate-risk guards;
- add tests and operator-facing notes.

## 3. Code Work Completed

## 3.1 Prior diagnostic strategy work already in place

The diagnostic control strategy was already available before the latest review cycle:

- `a67d6c9` `feat(runtime): add marketable limit diagnostic mode`
- `39db59e` `fix(runtime): defer market buy entry until second bar`
- `6763484` `fix(runtime): use wall clock for live order timeouts`
- `840bf65` `fix(runtime): align live request ids with emitted commands`
- `9609e0a` `fix(runtime): align marketable limit state request ids`

This provided:

- `market_buy_and_close`
- `live_order_style = market | marketable_limit`
- a controlled live A/B path for `create:market` vs `create:limit`

## 3.2 TZ1 gateway observability patch

Gateway commit:

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

## 3.3 TZ2 `session_gap` recovery patch

Runtime commit:

- `a673741` `feat(session-gap): recover from transient cws transport errors`

Implemented:

- transient `cws_error` is separated from terminal business rejection;
- recoverable phase:
  - `EntryRecoveryVerificationPending`
- bounded verification before returning to `Flat`;
- duplicate-risk guards using late order and position evidence;
- explicit strategy logs for:
  - transient transport failure
  - recovery verification
  - recovery to `Flat`
  - terminal failure

Important current scope note:

- the reviewed `TZ2` implementation should currently be interpreted as entry-side transient recovery;
- exit-side transient ack policy is not yet fully aligned with that entry-side recovery policy;
- this does not invalidate the current `TZ2` review outcome, but it should remain an explicit follow-up note.

## 3.4 Runtime-side blocker fix found during live `L1`

The first current-line `marketable_limit` rerun exposed a separate runtime issue:

- strategy state could move into pending-entry before command publication became externally visible;
- this made the run invalid as a gateway/CWS verdict.

That blocker was fixed in:

- `a1ee034` `fix(runtime): restore state when live intents are dropped`

Implemented:

- pre-intent strategy state snapshot before callback execution;
- state rollback when all live intents are dropped before actual publication;
- explicit log:
  - `strategy_state_transition_reverted ... reason="intent_dropped_before_emit"`

This removed the false-positive `PendingEntry` condition and allowed a later clean rerun.

## 3.5 Local test status

Executed locally on the reviewed line:

- `cargo test -p strategy-runtime --quiet`
- `cargo test -p alor-gateway --lib --quiet`
- `cargo test -p alor-gateway --test json_contract --quiet`
- `cargo test -p alor-gateway --test redis_transport --quiet`

Result:

- `strategy-runtime`: green
- targeted `alor-gateway` tests: green
- the new runtime regression for dropped-before-emit state restoration was added and covered

## 4. Live Deployment And Topology

## 4.1 Reviewed live lines

The review package spans two closely related live lines:

- earlier reviewed live line:
  - `candidate-unify-a673741`
- fresh rerun line used for blocker-fix confirmation:
  - Docker images tagged `dev-a1ee034`
  - containing runtime fix `a1ee034`

## 4.2 Topology observed on VPS

The review package now contains evidence from three topology modes:

- `T2` coexistence:
  - `sessiongap` and `hybrid` active at the same time
- `T1` isolated:
  - only `sessiongap` active
- `T3` re-expanded:
  - `hybrid` restored after the successful isolated `T1` pass

Each stack used its own:

- gateway
- runtime
- Redis

## 4.3 Additional late discriminator modes

After the main `T1/T2/T3` sweep, the package was further narrowed by targeted late probes:

- gateway-only coexistence:
  - both gateways active
  - runtimes selectively disabled
  - manual `hybrid` passive limit create/cancel cycles completed cleanly
- `sessiongap runtime ON / hybrid runtime OFF`:
  - `hybrid` manual `place` passed
  - the first `hybrid` manual `cancel` reproduced `cws_error`
  - retry `cancel` after `hybrid` gateway recovery passed
- `hybrid runtime ON / sessiongap runtime OFF`:
  - `hybrid` manual `place/cancel` passed
- `both runtimes ON`:
  - late manual `hybrid` `place/cancel` also passed

## 5. Confirmed Live Evidence

## 5.1 Historical clean `B` reproduction remains valid

Earlier clean `marketable_limit` reproduction:

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

- this was already strong evidence against a purely `session_gap`-specific explanation.

## 5.2 Clean `M` baseline pass on the reviewed line

Clean live `M` cycle:

Entry:

- `request_id = a2f12657-d8de-5094-96df-48117e4abca1`
- command timestamp:
  - `1773859962`
  - `2026-03-18 18:52:42 UTC`
  - `2026-03-18 21:52:42 MSK`
- action:
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
- action:
  - `market`
- gateway ack:
  - `status = accepted`
  - `broker_order_id = 2023555914316948907`
- resulting position:
  - `USDRUBF qty = 0.0`

Runtime path:

- `PendingEntry -> InPosition -> PendingExit -> Done`
- final runtime state:
  - `Done`
  - `last_processed_bar_ts = 1773859920`

Gateway counters after this clean `M` cycle:

- `cws_protocol_reset_total = 0`
- `cws_reconnect_total = 0`
- `cws_limit_send_total = 0`

Interpretation:

- the live `create:market` baseline remained stable;
- no evidence was produced that the residual issue affects all live order types.

## 5.3 Runtime blocker confirmation and resolution

The first fresh `market-b` rerun after the earlier review no longer stayed in false pending-entry.

Observed runtime logs:

- repeated:
  - `market_buy_and_close live intent prepared`
- immediately followed by:
  - `strategy_state_transition_reverted ... reason="intent_dropped_before_emit"`

Observed runtime state after this phase:

- `strategy_state = Idle`
- no false persistent `PendingEntry`
- no fresh externally visible gateway activity yet

Interpretation:

- `a1ee034` worked as intended;
- the old blocker was real and is now removed;
- stale or dropped entry attempts no longer poison the diagnostic state.

## 5.4 Fresh current-line `L1` reproduction on 2026-03-23

After backlog catch-up, the diagnostic strategy reached current live bars and produced a fresh current-line `marketable_limit` incident.

Fresh command evidence:

- `request_id = 29b41853-4663-57da-a812-d1818b973b9f`
- `strategy_id = market_buy_and_close_diag_marketable_limit`
- command timestamp:
  - `1774251145`
  - `2026-03-23 07:32:25 UTC`
  - `2026-03-23 10:32:25 MSK`
- action:
  - `place`
- side:
  - `buy`
- price:
  - `83.38000000000001`

Fresh ack evidence:

- `processed_ts_utc = 1774251145`
- `status = error`
- `error_code = cws_error`
- `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
- `broker_order_id = null`
- `cws_request_guid = 29b41853-4663-57da-a812-d1818b973b9f`

Fresh runtime-state evidence after the error:

- state snapshots around:
  - `1774251301`
  - `1774251362`
  - `1774251422`
- showed:
  - `strategy_state = Blocked`
  - `reason = "entry_rejected status=Error"`
- `last_processed_bar_ts` had already caught up to current live bars:
  - `1774251180`
  - `1774251240`
  - `1774251300`

Matching live bars:

- `1774251240`
- `1774251300`
- `1774251360`

Interpretation:

- this was not stale bootstrap noise;
- this was a real live current-bar `create:limit` attempt;
- it failed in the same transport class as the earlier clean reproduction.

## 5.5 Fresh gateway counter movement on 2026-03-23

Pre-rerun baseline from gateway readiness:

- `cws_protocol_reset_total = 5`
- `cws_limit_send_total = 0`
- `cws_limit_error_total = 0`
- `cws_pending_failed_total = 0`

Post-incident gateway readiness:

- `cws_protocol_reset_total = 6`
- `cws_limit_send_total = 1`
- `cws_limit_error_total = 1`
- `cws_pending_failed_total = 1`
- `cws_connect_seq = 10`
- `cws_reconnect_seq = 9`
- `cws_connection_instance_id = d4f741ca-3e99-4981-9cb8-11ec76d4e533`
- `cws_connected_ts_utc = 1774251177`

Interpretation:

- the gateway really entered the limit-send path on the fresh rerun;
- the transport reset was real and observable through counters, not only logs;
- the reconnect sequence advanced after the incident, consistent with a real session reset.

## 5.6 Clean `L2 @ T2` passive command-path reproduction

The passive `create:limit` command-path test under coexistence topology also reproduced the same transport failure class.

Fresh command evidence:

- `request_id = 031d3de5-3d73-4ed3-bbc7-7200383a4f9f`
- `strategy_id = manual.limit.l2`
- command timestamp:
  - `1774253775`
  - `2026-03-23 08:16:15 UTC`
  - `2026-03-23 11:16:15 MSK`
- action:
  - `place`
- side:
  - `buy`
- price:
  - `82.43`

Ack result:

- `status = error`
- `error_code = cws_error`
- `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
- `broker_order_id = null`
- `cws_request_guid = 031d3de5-3d73-4ed3-bbc7-7200383a4f9f`

Gateway path showed:

- `command received`
- `cws_limit_send`
- `cws send opcode="create:limit"`
- `cws_transport_failure`
- `cws_fail_pending`
- `command ack published`

Gateway counter movement:

- `cws_protocol_reset_total: 6 -> 7`
- `cws_limit_send_total: 1 -> 2`
- `cws_limit_error_total: 1 -> 2`
- `cws_pending_failed_total: 1 -> 2`

Interpretation:

- the residual failure is reproducible even on the narrow passive command path;
- this further weakens any explanation that depends on strategy lifecycle alone.

## 5.7 Clean `T1 / L2` isolated-topology pass

After fully stopping the `hybrid` stack and leaving only `sessiongap`, the same passive limit scenario completed cleanly.

Place:

- `request_id = ca61b7c2-9963-4675-8ef5-0b253f658a7b`
- command timestamp:
  - `1774255217`
  - `2026-03-23 08:40:17 UTC`
  - `2026-03-23 11:40:17 MSK`
- `broker_order_id = 2023555922907157549`
- `status = accepted`
- order reached:
  - `working`

Cancel:

- `request_id = 8a351858-aad6-4659-be5f-833a30735f8f`
- command timestamp:
  - `1774255484`
  - `2026-03-23 08:44:44 UTC`
  - `2026-03-23 11:44:44 MSK`
- `status = accepted`
- broker message:
  - `"An order '2023555922907157549' has been deleted."`

Final order state:

- `order_id = 2023555922907157549`
- `status = canceled`
- `filled = 0.0`
- `price = 81.68`

Position effect:

- no position opened
- broker position remained flat

Gateway counters during the isolated pass:

- `cws_protocol_reset_total = 7` unchanged
- `cws_limit_send_total: 2 -> 3`
- `cws_limit_error_total = 2` unchanged
- `cws_pending_failed_total = 2` unchanged

Interpretation:

- `create:limit` can work cleanly in isolated single-stack topology;
- the current evidence no longer supports the claim that any valid live `create:limit` request always triggers the reset.

## 5.8 Clean `T3 / L2` re-expanded-topology reproduction

After restoring `hybrid` and returning to expanded coexistence topology, the same passive limit path reproduced the transport failure again.

Fresh command evidence:

- `request_id = d3a991df-4042-4b76-a01f-70e5da9faf99`
- `strategy_id = manual.limit.l2.t3`
- command timestamp:
  - `1774264615`
  - `2026-03-23 11:16:55 UTC`
  - `2026-03-23 14:16:55 MSK`
- action:
  - `place`
- side:
  - `buy`
- price:
  - `81.68`

Ack result:

- `status = error`
- `error_code = cws_error`
- `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
- `broker_order_id = null`
- `cws_request_guid = d3a991df-4042-4b76-a01f-70e5da9faf99`

Gateway path again showed:

- `command received`
- `cws_limit_send`
- `cws send opcode="create:limit"`
- `cws_transport_failure`
- `cws_fail_pending`
- `command ack published`

Gateway counter movement:

- `cws_protocol_reset_total: 7 -> 8`
- `cws_limit_send_total: 3 -> 4`
- `cws_limit_error_total: 2 -> 3`
- `cws_pending_failed_total: 2 -> 3`

Interpretation:

- after re-expanding topology, the same residual failure class returned;
- this materially strengthens the topology/coexistence sensitivity hypothesis.

## 5.9 Gateway-only coexistence discriminator passes

After the earlier `T3` reproduction, two gateway-only coexistence checks were executed to separate gateway presence from runtime activity.

First gateway-only pass:

- `request_id = 17327ab5-423d-47fb-b28b-b0b49e83d022`
- `broker_order_id = 2033126072115384072`
- order reached:
  - `working`
- follow-up cancel:
  - `request_id = 21c22563-fc52-4d86-9d77-f9f6838ba14a`
  - final order state:
    - `status = canceled`

Second gateway-only pass after the second gateway was fully ready:

- `request_id = 8d86cd43-bec3-43e6-bbdb-796db8430982`
- `broker_order_id = 2033126072115384279`
- order reached:
  - `working`
- follow-up cancel:
  - `request_id = 372e9e54-122d-4a7a-adf0-043d9454f550`
  - final order state:
    - `status = canceled`

Interpretation:

- presence of a second live gateway alone did not reproduce the failure;
- gateway-only coexistence is therefore not a sufficient trigger by itself.

## 5.10 `sessiongap runtime ON / hybrid runtime OFF` mixed-result discriminator

With `sessiongap` runtime active and `hybrid` runtime disabled, a manual `hybrid` passive limit probe produced a mixed result.

Place:

- `request_id = 31675dd4-f065-4cc0-a632-b4b6c19f4c8a`
- command timestamp:
  - `1774284933`
  - `2026-03-23 16:55:33 UTC`
  - `2026-03-23 19:55:33 MSK`
- `broker_order_id = 2033126072115384440`
- `status = accepted`
- order reached:
  - `working`

First cancel:

- `request_id = 752d48a3-2910-4758-b19a-e0b23f7c7032`
- command timestamp:
  - `1774285022`
  - `2026-03-23 16:57:02 UTC`
  - `2026-03-23 19:57:02 MSK`
- `status = error`
- `error_code = cws_error`
- `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
- in-flight opcode:
  - `delete:limit`

Matching gateway evidence:

- `cws_transport_failure`
- `cws_fail_pending`
- reconnect followed immediately after the failed cancel

Retry cancel after `hybrid` gateway recovery:

- `request_id = e14c2c8f-0347-4f4e-8b42-df9de638c6cd`
- command timestamp:
  - `1774285233`
  - `2026-03-23 17:00:33 UTC`
  - `2026-03-23 20:00:33 MSK`
- `status = accepted`
- broker message:
  - `"An order '2033126072115384440' has been deleted."`
- final order state:
  - `order_id = 2033126072115384440`
  - `status = canceled`

Interpretation:

- this late discriminator did not reproduce a blanket `create:limit` failure;
- it instead produced a transient `delete:limit` control-path reset;
- `sessiongap` runtime activity remained a plausible sensitivity factor, but not a deterministic one.

## 5.11 `hybrid runtime ON / sessiongap runtime OFF` clean pass

With `hybrid` runtime active and `sessiongap` runtime disabled, the same manual `hybrid` passive limit scenario completed cleanly.

Place:

- `request_id = eccfd7bf-7218-47be-af90-d3d0b01bb9f3`
- command timestamp:
  - `1774285364`
  - `2026-03-23 17:02:44 UTC`
  - `2026-03-23 20:02:44 MSK`
- `broker_order_id = 2033126072115385666`
- `status = accepted`
- order reached:
  - `working`

Cancel:

- `request_id = 8e5d07a0-0a18-4778-b8e3-c41fd8749a8b`
- command timestamp:
  - `1774285487`
  - `2026-03-23 17:04:47 UTC`
  - `2026-03-23 20:04:47 MSK`
- `status = accepted`
- broker message:
  - `"An order '2033126072115385666' has been deleted."`

Final order state:

- `order_id = 2033126072115385666`
- `status = canceled`

Interpretation:

- `hybrid` runtime activity alone was not sufficient to reproduce the failure.

## 5.12 `Both runtimes ON` clean discriminator pass

After re-enabling `sessiongap` runtime so that both runtimes were active at the same time, a fresh manual `hybrid` passive limit probe still completed cleanly.

Place:

- `request_id = e928c166-b807-480b-bdb1-8edd1bef7c0b`
- command timestamp:
  - `1774285660`
  - `2026-03-23 17:07:40 UTC`
  - `2026-03-23 20:07:40 MSK`
- `broker_order_id = 2033126072115386705`
- `status = accepted`
- order reached:
  - `working`

Cancel:

- `request_id = 26e5ad40-48f0-495d-814f-dac386400c18`
- command timestamp:
  - `1774285687`
  - `2026-03-23 17:08:07 UTC`
  - `2026-03-23 20:08:07 MSK`
- `status = accepted`
- broker message:
  - `"An order '2033126072115386705' has been deleted."`

Final order state:

- `order_id = 2033126072115386705`
- `status = canceled`

Readiness after the pass:

- `hybrid`:
  - `readiness = true`
  - `gateway_phase = LiveReady`
  - `cws_protocol_reset_total = 0`
  - `cws_limit_error_total = 0`
  - `cws_pending_failed_total = 0`
- `sessiongap`:
  - `readiness = true`
  - `gateway_phase = LiveReady`
  - `cws_protocol_reset_total = 0`

Interpretation:

- `both runtimes ON` was not a sufficient deterministic trigger either;
- the late live picture is now consistent with intermittent sensitivity, not a simple on/off topology rule.

## 5.13 Expanded comparative live matrix

The current live matrix is now:

- `M @ T2` = `PASS`
- `L1 @ T2` = `REPRO`
- `L2 @ T2` = `REPRO`
- `L2 @ T1` = `PASS`
- `L2 @ T3` = `REPRO`
- `L2 @ gateway-only coexistence` = `PASS`
- `hybrid manual L2 @ sessiongap runtime only` = `MIXED`
- `hybrid manual L2 @ hybrid runtime only` = `PASS`
- `hybrid manual L2 @ both runtimes ON` = `PASS`

Comparative interpretation:

- the issue is not a blanket live-trading failure;
- the issue is not `session_gap`-specific;
- the issue is not "every `create:limit` always fails";
- the issue is not explained by second-gateway presence alone;
- the issue can surface on `delete:limit`, not only `create:limit`;
- the late discriminator package does not support a simple deterministic topology trigger;
- the narrowest current working hypothesis is:
  - intermittent shared CWS limit-order control-plane instability
  - with topology/runtime/timing sensitivity.

## 6. What Is Now Confirmed

## 6.1 Strong conclusions

1. `TZ1` observability implementation is live and effective.

2. `TZ2` recovery-semantics implementation is code-complete and locally test-complete.

3. The live `create:market` baseline is stable on the reviewed line.

4. The runtime-side `marketable_limit` blocker was a separate issue and has been fixed.

5. After that fix, both a fresh `L1 @ T2` rerun and a passive `L2 @ T2` command-path test reproduced:
   - `cws_error`
   - `protocol_reset_without_close_handshake`
   - `broker_order_id = null`

6. The isolated `T1 / L2` pass proves that valid live `create:limit` requests can succeed cleanly in single-stack topology.

7. Gateway-only coexistence did not reproduce the issue by itself.

8. The re-expanded `T3 / L2` reproduction makes topology/coexistence sensitivity a materially stronger hypothesis, but the later discriminator probes show that it is not a simple deterministic on/off rule.

9. A late `sessiongap runtime ON / hybrid runtime OFF` probe produced:
   - clean `create:limit`
   - first `delete:limit` reproduction
   - clean retry delete after gateway recovery

10. Late `hybrid runtime ON / sessiongap runtime OFF` and `both runtimes ON` probes both passed end-to-end on the same `hybrid` passive path.

11. The strongest current framing is therefore:
   - not `session_gap_standalone` only
   - not a blanket live-order failure
   - not a simple deterministic coexistence switch
   - more likely intermittent shared CWS limit-order control-plane instability under specific topology/runtime/timing conditions

## 6.2 What is still not proved

1. The exact source of the sensitivity across `T1/T2/T3` and the late discriminator modes.

2. Whether the interference is driven by:
   - coexistence of multiple live stacks,
   - broker-side session competition,
   - reconnect timing,
   - runtime-driven timing interaction,
   - or another shared-session transport condition.

3. Why the first `delete:limit` under `sessiongap runtime ON / hybrid runtime OFF` reproduced cleanly while later comparable probes passed.

4. Whether the broker-side reset is pure transport/session instability or a rarer protocol-sensitive behavior in the shared limit-order control path.

## 7. Status Against The TZs

## 7.1 TZ1: shared `create:limit` / CWS transport diagnostics

### Completed

- connection-instance logging
- transport counters
- gateway readiness exposure
- topology comparison across `T1/T2/T3`
- clean `M` baseline pass
- historical clean `L1` reproduction
- fresh current-line clean `L1` reproduction after blocker fix
- clean `L2 @ T2` passive command-path reproduction
- clean `T1 / L2` isolated-topology pass
- clean `T3 / L2` re-expanded-topology reproduction
- clean gateway-only coexistence discriminator passes
- mixed `sessiongap runtime ON / hybrid runtime OFF` discriminator result
- clean `hybrid runtime ON / sessiongap runtime OFF` discriminator pass
- clean `both runtimes ON` discriminator pass

### Not completed

- specialist root-cause narrowing and remediation decision

Status:

- implementation complete
- comparative live diagnostics materially complete for review and further narrowed by late discriminator probes
- root-cause closure still pending

## 7.2 TZ2: `session_gap` recovery semantics after transient `cws_error`

### Completed

- code implementation
- tests
- transient transport vs terminal error split

### Still pending operationally

- broader live confidence package
- short operator-facing follow-up note if the team wants one
- follow-up review of exit-side transient policy so that entry and exit handling are intentionally aligned

Status:

- implementation complete
- live confidence partial but reviewable

## 8. Review Position

Recommended review position:

- `TZ1 observability implementation`: complete
- `TZ1 live diagnostics`: materially complete for review, narrowed further by late discriminator probes, not yet closed
- `TZ2 recovery-semantics implementation`: complete
- `TZ2 live confidence`: partial but sufficient for code review
- `Overall`: ready for specialist review, not ready for closure

## 8.1 Project-management review outcome

Project-management review accepted the current framing:

- `ready for specialist review, not ready for closure`

Accepted:

- `TZ1` observability implementation
- fresh `2026-03-23` `L1` reproduction as a valid current-line artifact
- the comparative `L2/T1/T3` evidence package
- `TZ2` implementation-complete status in code
- the strengthened shared `create:limit` / CWS hypothesis

Late additional discriminator probes gathered after that review further narrow the picture:

- they do not overturn the accepted `ready for specialist review, not ready for closure` position;
- they reduce confidence in any simple deterministic topology trigger;
- they strengthen the case for treating the residual issue as intermittent and timing-sensitive.

Explicitly not accepted as closed:

- full `TZ1` diagnostic program
- root cause closure
- full end-to-end live confirmation for `session_gap`

## 8.2 Specialist review outcome

Specialist review of the narrowed package converged on the following practical readout:

- the residual issue should continue to be treated as shared limit-order / CWS-path behavior, not as a `session_gap`-only bug;
- the strongest current framing remains:
  - intermittent shared CWS limit-order control-plane instability
  - with topology/runtime/timing sensitivity;
- the baseline `create:market` path still argues against any blanket live-trading failure;
- the earlier structural `create:limit` payload-shape hypothesis is now materially weakened by the late clean command-path and native-path passes;
- the `T1/T2/T3` matrix remains strong evidence that coexistence/session conditions matter, even if the exact trigger is still unproven.

The same review also makes several lower-probability interpretations explicit:

- float quantity is currently a weak explanation for the reset class;
- a purely strategy-specific `session_gap` defect is currently a weak explanation;
- another broad rerun without a narrower target is less likely to add value than a controlled next check.

## 9. Recommended Next Step After Specialist Review

The review can now focus on root-cause narrowing and next technical action, not on whether evidence exists.

Recommended next-step order:

1. focus first on the session/coexistence factor:
   - verify whether both stacks use the same principal/token;
   - verify whether multiple broker CWS sessions under that principal are expected to coexist cleanly;
   - explicitly look for replacement/collision behavior;
2. ensure transport telemetry is reviewed consistently around:
   - `cws_limit_send`
   - `cws_transport_failure`
   - with emphasis on:
     - `auth_principal_fingerprint`
     - `stack_name`
     - `gateway_instance_id`
     - `connection_age_ms`
     - `time_since_last_reconnect_ms`
     - `in_flight_pending_count`;
3. prefer a narrow repeated live matrix over another broad rerun:
   - repeated `L2 @ T1`
   - repeated `L2 @ T2`
   - if feasible, comparison under different principals
   - reconnect-timing checks:
     - immediately after reconnect
     - after `30-60` seconds
     - after a longer stable session;
4. continue to treat `TZ1` and `TZ2` as separate workstreams:
   - shared `create:limit` / CWS transport diagnostics
   - `session_gap` recovery semantics after transient `cws_error`
5. only then choose the next controlled validation or fix.

My recommended operational order after review:

1. use the expanded comparative matrix as the primary review artifact
2. narrow principal/session collision risk first
3. use telemetry plus repeated `L2` loops to test the timing-sensitive hypothesis
4. keep `TZ2` recovery follow-up separate from `TZ1` root-cause narrowing
5. only then return to live confirmation

## 10. Bottom Line

The report is strong enough to support specialist review and follow-on narrowing.

Why:

- code work for `TZ1` observability and `TZ2` recovery is in place;
- the baseline `create:market` path is clean;
- the runtime-side diagnostic blocker was found and fixed;
- after that fix, the first comparative live matrix showed:
  - coexistence repro,
  - isolated pass,
  - re-expanded coexistence repro;
- late discriminator probes then showed:
  - gateway-only coexistence passes,
  - `sessiongap runtime ON / hybrid runtime OFF` mixed result with first `delete:limit` repro,
  - `hybrid runtime ON / sessiongap runtime OFF` pass,
  - `both runtimes ON` pass.

What this means:

- the remaining problem should continue to be treated primarily as an intermittent shared CWS limit-order control-plane issue;
- topology/runtime/timing sensitivity remains real, but is no longer well described by a simple deterministic topology switch;
- `session_gap_standalone` is still relevant for recovery semantics, but is no longer the only or best explanation for the residual live incident class;
- the package is review-ready and specialist-reviewed at the framing level, while root-cause closure remains incomplete.
