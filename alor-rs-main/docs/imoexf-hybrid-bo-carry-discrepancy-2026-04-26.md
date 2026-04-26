# IMOEXF Hybrid BO Carry Discrepancy Note (2026-04-26)

## Scope

This note records a known reference-package discrepancy before implementing the
updated `IMOEXF` hybrid primary candidate in Rust:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

The issue is not a model-selection blocker, but it is a replay/parity contract
edge case that must be handled explicitly.

## Finding

The source `replay_trades.csv` contains BO trades whose entry and exit dates do
not match. These are consistent with the documented Backtrader next-bar fill
semantics, but they are not acceptable as live/runtime carry semantics for the
Rust implementation.

For `base_realistic` primary candidate:

```text
all trades:        844
BO trades:         370
MR trades:         474
cross-day BO:        9
cross-day MR:        0
weekend entries:     0
weekend exits:       0
max BO duration:    64.0 hours
```

The same `bo_new_k053` BO component is shared by the primary and
shadow/adaptive candidates, so the cross-day BO count is also `9` for:

```text
hybrid_mr_adaptive_lb120_mean__bo_new_k053
```

## Representative Rows

```text
entry_ts             exit_ts              side   gross_points  net_points
2023-11-17 17:00:00  2023-11-20 09:00:00 long          15.0     14.477402
2023-11-27 16:40:00  2023-11-28 10:00:00 short          6.5      5.978029
2023-12-04 19:20:00  2023-12-05 10:00:00 short         -5.0     -5.510916
2023-12-21 14:30:00  2023-12-22 10:00:00 short         -3.5     -4.007187
2024-01-24 13:10:00  2024-01-25 10:00:00 short          1.5      0.981791
2024-02-06 18:40:00  2024-02-07 09:00:00 long           1.5      0.971957
2024-02-29 13:40:00  2024-03-01 09:00:00 long          18.5     17.970439
2024-03-14 14:00:00  2024-03-15 09:00:00 short          7.5      6.964037
2026-02-23 18:40:00  2026-02-24 10:00:00 long          -0.5     -0.968973
```

The largest case is a Friday-to-Monday BO carry:

```text
2023-11-17 17:00:00 -> 2023-11-20 09:00:00
```

## Interpretation

This is a reference replay artifact, not a desired live behavior.

The Rust replay/runtime contract remains:

- no weekend trading;
- no weekend fills;
- no BO overnight carry as an accepted live semantic;
- `force_exit_time = 23:30` must be protected by a gap-flatten assertion;
- if a later same-day fill bar is missing, Rust may flatten via the
  no-overnight bar/event guard instead of reproducing Backtrader's next-bar
  carry.

## Validation Implication

Strict row-for-row parity against the source `replay_trades.csv` is expected to
fail for these cross-day BO rows once Rust enforces the no-overnight contract.

The patched replay comparison should therefore classify these rows as an
explained discrepancy class:

```text
bo_cross_day_reference_carry
```

Acceptance should require:

- the discrepancy count is bounded and explicitly reported;
- every affected trade is BO-owned;
- no MR trade is affected;
- no Saturday/Sunday entry or exit is introduced by Rust;
- the daily/summary PnL delta caused by earlier flattening is reported
  separately from signal-generation drift.

## Current Status

Rust replay already has:

- `hybrid_replay --assert-gap-flatten`;
- an e2e test proving next-day EOD fills fail under the gap-flatten assertion;
- a `baseline_skip` guard preventing pending fills on weekend bars.

Remaining work:

- implement the primary candidate replay path;
- compare against the reference package while recognizing
  `bo_cross_day_reference_carry` as an explained discrepancy;
- write the final parity/discrepancy report before any runtime promotion.
