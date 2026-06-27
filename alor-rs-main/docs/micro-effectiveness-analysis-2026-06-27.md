# Micro effectiveness analysis — broker-truth snapshot

Date: 2026-06-27.
Source: Alor broker REST history, `dateFrom=2026-04-01`, `limit=1000` per portfolio.

This memo is a broker-truth first pass. It uses executed broker trades and exchange contract economics, not runtime model PnL.

## Inputs and assumptions

Portfolios:

- `7502MIW`
- `7502T0U`
- `7502SN6`

Fetched records:

- `7502MIW`: 300 trades, 2026-04-01 to 2026-06-26.
- `7502T0U`: 197 trades, 2026-04-07 to 2026-06-26.
- `7502SN6`: 111 trades, 2026-04-03 to 2026-05-25.

Current-session endpoint was also checked on 2026-06-27 and returned zero trades for all three portfolios.

Contract economics used:

| Instrument | Price step | Step value |
|---|---:|---:|
| `RTS-*` / RI | 10 | 15.12694 RUB |
| `IMOEXF` | 0.5 | 5 RUB |
| `USDRUBF` | 0.01 | 10 RUB |

Commissions and exchange fees are not included because `commission` was `null` in the broker API response.

Broker `comment` was empty for these records, so this pass attributes systems by portfolio + symbol + known deployment timeline. A follow-up reconciliation should join broker trades to runtime `cmd.orders`, `cmd.acks`, `broker.trades`, and strategy state to split MR/BO and Author41/Author42 precisely.

## Executive result

Broker-truth gross PnL across all reviewed micro systems:

```text
+22,451.63 RUB
```

All groups ended flat by broker-trade net quantity.

High-level grouping:

| Group | Round-trips | Gross PnL | Win rate | Profit factor | Max win | Max loss |
|---|---:|---:|---:|---:|---:|---:|
| USDRUBF total | 104 | +5,450.00 | 51.9% | 1.62 | +1,920.00 | -540.00 |
| IMOEXF total | 139 | +4,295.00 | 61.2% | 1.52 | +960.00 | -1,350.00 |
| RI total | 70 | +12,706.63 | 78.6% | 1.67 | +2,284.17 | -6,262.55 |
| All systems | 313 | +22,451.63 | 62.0% | 1.63 | +2,284.17 | -6,262.55 |

The result is positive, but the distribution is not smooth. The main positive driver was old RI / `RTS-6.26`; the main negative event was current RI / `RTS-9.26` on the 2026-06-16 Hormuz/news-shock session.

## System-level stats

| System | Round-trips | Gross PnL | Win rate | Avg trade | Profit factor | Worst day | Max loss |
|---|---:|---:|---:|---:|---:|---:|---:|
| USDRUBF / `7502MIW` | 59 | +2,710.00 | 49.2% | +45.93 | 1.52 | -1,010.00 | -540.00 |
| USDRUBF / `7502T0U` | 45 | +2,740.00 | 55.6% | +60.89 | 1.78 | -590.00 | -380.00 |
| IMOEXF old hybrid / `7502SN6` | 57 | +590.00 | 61.4% | +10.35 | 1.35 | -220.00 | -190.00 |
| IMOEXF / `7502MIW` | 40 | +2,310.00 | 60.0% | +57.75 | 1.78 | -1,140.00 | -1,140.00 |
| IMOEXF / `7502T0U` | 42 | +1,395.00 | 61.9% | +33.21 | 1.38 | -1,015.00 | -1,350.00 |
| RI old `RTS-6.26` / `7502MIW` | 47 | +19,513.75 | 87.2% | +415.19 | 7.90 | -1,346.30 | -2,042.14 |
| RI old `RTS-6.26` / `7502T0U` | 6 | +2,057.26 | 83.3% | +342.88 | 35.00 | -60.51 | -60.51 |
| RI current `RTS-9.26` / `7502MIW` | 9 | -4,492.70 | 44.4% | -499.19 | 0.44 | -6,262.55 | -6,262.55 |
| RI current `RTS-9.26` / `7502T0U` | 8 | -4,371.69 | 62.5% | -546.46 | 0.45 | -6,262.55 | -6,262.55 |

## Comments by system

### USDRUBF

USDRUBF is modestly positive and relatively stable.

Read:

- Both portfolios are positive.
- Win rate is near coin-flip, but wins are larger than losses.
- Tail size is small relative to RI and IMOEXF.
- The best single USDRUBF broker-truth trade was +1,920 RUB on 2026-06-26.

Operational conclusion:

- USDRUBF is a good candidate to port to a new gateway early.
- It is useful as a low-risk live integration smoke system because market entry/exit is simpler than bracket MR.
- Keep it micro until the new broker gateway proves fills, positions, and reconnect behavior.

What to port:

- market entry/exit semantics;
- live guard / readiness wait;
- broker-truth reconciliation;
- per-symbol position filtering;
- current small sizing.

### IMOEXF

IMOEXF is positive overall, but less clean than USDRUBF.

Read:

- Old `7502SN6` hybrid was only slightly positive, with small average trade.
- Newer `7502MIW` / `7502T0U` IMOEXF lines are more productive, but show larger downside on some bracket/position-management days.
- The largest losses are tied to later larger qty trades and/or residual/manual/emergency flows, not necessarily pure model alpha.

Operational conclusion:

- IMOEXF should be ported after the new gateway proves basic market entry/exit.
- MR bracket semantics should not be the first live test on a new broker adapter.
- Split MR and BO before scale-up: broker comment is empty, so runtime reconciliation is required.

