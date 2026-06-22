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
