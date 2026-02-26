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

Дополнительный ускоренный path для отказных сценариев (`FT-01..FT-03`):

- runtime config: `configs/runtime.mock-live.toml`
- `strategy_kind = "mock_live_probe"`
- сценарий выбирается по суффиксу `strategy_id` (например `mock_live_probe.place_limit_bad_step`)

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
- допустимый ускоренный вариант для troubleshooting: использовать уже известный historical `order_id` в terminal state (например из `broker.orders` snapshot/лога) и отправить `Cancel` как negative test.

Шаги (manual / рекомендуется ускоренный path через `mock_live_probe.cancel_after_terminal`):

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

- `PASS` (manual command via Redis command path, historical terminal `order_id`)

Подтвержденное поведение (наблюдение):

- test input: `Cancel` на historical terminal order (`filled`) через `cmd.orders.<portfolio>`
- gateway отправил `delete:limit`
- CWS вернул `httpCode=400`, `message="Order to cancel not found"`
- gateway опубликовал `cmd.ack status=Rejected` с `error_code="cws_http_400"`
- `command_consumer` продолжил работу штатно (poll loop жив)

---

### FT-02: Broker reject scenarios (family)

Цель:

- проверить end-to-end поведение на отказах брокера/биржи/валидации заявки.

Почему важно:

- это high-value negative path для live readiness;
- должен корректно отрабатывать `ack.status=Rejected/Error/Expired`.

Подготовка:

- безопасный способ воспроизвести reject (бумажный режим с искусственным reject path / live малым объемом и контролируемым условием).

Подтипы (минимальный набор):

- `FT-02A` — insufficient funds / margin
- `FT-02B` — price out of range / price limits
- `FT-02C` — outside trading session / weekend
- `FT-02D` — invalid symbol / unavailable instrument for account type
- `FT-02E` — invalid lot size
- `FT-02F` — invalid price step
- `FT-02G` — `BookOrCancel` reject (цена пересекает спред / приводит к немедленному исполнению)

Шаги (manual/controlled, рекомендуется `mock_live_probe`):

1. Запустить gateway + runtime.
2. Выбрать один подтип (`FT-02A..FT-02G`).
3. Сгенерировать команду, которая гарантированно вызовет выбранный reject.
4. Зафиксировать `cmd.ack`, runtime logs, strategy phase.

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
- для каждого подтипа: фактический broker message / error text (для каталога отказов)

Статус:

- `PARTIAL` (несколько подтипов уже подтверждены, family в целом еще не закрыт)

Первые наблюдения (уже подтверждено):

- `FT-02A` (`insufficient funds / margin`) — `PASS`
  - подтвержден `cws_http_code=400`
  - пример broker/CWS message: `"Нехватка средств по лимитам клиента."`
  - `cmd.ack status=Rejected`
  - runtime логирует `command rejected` и контур продолжает работать

- `FT-02F` (`invalid price step`) — `PARTIAL`
  - через текущий gateway path сценарий воспроизводился как валидная заявка (`cmd.ack=Accepted`, далее `working/fill`)
  - вероятная причина: нормализация/округление цены до `price_step` до отправки в CWS (или аналогичная server-side normalization)
  - вывод: как broker reject в текущем path ненадежен, не использовать как основной regression reject scenario

