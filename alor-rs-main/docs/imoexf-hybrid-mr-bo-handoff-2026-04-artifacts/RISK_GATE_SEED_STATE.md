# Risk Gate Seed State: IMOEXF High180 LB120

Date: 2026-04-26

## Purpose

This note defines how to initialize and maintain the daily state for:

```text
riskgate_high180_lb120
```

The gate is an optional daily permission layer for the High180 MR contour. It
does not replace the MR signal logic. It only decides whether MR is allowed for
the next regular session.

The seed file is a bootstrap artifact, not a permanent source of truth. After
the initial import, the live runtime should maintain its own append-only
regular-session ledger and use that ledger for future gate decisions.

## Seed Files

Seed CSV:

```text
riskgate_high180_lb120_seed_2026-04-26.csv
```

Metadata:

```text
riskgate_high180_lb120_seed_2026-04-26_metadata.json
```

The seed was built from the available raw IMOEXF `10m` data using the frozen
model feed contract:

```text
Monday-Friday only
09:00..23:49
exclude pre-09 service bars such as 08:50
exclude Saturday/Sunday sessions from model state
regular weekday previous-session anchors
```

The local raw input used for this seed ends at:

```text
2026-04-21 23:40
```

Therefore, the seed includes the latest regular sessions available in the local
raw data, not the missing `2026-04-22..2026-04-24` sessions. `2026-04-25` and
`2026-04-26` are weekend dates and must not be added as gate rows.

## Seed Summary

```text
seed rows:                 180 regular sessions
seed first date:           2025-08-06
seed last date:            2026-04-21
lookback sessions:         120
minimum history sessions:   60
last-120 shadow PnL:       +161.9 points
next regular MR enabled:   true
```

## CSV Contract

Required columns:

```text
date
shadow_pnl_points
shadow_trade_count
rolling_120_pnl_before_session
mr_enabled_for_session
source
status
```

Column meanings:

- `date`: regular weekday session date.
- `shadow_pnl_points`: daily point PnL of the shadow High180 MR contour,
  attributed by exit date.
- `shadow_trade_count`: number of shadow High180 MR trades that exited on that
  date.
- `rolling_120_pnl_before_session`: sum of the prior 120 regular-session
  `shadow_pnl_points`, shifted by one session so the current date is not used
  for its own gate decision.
- `mr_enabled_for_session`: `true` when `rolling_120_pnl_before_session > 0`.
- `source`: `seed` for historical rows, runtime may use `runtime` for appended
  rows.
- `status`: `complete` for finished sessions.

## State Ownership Model

Use three separate layers:

```text
immutable seed artifact      -> one-time bootstrap / controlled rebuild input
runtime-owned session ledger -> long-lived source of truth for the gate
materialized fast state      -> quick startup/readiness cache
runtime snapshot             -> current intraday process state only
```

The seed must not be treated as a live config file. Runtime should not reread it
on every start to override the ledger. After the first import, gate decisions
come from the ledger.

Canonical Redis keys:

```text
runtime.riskgate.sessions.<strategy_id>.<profile_id>
runtime.riskgate.state.<strategy_id>.<profile_id>
runtime.riskgate.finalized.<strategy_id>.<profile_id>.<session_date>
```

Roles:

- `runtime.riskgate.sessions...` is an append-only Redis stream. One entry is
  one finalized regular weekday session.
- `runtime.riskgate.state...` is a small hash/materialized cache for startup,
  readiness, and diagnostics.
- `runtime.riskgate.finalized...` is a per-session idempotency guard. A session
  row is written only once.

The ledger stream is the canonical source of truth after seed import. The fast
state key is allowed to be rebuilt from the stream; it must not be the only
historical source.

Runtime modes:

```text
bootstrap_from_seed
normal_append
rebuild_from_history
```

- `bootstrap_from_seed`: allowed only when no runtime-owned ledger exists.
- `normal_append`: default live mode; append one finalized row per regular
  session.
