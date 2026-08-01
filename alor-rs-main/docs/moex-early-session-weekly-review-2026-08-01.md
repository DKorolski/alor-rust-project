# MOEX Early-Session Weekly Review

Date: 2026-08-01

## Scope

This review covers two horizons:

- last complete trading week: `2026-07-27..2026-07-31`;
- full observation period after the MOEX session transition:
  `2026-07-14..2026-07-31`.

The review keeps three layers separate:

1. frozen model/replay economics for `legacy09` and `canonical07`;
2. operational shadow journals where a complete append-only history exists;
3. live broker truth, including commissions and actual fill timing.

Native model points are not added across RI, USDRUBF and IMOEXF because the
instruments have different point values. Broker results are reported in RUB
and, where useful, as a per-contract native-point equivalent.

## Full Period: 2026-07-14..2026-07-31

| Strategy | canonical07 | legacy09 | canonical07 delta | Read |
|---|---:|---:|---:|---|
| RI Author41/42 | 27 trades, +8731.0 | 21 trades, +7370.5 | +1360.5 | canonical07 remains ahead, mainly through BO |
| Alor-USDRUBF | 17 trades, +0.833569 | 15 trades, +1.506254 | -0.672685 | extended sample now favors legacy09 |
| IMOEXF Hybrid | 16 trades, +56.592789 | 11 trades, +55.893815 | +0.698974 | economically almost tied |

Component attribution:

| Strategy | Contour | MR | BO |
|---|---|---:|---:|
| RI Author41/42 | canonical07 | +1731.0 | +7000.0 |
| RI Author41/42 | legacy09 | +4378.5 | +2992.0 |
| Alor-USDRUBF | canonical07 | +0.436355 | +0.397214 |
| Alor-USDRUBF | legacy09 | +0.620212 | +0.886042 |
| IMOEXF Hybrid | canonical07 | -8.1 | +64.692789 |
| IMOEXF Hybrid | legacy09 | -1.2 | +57.093815 |

The RI operational canonical07 journal gives `+8731`, while the reproducible
history replay gives `+8721`. The remaining 10-point difference is small and
does not change the contour decision. The legacy09 result is exactly
`+7370.5` in both views.

## Last Week: 2026-07-27..2026-07-31

| Strategy | canonical07 | legacy09 | canonical07 delta |
|---|---:|---:|---:|
| RI Author41/42 | 10 trades, -1405.0 | 6 trades, -1682.0 | +277.0 |
| Alor-USDRUBF | 7 trades, -0.170358 | 5 trades, +0.393484 | -0.563842 |
| IMOEXF Hybrid | 8 trades, -29.686215 | 3 trades, -21.886212 | -7.800003 |

Weekly component split:

| Strategy | Contour | MR | BO |
|---|---|---:|---:|
| RI Author41/42 | canonical07 | -567.0 | -838.0 |
| RI Author41/42 | legacy09 | -456.0 | -1226.0 |
| Alor-USDRUBF | canonical07 | +0.054919 | -0.225277 |
| Alor-USDRUBF | legacy09 | +0.292498 | +0.100986 |
| IMOEXF Hybrid | canonical07 | -14.0 | -15.686215 |
| IMOEXF Hybrid | legacy09 | -16.1 | -5.786212 |

The week is an important counter-sample: all canonical07 model contours were
negative. RI canonical07 lost slightly less than legacy09, while USDRUBF and
IMOEXF legacy09 were better. This is not sufficient evidence to reverse a live
contour immediately, but it prevents treating the earlier canonical07 lead as
already stable.

## Live Broker Truth for the Week

| Live strategy | Qty | Flat cycles | Net RUB | Net native points per contract | Notes |
|---|---:|---:|---:|---:|---|
| RI Author41/42 | 1 | 6 | -2107.02 | -1488.217 | MR -509.087, BO -979.130 |
| Alor-USDRUBF | 2 | 7 | +698.12 | +0.349060 | all observed cycles were BO |
| IMOEXF Hybrid | 6 | 8 | -4725.40 | -78.756667 | MR -58.088667, BO -20.668000 |

USDRUBF finished the week positive only because of the large July 28 BO win.
Its live entry/exit times differ materially from the canonical replay on
several days. The direction and component can still be valid, but this is not
a clean parity confirmation and needs a focused signal-to-fill audit.

IMOEXF was still on live09 on July 28 and moved to canonical07 for July 29-31.
For the canonical live sub-period, broker execution was approximately
`-61.965` net native points per contract versus approximately `-32.086` in the
canonical model replay. The roughly 30-point gap is execution/timing drift,
not evidence that can be attributed to signal logic alone.

RI broker execution for the week is closer to the legacy09 shadow result than
to the raw live model-decision total. This supports continuing broker-truth
reconciliation rather than promoting canonical07 solely from its cumulative
shadow PnL.

## Operational and Data Quality Checks

- All three broker positions were flat at the review checkpoint.
- All nine live/shadow runtimes were healthy with zero restarts.
- No isolated shadow command stream contained broker commands.
- RI shadow journals passed strict review after reducing events to the latest
  state per `decision_key`; restart duplicates were not counted as trades.
- Full USDRUBF and IMOEXF shadow economics were reconstructed with the frozen
  MOEX-data replay because their operational CSV outputs were overwritten by
  restarts and do not provide one continuous append-only history.
- Exact broker-truth reconstruction is reliable for July 27-31. The current
  Redis history does not provide a complete, balanced broker ledger for every
  cycle from July 14 onward, so no exact full-period broker PnL is claimed.
- The isolated IMOEXF shadow risk-gate ledgers have a historical Jul9-Jul21
  observation gap from before continuous ledger ownership and are also missing
  the July 24 row inside the active observation interval. The production live
  ledger is complete through July 30, so live gate operation was not affected.

## Verdict

### RI Author41/42

`KEEP_OBSERVING`. canonical07 remains the best cumulative shadow contour since
July 14, with a `+1360.5` lead over legacy09, but the lead is concentrated in a
small number of strong BO days. Both contours lost money in the latest week.
Keep live RI unchanged for now and collect more complete sessions before a
live07 decision.

### Alor-USDRUBF

`NO_PROMOTION`. The longer replay now favors legacy09, while the positive live
week is concentrated in one BO trade and does not align cleanly with the
canonical signal schedule. Keep the current micro size and prioritize daily
model-intent-fill reconciliation.

### IMOEXF Hybrid

`CONTROLLED_LIVE07_OBSERVATION`. Full-period canonical07 and legacy09 model
economics are effectively tied, and the last week favored legacy09. Continue
the already-started canonical07 micro observation without increasing quantity.
The main near-term question is the live execution gap, especially MR timing and
bracket lifecycle, rather than a further parameter change.

## Follow-up

1. Continue at least 10-20 additional complete sessions before another
   session-anchor or quantity decision.
2. Produce a daily USDRUBF and IMOEXF audit joining model signal time, runtime
   intent, broker order/fill, exit reason and final broker-flat state.
3. Deploy the restart-safe cumulative report patch described in
   `moex-shadow-report-persistence-rollout-2026-08-01.md`.
4. Use the new continuity warning to investigate the missing July 24 row in
   isolated IMOEXF shadow ledgers; do not modify the complete production ledger.
5. Do not change K, TP/SL, quantity or RI live session anchor from this weekly
   sample alone.

Overall status: `KEEP_OBSERVING / NO_SCALE_CHANGE`.
