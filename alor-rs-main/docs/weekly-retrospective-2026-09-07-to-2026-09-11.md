# Weekly Retrospective: MOEX Live Micro And Shadow Contours

- Period: `2026-09-07 00:00 UTC` through `2026-09-11 Friday flat snapshot`
- VPS: `155.212.170.21`
- Scope: RI Author41/42 live07, Alor-USDRUBF live07, IMOEXF Hybrid BO-only live07, active early-session shadow contours
- Source: Docker runtime logs, `broker.orders.7502MIW`, shadow report files, Redis/Docker resource checks
- Status: `OBSERVATION_RECORDED`, `FRIDAY_FLAT_SNAPSHOT_CONFIRMED`

## Executive Read

The week ended operationally clean:

- all active live and shadow containers were `healthy`;
- no `WARN`, `ERROR`, `panic`, `orphan`, `residual`, rejected command, or stuck pending path was found in the last 7d live runtime grep;
- reconstructed live execution stream has no open position;
- latest broker position stream explicitly shows `RTS-9.26 qty=0` and `IMOEXF qty=0`; USDRUBF has no reconstructed open residual and runtime logged `open_to_flat` after the last exit;
- VPS resources remain comfortable: disk `/` about `28%` used, RAM about `5.9GiB` available;
- Redis memory is below pressure levels.

Main trading read:

- RI live07 was modestly positive: `8` reconstructed closed rounds, gross `+920` RI points, approx net `+1151 RUB`.
- USDRUBF live07 recovered versus the prior weak week: `9` reconstructed closed rounds, gross `+1.24` price points, approx net `+1113 RUB`.
- IMOEXF live BO-only was negative: `7` reconstructed closed partial-fill rounds, gross `-71.5` price-lot points, approx net `-787 RUB`.
- Combined engineering estimate for the three live contours: approx `+1476 RUB` net for the week before broker-statement reconciliation.

## Live Actual Rounds

Approximate economics are calculated from `execution_confirmed` runtime fills. Broker statement remains the accounting source of truth.

Conversion assumptions:

- RI: tick `10`, tick value `14.15804 RUB`.
- USDRUBF: `1.00` price point is about `1000 RUB`; table PnL already includes live qty.
- IMOEXF: tick `0.5`, tick value `5 RUB`, so `1.0` point is about `10 RUB`; table PnL already includes live qty.

| Strategy | Closed rounds | Gross model-price points | Approx net RUB | Open position |
|---|---:|---:|---:|---:|
| RI Author41/42 live07 | 8 | +920 RI points | +1151 | 0 |
| Alor-USDRUBF live07 | 9 | +1.24 price points | +1113 | 0 |
| IMOEXF Hybrid BO-only live07 | 7 partial-fill rounds | -71.5 price-lot points | -787 | 0 |

### RI Author41/42 Live07

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross points | Approx net RUB |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| 2026-09-07 | BO | Short | 11:10 | 14:10 | 1 | 82270 | 82650 | -380 | -557 |
| 2026-09-08 | MR | Short | 07:10 | 07:50 | 1 | 82880 | 82640 | +240 | +321 |
| 2026-09-08 | MR | Short | 08:00 | 08:10 | 1 | 82740 | 82610 | +130 | +165 |
| 2026-09-08 | BO | Long | 13:10 | 23:10 | 1 | 83490 | 83710 | +220 | +293 |
| 2026-09-09 | MR | Long | 07:10 | 10:40 | 1 | 83640 | 83740 | +100 | +122 |
| 2026-09-10 | MR | Short | 07:10 | 11:10 | 1 | 84160 | 85160 | -1000 | -1435 |
| 2026-09-10 | BO | Long | 12:10 | 23:10 | 1 | 85630 | 86560 | +930 | +1298 |
| 2026-09-11 | MR | Long | 07:10 | 13:30 | 1 | 86350 | 87030 | +680 | +943 |

Read:

- operationally clean, all commands used the expected action-scoped market path;
- one large MR stop on `2026-09-10` absorbed most of the week;
- BO remained the stronger contributor for the week;
- keep the RI MR after-BO/tight-stop watchlist item observational for now, not a production patch.

