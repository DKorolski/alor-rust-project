# Intent Path Unification Fix Plan

Date: 2026-04-17

## Purpose

This document defines the implementation plan and technical specification for the post-soak fix line.

It is driven by the conclusion memo:

- [extended-live-micro-soak-final-memo-2026-04-17.md](./extended-live-micro-soak-final-memo-2026-04-17.md)

and the engineering synthesis:

- [extended-live-micro-soak-engineering-analysis-2026-04-17.md](./extended-live-micro-soak-engineering-analysis-2026-04-17.md)

The central objective is:

- make the lifecycle and control path of identical intent classes more uniform,
- while preserving valid strategy-specific business logic differences.

## Problem Statement

The soak showed a repeatable asymmetry:

- `sessiongap` simple `create:limit` intents behaved like clean action-scoped one-shot operations;
- `hybrid` protective TP / SL intents behaved like a separate, older persistent-CWS world;
- `alor-usdrubf` market retry behavior was resilient, but repeated `create:market` failures still accumulated through persistent-CWS style behavior before a later retry succeeded.

This creates three risks:

1. operator ambiguity,
2. uneven reliability between intent classes,
3. strategy-owned retry / cleanup logic where host-level semantics should exist.

## Goal

Refactor and harden intent handling so that:

- identical intent classes have comparable lifecycle semantics,
- protective traffic uses a resilient contour rather than a stale long-lived path,
- market retry is bounded and explicit,
- and broker-truth convergence is operator-readable and consistent.

## Systems Affected

### 1. `trading-sessiongap`

Primary role in this fix line:

- reference / baseline system

Expected change impact:

- low

Intended work:

- keep behavior stable,
- align repo/deploy docs and config naming with the actual live action-scoped phase2 path,
- use it as a regression baseline for patched validation soak.

### 2. `trading-hybrid`

Primary role in this fix line:

- highest-priority protective hot path

Expected change impact:

- high

Intended work:

- remove the asymmetric protective path,
- make TP / SL / delete-stop-limit use the same resilient contour family as healthy entry traffic,
- make protective repair / cleanup semantics explicit and bounded.

### 3. `trading-alor-usdrubf`

Primary role in this fix line:

- highest-priority market retry / transport convergence hot path

Expected change impact:

- high

Intended work:

- reduce repeated `create:market` accumulation,
- make retry / defer / inflight clearing more explicit,
- align actual market send behavior with the intended resilient control semantics.

### 4. Shared `alor-gateway`

Primary role in this fix line:

- gateway control-path routing and CWS behavior

Expected change impact:

- high

Intended work:

- routing / contour choice by intent class,
- pending cleanup,
- stale connection handling,
- protective and market send path consistency.

### 5. Shared `strategy-runtime`

Primary role in this fix line:

- minimal patched-soak observability first, broader lifecycle semantics later

Expected change impact:

- medium to high

Intended work:

- Patch A:
  - add only the minimum emit / timeout / reset / reject distinctions needed for patched validation soak;
- Patch B:
  - normalize intent lifecycle states,
  - normalize reject / timeout taxonomy,
  - standardize broker-truth convergence logging and state transitions.

## Out of Scope

The following are explicitly out of scope for this fix line:

- changing broker/gateway contour architecture as a whole,
- introducing a new transport framework,
- changing strategy signal logic,
- changing position sizing,
- rewriting all strategies to a single identical business workflow,
- broad runtime host refactor unrelated to intent-path issues.

## Implementation Phasing

This fix line is intentionally split into two phases.

### Patch A. Functional hotfix line

This is the line to implement now.

Scope:

- WP1 baseline alignment,
- WP2 hybrid protective contour unification,
- WP3 alor-usdrubf market path hardening,
- minimal observability additions only where they are required for patched validation soak.

Non-goals for Patch A:

- full host-level lifecycle normalization,
- full reject taxonomy normalization,
- broad broker-truth cleanup unification.

Rationale:

- the soak localized the main problem to hot-path routing asymmetry;
- mixing routing fixes with large semantic normalization would widen side-effect risk;
- a narrower hotfix line keeps causal attribution clear in the next live validation cycle.

