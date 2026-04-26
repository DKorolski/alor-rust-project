# Service Bar Exclusion Backtest: IMOEXF Hybrid MR + BO

Date: 2026-04-26

## Question

Does excluding the non-tradable `08:50` service bar create a backtest
regression?

## Setup

Compared two feed contours:

```text
official_09 = 09:00..23:49
with_service_0850 = 08:50..23:49
```

The raw parquet contains `375` pre-`09:00` rows, all at `08:50`.

Model checked:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
scenario = base_realistic
```

Method:

- Recomputed `bo_new_k053` on both feed contours.
- Merged each BO result with the frozen MR riskgate source trades using the
  same no-overlap MR-priority merge.
- Compared BO standalone and hybrid metrics.

## Result Summary

Excluding the service bar does not create a regression. It slightly improves the
official backtest and prevents non-tradable `08:50` executions.

### Hybrid Result

```text
period              official_09   with_service_0850   official_delta
full                2688.05 pts   2650.41 pts         +37.64 pts
test_30              716.81 pts    697.81 pts         +19.00 pts
y2026_available      178.33 pts    174.83 pts          +3.50 pts
recent_forward_30d    68.11 pts     67.11 pts          +1.00 pts
```

Sharpe also remains slightly better under the official `09:00..23:49` contour:

```text
period              official_09   with_service_0850
full                3.49          3.45
test_30             3.54          3.46
y2026_available     4.26          4.25
recent_forward_30d  5.05          5.00
```

### BO Standalone Result

```text
period              official_09   with_service_0850   official_delta
full                1815.73 pts   1765.73 pts         +50.00 pts
test_30              577.71 pts    558.71 pts         +19.00 pts
y2026_available      104.53 pts    101.03 pts          +3.50 pts
recent_forward_30d    40.51 pts     39.51 pts          +1.00 pts
```

## Structural Finding

Including `08:50` creates illegal service-bar executions:

```text
pre-09 BO entries = 1
pre-09 BO exits   = 4
```

The pre-`09:00` BO entry is:

```text
2025-02-14 08:50:00 -> 2025-02-14 09:00:00
side = long
net_points = -1.030221
```

The service-bar contour also shifts many BO entries from `13:10` to `13:00`
because `bo_wait_hours = 4.0` starts from `08:50` instead of `09:00`.

## Verdict

`08:50` should be treated as a raw/audit-only service bar.

The frozen model feed should remain:

```text
regular weekdays only
09:00..23:49
exclude pre-09 service bars
exclude weekend sessions from trading/anchors
```

This exclusion is not a performance regression. It is both cleaner
operationally and slightly better in the replay metrics.
