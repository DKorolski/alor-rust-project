# Alor Gateway Runbook

**Правило №1:** все актуальные TOML-конфиги лежат в `./configs/`. Другие `.toml` в репозитории считаются legacy.

## Config file path
- `./configs/gateway.sessiongap.live.7502MIW.toml` — gateway профиль для `session_gap`.
- `./configs/gateway.sessiongap.live.7502MIW.action-scoped.toml` — Phase 1 candidate профиль для `session_gap` с `action_scoped` только для `create/delete`.
- `./configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml` — Phase 2 candidate профиль для `session_gap` с action-scoped `entry` и `flatten` через `IntentClass`.
- `./configs/gateway.hybrid.live.7502SN6.toml` — gateway профиль для `hybrid`.
- `./configs/gateway.example.toml` — минимальный шаблон для первичной настройки.

## Run command
```bash
ALOR_STACK_NAME=${ALOR_STACK_NAME:-sessiongap} \
RUST_LOG=info,alor_gateway::services::command_consumer=debug,alor_gateway::transport_redis=debug \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.toml \
  --redis-url redis://127.0.0.1/
```

Для controlled candidate окна Phase 1 используйте отдельный config path, а не изменение базового live TOML:

```bash
ALOR_STACK_NAME=${ALOR_STACK_NAME:-sessiongap} \
RUST_LOG=info,alor_gateway=debug \
cargo run -p alor-gateway --bin alor_gateway_transport_runner -- \
  --config ./configs/gateway.sessiongap.live.7502MIW.action-scoped.toml \
  --redis-url redis://127.0.0.1/
```

Текущий статус Phase 1 candidate:

- первый controlled live `create -> delete` на `sessiongap` action-scoped candidate уже прошёл успешно
- результат зафиксирован в `./docs/action-scope-cws-phase1-create-delete-results-2026-04-02.md`
- исходный canonical `~30m idle gap` на candidate со старым cached token state дал `FAIL`
- повторный canonical `~30m idle gap` на force-refresh candidate уже дал `PASS`
- результат зафиксирован в `./docs/action-scope-cws-phase1-force-refresh-idle-gap-results-2026-04-02.md`
- ещё один force-refresh `~30m idle gap` confidence retest тоже дал `PASS`
- результат зафиксирован в `./docs/action-scope-cws-phase1-force-refresh-idle-gap-retest-results-2026-04-02.md`
- ещё один longer-gap confidence retest после `~50m` тоже дал `PASS`
- результат зафиксирован в `./docs/action-scope-cws-phase1-force-refresh-long-gap-retest-results-2026-04-02.md`
- базовый live TOML при этом не менялся
- `exit/flatten` по-прежнему вне scope и выключены
- decision note по новой основной Phase 1 линии:
  - `./docs/action-scope-cws-phase1-rollout-decision-2026-04-02.md`
- первый controlled Phase 2 `entry -> flatten` lifecycle уже тоже прошёл:
  - `./docs/action-scope-cws-phase2-entry-flatten-results-2026-04-02.md`
- decision note по текущей основной Phase 2 линии:
  - `./docs/action-scope-cws-phase2-rollout-decision-2026-04-02.md`
- операторский runbook для следующей разработки:
  - `./docs/action-scope-cws-phase2-entry-flatten-runbook-2026-04-02.md`

Для сравнительной live-диагностики задавайте `ALOR_STACK_NAME` явно:

- `sessiongap` для `sessiongap` gateway
- `hybrid` для `hybrid` gateway

Это имя появится:

- в `/readiness`
- в `cws_limit_send`
- в `cws_transport_failure`
- в `cws_fail_pending`

## 3) Health endpoints

Gateway поднимает HTTP health server:
- `GET /liveness` — процесс жив (ожидается `200 OK`).
- `GET /readiness` — JSON со статусом готовности (`readiness`) и диагностикой фаз.

`/readiness` возвращает HTTP `503 Service Unavailable`, когда `readiness=false`.
Это ожидаемое поведение для orchestration/k8s: endpoint должен использоваться именно как readiness-probe.

> Примечание: endpoint `/startup` в текущей реализации отсутствует. Для startup-probe используйте `/liveness` до появления отдельного `/startup`.

Критичные поля `/readiness`:
- `gateway_phase` (`SyncingHistory`, `Reconnecting`, `SyncingGap`, `LiveReady`),
- `stack_name`, `gateway_instance_id`, `auth_principal_fingerprint`,
- `ws_connected`, `cws_authorized`,
- `cws_connection_instance_id`, `cws_connect_seq`, `cws_reconnect_seq`,
- `cws_last_connect_ts_utc`, `cws_last_transport_failure_ts_utc`,
- `cws_last_limit_send_ts_utc`, `cws_last_limit_error_ts_utc`,
- `cws_last_successful_send_ts_utc`, `cws_last_successful_ack_ts_utc`,
- `cws_pending_count`,
- `last_bar_age_sec`, `ws_last_rx_age_sec`,
- `active_subscriptions_count` vs `desired_subscriptions_count`,
- `ws_reconnects_total`, `backpressure_lagged`.

