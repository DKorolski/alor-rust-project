# IMOEXF Primary Runtime Integration Review Handoff (2026-04-26)

## Status

Current read:

```text
CONDITIONAL_GO_CANDIDATE / ARCHITECTURE_REVIEW_REQUIRED
```

This is not an unconditional production GO. The replay/parity package is now
strong enough to support a conditional extended micro soak at size `1`, but only
after the runtime integration design below is accepted and implemented narrowly.

## What Is Done

- Rust replay profile exists for the primary candidate:
  `imoexf_primary_riskgate_k053`.
- Filtered model-feed bundle builder exists:
  `scripts/build_imoexf_filtered_bundle.py`.
- One-command review pipeline exists:
  `scripts/run_imoexf_primary_parity_review.py`.
- Layer diagnostics exist for:
  - saved source vs Rust trade diff;
  - MR residual root-cause read;
  - BO execution-contract drift.
- Consolidated review report exists:
  `docs/imoexf-primary-parity-review-report-2026-04-26.md`.
- Replay model-feed guard is implemented in the review path:
  Monday-Friday `09:00..23:49` only.
- Replay High180 MR was aligned with source-like mechanics:
  - running midpoint target;
  - midpoint-side guard;
  - `max_hold = 180` minutes;
  - risk gate with `lookback = 120` sessions and `min_history = 60`;
  - maker-like shadow cost `0.1` point.
- BO no-overnight/gap behavior is explicit in replay through
  `bo_gap_flatten` and `--assert-gap-flatten`.

## Current Review Facts

Filtered bundle generated from raw/audit data through `2026-04-21`:

```text
rows:                    52572
first_ts:                2023-11-14 10:00:00
last_ts:                 2026-04-21 23:40:00
weekend_rows:            0
pre_session_rows:        0
post_session_rows:       0
non_monotonic_rows:      0
regular_model_contract:  true
```

Rust replay output:

```text
trades:                  846
mean_reversion:          471
intraday_breakout:       375
gap_flatten_violations:    0
bo_gap_flatten_actions:    7
```

MR read:

```text
saved-source MR vs Rust:       474 vs 471, exact 464
filtered canonical MR vs Rust: 471 vs 471, exact 470
```

The remaining filtered-canonical `1 / 1` MR shift is a BO gap-flatten
interaction, not broad MR signal drift.

BO read:

```text
fill-level exact:              0 / 370
entry-signal common after -10m: 361 / 370
entry+exit signal common:      350 / 370
source cross-day carry:        9
Rust gap-flatten/cross-day:    7
```

Interpretation:

```text
SIGNAL_NEAR_PARITY / EXECUTION_CONTRACT_DRIFT_EXPLICIT
```

BO fill-level exact parity is intentionally not the acceptance metric while
comparing Backtrader next-bar fills with Rust close-bar/event-loop behavior.

## Seed Artifact Review

Added seed artifacts:

- `docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts/riskgate_high180_lb120_seed_2026-04-26.csv`
- `docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts/riskgate_high180_lb120_seed_2026-04-26_metadata.json`
- `docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts/RISK_GATE_SEED_STATE.md`

Mechanical validation:

```text
seed rows:                 180
seed first date:           2025-08-06
seed last date:            2026-04-21
weekend rows:              0
non-monotonic rows:        0
source counts:             seed=180
status counts:             complete=180
last-120 shadow PnL:       161.9
MR enabled next session:   true
```

Architectural read:

```text
SEED_ACCEPTABLE_AS_BOOTSTRAP / NOT_PERMANENT_SOURCE_OF_TRUTH
```

The seed is suitable for initial live validation because the risk gate needs
`120` regular sessions with `min_history = 60`. It should not become a hidden
long-lived config file that the strategy rereads as the continuing source of
truth. After import, the runtime should own and persist the regular-session
ledger.

Architecture formula:

```text
Seed    = one-time bootstrap / controlled rebuild artifact
Ledger  = long-lived source of truth for the gate
Snapshot = current operational process state only
```

Repository packaging note:

```text
docs/*-artifacts/ is ignored by alor-rs-main/.gitignore by default.
```

The ignore rule is narrowed for the review-critical files:

- `SERVICE_BAR_EXCLUSION_BACKTEST.md`
- `RISK_GATE_SEED_STATE.md`
- `riskgate_high180_lb120_seed_2026-04-26.csv`
- `riskgate_high180_lb120_seed_2026-04-26_metadata.json`

This keeps the full artifact directory protected from accidental bulk commits
while allowing the seed/state package to be reviewed normally.

## What Is Not Production-Integrated Yet

The current implementation is a replay/review contour, not yet the final live
strategy contour.

Important gaps:

- `High180MrEngine` has been moved into the shared hybrid strategy module and
  the live runtime can select it through `mr_variant = "high180"`.
- Config/profile parsing now recognizes the IMOEXF primary profile and risk-gate
  fields, but non-disabled risk-gate modes still fail fast in the live adapter
  until the ledger/import flow is implemented. This prevents a config from
  silently looking enabled while still running an incomplete gate contour.
- Production runtime can now suppress bars outside an optional model-session
  guard before model-state updates.
- The risk gate requires long historical context:
  `120` trading sessions with `min_history = 60`. It cannot be reconstructed
  from the short Redis retention window used for live operations.
- `bo_gap_flatten` still needs an explicit team decision:
  accepted runtime safety contract vs Backtrader fill-parity mismatch.

## Implementation Progress (2026-04-27)

Completed in the first runtime-integration slice:

- Added shared High180 MR module:
  `strategy-runtime/src/strategies/hybrid_intraday/high180.rs`.
- Added risk-gate session ledger primitives:
  `strategy-runtime/src/strategies/hybrid_intraday/risk_gate.rs`.
- Extracted replay High180 usage to the shared module so replay and future live
  integration use one implementation.
- Added hybrid config fields:
  `profile`, `mr_variant`, `mr_gate_policy`, `risk_gate_mode`,
  `risk_gate_seed_file`, `risk_gate_ledger_key`,
  `model_session_start_time`, and `model_session_end_time`.
- Added TOML loader coverage for those fields under `[strategy.hybrid_intraday]`.
- Added runtime profile/session guard plumbing with a pre-state-update
  `hybrid_model_session_bar_suppressed` log event.
- Added live High180 MR branch inside `HybridIntradayRuntimeStrategy`; Classic
  MR remains the default.
- Added deterministic risk-gate seed/ledger helpers:
  seed CSV parsing, regular-session ledger validation, startup reconciliation
  decisions, next-session gate calculation, and runtime-row construction.
- Added the Redis storage contract helpers:
  canonical stream/state/finalized key names, ledger stream fields, materialized
  fast-state fields, stream-to-state rebuild helpers, and an atomic write
  skeleton in `redis_transport`.
- Added deterministic startup planning:
  validate profile identity, choose import/use-existing/rebuild, produce
  `records_to_write`, and rebuild fast state from the selected canonical ledger.
- Added runtime-facing store helpers:
  load field-based ledger records, load materialized state, and persist startup
  artifacts through the finalized guard while keeping strategy snapshot out of
  the canonical ledger.
- Added the high-level startup-store path:
  load configured seed CSV, load Redis ledger/state, run the deterministic
  planner, and persist guarded startup artifacts as one runtime-facing operation.
- Added adapter tests that allow baseline/profile/High180 parsing while
  deliberately rejecting active risk-gate modes until ledger integration is
  finished.

Validation:

```text
cargo test -p strategy-runtime -- --test-threads=1
```

Result:

```text
passed: 215 lib tests + all strategy-runtime integration/e2e/doc tests
```

Current engineering status:

```text
LIVE_HIGH180_READY / RISK_GATE_ENFORCEMENT_CODE_READY / SHADOW_VALIDATION_CONFIGS_READY
```

## Architecture Review Needed

### 1. Runtime Profile Boundary

Decision needed:

```text
Should IMOEXF primary be a profile inside hybrid_intraday, or a separate
strategy_kind?
```

Preferred direction:

```text
Keep it inside hybrid_intraday as an explicit profile/mode.
```

Suggested config shape:

```text
strategy_kind = "hybrid_intraday"
profile = "imoexf_primary_riskgate_high180_lb120"
mr_variant = "high180"
mr_gate_policy = "shadow_pnl_lb120_positive"
```

Rationale:

- Existing runtime already handles MR/BO ownership, pending lifecycle, live
  order comments, action-scoped paths, protective repair, deferred exits, and
  no-overnight BO guard.
- A new strategy kind would duplicate already-hardened live execution plumbing.

### 2. MR Engine Mode

Decision needed:

```text
How should Classic MR and High180 MR coexist?
```

Preferred direction:

```text
Add an explicit MR mode, for example:
ClassicPrevDayRange
High180
```

The current production MR mode should remain unchanged for the existing hybrid
baseline. The IMOEXF primary profile should opt into High180 only through
config.

Keep signal variant and gate policy separate:

```text
mr_variant = classic_prev_day_range | high180
mr_gate_policy = disabled | shadow_pnl_lb120_positive
```

High180 defines the MR signal/exit logic. The risk gate only decides whether
live MR emission is allowed for a regular session.

### 3. Risk-Gate State Source

Decision needed:

```text
How does live runtime initialize and maintain the 120-session shadow risk gate?
```

