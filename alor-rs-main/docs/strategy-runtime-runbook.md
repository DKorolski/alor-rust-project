# Strategy Runtime Runbook

**Правило №1:** все актуальные TOML-конфиги лежат в `./configs/`. Другие `.toml` в репозитории считаются legacy.

## Config file path
- `./configs/runtime.paper.toml` — paper режим (без live ордеров).
- `./configs/runtime.live.toml` — live режим (реальные ордера, включать осознанно).
- `./configs/runtime.replay.toml` — replay через `strategy_runtime_runner`.

## Run command
```bash
RUST_LOG=info,strategy_runtime=info \
cargo run -p strategy-runtime --bin strategy_runtime_runner -- \
  --config ./configs/runtime.paper.toml
```

## 8) Health endpoints

В runtime добавлен встроенный health-server (`[runtime.health]`):

```toml
[runtime.health]
enabled = true
listen_addr = "127.0.0.1:8091"
expose_metrics = false
```

Также поддержаны env overrides:
- `RUNTIME_HEALTH_ENABLED`
- `RUNTIME_HEALTH_LISTEN_ADDR`

### 8.1 Проверка liveness
```bash
curl -sf http://127.0.0.1:8091/liveness
```

### 8.2 Проверка readiness
```bash
curl -sf http://127.0.0.1:8091/readiness
```

`/readiness` возвращает:
- `200 OK`, если `live_guard=ALLOWED`.
- `503 Service Unavailable`, если `live_guard=BLOCKED`.

Ключевые поля:
- `gateway.health_age_sec` — «возраст» последнего health payload gateway.
- `live_guard_reasons` — причины блокировки (`gateway_health_stale`, `phase=...`, и т.д.).
- `scheduler.state` — состояние торговой сессии (`Open`, `Weekend`, `OutsideSession`, `Break1`, `Break2`).
- Если расписание не задано, runtime возвращает `scheduler.state=Unconfigured`, `scheduler.now_local=unknown`, `scheduler.note=trading_periods missing`.

Для совместимости можно использовать `GET /healthz` (алиас `/readiness`).
