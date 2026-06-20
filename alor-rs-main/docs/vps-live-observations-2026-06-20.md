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

