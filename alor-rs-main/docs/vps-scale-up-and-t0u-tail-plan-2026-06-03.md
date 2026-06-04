# VPS Scale-Up And 7502T0U Tail Plan - 2026-06-03

Status timestamp: 2026-06-03 08:18-08:25 MSK.

## Pre-Open Check

Active VPS containers are healthy:

- `trading-ri-author41-42-7502miw-*`
- `trading-alor-usdrubf-*`
- `trading-hybrid-*`
- `trading-hybrid-author41-7502t0u-*`

Resource snapshot:

- `trading-ri-author41-42-7502miw-redis-1`: about `202M / 768M`.
- `trading-alor-usdrubf-redis-1`: about `234M / 1G`.
- `trading-hybrid-redis-1`: about `222M / 1G`.
- `trading-hybrid-author41-7502t0u-redis-1`: was at `508-512M / 512M` before safe trim.

`7502MIW` broker snapshot:

- `RTS-6.26 qty = 0`.
- `USDRUBF qty = 0`.
- `IMOEXF qty = 0`.
- `RUB cash ~= 151073.74`.

`7502T0U` broker snapshot:

- `USDRUBF qty = 0`.
- `RTS-6.26 qty = 0`.
- `IMOEXF qty = +1`, avg `2597.5`.
- This is a trial `hybrid-author41` IMOEXF tail, not the earlier corporate USDRUBF tail.

## Immediate Maintenance Done

Safe Redis trim was performed on `trading-hybrid-author41-7502t0u-redis-1`.

Trimmed streams:

- `events.health.hybrid_author41_short.7502T0U` to about `2000` entries.
- `broker.snapshots.7502T0U` to about `2000` entries.
- `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U` to `200` entries.
- `broker.positions.7502T0U` to about `5000` entries.

Memory result:

- Before: about `512M / 512M`.
- After: about `57-90M / 512M` during immediate post-trim checks.

No broad Redis flush was used.

## 7502T0U Tail Diagnosis

Observed sequence:

- `2026-06-02 12:10 MSK`: trial `hybrid-author41` emitted `IntradayBreakout Long`.
- Fill: buy `1` IMOEXF at `2597.5`.
- Around `2026-06-02 19:36 MSK`: runtime blocked due `gateway_health_stale`.
- Redis was later found full / near full, and gateway health stream was very large.
- After the safe trim, runtime processed the delayed `2026-06-02 23:30 MSK` bar and generated `BreakoutEodExit`.
- The EOD exit was emitted on `2026-06-03 08:21 MSK`, while gateway scheduler was `OutsideSession`.
- Gateway rejected it as `trading_window_closed`.
- Runtime correctly moved it to deferred exit:
  - `deferred_exit_owner = intraday_breakout`.
  - `deferred_exit_reason = breakout_eod_exit`.
  - `deferred_exit_cycle_id = 6a1e9de805`.

Working root cause:

- The `7502T0U` trial contour accumulated excessive Redis health/snapshot state.
- That likely contributed to stale gateway health / delayed EOD processing.
- The strategy did generate the correct BO EOD exit, but too late for the exchange session, so it now waits as deferred exit.

Operational action:

- At the next tradable open, confirm the deferred exit is reissued and closes `IMOEXF +1`.
- If it does not close promptly after open, close manually and stop/restart the trial contour from clean runtime state.

## Scale-Up Plan

### Stage 0 - Today / Before Any Scale-Up

Do not increase size while `7502T0U` has an unresolved IMOEXF tail.

Required first:

- Close or confirm closure of `7502T0U` `IMOEXF +1`.
- Confirm `7502MIW` remains flat before changing configs.
- Keep current production sizes:
  - `RI = 1`.
  - `Alor-USDRUBF = 1`.
  - `IMOEXF hybrid = 2`.

### Stage 1 - RI Increase Candidate

Preferred first scale-up: `RI 1 -> 2`.

Reason:

- RI has the strongest observed live micro economics.
- After `2026-05-22`, runtime-log read showed continued positive closed-cycle contribution.
- RI has fewer active engineering watchlist items than IMOEXF hybrid.

Conditions before rollout:

- Additional capital received and visible in broker cash.
- `7502MIW` broker flat for `RTS-6.26`.
- No active RI pending command/ack state.
- Redis/resources normal.
- No new RI-specific WARN/ERROR beyond known fill-before-ack ordering noise.

Capital note:

- Current `7502MIW` cash around `151k` is likely too tight for comfortable `RI=2` combined with `USDRUBF=1` and `IMOEXF=2`.
- Wait for the planned capital top-up before enabling `RI=2`.

Rollout shape:

- Change only RI quantity from `1` to `2`.
- Do not change RI execution contract.
- Keep action-scoped-only path.
- Clear only RI runtime state / command / ack streams if doing a controlled from-zero runtime restart.
- Keep RI bars/history streams.
- Observe `5-10` clean trading sessions before any further RI increase.

### Stage 2 - IMOEXF Hybrid Increase Candidate

Candidate: `IMOEXF hybrid 2 -> 4`, but not before RI step and watchlist stabilization.

Reasons to delay:

- Existing watchlist includes:
  - partial-fill / cleanup idempotency at qty `2`;
  - near-zero MR bracket churn;
  - protective TP action-scoped `open_timeout`;
  - weekend-gap BO attribution;
  - current `7502T0U` trial BO overnight tail.
- Increasing `IMOEXF` from `2` to `4` doubles the operational visibility and potential cleanup noise of the same edge cases.

Conditions before rollout:

- At least several clean sessions after the current watchlist items, especially no repeated stale stop orders and no repeated TP open-timeout repair flatten.
- Broker-ledger reconciliation supports the economics after `2026-05-22`.
- `7502T0U` trial contour either fixed/stopped or clearly isolated from production decisions.

Recommended near-term decision:

- Keep `IMOEXF hybrid = 2` for now.
- Revisit `2 -> 4` only after RI `1 -> 2` has at least several clean sessions and the `7502T0U` tail root cause is resolved.

### Stage 3 - Alor-USDRUBF

Keep `Alor-USDRUBF = 1` for now.

Reason:

- It is useful as a diversifying component, but recent realized contribution is weaker than RI.
- Do not increase it together with RI until the new margin/capital envelope is confirmed.

## Watchlist / Engineering Items

Immediate:

- Confirm `7502T0U` deferred BO exit closes the `IMOEXF +1` tail at the next tradable open.
- Reduce or automate retention for `hybrid-author41 7502T0U`, especially:
  - `events.health.hybrid_author41_short.7502T0U`;
  - `broker.snapshots.7502T0U`;
  - `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U`.

Near-term:

- Add/verify safe trim job coverage for trial contours.
- Consider lowering runtime trim values for the `7502T0U` trial:
  - `positions` from `20000` to a smaller value;
  - `runtime_state` from `500` to `200`;
  - keep `health` small.
- Investigate whether gateway snapshot stream retention needs its own trim path, because `broker.snapshots.7502T0U` was one of the largest streams.

Scale-up:

- RI first, then IMOEXF.
- Do not jump directly to research proportions such as `RI/USDRUBF/IMOEXF = 2/2/12`; live micro strength and operational maturity are not equal across systems.

## Execution Update - 2026-06-03 08:32-08:35 MSK

User-approved operational decision: proceed with immediate controlled size increase on `7502MIW`, despite the earlier conservative staging recommendation.

Applied local and VPS config changes:

- `RI Author41/42 7502MIW`: `qty 1.0 -> 2.0`.
- `IMOEXF hybrid riskgate 7502MIW`: `qty 2.0 -> 4.0`.
- `hybrid-author41 7502T0U`: quantity intentionally unchanged at `qty 1.0`.

VPS rollout:

- Recreated only `strategy-runtime` in `/opt/trading-ri-author41-42-7502miw`.
- Recreated only `strategy-runtime` in `/opt/trading-hybrid`.
- Did not restart `trading-hybrid-author41-7502t0u`, because it had an open/deferred `IMOEXF +1` tail.

Post-change verification:

- RI runtime config log confirms `strategy_kind = RiAuthor4142`, `portfolio = 7502MIW`, `qty = 2.0`, `execution_path = action_scoped_only`.
- IMOEXF hybrid runtime config log confirms `portfolio = 7502MIW`, `profile = imoexf_primary_riskgate_high180_lb120`, `qty = 4.0`.
- IMOEXF riskgate loaded existing Redis ledger: `decision = UseExistingLedger`, `ledger_rows_count = 203`, `mr_enabled_current_session = true`, `rolling_sum_lb120 ~= 164.6`.
- Both restarted runtimes were healthy after recreate.
- Pre-open guard state was `waiting_for_next_bar_after_restart`, expected before the next live bar.

Maintenance update:

- `7502T0U` trial Redis was trimmed again without service stop.
- Memory moved from about `84M / 512M` to about `45M / 512M`.
- Stream lengths after trim:
  - `events.health.hybrid_author41_short.7502T0U = 2000`.
  - `broker.snapshots.7502T0U = 2000`.
  - `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U = 200`.
  - `broker.positions.7502T0U = 5000`.

Operational risk note:

- With `RI=2`, `IMOEXF=4`, and `Alor-USDRUBF=1`, the `7502MIW` margin buffer is tighter than before.
- Watch broker rejects for `insufficient_funds` / margin, partial fills on `IMOEXF`, and any action-scoped CWS open-timeout regressions.