- `rebuild_from_history`: explicit operator/service mode; rebuild ledger from
  canonical `10m` history and record the rebuild metadata.

The gate applies to the next regular session. The current session's shadow PnL
must not change the current session's MR permission.

## First Runtime Start

On first start:

1. Load the seed CSV.
2. Validate that rows are strictly increasing by `date`.
3. Validate that all rows are regular weekdays.
4. Validate that `status = complete` for historical rows.
5. Use only rows with `date < current_regular_session_date` for today's gate
   decision.
6. Sum the last `120` available regular-session `shadow_pnl_points`.
7. Enable MR for the current session only if the sum is positive.
8. Mark the seed as imported in the runtime-owned risk-gate ledger metadata.

If fewer than `60` complete regular sessions are available, runtime should keep
MR disabled or run the gate in `shadow_only` mode until enough history exists.

If a runtime-owned risk-gate ledger already exists, do not silently reload the
seed. Either refuse startup with an operator-visible error or require an explicit
rebuild/import command.

Startup reconciliation rules:

- If no ledger exists, `bootstrap_from_seed` may import the seed.
- If a ledger exists and `ledger_last_session_date >= seed_last_session_date`,
  ignore the seed and continue in `normal_append`.
- If a ledger exists and `ledger_last_session_date < seed_last_session_date`,
  refuse automatic startup unless an explicit import/rebuild mode is selected.
- If seed metadata does not match `symbol`, `profile_id`, `mr_variant`,
  `timeframe`, `lookback_sessions`, or session policy, refuse import.
- If the seed's latest available regular session is older than the latest
  required canonical `10m` history, warn and require either gap fill or explicit
  shadow-only/disabled decision.

Implementation note as of 2026-04-27:

```text
seed CSV parsing, regular-session validation, startup reconciliation decisions,
next-session gate calculation, runtime-row construction, Redis key naming,
ledger-stream field serialization/parsing, fast-state field
serialization/parsing, stream-to-state rebuild helpers, ledger-record identity
validation, and deterministic startup planning are implemented in
strategy-runtime/src/strategies/hybrid_intraday/risk_gate.rs.

The atomic Redis write skeleton lives in
strategy-runtime/src/redis_transport.rs:

SETNX-style finalized guard + XADD ledger stream + HSET materialized state.

The Redis transport also exposes field-preserving stream reads for the risk-gate
ledger. This is intentionally separate from the normal runtime-state `payload`
stream helpers, because risk-gate session rows are first-class stream fields and
not opaque JSON snapshots.

The runtime-facing store helper lives in
strategy-runtime/src/risk_gate_store.rs. It is the boundary between generic
Redis transport and risk-gate domain semantics:

- read ledger stream fields and parse them into `RiskGateLedgerRecord`;
- read materialized fast state from the state hash;
- load and validate the configured seed CSV;
- run the deterministic startup planner;
- apply startup artifacts by writing ledger records through the finalized guard
  and refreshing the fast state;
- validate profile identity before writing artifacts.
```

The live runtime still keeps active risk-gate modes fail-fast until the
runtime-owned startup/import/read/rebuild lifecycle is wired end-to-end. This is
intentional: the seed is parseable and reviewable, but it must not become a
hidden reread-on-restart source of truth.

Required metadata identity:

```text
symbol = IMOEXF
profile_id = imoexf_primary_riskgate_k053
mr_variant = high180
lookback_sessions = 120
min_history_sessions = 60
timeframe = 10m
session_policy = regular weekdays 09:00..23:49, no service/weekend bars
generation_date
source_artifact
script_version_or_commit
```

## Daily Update

After each completed regular weekday session:

1. Run the same shadow High180 MR contour on the session bars, even if real MR
   was disabled by the gate.
2. Append one daily row with the session's shadow PnL and shadow trade count.
3. Recompute `rolling_120_pnl_before_session` for the next session.
4. Persist the updated runtime-owned session ledger atomically.
5. Keep at least 180 regular-session rows for audit and recovery.

