# Action-Scope CWS Phase 2 Entry/Flatten Results

Date: 2026-04-02

## Goal

Validate the first real `session_gap` lifecycle on top of the selected Phase 1 baseline:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`
- `action_scope_enable_exit = true`

This check intentionally exercised the production-shape Phase 2 discriminator:

- entry via `CommandAction::Place` with `intent_class = "entry"`
- flatten via `CommandAction::Place` with `intent_class = "exit"`

## Candidate Rollout

VPS gateway-only rollout:

- `GATEWAY_IMAGE_TAG=dev-71c09ac-actionscope2-20260402-131941`
- `GATEWAY_CONFIG=/configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`

Runtime stayed unchanged:

- `RUNTIME_IMAGE_TAG=dev-cf913bd-exit21a-r1-20260401-165815`

Before the cycle:

- gateway `LiveReady`
- runtime `LiveReady / ALLOWED`
- runtime state `phase="Flat"`
- no open `USDRUBF` position remained

## Entry Cycle

Submitted at `2026-04-02 13:38:39 MSK`.

Command:

- `request_id = cda8d1f3-fcc9-4593-bcdb-bd252bbd0a3a`
- `strategy_id = manual.phase2.entry`
- `symbol = USDRUBF`
- `side = buy`
- `price = 81.00`
- `intent_class = "entry"`

Observed outcome:

- `command_ack status = accepted`
- `broker_order_id = 2023555957266833215`
- `cws_http_code = 200`
- `cws_request_guid = 2e35e47f-d7e0-4488-b566-93559230ce0c`
- order lifecycle:
  - `working`
  - `filled`
- broker position opened:
  - `symbol = USDRUBF`
  - `qty = 1.0`
  - `avg_price = 80.28`

## Flatten Cycle

Submitted at `2026-04-02 13:39:14 MSK`.

Command:

- `request_id = 7e734dc7-6a64-41c6-af0c-e960b132c465`
- `strategy_id = manual.phase2.exit`
- `symbol = USDRUBF`
- `side = sell`
- `price = 79.50`
- `intent_class = "exit"`

Observed outcome:

- `command_ack status = accepted`
- `broker_order_id = 2023555957266833692`
- `cws_http_code = 200`
- `cws_request_guid = 034be5e5-3a54-4468-a937-53b4ab311179`
- order lifecycle:
  - `working`
  - `filled`
- broker position returned to zero:
  - `symbol = USDRUBF`
  - `qty = 0.0`
  - `avg_price = 0.0`

## Runtime Outcome

Observed after flatten:

- runtime readiness remained `true`
- runtime phase remained `LiveReady`
- `live_guard = "ALLOWED"`
- `open_risk_position_unflattened = false`
- `runtime.state.session_gap_standalone.live.7502MIW` returned to:
  - `phase = "Flat"`

## Gateway Evidence

The gateway remained healthy after the cycle:

- `gateway_phase = "LiveReady"`
- `control_cws_mode = "action_scoped"`
- `action_scope_open_total = 2`
- `action_scope_send_total = 4`
- `action_scope_close_total = 2`
- `commands_received_total = 2`
- `command_processed_total = 2`
- `cws_create_limit_send_total = 2`
- `cws_create_limit_success_total = 2`

Important log shape was present for both entry and flatten:

- `action_scope_session_open_start`
- `invalidated cached alor access token`
- `refreshed alor access token consumer="action_scope_cws_authorize"`
- `action_scope_authorize_ok ... access_token_source="refreshed"`
- `action_scope_send_result ... opcode="create:limit" http_code=Some(200)`
- `action_scope_close_result ... outcome="ok"`

This confirms that the flatten path stayed on the intended Phase 2 route:

- `CommandAction::Place`
- `intent_class = "exit"`
- action-scoped `create:limit`

It did not silently fall back to the old long-lived control path.

## Reading

The first same-day Phase 2 live lifecycle passed end-to-end:

- entry was accepted and filled
- flatten was accepted and filled
- the position returned to zero
- runtime returned to `Flat`
- no orphan order remained
- no orphan position remained
- no degraded tail remained

Operationally, this is the first live confirmation that:

- the selected Phase 1 baseline also supports the real `session_gap` exit path
- `action_scope_enable_exit = true` now maps to actual `Place + IntentClass::Exit` behavior
- `action_scoped + forced token refresh` is strong enough to continue Phase 2 development from this line
