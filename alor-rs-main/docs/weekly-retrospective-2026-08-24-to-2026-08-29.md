# Weekly Retrospective: MOEX Live Micro and Shadow Contours

- Period: `2026-08-24 00:00 MSK` through `2026-08-29 06:42 MSK`
- VPS: `155.212.170.21`
- Scope: RI Author41/42, Alor-USDRUBF Hybrid, IMOEXF Hybrid live micro and active shadow contours
- Source: Docker logs, Redis streams/state snapshots, strategy report files, broker terminal ledger extract
- Status: `OBSERVATION_RECORDED`, `BROKER_LEDGER_RECONCILED`

## Executive Read

The week closed operationally clean:

- all active containers were `healthy`;
- live broker-facing strategies were flat by the latest Redis/runtime snapshot;
- no live runtime panic/error/orphan/residual/manual-intervention pattern was found;
- live command groups had `pending=0`;
- resource usage was comfortable: RAM available about `6.2GiB`, disk `/` about `25%`;
- Redis memory was well below pressure levels.

Main observations:

- RI live07 had the strongest live week by execution logs: `8` closed rounds, all action-scoped market path, no rejects.
- USDRUBF live07 had one recoverable `action-scope cws open timeout` on `2026-08-28 18:20 MSK`; runtime deferred and reissued on the next bar at `18:30 MSK`, then closed flat.
- IMOEXF live remains BO-only. MR signals were suppressed by `live_mr_entries_disabled`; risk-gate shadow ledger continued to update.
- Shadow comparisons continue to favor early/canonical variants for RI and selected IMOEXF weekend-state variants, but IMOEXF shadow CSVs still need careful interpretation around next-session/gap timestamps.
- Broker terminal ledger for `2026-08-24..2026-08-28` reconciles with the live actual rounds below.

## Live Actual Rounds

Approximate economics below are calculated from `execution_confirmed` log fills, not from broker statement. They are useful for engineering retrospective, while broker ledger remains the accounting source of truth.

Broker ledger reconciliation:

- RI ledger matches the runtime-derived `8` closed rounds and `+2520` gross RI points.
- USDRUBF ledger matches the runtime-derived `8` closed rounds and `-0.84` gross price points after grouping partial fills into one logical round.
- IMOEXF ledger matches the runtime-derived `2` closed BO rounds and `-9.0` gross price-lot points after grouping the `2026-08-26 23:40` partial cover `1 + 5` into one qty `6` exit.
- `2026-08-28` RI has two separate logical rounds: long `07:20 -> 07:50` and short `08:00 -> 11:30`; this is not a broker duplicate.
- `2026-08-28` USDRUBF `18:30` has two qty `1` buy fills at the same price; they reconcile to one qty `2` long entry that exited at `19:00`.

| Strategy | Closed rounds | Gross model-price points | Logged commission | Approx gross RUB | Approx net RUB | Open position |
|---|---:|---:|---:|---:|---:|---:|
| RI Author41/42 live07 | 8 | +2520 RI points | 139.80 RUB | +3567.83 | +3428.03 | 0 |
| Alor-USDRUBF live07 | 8 | -0.84 price points | 124.60 RUB | -840.00 | -964.60 | 0 |
| IMOEXF Hybrid BO-only live07 | 2 | -9.0 price-lot points | 33.60 RUB | -90.00 | -123.60 | 0 |

Conversion assumptions:

- RI: tick `10`, tick value `14.15804 RUB`.
- USDRUBF: `1.00` price point equals about `1000 RUB` per contract-equivalent; table value already includes live qty from fills.
- IMOEXF: tick `0.5`, tick value `5 RUB`, so `1.0` index point is about `10 RUB` per contract-equivalent; table value already includes live qty from fills.

### RI Author41/42 Live07

All fills used action-scoped market execution. No command rejects, no partial-entry accumulation, no orphan/residual/manual sequence.

