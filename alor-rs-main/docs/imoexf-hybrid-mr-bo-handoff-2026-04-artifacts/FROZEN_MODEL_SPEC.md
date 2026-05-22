# Frozen Model Spec: IMOEXF Hybrid MR + BO

Date: 2026-04-26

## Scope

Instrument: `IMOEXF`

Reference timeframe: `10m`

Signal and exit evaluation: close-bar based.

Hybrid position model: one position at a time. When MR and BO signals overlap,
MR has priority and BO is skipped until the MR position is closed.

Point value: `1 point = 10 RUB`.

Tick size: `0.5` point.

## Weekend Session Policy

Weekend/session handling is part of the frozen model contract and is specified
in `WEEKEND_SESSION_POLICY.md`.

Canonical rule:

- Keep weekend bars in the raw audit dataset.
- Do not generate signals or exits on Saturday/Sunday sessions.
- Compute all previous-session anchors from the most recent regular weekday
  session.
- Monday may trade, but Monday's anchor must be Friday or the latest earlier
  regular weekday session if Friday is missing/holiday.
- Weekend sessions must not become `previous_close`, `previous_high`,
  `previous_low`, or `previous_range` for either MR or BO.

## Recommended Runtime Candidate

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

This means:

- BO block: fixed `bo_new_k053`.
- MR block: high-K midpoint-take MR contour.
- MR risk gate: enable the MR contour only when its trailing 120-session shadow PnL is positive.

## BO Block

Parameter set:

```text
bo_k = 0.53
bo_wait_hours = 4.0
bo_min_range = 1.01
bo_min_range_mode = absolute
bo_big_move_threshold = 0.025
bo_stop1_range = 0.35
bo_stop2_range = 0.70
bo_eod_exit_time = 23:30
bo_exclude_weekends = true
```

Entry rules:

```text
long  if close > previous_close + bo_k * previous_day_range
short if close < previous_close - bo_k * previous_day_range
```

Additional BO filters:

- Check entries only after `bo_wait_hours` from the day start.
- Require `previous_day_range >= bo_min_range`.
- Use previous regular weekday close/range when weekend bars exist in the raw
  feed.
- If previous day return is less than `-bo_big_move_threshold`, suppress BO long.
- If previous day return is greater than `bo_big_move_threshold`, suppress BO short.
- Allow at most one BO long and one BO short per day.

BO exits:

- `stop1` is checked only on bars where minute is `:50`.
- `stop2` is checked on every evaluated close bar.
- Force exit at `23:30`.

Important interpretation:

- `bo_stop1_range = 0.35` is a wider/less restrictive stop1 setting versus the old `0.51`.
- `bo_stop2_range = 0.70` is a wider hard stop versus the old `0.35`.
- The recommended default is `bo_k = 0.53`; `bo_k = 0.59` remains a shadow recent-forward alternate.

## MR Block

Base contour:

```text
regular_weekday_anchor|broad_005_050|high180_kl085_ks090_sl7
```

Parameter set:

```text
anchor_policy = regular_weekday_anchor
range_gate_long = 0.005..0.050
range_gate_short = 0.005..0.050
k_long = 0.085
k_short = 0.090
stop_loss_mult = 7.0
max_hold_minutes = 180
entry_window = 09:00..11:59
trade_weekends = false
trade_mondays = true
```

MR anchor policy:

- Follow `WEEKEND_SESSION_POLICY.md`.
- Keep weekend bars in the raw audit dataset.
- Compute previous anchor from the previous regular weekday session.
- Do not trade weekends.
- Allow Monday trades using the previous regular weekday anchor.

MR entry rules:

```text
long if:
  previous_regular_range / close is in 0.005..0.050
  close < previous_regular_close
  close > previous_regular_close - k_long * previous_regular_range

short if:
  previous_regular_range / close is in 0.005..0.050
  close > previous_regular_close
  close < previous_regular_close + k_short * previous_regular_range
```

MR target and stop:

```text
running_midpoint = (running_day_high + running_day_low) / 2
take_profit = running_midpoint
stop_distance = stop_loss_mult * abs(take_profit - entry_price)
```

MR exits:

- Exit when close reaches/passes midpoint target.
- Exit when close reaches/passes stop.
- Exit after `max_hold_minutes = 180`.
- Exit at end of replay window if still open.

## MR Risk Gate

Primary gate:

```text
riskgate_high180_lb120
```

Gate rule:

```text
Enable tomorrow's MR contour only if trailing 120 regular-session shadow PnL
of the base contour is positive, using data available before tomorrow.
Otherwise go to cash for the MR block.
```

The shadow High180 MR contour must continue to run and update daily shadow PnL
on every regular session even when real MR is disabled by the gate. Otherwise
the gate can become a permanent off switch and will not match the frozen model
contract.

The BO block is not disabled by this MR gate.

## Cost Scenarios

Primary review scenario:

```text
base_realistic
```

Interpretation:

- MR: maker-like, broker `1 RUB` round-trip, no exchange fee, no slippage.
- BO: taker fee model from the research runner, no extra slippage.

Stress scenarios:

```text
stress_1tick
conservative_2tick
```

These add execution slippage and should remain part of replay/backtest reporting.

## Candidate Ranking

Primary:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

Shadow/canary:

```text
hybrid_mr_adaptive_lb120_mean__bo_new_k053
```

Research-only / not default:

```text
hybrid_mr_*__bo_new_k059
bo_new_k059
```

Rejected as default:

```text
baseline_runtime_hybrid
```

The baseline remains useful as a comparison anchor, not as the preferred updated
model.
