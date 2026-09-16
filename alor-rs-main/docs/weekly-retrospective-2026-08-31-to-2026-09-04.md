# Weekly Retrospective: MOEX Live Micro and Shadow Contours

- Period: `2026-08-31 00:00 MSK` through `2026-09-04 19:56 MSK`
- VPS: `155.212.170.21`
- Scope: RI Author41/42, Alor-USDRUBF Hybrid, IMOEXF Hybrid live micro and active shadow contours
- Source: Docker logs, Redis runtime snapshots, shadow report files, VPS resource checks
- Status: `OBSERVATION_RECORDED`, `FRIDAY_FLAT_SNAPSHOT_CONFIRMED`

## Executive Read

The week ended operationally clean:

- all active live and shadow containers were `healthy`;
- all three broker-facing live systems were flat by Redis runtime snapshots;
- no `panic`, OOM, orphan/residual/manual-close sequence, or stuck pending TP/SL/exit was found;
- VPS resources were comfortable: RAM about `6.0GiB` available, disk `/` about `25%` used;
- Redis memory stayed far below pressure levels.

Main trading read:

- RI Author41/42 live07 had a strong week: `6` closed rounds, gross `+4680` RI points, approx net `+6516 RUB`.
- USDRUBF live07 was weak/choppy: `8` closed rounds, gross `-1.96` price points, approx net `-2088 RUB`.
- IMOEXF Hybrid BO-only live07 had `2` closed BO rounds, gross `+234` price-lot points, approx net `+2306 RUB`.
- Portfolio-level engineering estimate for these three live contours: approx `+6734 RUB` net for the week before any broker-statement reconciliation differences.

## Live Actual Rounds

Approximate economics are calculated from `execution_confirmed` runtime fills. Broker statement remains the accounting source of truth.

Conversion assumptions:

- RI: tick `10`, tick value `14.15804 RUB`.
- USDRUBF: `1.00` price point is about `1000 RUB` per contract-equivalent; table PnL already includes live qty.
- IMOEXF: tick `0.5`, tick value `5 RUB`, so `1.0` point is about `10 RUB` per contract-equivalent; table PnL already includes live qty.

| Strategy | Closed rounds | Gross model-price points | Logged commission | Approx gross RUB | Approx net RUB | Open position |
|---|---:|---:|---:|---:|---:|---:|
| RI Author41/42 live07 | 6 | +4680 RI points | 109.28 RUB | +6625.16 | +6515.88 | 0 |
| Alor-USDRUBF live07 | 8 | -1.96 price points | 127.96 RUB | -1960.00 | -2087.96 | 0 |
| IMOEXF Hybrid BO-only live07 | 2 | +234.0 price-lot points | 33.96 RUB | +2340.00 | +2306.04 | 0 |

### RI Author41/42 Live07

All successful orders used action-scoped market execution. The system ended `phase=flat` with no pending entry/exit request.

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross points |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 2026-08-31 | BO | Long | 09:10 | 23:10 | 1 | 78170 | 80130 | +1960 |
| 2026-09-01 | MR | Long | 07:10 | 13:20 | 1 | 80150 | 80040 | -110 |
| 2026-09-03 | MR | Short | 07:50 | 11:20 | 1 | 79540 | 79240 | +300 |
| 2026-09-03 | BO | Long | 14:10 | 23:10 | 1 | 80490 | 82690 | +2200 |
| 2026-09-04 | MR | Long | 07:10 | 09:40 | 1 | 82490 | 82520 | +30 |
| 2026-09-04 | MR | Short | 09:50 | 10:10 | 1 | 82590 | 82290 | +300 |

Watch details:

- `2026-09-03 07:20 MSK`: first MR short entry was rejected by `cws_error`, message `action-scope cws open timeout`. Runtime logged `ri_live_entry_rejected_rolled_back`, broker qty stayed `0`, and a later MR short entered normally at `07:50 MSK`.
- `2026-09-04`: live executed a short MR round scheduled around `09:40 -> 10:00`; later the decision log also showed a `10:00 -> 11:30` long model decision without live entry. This is a journal/parity interpretation item, not a safety issue, because runtime state ended flat.