What to port:

- MR bracket lifecycle only after:
  - create market/limit/stop support is stable;
  - order-id correlation is proven;
  - partial fill scope is confirmed MR-only;
  - emergency residual handling is broker-adapter neutral.
- BO can remain separate from MR bracket semantics.

### RI / RTS

RI has the strongest evidence of edge, but also the clearest tail-risk warning.

Old contract `RTS-6.26`:

- +21,571.02 RUB total.
- 53 round-trips.
- 86.8% win rate.
- Profit factor 8.47.

Current contract `RTS-9.26`:

- -8,864.39 RUB total.
- 17 round-trips.
- Profit factor 0.45.
- Dominated by 2026-06-16 Hormuz/news-shock tail event.

Without the 2026-06-16 tail event:

```text
RTS-9.26 ex-Hormuz: +3,660.72 RUB
round-trips: 15
win rate: 60.0%
profit factor: 2.05
max loss: -922.74 RUB
```

Read:

- RI should not be dismissed because of the RTS-9.26 headline loss.
- The post-tail behavior recovered about 7.1k RUB from the low.
- However, RI MR is not ready for scale-up without an explicit event-risk control.

Operational conclusion:

- RI is the most interesting strategy to port, but not the first one to scale.
- Port it in micro first.
- Add event-risk kill switch before larger size.

What to port:

- closed-bar signal to next-bar-open intent semantics;
- readiness wait / no first-intent drop;
- runtime state preservation on readiness delay;
- event-risk pause flag;
- pre-open gap / volatility guard;
- detailed model-signal-ts / intent-ts / fill-ts audit.

## Largest positive and negative observations

Largest losses:

| System | Open -> close | Side | PnL |
|---|---|---|---:|
| RI `RTS-9.26` / `7502MIW` | 2026-06-16 06:20Z -> 13:40Z | long | -6,262.55 |
| RI `RTS-9.26` / `7502T0U` | 2026-06-16 06:20Z -> 13:40Z | long | -6,262.55 |
| RI `RTS-6.26` / `7502MIW` | 2026-05-11 06:30Z -> 10:00Z | short | -2,042.14 |
| IMOEXF / `7502T0U` | 2026-06-23 07:00Z -> 17:10Z | short qty 3 | -1,350.00 |

Largest wins:

| System | Open -> close | Side | PnL |
|---|---|---|---:|
| RI `RTS-9.26` / `7502MIW` | 2026-06-23 06:20Z -> 06:50Z | short | +2,284.17 |
| RI `RTS-9.26` / `7502T0U` | 2026-06-23 06:20Z -> 06:50Z | short | +2,253.91 |
| RI `RTS-6.26` / `7502MIW` | 2026-05-20 08:10Z -> 20:10Z | short | +2,027.01 |
| USDRUBF / `7502MIW` | 2026-06-26 08:10Z -> 14:35Z | long | +1,920.00 |

## What to carry forward to a Finam/T-Bank gateway

### Port first

1. Broker-neutral event model:
   - normalized orders;
   - normalized trades;
   - normalized positions;
   - stable broker order id mapping;
   - explicit exchange timestamp and receive timestamp.

2. Auth/readiness/live guard:
   - no trading until broker connection, positions, and order streams are ready;
   - readiness wait instead of dropping first valid intent;
   - explicit `missed_due_runtime_not_ready` only after timeout.

3. Market entry/exit path:
   - start with USDRUBF-like simple market lifecycle;
   - validate current-session trades;
   - validate position flatness;
   - validate reconnect replay behavior.

4. Reconciliation layer:
   - broker-truth import;
   - runtime command matching;
   - daily PnL by strategy owner;
   - orphan/unmatched trade classification.

### Port second

1. IMOEXF MR bracket:
   - only after market path and order correlation are stable;
   - prove bracket placement/cancel semantics on the new broker;
   - keep partial-fill accumulation MR-bracket scoped.

2. RI MR:
   - port in micro;
   - require event-risk guard before scale-up;
   - preserve freeze-intent parity.

### Do not port blindly

- Alor-specific CWS/action-scoped implementation details.
- Assumptions about order/trade replay ordering.
- Alor-specific orphan-trade interpretation.
- Any emergency residual behavior that depends on Alor stream timing.

## Recommended migration sequence

1. Build new gateway adapter in shadow/read-only mode:
   - auth;
   - portfolios;
   - positions;
   - orders;
   - trades;
   - current-session and historical import where available.

2. Run broker-truth reconciliation in parallel with Alor:
   - no orders;
   - compare market data and portfolio state.

3. Enable one simple market-entry system in micro:
   - preferably USDRUBF-style market entry/exit;
   - no bracket semantics yet.

4. Add IMOEXF simple/BO path.

5. Add MR bracket path.

6. Add RI MR only after:
   - freeze-intent timing is validated;
   - event-risk guard is implemented;
   - new broker fill/replay behavior is characterized.

## Decision read

The live micro program has positive gross expectancy across the reviewed period. It is not just noise, but it is not yet a scale-up-ready production result either.

Best candidates to carry forward:

1. RI MR alpha, with event-risk guard and micro-only initial migration.
2. USDRUBF as a stable simple-market gateway validation system.
3. IMOEXF MR/BO after adapter bracket semantics are proven.

Main blockers before scale-up:

- broker migration risk;
- lack of commission/fee-inclusive net PnL;
- missing precise strategy-owner attribution in broker comments;
- RI event-tail control;
- MR bracket adapter semantics on the new broker.