```bash
curl -sf http://127.0.0.1:8081/readiness
```

### 3.1 Рекомендация для Kubernetes probes

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

- `liveness` отвечает только за «процесс жив».
- `readiness` отвечает за готовность принимать боевой трафик и может временно возвращать `503`.

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

### 4.4 CWS transport failure after send
Симптомы:
- `cmd.acks.*` содержит `status=error`, `error_code=cws_error`,
- `error_msg` выглядит как `cws disconnected: <disconnect_kind>`,
- в gateway логах есть `action=cws_transport_failure`,
- следом есть `action=cws_fail_pending`.

Что смотреть:
1. `request_id` из `cmd.orders.*` и `cmd.acks.*`.
2. `cws_guid` из:
   - `cws_limit_send`,
   - `cws_limit_ack`,
   - published `command_ack`.
3. `disconnect_kind` и `raw_error` в `cws_transport_failure`.
4. `affected` список в `cws_fail_pending`:
   - `request_id`,
   - `cws_guid`,
   - `opcode`,
   - `symbol`.
5. telemetry around the incident:
   - `stack_name`,
   - `gateway_instance_id`,
   - `auth_principal_fingerprint`,
   - `connection_age_ms`,
   - `time_since_last_reconnect_ms`,
   - `in_flight_pending_count`,
   - `last_successful_send_ts_utc`,
   - `last_successful_ack_ts_utc`.
6. `events.health` рядом с инцидентом:
   - `ws_connected`,
   - `cws_authorized`,
   - `gateway_phase`,
   - reconnect counters.

Интерпретация:
- `protocol_reset_without_close_handshake` обычно указывает на transport/session reset без нормального broker ack.
- `close_frame` означает, что сервер/peer прислал close frame; дополнительно смотреть `close_code` и `close_reason`.
- `eof` означает, что поток закончился без broker response.
- `send_error` полезен, когда ошибка произошла на отправке уже подготовленного запроса.

Действия:
1. Убедиться, что pending request действительно был fail’нут с сохранённым `cws_guid`.
2. Проверить, что gateway ушёл в reconnect и вернулся в `LiveReady`.
3. Сверить `broker.orders.*` / `broker.positions.*` перед ручным recovery.
4. Если это `session_gap`, проверить `runtime.state.session_gap_standalone.live.<portfolio>` на хвост `Blocked`.

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
cargo run -p alor-gateway --bin alor_gateway_runner -- --config ./configs/gateway.sessiongap.live.7502MIW.toml
```

---

## 6) Preflight checklist
- [ ] Redis доступен, stream names валидны.
- [ ] Token/portfolio/symbols проверены.
- [ ] Health endpoint доступен по `health_listen_addr`.
- [ ] Readiness достигает `LiveReady` до разрешения live-ордеров в runtime.

## 7) Phase 1 Controlled Candidate Checks

Для Phase 1 `action_scoped` candidate на `sessiongap` безопасная live-последовательность сейчас такая:

1. Убедиться, что gateway и runtime в `LiveReady`.
2. Убедиться, что по `USDRUBF` нет открытой позиции и нет рабочего ордера.
3. Отправить пассивный `create:limit` вне рынка.
4. Дождаться `command_ack accepted` и статуса `working` с `filled=0.0`.
5. Отправить `delete:limit` для того же `order_id`.
6. Дождаться `command_ack accepted` и финального статуса `canceled` с `filled=0.0`.
7. Снять `/readiness`, `cmd.acks.*`, `broker.orders.*`, `broker.positions.*`.

Канонический следующий acceptance-case:

1. после первого успешного bounded window не держать open CWS session
2. выдержать около `30m` без control action
3. повторить controlled passive `create -> delete`
4. подтвердить, что второй send тоже идёт через fresh short-lived action-scoped session

Текущий вывод по этому acceptance-case:

1. на старом cached-token variant он уже падал
2. на variant с `action_scope_force_token_refresh_before_authorize = true` он уже прошёл
3. второй такой же force-refresh retest тоже уже прошёл
4. longer-gap retest после `~50m` тоже уже прошёл
5. при post-gap pass логи должны показывать:
   - `invalidated cached alor access token`
   - `refreshed alor access token consumer="action_scope_cws_authorize"`
   - `action_scope_authorize_ok ... access_token_source="refreshed"`
