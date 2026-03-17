# Live Incident Note: `sessiongap` and `hybrid`

Date: 2026-03-17  
Deployment tag: `candidate-unify-d87f8cf`

## 1. Executive summary

Two separate live incidents were observed after rollout of the residual CWS observability fix:

1. `sessiongap`
- a native runtime-generated `create:limit` request hit a transport reset immediately after send;
- the new observability path worked as intended:
  - `cws_guid` was preserved,
  - `disconnect_kind` was recorded,
  - affected pending requests were logged,
  - the published `command_ack` carried the preserved `cws_request_guid`;
- the strategy still transitioned `PendingEntry -> Blocked`, so the remaining issue is recovery semantics after transient `cws_error`, not failure-path observability.

2. `hybrid`
- runtime state persisted with:
  - `last_position_qty = -1.0`,
  - `safe_mode_close_only = true`,
  - `safe_mode_reason = "recovered_position_owner_unknown"`,
  - `current_owner = null`,
  - `current_side = null`;
- this looks like a separate recovered-position ownership/state-reconciliation issue and is not explained by the `sessiongap` CWS transport incident.

## 2. Validation status before incident review

The following verification was completed on the fixed gateway/runtime line:

| Check | Scope | Result |
|---|---|---|
| Level A | broker contract probe for corrected `create:limit` payload | PASS |
| Level B1 | production command path create/cancel through gateway | PASS |
| B2 command-path | manual production-path entry -> fill -> flatten -> `Flat` | PASS |
| B2 runtime-native | natural strategy-generated full lifecycle | NOT RUN / not confirmed |

### B2 command-path evidence

Entry:
- `request_id = 86f4e9cd-3285-4f94-bbef-3103761ca7fe`
- `ts_utc = 1773671631` -> `2026-03-16 14:33:51 UTC` / `2026-03-16 17:33:51 MSK`
- `broker_order_id = 2023555901432457888`
- `cws_request_guid = 86f4e9cd-3285-4f94-bbef-3103761ca7fe`
- order lifecycle: `working -> filled`
- resulting position: `USDRUBF qty = 1.0 avg_price = 81.07`

Flatten:
- `request_id = 5ba9825a-3a2d-42b6-aec2-bb49125ee8d2`
- `ts_utc = 1773671734` -> `2026-03-16 14:35:34 UTC` / `2026-03-16 17:35:34 MSK`
- `broker_order_id = 2023555901432459671`
- `cws_request_guid = 86b3cbc0-fbfc-4e3a-b2b9-fceeeef0cbe3`
- order lifecycle: `working -> filled`
- resulting position: `USDRUBF qty = 0.0`
- runtime state returned to `phase = "Flat"`

This confirms the fixed command/gateway/CWS path, but it does not by itself prove that the strategy recovers correctly after a native transient transport failure.

## 3. Incident A: `sessiongap` native `create:limit` transport reset

## 3.1 Context

After the rollout of `candidate-unify-d87f8cf`, `sessiongap` later emitted a natural runtime-generated entry intent. The request was prepared correctly and sent through the corrected `create:limit` path, but the CWS transport reset immediately after send.

This incident is important because it exercises the exact residual-failure class the fix was designed to make observable.

## 3.2 Incident identity

- Strategy: `session_gap_standalone`
- Portfolio: `7502MIW`
- Symbol: `USDRUBF`
- `request_id = bd4881c6-bcc8-525e-9da8-06509d0bfdf8`
- Action: `place`
- Side: `buy`
- Qty: `1.0`
- Price: `81.28`

Key timestamps:
- intent timestamp `1773673140` -> `2026-03-16 14:59:00 UTC` / `2026-03-16 17:59:00 MSK`
- failure/ack timestamp `1773673202` -> `2026-03-16 15:00:02 UTC` / `2026-03-16 18:00:02 MSK`

## 3.3 Runtime evidence

Observed in `strategy-runtime` logs:

- `live phase transition` `Flat -> PendingEntry`
- `intent_emitted action="place" request_id=bd4881c6-bcc8-525e-9da8-06509d0bfdf8`
- `command rejected`
  - `error_code = Some("cws_error")`
  - `error_msg = Some("cws disconnected: protocol_reset_without_close_handshake")`
  - `cws_request_guid = Some("bd4881c6-bcc8-525e-9da8-06509d0bfdf8")`
- `live phase transition` `PendingEntry -> Blocked`

This already shows that downstream runtime now receives materially better failure information than before:
- preserved `cws_request_guid`,
- specific disconnect class in `error_msg`,
- explicit transition point into `Blocked`.

## 3.4 Gateway evidence

Observed in `alor-gateway` logs:

1. Command accepted from runtime
- `command received request_id=bd4881c6-bcc8-525e-9da8-06509d0bfdf8`

2. Corrected `create:limit` request prepared
- `action="cws_limit_send"`
- `opcode="create:limit"`
- `cws_guid="bd4881c6-bcc8-525e-9da8-06509d0bfdf8"`
- `symbol="USDRUBF"`
- `instrument_group=RFUD`
- `side="buy"`
- `qty=1`
- `price=81.28`
- `time_in_force="OneDay"`
- `allow_margin=true`
- `check_duplicates=true`

3. Transport failure logged with classification
- `action="cws_transport_failure"`
- `disconnect_kind="protocol_reset_without_close_handshake"`
- `opcode_in_flight=Some("create:limit")`
- `request_id=Some("bd4881c6-bcc8-525e-9da8-06509d0bfdf8")`
- `cws_guid=Some("bd4881c6-bcc8-525e-9da8-06509d0bfdf8")`
- `pending_count=1`
- `raw_error=WebSocket protocol error: Connection reset without closing handshake`

