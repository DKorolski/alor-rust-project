# IMOEXF Hybrid MR + BO Handoff (2026-04-26)

## Status

Current verdict: `PENDING_BO_GAP_FLATTEN_PARITY_CHECK`.

The updated `IMOEXF` hybrid package is promising enough for an extended micro
soak candidate, but it should not be promoted as a runtime replacement until a
dedicated `10m` replay parity pass is complete. The remaining BO item is a
replay/runtime fill-timing assertion, not evidence that the model logic is
broken.

## Recommended Candidate

Primary:

```text
hybrid_mr_riskgate_high180_lb120__bo_new_k053
```

Shadow/canary:

```text
hybrid_mr_adaptive_lb120_mean__bo_new_k053
```

## What Changed Versus Current Runtime Baseline

BO block:

```text
bo_k = 0.53
bo_wait_hours = 4.0
bo_stop1_range = 0.35
bo_stop2_range = 0.70
bo_min_range = 1.01
bo_min_range_mode = absolute
bo_big_move_threshold = 0.025
bo_eod_exit_time = 23:30
```

MR block:

```text
anchor_policy = regular_weekday_anchor
k_long = 0.085
k_short = 0.090
range_gate = 0.005..0.050
stop_loss_mult = 7.0
max_hold_minutes = 180
entry_window = 09:00..11:59
trade_weekends = false
trade_mondays = true
risk_gate = trailing 120-day positive shadow PnL
```

Hybrid behavior:

```text
single-position merge
MR priority over BO on overlapping signals
close-bar evaluation on 10m bars
```

Weekend/session policy:

```text
raw_data_policy = keep weekend bars for audit
tradable_sessions = Monday-Friday only
anchor_policy = previous regular weekday session
trade_weekends = false
trade_mondays = true
bo_exclude_weekends = true
```

The 2026 weekend-session issue is part of the handoff contract. Saturday/Sunday
bars must not generate trades, become Monday's previous-session anchor, or update
MR/BO previous close/range state. Monday should use Friday as the anchor when
Friday is the latest regular weekday session, otherwise the latest earlier
regular weekday. See `WEEKEND_SESSION_POLICY.md` in the artifact mirror.

Session audit note: the research artifact passes MR entry-window and MR/BO
no-overlap checks, but the current Backtrader-style BO artifact has one weekend
carry and nine non-same-day BO exits because EOD `close()` can fill on the next
available bar. Runtime/replay parity should therefore include a BO gap-flatten
assert before promotion.

## Required Developer Work

1. Add or adapt a Rust replay path for this exact `10m` contract.
2. Implement the weekend/session policy exactly before comparing parity.
3. Reproduce the primary candidate against the package reference files.
4. Confirm entry/exit parity, owner/side sequence, daily PnL, and stress metrics.
5. Only after replay parity, freeze a runtime config profile for extended micro soak.

## Package Location

Research package:

```text
analiz_alpha_si/imoexf_hybrid_mr_bo_handoff_2026_04
```

Docs-local mirror:

```text
docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts
```

Key files:

- `FROZEN_MODEL_SPEC.md`
- `WEEKEND_SESSION_POLICY.md`
- `IMOEXF_MR_BO_SESSION_AUDIT.md`
- `IMPLEMENTATION_TODO.md`
- `REPLAY_CONTRACT.md`
- `imoexf_hybrid_mr_bo_manifest.csv`

## Acceptance Gate

Do not judge this branch by raw `1m` runtime-like behavior yet. The reference
contract is `10m`, close-bar based, and uses a no-overlap hybrid merge. If the
runtime will continue to consume `1m` bars, either feed/aggregate to `10m` for
this strategy or run a separate explicit `1m` translation study.

Promotion to extended micro soak requires:

- Rust replay output compared to `replay_trades.csv`.
- Daily PnL compared to `replay_daily.csv`.
- Summary metrics compared to `replay_expected_summary.csv`.
- A written discrepancy note if any timestamps or fill prices differ.

## Operational Watchpoints

- MR remains phase-sensitive even after risk gating.
- BO is the cleaner carry-forward layer.
- Weekend data must remain audit-visible but non-tradable for this package.
- Monday anchor behavior must use previous regular weekday data.
- BO must not carry across weekend/non-tradable gaps under the frozen contract.
- Existing hybrid protective-order/runtime fragility should be treated as an
  operational gate separate from model quality.
