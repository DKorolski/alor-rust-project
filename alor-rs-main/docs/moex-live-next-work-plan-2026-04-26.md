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

Status: `GAP_FLATTEN_ASSERT_ADDED / REPLAY_PARITY_IN_PROGRESS`.

The model is not considered broken. The remaining required blocker is
replay/runtime parity around Backtrader next-bar fill semantics versus the Rust
bar/event no-overnight safety guard. A stricter runtime timer/event-loop hook is
useful, but is explicitly a follow-up `nice to have`, not a blocker for the
main patch line.

Required before runtime promotion:

- add or adapt a Rust `10m` replay harness;
- implement the regular-weekday session policy;
- ensure Saturday/Sunday bars are audit-visible but non-tradable;
- ensure Monday anchors use Friday or the latest earlier regular weekday;
- add a BO gap-flatten assert for `force_exit_time = 23:30` (implemented in
  `hybrid_replay --assert-gap-flatten`; full candidate parity still pending);
- reproduce the primary candidate against package reference files;
- write a discrepancy note for any timestamp/fill-price differences.

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
