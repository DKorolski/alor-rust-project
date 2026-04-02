# Action-Scope CWS Phase 1 Canonical Idle-Gap Results

Date: 2026-04-02

## Goal

Run the canonical Phase 1 acceptance case on the live `sessiongap` action-scoped candidate:

1. complete one successful bounded `create -> delete`,
2. leave no open control CWS session,
3. wait about `30m`,
4. attempt a new fresh `create:limit`.

The purpose of this check was to test whether a second fresh short-lived control window still succeeds after a real `~30m` idle gap with no open control session between the two windows.

## Prior Successful Window

The earlier same-day controlled cycle had already passed:

- result note: `docs/action-scope-cws-phase1-create-delete-results-2026-04-02.md`
- previous successful final close: about `2026-04-02 09:30:54 MSK`

## Idle-Gap Attempt

The second bounded window started at about `2026-04-02 10:00:57 MSK`, which is about `30m` after the previous successful close.

Test parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1.0`
- price: `78.00`

Command:

- `request_id=ba88ca59-633e-426b-8ac4-af751a4a1da1`

## Observed Sequence

Gateway logs show:

1. action-scoped session open started
2. action-scoped session open succeeded
3. `authorize` send started
4. `authorize` succeeded
5. `create:limit` send started
6. send failed immediately with:
   - `WebSocket protocol error: Connection reset without closing handshake`
7. action-scoped close attempted
8. close returned:
   - `Trying to work with closed connection`

Published `command_ack`:

- `status=error`
- `error_code=cws_error`
- `error_msg=WebSocket protocol error: Connection reset without closing handshake`
- `broker_order_id=null`
- `cws_http_code=null`

No `broker.orders` event was created for a new order and no `USDRUBF` position was opened.

## Readiness / Counters

Pre-run action-scope counters:

- `action_scope_open_total=2`
- `action_scope_send_total=4`
- `action_scope_close_total=2`
- `commands_received_total=2`

Post-run counters:

- `action_scope_open_total=3`
- `action_scope_send_total=6`
- `action_scope_close_total=3`
- `action_scope_send_failed_total=1`
- `commands_received_total=3`
- `command_processed_total=2`
- `last_action_scope_primary_opcode=create:limit`
- `last_action_scope_error=Trying to work with closed connection`

Interpretation:

- a new fresh action-scoped session really was opened;
- `authorize` on that fresh session succeeded;
- the first real `create:limit` send after the `~30m` gap still failed.

## Safety Outcome

- no new broker order was accepted
- no `broker_order_id` was assigned
- no `USDRUBF` fill occurred
- latest broker `USDRUBF` position remained `qty=0.0`
- gateway remained `LiveReady`
- runtime remained `LiveReady / ALLOWED`

## Main Conclusion

This canonical idle-gap acceptance case failed on the live `sessiongap` action-scoped candidate.

What is now established:

- the Phase 1 immediate bounded `create -> delete` flow can pass in live on action-scoped CWS;
- but a new fresh action-scoped `create:limit` attempted after a real `~30m` idle gap still reproduced the same failure class:
  - `Connection reset without closing handshake`

Practical reading:

- action-scoped `create/delete` is stronger than the previous long-lived baseline for an immediate bounded control window;
- but the current gateway-side action-scoped candidate is not yet a proven production baseline for the canonical `~30m gap -> new fresh send` case.
