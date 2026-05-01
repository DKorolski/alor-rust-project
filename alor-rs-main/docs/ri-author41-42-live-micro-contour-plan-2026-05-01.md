# RI Author41/42 Live Micro Contour Plan

Date: `2026-05-01`

Related documents:

- [`moex-author41-42-shadow-implementation-plan-2026-04-28.md`](./moex-author41-42-shadow-implementation-plan-2026-04-28.md)
- [`ri-shadow-vps-rollout-2026-04-28.md`](./ri-shadow-vps-rollout-2026-04-28.md)
- [`vps-live-observations-2026-05-01.md`](./vps-live-observations-2026-05-01.md)
- [`intent-path-unification-fix-plan-2026-04-17.md`](./intent-path-unification-fix-plan-2026-04-17.md)
- [`live-incident-note-2026-04-17-trading-hybrid-bo-exit-break2-request-id-skew.md`](./live-incident-note-2026-04-17-trading-hybrid-bo-exit-break2-request-id-skew.md)

Status:

```text
DESIGN_LOCK_CANDIDATE / SHADOW_FIRST / NO_LIVE_ORDERS_YET
```

## Decision

RI Author41/42 should be implemented as an isolated live/micro contour, not as
a direct reuse of `sessiongap`, `hybrid`, or `alor-usdrubf` runtime state.

The implementation should reuse proven design patterns from the existing live
systems:

- action-scoped CWS command path;
- isolated Redis state and consumer groups;
- explicit MR/BO ownership and no-overlap arbitration;
- no-overnight and gap-flatten safety;
- from-zero rollout discipline;
- operator-readable lifecycle logs and append-only observation journals.

Recommended target:

```text
strategy_kind = ri_author41_42
symbol        = RIM6 / RI active contract mapping
timeframe     = 10m canonical
mode_1        = shadow
mode_2        = micro_live_size_1
execution     = action_scoped_only
components    = author41_mr + author42_bo
```

## Why Not Reuse Existing Strategy Directly

### SessionGap

`sessiongap` is too simple structurally. It is a good one-shot baseline, but RI
needs two component branches, no-overlap arbitration, component attribution, and
more detailed model-state journaling.

### Existing IMOEXF Hybrid

`hybrid` is the best design reference, but not the right direct inheritance
target. It already contains IMOEXF-specific risk-gate, session, and BO/MR
details. RI should not inherit those hidden assumptions.

Use from `hybrid`:

- component ownership;
- one active branch at a time;
- MR priority over BO where the frozen combo contract requires it;
- clear `entry/exit/suppressed/deferred` logs;
- no-overnight/gap-flatten safety pattern.

Do not copy from `hybrid`:

- IMOEXF riskgate high180/lb120 state;
- IMOEXF-specific session quirks;
- protective TP/SL paths unless RI model explicitly requires them;
- live state keys or consumer groups.

### AlorUsdrubf

`alor-usdrubf` has useful retry/broker-truth lessons, but too much instrument
and strategy-specific behavior. RI should borrow the stability discipline, not
the strategy implementation.

## Core Live Design Principles

### 1. Isolated Contour

RI must have its own operational namespace:

```text
strategy_id / profile_id
runtime state key
Redis consumer group
command stream namespace
ack stream namespace
shadow/live journal path
reports directory
docker compose stack or isolated service section
runbook
```

This reduces regression risk when IMOEXF, USDRUBF, and RI evolve at different
paces.

### 2. Action-Scoped Execution Only

Live order emission must use the action-scoped CWS path only.

Reason:

- previous live soaks showed legacy long-lived CWS path fragility;
- action-scoped command sessions with token refresh were the stable path;
- create/market, create/limit, cancel, and cleanup semantics should not fall
  back to older CWS contours.

Acceptance:

```text
Every RI live order log must show action-scoped execution metadata.
No RI live command may use legacy long-lived CWS as its primary path.
```

Required command classes:

- entry order;
- model exit order;
- EOD/gap-flatten exit;
- cancel/cleanup if working orders are introduced later.

If a command class is not yet supported by action-scoped execution, RI live
promotion is blocked until support is added or the command class is removed
from the live contract.

### 3. Frozen 10m Model Layer

The current Author41/42 handoff and shadow validation are based on the frozen
`10m` contract. The live strategy should not silently reinterpret the model
from `1m` bars or ticks.

Contract:

```text
feed      = canonical RI 10m bars
session   = regular MOEX weekday session
service bars and weekends do not update model state
```

If a future implementation reconstructs RI from `1m` or tick data, it must be
treated as a separate parity branch.

### 4. Component Ownership

Every live intent must carry component ownership:

```text
owner = author41_mr | author42_bo | safety_flatten
cycle_id
model_signal_ts
source_bar_ts
intent_class = entry | exit | flatten | cleanup
```

The runtime must be able to answer:

- which component opened the position;
- which component requested exit;
- whether BO was suppressed because MR was active;
- whether an exit is model-driven or safety-driven.

### 5. No-Overlap Arbitration

The frozen combo contract removes BO trades that overlap MR trades.

Live rule:

```text
MR has priority over BO.
BO must not open while MR position/cycle is active.
BO suppression must be logged with reason = mr_overlap_or_active.
```

This rule should be enforced before order emission, not after broker feedback.

### 6. No Overnight / Gap Flatten

RI live/micro must not intentionally carry model positions through non-tradable
gaps.

Required behavior:

- flatten by the model's same-day exit rule when available;
- if the regular session ends while a position remains open, use safety
  `gap_flatten`;
- do not carry BO or MR into the next regular session;
- log all safety exits separately from model exits.

Status:

```text
gap_flatten = live safety contract
not part of frozen parity claim
```

### 7. Request-Id And Pending-State Discipline

RI must not repeat the request-id skew class found in the hybrid BO exit
incident.

Rule:

```text
Runtime is the source of truth for emitted request_id.
Strategy-owned pending state must store the exact emitted request_id after
runtime finalization, not a separately precomputed id.
```

Closed-window or non-tradable-period exits should be deferred before emit when
the condition is locally known. Gateway rejects remain a safety net, not the
normal control path.

### 8. From-Zero Rollout

First live/micro deployment should start from a clean operational state:

```text
flat account
no working orders
no stop orders
fresh RI runtime state
fresh RI consumer group
active journal archived/reset
shadow journal retained separately for review
```

Do not reuse shadow runner state as live order state.

## Engineering Architecture Model

The live RI contour should be built as a staged extension around the existing
Author41/42 model module, not as a rewrite of the current shadow runner.

High-level shape:

```text
10m RI feed
  -> feed guard / session policy
  -> Author41/42 model core
  -> decision journal
  -> live adapter gate
  -> Strategy Intent
  -> runtime command builder
  -> action-scoped gateway path
  -> broker events / reconciliation
```

### Code Placement

Existing model code should remain the source for signal semantics:

```text
strategy-runtime/src/strategies/moex_author41_42.rs
```

New live contour code should be isolated, for example:

```text
strategy-runtime/src/strategies/ri_author41_42_live.rs
```

or, if the first implementation is deliberately split into smaller pieces:

```text
strategy-runtime/src/strategies/ri_author41_42/
  mod.rs
  state.rs
  adapter.rs
  lifecycle.rs
  journal.rs
```

The existing shadow runner should remain a shadow/diagnostic tool:

```text
strategy-runtime/src/bin/moex_author41_42_shadow_runner.rs
```

It should not become the live order-emitting process.

### StrategyKind And Config Boundary

Add a dedicated strategy kind only when the live skeleton is ready:

```text
StrategyKind::RiAuthor41_42
```

The config should be explicit about mode and order-emission permission:

```toml
[strategy]
strategy_kind = "ri_author41_42"
strategy_id = "ri_author41_42.micro.7502XXX"
symbol = "RIM6"
timeframe = "10m"
profile_id = "ri_author41_42_primary_combo_cost2"
mode = "shadow"              # shadow | dry_run | micro_live
allow_order_emission = false # must remain false until GO
qty = 1
execution_path = "action_scoped_only"
```

The presence of a live-capable strategy kind must not imply live permission.
Order emission must require both:

```text
mode = micro_live
allow_order_emission = true
```

and the regular runtime `allow_live_orders` gate.

### Model Core Contract

The model core should expose deterministic decisions without knowing about
broker state:

```text
ModelInput:
  closed 10m bar
  regular session context
  prior regular-day anchors
  current component state

ModelOutput:
  component = author41_mr | author42_bo
  decision = enter | exit | suppress | hold
  side
  model timestamps
  reason
  no-overlap decision
  shadow pnl fields where available
```

The model core must not create `Intent` directly. This keeps signal parity and
execution semantics separate.

### Live Adapter Contract

The live adapter is the only layer that converts model decisions into runtime
intents.

Responsibilities:

- check `mode` and `allow_order_emission`;
- enforce one active component at a time;
- translate accepted entries/exits into `Intent::Market` or `Intent::Place`
  according to the frozen live contract;
- add `IntentClass::Entry` / `IntentClass::Exit` explicitly;
- attach operator-readable comments/tags;
- suppress or defer decisions when the exchange window is closed;
- never emit if broker/runtime state is ambiguous.

The adapter should log every decision as one of:

```text
shadow_recorded
intent_suppressed
intent_deferred
intent_emitted
manual_intervention_required
```

### State Boundaries

RI should keep separate state layers.

Model/live strategy state:

```text
current_component_owner
current_cycle_id
current_side
pending_entry_request_id
pending_exit_request_id
last_model_bar_ts
last_decision_hash
deferred_exit
broker_position_view
```

Shadow journal:

```text
append-only model decisions and finalized shadow records
```

Command lifecycle:

```text
runtime command stream / ack stream / broker events
```

Operational observation:

```text
daily VPS notes and compacted review artifacts
```

The live strategy state must not be rebuilt from the shadow journal. Shadow
journal is evidence, not live state.

### Broker Reconciliation Model

On startup and after reconnect, RI must reconcile:

```text
broker position
working orders
runtime pending request ids
current component owner
```

Conservative rule:

```text
If broker state is not flat and runtime cannot prove ownership, block new
entries and emit manual_intervention_required.
```

Exit intents may remain allowed under a strict safety path if the position side
and quantity are known.

### Execution Model

Initial micro-live should prefer the simplest executable order surface:

```text
entry = market or clearly marketable limit, action-scoped
exit  = market or clearly marketable limit, action-scoped
protective exchange-native TP/SL = not in P0 unless model contract requires it
```

Rationale:

- the Author41/42 frozen model does not require exchange-native protective
  stop-limit orders as its primary semantics;
- previous live incidents showed protective CWS paths are more operationally
  fragile;
- the first RI micro goal is model-transfer validation, not maximizing
  execution finesse.

If later RI needs passive/limit execution, add it as a separate execution
profile after the marketable P0 contour is validated.

### Observability Contract

Minimum live logs:

```text
ri_model_decision
ri_no_overlap_suppressed
ri_intent_suppressed
ri_intent_deferred
ri_intent_emitted
ri_command_accepted
ri_command_rejected
ri_position_reconciled
ri_gap_flatten_triggered
ri_manual_intervention_required
```

Every emitted command must be traceable by:

```text
component
cycle_id
model_signal_ts
bar_ts
request_id
intent_class
execution_path
broker_order_id
```

### Work Allowed Before GO/NO-GO

The following work can proceed while RI shadow soak continues:

- architecture and live contract documentation;
- config schema scaffolding with `allow_order_emission=false` default;
- isolated strategy skeleton registered behind tests;
- model-output-to-decision data structures;
- shadow/dry-run adapter that produces no intents;
- journal/observability schema;
- action-scoped path tests for command classes;
- from-zero runbook and VPS config templates with live disabled.

The following must wait for GO/NO-GO:

- enabling `mode = micro_live`;
- setting `allow_order_emission=true`;
- deploying a live order-emitting RI service;
- increasing size above `1`;
- adding more complex execution profiles.

## Implementation Checkpoint

### 2026-05-01 Safe Scaffold

Implemented a first safe runtime scaffold:

```text
StrategyKind::RiAuthor4142
external config value = "ri_author41_42"
module = strategy-runtime/src/strategies/ri_author41_42_live.rs
```

Scope:

- dedicated `strategy_kind`;
- config payload under `[strategy.ri_author41_42]`;
- runtime modes: `shadow`, `dry_run`, `micro_live`;
- execution path enum with only `action_scoped_only`;
- strategy state payload for RI scaffold counters;
- registry integration;
- config loader support;
- disabled shadow config template:
  `configs/runtime.ri_author41_42.shadow.example.toml`.

Safety behavior:

```text
mode = shadow         -> allowed, emits no intents
mode = dry_run        -> allowed, emits no intents
mode = micro_live     -> rejected until GO/NO-GO
allow_order_emission=true -> rejected until GO/NO-GO
```

The scaffold observes bars and updates local model-bar counters, but it does
not yet adapt Author41/42 model decisions into live intents.

Validation:

```text
cargo test -p strategy-runtime
```

Result:

```text
PASS
```

### 2026-05-01 ModelDecision Dry-Run Layer

Added a safe `ModelDecision` bridge inside the RI scaffold.

Scope:

- keep an in-memory canonical `10m` model-bar buffer;
- reuse the existing Author41/42 shadow engine:
  `build_ri_author41_42_combo_shadow_journal`;
- convert finalized shadow records into internal dry-run decisions;
- map accepted MR/BO records to `enter` decisions;
- map BO records dropped by MR overlap to `suppress` decisions;
- log `ri_model_decision`;
- keep `Vec<Intent>` empty in all modes currently allowed before GO/NO-GO.

Provisional-tail guard:

```text
same-day forced_last_bar_close records are not treated as finalized decisions
```

This preserves the watermark fix semantics and avoids reintroducing duplicate
same-day BO tail artifacts through the future live scaffold.

