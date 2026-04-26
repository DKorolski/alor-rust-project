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
- BO contour `bo_new_k053`:
  - `bo_k = 0.53`
  - `bo_stop1_range = 0.35`
  - `bo_stop2_range = 0.70`
  - `bo_wait_hours = 4.0`
  - `bo_min_range = 1.01`
- MR priority over BO while a position is open.
- Weekend sessions skipped under `baseline_skip`.
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

Observed Rust scaffold output:

```text
trades:                 1192
mean_reversion:          839
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

Current Rust scaffold:

```text
trades:           1192
mean_reversion:    839
BO:                353
```

## Current Interpretation

The BO guard behavior moved in the right direction:

- no weekend entries;
- no weekend exits;
- no late `09:50` stop-path carry after an overnight gap;
- `bo_gap_flatten_actions = 7` is explicit in the report.

The MR side is not parity-clean yet. The Rust internal shadow risk gate is too
permissive versus the source package and enables materially more MR trades.

## Remaining Work

Before runtime promotion, the next patch must align the MR gate with the source
package:

- identify the exact source gate semantics or import a frozen gate series;
- verify first enabled MR date and enabled-day set against the source package;
- reduce MR trade count drift before judging BO/MR interaction drift;
- only then write the final parity/discrepancy report.

Status:

```text
REPLAY_SCAFFOLD_READY / MR_RISKGATE_PARITY_PENDING
```
