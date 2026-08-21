# ALOR reference engineering audit

**Дата аудита:** 2026-08-21  
**Роль документа:** долгоживущий reference/oracle для миграции ALOR → broker-neutral core + FINAM  
**Исходный пакет:** `bybit_barter_test_sanitized 2(5).zip`  
**SHA-256 исходного пакета:** `dbd537f975137d5e3be5984ee1c052da9a81e8b71dcbeb7e8557b931cc0fe313`  
**Archive safety:** 604 entries, unsafe paths не обнаружены  
**Статус:** REFERENCE ACCEPTED — использовать как oracle операционной зрелости, но не как шаблон для механического копирования архитектурных долгов

---

## 1. Executive summary

Исходный ALOR-проект является не просто HTTP/WebSocket-коннектором к брокеру, а зрелым торговым сервисным комплексом с выраженными операционными контрактами между broker truth, gateway, Redis transport, runtime стратегий, persistent state, risk controls и операторским контуром.

Главный вывод для миграции:

> **Нельзя считать задачу перехода на FINAM заменой ALOR API-клиента.**
> Необходимо восстановить в broker-neutral архитектуре наблюдаемое поведение и safety-гарантии всего ALOR service contract.

ALOR-проект следует использовать одновременно в двух ролях:

1. **Oracle зрелости** — что новая система обязана уметь до допуска к production/live.
2. **Oracle семантики стратегий** — как одна и та же стратегия должна принимать решения на одинаковом canonical input.

При этом найденные в ALOR архитектурные долги **не являются требованиями parity**. Их следует исправить в новой системе, а не переносить.

### Итоговая оценка

| Область | Оценка | Комментарий |
|---|---:|---|
| Operational live architecture | Высокая | Реальные restart/reconnect/readiness/recovery contracts |
| Strategy/runtime safety | Высокая | Fail-closed host lifecycle, stale-bar protection, risk state |
| Broker-truth/reconciliation discipline | Высокая | Broker truth используется как authoritative external state |
| Test maturity | Высокая | Широкая unit/e2e/negative/operational поверхность |
| Observability/operator runbooks | Высокая | Много incident-derived документов и diagnostics |
| Broker independence | Низкая/средняя | ALOR/CWS детали протекают в protocol/runtime contracts |
| Crash-safe execution identity | Средняя | Есть критичные process-local/idempotency gaps |
| Schema governance | Средняя | Дублированные event DTO и permissive versioning |
| Maintainability | Средняя | Крупные runtime modules, исторический special-case growth |

---

## 2. Архитектурная модель исходника

Упрощённый production flow:

```text
ALOR REST / WebSocket / CWS
        |
        v
   ALOR Gateway
        |
        | broker events / health / readiness / commands
        v
   Redis Streams
        |
        v
 Strategy Runtime Host
        |
        +--> strategy semantics
        +--> durable/persisted runtime state
        +--> risk gate / ownership / trade ledger
        |
        v
   command stream
        |
        v
   ALOR Gateway
        |
        v
      Broker
```

Ключевая архитектурная характеристика: runtime зависит не только от рыночных bars, но и от более широкого **service contract**:

- broker-truth snapshots/events;
- orders/trades/positions lifecycle;
- command ACK lifecycle;
- request/broker identity correlation;
- Redis consumer-group semantics;
- restart reconciliation;
- reconnect/gap handling;
- health/readiness;
- host-owned live guard;
- risk/persistence state;
- operator intervention states.

---

## 3. Сильные стороны, которые необходимо сохранить

### 3.1. Broker truth раньше strategy authority

Bootstrap runtime в ALOR выстроен в правильном порядке: сначала восстанавливается внешний факт о брокере и инфраструктуре, затем локальная стратегия получает право интерпретировать своё состояние.

Концептуальный порядок:

1. consumer groups / transport preparation;
2. broker snapshots;
3. persisted runtime state;
4. persistent risk-gate state;
5. strategy bootstrap notification;
6. runtime-state-restored notification;
7. history warmup;
8. pending/ACK recovery;
9. orders/stop-orders;
10. trades;
11. positions;
12. bars;
13. ordinary operation.

