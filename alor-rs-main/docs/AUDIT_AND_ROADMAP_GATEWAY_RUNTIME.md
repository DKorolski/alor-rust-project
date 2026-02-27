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

6. Подготовлен и использовался replay-контур как компенсация ограничения history bootstrap в gateway:
- при подгружаемой истории порядка ~5000 баров (примерно 3-4 дня в `gateway.live`) глубокая проверка логики выполнялась через replay;
- replay-тест показал воспроизведение логики standalone backtest;
- наблюдались небольшие отклонения, но они были оценены как несущественные и приемлемые.

Вывод для аудита:
- базовая operational механика (reconnect/resync, dedup, paper/live path) имеет **положительное эмпирическое подтверждение**;
- replay используется как практический workaround/verification path при ограниченном cold-start history;
- приоритет смещается на negative/failure-path coverage, формализацию контрактов и документирование допустимых расхождений replay vs standalone.

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

7. Есть replay-контур для проверки стратегии при ограниченной глубине history bootstrap:
- replay-проверка воспроизводит standalone backtest с допустимыми небольшими отклонениями.

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

## Execution Status (первая волна failure scenarios)

Статус выполнения матрицы (`docs/failure-test-matrix.md`) на текущий момент:

- `FT-01` (cancel terminal order) — `PASS`
  - manual cancel на historical terminal `order_id` через `cmd.orders`
  - `gateway -> cws delete:limit -> httpCode=400 -> cmd.ack Rejected`
  - `command_consumer` остается жив
- `FT-02A` (insufficient funds / limits) — `PASS`
- `FT-02B` (price out of range) — `PASS`
- `FT-02G` (BookOrCancel immediate execution reject) — `PASS`
- `FT-02F` (invalid price step) — `PARTIAL`
  - в текущем path нередко не дает broker reject (вероятна нормализация/округление цены)
- `FT-03` (Redis publish failure around `publish_command_and_state`) — `PASS`
  - подтвержден сценарий с искусственной задержкой перед publish:
    `RUNTIME_ENABLE_TEST_HOOKS=true` + `RUNTIME_TEST_DELAY_BEFORE_PUBLISH_MS=5000`
  - при restart Redis в delay-window тестовый `request_id` отсутствует в `cmd.orders`,
    в gateway нет `command received`, флуда заявок нет
- `FT-04` (stale health -> runtime BLOCKED) — `PASS`
  - runtime корректно переходит `ALLOWED -> BLOCKED` по `gateway_health_stale`
  - readiness переключается в `false`

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
- зафиксировать policy для оценки расхождений replay vs standalone (что считается допустимым отклонением).

## Deep Dive: runtime-only panic path audit (production code, не тесты)

Ниже — targeted pass по `unwrap/expect/panic` в production-path. Цель: отделить реальный риск от шумных test-only мест.

### Итог высокого уровня

- Массовой production panic-проблемы (как в первом коннекторе) **не выявлено**.
- Большинство совпадений `unwrap/expect/panic` находятся в:
  - `#[cfg(test)]` блоках,
  - тестовых сценариях стратегий,
  - replay/утилитных бинарях.
- В production-path остались точечные места (в основном hardening/гигиена, а не архитектурная аварийность).

Это хорошо согласуется с твоими эмпирическими результатами:

- live reconnect/resync проходили;
- артефакты последовательны и без дублей;
- paper/live базовые сценарии отрабатывают.

### Что проверено вручную (production / near-production)

#### 1) `strategy-runtime` runner: SIGTERM handler install (`expect`)

- `strategy-runtime/src/bin/strategy_runtime_runner.rs:79`

Текущее поведение:
- `tokio::signal::unix::signal(...).expect("install SIGTERM handler")`

Оценка риска:
- `P2` (операционно редко падает, но это реальный startup panic path в production binary).

Рекомендация:
- заменить на graceful fallback:
  - `warn!` + работа только через `ctrl_c`.

#### 2) Time conversions в `alor-gateway` helper (`unwrap` fallback)

- `alor-gateway/src/ws_hub.rs:959`
- `alor-gateway/src/supervisor.rs:1130`

