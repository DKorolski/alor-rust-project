# Аудит `alor-rs-main` (gateway + strategy runtime) и roadmap доработок

## Цель документа

Документ фиксирует:

- критический аудит текущей архитектуры `gateway + strategy runtime + redis_transport`;
- основные эксплуатационные и кодовые риски (`P0/P1/P2`);
- roadmap доработок по итерациям с критериями приемки.

Фокус этого аудита:

- `alor-gateway` как шлюз к Alor (`WS/CWS`, нормализация, Redis streams);
- `strategy-runtime` как исполнитель стратегии (`paper/live/replay`);
- `redis_transport` как шина обмена событиями/командами через Redis Streams.

Важно: текущий этап — **только аудит и ТЗ**, без code changes.

## Подтверждено пользователем (эмпирически, важно для оценки рисков)

Ниже — не статический анализ, а результаты фактической проверки эксплуатации/тестов:

1. Для взаимодействия с runtime используется:
- `alor_gateway_transport_runner`

2. Выполнялись live тесты:
- reconnect/resync отрабатывали;
- журнал баров и отчетные артефакты были последовательными, без дублей;
- отдельные live-сценарии `limitCancel`, `MarketBuyAndClose` выполнялись успешно.

3. Выполнялся paper test:
- проходил с записью сделки в журнал.

4. Непокрытые (или неуверенно покрытые) сценарии troubleshooting:
- отмена заявки в terminal state (`filled/canceled/rejected`);
- ошибки уровня брокера/риска (например, нехватка средств);
- другие отказные/граничные сценарии CWS/ACK path.

5. Известное текущее ограничение (не обязательно дефект):
- `cold_start_history_days_back` практически ограничен несколькими днями;
- причина по наблюдениям: ограничение объема исторической выборки/батча (порядка ~5000) в подписочном сценарии и отсутствие пагинации;
- для глубокого исторического прогона использовался старый standalone backtest/replay flow.

Вывод для аудита:
- базовая operational механика (reconnect/resync, dedup, paper/live path) имеет **положительное эмпирическое подтверждение**;
- приоритет смещается на negative/failure-path coverage и формализацию контрактов.

## Что уже выглядит хорошо (сильные стороны)

1. Архитектура уже разделена на подсистемы:
- `alor-gateway`
- `strategy-runtime`
- `alor-protocol`
- `alor-types`

2. Есть эксплуатационная документация/runbooks:
- `docs/alor-gateway-runbook.md`
- `docs/strategy-runtime-runbook.md`
- `docs/state-and-restarts.md`

3. Есть health/readiness модель и явные operational сигналы:
- `gateway_phase`, `ws_connected`, `cws_authorized`, lag/backpressure counters
- runtime `live_guard`, readiness/liveness, scheduler state

4. Есть задел под устойчивость:
- reconnect/resubscribe в gateway (`ws_hub`, `supervisor`)
- idempotency для команд (`command_consumer`)
- recover/drain pending сообщений в runtime

5. Есть CI и тесты (в отличие от первого репо):
- `.github/workflows/ci.yml`
- unit/integration tests (часть интеграционных требует Docker)

6. Есть эмпирическое подтверждение ключевых сценариев:
- live reconnect/resync проходили;
- paper flow с записью сделки проходил;
- отдельные live order scenarios выполнялись успешно.

## Краткое резюме (на текущем этапе)

Проект выглядит **существенно более зрелым**, чем исходный `alor-rust-connector`, и уже ориентирован на pre-prod / paper / live.

Основные риски смещаются:

- не столько в "паники everywhere",
- сколько в корректность runtime semantics:
  - подписки/ресабскрайб,
  - консистентность Redis Streams и ack-ов,
  - деградация под backpressure,
  - восстановление после разрывов/рестартов,
  - детерминизм стратегии и state reconcile.

С учетом подтвержденных live/paper тестов, основной фокус доработок — не “сделать базовую механику”, а:

- расширить покрытие отказных сценариев;
- формализовать invariants/контракты;
- усилить confidence в edge-cases перед pre-prod/live.

## Findings (P0 / P1 / P2)

### P0 / P1: Надежность торгового контура и консистентность состояния