**Invariant:** локальный persisted strategy state не имеет права автоматически победить свежую broker truth.

### 3.2. Host-owned live guard

Для торговли недостаточно факта, что gateway-процесс запущен. Live допуск зависит от совокупности условий, включая:

- `trade_mode=Live`;
- явный `allow_live_orders`;
- новый live bar после запуска;
- `GatewayPhase::LiveReady`;
- свежий gateway health;
- readiness;
- WebSocket connected;
- command/execution authorization.

Это правильный fail-closed pattern.

### 3.3. Atomic command + strategy-state publication boundary

Runtime использует Redis transactional boundary (`MULTI/EXEC`) для публикации command и соответствующего strategy state.

Это снижает риск расхождения вида:

- стратегия уже считает intent опубликованным, а command отсутствует;
- command появился, а связанное состояние стратегии не зафиксировано.

Важно: это **не** решает exactly-once broker execution; execution crash windows должны закрываться отдельным durable journal/reconciliation contract.

### 3.4. Critical event delivery и backpressure

Orders/trades/positions/bars и другие critical events не рассматриваются как необязательная телеметрия. При деградации Redis publisher:

- critical publication retry-ится;
- sink переводится в degraded;
- readiness снимается;
- admission новых команд может блокироваться.

Это важный safety pattern для FINAM.

### 3.5. Restart/reconnect semantics

ALOR-проект содержит зрелые решения для сценариев, которые часто отсутствуют в greenfield gateway:

- generation-aware reconnect;
- resubscribe;
- gap recovery;
- stale-health detection;
- startup replay suppression;
- ожидание первого нового bar после restart;
- deferred exits;
- safe-mode при неизвестном owner;
- partial-fill accumulation;
- trade-before-ACK / trade-before-order ordering;
- orphan classification только после settlement/reconciliation;
- conservative dirty-start policy.

### 3.6. Incident-derived документация

Сильная сторона проекта — документация фиксирует не абстрактные пожелания, а уже встретившиеся классы отказов:

- request-id skew;
- duplicate/deferred exits;
- partial-fill races;
- paper/live contamination;
- reconnect/gap-sync;
- stale readiness;
- action-scoped execution transport;
- broker residual state;
- risk-gate persistence.

Эти документы должны использоваться как источник acceptance scenarios для FINAM.

---

## 4. Test surface

Статический аудит исходника выявил около **489 Rust test cases/attributes**:

| Crate / область | Примерное количество |
|---|---:|
| `strategy-runtime` | ~395 |
| `alor-gateway` | ~80 |
| `alor-types` | 9 |
| `alor-protocol` | 5 |
| **Всего** | **~489** |

Кроме unit tests присутствуют runtime/restart/live-guard/Redis/integration/operational сценарии.

### Ограничение независимого аудита

В среде review отсутствовали `cargo` и `rustc`, поэтому тесты не были независимо бинарно воспроизведены. В этом документе различаются:

1. наличие теста в source tree;
2. project evidence о PASS;
3. независимый reviewer execution.

Для production acceptance FINAM желательно всегда иметь третий уровень для критичных gates.

---

## 5. Найденные архитектурные долги ALOR, которые НЕ нужно переносить

### P0-MIG-01 — production idempotency process-local

В production transport runner используется `InMemoryIdempotency`, хотя в проекте существует Redis-вариант.

После restart процесса история command dedup не является полноценным durable execution fact.

**Требование к FINAM:** использовать durable execution lifecycle/journal, а не process-local dedup.

Рекомендуемая state machine:

```text
Received
  -> Admitted
  -> AttemptDurable
  -> TransportEntered
  -> BrokerAccepted / OutcomeUnknown / DefinitelyNotSent
  -> Reconciled
  -> Settled
```

### P0-MIG-02 — Redis SET NX недостаточен как exactly-once execution contract

Даже durable `SET NX request_id` до send создаёт crash windows:

**A.** marker записан → crash до send → команда после restart выглядит duplicate, хотя broker effect не было.

**B.** broker принял request → crash до durable ACK/outcome → duplicate marker не восстанавливает `broker_order_id` и исходный результат.

**Требование к FINAM:** attempt-before-send journal + no-blind-retry + broker-truth reconciliation.

### P0-MIG-03 — request_id ↔ broker_order_id correlation process-local

ALOR gateway использует process-local mapping для восстановления correlation между runtime request и broker order event.

Restart может уничтожить эту связь.

**Требование к FINAM:** durable identity chain:

```text
StrategyRequestId
   <-> ClientOrderId
   <-> BrokerOrderId
   <-> BrokerTradeId(s)
```

с broker-native `client_order_id`, если доступен, плюс локальная durable correlation history.

### P0-MIG-04 — account boundary недостаточно authoritative

Command payload несёт portfolio/account-related данные, а gateway исторически доверяет части command scope.

**Требование к FINAM:** gateway сам authoritative по:

- account;
- instrument allowlist;
- venue/board;
- allowed actions;
- quantity/notional budget;
- execution capability.

Runtime не должен расширять execution authority содержимым команды.

### P0-MIG-05 — broker-specific protocol leakage

Canonical protocol содержит ALOR/CWS-specific diagnostics и смешанные identifier forms.

**Требование к FINAM/broker-neutral core:** broker-specific metadata отделяется от canonical domain.

Например:

```text
BrokerOrderId(String)
BrokerTradeId(String)
ClientOrderId(String)
```

должны быть opaque canonical IDs независимо от того, число или строку возвращает конкретный broker.

### P0-MIG-06 — duplicated event schemas

Gateway и runtime имеют отдельные определения одних и тех же broker events и поддерживают совместимость через JSON/Serde conventions.

Это создаёт риск silent schema drift.

**Требование:** один authoritative broker-neutral event/domain crate, импортируемый producer и consumer.

---

## 6. P1 findings

### P1-01 — fail-open-ish configuration parsing

Исторически встречаются defaults, при которых неизвестное значение может интерпретироваться как торговое действие, например unknown mode/style/side falling back к Live/Market/Buy-подобному поведению.

**FINAM rule:** unknown enum/config = startup/admission failure. Никаких trading defaults на ошибочном input.

### P1-02 — execution capability не является полноценным live-readiness dimension

Документация ALOR позднее требует action-scoped execution path, но generic config/defaults могут сохранять legacy execution mode.

**FINAM rule:** readiness должна включать fingerprint concrete execution capabilities, а не просто `gateway_ready=true`.

### P1-03 — `f64` на broker/order boundary

Для price/quantity это создаёт ненужный класс rounding/step ошибок.

**FINAM rule:** Decimal или integer ticks/lots на broker boundary. `f64` допустим внутри indicator math.

### P1-04 — instrument contract недостаточно богат

Новый broker-neutral registry должен authoritative связывать:

- canonical instrument;
- broker symbol;
- venue/MIC/board;
- tick size;
- lot size;
- quantity semantics;
- currency;
- trading schedule;
- tradability;
- order capabilities;
- account eligibility.

### P1-05 — крупные runtime modules / special-case growth

ALOR runtime достиг значительных размеров. Большой rewrite runtime одновременно с broker migration повышает regression risk.

**Migration rule:** сначала broker-neutral seam + parity, затем отдельный refactor после operational equivalence.

### P1-06 — protocol/schema versioning permissive

Backward compatibility во многом держится на `serde(default)`.

**FINAM rule:** explicit min/current schema, fixtures, migrations, unknown-field behavior, compatibility tests.

### P1-07 — historical documentation/status drift

В разных исторических документах статусы некоторых failure families расходятся.

**FINAM rule:** один authoritative `current-status`/acceptance ledger; исторические отчёты immutable, но не определяют current truth.

### P1-08 — shutdown delivery не заменяет broker-truth recovery