### Alor-USDRUBF Live07

The system ended `hybrid_state=flat`, `open_position_qty=0`, no pending request ids, and no tracked bracket orders.

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross price points |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 2026-08-31 | BO | Short | 09:10 | 10:00 | 2 | 86.29 | 86.44 | -0.30 |
| 2026-09-01 | BO | Short | 09:10 | 10:00 | 2 | 85.81 | 86.06 | -0.50 |
| 2026-09-01 | BO | Long | 10:10 | 23:40 | 2 | 86.36 | 86.73 | +0.74 |
| 2026-09-02 | BO | Short | 10:40 | 11:00 | 2 | 86.44 | 86.62 | -0.36 |
| 2026-09-02 | BO | Long | 14:10 | 15:00 | 2 | 87.12 | 87.13 | +0.02 |
| 2026-09-03 | BO | Short | 12:10 | 13:00 | 2 | 86.47 | 86.71 | -0.48 |
| 2026-09-04 | BO | Short | 10:10 | 10:40 | 2 | 86.12 | 86.58 | -0.92 |
| 2026-09-04 | BO | Long | 11:10 | 12:00 | 2 | 86.72 | 86.64 | -0.16 |

Read:

- the week was mostly BO churn, not MR bracket behavior;
- partial fills appeared on entries but completed to target qty and did not create residual/orphan incidents;
- no scale-up signal from this week.

### IMOEXF Hybrid Live07

Live remains BO-only for execution; MR entries are suppressed by `live_mr_entries_disabled`. The system ended with `last_position_qty=0`, no pending TP/SL/exit ids, and no deferred state.

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross price-lot points |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 2026-08-31 | BO | Long | 10:20 | 12:00 | 6 | 2140.5 | 2133.5 | -42.0 |
| 2026-09-03 | BO | Long | 13:50 | 23:40 | 6 | 2204.5 | 2250.5 | +276.0 |

Risk-gate ledger notes:

- `2026-08-31`: finalized `2026-08-28`, shadow PnL `+4.8`, rolling sum `60.3`.
- `2026-09-01`: finalized `2026-08-31`, shadow PnL `0.0`, rolling sum `64.6`.
- `2026-09-02`: finalized `2026-09-01`, shadow PnL `0.0`, rolling sum `61.4`.
- `2026-09-03`: finalized `2026-09-02`, shadow PnL `0.0`, rolling sum `61.4`.
- `2026-09-04`: finalized `2026-09-03`, shadow PnL `0.0`, rolling sum `72.0`.
- Current snapshot showed in-session `2026-09-04` risk-gate shadow PnL `+2.3`, `2` shadow trades, not finalized yet.

MR suppression observed on `2026-09-04`:

- High180 MR long candidates around `07:00`, `07:20`, `07:40`;
- High180 MR short candidates around `09:00`, `09:30`;
- all suppressed as expected by `live_mr_entries_disabled`.

## Shadow Contour Weekly Read

### RI Shadow

RI shadow stats are taken from runtime `ri_shadow_path_active` logs, deduplicated by `decision_key`.

| Contour | Active closed trades | Total shadow points | MR points | BO points | Read |
|---|---:|---:|---:|---:|---|
| canonical07 prospective | 6 | +4358 | +302 | +4056 | Strong and close to live directionally |
| legacy09 | 8 | +5284 | +1318 | +3966 | Stronger this week, mainly from additional MR candidates |
| canonical07 BO last_entry17 | 6 | +4358 | +302 | +4056 | Same as canonical07 this week |
| canonical07 BO max2 | 6 | +4358 | +302 | +4056 | Same as canonical07 this week |

Read:

- live07 remains healthy, but this specific week did not show canonical07 dominance over legacy09;
- `bo_last_entry17` and `bo_max2` did not change this week versus canonical07;
- keep RI on observation, no parameter change from one week of shadow advantage.

