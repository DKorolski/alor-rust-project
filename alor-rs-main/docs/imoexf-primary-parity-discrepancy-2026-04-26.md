# IMOEXF Primary Parity Discrepancy Note (2026-04-26)

## Scope

This note compares the Rust `imoexf_primary_riskgate_k053` replay scaffold with
the handoff source package primary candidate:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
scenario = base_realistic
```

The comparison uses the clarified frozen model feed contract:

```text
prepared/model feed = Monday-Friday regular tradable bars, 09:00..23:49
raw/audit feed      = may contain service bars such as 08:50
```

Service bars are raw/audit-only and must not update MR midpoint, BO
anchor/level, risk-gate shadow PnL, entry/exit logic, or replay parity state.

## Filtered Source Smoke

Temporary filtered source-prepared bundle:

```text
rows:            52572
date range:      2023-11-14 10:00 -> 2026-04-21 23:40
weekend rows:    0
pre-09 rows:     0
```

Rust replay command:

```bash
cargo run -p strategy-runtime --bin hybrid_replay -- \
  --bundle-dir /tmp/imoexf_source_filtered_model_bundle \
  --split hybrid \
  --out-dir /tmp/imoexf_primary_riskgate_filtered_bundle \
  --profile imoexf_primary_riskgate_k053 \
  --assert-gap-flatten
```

Layer-drift helper:

```bash
python3 scripts/imoexf_primary_parity_diff.py \
  --actual /tmp/imoexf_primary_riskgate_filtered_bundle/actual_trades_hybrid.csv \
  --reference /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/replay_trades.csv \
  --out-json /tmp/imoexf_primary_parity_diff.json
```

The helper normalizes Rust `pnl_comm` to per-contract `net_points` for samples
and keeps the full-position value separately as `actual_pnl_comm`, so review
examples stay comparable with the source reference point units.

MR residual diagnostic:

```bash
python3 scripts/imoexf_mr_residual_diagnostic.py \
  --actual /tmp/imoexf_primary_riskgate_filtered_bundle/actual_trades_hybrid.csv \
  --reference /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/replay_trades.csv \
  --source-mr /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/imoexf_mr_execution_economics_strategy_trades.csv \
  --raw /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_micro_live_audit_2026_04/cache/imoexf_2023-10-01_2026-04-21_raw.parquet \
  --out-json /tmp/imoexf_mr_residual_diagnostic.json
```

BO execution-contract diagnostic:

```bash
python3 scripts/imoexf_bo_execution_contract_diagnostic.py \
  --actual /tmp/imoexf_primary_riskgate_filtered_bundle/actual_trades_hybrid.csv \
  --reference /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/replay_trades.csv \
  --out-json /tmp/imoexf_bo_execution_contract_diagnostic.json
```

One-command review pipeline:

```bash
python3 scripts/run_imoexf_primary_parity_review.py \
  --raw /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_micro_live_audit_2026_04/cache/imoexf_2023-10-01_2026-04-21_raw.parquet \
  --reference /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/replay_trades.csv \
  --source-mr /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/imoexf_mr_execution_economics_strategy_trades.csv
```

Rust output:

```text
trades:                  846
mean_reversion:          471
intraday_breakout:       375
gap_flatten_violations:    0
bo_gap_flatten_actions:    7
```

Source primary reference:

```text
trades:                  844
mean_reversion:          474
intraday_breakout:       370
```

## Layer Drift

Exact trade key:

```text
family, entry_ts, exit_ts, side, entry_price, exit_price
```

MR:

```text
source MR:       474
Rust MR:         471
exact common:    464
source missing:   10
Rust extra:        7
```

BO:

```text
source BO:       370
Rust BO:         375
exact common:      0
source missing:  370
Rust extra:      375
```

The BO exact-match result is expected to be poor under the current comparison
key because this is mostly execution-contract drift, not signal drift.

## Signal Drift

MR is the cleaner signal-parity read:

- The risk gate now uses the source-like shifted rolling window over trading
  dates: `lookback = 120`, `min_history = 60`.
- The high180 midpoint guard now rejects entries where midpoint is already on
  the wrong side of entry close.
- Remaining MR drift is small relative to the previous overtrade and should be
  reviewed as residual parity cleanup.

Observed residual MR examples:

```text
source missing:
2024-02-16 09:00 -> 12:00 short 3269.0 -> 3273.0
2024-03-26 09:00 -> 10:50 short 3289.0 -> 3283.5
2025-04-03 09:20 -> 11:00 long 2956.0 -> 2968.0
2025-10-13 10:50 -> 13:50 long 2575.0 -> 2568.5

