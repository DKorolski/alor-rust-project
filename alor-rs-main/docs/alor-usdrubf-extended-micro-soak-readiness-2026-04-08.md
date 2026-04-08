# Alor USDRUBF Extended Micro Soak Readiness (2026-04-08)

## Scope

This memo records the current engineering assessment after:

- the `strategy-runtime` refactor line,
- full wiring of the third Alor strategy (`AlorUsdrubfHybrid`),
- pre-live hardening,
- successful 2026-04-07 live soak observations,
- and the 2026-04-08 VPS rollout/alignment pass.

It also fixes the decision boundary for which follow-up items should be done **before** extended micro soak and which should be deferred **until after** it.

## Current Status

### 1. Runtime refactor / third strategy status

The original `strategy-runtime` refactor scope is no longer only a plan or partial skeleton.

Implemented baseline:

- internal host-facing module (`strategy_host`),
- registry/factory creation path,
- strategy adapters,
- split `StrategyConfig` (`common + specific payload`),
- state envelope with compatibility path,
- capability-driven runtime gating,
- partial extraction of runtime strategy-specific special cases into hooks,
- full registration and wiring of `StrategyKind::AlorUsdrubfHybrid`.

The third strategy is therefore not only a placeholder:

- it has its own config payload,
- its own persisted state payload,
- its own adapter and registry path,
- strategy-owned runtime hooks,
- and live hardening logic already exercised on VPS.

### 2. Operational/live status

As of 2026-04-08:

- canonical VPS project for USDRUBF is `alorusdrubf`,
- duplicate USDRUBF compose project was removed,
- gateway contour is aligned to:
  - `control_cws_mode = "action_scoped"`
  - `action_scope_enable_create_limit = true`
  - `action_scope_enable_delete_limit = true`
  - `action_scope_enable_exit = true`
  - `action_scope_force_token_refresh_before_authorize = true`
- canonical stack was rolled to immutable tag:
  - `strategy-runtime:sha-4a0a266`
  - `alor-gateway:sha-4a0a266`

Post-rollout startup evidence is healthy:

- gateway resolved config matches the action-scoped baseline,
- runtime restored state correctly,
- runtime stayed blocked on startup until a fresh live-origin bar, which is the expected behavior.

## Readiness Verdict

### Extended micro soak

Readiness for **extended live micro soak** is assessed as:

- **YES**, acceptable to proceed on a frozen release,
- with the existing clean-start / micro-only operational discipline already described in soak docs.

### Scale-up / stronger confidence step

Readiness for any confidence step beyond extended micro soak is assessed as:

- **NOT YET**,
- because the current proof level is still centered on:
  - micro sizing,
  - clean-start profile,
  - no claim yet that all restart/non-flat/open-order permutations are fully proven.

## Open Engineering Remarks

The following remarks remain valid after the review:

### A. `alor_skeleton` aliasing is too permissive

Current config parsing still accepts:

- `alor_skeleton`
- `alor`

as aliases of `AlorUsdrubfHybrid`.

This was acceptable as a compatibility bridge during the refactor line, but it is no longer a good steady-state design if the next Alor strategy is expected to become a distinct strategy kind.

### B. Refactor docs are behind the implemented code

`docs/strategy-runtime-refactor/` still reflects the baseline / planned slices well, but it does not yet fully document the later code reality:

- partial runtime special-case extraction,
- full third-strategy wiring,
- follow-up hardening outcomes.

### C. Final strategy-host cleanup is not fully closed

The host is already much healthier than the original monolith, but there are still likely cleanup opportunities after soak:

- stricter naming / compatibility policy,
- clearer final adapter/descriptor boundaries,
- final documentation pass on what remains strategy-owned vs runtime-owned.

## Decision: do now vs after extended micro soak

### Do now

Only items that reduce operational ambiguity without changing live semantics:

- keep repo and VPS config aligned,
- keep canonical compose project single and explicit,
- keep rollout docs synchronized with the real VPS path,
- keep immutable image tags for rollout.

These items were already addressed on 2026-04-08.

### Do after extended micro soak

The following items should be deferred until soak completes:

1. tighten or remove `alor_skeleton` / `alor` compatibility aliases,
2. refresh `docs/strategy-runtime-refactor/` to the actual implemented end state,
3. decide whether an additional final host-cleanup slice is needed before adding the next strategy.

## Why defer those items

Because they are architecturally useful but not operationally urgent, and changing them now would increase moving parts during a soak that is meant to validate a frozen release.

The main objective of the next 5–10 sessions is not to improve elegance; it is to validate:

- startup guard behavior,
- replay guard correctness,
- entry/exit lifecycle,
- broker/runtime convergence,
- and absence of unresolved operational residue.

## Recommended next sequence

1. Run extended micro soak on the current frozen release.
2. Keep daily short notes for:
   - first fresh live bar,
   - replay guard cleared,
   - first `ALLOWED`,
   - entries/exits,
   - retries/rejects,
   - final flat,
   - manual intervention yes/no.
3. After soak:
   - revisit `alor_skeleton` alias policy,
   - update refactor docs,
   - decide whether the next strategy should land on top of the current host as-is or after one more cleanup slice.

## Practical recommendation

The best engineering tradeoff today is:

- **do not** reopen runtime architecture work before extended micro soak,
- **do** keep the release frozen and gather confidence,
- then return to the remaining refactor remarks with fresh operational evidence.