### USDRUBF Shadow

| Contour | Weekly trades | Weekly PnL, price points | Read |
|---|---:|---:|---|
| shadow07 | 8 | -1.60 | Weak/choppy, similar to live |
| shadow09 | 9 | -1.56 | Not materially better |
| shadow07 mr1040_bo2 | 9 | -1.36 | Slightly less negative |
| shadow07 mr1140_bo2 | 9 | -1.36 | Same as mr1040_bo2 this week |

Read:

- all USDRUBF shadow variants were negative this week;
- live actual `-1.96` was worse than shadow07 `-1.60`, consistent with execution timing/slippage;
- no evidence yet for increasing USDRUBF risk.

### IMOEXF Shadow

| Contour | Weekly trades | Weekly PnL, price-lot points | Read |
|---|---:|---:|---|
| shadow07 | 4 | +241.5 | Positive, partly gap/timestamp-sensitive |
| shadow09 | 9 | +546.0 | Stronger this week, more active |
| weekend-state live07_phase | 3 | +543.84 | Strongest clean low-trade candidate this week |
| weekend-state legacy09_author41 | 6 | +652.08 | Strongest total, but uses author41/legacy timing |
| weekend-state compromise_mr1059_bo4 | 3 | +501.84 | Strong but less than live07_phase |

Interpretation guard:

- some shadow exits are recorded at the next `07:00` eligible bar; this is a reporting/bar-driven gap nuance and should not be read as desired live overnight exposure;
- live IMOEXF BO exited at same-day live EOD/stop points and ended flat;
- MR remains disabled in live until we have a cleaner post-07:00 MR evidence package.

## Operations And Resources

VPS snapshot:

```text
Uptime: 16 days 13h
Load average: about 1.14 / 0.81 / 0.68
Memory: 7.7Gi total, 1.8Gi used, 6.0Gi available
Swap: 3.9Gi total, 253Mi used
Disk /: 79G total, 19G used, 57G available, 25%
Docker images: 6.697GB total, 2.07GB reclaimable
```

Redis memory:

| Redis | Used memory | Peak | Maxmemory | Read |
|---|---:|---:|---:|---|
| RI live | 61.32M | 61.49M | 512M | OK |
| USDRUBF live | 65.95M | 66.14M | unlimited | OK |
| IMOEXF live | 70.31M | 70.47M | unlimited | OK |
| RI shadow | 21.54M | 33.03M | 384M | OK |
| USDRUBF shadow | 21.66M | 32.73M | 384M | OK |
| IMOEXF shadow | 25.55M | 34.28M | 384M | OK |

## Watchlist

- RI: keep an eye on `action-scope cws open timeout`; one rejected entry was safely rolled back, but if it repeats near session start, consider action-scope open timeout/retry hardening.
- RI: compare live decision journal versus `shadow_path_active` for `2026-09-04` around `09:40..11:30`; this looks like an accounting/parity interpretation issue, not a live safety issue.
- RI: keep the Author41 MR after-BO tight-stop idea in watchlist only; source: `docs/ri-author41-mr-after-bo-tight-stop-watchlist-2026-09-04.md`.
- USDRUBF: weak BO week and execution/slippage gap versus shadow. Do not scale up; keep collecting broker rounds.
- IMOEXF: do not re-enable MR live yet. Continue shadow observation, especially `weekend_state_live07_phase` and `legacy09_author41`, but normalize/label next-session `07:00` gap exits before making production conclusions.
- Redis/Docker: no cleanup required now; Docker image reclaimable `2.07GB` is not urgent on a 57GB free disk.

## Verdict

`GO_CONTINUE_EXTENDED_MICRO_SOAK`.

No immediate rollout blocker was found after Friday close. Continue live micro observation with current sizing and keep the shadow family running. The main analytical focus for the next week is USDRUBF execution quality and IMOEXF MR shadow evidence under the post-07:00 regime.
