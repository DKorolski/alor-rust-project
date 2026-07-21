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

## RI shadow path journaling patch - 2026-07-21

Reason:

- The initial RI shadow journal was safe but too incremental for economic
  comparison: a decision accepted by a partial replay could remain in
  `shadow_recorded` even after a later replay found an earlier overlapping
  trade.

Patch:

- Commit: `9f7eafb Harden RI early-session shadow path journaling`
- Runtime image built on VPS:
  `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260721-ri-shadow-path-9f7eafb`

Rollout:

- Stopped only `runtime-shadow09` and `runtime-shadow07` in
  `/opt/trading-moex-early-shadow-ri` before the `07:00` session start.
- Kept RI shadow Redis and gateway running so market-data bars continued to
  accumulate.
- Recreated only `runtime-shadow09` and `runtime-shadow07` on the new runtime
  image.
- Gateway image/config and all live micro stacks were unchanged.

Verification:

- `runtime-shadow09` and `runtime-shadow07` healthy after recreate.
- Runtime image resolved to
  `manual-20260721-ri-shadow-path-9f7eafb` for both services.
- Command streams remained empty:
  `cmd.orders.7502MIW.shadow.ri_author41_42.shadow07 = 0`,
  `cmd.orders.7502MIW.shadow.ri_author41_42.shadow09 = 0`,
  `cmd.orders.7502MIW.moex_early_shadow.gateway_blackhole.ri = 0`.
- Logs showed `ri_shadow_path_active` and `ri_shadow_path_superseded`; the
  2026-07-20 canonical07 overlap case was marked with a superseded 09:00 long
  and active 07:40 short.

## Paper trading-window simulation hardening - 2026-07-21

Reason:

- IMOEXF shadow paper simulation could advance a BO entry during Break2
  (`18:50:00`-`19:04:59`) even though live correctly drops such broker intents
  before emission.
- This created a false virtual double-entry sequence around `19:00`/`19:10`
  and could surface later as a misleading `broker_residual_emergency_exit`
  (`previous_qty=6`, `broker_qty=12`) in shadow logs.

Patch:

- Commit: `1ac8f8e Harden paper shadow trading-window simulation`
- Runtime image built on VPS:
  `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260721-shadow-window-1ac8f8e`

Rollout:

- Recreated only `runtime-shadow09` and `runtime-shadow07` in all three
  early-session shadow stacks:
  `trading-moex-early-shadow-ri`,
  `trading-moex-early-shadow-usdrubf`,
  `trading-moex-early-shadow-imoexf`.
- Kept all shadow Redis and gateway containers running.
- Live micro stacks were not recreated or restarted.
- Backups were written as `docker-compose.yml.bak-20260721-shadow-window` in
  each shadow stack directory before image tag replacement.

Verification:

- All six shadow runtime containers were healthy after recreate and used
  `manual-20260721-shadow-window-1ac8f8e`.
- Shadow command streams remained empty for RI, USDRUBF, and IMOEXF, including
  gateway blackhole streams.
- Runtime consumer groups had `lag=0` and `pending=0` after restart.
- Fresh logs contained only expected startup warnings for pre-existing bar
  stream history; no fresh `ERROR`, `panic`, or new
  `broker_residual_emergency_exit` was observed after rollout.

Follow-up:

- On the next Break2 BO candidate, verify that shadow logs contain
  `paper_intent_dropped_by_trading_window` and that no synthetic paper position
  is opened before the `19:05` market reopen.
