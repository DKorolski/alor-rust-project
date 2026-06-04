# Broker Ledger Reconciliation - 2026-05-22

Source raw file: `docs/broker-ledger-2026-05-22-raw.md`.

Analysis timestamp: 2026-05-22.

## Parse Summary

Parsed broker ledger:

| Portfolio | Trades | Symbols |
| --- | ---: | --- |
| `7502MIW` | 78 | `USDRUBF`, `RTS-6.26` |
| `7502T0U` | 70 | `USDRUBF` |
| `7502SN6` | 81 | `IMOEXF` |

Total parsed rows:

- Trade rows: 229
- Broker commission rows: 66

Important attribution note:

- `7502MIW` is shared by `sessiongap` and `RI author41/42 micro`.
- `USDRUBF` rows belong to `sessiongap`.
- `RTS-6.26` / `RIM6` rows belong to `RI author41/42 micro`.
- Portfolio-level broker commission rows on `7502MIW` are mixed and cannot be assigned to one strategy without additional broker detail.

## 2026-05-22 Reconciliation

Broker ledger for 2026-05-22 contains only the RI micro cycle:

| Portfolio | Symbol | Time MSK | Side | Qty | Price |
| --- | --- | ---: | --- | ---: | ---: |
| `7502MIW` | `RTS-6.26` | 09:10 | Sell | 1 | 117570 |
| `7502MIW` | `RTS-6.26` | 09:40 | Buy | 1 | 117460 |

Runtime match:

| Runtime event | Time MSK | Side | Qty | Price | Order id |
| --- | ---: | --- | ---: | ---: | --- |
| `execution_confirmed` | 09:10:03 | Sell | 1 | 117570 | `1925039865741727949` |
| `execution_confirmed` | 09:40:37 | Buy | 1 | 117460 | `1925039865741768489` |

Result:

- Direction: short
- Gross: `+110` RTS points before commission
- Runtime commission: `11.06 + 11.06 = 22.12`
- No broker-ledger extra rows for `USDRUBF`, `IMOEXF`, or `7502T0U` today at the checked export point.
- No `orphan_trade`, reject, timeout, or insufficient-funds event appeared in runtime for this RI cycle.

Verdict:

- 2026-05-22 broker ledger fully matches the runtime observation for RI.
- Shared `7502MIW` portfolio did not create attribution ambiguity today because only `RTS-6.26` traded.

## Current-Contour Broker Aggregates

These aggregates use broker fills, FIFO pairing, and the current contour windows used in the live economics review.

| Contour | Broker window | Closed cycles | Closed qty | Gross points/contracts | Notes |
| --- | ---: | ---: | ---: | ---: | --- |
| `sessiongap` / `USDRUBF` / `7502MIW` | exits from 2026-04-23 | 11 | 11 | `+0.28` | Includes 2026-04-23 `-0.25` cycle that was outside the runtime-log economics slice. |
| `alor-USDRUBF` / `7502T0U` | exits from 2026-04-23 | 21 | 21 | `+1.69` | Includes 2026-04-23 and 2026-04-24 early cycles not present in the runtime-log economics slice. |
| `hybrid IMOEXF` / `7502SN6` | exits from 2026-05-09 | 17 | 28 | `+30.0` | Includes the 2026-05-21 evening BO stop cycle after the 14:20 runtime-log review checkpoint. |
| `RI author41/42 micro` / `RTS-6.26` / `7502MIW` | exits from 2026-05-08 | 21 | 21 | `+5580` | Matches previous RI total `+5470` plus today’s `+110`. |

Open-lot check:

- Broker FIFO pairing ended with no open lots in the parsed export.
- This agrees with runtime flat-state checks across all active contours.

## Differences Versus Runtime-Log Economics Review

Reference: `docs/live-economics-review-2026-05-21.md`.

### Sessiongap

Runtime-log review:

- 10 cycles
- Gross `+0.53`
- First parsed runtime fill: 2026-04-28

Broker ledger:

- 11 cycles from 2026-04-23
- Gross `+0.28`

Explanation:

- Broker ledger includes an additional 2026-04-23 cycle with gross `-0.25`.
- From 2026-04-28 onward the broker-ledger day-level results match the runtime-log economics review.

### Alor-USDRUBF

Runtime-log review:

- 18 cycles
- Gross `+2.18`
- First parsed runtime fill: 2026-04-27

Broker ledger:

- 21 cycles from 2026-04-23
- Gross `+1.69`

Explanation:

- Broker ledger includes early 2026-04-23 and 2026-04-24 cycles totaling `-0.49`.
- From 2026-04-27 onward the broker-ledger day-level results match the runtime-log economics review.

### Hybrid IMOEXF

Runtime-log review:

- 16 cycles
- Closed qty 26
- Gross `+37.0`
- Review checkpoint was 2026-05-21 14:20 MSK.

Broker ledger:

- 17 cycles
- Closed qty 28
- Gross `+30.0`

Explanation:

- Broker ledger includes an additional evening cycle on 2026-05-21 after the runtime-log review checkpoint:
  - 20:20 MSK: buy 2 `IMOEXF` @ `2654.5`
  - 21:00 MSK: sell 2 `IMOEXF` @ `2651.0`
  - Gross `-7.0`
- Runtime logs confirm this was a normal `IntradayBreakout` exit:
  - `BreakoutStop1Long`
  - action emitted at 20:50 bar
  - exit execution confirmed at 21:00:02 MSK
- No orphan/reject/cleanup anomaly was observed in this evening cycle.

### RI Author41/42 Micro

Runtime-log review:

- Through 2026-05-21: 20 cycles, gross `+5470`

Broker ledger:

- Through 2026-05-22 09:40: 21 cycles, gross `+5580`

Explanation:

- Difference is exactly today’s clean RI short cycle, gross `+110`.
- Broker and runtime match for both prices and sides.

## Broker Commission Rows

Commission rows are useful for broker-level cash reconciliation, but should be handled carefully:

- `7502MIW` commission rows are portfolio-level and mix `USDRUBF` sessiongap with `RTS-6.26` RI.
- Some latest-day commission rows may not be present yet in the exported broker ledger.
- Broker commission rows do not provide enough detail here to allocate exact commission per strategy on the shared portfolio.
- Runtime `execution_confirmed` commission remains the better per-fill source for strategy-level diagnostics.

Recent examples:

| Portfolio | Date | Broker commission row |
| --- | ---: | ---: |
| `7502MIW` | 2026-05-20 | `-73.52 RUB` |
| `7502MIW` | 2026-05-19 | `-50.68 RUB` |
| `7502SN6` | 2026-05-20 | `-7.04 RUB` |
| `7502T0U` | 2026-05-20 | `-2.00 RUB` |

## Operational Read

- Current broker ledger confirms today’s RI cycle and current flat state.
- The shared `7502MIW` portfolio is manageable as long as we keep symbol-level attribution explicit.
- No hidden broker-side trade was found for 2026-05-22 outside RI.
- The main correction to prior economics is the 2026-05-21 evening `hybrid IMOEXF` cycle, which updates the current qty2 broker-view gross from `+37.0` to `+30.0`.

## Follow-Up

- Keep `docs/broker-ledger-2026-05-22-raw.md` as immutable raw input.
- Use this reconciliation as the derived analysis layer.
- If we repeat this workflow, add a small parser script for broker-ledger exports so runtime-log and broker-ledger economics stay reproducible.
