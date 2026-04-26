# Run Reference Replay

Date: 2026-04-26

This package includes the Python reference runner and the MR source trade file
needed to regenerate the expected `IMOEXF` hybrid MR+BO artifacts.

## Command

From repository root:

```bash
python analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04/replay_source_runner.py
```

## Expected Outputs

The command rewrites:

```text
replay_trades.csv
replay_daily.csv
replay_expected_summary.csv
replay_layer_attribution.csv
research_report_source.md
equity_full_base_realistic.png
equity_y2026_base_realistic.png
drawdown_full_base_realistic.png
components_full_base_realistic.png
components_y2026_base_realistic.png
```

## Notes

- This is the research/reference replay, not the Rust parity replay.
- The Rust replay should reproduce these outputs before runtime promotion.
- The runner depends on the existing repository modules under:

```text
analiz_alpha_si/moex_baseline_adaptivity_2026_04
dbo_mean_rev_test
```

## Primary Replay Check

Start with:

```text
model_id = hybrid_mr_riskgate_high180_lb120__bo_new_k053
scenario = base_realistic
```

Then check:

```text
stress_1tick
conservative_2tick
```