### Patch B. Semantic normalization follow-up

This line should start only after Patch A passes patched micro validation soak.

Scope:

- WP4 common intent lifecycle contract,
- WP5 reject taxonomy normalization,
- WP6 broker-truth and cleanup convergence semantics.

Rationale:

- these changes are valuable, but they are broader shared-runtime work;
- they should be validated after the main functional hot paths are stabilized;
- they should not be a prerequisite for the first patched live confidence read.

## Work Packages

## WP1. Freeze and align the live control-path baseline

### Objective

Make sure repo and deploy artifacts say the same thing as the actual VPS contour.

### Tasks

- document the actual live gateway configs for all three stacks;
- remove or clearly mark stale local config references that still imply legacy sessiongap routing;
- ensure runbooks and deploy notes reference the current live files and project names.

### Acceptance criteria

- no ambiguity remains about which gateway config each live stack uses;
- `sessiongap` action-scoped phase2 usage is reflected in repo docs / deploy artifacts;
- operators do not need to infer live semantics from ad hoc log reading.

## WP2. Hybrid protective contour unification

### Objective

Remove the asymmetry where healthy entry uses action-scoped behavior but protective TP / SL still depend on persistent long-lived CWS behavior.

### Tasks

- trace how `create:limit` and `create:stopLimit` for protective intents are currently routed;
- make protective install path use the same resilient contour family as the healthy entry path;
- ensure retries do not pile up silently on a stale socket;
- ensure delete / cleanup path remains compatible with the already validated stop-cleanup baseline.

### Acceptance criteria

- protective TP / SL install no longer depends on a stale long-lived CWS socket;
- protective requests do not accumulate multi-request pending tails on a dead or half-dead connection;
- hybrid can still perform:
  - entry
  - protective install
  - protective repair
  - cancel / delete-stop-limit cleanup
- `orphan_trade` frequency on protective failure days is reduced or at least better explained by normalized state transitions.

## WP3. Alor-usdrubf market path hardening

### Objective

Fix the repeated `create:market` failure pattern so that market entry / exit does not accumulate many failed sends before the eventual retry succeeds.

### Tasks

- trace why the problematic market traffic still surfaces through persistent `cws_client` behavior in the observed failure cases;
- reduce or eliminate burst accumulation of pending market sends on one stale connection;
- make deferred retry bounded and explicit;
- make inflight clearing and deferred-state transitions uniform and operator-readable.

### Acceptance criteria

- market-path failures do not build long pending queues before reconnect cleanup;
- a failed market attempt is clearly classified as one of:
  - dropped by guard,
  - emitted then timeout,
  - transport reset,
  - broker reject;
- later retry is bounded and state-consistent;
- end-of-day flatten remains guaranteed without hidden inflight residue.

## WP4. Common intent lifecycle contract

### Objective

Standardize the lifecycle vocabulary and semantics across strategies.

This work package belongs to Patch B, not to the first hotfix release.

### Tasks

Introduce or normalize host/runtime-visible concepts for:

- `signal_generated`
- `intent_prepared`
- `intent_dropped_by_guard`
- `intent_emitted`
- `command_acknowledged`
- `command_rejected`
- `execution_confirmed`
- `broker_position_converged`
- `cleanup_completed`

### Acceptance criteria

- all three strategies can be read through the same high-level lifecycle vocabulary;
- operator can distinguish:
  - not emitted,
  - emitted but timeout,
  - emitted and rejected,
  - accepted,
  - executed;
- state transitions and logs are not strategy-private jargon for these core phases.

## WP5. Reject taxonomy normalization

### Objective

Make failures comparable across strategies and across days.

This work package belongs to Patch B, not to the first hotfix release.

### Tasks

Normalize coarse classes:

- `guard_block`
- `transport_reset`
- `transport_timeout`
- `broker_reject`
- `trading_window_closed`
- `late_trade_or_orphan_trade`

### Acceptance criteria

- the same failure mode maps to the same coarse class in all stacks;
- docs and logs stop mixing low-level transport text with high-level policy meaning;
- post-trade analysis can count comparable failure classes across systems.

