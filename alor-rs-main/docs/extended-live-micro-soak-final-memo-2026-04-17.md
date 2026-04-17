# Extended Live Micro Soak Final Memo

Date: 2026-04-17

## Purpose

This memo records the final engineering reading after the extended live micro soak across:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

The purpose is to fix the conclusion boundary before implementation work starts:

1. what the soak proved,
2. what it did not prove,
3. where the main failures actually sit,
4. and why post-soak work should focus on intent-path unification rather than strategy-by-strategy folklore.

## Evidence Base

Primary evidence:

- [vps-live-observations-2026-04-07.md](./vps-live-observations-2026-04-07.md)
- [vps-live-observations-2026-04-08.md](./vps-live-observations-2026-04-08.md)
- [vps-live-observations-2026-04-09.md](./vps-live-observations-2026-04-09.md)
- [vps-live-observations-2026-04-10.md](./vps-live-observations-2026-04-10.md)
- [vps-live-observations-2026-04-11.md](./vps-live-observations-2026-04-11.md)
- [vps-live-observations-2026-04-13.md](./vps-live-observations-2026-04-13.md)
- [vps-live-observations-2026-04-14.md](./vps-live-observations-2026-04-14.md)
- [vps-live-observations-2026-04-15.md](./vps-live-observations-2026-04-15.md)
- [vps-live-observations-2026-04-16.md](./vps-live-observations-2026-04-16.md)

Supporting synthesis:

- [extended-live-micro-soak-engineering-analysis-2026-04-17.md](./extended-live-micro-soak-engineering-analysis-2026-04-17.md)
- [live-incident-note-2026-04-14-trading-hybrid-mr-tp-sl-cws.md](./live-incident-note-2026-04-14-trading-hybrid-mr-tp-sl-cws.md)
- [extended-micro-soak-spec-2026-04-07.md](./extended-micro-soak-spec-2026-04-07.md)

Additional live verification on VPS `155.212.170.21` on 2026-04-17:

- active compose projects and `.env` routing,
- live gateway config files in `/opt/trading-*`,
- gateway logs for `2026-04-14..2026-04-16`,
- control-path asymmetry confirmation for `action_scope_cws` vs persistent `cws_client`.

## Scope Boundary

This memo is not a code-change proposal by itself.

It is a conclusion memo:

- what the soak demonstrated,
- what should be treated as a design problem,
- and what should be treated as strategy-specific business behavior rather than a defect.

## Executive Verdict

### 1. The soak produced enough evidence

The soak is sufficient for a final engineering reading.

There is enough evidence to compare the systems not only by end-of-day outcome, but also by:

- control-path behavior,
- retry behavior,
- protective lifecycle behavior,
- broker-accept behavior,
- and state convergence semantics.

### 2. Final trading outcomes were often correct, but operational quality differed materially

Across the soak window:

- `sessiongap` most often behaved like a clean one-shot system.
- `trading-hybrid` often converged correctly, but with protective-order noise, repair complexity, and occasional event-ordering anomalies.
- `trading-alor-usdrubf` often converged correctly only after repeated market-path failures and deferred retries.

### 3. The main engineering problem is not “strategy A is worse than strategy B”

The main problem is:

- identical intent classes do not go through uniform lifecycle semantics,
- and in some cases not even through the same CWS contour class.

This is the central design conclusion of the soak.

## Live Path Verification on VPS

### Actual live gateway configs

On the current VPS:

- `trading-sessiongap` uses `gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`
- `trading-hybrid` uses `gateway.hybrid.live.7502SN6.action-scoped.toml`
- `trading-alor-usdrubf` uses `gateway.alor_usdrubf.live.7502T0U.toml`

