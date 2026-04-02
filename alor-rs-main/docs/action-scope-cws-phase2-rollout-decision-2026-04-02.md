# Action-Scope CWS Phase 2 Rollout Decision

Date: 2026-04-02

## Decision

Continue `session_gap` gateway development from the Phase 2 candidate below:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`
- `action_scope_enable_exit = true`

This becomes the current `sessiongap` development baseline after the first successful live `entry -> fill -> flatten -> Flat` cycle.

## Supporting Evidence

Relevant notes:

- `docs/action-scope-cws-phase1-rollout-decision-2026-04-02.md`
- `docs/action-scope-cws-phase2-entry-flatten-results-2026-04-02.md`
- `docs/trading-window-closed-blocked-observation-2026-04-02.md`

Operational reading:

- Phase 1 already established the winning discriminator:
  - action-scoped alone was not enough
  - action-scoped plus forced token refresh changed the post-gap outcome
- Phase 2 then exercised the real `session_gap` flatten semantics:
  - `Place + IntentClass::Entry`
  - `Place + IntentClass::Exit`
- the first live lifecycle completed without orphan state or degraded runtime tail

## What This Means Now

For ongoing engineering work:

- treat the Phase 2 candidate as the current `sessiongap` live-development line
- continue feature work from the `action_scoped + forced token refresh + intent-aware exit gating` path
- use the dedicated Phase 2 config path for further validation windows

Runtime note:

- the later same-day runtime follow-up for `trading_window_closed` recovery has now landed
- `sessiongap` and `hybrid` no longer need restart-only recovery for the closed-window reject class
- see `docs/trading-window-closed-blocked-observation-2026-04-02.md`

## What We Are Intentionally Not Waiting For

We are not requiring an additional same-day confidence retest before continuing development.

Reason:

- Phase 1 already had multiple post-gap passes on the same force-refresh baseline
- Phase 2 has now cleared the first full live lifecycle that mattered most
- the next highest-value work is no longer another duplicate confidence loop, but the next engineering slice

## What This Still Does Not Mean

This is still not the final broad production conclusion.

It does not yet imply:

- promotion of every contour to the same config
- completion of `replace:limit` migration
- unconditional mutation of every baseline live TOML
- immediate merge to `main` before the next development slice is prepared and reviewed
- blanket production acceptance without soak on the new runtime recovery semantics

Operational note:

- historical note: earlier on `2026-04-02`, clean restart was still the workaround for this reject class.
- after the runtime follow-up patch, deferred recovery is the intended semantic fix path.

## Next Working Point

The next development slice should build on this baseline instead of reopening the Phase 2 rollout question.