## WP6. Broker-truth and cleanup convergence semantics

### Objective

Make late trade / cleanup / flat confirmation behavior more uniform.

This work package belongs to Patch B, not to the first hotfix release.

### Tasks

- standardize how runtime reacts to broker truth after delayed or noisy control paths;
- standardize inflight clearing semantics;
- standardize post-flat cleanup semantics where relevant.

### Acceptance criteria

- late fills and `orphan_trade`-like events do not leave unclear ownership of truth;
- flat confirmation means the same thing across stacks;
- pending / cleanup / protective residue is explicit and inspectable.

## Recommended Order of Implementation

### Patch A

#### PR1. Control-path baseline alignment

- repo / deploy config alignment
- doc cleanup
- no semantic change

#### PR2. Hybrid protective routing fix

- highest-confidence narrow hotfix
- move TP / SL path off the unhealthy contour behavior

#### PR3. Alor-usdrubf market-path fix

- reduce `create:market` burst accumulation
- tighten deferred retry semantics

#### PR4. Minimal observability slice

- runtime / adapter level
- add only the minimum coarse distinctions needed for patched soak:
  - dropped before emit
  - emitted then timeout
  - transport reset
  - broker reject
- no broad lifecycle refactor in this PR

### Patch A validation gate

Run a focused patched micro validation soak before starting Patch B.

### Patch B

#### PR5. Shared lifecycle + reject taxonomy

- runtime / adapter level
- broader semantic normalization and logging/state consistency

#### PR6. Broker-truth convergence cleanup

- late trades
- inflight clearing
- flat semantics

## Validation Plan

Each PR should be validated at three levels:

### A. Unit / integration

- routing choice by intent class
- retry / defer semantics
- lifecycle state transitions
- reject taxonomy mapping where introduced

### B. Replay / synthetic scenarios

- transport reset before broker ack
- timeout without emit suppression
- guard drop before emit
- late trade after cleanup

### C. Live micro validation

Patch A must be followed by a focused patched micro validation soak before any scale-up decision.

Recommended duration:

- 3 to 5 sessions

Focus:

- hybrid protective install
- hybrid protective cleanup
- alor-usdrubf market entry retry
- alor-usdrubf market exit / EOD flatten
- sessiongap still staying clean on one-shot path

Scale-up policy after Patch A:

- `sessiongap`: candidate for first selective small rollout if no regression is observed;
- `hybrid`: conditional small only after protective install and cleanup behavior stabilizes;
- `alor-usdrubf`: keep on micro / limited exposure until a second positive confidence read, even if the first patched soak is materially better.

## Success Criteria

The full fix line should be considered successful if:

1. `sessiongap` remains as clean as before.
2. `hybrid` protective TP / SL no longer depends on the failure-prone old contour behavior.
3. `alor-usdrubf` no longer builds long failed `create:market` pending tails before eventual success.
4. Operators can tell, uniformly, whether an intent:
   - was dropped,
   - timed out,
   - was transport-rejected,
   - was accepted,
   - was executed.
5. End-of-day and broker-flat semantics remain reliable.

Patch A should be considered successful if:

1. `sessiongap` shows no regression on the clean one-shot path.
2. `hybrid` protective TP / SL install and cleanup use the intended resilient contour family.
3. `alor-usdrubf` no longer accumulates long stale pending market bursts before eventual success.
4. Operators can distinguish the minimum coarse classes needed for patched soak analysis.

Patch B should be considered successful if:

1. lifecycle vocabulary is genuinely comparable across strategies;
2. reject taxonomy is normalized across stacks;
3. broker-truth and cleanup convergence semantics are operator-readable and shared.

## Final Recommendation

Implementation should start without waiting for outside problem-definition input.

External review may still be useful before code rollout, but not to understand the problem.

The soak already provided enough evidence to proceed with an internal engineering fix plan.

Recommended execution shape:

- implement Patch A now,
- run focused patched micro validation soak,
- only then proceed to Patch B semantic normalization.

This keeps the direction of the original plan, but reduces near-term side-effect risk and makes the first patched live read easier to interpret.
