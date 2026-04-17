# Extended Live Micro Soak Engineering Analysis

Date: 2026-04-17

## Purpose

This note starts the engineering synthesis after the extended live micro soak.

The key question is not only whether the three systems eventually reached the correct trading outcome, but also whether they behaved consistently for the same classes of intents:

- entry
- protective follow-up
- timed / scheduled exit
- EOD flatten
- retry after transport failure
- broker-truth reconciliation

The working engineering concern is valid:

- the systems did not behave uniformly for the same intent classes,
- and some of that difference is strategy-specific by design,
- but some of it is clearly runtime / adapter / policy divergence rather than business-logic necessity.

## Evidence Base

Primary evidence comes from:

- [vps-live-observations-2026-04-07.md](./vps-live-observations-2026-04-07.md)
- [vps-live-observations-2026-04-08.md](./vps-live-observations-2026-04-08.md)
- [vps-live-observations-2026-04-09.md](./vps-live-observations-2026-04-09.md)
- [vps-live-observations-2026-04-10.md](./vps-live-observations-2026-04-10.md)
- [vps-live-observations-2026-04-11.md](./vps-live-observations-2026-04-11.md)
- [vps-live-observations-2026-04-13.md](./vps-live-observations-2026-04-13.md)
- [vps-live-observations-2026-04-14.md](./vps-live-observations-2026-04-14.md)
- [vps-live-observations-2026-04-15.md](./vps-live-observations-2026-04-15.md)
- [vps-live-observations-2026-04-16.md](./vps-live-observations-2026-04-16.md)

Supporting soak scope and decision boundary:

- [extended-micro-soak-spec-2026-04-07.md](./extended-micro-soak-spec-2026-04-07.md)
- [alor-usdrubf-extended-micro-soak-readiness-2026-04-08.md](./alor-usdrubf-extended-micro-soak-readiness-2026-04-08.md)

## High-Level Reading

Across the soak window, the systems converged differently:

- `sessiongap` was the cleanest and most deterministic implementation of one-shot entry / one-shot exit.
- `trading-hybrid` was the richest in control-path complexity, but also the noisiest around protective legs, repair, cleanup, and event ordering.
- `trading-alor-usdrubf` showed the strongest deferred-retry survival behavior for market intents, but also the clearest repeated transport fragility on `create:market`.

This means there is no single system that should be copied wholesale.

The better engineering conclusion is:

- normalize by intent class,
- not by strategy as a whole.

## Cross-System Comparison by Intent Class

### 1. Simple entry intent

Observed patterns:

- `sessiongap` repeatedly showed the cleanest path:
  - signal
  - `intent_emitted`
  - `command accepted`
  - `execution confirmed`
  - immediate phase transition
- `trading-hybrid` entry itself was often fine, but its real problem started right after the fill when bracket legs had to be installed.
- `trading-alor-usdrubf` repeatedly hit transport failures on first `market` entry attempt, then deferred and retried successfully on a later bar.

Engineering reading:

- for plain entry lifecycle, `sessiongap` is the best reference implementation for observability and state clarity;
- `alor-usdrubf` adds an important resilience pattern that `sessiongap` does not need: deferred retry after transport failure.

Conclusion:

- future common entry semantics should combine:
  - `sessiongap` clarity,
  - plus `alor-usdrubf` deferred retry discipline.

### 2. Simple exit / scheduled flatten intent

Observed patterns:

- `sessiongap` again behaved most predictably:
  - `InPosition -> PendingExit -> Flat`
  - one emitted exit
  - one accepted order
  - one fill
- `trading-hybrid` exits succeeded, but often against a background of protective cleanup noise, `orphan_trade`, or stop-order side effects.
- `trading-alor-usdrubf` exits often required repeated retries after `protocol_reset_without_close_handshake`.

Engineering reading:

- `sessiongap` is the cleanest baseline for deterministic scheduled exits;
- `alor-usdrubf` is operationally stronger on “do not give up until flat”, but much noisier;
- `trading-hybrid` reveals how much ambiguity appears once exit, protection, and cleanup are interleaved.

Conclusion:

- scheduled flatten semantics should be standardized closer to `sessiongap`;
- transport-failure retry semantics on market exit should be borrowed from `alor-usdrubf`, but bounded and more explicit.

### 3. Protective follow-up intents

Observed patterns:

- `sessiongap` does not exercise this class in the same way.
- `trading-hybrid` repeatedly exposed the full lifecycle:
  - TP place
  - SL create
  - transport failure
  - repair
  - cancel / delete stop limit
  - occasional `orphan_trade`
- `trading-alor-usdrubf` mostly avoided this exact shape because its operational emphasis was on market retry / position reconciliation rather than bracket management.

Engineering reading:

- only `trading-hybrid` gives enough evidence for the protective-order class;
- however, its current implementation is not a “best clean solution”, only the most complete exercised one.

Conclusion:

- protective intent lifecycle needs its own standardized contract after soak;
- it should not remain strategy-private glue inside hybrid-only semantics.

### 4. Retry after transport failure

Observed patterns:

- `sessiongap` had relatively few transport-failure cases on core entry/exit.
- `trading-hybrid` often reacted through partial repair paths, temporary `BLOCKED`, protective retry, or later cleanup.
- `trading-alor-usdrubf` repeatedly used the clearest defer-and-retry model:
  - reject on first bar,
  - mark deferred,
  - restore / clear guard,
  - retry on next eligible bar,
  - converge to flat/open.

