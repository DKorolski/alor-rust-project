# RI Author41 MR After-BO Tight Stop Watchlist

- Date: `2026-09-04`
- Status: `WATCHLIST`, `SHADOW_ONLY`, `NO_LIVE_CHANGE`
- Scope: RI Author41/42 `live07` MR risk-control challenger
- Source analysis: `analiz_alpha_si/ri_mr_after_bo_tight_stop_2026_09_04/ri_mr_after_bo_tight_stop_note.md`

## Idea

Do not disable Author41 MR after a BO or BO-like risk day. Instead, test a
narrow challenger that tightens MR stops only on explicitly defined risk days.

Baseline Author41 MR stops:

- short stop: `0.75 * prev_range`
- long stop: `1.00 * prev_range`

Medium challenger stops:

- short stop: `0.45 * prev_range`
- long stop: `0.65 * prev_range`

Candidate gates from the analysis:

- previous actual Author42 BO day
- Author42-like close break plus previous range `>= 3%`
- previous wide range `>= 3%`

## Evidence Summary

Long legacy sample:

- Baseline: `2213` trades, `+148481.50` points, Sharpe `1.40`, MaxDD
  `29480.00`, worst day `-8254.00`.
- `a42_close_wide3_mid_s045_l065`: `2215` trades, `+149681.00` points,
  Sharpe `1.48`, MaxDD `22416.50`, worst day `-5792.00`.
- `actual_bo_prev_mid_s045_l065`: `2221` trades, `+131774.50` points,
  Sharpe `1.29`, MaxDD `31042.00`, worst day unchanged at `-8254.00`.
- `wide3_only_mid_s045_l065`: improves MaxDD and worst day, but is too broad
  as a production semantic change without a dedicated shadow line.

Post-transition `live07` sample:

- Baseline: `44` trades, `+9997.00` points, Sharpe `8.39`, MaxDD `1131.00`,
  worst day `-1127.00`.
- `actual_bo_prev_mid_s045_l065`: `44` trades, `+9608.50` points, Sharpe
  `8.43`, MaxDD `657.00`, worst day `-653.00`.
- Other tight-stop gates either do not improve the sample or reduce total PnL
  too much for a live decision.

## Current Read

The analysis supports a watchlist item, not a production default.

Tightening stops after BO-like days is less destructive than hard-disabling MR,
but the evidence is mixed:

- the full legacy sample does not show a uniformly clean improvement for the
  narrow actual-BO gate;
- the post-`2026-07-14` `live07` sample is encouraging for drawdown control,
  but too small;
- the broad wide-range gate is a different strategy behavior and should not be
  promoted without isolated shadow observation.

## Recommended Shadow Candidate

If we implement a shadow contour, prefer the narrowest live-explainable version:

```text
author41_mr_after_bo_tight_stop_actual_bo_prev_mid_s045_l065
```

Shadow-only contract:

- keep Author42 BO unchanged;
- keep Author41 MR entry logic unchanged;
- change only Author41 MR stop multiplier on eligible risk days;
- do not alter take, breakeven, time-exit, no-overlap or action-scoped
  execution path;
- journal every day when the tight-stop gate is active.

## Acceptance Before Promotion

No live promotion before at least `20-30` risk sessions are observed.

Promotion can be discussed only if the shadow contour:

- materially reduces MR tail loss or active drawdown;
- does not materially degrade total RI combo PnL;
- preserves the current `live07` no-overlap contract;
- does not create timing drift versus the baseline RI decision journal;
- explains whether the improvement comes from actual BO-day context or from a
  broader volatility regime filter.

## Decision

Keep current RI `live07` production unchanged.

Add this line to the RI watchlist as a potential shadow risk-control challenger.
Do not deploy it into live micro until the evidence is stronger and the contour
has been observed side-by-side with the current baseline.