### 1. `gateway`/`runtime` сильно завязаны на Redis Streams semantics; нужен явный контракт delivery/ack/idempotency на уровне документа и тестов

Почему это важно:

- `strategy-runtime` читает несколько stream-ов (`bars/orders/trades/positions/acks`), `xack` делает после обработки;
- `gateway` публикует события/ack/health и также обслуживает командный канал;
- ошибка в порядке `process -> persist -> xack` может дать:
  - повторную обработку (дубликаты),
  - потерю связи между state snapshot и отправленной командой,
  - рассинхронизацию ledger/state/стратегии.

Наблюдения по коду:

- `strategy-runtime/src/runtime.rs` (`drain_stream`, `dispatch_message`, `handle_*`, `persist_state`)
- `alor-gateway/src/services/command_consumer.rs` (idempotency + publish ack + source ack)

Риск:

- subtle regressions при изменении порядка операций или retry/publish поведения.

Статус:

- код уже учитывает многие кейсы, но нужен **явный invariants contract** + интеграционные тесты на failure scenarios.

---

### 2. Reconnect / resubscribe логика в `gateway` реализована, но это зона повышенного риска и требует property-like и сценарных тестов

Почему это важно:

- `alor-gateway/src/ws_hub.rs` содержит:
  - subscribe ack timeout/retries
  - reconnect
  - resubscribe all / from_ts
  - generation-based filtering
- `alor-gateway/src/supervisor.rs` содержит:
  - phase transitions (`SyncingHistory/Reconnecting/SyncingGap/LiveReady`)
  - bar silence detection
  - resync/backfill decisions

Это правильная архитектура, но сложность высокая.

Риск:

- race conditions между `generation`, delayed ack/events и phase transitions;
- ложные ресинхронизации/флаппинг readiness;
- некорректный warm/cold backfill режим при редких/нестандартных разрывах.

Статус:

- сильный плюс, что механика есть; минус — нужен расширенный набор сценарных тестов и формализация инвариантов.
- Дополнение по факту: пользователь подтверждает, что live reconnect/resync уже выполнялись успешно, а артефакты баров/отчеты были последовательными и без дублей. Это снижает риск базовой реализации, но не отменяет потребность в систематических failure tests.

---

### 3. Runtime readiness/live-guard зависит от качества health-событий gateway; stale/partial health может блокировать или, хуже, открыть live too early

Почему это важно:

- runtime использует health stream gateway (`streams.health`) и `live_guard`;
- решения `ALLOWED/BLOCKED` критичны для live режима.

Наблюдения:

- `strategy-runtime/src/runtime.rs`: `refresh_health_if_due`, `refresh_health_snapshot`, `log_live_guard_status_if_due`
- runbook описывает `gateway_health_stale_sec`, `phase`, `readiness`.

Риск:

- false positive readiness (команда уйдет в live при деградировавшем gateway);
- false negative (runtime “залипает” в BLOCKED без операционной причины).

Статус:

- архитектурно правильно, но это high-stakes зона, требует тестов на stale/missing/out-of-order health events.

---

### 4. Backpressure в gateway детектируется, но нужны четкие гарантии поведения при перегрузке и последствия для downstream

Наблюдения:

- `alor-gateway/src/supervisor.rs`: `raw_tx.try_send(...)`, `backpressure_lagged = true`, readiness=false, warning/log event
- это правильно как сигнал деградации.

Риск:

- если `raw queue full`, часть событий может быть отброшена/необработана (зависит от upstream/downstream цепочки);
- необходимо точно задокументировать:
  - что именно может потеряться,
  - как происходит recovery,
  - что увидит runtime.

Статус:

- detection есть, но нужен formal operational contract + нагрузочные тесты.

---

### 5. Порядок `persist state` / `publish command` / `ack` в runtime критичен; нужны тесты на atomicity-ish поведение при сбоях Redis

Наблюдения:

- `strategy-runtime/src/runtime.rs`: `persist_state(Some(command))` вызывает `publish_command_and_state(...)`
- ошибки publish/state persistence приводят к `Err(...)`, что хорошо (fail fast)
- `xack` происходит после успешной обработки в `handle_*`

Риск:

- при частичных сбоях Redis/сети возможны повторы сообщений и повторная генерация intent/command;
- защита частично есть (`our_request_ids`, dedup state), но это нужно покрыть сценариями.

Статус:

- вероятно operationally workable, но требует failure-mode tests (disconnect / xadd fail / ack fail).
- Дополнение по факту: paper flow и отдельные live-сценарии команд подтверждены пользователем; основная недопроверенная зона — error/troubleshooting paths (terminal cancel, insufficient funds, broker rejects).

### P1 / P2: API/UX, сопровождение, эксплуатация

### 6. `Alor`-специфичные инварианты (подписки, order state mapping, CWS/WS semantics) не выделены в единый “protocol contract” документ

Сейчас знания размазаны по:

- коду `gateway`
- runbooks
- тестам/мокам

Риск:

- при доработках команды/подписок можно сломать поведение без явного понимания контракта.

Что нужно:

- отдельный документ уровня `docs/protocol_contract.md` с invariants:
  - source-of-truth для `order_id`
  - mapping ack/status
  - resubscribe expectations
  - duplicate/out-of-order handling

---

### 7. `unwrap/expect/panic` в runtime-пути есть, но по первым признакам в основном сосредоточены в тестах/вспомогательных участках; нужен точечный runtime review

Первичный поиск по `alor-gateway/src` и `strategy-runtime/src` показывает множество `unwrap/expect`, но значительная часть — тесты.

Однако есть runtime-кандидаты, требующие проверки:

- `strategy-runtime/src/bin/strategy_runtime_runner.rs` — `expect("install SIGTERM handler")`
- `alor-gateway/src/cws_client.rs` — `expect(...)` на shape payload (нужна проверка, runtime ли это path)
- отдельные timestamp/time parsing helper-ы в gateway/runtime (часть может быть production path)

Риск:

- неожиданный payload/OS edge-case -> panic.

Статус:

- нужен отдельный targeted pass “runtime-only unwraps”.

---

### 8. Публичные контракты Redis stream names / consumer group semantics выглядят мощно, но требуют freeze/compatibility policy

Почему:

- `configs/*` + env overrides + stream names активно используются в продобном контуре;
- изменение названий/trim policy/group policy может silently сломать совместимость.

Что нужно:

- документированный compatibility policy для stream schema + naming + trim.

---

### 9. README верхнего уровня уже полезный, но для инженерного аудита не хватает отдельной “architecture map”

Сейчас есть runbooks (это плюс), но для новых разработчиков/ревью:

- хорошо бы иметь 1 документ с dataflow:
  - Alor -> gateway -> Redis streams -> runtime -> command stream -> gateway -> CWS
  - health path
  - state snapshots path

Это снизит риск ошибок при будущих рефакторингах.

### 10. Ограничение cold-start истории через подписочный поток требует явного документирования и альтернативного пути для глубокого bootstrap/backfill

Фактическое наблюдение пользователя:

- `cold_start_history_days_back` практически не удается выставить на большие значения (ограничение по объему/батчу, отсутствует пагинация в текущем подходе);
- для длинных исторических прогонов использовался standalone/backtest контур.

Почему это важно:

- это не обязательно “баг”, но важное ограничение архитектуры cold-start режима;
- оператор может ожидать глубокий bootstrap истории от gateway/runtime и получить неполный прогрев.

Риск:

- ложные ожидания от конфигурации;
- некорректный pre-prod/live запуск после “глубокого” cold start, если история фактически урезана.

Что нужно:

- явно документировать limit/ограничение;
- добавить preflight warning/validation;
- определить supported path для deep history (replay/standalone/export path).

## Предварительный список вопросов (для следующего этапа, перед code changes)

1. Подтвержденный основной путь запуска gateway для связки с runtime:
- `alor_gateway_transport_runner` (по информации пользователя)

2. Какие именно отказные сценарии считаются приоритетными для следующей волны тестов:
- cancel already terminated order (`filled/canceled/rejected`)
- insufficient funds / broker reject
- stale health / runtime blocked
- Redis transient failures during publish/ack

3. Какие тесты считаются обязательными перед pre-prod/live:
- unit only?
- integration с Redis/Docker?
- paper smoke?
- replay regression?