Engineering reading:

- `alor-usdrubf` currently provides the best operational pattern for transport-failure recovery on market intents;
- but the policy is too strategy-owned and too specific in logs / state transitions.

Conclusion:

- transport-failure retry should become a normalized host-level policy surface for entry and exit intents;
- it should not be re-invented separately per strategy.

### 5. Guard-drop before emit vs emitted-then-timeout

Observed patterns:

- `sessiongap` mostly looks binary and clean: either the system is ready and emits, or it is not.
- `trading-hybrid` often transitions through `BLOCKED` around repair / gateway instability, but the operator view is not always cleanly separated between “not emitted” and “emitted but failed”.
- `trading-alor-usdrubf` explicitly demonstrated both cases:
  - many bars where intent was emitted and later timed out,
  - then a bar where `intent_dropped_by_guard` happened before emit,
  - then `strategy_state_transition_reverted`,
  - then later successful retry.

Engineering reading:

- this distinction is operationally important and should be first-class everywhere.

Conclusion:

- all strategies should expose the same distinction:
  - dropped before emit,
  - emitted but rejected,
  - emitted but transport-timeout,
  - emitted and accepted.

### 6. Broker-truth reconciliation

Observed patterns:

- `sessiongap` usually looks trivial because it is simpler and converges quickly.
- `trading-hybrid` exposed ordering anomalies such as `orphan_trade`, especially on stop or cleanup paths.
- `trading-alor-usdrubf` repeatedly showed explicit broker-position convergence and flat confirmation after noisy retries.

Engineering reading:

- `alor-usdrubf` has the strongest explicit broker-truth awareness;
- `trading-hybrid` shows why this needs to be standardized, because event ordering can otherwise confuse runtime-owned truth.

Conclusion:

- broker-truth convergence should be made more uniform across strategies;
- especially for late trades, cleanup after flat, and inflight-exit clearing.

## What Is Strategy-Specific vs What Is Engineering Drift

### Legitimately strategy-specific

These differences are expected and do not need forced unification:

- signal generation logic
- whether a strategy uses bracket protection at all
- whether exits are time-cut, stop-driven, or EOD-driven
- whether entries are `place` or `market`

### Engineering drift that should probably be reduced

These differences currently look larger than necessary:

- how intent lifecycle states are named and surfaced
- how guard drops are logged vs rejected intents
- how deferred retry is represented
- how protective repair / cleanup is surfaced
- how broker-truth convergence is logged
- how event-ordering anomalies are handled and explained

## Best-of-Breed Reference by Intent Class

### Use `sessiongap` as the reference for

- simple entry state transitions
- simple exit state transitions
- clean phase naming
- operator-readable one-shot lifecycle logs

### Use `alor-usdrubf` as the reference for

- deferred retry after transport failure
- explicit broker-truth convergence
- “do not silently forget the exit” behavior

### Use `trading-hybrid` as the reference only for

- what kinds of protective-order lifecycle states must be modeled
- what cleanup semantics are required once brackets exist

But not as the stylistic baseline for overall intent handling, because its current path is the noisiest.

## Engineering Conclusion

Yes, the soak evidence supports the concern:

- the systems behaved differently for the same intent classes in ways that are not fully justified by strategy logic alone.

The strongest conclusion is not:

- “copy one strategy everywhere”

but rather:

- “extract the best behavior per intent class and make the host semantics more uniform after soak.”

## Recommended Post-Soak Follow-Up

### 1. Standardize the intent lifecycle contract

Introduce a clearer common lifecycle across strategies, at least conceptually:

- `signal_generated`
- `intent_prepared`
- `intent_dropped_by_guard`
- `intent_emitted`
- `command_acknowledged`
- `command_rejected`
- `execution_confirmed`
- `broker_position_converged`
- `cleanup_completed`

### 2. Standardize reject taxonomy

All strategies should classify failures through the same coarse buckets:

- `guard_block`
- `transport_reset`
- `transport_timeout`
- `trading_window_closed`
- `broker_reject`
- `late_trade_or_orphan_trade`

### 3. Unify deferred retry semantics

For entry and exit intents:

- explicit deferred state
- explicit next retry reason
- explicit inflight clearing rule
- bounded retry / escalation policy

### 4. Make protective lifecycle a first-class host concern

Especially for `trading-hybrid`-style TP / SL / cleanup:

- protective install
- protective repair
- protective cancel / delete
- cleanup while flat

should be modeled consistently rather than remaining mostly strategy-private operational glue.

### 5. Keep one operator vocabulary

The soak shows that engineering confidence improves when logs read similarly across strategies.

We should aim for:

- same lifecycle nouns
- same risk-state nouns
- same retry nouns
- same broker-convergence nouns

even if the strategies themselves remain different.

## Immediate Practical Takeaway

For the final soak assessment, the systems should not be judged only as:

- “clean” or “noisy”

but also as:

- how reusable their intent-handling pattern is as a future host-level baseline.

At the moment:

- `sessiongap` is the best baseline for clean one-shot intent lifecycle,
- `alor-usdrubf` is the best baseline for persistence under transport failure,
- `trading-hybrid` is the best evidence source for what a full protective-order contract still needs to cover.
