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
- `action_scope_force_token_refresh_before_authorize = true`

Timeout/window settings are intentionally conservative:

- `action_scope_open_timeout_ms = 5000`
- `action_scope_authorize_timeout_ms = 5000`
- `action_scope_force_token_refresh_before_authorize = true`
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

## Current Status

The first controlled live Phase 1 `create -> delete` check has already passed on the dedicated `sessiongap` candidate contour.

Result note:

- `docs/action-scope-cws-phase1-create-delete-results-2026-04-02.md`

Observed outcome for the first live bounded window:

- one passive `create:limit` was accepted
- the order reached `working`
- one `delete:limit` was accepted for the same broker `order_id`
- final order status became `canceled`
- `filled=0.0`
- no broker position was opened

Gateway evidence after that run:

- `control_cws_mode=action_scoped`
- `action_scope_open_total=2`
- `action_scope_send_total=4`
- `action_scope_close_total=2`
- `commands_received_total=2`
- `command_processed_total=2`

This means the candidate has already cleared the first live acceptance slice for Phase 1 `create/delete`.

## Next Acceptance Step

The next canonical acceptance case is:

1. leave no open control CWS session between actions
2. wait about `30m`
3. run another controlled passive `create -> delete`
4. verify that the second bounded window also passes through fresh short-lived sessions

That test is the strongest next discriminator for the action-scoped baseline.

## Canonical Idle-Gap Status

That next canonical idle-gap check has now also been run on `2026-04-02`.

Result note:

- `docs/action-scope-cws-phase1-idle-gap-results-2026-04-02.md`

Outcome:

- a new fresh action-scoped session opened successfully after about `30m`
- `authorize` succeeded on the fresh session
- the first real `create:limit` send still failed with:
  - `WebSocket protocol error: Connection reset without closing handshake`

So the current rollout picture is now:

- immediate bounded Phase 1 `create -> delete` live check: `PASS`
- canonical `~30m idle gap -> new fresh create` live check: `FAIL`

This means the candidate remains diagnostically valuable, but it is not yet a fully proven replacement baseline.

## Fresh-Token Discriminator

An additional same-day discriminator has now also been run:

- `docs/action-scope-cws-phase1-fresh-token-restart-results-2026-04-02.md`

Observed outcome:

- after a gateway-only restart, the process obtained a fresh access token at startup
- a new controlled passive `create -> delete` bounded window then passed again

This strengthened the working hypothesis that token freshness or process-lived auth state matters for the failure class.

To support the next live check without requiring a full gateway restart, the candidate config now enables:

- `action_scope_force_token_refresh_before_authorize = true`

Interpretation:

- each action-scoped bounded window now invalidates the in-process cached token before `authorize`
- the next `authorize` therefore obtains a fresh access token again
- this is an explicit diagnostic discriminator, not yet a final architecture conclusion

## Force-Refresh Idle-Gap Status

That next no-restart discriminator has now also been run on `2026-04-02`.

Result note:

- `docs/action-scope-cws-phase1-force-refresh-idle-gap-results-2026-04-02.md`

Observed outcome:

- after about `33m` with no control action, a new passive `create:limit` succeeded
- a follow-up `delete:limit` for the same broker `order_id` also succeeded
- gateway logs showed:
  - cached token invalidated before `authorize`
  - fresh token refresh for `action_scope_cws_authorize`
  - `action_scope_authorize_ok` with `access_token_source="refreshed"`

So the current Phase 1 diagnostic picture is now:

- immediate bounded action-scoped live check: `PASS`
- canonical idle-gap on cached in-process token state: `FAIL`
- fresh-token restart bounded window: `PASS`
- canonical idle-gap on force-refresh action-scoped candidate: `PASS`

An additional confidence retest has now also been run on the same candidate:

- `docs/action-scope-cws-phase1-force-refresh-idle-gap-retest-results-2026-04-02.md`

Observed outcome:

- another `~30m` post-gap passive `create -> delete` cycle also passed
- the same gateway process again logged:
  - cached token invalidation
  - fresh token refresh for `action_scope_cws_authorize`
  - `action_scope_authorize_ok` with `access_token_source="refreshed"`

Current reading:

- the earlier post-gap failure was not explained by short-lived CWS session lifetime alone
- forcing token freshness before action-scoped `authorize` changed the post-gap outcome
- two consecutive post-gap passes on the force-refresh candidate now exist
- token freshness or process-lived auth state is now the strongest working discriminator

## Out Of Scope For This Candidate

- unattended live exit validation
- `replace:limit`
- `exit/flatten` migration
- `hybrid` rollout

Those remain later phases after separate validation.