All three live configs currently declare:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`

This means the production question is no longer “which config file in repo says legacy”.

The real question is:

- which intent classes actually still run through old-style persistent CWS behavior in practice.

### Important live discovery

The soak evidence plus direct VPS log checks show:

- `sessiongap` core `create:limit` entry / exit path really uses fresh action-scoped open -> refresh -> authorize -> send -> close.
- `hybrid` entry path can use that healthy action-scoped contour, but protective TP / SL traffic still went through persistent `cws_client` behavior.
- `alor-usdrubf` has action-scoped config enabled, but the problematic `create:market` flows in the observed failures were still surfacing through persistent `cws_client` traffic with pending buildup and transport failure cleanup.

That asymmetry is now evidence-backed.

## System-Level Verdicts

### `trading-sessiongap`

#### What the soak proved

- core entry / exit lifecycle is the cleanest of the three systems;
- operator-facing logs and phase transitions are the easiest to reconstruct;
- action-scoped + forced token refresh worked cleanly on the observed one-shot limit path;
- the stack repeatedly converged to flat without the same class of transport drama seen elsewhere.

#### Remaining concerns

- some older local repo/deploy artifacts still refer to legacy files, even though live VPS uses the action-scoped phase2 config;
- `orphan_trade` / event-ordering issues appeared occasionally, so it is not mathematically perfect.

#### Final reading

`sessiongap` is the best baseline for:

- simple entry
- simple scheduled exit
- operator-readable one-shot lifecycle semantics

It is not the whole answer, but it is the cleanest reference.

### `trading-hybrid`

#### What the soak proved

- the strategy can converge correctly to flat even under noisy protective lifecycle conditions;
- `MeanRevTimeCutoff` and later cleanup paths can still flatten the book when bracket installation fails;
- action-scoped entry works;
- the protective contract is the richest exercised contract in the system.

#### What the soak exposed

- TP / SL `create:limit` / `create:stopLimit` are still the most problematic engineering path;
- long-lived CWS behavior, timeouts, transport failure cleanup, repair, and `orphan_trade` make this stack the noisiest;
- some protective intents did not reach broker acceptance at all in the observed failure windows;
- the stack often still achieved the right final result only because later exits or cleanup paths compensated.

#### Final reading

`trading-hybrid` is the best evidence source for:

- what a full protective-order lifecycle must cover

but it is not the best stylistic baseline for common intent handling.

### `trading-alor-usdrubf`

#### What the soak proved

- deferred retry logic is materially stronger here than in the simpler stacks;
- broker-truth reconciliation and explicit flat confirmation are better surfaced;
- the strategy does not silently forget exits;
- even after repeated transport failures it usually converged.

#### What the soak exposed

- `create:market` is the dominant failure class;
- repeated transport resets and timeout-style failures can accumulate before a later retry succeeds;
- some entire bursts of emitted intents never reached broker acceptance on their original attempt path;
- the strategy survived by persistence, not by a clean transport path.

#### Final reading

`trading-alor-usdrubf` is the best baseline for:

- deferred retry policy
- explicit broker-truth convergence
- “do not give up until flat”

But it is not yet an example of a clean stable transport path.

## Quantitative Signal From Live Gateway Logs

For the directly sampled live VPS logs on `2026-04-14..2026-04-16`:

### `trading-sessiongap`

- `create:limit` accepted: **6**
- `create:limit` ack errors: **0**
- `create:limit` transport failures in sampled window: **0**

### `trading-hybrid`

- limit accepted: **16**
- limit ack errors: **6**
- `create:limit` transport failures: **4**
- `create:stopLimit` transport failures: **1**

Important nuance:

- the healthiest `hybrid` entry path used `action_scope_cws`,
- but the problematic protective requests used persistent `cws_client`.

### `trading-alor-usdrubf`

- sampled `create:market` transport failures in live gateway logs: **19**

The accepted side is visible in runtime/state journals and in focused log windows, but the key engineering takeaway is simpler:

- the failing market path is recurring enough that counting transport failures is already diagnostically sufficient.

## Core Engineering Findings

### A. The systems differ too much on identical intent classes

This is the main conclusion.

The soak supports the claim that some differences are not business-driven:

- same kind of intent,
- same broker,
- same operational contour family,
- but materially different lifecycle semantics.

### B. Fresh action-scoped one-shot flow looks better than stale persistent CWS behavior

The evidence is strongest here:

- `sessiongap` clean one-shot limit flow,
- `hybrid` entry healthy on action-scoped but TP/SL unhealthy on long-lived,
- `alor-usdrubf` market failures appearing on persistent `cws_client` behavior.

### C. “Eventually flat” is not the same as “good operational semantics”

Several soak days ended correctly while still exposing:

- non-trivial transport fragility,
- repeated retries,
- missing broker acks for original attempts,
- and protective / cleanup ambiguity.

So final flat alone is not enough to call the path healthy.

## What Is Already Good Enough to Reuse

### Reuse from `sessiongap`

- one-shot entry state transitions
- one-shot exit state transitions
- simple operator vocabulary
- minimal ambiguity between emit / accept / execution / phase transition

### Reuse from `alor-usdrufb`

- deferred retry after transport failure
- explicit broker-position convergence
- stronger refusal to silently lose an exit obligation

### Reuse from `trading-hybrid`

- only the coverage of protective-order lifecycle requirements
- not the current implementation style as the general baseline

## What Must Change After Soak

### 1. Intent lifecycle semantics must become more uniform

At minimum, all strategies should surface the same distinctions:

- dropped before emit
- emitted but timed out
- emitted and transport-rejected
- accepted by broker
- execution confirmed
- broker-position converged

### 2. Protective traffic must stop being a hybrid-only special world

Protective install / repair / cleanup semantics need to be modeled as a first-class contract rather than a mostly strategy-private glue layer.

### 3. Market retry must stop being mostly strategy-owned behavior

The best parts of `alor-usdrubf` deferred retry should become a reusable policy surface rather than a unique strategy personality.

### 4. Repo / deploy control-path visibility must be cleaned up

The live `sessiongap` path is action-scoped phase2, but not every local artifact says so.

That drift is not the root of the runtime failures, but it is still an engineering risk for future rollout work.

## Final Conclusion

The extended live micro soak should be considered successful as an evidence-gathering phase.

It proved that:

- all three stacks can trade live and often converge correctly;
- infra-level Redis pressure was mitigated successfully after the VPS/RAM and Redis-limit fix;
- the dominant remaining issue is no longer infra, but intent-path asymmetry and transport-path inconsistency between strategies and intent classes.

The next engineering step should therefore be:

- not another vague soak extension by default,
- but a focused implementation plan to unify the broken intent classes using the best observed behavior from the three stacks.