Паттерн:
- `DateTime::from_timestamp(...).unwrap_or_else(|| Utc.timestamp_opt(0,0).unwrap())`

Оценка риска:
- `P2` (низкий риск, но формально panic path есть во внутреннем fallback).

Комментарий:
- на практике почти безопасно, но легко зачистить.

#### 3) Timezone/timestamp mapping в `alor-gateway/src/strategy_adapter.rs`

- `alor-gateway/src/strategy_adapter.rs:129`
- `alor-gateway/src/strategy_adapter.rs:133`
- `alor-gateway/src/strategy_adapter.rs:235`

Текущее поведение:
- `FixedOffset::east_opt(...).unwrap()`
- `timestamp_opt(...).single().expect(...)`

Оценка риска:
- `P1/P2` (production path, зависит от корректности входных timestamps; при поврежденных данных/контракте возможен panic).

Рекомендация:
- перевести на `Result`/skip + diagnostics.

#### 4) Timezone/timestamp conversion в `session_gap_standalone` production logic

- `strategy-runtime/src/strategies/session_gap_standalone.rs:139`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:142`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:1019`

Текущее поведение:
- `expect("valid fixed offset")`
- fallback через `Utc.timestamp_opt(...).unwrap()`

Оценка риска:
- `P1/P2` (стратегический production path; риск небольшой, но это уже не test-only код).

Рекомендация:
- hardening в отдельном небольшом patch-set (без изменения торговой логики).

### Что НЕ является production blocker (хотя grep показывает много совпадений)

#### Test-only места (подтверждено по `#[cfg(test)]`)

- `alor-gateway/src/router.rs` (`expect("position/order/trade")`) — test-only (`#[cfg(test)]` с `router.rs:366`)
- `alor-gateway/src/cws_client.rs` payload shape `expect(...)` — test-only (`#[cfg(test)]` с `cws_client.rs:475`)
- большая часть `strategy-runtime/src/runtime.rs` `unwrap` на линиях ~2564+ — test module (`#[cfg(test)]` с `runtime.rs:2442`)
- `strategy-runtime/src/state.rs` `unwrap/panic` — test module (`#[cfg(test)]` с `state.rs:191`)
- значительная часть `session_gap_standalone.rs` после ~1072 — test section (`#[cfg(test)]` с `session_gap_standalone.rs:1072`)

#### Utility / replay binaries

- `strategy-runtime/src/bin/session_gap_replay.rs` содержит `unwrap/expect`, но это отдельный replay-утилитный путь, не основной runtime live/paper loop.

### Вывод для roadmap

По panic-path hygiene:

- можно сделать **малый low-risk hardening patch** (небольшая итерация) без влияния на core механику:
  - убрать `expect` в `strategy_runtime_runner` (SIGTERM handler)
  - заменить production `unwrap/expect` в timezone/timestamp conversions на `warn + fallback` / `Result`

Это не должно быть приоритетнее failure-mode testing, но хорошо подходит как “быстрая инженерная уборка” перед deeper changes.

## Deep Dive: стратегия `session_gap_standalone` (основной production path)

Аудит этого раздела основан на том, что ты запускаешь runtime с:

- `cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.live.toml`
- `strategy_id = "session_gap_standalone"`

То есть именно эта стратегия является **главным production path** для текущего контура.

### Что выглядит сильно (по коду и по тестам)

#### 1) Явная live state-machine с фазами

Стратегия использует `SessionGapLivePhase` и явно разделяет состояния:

- `Flat`
- `PendingEntry`
- `InPosition`
- `PendingExit`
- `Blocked`

Это хороший признак:
- поведение в live не “размазано” по флагам;
- таймауты/реакции на ack/position выражены через фазовые переходы;
- легче диагностировать и тестировать.

Ключевые места:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:678`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:829`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:872`

#### 2) Встроенный dedup на баре + перенос маркера в persisted state

На входе `on_bar`:

- проверка `last_processed_bar_ts`
- игнор баров `<= last_processed_bar_ts`

Это критически важно для at-least-once модели Redis Streams и рестартов.

Ключевое место:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:639`

