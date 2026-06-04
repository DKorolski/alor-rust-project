# Live economics review - 2026-05-21

Status timestamp: 2026-05-21 14:20 MSK.

## Scope

This review summarizes live economics from the active VPS runtime logs.

The windows are intentionally contour-specific:

| System | Runtime container | Window start | Reason |
| --- | --- | ---: | --- |
| `sessiongap` | `trading-sessiongap-strategy-runtime-1` | 2026-04-23 00:00 MSK | Current 10m/from-zero live contour. |
| `alor-USDRUBF` | `trading-alor-usdrubf-strategy-runtime-1` | 2026-04-23 00:00 MSK | Current 10m/from-zero live contour. |
| `hybrid IMOEXF` | `trading-hybrid-strategy-runtime-1` | 2026-05-09 00:00 MSK | Current size-2/riskgate-shadow live contour. |
| `RI author41/42 micro` | `trading-ri-author41-42-7502miw-strategy-runtime-1` | 2026-05-08 00:00 MSK | Clean micro-live contour after symbol-routing remediation. |

## Method

- Source: runtime `docker logs` on VPS `155.212.170.21`.
- Parsed fills: `execution_confirmed` plus `orphan_trade` lines when a broker trade arrived before a matching command ack.
- Pairing model: FIFO position pairing per symbol and side.
- Current result: all parsed windows end flat, so no incomplete open-lot tail is present in this report.
- Gross result is reported as price points/contracts before fees.
- Runtime logged commissions are reported separately. `orphan_trade` lines do not carry commission, so missing commission is estimated from observed per-contract commission for the same symbol.
- Do not interpret `gross_points - commission` as final RUB PnL without applying the correct instrument multiplier/tick value.

## Summary

| System | Symbol | Closed cycles | Closed qty | Gross points/contracts | Logged commission | Est. missing commission | Est. total commission | First parsed fill | Last parsed fill |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| `sessiongap` | `USDRUBF` | 10 | 10 | `+0.53` | `51.04` | `17.01` | `68.05` | 2026-04-28 13:00 MSK | 2026-05-20 23:30 MSK |
| `alor-USDRUBF` | `USDRUBF` | 18 | 18 | `+2.18` | `102.35` | `20.47` | `122.82` | 2026-04-27 11:20 MSK | 2026-05-21 12:00 MSK |
| `hybrid IMOEXF` | `IMOEXF` | 16 | 26 | `+37.0` | `73.68` | `17.53` | `91.21` | 2026-05-11 12:10 MSK | 2026-05-21 10:00 MSK |
| `RI author41/42 micro` | `RTS-6.26` | 20 | 20 | `+5470` | `365.70` | `77.57` | `443.27` | 2026-05-08 09:10 MSK | 2026-05-21 10:20 MSK |

## Direction split

| System | Long cycles | Long gross | Short cycles | Short gross | Read |
| --- | ---: | ---: | ---: | ---: | --- |
| `sessiongap` | 3 | `-0.04` | 7 | `+0.57` | Positive contribution came from short-side trades. |
| `alor-USDRUBF` | 4 | `+0.31` | 14 | `+1.87` | Mostly short-side activity, both sides positive in this window. |
| `hybrid IMOEXF` | 11 | `+11.0` | 5 | `+26.0` | Both components/directions contributed; current qty2 contour remains positive. |
| `RI author41/42 micro` | 10 | `+4170` | 10 | `+1300` | Strongest absolute point result; long-side contribution dominates. |

## Daily gross

### Sessiongap USDRUBF

| Date | Cycles | Gross points/contracts |
| --- | ---: | ---: |
| 2026-04-28 | 1 | `+0.03` |
| 2026-05-04 | 1 | `+0.02` |
| 2026-05-06 | 1 | `+0.03` |
| 2026-05-08 | 1 | `+0.12` |
| 2026-05-11 | 1 | `-0.07` |
| 2026-05-13 | 1 | `+0.01` |
| 2026-05-14 | 1 | `+0.00` |
| 2026-05-18 | 1 | `+0.16` |
| 2026-05-19 | 1 | `+0.32` |
| 2026-05-20 | 1 | `-0.09` |

