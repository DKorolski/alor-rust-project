# MOEX USDRUBF Tuning Note (2026-04-26)

## Scope

This note locks the immediate tuning direction for:

- `session_gap_standalone` (USDRUBF)
- `alor_usdrubf_hybrid` (USDRUBF)

It is intentionally narrow: keep baseline logic stable, test only targeted challengers.

Related MR comparison memo:

- `docs/alor-usdrubf-mr-model-comparison-2026-04-27.md`

## 1) SessionGap USDRUBF

### Keep as baseline runtime contract

- feed: `10m`
- `signal_minute = 50`
- `wait_hours = 3`
- no overnight; forced flatten around `23:30` with fallback if late bar is missing
- keep SL unchanged for now:
- `k_sl_long = 0.68`
- `k_sl_short = 0.65`

### Targeted challenger (TP short only)

- baseline: `k_tp_long = 0.28`, `k_tp_short = 0.28`
- challenger A: `k_tp_long = 0.28`, `k_tp_short = 0.50`
- challenger B: `k_tp_long = 0.28`, `k_tp_short = 0.60`

### Evidence from existing scan (window `2026-02-12 .. 2026-04-22`)

Source: `analiz_alpha_si/usdrubf_sessiongap_audit_2026_04/sessiongap_tp_scan_summary.csv`

- baseline `0.28/0.28`:
- `test`: return `2.31%`, Sharpe `3.22`, MaxDD `0.96%`
- `forward`: return `1.13%`, Sharpe `1.15`, MaxDD `2.89%`
- challenger `0.28/0.50`:
- `test`: return `2.80%`, Sharpe `3.48`, MaxDD `1.38%`
- `forward`: return `2.14%`, Sharpe `1.91`, MaxDD `3.01%`
- challenger `0.28/0.60`:
- `test`: return `3.44%`, Sharpe `4.04`, MaxDD `1.31%`
- `forward`: return `2.75%`, Sharpe `2.32`, MaxDD `3.00%`

Decision read: short-side TP widening is the best first change; avoid broad retune.

## 2) alor-USDRUBF Hybrid

### Keep as baseline

- `bo_k = 0.45` (baseline)
- adaptive BO (`60d/90d`) stays challenger-only, not production default

### Targeted MR short recut challenger

- challenger: `mr_k_short = 0.035`
- preserve MR shape and execution contract
- keep `mr_force_exit_time = 11:50` as in the current baseline

### Why this is next

- It is a narrow change in the weakest sensitive block (MR short trigger), with minimal risk of unintended drift in BO behavior.
- This keeps parity/replay comparability with current runtime architecture.

### Challenger check result

Source:

- `analiz_alpha_si/moex_micro_live_audit_2026_04/usdrubf_hybrid_mr035_challenger_summary.csv`
- `analiz_alpha_si/moex_micro_live_audit_2026_04/usdrubf_hybrid_mr035_challenger_report.md`

Baseline vs `mr035_exit1200` research check:

- `full`: baseline `54.19%`, challenger `57.18%`; MaxDD improves from `3.20%` to `2.46%`
- `test_30`: baseline `11.63%`, challenger `12.97%`; MaxDD improves from `2.05%` to `1.83%`
- `recent_forward`: baseline `2.37%`, challenger `2.00%`; MaxDD worsens slightly from `1.70%` to `1.78%`

Decision read: `mr_k_short = 0.035` is a valid challenger, not a production-default replacement yet. For the immediate runtime challenger keep the existing `11:50` forced exit so the next validation changes only one core knob. The `12:00` exit remains research context, not the next live profile.

## 3) Execution order for next pass

1. Run SessionGap baseline vs TP-short challengers only (`0.50`, `0.60`) on the same 10m replay window.
2. Run alor-USDRUBF Hybrid baseline vs `mr_k_short=0.035`, `mr_force_exit_time=11:50` challenger (same data split and cost assumptions).
3. Compare by: return, Sharpe, MaxDD, trade count, and day-level concentration.
4. Promote only if uplift is visible on both `test` and `forward` and does not materially worsen drawdown profile.

## 4) Guardrails

- No search-space expansion in this pass.
- No simultaneous retune of multiple core knobs.
- Keep runtime/replay parity contract fixed while evaluating challengers.
