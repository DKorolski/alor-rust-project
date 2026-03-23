# DevOps Runbook: paper launch (gateway + runtime)

Документ для быстрого старта в pre-prod/paper режиме.

## 1) Подготовка окружения

Создайте локальный файл `.env.preprod` на основе `docs/preprod-env.example` и заполните секреты (минимум `ALOR_REFRESH_TOKEN`).

Загрузите переменные в текущую shell-сессию:

```bash
set -a
source ./.env.preprod
set +a
```

Проверка обязательных переменных:

```bash
echo "$ALOR_GATEWAY_CONFIG"
echo "$ALOR_STACK_NAME"
echo "$ALOR_REFRESH_TOKEN" | wc -c
```

Рекомендация для диагностических запусков:

- задавайте `ALOR_STACK_NAME` явно, например `sessiongap` или `hybrid`;
- это позволит видеть одинаковое имя стека в `/readiness`, `cws_limit_send` и `cws_transport_failure`, а не только `HOSTNAME`.

## 2) Запуск Redis (если не запущен)

Локально:

```bash
redis-server
```

Проверка:

```bash
redis-cli PING
```

Ожидается: `PONG`.

## 3) Запуск gateway (terminal #1)

Рекомендуемая команда:

```bash
ALOR_STACK_NAME=${ALOR_STACK_NAME:-sessiongap} \
RUST_LOG=info,alor_gateway::supervisor=info,alor_gateway::ws_hub=info \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.toml \
  --redis-url redis://127.0.0.1/
```

Health проверка gateway:

```bash
curl -sS http://127.0.0.1:8081/liveness
curl -sS http://127.0.0.1:8081/readiness
```

## 4) Запуск strategy-runtime в режиме paper (terminal #2)

Запуск с paper-конфигом:

```bash
RUST_LOG=info,strategy_runtime=info \
cargo run -p strategy-runtime --bin strategy_runtime_runner -- \
  --config ./configs/runtime.paper.toml
```

Health проверка runtime:

```bash
curl -sS http://127.0.0.1:8091/liveness
curl -sS http://127.0.0.1:8091/readiness
```

## 5) Базовая диагностика потоков Redis

Проверить, что бары идут в stream:

```bash
redis-cli XLEN md.bars.7502T0U.1m
redis-cli --raw XREVRANGE md.bars.7502T0U.1m + - COUNT 1
```

Проверить runtime-state stream:

```bash
redis-cli XLEN runtime.state.session_gap_standalone.live.7502T0U
```

> Подставьте фактические stream names из вашего `runtime.paper.toml`.

## 6) Kubernetes probes (рекомендация)

Используйте `/liveness` строго как liveness/startup, а `/readiness` — как readiness.

```yaml
startupProbe:
  httpGet:
    path: /liveness
    port: 8081

livenessProbe:
  httpGet:
    path: /liveness
    port: 8081

readinessProbe:
  httpGet:
    path: /readiness
    port: 8081
```

Пояснение:
- `liveness` проверяет, что процесс жив.
- `readiness` проверяет, что сервис готов к трафику и может возвращать `503`, пока не готов.

## 7) Аккуратная остановка

Остановите runtime и gateway через `Ctrl+C` (или SIGTERM в orchestrator).

После остановки можно проверить, что процессы завершились, и при необходимости собрать postmortem:

```bash
curl -sS http://127.0.0.1:8081/readiness || true
curl -sS http://127.0.0.1:8091/readiness || true
```

## 8) Failure-Matrix Status (актуально на сейчас)

Текущий статус выполнения отказных сценариев:

- `FT-01` terminal cancel — `PASS`
- `FT-02A` insufficient funds — `PASS`
- `FT-02B` price out of range — `PASS`
- `FT-02G` BookOrCancel immediate-exec reject — `PASS`
- `FT-02F` invalid price step — `PARTIAL` (недетерминированно как broker reject)
- `FT-03` publish failure around runtime command/state publish — `PASS`
- `FT-04` stale health -> runtime blocked — `PASS`

Подробности и артефактные ожидания:

- `docs/failure-test-matrix.md`
- `docs/AUDIT_AND_ROADMAP_GATEWAY_RUNTIME.md`

Примечание для воспроизводимости `FT-03`:

- используется тестовый env-хук (test-only):
  - `RUNTIME_ENABLE_TEST_HOOKS=true`
  - `RUNTIME_TEST_DELAY_BEFORE_PUBLISH_MS=<ms>`
