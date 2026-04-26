# IMOEXF Hybrid MR + BO Handoff Package

Date: 2026-04-26

This package freezes the current research recommendation for the updated `IMOEXF`
hybrid model and defines what must be implemented or replay-checked before using
it in extended micro soak.

## Package Verdict

Status: `PENDING_BO_GAP_FLATTEN_PARITY_CHECK`

The current recommended model is not a blind runtime replacement. It is a
candidate runtime update that should pass a dedicated `10m` replay parity pass
first. The remaining BO gap-flatten item is a replay/runtime parity check around
fill timing, not evidence that the model logic is broken.

## Recommended Model

Primary carry-forward candidate:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

Secondary shadow/canary candidate:

```text
hybrid_mr_adaptive_lb120_mean__bo_new_k053
```

Reasoning:

- `bo_new_k053` is the cleaner broad/test BO contour.
- `K=0.59` is only a recent-forward alternate.
- The MR fixed high-K contour is phase-sensitive, so the recommended runtime
  version uses the simpler `riskgate_high180_lb120` smoothing rule.
- Hybrid accounting uses single-position no-overlap merge with MR priority.

## Included Artifacts

- `FROZEN_MODEL_SPEC.md`: model logic and parameters.
- `WEEKEND_SESSION_POLICY.md`: 2026 weekend-session handling and regular
  weekday anchor contract.
- `IMOEXF_MR_BO_SESSION_AUDIT.md`: audit of MR window, MR/BO overlap, and BO
  weekend/non-trading-gap behavior in the current replay artifact.
- `IMPLEMENTATION_TODO.md`: developer task list.
- `REPLAY_CONTRACT.md`: required replay contract and acceptance criteria.
- `imoexf_hybrid_mr_bo_manifest.csv`: frozen model/package manifest.
- `replay_source_runner.py`: Python reference runner used to generate artifacts.
- `RUN_REFERENCE_REPLAY.md`: command for rerunning the Python reference replay.
- `imoexf_mr_execution_economics_strategy_trades.csv`: MR source trades needed by
  the reference replay runner.
- `replay_expected_summary.csv`: expected summary metrics.
- `replay_layer_attribution.csv`: MR/BO attribution.
- `replay_daily.csv`: expected daily PnL series.
- `replay_trades.csv`: expected trade-level replay output.
- `research_report_source.md`: source research report.
- `equity_*.png`, `drawdown_*.png`, `components_*.png`: review charts.

## Do Not Change During Replay

- Do not retune `bo_k`, BO stops, or MR contour during replay.
- Do not switch from `10m` close-bar contract to `1m` runtime bars for the parity
  verdict.
- Do not trade Saturday/Sunday sessions.
- Do not use weekend calendar previous-day anchors for MR or BO.
- Do not let weekend bars update the MR risk gate or create separate decision
  dates.
- Do not allow BO positions to carry across weekend/non-tradable gaps; this is
  a known artifact issue pending replay correction.
- Do not evaluate MR exits on intra-bar high/low unless a separate execution
  realism pass is explicitly created.

## Next Gate

Before extended micro soak:

1. Implement or adapt a Rust replay path for this frozen package.
2. Run replay against `replay_trades.csv` / `replay_daily.csv`.
3. Confirm signal, side, owner, entry/exit timestamp, and PnL parity within the
   tolerances in `REPLAY_CONTRACT.md`.
4. Only then promote the primary candidate to extended micro soak.