## Roadmap / ТЗ по итерациям

Ниже — практичный порядок доработок после завершения аудита (и перед точечными правками).

## Итерация 1. Критические инварианты и observability contract (без ломки архитектуры)

### Цель

Зафиксировать корректное поведение системы в сложных состояниях (reconnect/resubscribe/backpressure/stale health) и сделать это проверяемым.

### Объем работ

- Описать protocol/runtime invariants в `docs/`:
  - Redis delivery/ack contract
  - command idempotency contract
  - order lifecycle mapping (ack/order/trade/position)
  - health/live_guard decision inputs
- Добавить structured diagnostic snapshots:
  - gateway phase / subscription state
  - runtime guard decision state
- Добавить явные reason codes для ключевых блокировок/resync triggers (если еще не везде унифицированы)
- Провести targeted runtime-only review `unwrap/expect/panic` (без тестовых модулей)
- Отдельно зафиксировать documented limitation:
  - `cold_start_history_days_back` / historical bootstrap depth через подписки
  - supported альтернативный путь для deep history

### Критерии приемки

- Есть документ с инвариантами и failure semantics
- Есть понятный набор диагностических сигналов для postmortem
- Runtime panic-paths в production code перечислены и классифицированы

## Итерация 2. Failure-mode testing для Redis + reconnect/resubscribe

### Цель

Проверить устойчивость торгового контура на типовых сбоях, не меняя пока архитектуру радикально.

### Объем работ

- Интеграционные/сценарные тесты (часть с mock/testcontainers):
  - subscribe ack timeout -> retry -> success
  - ws reconnect -> resubscribe -> LiveReady recovery
  - bar silence -> resync trigger -> recovery
  - stale health event -> runtime BLOCKED
  - Redis publish fail (`publish_command_and_state`) -> no premature ack
  - duplicate command/request_id -> duplicate ack, no duplicate execution
  - cancel command for terminal order state (server reject / no-op path)
  - insufficient funds / broker-level reject path with expected ack/guard/logging
- Проверить out-of-order / duplicate events в runtime:
  - acks before orders
  - trades before known order (orphan path)

### Критерии приемки

- Есть автоматические сценарии на основные failure modes
- Поведение при сбоях документировано и воспроизводимо
- Нет silent data-loss path в известных сценариях без явного health/readiness degradation

## Итерация 3. API/операционная эргономика и safety rails (pre-prod hardening)

### Цель

Упростить эксплуатацию и снизить вероятность operator/user errors.

### Объем работ

- Freeze policy для stream schemas / names / trim settings
- Preflight validation:
  - конфиг
  - stream names
  - consumer group readiness
  - health endpoints
- Улучшить startup/shutdown semantics:
  - graceful stop timeouts
  - flush/report guarantees
- Отдельная архитектурная схема dataflow + component responsibilities

### Критерии приемки

- Оператор может пройти preflight-checklist без чтения кода
- Ошибки конфигурации/окружения обнаруживаются до начала live работы
- Dataflow и ownership зон понятны новому инженеру

## Итерация 4. Production confidence (pre-prod -> live readiness)

### Цель

Подготовить репо к стабильной эксплуатации и сопровождению командой.

### Объем работ

- Расширить regression matrix:
  - paper smoke
  - replay deterministic checks
  - gateway/runtime compatibility checks
- SLO/alerts рекомендации (health, reconnect rate, lag, guard blocks)
- Release checklist / rollback procedure
- Документ postmortem workflow

### Критерии приемки

- Есть понятный release gate перед live
- Есть воспроизводимый smoke/rollback workflow
- Основные инциденты диагностируются по логам/health without code digging

## Рекомендуемый порядок следующего шага (после утверждения этого аудита)

1. Зафиксировать в runbook/README, что основной путь gateway для runtime — `alor_gateway_transport_runner`.
2. Добавить в docs явное ограничение `cold_start_history_days_back` (без пагинации) и supported workaround.
3. Сделать targeted code-review проход по runtime-only panic paths.
4. Выбрать 3-4 failure scenarios для первой волны интеграционных тестов (начать с broker reject/terminal cancel/Redis publish fail).
5. Только после этого — точечные code changes.