Даже blocking publisher имеет конечный shutdown/drain boundary. Поэтому отсутствие event в Redis после crash/shutdown нельзя считать доказательством отсутствия broker effect.

**FINAM rule:** broker truth + reconciliation является обязательной recovery authority.

---

## 7. Неприкосновенные migration invariants

Ниже — минимальный oracle contract, который должен сохраниться или стать сильнее в broker-neutral + FINAM системе.

1. **Broker truth before strategy authority.**
2. **Dirty start не разрешает blind entry.**
3. **Fresh broker readiness обязательна для live.**
4. **Первый intent после restart не исполняется на stale/replayed bar.**
5. **Entry и risk-reducing exit имеют разные failure semantics.**
6. **Закрытая торговая сессия не оставляет вечный pending exit.**
7. **Command и strategy state имеют согласованную durable publication boundary.**
8. **ACK/order/trade/position могут приходить в любом порядке.**
9. **Partial fill — first-class lifecycle.**
10. **Unknown broker outcome → reconciliation, никогда blind retry.**
11. **Duplicate request не создаёт duplicate broker effect.**
12. **Restart восстанавливает ownership и correlation.**
13. **Unowned/unknown broker state → safe/close-only/manual state.**
14. **Backpressure/degraded critical sink снимает readiness.**
15. **Reconnect требует resync/reconciliation до `LiveReady`.**
16. **Paper/backtest не импортирует live exposure как собственную paper position.**
17. **History может warm-up indicators, но не порождает stale live orders.**
18. **Risk-gate state durable и отделён от transient strategy snapshot.**
19. **Каждый block имеет operator-visible reason code.**
20. **Каждое открытие real execution surface проходит отдельный promotion gate.**

---

## 8. Как использовать ALOR oracle при FINAM review

Для каждого FINAM stage проверять две разные parity dimensions.

### 8.1. Semantic parity

На одинаковом canonical input стратегия должна давать одинаковое решение:

- owner/state;
- intent/action;
- side;
- quantity;
- order role;
- pending/deferred state;
- risk-gate decision;
- day/session lifecycle.

### 8.2. Operational parity

Новая система должна не хуже ALOR переживать:

- restart;
- reconnect;
- stale data;
- partial fills;
- duplicate delivery;
- ACK/order/trade reordering;
- broker reject;
- timeout after possible send;
- unknown residual order/position;
- Redis outage/backpressure;
- kill switch;
- ownership conflict;
- dirty start;
- session close.

### 8.3. Допустимые различия

Не являются regression сами по себе:

- broker-native ID values;
- broker event timestamps при сохранении causal correctness;
- transport-specific diagnostics;
- более строгий fail-closed behavior;
- улучшенная durable execution model;
- Decimal вместо `f64`;
- новый canonical protocol.

---

## 9. Recommended acceptance hierarchy для нового проекта

```text
Source semantics parity
    ↓
Market-data parity
    ↓
Broker-truth/bootstrap parity
    ↓
Paper lifecycle parity
    ↓
Crash/restart/reconciliation parity
    ↓
Multi-session operational paper parity vs ALOR
    ↓
Bounded FINAM live-micro
    ↓
Repeated clean live-micro evidence
    ↓
Port next strategy
```

Нельзя заменять предыдущий слой последующим. Например, успешный один реальный FINAM order не компенсирует отсутствие multi-session paper parity.

---

## 10. Финальный verdict

**ALOR project: ACCEPTED AS REFERENCE/ORACLE.**

Его следует считать эталоном **операционных гарантий и runtime semantics**, но не кодовым шаблоном. Broker-neutral + FINAM проект должен:

- сохранить перечисленные invariants;
- устранить ALOR process-local identity/idempotency gaps;
- усилить broker/account/instrument authority;
- иметь единый canonical domain;
- иметь crash-safe attempt/reconciliation model;
- доказать parity сначала в repeatable paper/shadow режиме;
- только затем переходить к strategy-driven live micro.

Этот документ рекомендуется версионировать отдельно и изменять только при обнаружении нового существенного ALOR invariant или нового подтверждённого incident-derived behavior.
