# VPS Live Observations - 2026-06-19

Observation and rollout window: `07:41-07:48 MSK`.

## Pre-Rollout Gate

The planned Alor-USDRUBF timing rollout was performed only after fresh broker
truth confirmed:

- `USDRUBF` position: `0`;
- working strategy orders: `0`;
- working stop orders: `0`;
- runtime, gateway, and Redis containers: healthy.

Overnight CWS/market-data reconnects recovered automatically. No command was in
flight during those reconnects.

## Runtime Change

Source commit:

```text
a96a80a Align USDRUBF live entry with next-bar model timing
```

The change removes the additional completed-bar wait from live Alor-USDRUBF
entries. A signal generated from completed bar `N` now emits the live market
intent immediately, matching the model's next-bar-open execution contract.

Runtime image:

```text
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260619-usdrubf-timing-a96a80a
```

The image was built for `linux/amd64`. GHCR rejected the local push because the
local registry authorization had expired, so the same built image was
transferred directly to the VPS and loaded into Docker under the unique tag.

## Rollout Result

Only the `trading-alor-usdrubf` strategy-runtime container was recreated.
Gateway and Redis were not restarted.

The previous environment file was saved as:

```text
/opt/trading-alor-usdrubf/.env.bak.20260619-074634
```

Post-rollout checks:

- strategy-runtime healthy;
- gateway healthy;
- Redis healthy;
- expected runtime image active;
- bootstrap reconciled `USDRUBF=0`;
- restored pending requests: `0`;
- working strategy orders: `0`;
- working stop orders: `0`;
- no startup `WARN`, `ERROR`, or panic.

Before the regular session the runtime remained conservatively blocked in
`SyncingHistory` and `waiting_for_next_bar_after_restart`. This is expected:
trading becomes eligible only after gateway synchronization and the first new
live `10m` bar.