4. Pending failures logged with affected request list
- `action="cws_fail_pending"`
- `disconnect_kind="protocol_reset_without_close_handshake"`
- `pending_count=1`
- `affected=[{"cws_guid":"bd4881c6-bcc8-525e-9da8-06509d0bfdf8","opcode":"create:limit","request_id":"bd4881c6-bcc8-525e-9da8-06509d0bfdf8","symbol":"USDRUBF"}]`

5. Error ack published with preserved correlation
- `status="error"`
- `error_code="cws_error"`
- `error_msg="cws disconnected: protocol_reset_without_close_handshake"`
- `cws_request_guid=Some("bd4881c6-bcc8-525e-9da8-06509d0bfdf8")`

## 3.5 Redis evidence

`cmd.orders.7502MIW`:
- stream id `1773673202399-0`
- payload carried the same request:
  - `request_id = bd4881c6-bcc8-525e-9da8-06509d0bfdf8`
  - `strategy_id = session_gap_standalone`
  - `symbol = USDRUBF`
  - `place.price = 81.28`
  - `place.qty = 1.0`
  - `place.side = buy`

`cmd.acks.7502MIW`:
- stream id `1773673202407-0`
- payload:
  - `status = error`
  - `error_code = cws_error`
  - `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
  - `broker_order_id = null`
  - `cws_request_guid = "bd4881c6-bcc8-525e-9da8-06509d0bfdf8"`

## 3.6 Current assessment

What is now confirmed:
- the original `create:limit` structural bug is no longer the primary hypothesis;
- the new transport observability/error-path implementation works on a real native incident;
- `cws_guid` is no longer lost in the failure path;
- the failure is reconstructable from normal `info/warn` logs without global debug;
- downstream runtime receives the precise disconnect class.

What remains open:
- `session_gap_standalone` still reacts to this transient transport failure by moving into `Blocked`;
- readiness/live-guard may recover, while strategy state remains blocked until operator cleanup;
- this is now a strategy recovery-semantics issue, not a gateway observability issue.

## 4. Incident B: `hybrid` recovered position owner unknown

## 4.1 Context

The `hybrid` paper stack stayed infrastructure-healthy after rollout, but runtime state snapshots showed a persistent recovered-position tail. This is separate from the `sessiongap` transport-reset path.

## 4.2 State snapshot

Observed in `runtime.state.hybrid_intraday.paper.imoexf.7502SN6`:

- snapshot timestamp `1773769684` -> `2026-03-17 17:48:04 UTC` / `2026-03-17 20:48:04 MSK`
- `active_cycle_id = "69b9420404"`
- `last_position_qty = -1.0`
- `current_owner = null`
- `current_side = null`
- `pending_entry_* = null`
- `pending_exit_* = null`
- `safe_mode_close_only = true`
- `safe_mode_reason = "recovered_position_owner_unknown"`
- `entry_ready = true`

The same shape persisted across several consecutive state snapshots, so this is not a transient one-frame condition.

## 4.3 Related runtime log trail

Observed in `strategy-runtime` logs for `hybrid_intraday_runtime` on 2026-03-17:

- `09:01:00 UTC`: generated `submit_entry owner=IntradayBreakout side=Long style=Market`
- `09:51:09 UTC`: generated `submit_exit owner=IntradayBreakout reason=BreakoutStop1Long`

Despite this, later state reconstruction shows:
- non-zero recovered position quantity,
- no active owner attribution,
- safe mode locked to `close_only`.

## 4.4 Current assessment

This looks like a separate hybrid runtime issue in recovered-position attribution or state reconciliation:

- runtime believes a position tail exists: `last_position_qty = -1.0`;
- the position has no recoverable owner mapping;
- runtime protects itself with `safe_mode_close_only = true`;
- this is distinct from the `sessiongap` CWS transport-reset incident.

Operationally, `hybrid` should be treated as:
- infrastructure healthy,
- strategy state degraded,
- pending separate investigation.

## 5. Current status by stack

### `sessiongap`
- Gateway health: OK
- Runtime health: OK
- Transport observability fix: VERIFIED on native incident
- Strategy state: NOT CLEAN
- Open issue: `PendingEntry -> Blocked` after transient `cws_error`

### `hybrid`
- Gateway health: OK
- Runtime process health: OK
- Paper mode block: expected
- Strategy state: NOT CLEAN
- Open issue: `recovered_position_owner_unknown`, `safe_mode_close_only = true`

## 6. Recommended follow-up

1. `sessiongap`
- investigate strategy-side recovery policy after transient transport error;
- decide whether `cws_error` of this class should:
  - remain `Blocked`,
  - retry,
  - or downgrade into an operator-visible non-terminal state;
- keep capturing the same evidence set on the next native incident:
  - runtime logs,
  - gateway logs,
  - `cmd.orders`,
  - `cmd.acks`,
  - `broker.orders`,
  - `broker.positions`,
  - `runtime.state`.

2. `hybrid`
- investigate how runtime reconstructs owner/cycle after recovered positions;
- inspect why `last_position_qty = -1.0` persists while ownership fields are null;
- determine whether this requires:
  - runtime-state cleanup,
  - repair logic change,
  - or owner-mapping fix in recovery path.

## 7. Conclusion

The residual CWS observability task should be considered successful:
- the live native `sessiongap` incident is now diagnosable,
- the failure path preserves correlation,
- disconnect class is explicit,
- affected pending requests are visible,
- published `command_ack` is materially more useful.

At the same time, the live investigation exposed two remaining operational issues:
- `sessiongap` strategy recovery semantics after transient transport failure,
- `hybrid` recovered-position ownership reconciliation.

These should be tracked as separate follow-up incidents from the completed gateway observability fix.
