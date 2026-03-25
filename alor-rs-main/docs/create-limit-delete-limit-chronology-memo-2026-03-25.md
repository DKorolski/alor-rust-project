# Engineering Memo: `delete:limit` Chronology Follow-Up

Date: 2026-03-25

Related artifacts:

- `docs/create-limit-delete-limit-instrumented-interim-2026-03-25.md`
- `docs/create-limit-delete-limit-formal-chronology-2026-03-25.md`
- VPS capture:
  - `/opt/diag-captures/20260325-131326`

## 1. What Was Done

This follow-up completed the `TZ 1.1` narrowing step on two tracks.

### 1.1 Existing failing run was reconstructed formally

The already collected fresh failing run was rebuilt as a single chronology using both:

- `request_id`
- `order_id`

Reviewed sources:

- `sessiongap.cmd.orders.post.txt`
- `sessiongap.cmd.acks.post.txt`
- `sessiongap.broker.orders.post.txt`
- `sessiongap.broker.positions.post.txt`
- `sessiongap.gateway.post.log`
- `sessiongap.runtime.post.log`

### 1.2 Gateway tracing was tightened locally for the next narrow run

The local patch now adds:

- `order_id` inside pending-request / transport-failure diagnostics;
- handler-level logs around:
  - `socket_writer`
  - `socket_reader`
  - `pending_resolver`
  - `command_consumer`
  - `supervisor`;
- explicit `state_before` / `state_after` fields on the main chronology-relevant transitions;
- `_diag_cws_request_order_id` propagation on the direct CWS response path;
- `trace-order` support in `scripts/limit_diag.sh`.

Changed files:

- `alor-gateway/src/cws_client.rs`
- `alor-gateway/src/services/command_consumer.rs`
- `alor-gateway/src/supervisor.rs`
- `scripts/limit_diag.sh`

## 2. What Was Confirmed

### 2.1 Passive create passed cleanly

For the fresh passive probe:

- `request_id = f8d63638-5239-49bf-84bd-0f6fe985912f`
- `order_id = 2023555931497048623`

Observed:

- direct CWS response accepted;
- command ack published as `Accepted`;
- order lifecycle reached `working`;
- no fill occurred.

### 2.2 Both cancel attempts failed on the direct response path

For:

- `request_id = 1fb09326-b31c-429d-a4c4-f6091110d6c6`
- `request_id = 43f28f4d-cbee-4f39-bc3d-9d5af60c5419`

Observed both times:

- `delete:limit` send occurred;
- immediate `protocol_reset_without_close_handshake` followed;
- `cws_fail_pending` fired;
- runtime received `Error` / `cws_error`;
- no direct broker success response for cancel was observed.

### 2.3 Final `canceled` event is not clean causal proof of downstream cancel success

This is the key result of the chronology review.

The final terminal event arrived only after manual broker-terminal cancellation, and it arrived with:

- `event_request_id = null` on wire;
- `state_request_id = 43f28f4d-cbee-4f39-bc3d-9d5af60c5419` locally;
- `request_map_hit = true`.

At the same time, `command_consumer` had already preinserted / overwritten:

- `request_map[2023555931497048623] = 43f28f4d-cbee-4f39-bc3d-9d5af60c5419`

before any broker confirmation of the second cancel existed.

Therefore the final `canceled` event was locally attributed to the second cancel request, but that attribution does not prove broker-side acceptance of that gateway cancel.

### 2.4 Post-failure auth behavior points to cached-token recovery

After later reconnect attempts on the same diagnostic line, both gateways showed repeated:

- `httpCode = 401`
- `message = "Invalid JWT token!"`
- `cws_authorized = false`

Operationally, restarting only `alor-gateway` with the same unchanged `refresh_token` restored CWS authorization on both stacks.

This materially strengthens the following narrow interpretation:

- the failing post-incident auth loop is plausibly driven by a stale cached `access_token`;
- it is not best explained as a permanently invalid `refresh_token`;
- a targeted cache invalidation on explicit CWS `401` is the appropriate minimal fix.

## 3. Strongest Current Conclusion

The current failing run supports the following narrow conclusion.

What is proved:

- `delete:limit` is definitely failing on the direct response / transport path.

What is not yet proved:

- whether broker-side cancel business logic was never reached;
- or whether broker-side cancel did happen and only the direct response path failed.

The current run cannot close that distinction because the terminal `canceled` event is causally ambiguous after:

- local `request_map` overwrite by cancel intent;
- manual broker-terminal cancellation.

## 4. Practical Readout For Review

For project review:

- the task narrowed the problem materially;
- the fresh failing path remains `delete:limit`, not passive `create:limit`;
- separate tokens did not eliminate the incident class;
- client-side correlation behavior is now proven relevant to interpretation of the terminal event.

For engineering review:

- the freshest evidence now cleanly isolates direct-response / transport failure on cancel;
- downstream business-effect attribution remains open;
- the new local patch is specifically aimed at making the next narrow incident chronology non-ambiguous on:
  - `order_id`
  - handler ownership
  - transition state
  - pending/request-map causality.
- an additional narrow fix is now justified:
  - invalidate cached `access_token` on explicit CWS authorize `401`
  - so the next reconnect can refresh a new token without process restart.
