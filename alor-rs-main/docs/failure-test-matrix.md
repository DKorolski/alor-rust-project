# Failure Test Matrix (gateway + runtime + `session_gap_standalone`)

Рабочая матрица отказных/граничных сценариев для:

- `alor-gateway` (`alor_gateway_transport_runner`)
- `strategy-runtime` (`strategy_runtime_runner`)
- стратегия `session_gap_standalone`
- транспорт через Redis Streams

Цель:

- зафиксировать ожидаемое поведение до code changes;
- проверять реальное поведение по логам/health/Redis;
- использовать как regression checklist после доработок.

## Scope и контекст

Подтвержденный основной запуск:

- gateway: `cargo run -p alor-gateway --bin alor_gateway_transport_runner -- --config ./configs/gateway.live.toml --redis-url redis://127.0.0.1/`
- runtime: `cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.live.toml`
- strategy: `strategy_id = "session_gap_standalone"`

Известное ограничение:

- deep cold-start history через gateway ограничен (порядка ~5000 баров / ~3–4 дня);
- для длинной проверки логики используется replay (см. `docs/replay-backtest-guide.md`).

## Как использовать матрицу

Для каждого сценария фиксировать:

- дата/время
- commit SHA
- конфиги (`gateway.live.toml`, `runtime.live.toml`)
- trade mode (`paper` / `live` / `replay`)
- результат (`PASS` / `FAIL` / `PARTIAL`)
- ссылки на артефакты (логи, readiness snapshots, Redis excerpts)

Рекомендуемый формат папки артефактов:

- `replay_out/failure-tests/<yyyy-mm-dd>/<scenario-id>/...`

## Базовые наблюдаемые сигналы (чеклист)

### Gateway (`/readiness`)

Ключевые поля:

- `gateway_phase`
- `ws_connected`
- `cws_authorized`
- `active_subscriptions_count`
- `desired_subscriptions_count`
- `backpressure_lagged`
- `ws_reconnects_total`
- `ws_last_rx_age_sec`
- `last_bar_age_sec`

### Runtime (`/readiness`)

Ключевые поля:

- `readiness` (`200`/`503`)
- `live_guard_status` (`ALLOWED` / `BLOCKED`)
- `live_guard_reasons`
- `gateway_health_age_sec`
- `scheduler.state`

### Redis Streams

Минимум:

- `cmd.orders.<portfolio>`
- `cmd.acks.<portfolio>`
- `broker.orders.<portfolio>`
- `broker.positions.<portfolio>`
- `events.health`

## Матрица сценариев (первая волна)

### FT-01: Cancel already terminated order (terminal cancel)

Цель:

- проверить поведение при попытке отменить заявку, которая уже в terminal state (`filled` / `canceled` / `rejected`).

Почему важно:

- это один из непокрытых troubleshooting сценариев;
- может сломать ожидания по ack/phase в runtime или породить шумные ошибки.

Подготовка:

- иметь заявку, которая гарантированно перейдет в terminal state;
- затем инициировать cancel через обычный command path.

Шаги (manual):

1. Запустить gateway + runtime.
2. Дождаться `gateway_phase=LiveReady`, `runtime live_guard=ALLOWED` (если live scenario).
3. Создать/дождаться terminal state по заявке.
4. Отправить cancel command на уже terminal order.

Ожидаемое поведение (целевая модель):

- Gateway:
  - публикует `cmd.ack` с понятным статусом/reject reason (или duplicate/no-op semantics, если так задумано).
  - не уходит в reconnect/resync из-за этого сценария.
- Runtime:
  - не ломает phase/state;
  - не зависает в неконсистентном `Pending*`;
  - если уходит в `Blocked`, причина должна быть ясной и диагностируемой.
- Strategy (`session_gap_standalone`):
  - не должна эмитить повторные intents из-за некорректного ack path.

Что собрать:

- runtime logs (`command rejected` / strategy phase transition / blocked reason)
- последние записи `cmd.orders`, `cmd.acks`, `broker.orders`
- snapshots `/readiness` gateway/runtime до/после

Статус:

- `TODO`

---

### FT-02: Broker reject / insufficient funds

Цель:

- проверить end-to-end поведение на отказе брокера/риска (например, нехватка средств).

Почему важно:

- это high-value negative path для live readiness;
- должен корректно отрабатывать `ack.status=Rejected/Error/Expired`.

Подготовка:

- безопасный способ воспроизвести reject (бумажный режим с искусственным reject path / live малым объемом и контролируемым условием).

Шаги (manual/controlled):

1. Запустить gateway + runtime.
2. Сгенерировать команду, которая гарантированно будет отклонена брокером.
3. Зафиксировать `cmd.ack`, runtime logs, strategy phase.

Ожидаемое поведение (целевая модель):

- Gateway:
  - публикует `cmd.ack` со статусом reject/error и broker/cws details;
  - command consumer остается жив.
- Runtime:
  - `handle_ack()` обрабатывает reject;
  - стратегия переходит в `Blocked` (или другой явно задокументированный safe-state);
  - нет повторной автоматической эмиссии команды без operator action.