### Alor-USDRUBF Live07

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross price points | Approx net RUB |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| 2026-09-07 | BO | Long | 10:20 | 23:40 | 2 | 85.99 | 86.72 | +1.46 | +1444 |
| 2026-09-08 | BO | Long | 09:10 | 10:20 | 2 | 87.00 | 86.63 | -0.74 | -756 |
| 2026-09-08 | BO | Short | 13:30 | 15:00 | 2 | 86.34 | 86.41 | -0.14 | -156 |
| 2026-09-09 | BO | Long | 09:20 | 10:10 | 2 | 85.97 | 85.82 | -0.30 | -316 |
| 2026-09-09 | BO | Short | 11:20 | 12:00 | 2 | 85.29 | 85.38 | -0.18 | -196 |
| 2026-09-10 | BO | Short | 09:10 | 23:40 | 2 | 84.96 | 84.18 | +1.56 | +1544 |
| 2026-09-11 | MR | Short | 09:50 | 10:00 | 1 | 84.16 | 84.22 | -0.06 | -68 |
| 2026-09-11 | MR | Short | 09:50 | 10:00 | 1 | 84.16 | 84.22 | -0.06 | -68 |
| 2026-09-11 | BO | Long | 10:20 | 11:00 | 2 | 84.47 | 84.32 | -0.30 | -316 |

Read:

- week was slightly positive because the `2026-09-07` and `2026-09-10` BO EOD exits offset smaller stop-outs;
- rare MR bracket episode appeared on `2026-09-11`: entry short, TP/SL were installed, then canceled around `10:00` and the position exited by `mr_time_cutoff`;
- MR exit filled as two `1+1` partial fills under the same order id and returned flat without residual/orphan noise;
- keep USDRUBF at current risk until more post-bracket data accumulates.

### IMOEXF Hybrid Live07 BO-Only

Live remains BO-only; MR entries are still disabled in live and only observed via shadow.

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross price-lot points | Approx net RUB |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| 2026-09-07 | BO | Short | 10:10 | 14:00 | 5 | 2239.0 | 2258.0 | -95.0 | -965 |
| 2026-09-07 | BO | Short | 10:10 | 14:00 | 1 | 2239.0 | 2258.0 | -19.0 | -193 |
| 2026-09-08 | BO | Long | 14:00 | 17:00 | 1 | 2301.5 | 2288.0 | -13.5 | -138 |
| 2026-09-08 | BO | Long | 14:00 | 17:00 | 5 | 2301.5 | 2287.5 | -70.0 | -715 |
| 2026-09-10 | BO | Long | 15:20 | 23:40 | 6 | 2281.0 | 2312.5 | +189.0 | +1872 |
| 2026-09-11 | BO | Short | 13:50 | 18:00 | 1 | 2277.0 | 2287.5 | -10.5 | -108 |
| 2026-09-11 | BO | Short | 13:50 | 18:00 | 5 | 2277.0 | 2287.5 | -52.5 | -540 |

Read:

- partial fills on exits were normal same-order fills and did not create residual risk;
- the week was negative because `2026-09-07`, `2026-09-08`, and `2026-09-11` BO stopped out, while `2026-09-10` was a strong EOD-positive BO;
- keep MR disabled in live until the shadow evidence is strong enough.

## Shadow Contour Weekly Read

### RI Shadow

RI shadow stats are taken from decision JSONL, deduplicated by `decision_key`, using `shadow_path_active` over `shadow_recorded` when both exist.

| Contour | Trades | Total points | MR points | BO points | Read |
|---|---:|---:|---:|---:|---|
| canonical07 prospective | 8 | +1221.5 | -122.5 | +1344.0 | Best active family this week |
| canonical07 BO last_entry17 | 8 | +1221.5 | -122.5 | +1344.0 | Same as baseline this week |
| canonical07 BO max2 | 8 | +1221.5 | -122.5 | +1344.0 | Same as baseline this week |
| legacy09 | 8 | -143.5 | -1225.5 | +1082.0 | Weak MR dominated the week |

Read:

- live07/canonical07 remained materially better than legacy09 this week;
- `bo_last_entry17` and `bo_max2` did not change outcomes this week;
- the weak spot is still MR tail-risk, not BO.

