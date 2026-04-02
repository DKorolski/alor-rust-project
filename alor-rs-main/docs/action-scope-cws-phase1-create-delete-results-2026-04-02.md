# Action-Scope CWS Phase 1 Controlled Create/Delete Results

Date: 2026-04-02

## Context

Controlled live verification for the `sessiongap` action-scoped candidate:

- gateway image: `dev-3642910-actionscope1-20260402-004402`
- gateway config: `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`
- runtime image: `dev-cf913bd-exit21a-r1-20260401-165815`
- market session: open
- pre-run contour state: `LiveReady / ALLOWED`

The test intentionally used a passive limit order below market:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1.0`
- price: `79.00`

## Controlled Cycle

Place command:

- `request_id=73f02046-6a87-4de1-ada3-118a58ee8fae`
- broker `order_id=2023555957266595478`
- command ack: `accepted`
- `cws_http_code=200`
- message: order created

Observed order event after place:

- `status=working`
- `filled=0.0`
- `comment=sessiongap_73f02046-6a87-4de1-ada3-118a58ee8fae`

Cancel command:

- `request_id=85a8fa4f-1af5-4435-9269-d9dc7b6d5a37`
- broker `order_id=2023555957266595478`
- command ack: `accepted`
- `cws_http_code=200`
- message: order deleted

Observed final order event after cancel:

- `status=canceled`
- `filled=0.0`

## Action-Scope Evidence

Gateway logs show two separate short-lived control windows:

1. `create:limit`
   - open
   - authorize
   - send `create:limit`
   - close
2. `delete:limit`
   - open
   - authorize
   - send `delete:limit`
   - close

Relevant readiness counters after the run:

- `control_cws_mode=action_scoped`
- `action_scope_open_total=2`
- `action_scope_send_total=4`
- `action_scope_followup_total=0`
- `action_scope_close_total=2`
- `commands_received_total=2`
- `command_processed_total=2`
- `last_action_scope_primary_opcode=delete:limit`
- `last_action_scope_error=none`

Interpretation:

- the candidate did not reuse a long-lived idle control session;
- both control actions went through fresh short-lived CWS sessions;
- both sessions authorized, sent, and closed cleanly.

## Safety Outcome

- no fill occurred during the cycle
- final order status was `canceled`
- latest `USDRUBF` broker position remained `qty=0.0`
- runtime stayed `LiveReady / ALLOWED`
- gateway stayed `LiveReady`

## Main Conclusion

This controlled live Phase 1 check passed for `sessiongap` on the action-scoped candidate:

- `create:limit` succeeded through a short-lived control session
- `delete:limit` succeeded through a second short-lived control session
- no position was opened
- no immediate regression appeared in readiness/runtime behavior

This is positive evidence that the new action-scoped control path is viable for Phase 1 `create/delete` on the live candidate contour.
