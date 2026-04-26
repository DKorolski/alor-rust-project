# MOEX Live Next Work Plan (2026-04-26)

## Goal

Freeze the current research and observation artifacts before code work, then
move through the next implementation steps with narrow, auditable changes.

## Workstream A: USDRUBF SessionGap

Status: `CHALLENGER_CONFIGS_READY`.

Baseline stays unchanged:

- feed: `10m`
- `signal_minute = 50`
- `wait_hours = 3`
- forced flatten around `23:30`
- `k_tp_long = 0.28`
- `k_tp_short = 0.28`
- `k_sl_long = 0.68`
- `k_sl_short = 0.65`

Challengers:

- `runtime.sessiongap.live.7502MIW.challenger_tp_short_050.toml`
- `runtime.sessiongap.live.7502MIW.challenger_tp_short_060.toml`

Validation gate:

- replay baseline vs challengers on the same `10m` window;
- compare return, Sharpe, MaxDD, trade count, and day concentration;
- no production default change before a positive validation read.

## Workstream B: USDRUBF alor Hybrid

Status: `NARROW_CHALLENGER_CONFIG_READY`.

Baseline stays unchanged:

- `bo_k = 0.45`
- adaptive BO stays challenger-only;
- `mr_force_exit_time = 11:50`.

Immediate challenger:

- `mr_k_short = 0.035`
- `mr_force_exit_time = 11:50`

Config:

- `runtime.alor_usdrubf.live.7502T0U.challenger_mr035.toml`

Validation gate:

- validate the `mr_k_short` change separately from any force-exit timing
  change;
- keep live size at `1`;
- use clean runtime state only after flat/no-working-orders confirmation.

## Workstream C: IMOEXF Hybrid MR + BO

Status: `LIVE_HIGH180_READY / RISK_GATE_ENFORCEMENT_CODE_READY / READY_FOR_SHADOW_VALIDATION`.

The model is not considered broken. The remaining required blocker is
replay/runtime parity around Backtrader next-bar fill semantics versus the Rust
bar/event no-overnight safety guard. A stricter runtime timer/event-loop hook is
useful, but is explicitly a follow-up `nice to have`, not a blocker for the
main patch line.

Runtime integration review handoff:

- `docs/imoexf-primary-runtime-integration-review-handoff-2026-04-26.md`

First runtime-integration slice completed:

- shared High180 MR module extracted from replay;
- High180 MR branch wired into `HybridIntradayRuntimeStrategy` behind
  `mr_variant = "high180"`;
- risk-gate session ledger primitives, seed CSV parser, startup reconciliation
  rules, next-session gate calculation, and deterministic startup planner added;
- runtime-facing risk-gate store helper added for ledger/state read and guarded
  startup-artifact persistence;
- high-level startup-store path added for configured seed load, Redis ledger/state
  read, deterministic plan, and guarded artifact persistence;
- hybrid profile/risk-gate config fields added and covered by TOML tests;
- optional model-session guard wired before live model-state updates;
- live adapter currently fails fast if active risk-gate modes are configured
  before the ledger/import flow is finished.

Required before runtime promotion:

- keep the implemented Rust `10m` replay harness and primary profile
  (`hybrid_replay --profile imoexf_primary_riskgate_k053`) as the review path;
- keep the implemented BO gap-flatten assert for `force_exit_time = 23:30`
  (`hybrid_replay --assert-gap-flatten`);
- keep the frozen IMOEXF model feed contour before any replay state update:
  Monday-Friday regular tradable bars only, `09:00..23:49`. Raw/audit
  pre-session/service bars such as `08:50` must remain outside MR midpoint,
  BO anchor/level, risk-gate, entry/exit, and parity state.
- use `scripts/build_imoexf_filtered_bundle.py` to regenerate the official
  filtered parity bundle from raw/audit data through `2026-04-21` before the
  final replay read;
- use `scripts/run_imoexf_primary_parity_review.py` as the one-command review
  path for bundle build, Rust replay, diagnostics, and consolidated report
  generation;
- publish one final parity report that separates MR signal drift, BO signal
  drift, and BO execution-contract drift.
  `docs/imoexf-primary-parity-review-report-2026-04-26.md` is the consolidated
  review report, and `docs/imoexf-primary-parity-discrepancy-2026-04-26.md`
  records the current layer split and the promotion gate;
- use `scripts/imoexf_primary_parity_diff.py` or the same logic promoted into
  Rust report output to keep signal drift separate from BO execution-contract
  drift;
- use `scripts/imoexf_mr_residual_diagnostic.py` for the MR residual read. It
  reproduces the current diagnosis: saved-source MR drift is explained by old
  service-bar midpoint effects, calendar-zero riskgate, and one BO gap-flatten
  interaction;
- use `scripts/imoexf_bo_execution_contract_diagnostic.py` for the BO read. It
  normalizes saved source timestamps by the next-bar offset and shows that
  entry-signal agreement is high (`361 / 370`) even though fill-level exact
  parity is zero under Backtrader-vs-Rust execution semantics;
- keep the MR risk gate aligned with the source package. The first Rust scaffold is
  documented in
  `docs/imoexf-primary-riskgate-replay-progress-2026-04-26.md`; the gate and
  high180 midpoint semantics are now source-aligned. The saved source reference
  still carries old service-bar and calendar-zero riskgate semantics, so
  official parity needs a refreshed filtered replay bundle/reference before
  judging the final BO execution-contract drift.
- treat `riskgate_high180_lb120_seed_2026-04-26.csv` as a one-time bootstrap
  artifact only. After import, a runtime-owned regular-session ledger should be
  the source of truth for the 120-session MR gate. Runtime must expose explicit
  modes: `bootstrap_from_seed`, `normal_append`, and `rebuild_from_history`.
  The accepted storage contract is a Redis stream
  `runtime.riskgate.sessions.<strategy_id>.<profile_id>` plus a small
  materialized state key `runtime.riskgate.state.<strategy_id>.<profile_id>`;
  the ordinary strategy snapshot must not be the canonical gate ledger. Startup
  bootstrap import/read/write, daily runtime append, and enforced MR gate
  application are now wired into runtime. Operational use of `enforced` remains
  after shadow validation and review approval;
- decide explicitly whether Rust close-bar/no-overnight `bo_gap_flatten` is the
  accepted runtime contract. If accepted and final report is clean, IMOEXF can
  move to extended micro soak at size `1` with explicit MR/BO attribution
  monitoring.

Follow-up after main work:

- consider a runtime timer/event-loop hook so `23:30` flatten can fire even
  without any later bar/event; this should be scheduled after the core
  SessionGap, alor-USDRUBF, and IMOEXF parity/rollout work.

Primary candidate:

- `hybrid_mr_riskgate_high180_lb120__bo_new_k053`

Reference artifacts:

- `docs/imoexf-hybrid-mr-bo-handoff-2026-04-26.md`
- `docs/imoexf-hybrid-mr-bo-handoff-2026-04-artifacts/`

## Rollout Guardrails

- Do not bundle SessionGap, alor-USDRUBF, and IMOEXF logic changes in one live
  rollout.
- Roll out only while target account is flat and no working/stop orders remain.
- Use from-zero runtime state for validation stacks when a stateful runtime
  path changes.
- Keep live order size at `1` until patched micro validation is clean.
- Treat Redis retention and runtime-state cleanup as operational gates, not as
  model-quality signals.