### Alor-USDRUBF

| Date | Cycles | Gross points/contracts |
| --- | ---: | ---: |
| 2026-04-27 | 1 | `-0.07` |
| 2026-04-29 | 1 | `+0.13` |
| 2026-04-30 | 1 | `+0.06` |
| 2026-05-04 | 1 | `+0.13` |
| 2026-05-05 | 1 | `-0.17` |
| 2026-05-06 | 1 | `+0.08` |
| 2026-05-07 | 1 | `-0.03` |
| 2026-05-08 | 1 | `-0.03` |
| 2026-05-11 | 1 | `-0.07` |
| 2026-05-12 | 1 | `-0.14` |
| 2026-05-13 | 1 | `-0.10` |
| 2026-05-14 | 1 | `+0.17` |
| 2026-05-15 | 1 | `+0.06` |
| 2026-05-18 | 1 | `+0.63` |
| 2026-05-19 | 1 | `+0.79` |
| 2026-05-20 | 1 | `+0.35` |
| 2026-05-21 | 2 | `+0.39` |

### Hybrid IMOEXF

| Date | Cycles | Closed qty | Gross points/contracts |
| --- | ---: | ---: | ---: |
| 2026-05-11 | 1 | 2 | `+16.0` |
| 2026-05-12 | 3 | 4 | `-22.0` |
| 2026-05-13 | 1 | 2 | `-15.0` |
| 2026-05-14 | 1 | 2 | `+33.0` |
| 2026-05-15 | 1 | 2 | `-10.0` |
| 2026-05-18 | 2 | 2 | `+14.0` |
| 2026-05-19 | 2 | 4 | `+14.0` |
| 2026-05-20 | 1 | 2 | `+5.0` |
| 2026-05-21 | 4 | 6 | `+2.0` |

### RI author41/42 micro

| Date | Cycles | Gross points/contracts |
| --- | ---: | ---: |
| 2026-05-08 | 2 | `+280` |
| 2026-05-11 | 2 | `-890` |
| 2026-05-12 | 3 | `+690` |
| 2026-05-13 | 1 | `+150` |
| 2026-05-14 | 2 | `+410` |
| 2026-05-15 | 1 | `-50` |
| 2026-05-18 | 2 | `+1390` |
| 2026-05-19 | 3 | `+1170` |
| 2026-05-20 | 2 | `+1890` |
| 2026-05-21 | 2 | `+430` |

## Interpretation

- `RI author41/42 micro` has the strongest absolute point result in the clean micro window: `+5470` RTS points over 20 closed cycles. It should still remain in controlled micro observation because the clean live window is shorter than the USDRUBF contours and still includes `orphan_trade` observability cases.
- `hybrid IMOEXF` is positive in the current qty2 contour: `+37.0` IMOEXF points/contracts over 26 closed contracts. Size-2 partial TP behavior was observed and did not create an uncontrolled position. Keep it at qty 2 until cleanup idempotency is either stable for several more sessions or patched.
- `alor-USDRUBF` is positive over the current 10m/from-zero contour: `+2.18` USDRUBF price-points/contracts over 18 cycles. Recent contribution improved materially from 2026-05-18 through 2026-05-21.
- `sessiongap` is low-frequency and mildly positive: `+0.53` USDRUBF price-points/contracts over 10 cycles. Most positive contribution came from short-side trades; this is not enough to justify aggressive scaling by itself.
- Raw totals are not directly comparable across `USDRUBF`, `IMOEXF`, and `RTS-6.26` without contract multipliers, tick values, and capital/margin normalization.

## Watch items

- Continue daily economics capture with the same parser rules so `orphan_trade` fills are not accidentally dropped from PnL.
- Add a small reusable economics helper script if this review becomes a weekly routine; ad-hoc parsing is already useful but too easy to drift.
- Keep `hybrid IMOEXF` at qty 2 while watching cleanup idempotency after TP/SL bracket cycles.
- Keep `RI author41/42` in micro-live observation for several more clean sessions before considering any increase.
- If a RUB net view is needed, add instrument metadata: tick size, tick value, contract multiplier, actual broker fee schedule, and currency conversion where applicable.
