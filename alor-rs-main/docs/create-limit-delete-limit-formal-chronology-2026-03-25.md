# Formal Chronology: `delete:limit` Incident On Instrumented Passive Probe

Date: 2026-03-25

Related documents:

- `docs/create-limit-delete-limit-instrumented-interim-2026-03-25.md`
- `docs/create-limit-and-sessiongap-review-ready-2026-03-23.md`

## 1. Scope

This document reconstructs the fresh failing run collected under:

- instrumented gateway build;
- separate broker auth principals for `sessiongap` and `hybrid`;
- narrow passive `create:limit -> delete:limit` probe;
- no broad live rerun.

Capture source:

- VPS run directory:
  - `/opt/diag-captures/20260325-131326`

Primary failing case under review:

- place request:
  - `f8d63638-5239-49bf-84bd-0f6fe985912f`
- cancel request #1:
  - `1fb09326-b31c-429d-a4c4-f6091110d6c6`
- cancel request #2:
  - `43f28f4d-cbee-4f39-bc3d-9d5af60c5419`
- order id:
  - `2023555931497048623`

## 2. Formal Chronology

| Step | ts_utc | send_seq | recv_seq | request_id | order_id | guid / requestGuid | message_class | handler / source | state_before -> state_after | Observation |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `create:limit` command received | `1774433613` | `n/a` | `n/a` | `f8d63638-5239-49bf-84bd-0f6fe985912f` | `n/a` | `n/a` | `command` | `command_consumer` | inferred: `command_received -> command_validated` | Manual passive buy probe entered gateway command path. |
| `create:limit` send | `1774433613` | `3` | `n/a` | `f8d63638-5239-49bf-84bd-0f6fe985912f` | `n/a` | `guid=f8d63638-5239-49bf-84bd-0f6fe985912f` | outbound command | `cws_client/socket_writer` | inferred: `ready_to_send -> pending_registered` | Gateway sent `opcode="create:limit"` on connection `e6e40999-29b4-4c0f-9bda-6639fb4f9bc2`, `connect_seq=1`, `reconnect_seq=0`. |
| `create:limit` direct response | `1774433613` | `3` | `3` | `f8d63638-5239-49bf-84bd-0f6fe985912f` | `2023555931497048623` | `requestGuid=f8d63638-5239-49bf-84bd-0f6fe985912f` | `response` | `cws_client/pending_resolver` | inferred: `pending_open -> pending_resolved_response` | Pending request resolved cleanly. |
| `create:limit` ack published | `1774433613` | `3` | `3` | `f8d63638-5239-49bf-84bd-0f6fe985912f` | `2023555931497048623` | `cws_request_guid=f8d63638-5239-49bf-84bd-0f6fe985912f` | `command_ack` | `command_consumer` | inferred: `response_received -> ack_published` | Ack `Accepted`; `cws_message="An order '2023555931497048623' has been created."`. |
| `request_map` updated from create ack | `1774433613` | `3` | `3` | `f8d63638-5239-49bf-84bd-0f6fe985912f` | `2023555931497048623` | `cws_request_guid=f8d63638-5239-49bf-84bd-0f6fe985912f` | local state | `command_consumer` | inferred: `request_map_before_ack_insert -> request_map_after_ack_insert` | Gateway associated `order_id -> request_id` from accepted create response. |
| `working` order event received | `1774433613` | `n/a` | `n/a` | `null` on wire, restored to `f8d63638-5239-49bf-84bd-0f6fe985912f` locally | `2023555931497048623` | `n/a` | `domain event` | `supervisor` | inferred: `event_request_id_missing -> event_request_id_resolved` | Broker order lifecycle event arrived with `event_request_id = null`; gateway restored identity from `request_map`; status `working`, `filled=0.0`. |
| `delete:limit` #1 command received | `1774433853` | `n/a` | `n/a` | `1fb09326-b31c-429d-a4c4-f6091110d6c6` | `2023555931497048623` | `n/a` | `command` | `command_consumer` | inferred: `command_received -> request_map_after_cancel_preinsert` | Before any broker ack, gateway preinserted `order_id -> cancel request_id` into `request_map`. |
| `delete:limit` #1 send | `1774433853` | `4` | `n/a` | `1fb09326-b31c-429d-a4c4-f6091110d6c6` | `2023555931497048623` | `guid=40ec7116-d1c9-4e1e-a78a-c744c61b9d26` | outbound command | `cws_client/socket_writer` | inferred: `ready_to_send -> pending_registered` | Gateway sent `opcode="delete:limit"` on the same first connection, `connect_seq=1`, `reconnect_seq=0`. |
| transport failure on `delete:limit` #1 | `1774433853` | `4` | `n/a` | `1fb09326-b31c-429d-a4c4-f6091110d6c6` | not logged explicitly in this build | `guid=40ec7116-d1c9-4e1e-a78a-c744c61b9d26` | transport failure | `cws_client/socket_reader` | inferred: `pending_open -> transport_failure_detected` | Immediate `protocol_reset_without_close_handshake`; no direct broker response payload observed. |
| pending failed after `delete:limit` #1 | `1774433853` | `4` | `n/a` | `1fb09326-b31c-429d-a4c4-f6091110d6c6` | target order was `2023555931497048623` | `guid=40ec7116-d1c9-4e1e-a78a-c744c61b9d26` | pending failure | `cws_client/pending_resolver` | inferred: `pending_open -> pending_failed_transport` | `cws_fail_pending` emitted for one in-flight `delete:limit` request. |
| `delete:limit` #1 ack published | `1774433853` | `4` | `n/a` | `1fb09326-b31c-429d-a4c4-f6091110d6c6` | `null` in ack | `cws_request_guid=40ec7116-d1c9-4e1e-a78a-c744c61b9d26` | `command_ack` | `command_consumer` | inferred: `transport_error_ready_to_publish -> ack_published` | Ack `Error`, `error_code=cws_error`, `error_msg="cws disconnected: protocol_reset_without_close_handshake"`. |
| runtime saw cancel #1 as rejected | `1774433853` | `n/a` | `n/a` | `1fb09326-b31c-429d-a4c4-f6091110d6c6` | `n/a` | `cws_request_guid=40ec7116-d1c9-4e1e-a78a-c744c61b9d26` | runtime ack | `strategy_runtime` | `n/a` | Runtime received `Error` for the first cancel. |
| `delete:limit` #2 command received | `1774434157` | `n/a` | `n/a` | `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | `2023555931497048623` | `n/a` | `command` | `command_consumer` | inferred: `command_received -> request_map_after_cancel_preinsert` | Gateway overwrote `request_map[2023555931497048623]` with the second cancel `request_id` before any broker ack. |
| `delete:limit` #2 send | `1774434157` | `1` on new socket | `n/a` | `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | `2023555931497048623` | `guid=63ed5695-c5d5-4629-8d3d-8b7b7aaf6ebf` | outbound command | `cws_client/socket_writer` | inferred: `ready_to_send -> pending_registered` | Command was sent after reconnect on new connection `b4145a24-887e-4d13-91b3-6c8cc7a9b132`, `connect_seq=2`, `reconnect_seq=1`. |
| transport failure on `delete:limit` #2 | `1774434157` | `1` | `n/a` | `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | not logged explicitly in this build | `guid=63ed5695-c5d5-4629-8d3d-8b7b7aaf6ebf` | transport failure | `cws_client/socket_reader` | inferred: `pending_open -> transport_failure_detected` | Same immediate `protocol_reset_without_close_handshake`; again no direct broker response payload observed. |
| pending failed after `delete:limit` #2 | `1774434157` | `1` | `n/a` | `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | target order was `2023555931497048623` | `guid=63ed5695-c5d5-4629-8d3d-8b7b7aaf6ebf` | pending failure | `cws_client/pending_resolver` | inferred: `pending_open -> pending_failed_transport` | `cws_fail_pending` emitted for one in-flight `delete:limit` request on the reconnected socket. |
| `delete:limit` #2 ack published | `1774434157` | `1` | `n/a` | `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | `null` in ack | `cws_request_guid=63ed5695-c5d5-4629-8d3d-8b7b7aaf6ebf` | `command_ack` | `command_consumer` | inferred: `transport_error_ready_to_publish -> ack_published` | Second cancel also reached runtime as `Error`. |
| runtime saw cancel #2 as rejected | `1774434157` | `n/a` | `n/a` | `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | `n/a` | `cws_request_guid=63ed5695-c5d5-4629-8d3d-8b7b7aaf6ebf` | runtime ack | `strategy_runtime` | `n/a` | Runtime again observed `cws_error`. |
| manual broker-terminal cancel | between `1774434157` and `1774434287` | `n/a` | `n/a` | external manual action | `2023555931497048623` | `n/a` | external action | broker terminal | `n/a` | Operator manually canceled the still-working order outside gateway. |
| final `canceled` order event | `1774434287` | `n/a` | `n/a` | `null` on wire, restored locally to `43f28f4d-cbee-4f39-bc3d-9d5af60c5419` | `2023555931497048623` | `n/a` | `domain event` | `supervisor` | inferred: `event_request_id_missing -> event_request_id_resolved` | Terminal order event arrived with `event_request_id = null`; gateway restored `request_id` from `request_map`, which at that time already pointed to the second cancel request. |

