# Alor USDRUBF 7502T0U Smoke Checklist

Date: 2026-04-06

## Scope

This checklist is for controlled rollout of `alor_usdrubf_hybrid_v1` on `USDRUBF` with portfolio `7502T0U`.

## Related Docs

- `docs/alor-usdrubf-development-observations-2026-04-06.md`
- `docs/alor-usdrubf-local-bringup-report-2026-04-06.md`
- `docs/alor-usdrubf-live-hardening-tz-2026-04-06.md`
- `docs/alor-usdrubf-followup-review-memo-2026-04-06.md`

## Preflight (Before Smoke/Soak)

- run a clean diagnostic cycle first:
  - use `reset_state_on_start = true`
  - use a fresh `consumer_group` for runtime
  - use a fresh `streams.runtime_state` key for the run
  - use isolated stream namespace for bars/orders/trades/positions/acks/commands/health
- confirm no stale-tail symptoms in logs:
  - no startup stale entry attempts
  - no repeated `intent_dropped_*` + immediate state revert pattern
  - no unexpected duplicate request churn from historical tails
- proceed to smoke/canary/soak only after preflight is clean

### Supported startup profile (current)

- supported: clean-start profile only (flat account, isolated namespace, fresh consumer group and runtime-state stream),
- not yet proven: restart with open position and/or working orders/stop orders.

### Mandatory evidence to capture in next run

- bootstrap summary line,
- replay guard armed line,
- replay guard cleared line,
- first fresh live-origin bar marker,
- first `live_guard=ALLOWED`,
- first allowed entry,
- first broker-truth position transition.

Guard semantics to verify in logs:

- `live_ready` is cleared only by fresh `DataOrigin::Live` bar,
- fresh `history/history_gap/replay` bars must stay suppressed during startup gate.

## Config Files

- `configs/gateway.alor_usdrubf.live.7502T0U.toml`
- `configs/runtime.alor_usdrubf.paper.7502T0U.toml`
- `configs/runtime.alor_usdrubf.live.7502T0U.toml`

Gateway control contour must be aligned with active `sessiongap/hybrid` live stacks:

- `control_cws_mode = "action_scoped"`
- `action_scope_enable_create_limit = true`
- `action_scope_enable_delete_limit = true`
- `action_scope_enable_replace_limit = false`
- `action_scope_enable_exit = true`
- `action_scope_force_token_refresh_before_authorize = true`

## Safety Invariants

- quantity semantics in runtime: `size` means contracts count
- micro-soak default: `use_fixed_live_size = true`, `live_fixed_units = 1.0`
- no implicit multiplication by contract lot in order quantity mapping
- `tick_size = 0.01` for live MOEX routing

## Stage 1: Paper Smoke

- run gateway with `gateway.alor_usdrubf.live.7502T0U.toml`
- run runtime with `runtime.alor_usdrubf.paper.7502T0U.toml`
- verify:
  - gateway `Resolved config` confirms `control_cws_mode="action_scoped"`
  - health endpoint is ready
  - bars stream is consumed
  - strategy emits intents only in paper mode
  - no runtime errors around config parsing or state restore

## Stage 2: Live Micro-Soak

- switch runtime to `runtime.alor_usdrubf.live.7502T0U.toml`
- confirm before start:
  - `allow_live_orders = true`
  - `strategy_id = alor_usdrubf_hybrid_v1`
  - `strategy_kind = alor_usdrubf_hybrid`
  - `live_fixed_units = 1.0`
- verify in early session:
  - create/ack/order/position flow is healthy
  - requested live quantity is exactly `1`
  - no oversize incidents and no unexpected quantity transformations

## VPS rollout from `main` (Docker / GHCR; other stacks untouched)

Layout on VPS mirrors `sessiongap` / `trading-hybrid`: **отдельная директория** и **отдельный** `docker compose` project для USDRUBF, чтобы не трогать extended soak стеки.

- Compose project name (по именам контейнеров): **`alorusdrubf`** → `alorusdrubf-strategy-runtime-1`, `alorusdrubf-alor-gateway-1`.
- Каталог на хосте задайте по своей схеме (часто рядом с `/opt/trading-sessiongap`, `/opt/trading-hybrid`); в репозитории путь не захардкожен.

### Образы

После merge/push в `main` используйте тот GHCR image tag, который был выпущен и подтверждён зелёным CI/publish pipeline для целевого коммита.
Для rollout предпочтителен фиксированный tag, а не `latest`.

- `ghcr.io/dkorolski/alor-rust-project/strategy-runtime` — теги `latest`, `sha-<короткий_sha>`
- `ghcr.io/dkorolski/alor-rust-project/alor-gateway` — то же

Для фиксированного rollout возьмите **SHA коммита** с `main` и используйте соответствующий published tag (`sha-...`, `vps-...` или иной фактически опубликованный immutable tag, в зависимости от текущего release процесса).

### Шаги (только стек Alor USDRUBF)

1. Дождаться **зелёного** workflow на нужном коммите `main`.
2. На VPS: убедиться, что активен только один canonical compose project для USDRUBF.
3. Перейти в каталог compose-проекта `alorusdrubf`.
4. Бэкап `.env`:  
   `cp .env ".env.bak.$(date +%Y%m%d-%H%M%S)"`
5. Обновить теги образов:
   - если в `.env` один **`IMAGE_TAG`** (как в корневом `docker-compose.yml` репозитория): выставить `IMAGE_TAG=sha-<commit>`;
   - если на VPS раздельно **`RUNTIME_IMAGE_TAG`** / **`GATEWAY_IMAGE_TAG`** (как в runbook hybrid): обновить **runtime** обязательно; gateway — если менялся gateway-код в том же коммите (при одном `IMAGE_TAG` обновляются оба).
6. При необходимости синхронизировать **только** TOML из `alor-rs-main/configs/` для `7502T0U` в `configs/` этого стека (если в коммите менялись конфиги).
7. Применить **только этот** проект:  
   `docker compose pull`  
   `docker compose up -d`  
   (при необходимости `-p alorusdrubf` и `-f <path/to.compose.yml>` — как заведено на хосте).
8. Не менять каталоги **`sessiongap`** и **`trading-hybrid`** и их `.env`.

### Проверка после rollout

- `docker compose ps` в каталоге USDRUBF — оба сервиса up.
- Логи gateway: `Resolved config` с ожидаемым `control_cws_mode` и action-scope флагами (см. выше).
- Логи runtime: новые события с полем `action` (`position_transition`, `replay_guard_cleared`, `intent_emitted`, …), меньше спама по позиции.
- `readiness` gateway/runtime (как в `README-DEPLOY.md`).

### Rollback

- восстановить `.env` из бэкапа и повторить `docker compose pull && docker compose up -d` **только** для проекта `alorusdrubf`.

## Rollback

- stop runtime process first
- keep gateway running for diagnostics if needed
- switch runtime back to paper config
- preserve logs and runtime state key for incident analysis
