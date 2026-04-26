# Replay Contract: IMOEXF Hybrid MR + BO

Date: 2026-04-26

## Purpose

This contract defines the parity target for moving the updated `IMOEXF` hybrid
MR+BO model from research into extended micro soak.

## Canonical Input

Use `10m` bars.

The reference runner loads:

```text
base.IMOEX_RAW
```

and filters to:

```text
09:00:00..23:49:00
```

Do not use raw `1m` live-like feed as the parity verdict for this package.

## Weekend Session Contract

Use `WEEKEND_SESSION_POLICY.md` as the canonical 2026 weekend/session policy.

Replay calendar rules:

- Keep weekend bars in the raw audit input if present.
- Exclude Saturday/Sunday sessions from signal generation, trade entry, and
  trade exit simulation.
- Use the most recent prior regular weekday session for all previous-session
  anchors.
- Monday anchors must come from Friday or the latest earlier regular weekday
  session if Friday is missing/holiday.
- Do not let weekend bars update MR anchors, BO anchors, or MR risk-gate
  decision dates.

## Canonical Output

The replay must emit:

```text
model_id
scenario
component_id
family
source
entry_ts
exit_ts
side
entry_price
exit_price
gross_points
cost_points
net_points
net_rub
```

The reference output is:

```text
replay_trades.csv
```

Daily reference:

```text
replay_daily.csv
```

Summary reference:

```text
replay_expected_summary.csv
```

## Primary Candidate To Check First

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

Primary scenario:

```text
base_realistic
```

Secondary scenario checks:

```text
stress_1tick
conservative_2tick
```

## Signal Contract

Signals are evaluated on close bars.

No intra-bar high/low trigger is part of this replay contract.

Hybrid merge:

- Sort candidate trades by entry timestamp.
- Give MR priority over BO at the same timestamp.
- Accept a candidate only if its entry timestamp is at or after the current
  accepted position's exit timestamp.
- This approximates one-position runtime behavior.

## BO Exit Contract

- `stop2` checks on every evaluated close bar.
- `stop1` checks only when `minute == 50`.
- EOD exit at `23:30`.
- BO must be flat before weekend/non-tradable gaps. If `force_exit_time = 23:30`,
  the Rust replay/runtime must assert that the position closes no later than the
  last regular weekday bar or next regular runtime bar/event.
- If the EOD exit signal is generated but there is no later same-day fill bar,
  Rust may flatten through its bar/event no-overnight guard instead of
  reproducing Backtrader's next-bar fill. Carrying the BO position to the next
  regular session is not equivalent to this frozen package.
- A stricter runtime timer/event-loop hook, able to flatten exactly at `23:30`
  without waiting for a later bar/event, is a follow-up `nice to have` after the
  main parity and rollout work.

## MR Exit Contract

- Target is the running daily midpoint.
- Stop is `7 * distance_to_midpoint`.
- Exit on close-bar target/stop crossing.
- Exit on max hold after `180` minutes.

## Tolerance

Strict parity target:

- Same trade count.
- Same owner/family sequence.
- Same side sequence.
- Same entry timestamps.
- Same exit timestamps.
- Same gross points.

Allowed review tolerance if Rust execution contract intentionally differs:

- Price difference no more than `1` tick per entry or exit.
- Daily PnL difference no more than the explained slippage/fee delta.
- Every timestamp mismatch must be attributable to one named contract difference,
  for example `eod minute`, `stop1 minute`, or `close-bar vs next-open fill`.

## Expected Baseline Numbers

From `research_report_source.md`, base scenario `test_30`:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053:
  total_points = 716.8
  Sharpe = 3.54
  max_drawdown_points = 60.0
  trades = 203
  MR/BO = 109/94

hybrid_mr_adaptive_lb120_mean__bo_new_k053:
  total_points = 728.3
  Sharpe = 3.68
  max_drawdown_points = 33.1
  trades = 183
  MR/BO = 89/94

baseline_runtime_hybrid:
  total_points = 569.4
  Sharpe = 2.66
  max_drawdown_points = 42.0
  trades = 151
  MR/BO = 69/82
```

Use the CSV files for exact values.

## Fail Conditions

Replay parity fails if:

- Rust replay uses `1m` bars for the parity verdict.
- MR or BO uses calendar weekend anchors instead of previous regular weekday
  anchors.
- Saturday/Sunday bars generate trades, exits, or MR risk-gate decision dates.
- BO carries across a weekend or non-tradable overnight gap, or the replay lacks
  an explicit gap-flatten assertion for the `23:30` force-exit path.
- MR risk gate uses same-day PnL or future data.
- BO uses old runtime `bo_k=0.65`, `stop1=0.51`, `stop2=0.35`, `wait=3.0`.
- Hybrid allows simultaneous MR and BO positions.
- Runtime changes are promoted before the replay discrepancy report is written.