Validation:

```text
cargo test -p strategy-runtime ri_author41_42_live -- --nocapture
```

Result:

```text
PASS: 6 RI live scaffold tests
```

## Work Packages

### WP1. Live Contract Document

Freeze:

- RI profile id;
- model component ids;
- 10m feed/session policy;
- no-overlap arbitration;
- exit and gap-flatten rules;
- command classes allowed in live mode;
- action-scoped-only execution requirement.

Output:

```text
docs/ri-author41-42-live-contract-YYYY-MM-DD.md
```

### WP2. Isolated Strategy Skeleton

Create an isolated runtime contour:

```text
strategy_kind = ri_author41_42
mode = shadow | paper_disabled | micro_live
```

Initial implementation should be able to run in shadow mode with no order
emission.

Acceptance:

```text
NO_ORDER_EMISSION_PATH remains true until explicit micro-live switch.
```

### WP3. Reuse Shadow Model Engine

Use the already validated Author41/42 shadow model engine as the model source.

Do not retune:

- K values;
- filters;
- no-overlap logic;
- cost conventions;
- source exit reason taxonomy.

### WP4. Intent Adapter

Add a narrow adapter from finalized model decisions to live intents:

```text
author41_mr entry/exit
author42_bo entry/exit
safety_flatten
```

The adapter should be disabled in shadow mode.

Acceptance:

```text
same model decision can be logged in shadow mode and converted to intent only
when mode = micro_live.
```

### WP5. Action-Scoped Gateway Coverage

Verify or implement action-scoped command support for every RI live command
class.

Acceptance logs must include:

```text
action_scope_session_open_start
action_scope_authorize_ok
cws_*_ack status=accepted|working|rejected
request_id
broker_order_id when available
```

NO-GO if any RI command uses legacy long-lived CWS as the primary path.

### WP6. State And Journal Layout

Separate state layers:

- shadow/model journal;
- live runtime state;
- command/ack lifecycle;
- operator observation journal.

Required journal fields:

```text
component
cycle_id
model_signal_ts
bar_ts
side
entry/exit reason
no-overlap decision
emit/defer/suppress decision
request_id when emitted
broker_order_id when accepted
position_before/after when known
```

### WP7. Safety And Recovery

Implement:

- broker-flat reconciliation at startup;
- working-order scan before live enable;
- no-overnight flatten;
- closed-window pre-emit defer where applicable;
- stale pending detection;
- explicit `manual_intervention_required` log if broker state and runtime state
  diverge.

### WP8. Test Matrix

Minimum test scenarios:

- feed guard excludes service/weekend bars;
- MR entry/exit decision creates only shadow record in shadow mode;
- BO suppressed while MR active;
- gap-flatten creates safety exit and does not carry overnight;
- closed-window exit is deferred before emit;
- emitted request id matches pending state;
- action-scoped command path is selected for each command class;
- from-zero startup does not load stale shadow state as live state.

### WP9. VPS Shadow-To-Micro Rollout

Rollout order:

1. Continue RI shadow for 3-5 more trading sessions after the watermark patch.
2. Review active post-patch journal for duplicate-free finalized records.
3. Build live/micro image and configs, but keep mode disabled.
4. Dry-run startup with no order emission.
5. From-zero micro rollout only when account is flat and no working orders
   exist.
6. Start at size `1`.

## Promotion Gates

### GO To Micro-Live Size 1

All must be true:

- 3-5 additional RI trading sessions observed after watermark patch;
- active RI journal contains only finalized records;
- no duplicate same-day provisional BO rows;
- Redis consumer lag/pending remains stable;
- model decisions remain explainable against 10m feed;
- live adapter passes tests;
- action-scoped path is confirmed for all command classes;
- from-zero runbook is ready;
- manual broker flat check is complete.

### NO-GO

Any of the following blocks promotion:

- legacy CWS path used for live order emission;
- request-id skew between runtime and pending strategy state;
- stale pending position/order after restart;
- BO/MR overlap creates simultaneous live exposure;
- overnight carry is possible by design;
- live strategy reads shadow journal as live state;
- unexplained journal duplication reappears.

## Current Practical Verdict

RI should remain shadow-only while the post-watermark sample grows.

Target status before implementation starts:

```text
RI_SHADOW_WATERMARK_PATCHED / OBSERVE_3_TO_5_MORE_TRADING_SESSIONS
```

Target implementation direction:

```text
ISOLATED_RI_AUTHOR41_42_CONTOUR
HYBRID_PATTERNS_AS_REFERENCE
ACTION_SCOPED_ONLY_EXECUTION
FROM_ZERO_MICRO_ROLLOUT
```
