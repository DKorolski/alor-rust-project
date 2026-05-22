# RI Author41/42 Live Contract

Date: 2026-05-01

Status: `MICRO_CONTOUR_PREPARED / SERVICE_HARDENING_PATCH_READY`

This document freezes the first engineering contract for the isolated RI
Author41/42 live contour. It is intentionally conservative: shadow/dry-run
remain the research/observation modes, and micro-live size `1` requires explicit
operator GO, broker-flat confirmation, and from-zero runtime rollout discipline.

## Scope

The live contour is an isolated strategy:

```text
strategy_kind = ri_author41_42
profile_id = ri_author41_42_primary_combo_cost2
timeframe = 10m
symbol = RIM6
qty = 1
```

Current modes:

- `shadow`
- `dry_run`
- `micro_live` only after explicit operator GO and service-hardening rebuild

Blocked until a separate later GO/NO-GO:

- any size above `1`;
- any legacy CWS primary path;
- any contract roll without from-zero restart.

## Model Contract

The model source is the frozen RI Author41/42 10m handoff profile.

The runtime must not retune:

- Author41 MR parameters;
- Author42 BO parameters;
- overlap arbitration;
- cost conventions;
- source exit reason taxonomy.

The model feed contract is:

- 10m bars only;
- regular weekday model session only;
- no weekend model bars;
- no pre-session/service bars in model state;
- raw/audit feeds may retain service bars, but they are not model inputs.

## Component Contract

Components:

- `author41_mr`
- `author42_bo`

Overlap rule:

- MR and BO must not create simultaneous live exposure;
- accepted decisions may produce one entry leg and one exit leg;
- dropped overlap decisions are journaled as suppressions, not as live intents.

## Execution Contract

RI live execution is action-scoped only.

Required config:

```text
execution_path = action_scoped_only
```

Legacy long-lived CWS is not an accepted primary path for RI commands.

Candidate order style:

```text
market_p0
```

Candidate intent classes:

- entry leg -> `IntentClass::Entry`
- exit leg -> `IntentClass::Exit`
- future safety flatten leg -> `IntentClass::Exit`

The adapter may build candidate intents before GO, but it must suppress them
and write observation records. It must not return order-emitting `Intent`s
while `micro_live` and `allow_order_emission=true` are blocked.

## MR Exit Execution Contract

2026-05-08 analyst verdict:

```text
TP bracket / passive limit is not the primary live micro contract.
Keep Author41 MR take-profit semantics as closed-bar condition
(`take_author_close`) followed by marketable/action-scoped exit.
```

Rationale:

- the frozen RI parity contract models `take_author_close`, not a touch-based
  TP limit;
- TP-limit variants looked cosmetically better by win-rate but worse by
  expectancy in the analyst review;
- switching TP to a resting broker limit would create a live execution overlay
  and should not be mixed into the current parity-validation micro soak.

SL handling:

- research `stop` already behaves like a level-touch condition;
- broker-side SL / stop-limit protection may be evaluated separately as an
  operational safety overlay;
- enabling SL bracket protection requires a separate design/review/test line and
  must remain action-scoped only;
- SL bracket discussion must not implicitly enable TP bracket semantics.

Current live micro contract:

```text
MR entry: action-scoped marketable order
MR take_author_close: closed-bar condition -> action-scoped marketable exit
MR stop/time/breakeven exits: current model condition -> action-scoped marketable exit
TP resting limit: disabled in primary contract
SL broker bracket: future safety candidate, not enabled by default
```

## Safety Contract

Startup and restart must be conservative:

- non-flat broker snapshot -> `manual_intervention_required`;
- working order snapshot -> `manual_intervention_required`;
- working stop-order snapshot -> `manual_intervention_required`;
- restored pending request ids -> `manual_intervention_required`;
- restored known order ids -> `manual_intervention_required`;
- empty broker/runtime state may remain `flat`.

No-overnight rule:

- BO must not carry across non-tradable gaps in live/micro contour;
- gap flatten is a live safety overlay, not a frozen parity claim.

Contract roll rule:

- roll to the next RI futures contract 7 calendar days before current contract
  expiry;
- perform roll only between trading sessions;
- confirm broker-flat and no working orders before roll;
- change `symbol` / feed config to the next contract;
- restart the RI contour from zero for runtime/live state;
- warmup/history must be loaded from the new contract, not from the expired
  contract;
- do not run an intraday cross-contract transfer or hedge.

Closed-window rule:

- closed-window exits must not become stale live pending state;
- preferred behavior is pre-emit defer/safety handling before broker command
  emission;
- gateway reject handling remains a residual safety net.

## State Boundaries

Live pending request ids:

- runtime is the source of truth for final emitted `request_id`;
- strategy receives the exact id through `on_command_prepared`;
- `pending_entry_request_id` and `pending_exit_request_id` are persisted in
  `StrategyState::RiAuthor4142Live`;
- ack/reject callbacks clear pending state only for matching ids;
- mismatched ids must log `ri_pending_request_id_skew_detected` and leave
  pending state intact.

Shadow/model journal:

- source for observed model decisions and dry-run evidence;
- not a live position source.

Runtime state:

- current operational phase;
- current dry-run component/side/cycle;
- pending restore guards.

Broker truth:

- startup flat check;
- working order scan;
- manual flat check before any future micro-live rollout.

## Observability Contract

Every observed decision should preserve:

- component;
- cycle id;
- model signal timestamp;
- bar timestamp;
- side;
- entry/exit reason;
- no-overlap decision;
- emit/defer/suppress decision;
- request id when emitted;
- broker order id when accepted;
- position before/after when known.

Pre-GO journal records must show:

```text
adapter_decision = shadow_recorded | intent_suppressed | manual_intervention_required
execution_path = action_scoped_only
request_id = null
broker_order_id = null
```

## Promotion Gate

RI can be considered for micro-live size 1 only after:

- 3-5 additional post-watermark trading sessions are reviewed;
- finalized shadow journal is duplicate-free;
- model decisions remain explainable against the 10m feed;
- action-scoped coverage is confirmed for all future RI command classes;
- from-zero runbook is ready;
- broker/account is manually confirmed flat with no working orders.

Any legacy CWS primary path, stale pending tail, request-id skew, BO/MR live
overlap, or overnight carry possibility is a NO-GO.

Post-2026-05-07 service-hardening rollout gate:

- build includes commits from
  [`ri-author41-42-service-hardening-rollout-checklist-2026-05-07.md`](./ri-author41-42-service-hardening-rollout-checklist-2026-05-07.md);
- RI operational runtime state starts from zero;
- canonical `RIM6` 10m model bars are retained for warmup;
- broker account is flat and has no RI/RTS working orders or stop orders;
- first patched observation window remains size `1`.
