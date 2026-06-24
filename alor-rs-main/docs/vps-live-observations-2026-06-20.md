# VPS Live Observations - 2026-06-20

Observation and configuration window: `07:45-07:52 MSK`.

## IMOEXF Quantity Increase

Both IMOEXF micro-live contours were increased from `2` to `3` contracts:

- primary hybrid / risk-gate shadow on `7502MIW`;
- Author41-short hybrid on `7502T0U`.

At an indicative initial margin of `5,000 RUB` per contract, the new configured
IMOEXF margin is about `15,000 RUB` per contour.

## Change Gate

Fresh broker snapshots confirmed before the change:

- `IMOEXF=0` on both portfolios;
- no working broker orders;
- no working stop orders;
- all runtime, gateway, and Redis containers healthy.

Backups:

```text
backup timestamp = 20260620-075141
```

The corresponding live config files in each stack were backed up before
replacement.

## Result

Only these runtime containers were recreated:

- `trading-hybrid-strategy-runtime-1`;
- `trading-hybrid-author41-7502t0u-strategy-runtime-1`.

Gateways and Redis instances were not restarted.

Post-restart verification:

- both runtime containers healthy;
- both configs resolved with `qty: 3.0`;
- bootstrap found `IMOEXF=0`;
- working strategy orders: `0`;
- working strategy stop orders: `0`;
- persisted strategy/risk-gate state restored;
- no startup `WARN`, `ERROR`, or panic.

Before the next tradable session both runtimes remained conservatively blocked
in `SyncingHistory` / `waiting_for_next_bar_after_restart`, which is expected.

The next validation checkpoint is the first completed IMOEXF cycle at quantity
`3`, with particular attention to MR bracket sibling cleanup and final
broker-flat reconciliation.
# MR Partial-Entry Hardening Rollout — 2026-06-22

- Source commit: `8c50aaf Harden MR bracket partial entry lifecycle`.
- Runtime image: `manual-20260622-mr-partial-8c50aaf`.
- Rollout window: `12:32-12:40 MSK`.
- Preflight broker snapshots confirmed:
  - `7502MIW / IMOEXF = 0`;
  - `7502T0U / IMOEXF = 0`;
  - `7502MIW / USDRUBF = 0`;
  - no working strategy TP or stop orders.
- Restarted only the three affected strategy runtimes:
  - IMOEXF riskgate-shadow on `7502MIW`;
  - IMOEXF Author41 short on `7502T0U`;
  - Alor-USDRUBF on `7502MIW`.
- Gateways and Redis were not restarted.
- Both IMOEXF configurations remain at `qty = 3`; loaded
  `partial_entry_fill_timeout_ms = 3000`.
- All three runtimes became healthy, consumed the `12:40 MSK` live bar, and
  transitioned from restart guard to `ALLOWED / LiveReady`.
- Post-rollout snapshots remained flat with no working TP/SL.
- VPS resources remained normal: about `6.6 GiB` memory available, root disk
  `27%` used, load average below `0.5`.
- VPS backup timestamp: `20260622-123219`.

## Post-Rollout Partial-Fill Observation

At `13:20 MSK`, both IMOEXF portfolios entered short quantity `3` through two
broker fills (`1 + 2`) separated by milliseconds. The full target was reached
in roughly `0.4-0.5 seconds`, below the configured `3000 ms` timeout. No
emergency flatten, residual, reject, or stale protection was observed. Later
full-size exits returned both portfolios to flat.

The entry was BO-owned, but the emitted diagnostic said `MR entry partially
filled`. This is added to the watchlist as a scope/logging follow-up:

- confirm whether partial-entry accumulation should apply only to bracket MR;
- keep BO/ordinary market semantics unchanged;
- retain the current observation as evidence of convergent partial-fill
  handling, not yet as MR bracket partial-fill acceptance.

