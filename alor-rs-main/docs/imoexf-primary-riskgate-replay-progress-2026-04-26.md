# IMOEXF Primary Riskgate Replay Progress (2026-04-26)

## Scope

This note records the first Rust replay implementation slice for:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

The implementation is available as:

```text
hybrid_replay --profile imoexf_primary_riskgate_k053
```

This is a replay scaffold, not a final parity acceptance.

## Implemented

- Close-bar MR contour:
  - `k_long = 0.085`
  - `k_short = 0.090`
  - relative previous range gate `0.005..0.050`
  - running intraday midpoint target
  - stop distance `7 * abs(target - entry)`
  - max hold `180` minutes
- Internal shadow replay for `riskgate_high180_lb120`.
  - Uses source-like shifted rolling over the prepared trading-date index:
    `lookback = 120` sessions, `min_history = 60` sessions.
  - Uses maker-like shadow cost `0.1` point per MR trade, matching
    `maker_broker_1rub_rt`.
- MR midpoint guard:
  - long entries require `running_midpoint > entry_close`;
  - short entries require `running_midpoint < entry_close`.
- BO contour `bo_new_k053`:
  - `bo_k = 0.53`
  - `bo_stop1_range = 0.35`
  - `bo_stop2_range = 0.70`
  - `bo_wait_hours = 4.0`
  - `bo_min_range = 1.01`
- MR priority over BO while a position is open.
- Model-state feed guard:
  - only Monday-Friday `09:00..23:49` bars can update MR, BO, and risk-gate
    state;
  - raw/audit service bars such as `08:50` are ignored by the replay model
    path.
- BO first-event `bo_gap_flatten` guard when no same-day `23:30` exit is
  available.
- Parity report field:
  - `bo_gap_flatten_actions`

## Full Prepared-Hybrid Smoke

Command:

```bash
cargo run -p strategy-runtime --bin hybrid_replay -- \
  --bundle-dir /Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/pre_rust_handoff/replay_data/imoexf_2023_2026 \
  --split hybrid \
  --out-dir /tmp/imoexf_primary_riskgate_k053 \
  --profile imoexf_primary_riskgate_k053 \
  --assert-gap-flatten
```

Observed Rust scaffold output on the existing `alor_project/pre_rust_handoff`
bundle:

```text
trades:                  805
mean_reversion:          452
intraday_breakout:       353
bo_gap_flatten_actions:    7
weekend entries:           0
weekend exits:             0
gap_flatten_violations:    0
```

The seven BO cross-day timestamps are now first-event `09:00` guard exits, not
late stop-path exits:

```text
2023-11-27 16:30 -> 2023-11-28 09:00
2023-12-04 19:10 -> 2023-12-05 09:00
2023-12-21 14:20 -> 2023-12-22 09:00
2024-01-24 13:00 -> 2024-01-25 09:00
2024-03-25 23:40 -> 2024-03-26 09:00
2025-02-13 23:40 -> 2025-02-14 09:00
2026-02-23 18:30 -> 2026-02-24 09:00
```

## Reference Comparison

Reference source package for `base_realistic` primary candidate:

```text
trades:           844
mean_reversion:   474
BO:               370
```

Current Rust scaffold on the existing `alor_project/pre_rust_handoff` bundle:

```text
trades:            805
mean_reversion:    452
BO:                353
```

The existing bundle is not the same data contour as the source package:

- It ends at `2026-03-03`, while the source handoff runs through
  `2026-04-21`.
- It still contains weekend bars, while the accepted prepared parity/model feed
  should contain only Monday-Friday regular tradable bars.

## Source Session Contour Clarification

The raw source parquet contains `08:50` rows on some days (`375` rows in the
full raw file). These rows are raw feed artifacts, not part of the frozen
handoff replay session.

The canonical source runner uses:

```text
base.filter_session(base.load_raw_10m(base.IMOEX_RAW), "09:00:00", "23:49:00")
```

which maps to:

```text
between_time("09:00", "23:49")
```

Therefore, the official Rust parity bundle should exclude pre-`09:00` rows.
Pre-`09:00` bars must not update MR running midpoint, BO state, risk-gate shadow
state, or signal/execution state unless a separate new contract is explicitly
approved.

Using a temporary filtered source-prepared bundle generated from the handoff raw
data with Monday-Friday `09:00..23:49` only:

```bash
python3 scripts/build_imoexf_filtered_bundle.py \
  --raw /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_micro_live_audit_2026_04/cache/imoexf_2023-10-01_2026-04-21_raw.parquet \
  --out-dir /tmp/imoexf_filtered_bundle_scripted
```

```text
prepared rows:       52572
date range:          2023-11-14 10:00 -> 2026-04-21 23:40
weekend rows:            0
pre-09:00 rows:          0

Rust trades:           846
Rust mean_reversion:   471
Rust BO:               375
MR exact matches:      464 / 474 source riskgate MR trades
MR extra/missing:        7 / 10
gap_flatten_violations:  0
bo_gap_flatten_actions:  7
```

Generated metadata:

```text
weekend_rows:           0
pre_session_rows:       0
post_session_rows:      0
non_monotonic_rows:     0
regular_model_contract: true
```

## Current Interpretation

The BO guard behavior moved in the right direction:

- no weekend entries;
- no weekend exits;
- no late `09:50` stop-path carry after an overnight gap;
- `bo_gap_flatten_actions = 7` is explicit in the report.

Detailed drift analysis is recorded in:

```text
docs/imoexf-primary-parity-discrepancy-2026-04-26.md
docs/imoexf-primary-parity-review-report-2026-04-26.md
```

The MR side is much closer on the temporary filtered source contour, but
official parity must be rechecked after rebuilding the checked-in
`09:00..23:49` source bundle. The large earlier overtrade was caused by two
separate Rust-side alignment issues that are now fixed:

- risk gate semantics used a date-span sum without source `min_periods = 60`
  behavior;
- high180 entries did not reject cases where the running midpoint was already
  on the wrong side of the entry close.

The later `10` missing / `7` extra MR drift against the saved source reference
has been isolated as stale-reference drift:

- saved source MR trades still include pre-`09:00` service-bar effects in some
  running midpoint targets;
- saved source riskgate selection used weekend zero-PnL dates in the rolling
  gate;
- under a filtered weekday-only canonical MR recomputation, standalone MR tightens
  to `471` vs `471` trades with `470` exact matches. The final `1` / `1` shift is
  caused by a Rust BO `bo_gap_flatten` occupying `2024-03-26 09:00`.

This is now reproducible with:

```text
scripts/imoexf_mr_residual_diagnostic.py
```

Current root-cause counters:

```text
stale_service_bar_midpoint: 4
calendar_zero_riskgate:     5
bo_gap_flatten_interaction: 1
```

## Acceptance Checks

- Prepared parity/model feed contains only Monday-Friday regular tradable bars:
  `09:00..23:49`.
- Pre-`09:00` service bars remain raw/audit-only and do not update model state.
- MR entries are limited to the morning entry window; MR exits may continue
  until `max_hold_minutes = 180`.
- MR and BO cannot be open at the same time; MR keeps priority.
- BO does not use Saturday/Sunday anchors.
- BO does not carry through non-tradable gaps in Rust/barter replay.
- Backtrader BO carry differences are reported separately as
  `bo_cross_day_reference_carry` / `bo_gap_flatten`.
- Final parity report must split signal drift from execution-contract drift.

## Remaining Work

Before runtime promotion, the next patch must refresh the official Rust replay
data contour and make the execution-contract decision explicit:

- regenerate/replace the official `prepared_hybrid` data from the source
  handoff contour, including data through `2026-04-21` but excluding weekends
  and pre-`09:00` service bars;
- compare full hybrid output against `replay_trades.csv`, with MR and BO
  attribution separated;
- regenerate or annotate the source reference because the saved source
  `replay_trades.csv` still carries old service-bar and calendar-zero riskgate
  semantics;
- use `scripts/imoexf_bo_execution_contract_diagnostic.py` before treating BO
  exact drift as signal drift. Current source `-10m` next-bar normalization gives
  `361 / 370` BO entry-signal matches despite `0` fill-level exact matches;
- decide whether the `7` BO `bo_gap_flatten` cases are accepted runtime
  contract differences versus the backtrader reference carry;
- only then write the final parity/discrepancy report.

Status:

```text
SIGNAL_NEAR_PARITY / EXECUTION_CONTRACT_DRIFT_EXPLICIT
```
