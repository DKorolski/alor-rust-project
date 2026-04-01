# Action-Scope CWS Phase 1 Controlled Rollout

Date: 2026-04-02

## Goal

Define a safe candidate launch path for Phase 1 `action_scoped` control CWS work without mutating the current `sessiongap` live baseline.

This rollout note intentionally limits scope to:

- `create:limit`
- `delete:limit`
- bounded `create -> delete -> close`

It intentionally does not enable:

- `replace:limit`
- `exit/flatten`
- marketable-limit entry

## Candidate Config

New dedicated candidate gateway profile:

- `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`

The current baseline live profile remains unchanged:

- `configs/gateway.sessiongap.live.7502MIW.toml`

## Safety Rules

1. Keep the code/config default on `legacy_long_lived` for all existing contours.
2. Enable `action_scoped` only by explicitly pointing `sessiongap` to the dedicated candidate config.
3. Keep `hybrid` on legacy mode.
4. Keep `replace` and `exit` disabled during Phase 1.
5. Treat this as a controlled diagnostic/live candidate, not as a general default.

## Candidate Settings

The dedicated candidate profile uses:

- `control_cws_mode = "action_scoped"`
- `action_scope_enable_create_limit = true`
- `action_scope_enable_delete_limit = true`
- `action_scope_enable_replace_limit = false`
- `action_scope_enable_exit = false`

Timeout/window settings are intentionally conservative:

- `action_scope_open_timeout_ms = 5000`
- `action_scope_authorize_timeout_ms = 5000`
- `action_scope_followup_window_ms = 5000`
- `action_scope_max_session_lifetime_ms = 15000`
- `action_scope_close_timeout_ms = 2000`

## Local Launch

```bash
ALOR_STACK_NAME=sessiongap \
RUST_LOG=info,alor_gateway=debug \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.action-scoped.toml \
  --redis-url redis://127.0.0.1/
```

## VPS Rollout Shape

Do not overwrite the existing baseline file.

Instead, switch only the gateway config path for the candidate window:

```bash
export GATEWAY_CONFIG=/configs/gateway.sessiongap.live.7502MIW.action-scoped.toml
```

Or, if the stack is driven via `.env`, update only:

```bash
GATEWAY_CONFIG=/configs/gateway.sessiongap.live.7502MIW.action-scoped.toml
```

The runtime config should remain the normal live `sessiongap` runtime profile unless a separate runtime candidate is explicitly needed.

## Expected Readiness Signals

On the gateway side, the candidate should expose:

- `control_cws_mode = "action_scoped"`
- populated `last_action_scope_*` timestamps after control actions
- growth in `action_scope_open_total`
- growth in `action_scope_send_total`
- growth in `action_scope_close_total`

It should not rely on the long-lived control CWS path for Phase 1 `create/delete` actions.

## Minimal Acceptance For Controlled Candidate

1. Gateway starts cleanly with the candidate config.
2. `readiness` exposes `control_cws_mode = "action_scoped"`.
3. A controlled `create:limit` uses a short-lived action-scoped session.
4. A controlled `delete:limit` uses action-scoped semantics as configured.
5. No hidden idle-open control session remains between bounded actions.

## Out Of Scope For This Candidate

- unattended live exit validation
- `replace:limit`
- `exit/flatten` migration
- `hybrid` rollout

Those remain later phases after separate validation.
