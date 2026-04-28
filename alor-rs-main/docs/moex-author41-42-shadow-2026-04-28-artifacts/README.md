# MOEX Author41 RI Replay Artifacts 2026-04-28

This artifact set records the first exact Rust replay parity check for `ri_author41_mr_primary`.

Tracked files:

- `ri_author41_replay_comparison.json` - JSON summary produced by `moex_author41_replay`.

Local generated file, intentionally not tracked:

- `ri_2019-01-01_2026-03-27_prepared_10m.csv` - exported from the research parquet feed; 154534 rows, about 7.7 MB.

Export source:

```text
analiz_alpha_si/moex_ri_author41_long_horizon_2026_04/cache/ri_2019-01-01_2026-03-27_prepared.parquet
```

Replay command:

```text
cargo run -p strategy-runtime --bin moex_author41_replay -- \
  --bars-csv docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_2019-01-01_2026-03-27_prepared_10m.csv \
  --source-trades-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_trades.csv \
  --source-daily-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_daily.csv \
  --model-id ri_author41_mr_primary \
  --out-json docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_author41_replay_comparison.json
```

Result: exact parity, 1995/1995 trades and 1812/1812 daily rows.