- `FT-02G` (`BookOrCancel` immediate execution reject`) — `PASS`
  - подтвержден `cws_http_code=400`
  - `cmd.ack status=Rejected`
  - runtime логирует `command rejected` с `error_code/error_msg/cws_http_code/cws_request_guid`
  - gateway/runtime продолжают работать штатно

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

- `PASS` (manual execution on live stand)

Подтвержденное поведение (наблюдение):

- runtime переходил `ALLOWED -> BLOCKED` с причиной `gateway_health_stale` после остановки gateway и истечения stale timeout
- `runtime /readiness` до остановки gateway:
  - `readiness=true`
  - `live_guard=ALLOWED`
- `runtime /readiness` после stale timeout:
  - `readiness=false`
  - `live_guard=BLOCKED`
  - `live_guard_reasons=["gateway_health_stale"]`
- `gateway.health_age_sec` в runtime readiness вырос (наблюдалось `35s` при `gateway_health_stale_sec=20`)

Нюанс семантики (ожидаемо и полезно):

- даже в состоянии stale runtime может показывать последние известные `gateway_ready/ws_connected/cws_authorized=true`;
- блокировка определяется freshness health-событий (`gateway_health_stale`), а не только последними флагами готовности.

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

## Черновой контракт test strategy `mock_live_probe` (для ускорения FT-01..FT-03)

Статус:

- `v1 implemented`
- код: `strategy-runtime/src/strategies/mock_live_probe.rs`
- конфиг запуска: `configs/runtime.mock-live.toml`

Идея:

- добавить простую live-стратегию для runtime, которая быстро и детерминированно эмитит intents;
- использовать её для failure-path тестов runtime/gateway без ожидания редких сигналов `session_gap_standalone`.

### Зачем

- сокращает время ожидания интента;
- уменьшает шум от доменной логики основной стратегии;
- упрощает воспроизведение `FT-01..FT-03`.

### Принципы

- работает в `TradeMode::Live` (или `paper` при необходимости dry-run проверки path)
- эмитит intent на раннем этапе (`1-2` live бара)
- максимально простая state-machine
- режим сценария выбирается через суффикс `strategy_id` (v1)

### Режимы `mock_live_probe` (v1 реализованы)

- `place_market_once`
  - одна market команда после `N` live баров (`N = strategy.max_wait_bars_for_ack`)
  - базовый smoke для command path / `FT-03`

- `place_limit_bad_price`
  - лимитная заявка с заведомо некорректной ценой (для `FT-02B`)

- `place_limit_bad_step`
  - цена, не кратная `price_step` (для `FT-02F`)

- `place_boc_cross_spread`
  - агрессивная лимитная цена (через `BookOrCancel`) для детерминированного reject (для `FT-02G`)

- `cancel_after_terminal`
  - create -> дождаться terminal (через ack/order/trade/position path) -> отправить cancel (для `FT-01`)

### План режимов `mock_live_probe` (v2)

- `place_limit_bad_lot` (для `FT-02E`)
- `place_invalid_symbol` (для `FT-02D`)

### Минимальные observability требования к `mock_live_probe`

Логи стратегии должны явно писать:

- `scenario=<...>`
- `intent_emitted`
- `request_id` (если известен на уровне runtime/strategy)
- stage transitions (`waiting_live_bar`, `sent_create`, `waiting_terminal`, `sent_cancel`)

### Что не делать в v1

- не пытаться воспроизводить всю логику `session_gap_standalone`
- не делать сложный reconcile/state restore
- не смешивать много сценариев в одном запуске

### Как использовать в матрице

- `FT-01..FT-03` можно выполнять на `mock_live_probe`
- `FT-04`, `FT-05`, `FT-06` остаются привязанными к runtime/health/strategy semantics (`session_gap_standalone` / replay)

### Быстрый запуск `mock_live_probe`

```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.mock-live.toml
```

Режим меняется через `strategy.strategy_id`:

- `mock_live_probe.place_market_once`
- `mock_live_probe.place_limit_bad_price`
- `mock_live_probe.place_limit_bad_step`
- `mock_live_probe.place_boc_cross_spread`
- `mock_live_probe.cancel_after_terminal`

Примечание по артефактам:

- при `reset_state_on_start = true` runtime не восстанавливает state, но backlog в `broker.orders`/`broker.trades` streams может давать startup-шум (`orphan_trade`, historical `existing=true` events);
- для оценки сценария ориентироваться на события после `probe_emitting_intent`.

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