The gateway also briefly entered disconnect/gap-sync around `15:35-15:40 MSK`.
The live guard blocked trading and returned to `ALLOWED / LiveReady`; no order
anomaly accompanied the recovery.

# Live Session Review — 2026-06-23

Review window: `2026-06-23 00:00-24:00 MSK`; journal written
`2026-06-24 07:42-07:55 MSK`.

## Health and guards

- All live runtime, gateway, and Redis containers were healthy at review time.
- VPS resources normal: about `6.6 GiB` memory available, root disk `28%` used,
  load average around `0.3`.
- Overnight all contours entered expected `BLOCKED` states during gateway
  reconnect / history or gap sync and then returned to `ALLOWED / LiveReady`
  around the morning session start.
- No `safe_mode`, emergency flatten, broker residual, orphan trade, or
  partial-entry timeout was observed in the reviewed window.

## Trades observed

### IMOEXF / 7502MIW / risk-gate shadow

- The contour remained risk-gate shadow only.
- The previous session was finalized at `09:10 MSK` with
  `shadow_pnl_points = 26.8` and `shadow_trade_count = 2`.
- No live broker fills for this contour were emitted by the risk-gate-shadow
  runtime during the reviewed day.

### IMOEXF / 7502T0U / Author41-short

- `09:30:00-09:30:01 MSK`: MR bracket short entry `qty = 3` filled as a real
  partial sequence `1 + 2` at `2300.0`.
- The partial-entry hardening behaved as intended: after the first fill the
  runtime logged `partial_entry_progress` and waited for full target `3` before
  bracket lifecycle progressed. No partial timeout or emergency flatten fired.
- `09:49:37 MSK`: TP buy `qty = 3` filled at `2278.5`; position returned flat.
- Sibling stop cleanup saw transient `Order to cancel not found` rejects, then
  retried and confirmed the strategy-owned stop order terminal/canceled.
- `10:00:01 MSK`: next MR short entry `qty = 3` filled at `2292.5`.
- `20:10:33 MSK`: exit buy `qty = 3` filled at `2337.5`; snapshot later showed
  `IMOEXF = 0` and the related TP/SL orders canceled/terminal.

### USDRUBF / 7502MIW

- `09:40:02 MSK`: MR short entry `qty = 1` filled at `75.10`.
- `09:57:08 MSK`: TP/exit buy `qty = 1` filled at `75.03`; flat.
- `11:40:05 MSK`: BO/day-breakout short entry `qty = 1` filled at `74.59`.
- `12:00:01 MSK`: exit buy `qty = 1` filled at `74.73`; flat.
- Review snapshot later showed `USDRUBF = 0` and no live residual position.

### RI / RTS-9.26 / 7502MIW and 7502T0U

- Both RI micro-live contours dropped the first `09:00 MSK` entry candidate by
  trading-window guard while still completing morning readiness.
- `09:20 MSK`: both portfolios emitted and filled short entry `qty = 1`.
  - `7502MIW`: sell at `94910`.
  - `7502T0U`: sell at `94890`.
- `09:50 MSK`: both portfolios exited with buy `qty = 1` at `93400`.
- Later snapshots showed `RTS-9.26 = 0` on both portfolios.

## Follow-ups

- Treat the `2026-06-23` IMOEXF Author41-short partial `1 + 2` as the first
  successful live acceptance sample for the MR bracket partial-entry patch.
- Keep sibling stop cleanup noise on the watchlist: the final state was safe
  (`canceled` confirmed), but the transient `Order to cancel not found` rejects
  are worth observing across the next MR bracket exits.
- Add RI morning-open freeze-contract behavior to the watchlist as P1 parity
  audit before RI scale-up: the first `09:00 MSK` candidate was dropped by
  `bar_silence` / trading-window guard after overnight sync, and the next
  scheduled `09:10 MSK` cycle appears to have emitted/executed around the
  `09:20 MSK` processing pass. Audit against `model_signal_ts`, scheduled time,
  command emission, and broker fill before classifying it as compliant.
