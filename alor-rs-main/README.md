# Alor Trading Workspace

Rust workspace для запуска связки:
- `alor-gateway` — подключение к Alor WS/CWS, нормализация событий, публикация в Redis streams;
- `strategy-runtime` — исполнение торговой стратегии в режимах `paper` / `live` / `replay`;
- `alor-protocol` и `alor-types` — общие протоколы, модели и типы.

## Состав репозитория

- `alor-types/` — доменные типы (включая trading periods / scheduler структуры).
- `alor-protocol/` — протокольные модели и сериализация для Alor.
- `alor-gateway/` — gateway-процесс (WS/CWS + Redis transport + health).
- `strategy-runtime/` — runtime-процесс стратегии + replay/backtest утилиты.
- `configs/` — **единственная актуальная** папка с TOML-конфигами.
- `docs/` — runbooks и эксплуатационные инструкции.

## Правило №1 (конфиги)

Все актуальные TOML-конфиги лежат в `./configs/`.
Любые TOML вне `./configs/` считаются legacy и не должны использоваться для запуска.

## Быстрый старт

| Сценарий | Gateway | Runtime |
|---|---|---|
| SessionGap Live | `configs/gateway.sessiongap.live.7502MIW.toml` | `configs/runtime.sessiongap.live.7502MIW.toml` |
| Hybrid Live | `configs/gateway.hybrid.live.7502SN6.toml` | `configs/runtime.hybrid.live.7502SN6.toml` |
| Hybrid Paper | `configs/gateway.hybrid.live.7502SN6.toml` | `configs/runtime.hybrid.paper.7502SN6.toml` |
| Replay | — | `configs/runtime.replay.toml` |

### 1) Запуск Gateway

```bash
RUST_LOG=info,alor_gateway::services::command_consumer=debug,alor_gateway::transport_redis=debug \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.toml \
  --redis-url redis://127.0.0.1/
```

### 2) Запуск Runtime (paper)

```bash
RUST_LOG=info,strategy_runtime=info \
cargo run -p strategy-runtime --bin strategy_runtime_runner -- \
  --config ./configs/runtime.hybrid.paper.7502SN6.toml
```

### 3) Health-check Runtime

```bash
curl -sf http://127.0.0.1:8091/liveness
curl -sf http://127.0.0.1:8091/readiness
```

## Trading scheduler

`strategy-runtime` и `alor-gateway` используют единое расписание торговых окон.

Базовое правило:
- использовать top-level секцию `[trading_periods]` в runtime/gateway конфиге;
- `[strategy.trading_periods]` — только для явного override.

Пример:

```toml
[strategy]
max_silence_bars_sec = 900

[trading_periods]
session_start = "09:00:00"
session_end = "23:49:00"
break_start_1 = "14:00:00"
break_end_1 = "14:05:00"
break_start_2 = "18:50:00"
break_end_2 = "19:05:00"
weekends_off = true
timezone_offset_hours = 3
```

Если расписание отсутствует полностью, runtime readiness возвращает:
- `scheduler.state = "Unconfigured"`
- `scheduler.now_local = "unknown"`
- `scheduler.note = "trading_periods missing"`

## Документация

- [Strategy Runtime Runbook](docs/strategy-runtime-runbook.md)
- [Alor Gateway Runbook](docs/alor-gateway-runbook.md)
- [Session Gap B2 Runbook](docs/session-gap-b2-runbook.md)
- [Market Buy And Close Diagnostic Runbook](docs/market-buy-and-close-diagnostic-runbook.md)
- [Replay / Backtest Guide](docs/replay-backtest-guide.md)
- [State and Restarts](docs/state-and-restarts.md)
- [Hybrid Stage-2 Contract Freeze](docs/hybrid-stage2-contract-freeze.md)

## Итоги аудита

Проведен критический аудит `gateway + strategy-runtime` и выполнена первая волна failure-сценариев.

- Core-контур (live/paper/reconnect) подтвержден практическими прогонами.
- Ключевые сценарии `FT-01..FT-04` покрыты и зафиксированы (включая `terminal cancel`, broker rejects, `stale health -> BLOCKED`, publish-failure path).
- Подробные результаты, риски и roadmap:
  - [Audit and Roadmap](docs/AUDIT_AND_ROADMAP_GATEWAY_RUNTIME.md)
  - [Failure Test Matrix](docs/failure-test-matrix.md)

## Тесты

Быстрые команды:

```bash
cargo test -p strategy-runtime --lib
cargo test -p alor-gateway --lib
```

Hybrid parity (one command):

```bash
cargo run -p strategy-runtime --bin hybrid_replay -- \
  --bundle-dir ../../pre_rust_handoff/replay_data/imoexf_2023_2026 \
  --split golden \
  --out-dir /tmp/hybrid_out \
  --check \
  --strict
```

Exit codes:

- `0` PASS
- `2` DIFF (parity mismatch against expected artifacts)
- `1` ERROR (validation/runtime/parsing failure)

Artifacts written to `--out-dir`:

- `actual_actions_<split>.csv`
- `actual_trades_<split>.csv`
- `actual_summary_<split>.json`
- `parity_report_<split>.json`

CI regression guard (mini-golden):

```bash
cargo test -p strategy-runtime --test e2e_hybrid_golden
```

StopLimit broker smoke (no strategy logic):

```bash
cargo run -p alor-gateway --bin stop_limit_smoke -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.toml \
  --dry-run
```

Live (single create + single delete):

```bash
cargo run -p alor-gateway --bin stop_limit_smoke -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.toml \
  --live-confirm \
  --symbol USDRUBF \
  --side buy \
  --qty 1 \
  --trigger-price 1000 \
  --limit-price 999
```

StopOrders WS smoke (subscribe + verify status lifecycle for created stop order):

```bash
cargo run -p alor-gateway --bin stop_orders_ws_smoke -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.toml \
  --live-confirm \
  --symbol USDRUBF \
  --side buy \
  --qty 1 \
  --trigger-price 1000 \
  --limit-price 999
```

Runtime/Event bus contract note:

- `streams.orders` now carries both `Order` and `StopOrder` envelopes.
- Consumers must route by `message_type` (`order` vs `stop_order`).
- Unknown `message_type` must be ignored (or sent to DLQ), not treated as hard failure.

Интеграционные тесты gateway (`alor-gateway/tests/redis_transport.rs`) используют `testcontainers` и требуют установленный Docker.

## Лицензия и дисклеймер

Проект распространяется по лицензии MIT (см. `LICENSE`).

ПО предоставляется "как есть" и предназначено для инженерных/исследовательских задач. Использование в торговле и финансовых системах осуществляется на ваш риск; авторы и контрибьюторы не несут ответственности за финансовые потери или косвенный ущерб.
