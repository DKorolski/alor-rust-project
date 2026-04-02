# Action-Scope CWS Phase 1 Rollout Decision

Date: 2026-04-02

## Decision

Adopt the `sessiongap` gateway variant below as the primary Phase 1 development baseline:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`

This decision is based on same-day live evidence, not only local tests.

## Supporting Evidence

Relevant result notes:

- `docs/action-scope-cws-phase1-create-delete-results-2026-04-02.md`
- `docs/action-scope-cws-phase1-idle-gap-results-2026-04-02.md`
- `docs/action-scope-cws-phase1-fresh-token-restart-results-2026-04-02.md`
- `docs/action-scope-cws-phase1-force-refresh-idle-gap-results-2026-04-02.md`
- `docs/action-scope-cws-phase1-force-refresh-idle-gap-retest-results-2026-04-02.md`
- `docs/action-scope-cws-phase1-force-refresh-long-gap-retest-results-2026-04-02.md`

Operational reading:

- action-scoped alone was directionally good but not sufficient
- cached-token post-gap behavior still reproduced the reset
- force-refresh before action-scoped `authorize` changed the post-gap outcome
- multiple post-gap retests then passed on the same gateway process

## What This Decision Means

For ongoing gateway development:

- treat `action_scoped + forced token refresh` as the preferred `sessiongap` control-path baseline
- continue development from this line instead of the earlier cached-token action-scoped variant
- keep the rollout isolated to the dedicated `sessiongap` action-scoped config until later phase acceptance completes

## What This Decision Does Not Yet Mean

This is not yet the final broad production conclusion.

It does not yet imply:

- migration of every existing contour to `action_scoped`
- unconditional mutation of every baseline live TOML
- completion of `replace:limit` migration
- completion of `exit/flatten` validation
- immediate merge to `main` without Phase 2 evidence

## Phase 2 Start Point

Phase 2 now focuses on real strategy lifecycle semantics rather than passive bounded probes:

- runtime-native or production-path entry
- fill confirmation
- runtime-native or approved controlled flatten
- flat-state recovery without orphan order or orphan position

The first engineering step for Phase 2 is to make action-scoped routing explicitly intent-aware so that:

- `Entry` path remains opt-in and explicit
- `Exit` path is gated independently
- config and real behavior stay aligned during live validation

That first engineering step is now complete in code, and the next operator-facing artifacts are:

- `configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`
- `docs/action-scope-cws-phase2-entry-flatten-runbook-2026-04-02.md`
