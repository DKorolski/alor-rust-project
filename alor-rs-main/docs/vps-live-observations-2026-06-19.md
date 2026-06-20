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

## Session Result

Post-session review was completed on 2026-06-20.

All five active contours finished broker-flat:

- `7502MIW`: `USDRUBF=0`, `IMOEXF=0`, `RTS-9.26=0`;
- `7502T0U`: `IMOEXF=0`, `RTS-9.26=0`;
- latest broker snapshots contained no working stop orders.

### Alor-USDRUBF

One BO short cycle validated the timing rollout:

| Event | MSK time | Result |
| --- | --- | --- |
| completed signal bar | `13:30-13:40` | close/reference `73.34` |
| intent emitted | `13:40:00.593` | sell `1` |
| ACK accepted | `13:40:01.064` | about `471 ms` after intent |
| fill | `13:40:01.202` | sell `1` at `73.34` |
| exit intent | `15:00:03.411` | `bo_stop1_short` |
| exit fill | `15:00:04.080` | buy `1` at `73.35` |

Gross result was `-0.01` USDRUBF price units before commissions. Entry was
emitted on the model next-bar open instead of waiting for an additional
completed `10m` bar, so commit `a96a80a` behaved as intended.

The broker order records carried synthetic/reference prices `67.29` and
`79.43`, while the authoritative trade fills were `73.34` and `73.35`.
Runtime diagnostics explicitly distinguished these order-record prices from
execution prices; broker position accounting used the trade fills.

### RI Author41/42

Both micro contours executed the same two model cycles and finished flat.

`7502MIW`:

- short `1`: `102810 -> 102320`, gross `+490` points;
- long `1`: `102480 -> 102470`, gross `-10` points;
- session gross: `+480` points;
- four commissions of `9.98`, indicative net `+440.08`.

`7502T0U`:

- short `1`: `102800 -> 102290`, gross `+510` points;
- long `1`: `102470 -> 102490`, gross `+20` points;
- session gross: `+530` points;
- four commissions of `9.98`, indicative net `+490.08`.

All eight RI commands were accepted and filled. No reject, orphan-trade,
pending tail, or residual position was observed.

### IMOEXF

Neither the primary `7502MIW` hybrid contour nor the `7502T0U`
Author41-short contour emitted a new IMOEXF trade during the reviewed session.
Both remained broker-flat.

## Operational Read

- all 15 containers remained healthy;
- no runtime panic or terminal lifecycle error was found;
- no working strategy-owned order or stop tail remained after the session;
- server resources remained normal: about `6.6 GiB` RAM available, `27%`
  root-disk usage, and low load.
