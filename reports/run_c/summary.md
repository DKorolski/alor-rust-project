# Run C — full-period backtest summary (sample strategy)

## Scope

- **Period (UTC):** 2023-01-09 → 2026-02-24
- **Trades:** 439
- **Purpose:** reproducibility / engineering validation snapshot for repository demo.

> This report is a backtest/replay artifact and is not a performance guarantee.

## Metrics

| Metric | Value |
|---|---:|
| Starting capital | 30,000 |
| Ending capital | 42,558 |
| Total return | 41.86% |
| Net PnL (sum pnl) | 12,558.20 |
| Win rate | 71.98% |
| Profit factor | 1.85 |
| Max drawdown | -2.37% (-883) |
| Sharpe (daily, ffill days) | 2.38 |
| CAGR (est.) | 11.70% |
| Avg / Median PnL | 28.60 / 51.71 |
| Avg / Median per trade (bps notional) | 9.70 / 17.74 |
| Avg / Median holding time | 3.91h / 2.75h |

## Reproducibility checklist

Fill/keep these fields for exact reruns:

- Commit SHA: `<fill>`
- Strategy/config path: `<fill>`
- Data source + data snapshot/version: `<fill>`
- Fees model: `<fill>`
- Slippage model: `<fill>`
- Run command(s): `<fill>`

## Notes / limitations

- Backtest quality is sensitive to data quality, bar alignment, and execution assumptions.
- Live behavior may differ due to latency, fill mechanics, and broker-side specifics.
- Keep report + config + input data pinned together for fair comparison across runs.