Плюс:
- стратегия сохраняет `last_bar_ts` и индикаторы в `StrategyState::SessionGapStandalone`
- `on_runtime_state_restored` восстанавливает их обратно в RAM-поля стратегии

Ключевые места:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:565`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:982`

Это отлично согласуется с твоим наблюдением про последовательные артефакты и отсутствие дублей.

#### 3) Reconcile с bootstrap snapshot встроен в стратегию (а не только в runtime)

`on_bootstrap_snapshot()` вызывает `transition_live_reconcile_with_snapshot(...)`, который:

- сравнивает persisted phase и фактическую позицию брокера (`snapshot_qty`)
- корректирует state (`PendingEntry -> InPosition`, `InPosition/PendingExit -> Flat`)
- при этом старается не ломать persisted timestamps/markers

Ключевые места:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:415`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:940`

Это архитектурно сильное решение, потому что reconcile знает доменную phase-логику стратегии.

#### 4) Live-guard gating встроен прямо в `on_bar`

Перед эмиссией live-intents стратегия проверяет:

- `trade_mode == Live`
- `bar.origin == Live`
- `allow_live_orders`
- `gateway_phase == LiveReady`

Ключевое место:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:525`

Это хорошо:
- безопасность не только в runtime layer, но и на уровне стратегии;
- “двойной guard” снижает шанс случайной live-эмиссии.

#### 5) Хорошее покрытие тестами именно этой стратегии

По именам тестов видно, что уже покрыты важные инварианты:

- restore/reconcile и сохранение `last_bar_ts`
- live guard blocked path
- ack timeout -> blocked
- rollover behavior
- conflict bars / forced exit

Это сильная сторона проекта.

### Основные риски / зоны внимания (не как “всё плохо”, а как next-step hardening)

#### A. `on_order()` фактически no-op, а критическая live эволюция фазы сидит в `on_ack()` + `on_position()`

Текущее поведение:

- `on_order(...)` возвращает `Vec::new()`
- state transitions идут через:
  - `on_ack()` (ставит `acked = true`, блокирует на reject)
  - `on_position()` (фиксирует фактическое изменение позиции)

Ключевое место:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:868`

Почему это не обязательно плохо:
- это может быть сознательная модель (“order event informational, position is source-of-truth”).

Но риск:
- некоторые broker edge-cases отражаются в `order` раньше/детальнее, чем в `position`;
- при нестандартных последовательностях событий можно получить затяжное `Pending*`/`Blocked`.

Рекомендация:
- зафиксировать это как явный контракт стратегии:
  - source-of-truth для progression — `position` + `ack`
  - `order` используется только runtime ledger/observability layer.

#### B. `AckStatus::Duplicate` считается успешным ack (`acked = true`) — корректно, но требует тестов на повторную доставку и stale duplicate

В `on_ack()`:

- `Accepted | Confirmed | Duplicate` -> `acked = true`

Ключевое место:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:841`

Это, вероятно, правильное решение для idempotent command path.

Риск:
- duplicate ack от старого запроса/фазы (если request_id collision/bug вне стратегии) может сдвинуть фазу в “ack получен”.

Что смягчает риск:
- сравнение по `request_id` внутри текущей `Pending*` фазы.

Что стоит добавить:
- targeted тест на late duplicate ack after phase changed.

#### C. `Blocked` фаза выглядит терминальной без встроенного auto-recovery механизма

Текущее поведение:

- `SessionGapLivePhase::Blocked { .. } => phase` в `on_bar`
- стратегия сама не пытается восстановиться из `Blocked`

Ключевое место:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:816`

Это может быть правильным safety-by-default поведением.

Но нужно явно решить и задокументировать:

- кто и как “разблокирует” стратегию:
  - operator action / restart / reset state
  - reconcile via snapshot/position
  - специальный runtime command

С учетом live сценариев и troubleshooting это важная operational policy.

#### D. Broker reject handling сейчас грубо агрегируется в `Blocked(reason = "ack_failed:...")`

В `on_ack()` reject/error/expired переводят в:

- `Blocked { reason: format!("ack_failed:{:?}", ack.status) }`

