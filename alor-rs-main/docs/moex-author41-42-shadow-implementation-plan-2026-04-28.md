# MOEX Author41+42 Shadow Implementation Plan

Date: `2026-04-28`

Source package:

```text
analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04
```

Status:

```text
IMPLEMENTATION_PLANNING / SHADOW_ONLY
```

This note translates the frozen Author41+42 handoff bundle into an engineering
plan for Rust/runtime work. It does not authorize live orders or paper orders.

## Candidate Read

### RI

Recommended path:

```text
RI -> shadow-first candidate
```

Frozen candidate:

```text
ri_author41_42_primary_combo_cost2
```

Source variants:

```text
Author41 MR: dual_no_overlap_plateau
Author42 BO: grid_k0.42_both
Combo:       ri_41dual_42best_cost2_nooverlap
```

Current package label:

```text
GO_SHADOW_CANDIDATE
```

Engineering read:

- RI is the stronger cross-instrument confirmation in this branch.
- Gate 5 implementation sensitivity remains positive across tested stress
  scenarios.
- The first Rust contour explicitly accepts the frozen `10m`
  switch-continuous runtime contract for RI shadow. True `1m` roll
  reconstruction is deferred and must not block this initial shadow line.

### IMOEXF

Recommended path:

```text
IMOEXF -> passive shadow / watchlist candidate
```

Frozen candidate:

```text
imoexf_author41_42_primary_combo_cost2
```

Source variants:

```text
Author41 MR: author41_boundary_short
Author42 BO: grid_k0.44_both
Combo:       imoexf_41short_42best_cost2_nooverlap
```

Current package label:

```text
GO_SHADOW_LIMITED_CONCENTRATION_REVIEW
```

Engineering read:

- IMOEXF is useful because it is runtime-adjacent to the existing hybrid work.
- It is not as clean as RI: recent-window concentration and execution-cost
  sensitivity remain on the watchlist.
- It should not replace or interfere with the current live `hybrid_imoexf`
  micro-soak stack. Run it as a separate passive shadow/replay journal first.

## Explicit Non-Goals

- No live orders.
- No paper orders from this branch yet.
- No retuning of K values, filters, overlap rules, or cost conventions.
- No silent conversion from the frozen `10m` research contract to `1m` or tick
  semantics.
- No reuse of the current live hybrid state machine until replay parity proves
  the Author41+42 contract can be represented cleanly.

## Frozen Feed Contract

Both RI and IMOEXF use:

```text
timeframe: 10m
session:   Mon-Fri 09:00..23:49
filter:    exclude service/pre-session bars and weekend/session artifacts
anchors:   previous regular trading day only
```

Feed guard must run before any model state update.

The runtime must not mix:

- frozen `10m` research bars;
- true `1m XX:59` source timing;
- live tick/event trigger semantics.

If a future implementation uses `1m` or tick data, it needs a separate replay
parity branch and cannot be treated as the same frozen model.

## Model Contract

### Author41 MR

Class:

```text
opening-context mean reversion
```

Runtime representation:

- standalone component first;
- closed `10m` bar signal evaluation;
- next-bar-open proxy for replay accounting;
- timed exits and stop/target-like exits must preserve source reason taxonomy.

Required exit reason families:

```text
take_author_close
breakeven_limit
stop
time_exit
```

### Author42 BO

Class:

```text
raw-excess / opening-context breakout
```

Runtime representation:

- standalone component first;
- hourly `XX:50` check in the frozen `10m` reconstruction;
- entry at next `10m` open proxy;
- no Friday entries;
- skip reconstructed seasonal window `May 21 .. July 1`;
- skip first-hour extreme move larger than `1.5 * prev_range`;
- stop-like exits are next-open reactions, not exchange-native intrabar stop
  triggers.

Frozen K values:

```text
IMOEXF: K = 0.44
RI:     K = 0.42
```

BO cost convention:

```text
primary implementation candidate includes cost2 roundtrip
```

### Combo No-Overlap

Primary combo contract:

```text
remove Author42 BO trades that interval-overlap Author41 MR trades
```

No-overlap arbitration must be reproduced before any shadow promotion. The
runtime journal must show whether a BO candidate was accepted or dropped because
of MR overlap.

## Implementation Work Packages

### WP1. Artifact Ingestion And Registry

Create an internal model profile registry for:

```text
ri_author41_42_primary_combo_cost2
imoexf_author41_42_primary_combo_cost2
```

The registry should store:

- instrument;
- profile id;
- component ids;
- frozen parameters;
- timeframe contract;
- session policy;
- source package path;
- source artifact checksum or generation id when available.

### WP2. Feed Guard Fixture

Build replay fixtures that prove:

- only regular `10m` bars enter model state;
- service bars are retained only for audit, not for model state;
- weekends do not create anchors or signals;
- previous-day anchors use previous regular trading day.

Acceptance:

```text
feed_bar_count and regular-session dates match source artifacts or every drift
is explicitly documented.
```

### WP3. Author41 Standalone Replay