| Date | Round | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross points |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 2026-08-24 | BO | Short | 10:10 | 23:10 | 1 | 78570 | 78080 | +490 |
| 2026-08-25 | MR | Short | 07:10 | 10:40 | 1 | 78230 | 77690 | +540 |
| 2026-08-26 | BO | Short | 15:10 | 17:10 | 1 | 77610 | 77750 | -140 |
| 2026-08-26 | BO | Short | 19:10 | 23:10 | 1 | 76920 | 76280 | +640 |
| 2026-08-27 | MR | Short | 07:10 | 09:00 | 1 | 76450 | 76120 | +330 |
| 2026-08-27 | MR | Short | 09:10 | 10:20 | 1 | 76460 | 76100 | +360 |
| 2026-08-28 | MR | Long | 07:20 | 07:50 | 1 | 77180 | 77280 | +100 |
| 2026-08-28 | MR | Short | 08:00 | 11:30 | 1 | 77380 | 77180 | +200 |

Watch detail:

- `2026-08-28 07:50` short was emitted/executed as live-prospective MR entry and later exited at `11:30`. The active-only RI shadow journal did not count the same round as an active closed decision in the simple weekly aggregation. This is not an execution incident, but should be kept as a journal/parity interpretation item when comparing live vs shadow.

### Alor-USDRUBF Live07

The system ended flat with no residual/orphan state. MR bracket was observed on `2026-08-24`; most other rounds were BO.

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross price points |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 2026-08-24 | MR | Short | 08:40 | 09:52 | 2 | 83.44 | 83.35 | +0.18 |
| 2026-08-24 | BO | Long | 10:00 | 11:00 | 2 | 83.41 | 83.25 | -0.32 |
| 2026-08-25 | BO | Long | 10:20 | 17:40 | 2 | 84.61 | 84.08 | -1.06 |
| 2026-08-26 | BO | Long | 15:50 | 17:00 | 2 | 84.70 | 84.60 | -0.20 |
| 2026-08-27 | BO | Short | 09:10 | 10:00 | 2 | 85.15 | 85.22 | -0.14 |
| 2026-08-27 | BO | Long | 10:40 | 23:40 | 2 | 85.63 | 86.03 | +0.80 |
| 2026-08-28 | BO | Short | 09:20 | 10:00 | 2 | 85.84 | 85.92 | -0.16 |
| 2026-08-28 | BO | Long | 18:30 | 19:00 | 2 | 86.82 | 86.85 | +0.06 |

Operational notes:

- `2026-08-28 18:20 MSK`: entry command was rejected with `cws_error`, message `action-scope cws open timeout`.
- The strategy logged `entry_reject_deferred_next_bar`.
- `2026-08-28 18:30 MSK`: retry entered successfully, partial fills `1 + 1` accumulated to target qty `2`.
- No broker residual emergency exit or orphan trade followed.

### IMOEXF Hybrid Live07

Live config remains BO-only for execution; MR entries are suppressed. The week had two BO short rounds.

| Date | Component | Side | Entry MSK | Exit MSK | Qty | Entry | Exit | Gross price-lot points |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 2026-08-24 | BO | Short | 10:10 | 23:40 | 6 | 2073.0 | 2078.0 | -30.0 |
| 2026-08-26 | BO | Short | 19:00 | 23:40 | 6 | 2062.5 | 2059.0 | +21.0 |

Risk-gate notes:

- `2026-08-24`: finalized `2026-08-21`, shadow PnL `+0.9`, ledger rows `256`.
- `2026-08-25`: finalized `2026-08-24`, shadow PnL `0.0`, ledger rows `257`.
- `2026-08-26`: finalized `2026-08-25`, shadow PnL `0.0`, ledger rows `257`.
- `2026-08-27`: finalized `2026-08-26`, shadow PnL `0.0`, ledger rows `257`.
- `2026-08-28`: finalized `2026-08-27`, shadow PnL `+7.4`, rolling sum `56.4`, ledger rows `257`.
- Current in-session shadow state for `2026-08-28`: shadow PnL `+4.8`, shadow trades `2`, not yet finalized at the time of snapshot.

Suppressed MR events:

- `2026-08-27`: multiple High180 MR entries suppressed by `live_mr_entries_disabled`.
- `2026-08-28`: one MR entry suppressed by `live_mr_entries_disabled`; one candidate suppressed by `take_too_close_after_rounding`.

## Shadow Contour Weekly Read

### RI

RI shadow active-only aggregation uses the last `shadow_path_active` state per `decision_key`, excluding `live_prospective` duplicates.

