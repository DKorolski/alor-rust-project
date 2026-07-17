# MOEX Early Session Shadow Deployment - 2026-07-17

## Purpose

Deploy isolated `legacy09` and `canonical07` shadow contours for the MOEX early-session schedule change.

The goal is to collect 5-10 sessions of diagnostics before deciding whether live micro should move from the legacy 09:00 model clock to the canonical 07:00 model clock.

## Local commit

- Commit: `0d445d0 Add MOEX early-session shadow contours`
- Runtime image built on VPS: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260717-moex-early-shadow-0d445d0`
- Gateway image reused: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-20260618-oauth-68d1cd1`

## VPS stacks

All stacks are shadow-only and isolated from live Redis/state:

- `trading-moex-early-shadow-ri`
- `trading-moex-early-shadow-usdrubf`
- `trading-moex-early-shadow-imoexf`

Each stack has:

- one Redis container;
- one market-data gateway configured with `session_start = "07:00:00"`;
- two runtimes: `runtime-shadow09` and `runtime-shadow07`.

## Safety checks

Runtime configs use:

- `trade_mode = "paper"`
- `allow_live_orders = false`
- `allow_paper_orders = false`
- `require_gateway_ready = false`

Additional safety guard:

- Gateway command streams are blackhole streams and do not match runtime command streams.
- Runtime command streams were absent after startup.
- Gateway blackhole command streams had length `0` after startup.
- Existing live micro stacks were not recreated or restarted during this deployment.

## Initial Redis/state check

Initial bar stream sizes after startup:

- RI: `md.bars.7502MIW.RIU6.10m.moex_early_session` - `870`
- USDRUBF: `md.bars.7502MIW.10m.moex_early_session` - `765`
- IMOEXF: `md.bars.7502MIW.10m.moex_early_session` - `876`

Runtime state streams were created for all six shadow runtimes.

RI decision journals were produced under:

- `/opt/trading-moex-early-shadow-ri/volumes/reports/moex_early_session_ri_decisions_legacy09.jsonl`
- `/opt/trading-moex-early-shadow-ri/volumes/reports/moex_early_session_ri_decisions_canonical07.jsonl`

USDRUBF and IMOEXF observations are primarily available through runtime logs and runtime state streams.

## IMOEXF riskgate seed

The IMOEXF shadow stacks imported the seed ledger and were then switched to `normal_append` mode.

Observed state after switch:

- ledger rows: `180`
- `seed_loaded = true`
- `last_finalized_session_date = 2026-04-21`
- `rolling_sum_lb120 = 161.90000000000012`
- `mr_enabled_current_session = true`
- `mr_enabled_next_session = true`
- `current_shadow_session_date = 2026-07-17`

Follow-up check:

- Verify after the next completed regular session that IMOEXF appends a new riskgate ledger row in `normal_append` mode.

## Observation plan

1. Observe at least 5 sessions before preliminary review.
2. Do not promote live micro to canonical07 before at least 10 sessions unless there is a separate risk decision.
3. Compare `legacy09` vs `canonical07` by signal timing, component attribution, exits, and live-vs-shadow drift.
4. Keep live micro unchanged while shadow diagnostics are collected.