Implement Author41 MR as a standalone replay component for each instrument.

Acceptance:

- compare trade date;
- side;
- entry timestamp;
- exit timestamp;
- exit reason;
- net points;
- daily PnL.

Do not start combo integration until standalone MR drift is explained.

### WP4. Author42 Standalone Replay

Implement Author42 BO as a standalone replay component for each instrument.

Acceptance:

- compare candidate day and side;
- hourly check timestamp;
- next-bar entry timestamp;
- stop/timed exit timestamp;
- exit reason;
- daily PnL.

For RI, the first pass uses the accepted frozen `10m` switch-continuous
contract. True `1m` roll reconstruction is a follow-up validation line, not a
blocker for initial shadow replay.

### WP5. Combo Arbitration Replay

Implement no-overlap arbitration:

```text
MR interval has priority over overlapping BO interval.
```

Acceptance:

- count BO candidates before overlap removal;
- count BO candidates dropped by overlap;
- compare combo daily PnL;
- preserve component attribution in output.

### WP6. Shadow Journal

Add shadow-only journal output. Required fields:

- instrument;
- profile id;
- component: `author41_mr` or `author42_bo`;
- model variant id;
- bar timestamp;
- timeframe;
- previous regular day close/high/low/range;
- trigger levels;
- signal condition values;
- side;
- skip reason;
- scheduled entry timestamp;
- scheduled entry price proxy;
- scheduled exit timestamp;
- exit reason;
- no-overlap decision;
- shadow PnL under frozen contract;
- feed-quality flags.

The journal must be sufficient to explain every signal and every skipped signal
without looking at the live runtime internals.

### WP7. Runtime Shadow Admission

Admission order:

1. RI shadow-only.
2. IMOEXF passive shadow/watchlist.

Runtime mode:

```text
shadow_only = true
emit_orders = false
```

The shadow process may read live/captured bars and write journals, but it must
not publish order commands.

## GO / NO-GO Gates

GO to shadow-only if:

- feed guard matches frozen contract;
- Author41 standalone drift is explained;
- Author42 standalone drift is explained;
- no-overlap replay is implemented;
- component and combo daily PnL drift is within documented tolerance;
- journal can explain all signals and skips;
- runtime cannot emit live/paper orders in this mode.

NO-GO if:

- service/weekend bars affect state;
- K values or filters are changed during implementation;
- no-overlap is approximated instead of reproduced;
- paper/live order paths are enabled before replay parity;
- RI implementation silently drifts away from the accepted `10m`
  switch-continuous contract;
- IMOEXF concentration watchlist is ignored.

## Accepted Decisions

1. RI feed source:

```text
Use frozen 10m switch-continuous contract for first shadow.
Defer true 1m roll reconstruction to a follow-up validation line.
```

## Open Engineering Decisions

1. Runtime placement:

```text
Prefer a separate shadow/replay component or runner rather than extending the
current live hybrid order-emitting stack immediately.
```

2. Persistence:

```text
Shadow journal can be append-only files/Redis streams, but it must be clearly
separate from live strategy state and order lifecycle state.
```

3. IMOEXF relationship to current live hybrid:

```text
Run as passive observer beside the existing hybrid, not as replacement logic.
```

## Recommended Next Step

Start with a narrow Rust replay harness:

```text
RI Author41 standalone -> RI Author42 standalone -> RI combo no-overlap
```

Only after RI replay parity is explainable should we attach live shadow
journaling. IMOEXF should follow as a passive/watchlist contour using the same
framework.

## Implementation Progress

### 2026-04-28 Scaffold

Created a separate `strategy-runtime` model module:

```text
strategy-runtime/src/strategies/moex_author41_42.rs
```

Scope of this scaffold:

- frozen RI and IMOEXF profile registry;
- accepted RI `10m` switch-continuous shadow profile;
- IMOEXF passive-shadow/watchlist profile;
- regular MOEX `10m` feed guard (`Mon-Fri 09:00..23:49`);
- shadow journal record schema;
- explicit shadow/replay modes that cannot emit orders.

Safety boundary:

```text
The scaffold does not implement Strategy and does not return Intent.
It is therefore not connected to live/paper order emission.
```

Initial test coverage:

```text
cargo test -p strategy-runtime moex_author41_42 -- --nocapture
```

Result:

```text
PASS: 4 tests
```

### 2026-04-28 Artifact Loader

Extended the scaffold with source artifact readers for the fixed handoff CSVs:

```text
fixed_candidate_trades.csv
fixed_candidate_daily.csv
```

The loader handles the mixed source-column conventions used by the package:

```text
IMOEXF Author41 MR trades: net_points
RI Author41 MR trades:     points_pnl
Author42 BO trades:        pnl_points
Combo daily rows:          pnl
```

This keeps the next replay/parity step honest: component trades and combo daily
rows are read as source artifacts first, before any Rust model logic tries to
reproduce them.

Updated test coverage:

```text
cargo test -p strategy-runtime moex_author41_42 -- --nocapture
```

Result:

```text
PASS: 6 tests
```