Do not append Saturday, Sunday, service-bar-only, or incomplete-session rows.

Session finalization can be bar/event-driven for the first soak:

- finalize on the last eligible `10m` regular-session bar, or
- finalize the previous regular session on the first eligible event of the next
  regular session if delayed finalization is needed.

Do not introduce a timer/event-loop finalizer as a prerequisite for the first
soak. A stricter timer hook can be added later if operations require exact
`23:30`/session-end behavior without waiting for a bar/event.

## Permanent-Off Prevention

The gate must not stop its own input stream. Real MR may be disabled, but the
shadow High180 MR contour must continue to run on every regular session and
append daily shadow PnL. This is what allows the gate to turn MR back on after a
bad phase.

Incorrect behavior:

```text
MR disabled by gate -> stop shadow MR updates
```

Required behavior:

```text
MR disabled by gate -> real MR stays flat, shadow MR keeps updating
```

If MR remains disabled for a long period, for example `60` regular sessions,
runtime should emit an operator-visible warning. That warning should not
automatically override the gate.

Recommended ledger fields:

```text
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
status
model_version
finalized_at_utc
```

Recommended materialized fast-state fields:

```text
last_finalized_session_date
rolling_sum_lb120
mr_enabled_current_session
mr_enabled_next_session
seed_loaded
ledger_rows_count
current_shadow_session_date
current_shadow_pnl_points
current_generation
```

Recommended main runtime snapshot fields:

```text
current_session_date
current_shadow_pnl_accumulator
current_shadow_trade_count
session_row_finalized
temporary intraday diagnostics
```

The snapshot should not store the whole 120-session history and should not be
the canonical gate source. Historical memory belongs to the session ledger.

## Canonical Feed Rule

The gate history layer is canonical `10m`.

Live execution may use the existing runtime/live event flow, but
`shadow_pnl_lb120` accounting should be derived from the same regular-session
`10m` layer used by seed, replay, and diagnostics.

Do not mix raw `1m` live bars into the risk-gate history layer without an
explicit conversion/aggregation contract and a new parity check. The current
frozen parity baseline is `10m`; recomputing the gate opportunistically from
`1m` would reopen model-definition drift.

## Rebuild / Recovery

Runtime may rebuild the seed from raw/audit bars if needed. The rebuild must use
the exact same model feed and High180 contour:

```text
k_long = 0.085
k_short = 0.090
range_gate = 0.005..0.050
stop_loss_mult = 7.0
max_hold_minutes = 180
entry_window = 09:00..11:59
cost_points = 0.1
```

The rebuilt daily state should match the seed rows over the same date range. If
new bars for `2026-04-22..2026-04-24` become available, regenerate or append
those regular-session rows before using the gate for later live decisions.

10m history gap handling:

- Missing regular-session bars make that session incomplete.
- Do not append an incomplete session as `complete`.
- If the missing data is recoverable, fill/rebuild from canonical `10m` history.
- If the gap is not recoverable before live start, choose explicitly between:
  `shadow_only`, `mr_disabled_until_history_ok`, or a documented operator
  override.
- Log the affected session dates and the selected policy.

## Operational Recommendation

For the first extended micro soak, use:

```text
risk_gate.mode = enforced
risk_gate.seed_file = riskgate_high180_lb120_seed_2026-04-26.csv
risk_gate.lookback_sessions = 120
risk_gate.min_history_sessions = 60
risk_gate.retention_sessions = 180
```

If the implementation is not ready to enforce the gate safely, use
`shadow_only` first, but record that this is not the full frozen primary
candidate.

Required startup log fields:

```text
seed_loaded = true
seed_rows = 180
seed_last_session = 2026-04-21
seed_profile_id = imoexf_primary_riskgate_k053
seed_timeframe = 10m
lookback_sessions = 120
min_history_sessions = 60
model_feed_contract = Mon-Fri 09:00..23:49
```