- Runtime health:
  - readiness/live_guard остаются предсказуемыми (сценарий command reject не должен “ронять” весь runtime health loop).

Что собрать:

- `cmd.acks` payload (error_code/error_msg/cws_http_code/cws_request_guid)
- runtime logs + phase transition
- `runtime /readiness`

Статус:

- `TODO`

---

### FT-03: Redis publish failure during `publish_command_and_state` (runtime)

Цель:

- проверить, что при сбое публикации команды/состояния runtime не подтверждает входное сообщение преждевременно и не теряет согласованность.

Почему важно:

- это один из самых критичных failure paths для exactly/at-least-once semantics в контуре стратегии.

Подготовка:

- способ кратковременно сломать Redis в момент публикации:
  - остановка Redis
  - firewall/drop
  - неверный route (на тестовом окружении)

Шаги:

1. Запустить gateway + runtime.
2. Довести стратегию до эмиссии intent/команды.
3. В момент публикации временно нарушить доступ к Redis.
4. Восстановить Redis.
5. Наблюдать recovery/повторную обработку.

Ожидаемое поведение (целевая модель):

- Runtime:
  - логирует `failed to publish command and state`;
  - не делает premature `xack` входного сообщения;
  - после восстановления Redis поведение остается консистентным (без неконтролируемых дублей).
- Health/metrics:
  - рост `publish_failures_total`;
  - runtime не silently continues as success.

Что собрать:

- runtime logs вокруг `publish_command_and_state`
- Redis stream excerpts до/после
- подтверждение наличия/отсутствия дубликатов в `cmd.orders` / `runtime_state`

Статус:

- `TODO`

---

### FT-04: Stale health event -> runtime BLOCKED

Цель:

- проверить, что runtime корректно блокирует live торговлю при устаревшем health от gateway.

Почему важно:

- один из ключевых safety-механизмов для pre-prod/live.

Подготовка:

- возможность остановить gateway, оставив runtime работать
  или
- симулировать отсутствие обновлений `events.health`.

Шаги:

1. Запустить gateway + runtime и дождаться `ALLOWED`.
2. Остановить gateway (или прекратить health updates).
3. Наблюдать runtime `/readiness` и `live_guard`.

Ожидаемое поведение:

- Runtime:
  - `live_guard_status -> BLOCKED`
  - `readiness -> 503`
  - причины содержат stale/phase diagnostics (`gateway_health_stale`, `phase=...`)
- После восстановления gateway:
  - возвращение к `ALLOWED` только при выполнении всех условий (health свежий, phase `LiveReady`, bars/live path в норме)

Что собрать:

- последовательность runtime `/readiness` snapshots
- `events.health` последние записи до/после
- логи `live_guard_changed`

Статус:

- `TODO`

---

### FT-05: Late / duplicate ack after phase transition (`session_gap_standalone`)

Цель:

- проверить устойчивость strategy phase-machine к поздним/повторным ack после смены фазы (`PendingEntry -> InPosition`, etc.).

Почему важно:

- стратегия считает `Duplicate` как successful ack (что обычно правильно);
- нужно подтвердить, что stale duplicate не ломает phase.

Тип сценария:

- сначала unit/integration (предпочтительно с контролируемым event sequence)
- затем manual only if needed

Ожидаемое поведение:

- фаза не откатывается/не портится;
- `phase_last_change_ts_utc` остается консистентным;
- `last_bar_ts` не деградирует.

Артефакты:

- strategy/runtime logs
- state snapshot до/после
- (если unit/integration) deterministic assertion output

Статус:

- `TODO`

---

### FT-06: Replay vs standalone baseline consistency (регрессионный контроль)

Цель:

- поддерживать подтвержденную совместимость логики `replay` vs standalone backtest в пределах допустимых отклонений.

Почему важно:

- replay используется как рабочий verification path из-за ограничения cold-start history в gateway;
- это часть operational confidence для стратегии.

Что проверяем:

- ключевые итоговые метрики/результаты стратегии
- количество сделок / направления / основные transition points
- расхождения в рамках заранее принятого допуска

Нужно зафиксировать policy:

- какие метрики сравниваем
- какой порог отклонений допустим
- что считается regression/blocker

Статус:

- `PARTIAL` (по словам пользователя replay уже сравнивался и принят как близкий к standalone; formalized baseline policy еще нет)

## Шаблон результата по сценарию

```md
### FT-XX: <name>
- Date:
- Commit:
- Configs:
- Mode: live/paper/replay
- Result: PASS / FAIL / PARTIAL
- Summary:
- Evidence:
  - logs:
  - readiness snapshots:
  - redis excerpts:
  - reports/artifacts:
- Follow-up actions:
```

## Приоритет выполнения (рекомендация)

1. `FT-04` (stale health -> BLOCKED) — safety критично и быстро воспроизводимо
2. `FT-02` (broker reject / insufficient funds) — high-value negative path
3. `FT-01` (terminal cancel) — очень практичный troubleshooting кейс
4. `FT-03` (Redis publish failure) — критично, но сложнее воспроизводить
5. `FT-05` (late/duplicate ack) — лучше через controlled test harness
6. `FT-06` (replay baseline policy) — formalization step, не срочный hotfix

