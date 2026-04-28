# MOEX Author41+42 RI Replay Artifacts 2026-04-28

This artifact set records exact Rust replay parity checks for the RI standalone sleeves and combo.

Tracked files:

- `ri_author41_replay_comparison.json` - JSON summary produced by `moex_author41_replay`.
- `ri_author42_replay_comparison.json` - JSON summary produced by `moex_author42_replay`.
- `ri_author41_42_combo_replay_comparison.json` - JSON summary produced by `moex_author41_42_combo_replay`.

Local generated file, intentionally not tracked:

- `ri_2019-01-01_2026-03-27_prepared_10m.csv` - exported from the research parquet feed; 154534 rows, about 7.7 MB.

Export source:

```text
analiz_alpha_si/moex_ri_author41_long_horizon_2026_04/cache/ri_2019-01-01_2026-03-27_prepared.parquet
```

Author41 replay command:

```text
cargo run -p strategy-runtime --bin moex_author41_replay -- \
  --bars-csv docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_2019-01-01_2026-03-27_prepared_10m.csv \
  --source-trades-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_trades.csv \
  --source-daily-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_daily.csv \
  --model-id ri_author41_mr_primary \
  --out-json docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_author41_replay_comparison.json
```

Author41 result: exact parity, 1995/1995 trades and 1812/1812 daily rows.

Author42 replay command:

```text
cargo run -p strategy-runtime --bin moex_author42_replay -- \
  --bars-csv docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_2019-01-01_2026-03-27_prepared_10m.csv \
  --source-trades-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_trades.csv \
  --source-daily-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_daily.csv \
  --model-id ri_author42_bo_primary \
  --out-json docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_author42_replay_comparison.json
```

Author42 result: exact parity, 1233/1233 trades and 1812/1812 daily rows.

Combo replay command:

```text
cargo run -p strategy-runtime --bin moex_author41_42_combo_replay -- \
  --bars-csv docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_2019-01-01_2026-03-27_prepared_10m.csv \
  --source-daily-csv /Users/denisq/Documents/from_mac/projects/strategies_list/analiz_alpha_si/moex_imoexf_ri_author41_42_fixed_2026_04/fixed_candidate_daily.csv \
  --model-id ri_author41_42_primary_combo_cost2 \
  --out-json docs/moex-author41-42-shadow-2026-04-28-artifacts/ri_author41_42_combo_replay_comparison.json
```

Combo result: exact daily/source parity, 1812/1812 daily rows, total PnL
`322268.0`, Author41 component `171270.0`, Author42 component `150998.0`.

Note: combo artifact trade counts follow the source daily convention where
Author41 daily trades are active-day counts. The JSON also reports physical
diagnostics: Author41 trades `1995`, accepted Author42 trades `971`.
