# Hybrid CWS Stop Cleanup Observation (2026-04-07)

## Incident Window

- Stack: `trading-hybrid`
- Symbol: `IMOEXF`
- Runtime config: `runtime.hybrid.live.7502SN6.action-scoped.toml`
- Gateway config: `gateway.hybrid.live.7502SN6.action-scoped.toml`

## What Happened

At session open, entry and protective flow partially succeeded:

1. Entry limit create was accepted (`broker_order_id=2033126119359646405`).
2. Position/entry execution was confirmed by runtime.
3. TP create (`request_id=e193...`) failed due transport reset (`cws_error`).
4. SL create (`request_id=a74b...`) succeeded after reconnect (`stop_order_id=118088645`, status `working`).

Later, after position close:

1. Runtime emitted `delete_stop_limit` (`request_id=727d...`) for `order_id=118088645`.
2. Gateway sent `delete:stopLimit` and then hit CWS transport reset (`protocol_reset_without_close_handshake`).
3. Ack for delete returned `status=Error`, `error_code=cws_error`.
4. Result: stop cleanup did not complete in that attempt; stop could remain `working` until next reconcile/retry/operator action.

## Root Cause Class

- Primary class: transport-layer CWS disconnect during cleanup opcode in action-scoped session.
- Not a signal-generation issue; this is post-fill protection cleanup reliability under transient CWS resets.

## Runtime Observability Gap (Before Patch)

- Runtime already logged `command rejected`, but without explicit "flat + active stop remains" context.
- This made triage slower when an entry/exit flow looked mostly successful.

## Runtime Observability Patch (Applied)

Added explicit warnings in `HybridIntradayRuntimeStrategy`:

- `cleanup_ack_error_with_active_stop_while_flat`
  - emitted when an ack is terminal error/reject/expired while strategy is flat and there are still working stop orders.
- `stop_order_active_while_flat`
  - emitted on stop-order updates when strategy is flat and at least one stop remains active.

## Symmetric Gateway Patch (Applied)

`DeleteStopLimit` cleanup is now routed through the same action-scoped contour as entry/limit cleanup when:

- `control_cws_mode = "action_scoped"`
- `action_scope_enable_delete_limit = true`

Implementation notes:

1. `command_consumer` now selects `ActionScoped` path for `CommandAction::DeleteStopLimit`.
2. `execute_command` now sends `delete:stopLimit` via `ActionScopeCwsManager` on action-scoped path.
3. `ActionScopeCwsManager` now supports `delete_stop_limit(...)` with per-action open/authorize/send/close and fresh token policy.

Effect:

- stop cleanup no longer relies on long-lived CWS path when action-scoped delete is enabled,
- behavior is symmetric with the existing action-scoped create/delete limit logic.

## Follow-up Hardening Candidate

1. Track pending `delete_stop_limit` cleanup request(s) explicitly in strategy state.
2. On terminal cleanup ack error, enter deterministic retry/degraded path:
   - bounded retry with backoff while flat, or
   - explicit close-only/operator-required mode if retries exhausted.
3. Keep clear audit trail tying stop cleanup request id, stop order id, and recovery decision.

## Practical Operator Note

Until functional cleanup-retry patch is in place, when this warning appears:

- verify active stop list in gateway/broker snapshot,
- cancel residual stop manually if required by run protocol,
- keep event bundle (runtime + gateway logs around request ids) for follow-up fix validation.

## Post-Rollout Validation (Success)

After deploying `fbf744f` to `trading-hybrid`:

- gateway/runtime were restarted on `vps-fbf744f` images,
- `control_cws_mode="action_scoped"` and `action_scope_enable_delete_limit=true` were confirmed in resolved config,
- strategy resumed normal live flow and protective lifecycle,
- cleanup completed and the previously hanging stop order was removed.

Practical verdict:

- the incident path is considered mitigated for current rollout,
- `delete:stopLimit` now follows the same action-scoped lifecycle expectations as other limit cleanup actions.
