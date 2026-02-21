# Alor Gateway Runbook

**Правило №1:** все актуальные TOML-конфиги лежат в `./configs/`. Другие `.toml` в репозитории считаются legacy.

## Config file path
- `./configs/gateway.live.toml` — основной gateway конфиг (live/paper runtime uses same gateway feed).
- `./configs/gateway.example.toml` — минимальный шаблон для первичной настройки.

## Run command
```bash
RUST_LOG=info,alor_gateway::services::command_consumer=debug,alor_gateway::transport_redis=debug \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.live.toml \
  --redis-url redis://127.0.0.1/
```

## 3) Health endpoints

Gateway поднимает HTTP health server:
- `GET /liveness` — процесс жив (ожидается `200 OK`).
- `GET /readiness` — JSON со статусом готовности (`readiness`) и диагностикой фаз.

> Примечание: endpoint `/startup` в текущей реализации отсутствует. Для startup-probe используйте `/liveness` до появления отдельного `/startup`.

Критичные поля `/readiness`:
- `gateway_phase` (`SyncingHistory`, `Reconnecting`, `SyncingGap`, `LiveReady`),
- `ws_connected`, `cws_authorized`,
- `last_bar_age_sec`, `ws_last_rx_age_sec`,
- `active_subscriptions_count` vs `desired_subscriptions_count`,
- `ws_reconnects_total`, `backpressure_lagged`.

```bash
curl -sf http://127.0.0.1:8081/readiness
```
---

## 4) Типовые сбои и что делать

### 4.1 Reconnect loop
Симптомы:
- `gateway_phase=Reconnecting`, рост `ws_reconnects_total`.

Действия:
1. Проверить сеть/доступность `ws_url`.
2. Проверить корректность токена (`refresh_token`) и OAuth.
3. Убедиться, что clock sync хоста в норме (NTP).
4. Включить `RUST_LOG=debug` и проверить первичную ошибку reconnect.

### 4.2 Stale guid / resubscribe
Симптомы:
- подтверждения/события не мапятся на ожидаемые подписки/команды, частые resubscribe.

Действия:
1. Проверить, нет ли дублирующего консьюмера на тех же streams/portfolio.
2. Перезапустить gateway и runtime согласованно.
3. Снять readiness snapshot до/после перезапуска (для postmortem).

### 4.3 Нет данных баров
Симптомы:
- `last_bar_age_sec` растёт, runtime не торгует.

Действия:
1. Проверить `symbols` и соответствие тикеров.
2. Проверить `active_subscriptions_count`.
3. Проверить публикацию в Redis stream `bars`.
4. Убедиться, что downstream не блокирует event publisher (backpressure flags).

---

## 5) Логи/метрики
Минимальный набор сигналов:
- phase transitions gateway (`SyncingHistory -> LiveReady`),
- reconnect counters,
- event publisher degradation (`event_sink_degraded`, retries/timeouts/fails),
- command consumer health (`command_consumer_alive`, poll timestamp).

Рекомендуемый запуск:
```bash
RUST_LOG=info,alor_gateway=debug \
cargo run -p alor-gateway --bin alor_gateway_runner -- --config ./configs/gateway.live.toml
```

---

## 6) Preflight checklist
- [ ] Redis доступен, stream names валидны.
- [ ] Token/portfolio/symbols проверены.
- [ ] Health endpoint доступен по `health_listen_addr`.
- [ ] Readiness достигает `LiveReady` до разрешения live-ордеров в runtime.