## 3. Key Observations

### 3.1 What is directly proved

The run directly proves:

- passive `create:limit` succeeded through direct response and lifecycle:
  - accepted ack
  - `working` order event
- both `delete:limit` attempts failed on the direct response / transport path:
  - immediate `protocol_reset_without_close_handshake`
  - `cws_fail_pending`
  - error ack to runtime

### 3.2 What is not directly proved

The run does **not** directly prove that either failing `delete:limit` reached broker business execution.

Why not:

- no direct success response exists for either cancel;
- no broker order event shows an on-wire `request_id` linking the terminal `canceled` state to either cancel request;
- the terminal `canceled` event arrived only after manual broker-terminal intervention;
- gateway had already overwritten `request_map[2023555931497048623]` with the second cancel request before the final event arrived.

### 3.3 Why the final `canceled` event is causally ambiguous

The final event looked like:

- `event_request_id = null` on wire;
- `state_request_id = 43f28f4d-cbee-4f39-bc3d-9d5af60c5419` locally;
- `request_map_hit = true`.

This means the final event was **attributed** to the second cancel request by local gateway state.

It does **not** mean the broker explicitly confirmed that second cancel request.

The same final event is consistent with at least two explanations:

1. the manual broker-terminal cancel produced the terminal `canceled` event, and gateway mapped it to the latest local cancel `request_id` via `request_map`;
2. a downstream broker-side cancel effect from the second gateway cancel did happen, but the direct response path failed before the client saw it.

The current run does not distinguish those two explanations conclusively.

## 4. Strongest Current Conclusion

For this exact failing run, the strongest justified conclusion is:

- `delete:limit` is definitively failing on the direct response / transport path;
- the run does not yet conclusively prove whether broker-side cancel business logic was never reached or was reached but hidden by transport failure;
- the final `canceled` event cannot be used as proof of downstream cancel success because local `request_map` recovery can attribute a null-request-id lifecycle event to the latest local cancel request.

## 5. What This Means For Engineering Review

This run is already strong enough to separate three layers conceptually:

1. direct response / transport path:
   - definitely failing on both cancel attempts;
2. downstream business effect:
   - still ambiguous in this run;
3. client-side correlation / reorder:
   - definitely relevant, because request identity on lifecycle events is reconstructed from local state and can mask causality after manual external intervention.