| Contour | Active closed trades | Total shadow points | MR points | BO points | Read |
|---|---:|---:|---:|---:|---|
| canonical07 prospective | 7 | +2416 | +1362 | +1054 | Strongest baseline-style weekly shadow read |
| legacy09 | 8 | -1646 | -1394 | -252 | Weak this week, mainly due one large MR time-exit loss |
| canonical07 BO last_entry17 | 6 | +2088 | +1362 | +726 | Reduces late BO exposure but gave up one profitable BO this week |
| canonical07 BO max2 | 7 | +2416 | +1362 | +1054 | Same weekly total as baseline canonical07 this week |

Read:

- canonical07 remains materially better than legacy09 this week;
- BO risk filters did not clearly improve the week versus baseline canonical07;
- continue observation before changing live BO risk filter.

### USDRUBF

| Contour | Weekly trades | Weekly PnL, price points | Read |
|---|---:|---:|---|
| shadow07 | 8 | -0.835 | Slightly negative |
| shadow09 | 9 | -0.460 | Less negative than shadow07 this week |
| shadow07 mr1040_bo2 | 7 | -0.675 | No improvement this week |
| shadow07 mr1140_bo2 | 7 | -0.675 | Same as mr1040_bo2 this week |

Read:

- USDRUBF remained choppy during the week;
- live and shadow both show BO activity, but live actual ended slightly worse than shadow07 due execution timing and broker path;
- no scale-up signal from this week alone.

### IMOEXF

| Contour | Weekly trades | Weekly PnL, price-lot points | Read |
|---|---:|---:|---|
| shadow07 | 4 | +43.5 | Positive, modest |
| shadow09 | 5 | +114.0 | Stronger than baseline shadow07 this week |
| weekend-state live07_phase | 9 | +170.82 | Strongest weekly shadow read |
| weekend-state legacy09_author41 | 9 | +12.60 | Positive but weaker, hit by 2026-08-27 loss |
| weekend-state compromise_mr1059_bo4 | 12 | +85.50 | Positive, more active but noisier |

Read:

- the `weekend_state_live07_phase` contour is the best weekly IMOEXF shadow candidate;
- live BO-only was slightly negative this week;
- do not promote MR back to live yet; keep shadow observation and separate MR/BO attribution.

Important interpretation guard:

- Some IMOEXF shadow CSV rows show exits at the next `07:00` eligible bar. This is a shadow/reporting or bar-driven gap-flatten nuance, not evidence of live overnight exposure. Live IMOEXF exited at `23:40` in the observed BO rounds.

### IMOEXF Weekend-State Live07 Phase Since 2026-07-14

This slice combines:

- research audit package `analiz_alpha_si/moex_post_0714_regime_audit_2026_08_24/` for `2026-07-14..2026-08-23`;
- VPS shadow CSV from `trading-moex-early-shadow-imoexf-runtime-weekend-state-live07-phase-1` for `2026-08-24..2026-08-28`.

Contract:

- strategy: `author41_short_hybrid`;
- policy: `imo_live07_phase_mr0959_bo3`;
- weekend policy: `weekend_state_trade_weekdays`;
- live/shadow config: `weekend_state_policy=state_only`, weekday entries only, `MR` window until `09:59`, `BO` wait `3h`, qty `6`.

| Slice | Trades | Net points | Avg/trade | Wins | Losses | Win rate | Worst | Best |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Audit `2026-07-14..2026-08-23` | 48 | +137.03 | +2.85 | 29 | 19 | 60.4% | -27.39 | +85.14 |
| VPS shadow `2026-08-24..2026-08-28` | 9 | +170.82 | +18.98 | 8 | 1 | 88.9% | -54.00 | +48.96 |
| Combined `2026-07-14..2026-08-28` | 57 | +307.85 | +5.40 | 37 | 20 | 64.9% | -54.00 | +85.14 |

Component split for the combined period:

| Component | Trades | Net points | Avg/trade | Wins | Losses | Win rate |
|---|---:|---:|---:|---:|---:|---:|
| MR | 31 | +147.80 | +4.77 | 27 | 4 | 87.1% |
| BO | 26 | +160.05 | +6.16 | 10 | 16 | 38.5% |

Risk metrics:

- Active trading days with trades: `32`.
- Max drawdown: about `121.18` points.
- Longest active-day drawdown duration: `22` active trading days.
- Best day: `2026-08-27`, `+106.80` points.
- Worst day: `2026-08-25`, `-54.00` points.
- Approximate value, if points are treated as qty-adjusted IMOEXF points: about `+3078.5 RUB` before live broker/slippage differences, using `1.0` IMOEXF point is about `10 RUB`.

