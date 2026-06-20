# RI Author41/42 Live Trade Ledger - 2026-06-20

Scope: broker-confirmed `RTS-9.26` micro-live cycles for the two Author41/42
contours. Times are Moscow time. PnL is indicative and uses authoritative trade
fills, with reported broker commissions subtracted.

## 2026-06-19

### Portfolio 7502MIW

| Component | Side | Entry | Entry price | Exit | Exit price | Gross points | Commission | Indicative net |
| --- | --- | --- | ---:| --- | ---:| ---:| ---:| ---:|
| Author41 MR | short | `09:20:13` | 102810 | `10:30:01` | 102320 | +490 | 19.96 | +470.04 |
| Author41 MR | long | `10:40:01` | 102480 | `10:50:06` | 102470 | -10 | 19.96 | -29.96 |

Session total:

- gross: `+480` points;
- commission: `39.92`;
- indicative net: `+440.08`.

### Portfolio 7502T0U

| Component | Side | Entry | Entry price | Exit | Exit price | Gross points | Commission | Indicative net |
| --- | --- | --- | ---:| --- | ---:| ---:| ---:| ---:|
| Author41 MR | short | `09:20:13` | 102800 | `10:30:02` | 102290 | +510 | 19.96 | +490.04 |
| Author41 MR | long | `10:40:02` | 102470 | `10:50:05` | 102490 | +20 | 19.96 | +0.04 |

Session total:

- gross: `+530` points;
- commission: `39.92`;
- indicative net: `+490.08`.

## Operational Notes

- all eight entry/exit commands were accepted;
- intent-to-fill latency was generally below `600 ms`;
- both portfolios finished `RTS-9.26=0`;
- no reject, orphan trade, pending request tail, or residual position was
  observed;
- price differences between portfolios were normal independent market fills.