Ключевое место:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:846`

Риск:
- теряется детализация причины (`insufficient funds`, broker code, cws_http_code и т.п.) на уровне phase reason.

Рекомендация:
- расширить `Blocked.reason` (или structured blocked metadata) хотя бы ключевыми полями:
  - `ack.status`
  - `ack.error_code`
  - `ack.cws_http_code`

Это особенно полезно для твоего next-step сценария с troubleshooting tests.

#### E. Таймауты `entry_*` / `exit_*` завязаны на bar-driven progression

В `on_bar()` таймауты `ack`/`fill` проверяются через разницу `bar.close_time_utc - sent_ts`.

Ключевые места:

- `strategy-runtime/src/strategies/session_gap_standalone.rs:718`
- `strategy-runtime/src/strategies/session_gap_standalone.rs:800`

Плюс:
- стратегия детерминирована относительно bar timeline.

Риск/нюанс:
- при разреженном потоке баров, паузах/сессиях и unusual latency фактическое wall-clock время может сильно отличаться.
- это не обязательно ошибка, но важно документировать как **bar-time driven timeouts**, а не wall-clock.

### Что особенно важно проверить следующей волной тестов (для этой стратегии)

С учетом твоих комментариев о непокрытых сценариях:

1. Terminal cancel / stale command ack path
- попытка отмены уже `filled/canceled/rejected`
- ожидаемое поведение strategy/runtime:
  - не сломать фазу
  - не уйти в ложный `Blocked`, если это операционно допустимый no-op
  - или уйти в `Blocked`, но с понятной причиной

2. Insufficient funds / broker reject
- проверить:
  - `on_ack()` перевод в `Blocked`
  - качество причины в логах/runtime state
  - отсутствие повторной эмиссии intents без operator action

3. Late ack / duplicate ack after phase transition
- особенно после `PendingEntry -> InPosition` по `on_position()`
- проверить, что поздний duplicate ack не портит state

4. Position snapshot reconcile в конфликтных кейсах
- snapshot показывает позицию, а фаза уже `Blocked`
- snapshot показывает `Flat`, а фаза `PendingExit`
- убедиться, что `last_bar_ts` и phase timestamps не деградируют

### Вывод по стратегии `session_gap_standalone`

Это не “игрушечная” стратегия в терминах runtime engineering:

- есть state-machine,
- restore/reconcile,
- dedup,
- live guard gating,
- хорошие тесты на state invariants.

С учетом твоих успешных live/paper проверок, основной следующий выигрыш — не переписывать логику стратегии, а:

- расширить failure-path coverage,
- улучшить диагностику причин `Blocked`,
- формализовать operational policy разблокировки/восстановления,
- зафиксировать допустимые отклонения replay vs standalone (как regression baseline).

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

## Рекомендуемый следующий шаг (после выполнения первой волны)

1. Зафиксировать “phase-1 complete” в release/checklist документах:
   - что именно покрыто (`FT-01/02A/02B/02G/03/04`)
   - что осталось (`FT-02C/02D/02E`, `FT-05`, `FT-06`)
2. Убрать/ограничить тестовый хук `RUNTIME_TEST_DELAY_BEFORE_PUBLISH_MS`:
   - оставить как тестовый инструмент, но явно пометить non-prod usage.
3. Перейти к `FT-05` (late/duplicate ack) через controlled integration harness.
4. Формализовать policy для `FT-06` (replay vs standalone tolerance thresholds).
5. После этого — targeted code changes по остаточным findings (по приоритету из матрицы).

## Связанный документ для следующего этапа

- Матрица отказных сценариев (execution checklist): `docs/failure-test-matrix.md`
- Для ускорения `FT-01..FT-03` добавлен test strategy path `mock_live_probe` (`strategy-runtime/src/strategies/mock_live_probe.rs`, config `configs/runtime.mock-live.toml`)
- Для воспроизводимого `FT-03` добавлен тестовый pre-publish delay hook:
  `RUNTIME_ENABLE_TEST_HOOKS=true` + `RUNTIME_TEST_DELAY_BEFORE_PUBLISH_MS=<ms>` (использовать только в тестовых прогонах).