Read:

- This is the strongest IMOEXF candidate currently visible in the shadow family, and the fresh week materially improved the post-0714 read.
- The improvement is now balanced between `MR` and `BO`; earlier audit-only data was mostly BO-driven, while the latest week added strong MR contribution.
- This is still shadow/replay evidence, not a direct live promotion trigger. Before enabling MR live again, normalize the shadow next-session gap timestamp interpretation and compare broker-realistic execution for bracket/MR paths.

## Operations And Resources

VPS at `2026-08-29 06:42 MSK`:

```text
Uptime: 10 days
Load average: 0.67, 0.61, 0.64
Memory: 7.7Gi total, 1.5Gi used, 6.2Gi available
Swap: 3.9Gi total, 263Mi used
Disk /: 79G total, 19G used, 57G available, 25%
```

Redis memory:

| Redis | Used | Peak | Maxmemory |
|---|---:|---:|---:|
| RI live | 30.95M | 32.69M | 512M |
| USDRUBF live | 30.66M | 33.34M | unlimited |
| IMOEXF live | 59.14M | 59.24M | unlimited |
| RI shadow | 18.70M | 33.03M | 384M |
| USDRUBF shadow | 18.76M | 32.73M | 384M |
| IMOEXF shadow | 22.78M | 34.28M | 384M |

Redis stream/group notes:

- active live groups had `pending=0`;
- RI live07 broker streams had `lag=0`;
- old RI groups still show large lag, but they are legacy/non-active consumers and do not block the current runtime;
- IMOEXF live riskgate ledger stream length was `260`.

Gateway/runtime noise:

- recurring WS/CWS reconnect events were observed across gateways;
- most reconnect warnings had no command in flight and `pending_count=0`;
- no runtime-level effect was seen except the one USDRUBF recoverable `action-scope cws open timeout`.

## Current State

Latest snapshots:

- RI: `phase=flat`, no `pending_entry_request_id`, no `pending_exit_request_id`.
- USDRUBF: `hybrid_state=flat`, no pending entry/exit, tracked orders empty.
- IMOEXF: `last_position_qty=0`, no active cycle, no TP/SL ids, no pending/deferred entry/exit.

Latest broker position stream on `7502MIW`:

- `RTS-9.26`: `0`;
- `USDRUBF`: `0`;
- `RUB`: cash balance visible;
- zero `IMOEXF` row was not present in the last checked position entries, but IMOEXF runtime state is flat.

## Watchlist

1. RI `2026-08-28` live short round should be compared carefully against active-only shadow journal.
   - Live executed `07:50/08:00 -> 11:30` short successfully.
   - Simple active-only shadow aggregation did not include it as a closed active decision.
   - Treat as journal/parity interpretation item, not as execution incident.
2. USDRUBF `2026-08-28 18:20` action-scoped CWS open timeout.
   - Recovery worked: deferred retry next bar, successful entry, flat exit.
   - Keep in transport/retry watchlist; no immediate patch indicated by this single event.
3. IMOEXF shadow next-session timestamps.
   - Do not use raw shadow CSV totals for promotion without applying the agreed no-overnight/gap interpretation.
   - Live no-overnight behavior remained intact this week.
4. USDRUBF scale-up remains postponed.
   - Weekly live actual was negative and shadow variants were mixed.
   - Continue micro observation before increasing quantity.
5. RI BO risk filters remain diagnostic only.
   - `bo_max2` did not improve over canonical07 this week.
   - `last_entry17` reduced exposure but gave up one profitable BO.
6. RI Author41 MR after-BO tight-stop challenger.
   - Source: `docs/ri-author41-mr-after-bo-tight-stop-watchlist-2026-09-04.md`.
   - Status: `WATCHLIST / SHADOW_ONLY / NO_LIVE_CHANGE`.
   - Do not change live RI stops now; if tested, use an isolated shadow contour
     and compare against `live07` for at least `20-30` risk sessions.

## Retrospective Verdict

Operational verdict: `GREEN_WITH_WATCHLIST`.

No live emergency action is required. Continue live micro observation. Do not change USDRUBF lot size from this week alone. RI live07 remains the cleanest production contour. IMOEXF MR should remain disabled in live until shadow evidence is stronger and the shadow gap-timestamp interpretation is normalized for promotion analysis.
