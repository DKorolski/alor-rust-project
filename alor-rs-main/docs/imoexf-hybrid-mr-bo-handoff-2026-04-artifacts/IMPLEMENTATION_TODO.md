# Implementation TODO: IMOEXF Hybrid MR + BO

Date: 2026-04-26

## Goal

Implement and verify a Rust-compatible replay/runtime path for the updated
`IMOEXF` hybrid package before extended micro soak.

## Required Work

1. Add a dedicated `10m` replay harness for `IMOEXF` hybrid MR+BO.

   Required behavior:

   - Load the same `10m` bars used by the reference runner.
   - Evaluate signals and exits on close bars.
   - Preserve one-position hybrid behavior.
   - Apply MR priority over BO on overlapping signals.
   - Emit trade journal with owner, side, entry timestamp, exit timestamp, entry
     price, exit price, gross points, cost points, and net points.

2. Implement the weekend/session policy from `WEEKEND_SESSION_POLICY.md`.

   Required behavior:

   - Keep weekend bars available in the raw audit dataset.
   - Build a regular weekday trading-calendar view for model evaluation.
   - Do not trade Saturday/Sunday sessions.
   - Do not use Saturday/Sunday sessions as previous anchors.
   - For Monday, use Friday or the latest earlier regular weekday as the
     previous-session anchor.
   - Apply this anchor rule consistently to MR, BO, and the MR risk gate.
   - Add a BO gap-flatten parity assert. If `force_exit_time = 23:30`, the
     position must be closed no later than the last regular weekday bar or
     next regular runtime bar/event.
   - If the EOD exit signal is generated and no later same-day fill bar exists,
     Rust may flatten through the runtime bar/event no-overnight guard rather
     than reproducing Backtrader's next-bar fill.
   - Nice to have after the main work: add a runtime timer/event-loop hook that
     can flatten exactly at `23:30` even without any later bar/event.

3. Implement BO parameter override set `bo_new_k053`.

   Required values:

   ```text
   bo_k = 0.53
   bo_wait_hours = 4.0
   bo_stop1_range = 0.35
   bo_stop2_range = 0.70
   bo_min_range = 1.01
   bo_min_range_mode = absolute
   bo_big_move_threshold = 0.025
   bo_eod_exit_time = 23:30
   bo_exclude_weekends = true
   ```

4. Implement or emulate the updated MR contour.

   Required values:

   ```text
   anchor_policy = regular_weekday_anchor
   k_long = 0.085
   k_short = 0.090
   range_gate = 0.005..0.050
   stop_loss_mult = 7.0
   max_hold_minutes = 180
   entry_window = 09:00..11:59
   trade_weekends = false
   trade_mondays = true
   ```

5. Implement `riskgate_high180_lb120`.

   Required behavior:

   - Keep a shadow PnL series for the fixed high180 MR contour.
   - For each regular weekday decision date, compute rolling 120-day shadow PnL
     shifted by one day.
   - Enable MR only when that shifted rolling PnL is positive.
   - Do not use the current day result to decide current day activation.
   - Do not create weekend decision dates from Saturday/Sunday bars.

6. Keep BO independent of the MR risk gate.

7. Add a parity test mode against this package's reference files.

   Reference files:

   - `replay_trades.csv`
   - `replay_daily.csv`
   - `replay_expected_summary.csv`

8. Add config or runtime profile only after replay parity is accepted.

## Acceptance Checklist

- Rust replay can reproduce the primary candidate's trades on `10m` bars.
- Entry timestamps match the reference journal.
- Exit timestamps match the reference journal or any difference is explained by
  an explicit execution-contract choice.
- Side and owner match for all trades.
- Gross points match exactly or within one tick where the fill contract differs.
- Daily net PnL matches within agreed tolerance.
- Summary metrics match the reference `base_realistic` table.
- Saturday/Sunday bars do not produce trades, anchors, or risk-gate decision
  dates.
- BO has zero weekend-crossing carry trades, and every `23:30` force-exit path
  is covered by an explicit gap-flatten assertion.
- Stress scenarios are generated and reported.

## Extended Micro Soak Gate

The package can move to extended micro soak only after:

- replay parity is complete,
- runtime config is frozen,
- Redis/runtime state handling is tested from clean start,
- existing hybrid CWS/protective-order fragility is confirmed fixed or explicitly
  bypassed for the selected execution contract.

## Operational Caution

The current research candidate is promising, but phase-sensitive. During soak,
monitor separately:

- MR contribution,
- BO contribution,
- recent-forward degradation,
- drawdown in points and RUB,
- skipped BO trades due to MR overlap,
- weekend/Monday anchor behavior.
