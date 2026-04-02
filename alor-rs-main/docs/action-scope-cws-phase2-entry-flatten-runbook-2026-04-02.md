# Action-Scope CWS Phase 2 Entry/Flatten Runbook

Date: 2026-04-02

## Goal

Validate the real `session_gap` lifecycle on top of the selected Phase 1 baseline:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`

Phase 2 expands validation from passive bounded probes into the actual strategy lifecycle:

- entry submit
- fill confirmation
- flatten submit
- return to `Flat` without orphan order or orphan position

## Candidate Config

Dedicated Phase 2 gateway profile:

- `configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`

Key settings:

- `action_scope_enable_create_limit = true`
- `action_scope_enable_delete_limit = true`
- `action_scope_enable_exit = true`
- `action_scope_enable_replace_limit = false`
- `action_scope_force_token_refresh_before_authorize = true`

## Important Execution-Path Note

For `session_gap`, the live flatten path that matters here is not a market order.

Runtime-native `session_gap` live exit currently emits:

- `CommandAction::Place`
- `intent_class = "exit"`

This means the Phase 2 candidate is specifically validating:

- marketable-limit entry via action-scoped `create:limit`
- marketable-limit flatten via action-scoped `create:limit`

If you use a manual market exit, you will not be exercising the new action-scoped exit routing.

## Safety Rules

1. Keep the current Phase 1 candidate unchanged and available as rollback.
2. Do not overwrite the baseline live TOML.
3. Do not enable `replace:limit` during this phase.
4. Start with one controlled lifecycle at the smallest safe size.
5. Keep operator flatten readiness throughout the first live windows.

## VPS Rollout Shape

Use a dedicated config path for the candidate window:

```bash
GATEWAY_CONFIG=/configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml
```

Do not mutate:

- `configs/gateway.sessiongap.live.7502MIW.toml`
- `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`

until Phase 2 acceptance is complete.

## Preferred Validation Modes

Two validation modes remain available:

1. runtime-native `session_gap` lifecycle
2. manual command-path lifecycle

Preferred order:

1. runtime-native or approved one-shot forced `session_gap` lifecycle
2. manual command-path lifecycle only if needed for extra isolation

## Runtime-Native Mode

Use the existing `session_gap` B2 flow, but the gateway must point to the Phase 2 config above.

Reference runbook:

- `docs/session-gap-b2-runbook.md`

Expected control-path interpretation under the Phase 2 candidate:

- entry `Intent::Place + IntentClass::Entry` -> action-scoped
- flatten `Intent::Place + IntentClass::Exit` -> action-scoped

## Manual Command-Path Mode

If manual validation is needed, use marketable limit commands for both entry and flatten.

### Entry

Use a marketable `place` command with:

- `intent_class = "entry"`

Example template:

```bash
REQ_ID=$(cat /proc/sys/kernel/random/uuid)
TS=$(date +%s)

PLACE_PAYLOAD=$(cat <<JSON
{"schema_version":1,"ts_utc":$TS,"source":"manual-action-scope-phase2","msg_type":"command","payload":{"request_id":"$REQ_ID","created_ts_utc":$TS,"strategy_id":"manual.phase2.entry","portfolio":"7502MIW","exchange":"MOEX","symbol":"USDRUBF","action":{"place":{"price":<MARKETABLE_BUY_PRICE>,"qty":1.0,"side":"buy","comment":"phase2_entry_$REQ_ID"}},"intent_class":"entry","ttl_ms":600000}}
JSON
)
```

### Flatten

Use a marketable `place` command with:

- `intent_class = "exit"`

Example template:

```bash
EXIT_REQ_ID=$(cat /proc/sys/kernel/random/uuid)
TS2=$(date +%s)

EXIT_PAYLOAD=$(cat <<JSON
{"schema_version":1,"ts_utc":$TS2,"source":"manual-action-scope-phase2","msg_type":"command","payload":{"request_id":"$EXIT_REQ_ID","created_ts_utc":$TS2,"strategy_id":"manual.phase2.exit","portfolio":"7502MIW","exchange":"MOEX","symbol":"USDRUBF","action":{"place":{"price":<MARKETABLE_SELL_PRICE>,"qty":1.0,"side":"sell","comment":"phase2_exit_$EXIT_REQ_ID"}},"intent_class":"exit","ttl_ms":600000}}
JSON
)
```

This is the key Phase 2 discriminator:

- flatten must stay on the `Place + Exit` path
- not on `Market`

## Required Evidence

Capture:

- gateway `/readiness` before and after the cycle
- runtime `/readiness` before and after the cycle
- gateway logs
- runtime logs
- `cmd.orders.7502MIW`
- `cmd.acks.7502MIW`
- `broker.orders.7502MIW`
- `broker.positions.7502MIW`
- `runtime.state.session_gap_standalone.live.7502MIW`

Critical correlation fields:

- `request_id`
- `broker_order_id`
- `cws_request_guid`
- `primary_opcode`
- `access_token_source`

## Expected Gateway Log Shape

For both entry and flatten action windows, logs should show:

- `action_scope_session_open_start`
- `invalidated cached alor access token`
- `refreshed alor access token consumer="action_scope_cws_authorize"`
- `action_scope_authorize_ok ... access_token_source="refreshed"`
- `action_scope_send_result ... opcode="create:limit" http_code=Some(200)`
- `action_scope_close_result ... outcome="ok"`

## Acceptance

Phase 2 passes only if:

1. entry is accepted
2. entry fills
3. flatten is accepted
4. position returns to zero
5. runtime returns to `Flat`
6. no orphan working order remains
7. no orphan broker position remains
8. no residual `Blocked` or degraded close-only tail remains

## Non-Goals For This Step

- `replace:limit`
- `hybrid` rollout
- unconditional promotion to every live contour
- merge to `main` before Phase 2 evidence is captured
