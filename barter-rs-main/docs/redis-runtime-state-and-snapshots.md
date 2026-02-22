# Redis CLI: проверка runtime state и snapshots

Этот документ — быстрый набор `redis-cli` команд, чтобы посмотреть:
- что лежит в stream runtime state (`streams.runtime_state`),
- что лежит в snapshot stream (`streams.snapshots`).

> Используйте имена stream из вашего runtime-конфига (`./configs/runtime.*.toml`, секция `[streams]`).

---

## 1) Подключение

```bash
redis-cli -u redis://127.0.0.1/
```

Если нужен пароль:

```bash
redis-cli -u redis://:PASSWORD@127.0.0.1:6379/0
```

---

## 2) Проверка stream имён из конфига

Пример типичных имен:

- `runtime_state = "runtime.state.session_gap_standalone.paper.7502T0U"`
- `snapshots = "broker.snapshots.7502T0U"`

Можно быстро достать строки из конфига:

```bash
rg -n "^runtime_state\s*=|^snapshots\s*=" ./configs/runtime.paper.toml
```

---

## 3) Runtime state stream

### 3.1 Метаданные stream

```bash
redis-cli XINFO STREAM runtime.state.session_gap_standalone.paper.7502T0U
```

Смотрите:
- `length`
- `last-generated-id`
- `first-entry` / `last-entry`

### 3.2 Последние N записей

```bash
redis-cli XREVRANGE runtime.state.session_gap_standalone.paper.7502T0U + - COUNT 10
```

### 3.3 Первая запись (для проверки формата)

```bash
redis-cli XRANGE runtime.state.session_gap_standalone.paper.7502T0U - + COUNT 1
```

### 3.4 Только размер stream

```bash
redis-cli XLEN runtime.state.session_gap_standalone.paper.7502T0U
```

---

## 4) Snapshot stream

### 4.1 Метаданные stream

```bash
redis-cli XINFO STREAM broker.snapshots.7502T0U
```

### 4.2 Последние записи snapshot

```bash
redis-cli XREVRANGE broker.snapshots.7502T0U + - COUNT 10
```

### 4.3 Фильтр по payload (через grep)

```bash
redis-cli XREVRANGE broker.snapshots.7502T0U + - COUNT 20 | rg "SnapshotOrders|SnapshotPositions|payload"
```

---

## 5) Проверка consumer group runtime

Если runtime читает команды/snapshots через consumer group, полезно смотреть pending:

```bash
redis-cli XPENDING cmd.orders.7502T0U strategy-runtime
```

Детализация pending:

```bash
redis-cli XPENDING cmd.orders.7502T0U strategy-runtime - + 20
```

Информация по group:

```bash
redis-cli XINFO GROUPS cmd.orders.7502T0U
```

Информация по consumer:

```bash
redis-cli XINFO CONSUMERS cmd.orders.7502T0U strategy-runtime
```

---

## 6) Полезный one-liner для живой диагностики

```bash
watch -n 2 '
  echo "== runtime_state ==";
  redis-cli XLEN runtime.state.session_gap_standalone.paper.7502T0U;
  redis-cli XREVRANGE runtime.state.session_gap_standalone.paper.7502T0U + - COUNT 1;
  echo;
  echo "== snapshots ==";
  redis-cli XLEN broker.snapshots.7502T0U;
  redis-cli XREVRANGE broker.snapshots.7502T0U + - COUNT 1;
'
```

---

## 7) Что считается нормой

- `runtime_state` не пустой и обновляется по мере работы стратегии.
- `broker.snapshots.*` содержит как минимум snapshot orders/positions после старта gateway.
- `XINFO STREAM ...` не показывает аномально старый `last-generated-id` при активной системе.

Если stream пустой:
1. Проверить корректность имени stream в runtime/gateway конфиге.
2. Проверить, что gateway/runtime реально запущены и подключены к тому же Redis.
3. Проверить, что в логах нет ошибок сериализации/transport.