Options:

- Seed file generated from the official filtered bundle, used once for initial
  import.
- Runtime-owned append-only regular-session ledger with one row per session.

Preferred direction:

```text
Use a deterministic seed artifact for initial live validation, then make the
runtime-owned session ledger the source of truth.
```

This keeps Redis retention small while preserving the long-horizon model state
needed by the risk gate.

Recommended split:

```text
Canonical Redis session ledger:
  key = runtime.riskgate.sessions.<strategy_id>.<profile_id>
  type = stream
  session_date
  profile_id
  mr_variant
  timeframe = 10m
  session_policy = Mon-Fri 09:00..23:49
  shadow_pnl_points
  shadow_trade_count
  rolling_120_pnl_before_session
  mr_enabled_for_session
  rolling_sum_lb120
  mr_enabled_next_session
  source = seed | runtime
  model_version
  finalized_at_utc

Materialized fast state:
  key = runtime.riskgate.state.<strategy_id>.<profile_id>
  last_finalized_session_date
  rolling_sum_lb120
  mr_enabled_current_session
  mr_enabled_next_session
  seed_loaded
  ledger_rows_count
  current_shadow_session_date
  current_shadow_pnl_points
  current_generation

Dedupe guard:
  key = runtime.riskgate.finalized.<strategy_id>.<profile_id>.<session_date>

Main runtime snapshot:
  current intraday shadow accumulator only
  no canonical 120-session history
```

Required guardrails:

- import seed once;
- if ledger already exists, do not silently reload the seed;
- support explicit modes only:
  `bootstrap_from_seed`, `normal_append`, and `rebuild_from_history`;
- if `ledger_last_session_date >= seed_last_session_date`, continue from ledger
  and ignore seed in normal mode;
- if `ledger_last_session_date < seed_last_session_date`, refuse silent startup
  unless an explicit import/rebuild mode is selected;
- log `seed_loaded`, `seed_rows`, `seed_last_session`, `seed_profile_id`,
  `seed_timeframe = 10m`, `lookback_sessions`, `min_history_sessions`, and the
  session policy;
- preserve the `10m` regular-session layer as canonical for the gate.
- handle gaps in canonical `10m` history explicitly: fill/rebuild, run
  `shadow_only`, keep MR disabled until history is complete, or record an
  operator override.

### 4. Feed Contract Enforcement

Decision needed:

```text
Where should the IMOEXF regular-session model-feed guard live?
```

Preferred direction:

```text
Enforce it in runtime before any model-state update, even if upstream feed is
already filtered.
```

For the first live profile, do not compute the gate from raw `1m` feed. The
confirmed frozen parity layer is `10m`, and the previous transfer gap was partly
timeframe drift. Keep shadow accounting tied to the same `10m` regular-session
contract used by seed, replay, and diagnostics.

Separate execution and risk-accounting sources explicitly:

```text
live execution path      -> existing runtime/live event flow
risk-gate history layer  -> canonical 10m regular-session feed
```

Any future `1m -> 10m` aggregation for gate accounting needs its own explicit
contract and parity check.

Acceptance rule:

```text
Raw/audit may contain service bars such as 08:50.
Model state must not consume them.
```

Affected state:

- MR running midpoint;
- BO anchors/levels;
- risk-gate shadow PnL;
- entry/exit logic;
- replay parity state.

### 5. BO Gap-Flatten Contract

Decision needed:

```text
Is Rust close-bar/no-overnight bo_gap_flatten accepted as the live runtime
contract?
```

Preferred direction:

```text
Accept bo_gap_flatten as the live safety contract.
```

Do not tune Rust toward Backtrader cross-day carry unless the team explicitly
chooses replay-fill parity over no-overnight live safety.

Acceptance wording:

```text
bo_gap_flatten is a live safety overlay for the production profile, not a claim
of frozen Backtrader fill-level replay equivalence.
```

Requirements:

- emit an explicit audit/log event on every `bo_gap_flatten`;
- keep it switchable or classifiable for research/parity reports;
- report it as execution-contract drift, not signal drift.

### 6. Runtime Timer Hook

Decision needed:

```text
Is a strict timer/event-loop flatten required before first extended micro soak?
```

Preferred direction:

```text
Not required for the first conditional soak. Keep it as follow-up nice-to-have.
```

The existing event/bar-loop no-overnight guard is enough for conditional micro
validation. A timer hook can later make the `23:30` flatten stricter when no
bar/event arrives exactly at that time.

For the first soak, prefer bar/event-driven session finalization:

- finalize a session on the last eligible `10m` regular bar, or
- finalize the previous regular session on the first eligible event of the next
  regular session.