Rust extra:
2024-02-16 09:00 -> 09:10 short 3269.0 -> 3268.0
2024-03-26 09:10 -> 10:50 short 3289.0 -> 3283.5
2025-04-03 09:20 -> 09:50 long 2956.0 -> 2965.0
2025-09-17 09:00 -> 09:20 long 2809.0 -> 2797.0
```

Interpretation: MR residuals look like local timestamp/exit-contract differences
and possible BO/MR overlap sequencing effects, not a broad signal definition
failure.

## MR Residual Root Cause Read

The apparent MR residual against the saved source `replay_trades.csv` is mostly
explained by stale source-reference semantics rather than by a Rust model bug.

Three classes were isolated:

- Old source MR trades were generated before the service-bar exclusion contract.
  Example: on `2024-02-16 09:00`, the saved source MR target is `3267.25`,
  which equals the midpoint when the `08:50` service bar is included. The
  filtered model feed midpoint is `3268.0`, so Rust exits at `09:10` instead of
  carrying the trade to the `12:00` time stop.
- Old source riskgate selection used a rolling window over `raw_dates` that
  included weekend zero-PnL dates. The filtered Rust contract uses regular
  weekday dates only. This explains the main autumn flips:
  `2025-09-17` is Rust-only, while `2025-10-13`, `2025-10-16`,
  `2025-10-20`, `2025-10-22`, and `2025-10-31` are source-only under the old
  calendar-zero gate.
- After recomputing a filtered canonical standalone MR using only regular
  weekday `09:00..23:49` bars and weekday rolling riskgate, MR parity tightens
  to:

```text
filtered canonical MR: 471
Rust actual MR:        471
exact common:          470
missing / extra:       1 / 1
```

The only remaining standalone-MR-vs-Rust difference is:

```text
filtered canonical: 2024-03-26 09:00 -> 10:50 short 3289.0 -> 3283.5
Rust actual:        2024-03-26 09:10 -> 10:50 short 3289.0 -> 3283.5
```

That final shift is caused by BO execution-contract interaction: Rust carries a
prior BO into a first-event `bo_gap_flatten` exit at `2024-03-26 09:00`, so the
MR entry can only happen on the next bar. This belongs to the same accepted/needs
decision BO gap-flatten class, not to MR signal logic.

The diagnostic script classifies the saved-source MR mismatch as:

```text
saved source missing:
  stale_service_bar_midpoint: 4
  calendar_zero_riskgate:     5
  bo_gap_flatten_interaction: 1

saved source actual-extra:
  source_hybrid_merge_bo_overlap: 1

filtered canonical residual:
  bo_gap_flatten_interaction: 1
```

## Execution-Contract Drift

BO drift is dominated by systematic fill semantics:

- Source/backtrader reference often records next-bar-style entries and `23:40`
  late exits.
- Rust close-bar replay currently records same-bar close entries and `23:30`
  no-overnight exits.
- Rust also enforces gap flatten for BO instead of accepting Backtrader
  cross-day carry as desired runtime behavior.

Example:

```text
source BO: 2023-11-16 17:50 -> 23:40 short 3198.5 -> 3173.5
Rust BO:   2023-11-16 17:40 -> 23:30 short 3194.5 -> 3173.5
```

This is an execution-contract discrepancy. It should not be counted as a model
signal failure unless the final contract requires Backtrader next-bar fills.

The BO diagnostic makes this mechanical:

```text
fill-level exact:
  source BO:       370
  Rust BO:         375
  exact common:      0

after source next-bar timestamp normalization (-10m):
  entry-signal common:       361 / 370
  entry+exit-signal common:  350 / 370
  date+side count diffs:       7

cross-day / gap class:
  source cross-day reference carry: 9
  Rust cross-day gap-flatten:       7
```

Interpretation: exact BO parity is intentionally a poor metric while comparing
Backtrader next-bar fills with Rust close-bar event-loop actions. The remaining
`9` missing / `14` extra entry-signal rows after timestamp normalization should
be reviewed as hybrid interaction or true signal drift only after the team
decides whether Rust no-overnight `bo_gap_flatten` is the accepted production
contract.

## BO Gap/Carry Class

Source BO cross-day reference carries:

```text
source BO cross-day: 9
Rust BO cross-day:   7
```

Rust cross-day rows are first-event `bo_gap_flatten` exits, not accepted
overnight/weekend holds. These should be reported as:

```text
bo_cross_day_reference_carry
bo_gap_flatten
```

The preferred runtime/barter behavior remains:

```text
No BO carry through non-tradable gaps.
Flatten at the last same-day tradable event/timer where possible.
If no same-day timer/event exists in replay, flatten at the first next tradable
event and label it explicitly as bo_gap_flatten.
```

## Acceptance Status

Current status:

```text
SIGNAL_NEAR_PARITY / EXECUTION_CONTRACT_DRIFT_EXPLICIT
```

Not accepted yet for production promotion because:

- the checked-in official replay bundle still needs to be regenerated through
  `2026-04-21` with the filtered model feed contract;
- one final parity report must show MR signal drift, BO signal drift, and BO
  execution-contract drift as separate counters;
- the team still needs an explicit decision that Rust `bo_gap_flatten` is an
  accepted runtime contract, not a parity failure.

## Next Step

Use the helper above, or promote the same logic into the Rust parity report path,
so review always sees separate counters for:

- `signal_drift_mr`
- `signal_drift_bo`
- `execution_contract_drift_bo_next_bar`
- `execution_contract_drift_bo_gap_flatten`
- `bo_cross_day_reference_carry`

Only after that split should the team decide whether to tune Rust replay toward
Backtrader parity mode or keep the current runtime-safe close-bar/no-overnight
contract as the canonical live behavior.

## Promotion Gate

Before IMOEXF hybrid can move to extended micro soak:

- rebuild the official filtered bundle through `2026-04-21`: Monday-Friday
  `09:00..23:49`, no `08:50` service bars in model state;
- run the final `imoexf_primary_riskgate_k053` replay on that bundle;
- publish one final parity report with separate MR signal, BO signal, and BO
  execution-contract counters;
- record `bo_gap_flatten` as accepted runtime behavior, if the team agrees with
  the no-overnight live contract;
- start any extended micro soak at live size `1`, with explicit MR/BO
  attribution monitoring.
