# State and Restarts (SessionGapStandalone)

## 1) Что хранится в persisted state
Состояние стратегии (`StrategyState::SessionGapStandalone`) содержит:

### 1.1 Константы/индикаторы сессии
- `prev_close`
- `yesterday_range`
- `pre_prev_close`
- `first_min_high`
- `first_min_low`
- `first_hour_price`

### 1.2 Аккумуляторы текущей сессии
- `session_start_ts_utc`
- `session_end_ts_utc`
- `session_high`
- `session_low`
- `session_close`
- `last_dt_ts_utc`
- `traded_session`
- `session_date`

### 1.3 Дедуп и фазы
- `last_bar_ts` — **строго маркер обработанного бара**.
- `phase` (`Flat/PendingEntry/InPosition/PendingExit/Blocked`).
- `phase_last_change_ts_utc`.

---

## 2) Restore flow

### 2.1 `on_runtime_state_restored`
При старте runtime стратегия подтягивает persisted поля (индикаторы, сессионные timestamps, фазу и `last_bar_ts`).

### 2.2 `on_bootstrap_snapshot` + reconcile
После bootstrap snapshot стратегия сверяет phase с фактической позицией у брокера и может корректировать phase.

**Важно:** reconcile не должен «отматывать» дедуп-маркер бара.

---

## 3) Критичные правила

1. **`last_bar_ts` не обновляется ACK/Position событиями.**
   Только обработкой `on_bar`.

2. **`snapshot reconcile` не меняет `last_bar_ts`.**
   Иначе возможна повторная обработка последнего бара после рестарта.

3. **`last_bar_ts` монотонен по bar time.**
   Бары с `close_time_utc <= last_bar_ts` игнорируются (at-least-once защита).

4. **Причины выхода должны быть детерминированы** (например `session_exit` vs `tp/sl`) и одинаковы для runtime/paper/replay.

---

## 4) JSON backward compatibility
При добавлении новых полей в persisted state:
- ставьте `#[serde(default)]` на новых полях,
- задавайте безопасные default-значения,
- добавляйте тест десериализации legacy JSON.

Это позволяет поднимать новые бинарники на старых state snapshots без падений.

---

## 5) Когда нужен `reset_state_on_start`
Используйте `reset_state_on_start = true`, если:
- намеренно хотите начать стратегию «с чистого листа»;
- изменили логику state так, что старое состояние операционно больше невалидно.

Не включайте reset без причины в live: можно потерять дедуп и получить повторные действия.

---

## 6) Безопасный операционный сброс state
1. Остановить runtime.
2. Зафиксировать текущие артефакты/логи (для postmortem).
3. Включить `reset_state_on_start=true` на один запуск **или** очистить конкретный state stream/ключ.
4. Запустить runtime и проверить первые бары: нет ли duplicate processing.
5. Вернуть `reset_state_on_start=false` для нормальной эксплуатации.

---

## 7) Postmortem hints
При инциденте «дубли/пропуски после рестарта» соберите:
- commit SHA,
- persisted state (до/после рестарта),
- readiness snapshot gateway,
- последовательность bar timestamps,
- логи restore/reconcile/phase transitions.