### USDRUBF Shadow

| Contour | Weekly trades | Weekly PnL, price points | Wins/Losses | Read |
|---|---:|---:|---:|---|
| shadow07 | 8 | +1.281 | 2 / 6 | Close to live directionally |
| shadow09 | 7 | +0.760 | 3 / 3 | Weaker total |
| shadow07 mr1040_bo2 | 8 | +1.541 | 3 / 5 | Slightly stronger challenger |
| shadow07 mr1140_bo2 | 8 | +1.541 | 3 / 5 | Same as mr1040 this week |

Read:

- all USDRUBF shadow variants were positive this week;
- live actual `+1.24` price points is close to shadow07 `+1.281`, so this week does not show a large live-vs-shadow execution gap;
- still not enough to scale; keep collecting broker rounds.

### IMOEXF Shadow

Base early-session shadows:

| Contour | Weekly trades | Weekly PnL, price-lot points | Wins/Losses |
|---|---:|---:|---:|
| shadow07 | 9 | -36.0 | 4 / 5 |
| shadow09 | 8 | -88.5 | 3 / 5 |

Weekend-state candidate family:

| Contour | Trades | Total points | BO points | MR points | Read |
|---|---:|---:|---:|---:|---|
| weekend_state_live07_phase | 8 | +284.04 | +192.00 | +92.04 | Best balanced candidate this week |
| weekend_state_legacy09_author41 | 8 | +37.74 | +262.20 | -224.46 | BO good, MR weak |
| weekend_state_compromise_mr1059_bo4 | 10 | +24.36 | +190.20 | -165.84 | BO good, MR weak |

Read:

- this is the first week where the enriched shadow ledger gives useful MR/BO attribution;
- `weekend_state_live07_phase` is the cleanest candidate for further observation this week because both BO and MR were positive;
- do not promote yet: one week is small and there were `6` next-session `07xx` gap-labeled exits across the candidate family, which must remain interpretation-labeled before any live decision.

## Operations And Resources

VPS snapshot:

```text
Memory: 7.7Gi total, 1.9Gi used, about 5.9Gi available
Swap: 3.9Gi total, about 231Mi used
Disk /: 79G total, 21G used, 55G available, 28%
Docker images: 8.285GB total, 2.084GB reclaimable
```

Redis memory:

| Redis | Used memory | Peak | Maxmemory | Read |
|---|---:|---:|---:|---|
| RI live | 93.07M | 93.21M | 512M | OK |
| USDRUBF live | 103.56M | 103.75M | unlimited | OK |
| IMOEXF live | 93.04M | 93.18M | unlimited | OK |
| RI shadow | 25.63M | 33.03M | 384M | OK |
| USDRUBF shadow | 25.83M | 32.73M | 384M | OK |
| IMOEXF shadow | 29.86M | 34.28M | 384M | OK |

## Watchlist

- RI: keep observing MR downside after strong BO/uptrend regimes; `2026-09-10` MR short stop was the week’s largest single drag.
- RI: `bo_last_entry17` and `bo_max2` still need more data; this week they were identical to canonical07.
- USDRUBF: MR bracket lifecycle looked clean on `2026-09-11`, but sample remains tiny after bracket hardening.
- USDRUBF: `mr1040_bo2` / `mr1140_bo2` challengers were slightly stronger than current shadow07 this week; keep observing, no live change yet.
- IMOEXF: `weekend_state_live07_phase` improved in both BO and MR this week, but live MR remains disabled until more post-07:00 evidence is collected.
- IMOEXF: continue labeling next-session `07xx` shadow exits separately; do not treat them as desired live overnight behavior.
- Redis/Docker: no cleanup required now; disk and Redis have enough headroom.

## Verdict

`GO_CONTINUE_EXTENDED_MICRO_SOAK`.

No urgent patch or rollout blocker was found after Friday flat. Current live sizing can remain unchanged. For the next 1-2 weeks, the main analytical focus is:

- RI MR downside versus BO strength;
- USDRUBF post-bracket MR/BO execution quality;
- IMOEXF `weekend_state_live07_phase` shadow evidence, especially whether MR stays positive after more observations.