Do not use current-session shadow PnL to enable or disable current-session real
MR. The gate decision is for the next regular session.

## Proposed Narrow Patch Line

### Patch 1: Runtime Profile Plumbing

- Add config-level profile/mode for IMOEXF primary.
- Keep existing hybrid baseline unchanged.
- Add explicit `10m` / regular-session assumptions to config docs.
- Status: first slice implemented with live fail-fast for unfinished modes.

### Patch 2: High180 MR Engine

- Move High180 MR logic out of `hybrid_replay.rs` into shared strategy module.
- Keep Classic MR and High180 MR separate.
- Add unit tests for midpoint-side guard and `max_hold = 180`.
- Status: implemented for replay/shared module and live runtime branch.

### Patch 3: Risk-Gate Seed + State

- Define seed artifact format.
- Load seed once at startup/import for IMOEXF primary profile.
- Create or update a runtime-owned append-only session ledger.
- Persist daily shadow PnL forward in the ledger, not as an implicit reread of
  the seed CSV.
- Implement explicit modes:
  `bootstrap_from_seed`, `normal_append`, `rebuild_from_history`.
- Define startup reconciliation for `seed_last_session_date` vs
  `ledger_last_session_date`.
- Define gap handling for missing canonical `10m` regular sessions.
- Add observability for enabled/disabled risk-gate dates.
- Status: seed/state contract, CSV parser, startup reconciliation rules,
  Redis-backed startup persistence/import flow, runtime daily append, and
  enforced MR gate application exist. Operational promotion to `enforced`
  remains gated by shadow validation and review approval.

### Patch 4: Runtime Feed Guard

- Drop non-regular IMOEXF model bars before `update_day_aggregates()` and
  `orchestrator.on_bar()`.
- Log suppressed service bars at debug/info level with reason.
- Add tests that `08:50` and weekend bars do not update model state.
- Status: optional model-session guard is wired before model-state update;
  profile-specific acceptance tests still pending with live High180 integration.

### Patch 5: BO Gap-Flatten Decision

- If accepted, mark `bo_gap_flatten` as an accepted execution-contract class in
  docs and validation reports.
- Keep no-overnight behavior in live validation.

## Conditional Micro Soak Gate

Extended micro soak size `1` can be considered after:

- architecture review accepts the profile/mode boundary;
- risk-gate seed/state source is chosen;
- runtime model-feed guard is implemented;
- `bo_gap_flatten` is explicitly accepted or rejected;
- target account is flat;
- no working orders or stop orders remain;
- runtime starts from zero/clean validation state.

## Pre-Open From-Zero Rollout Checklist

Use this checklist only after review accepts the current conditional soak scope.

1. Confirm target account is flat.
2. Confirm broker has no working regular orders and no working stop orders.
3. Stop the previous IMOEXF runtime/gateway stack cleanly.
4. Archive only the minimal logs needed for review; do not keep stale Redis
   backups unless explicitly needed.
5. Clear runtime-owned state for the IMOEXF validation stack:
   runtime state stream, consumer group tail, pending command lifecycle, and
   risk-gate ledger/state keys for the selected profile.
6. Start with `risk_gate_mode = bootstrap_from_seed` and the checked-in
   `riskgate_high180_lb120_seed_2026-04-26.csv`.
   Use `configs/runtime.hybrid.live.7502SN6.riskgate-bootstrap.toml`.
   On VPS the seed CSV must be copied into the mounted config directory as
   `/configs/riskgate_high180_lb120_seed_2026-04-26.csv`; it is intentionally
   not read from `docs/` inside the runtime image.
7. Verify startup logs contain `risk_gate_startup_bootstrap` with seed import
   or existing-ledger decision, `state_refreshed = true`, and no identity
   mismatch.
8. Switch subsequent restarts to `risk_gate_mode = normal_append` once the
   ledger exists and seed bootstrap has succeeded.
   Use `configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml`.
9. Keep `risk_gate_mode = normal_append` for shadow validation. Move to
   `risk_gate_mode = enforced` only after the ledger/state events are clean and
   review explicitly accepts gate enforcement.
10. Keep live size at `1` during the first extended micro soak.
11. Monitor separately:
    MR High180 entries/exits, BO entries/exits, `bo_gap_flatten`, suppressed
    service bars, risk-gate startup decision, daily risk-gate append events,
    Redis memory, and live guard.

## Review Ask

Please review and decide:

1. Profile inside `hybrid_intraday` vs separate strategy kind.
2. High180 MR as explicit MR mode.
3. Risk-gate seed/state mechanism.
4. Runtime feed-guard location.
5. Whether `bo_gap_flatten` is accepted live contract.
6. Whether timer/event-loop flatten is required before first conditional soak or
   can stay follow-up.
