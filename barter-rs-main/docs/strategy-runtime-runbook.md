# Strategy Runtime Runbook

## 1) Назначение и режимы
`strategy-runtime` исполняет торговую стратегию поверх потоков данных (Redis streams) или replay CSV и поддерживает три основных режима:

- **live** — отправка реальных команд в gateway (`allow_live_orders = true`).
- **paper** — симуляция исполнения без реальных команд.
- **replay/backtest** — прогон исторических баров из CSV, запись отчётов.

Бинарники:
- `strategy_runtime_runner` — основной раннер с TOML-конфигом.
- `session_gap_replay` — специализированный replay для SessionGapStandalone через env.

---

## 2) Конфигурация

### 2.1 Минимальный пример TOML
```toml
redis_url = "redis://127.0.0.1/"
portfolio = "D00000"
exchange = "alor"

[strategy]
strategy_id = "session-gap"
strategy_kind = "session_gap_standalone"
symbol = "USDRUBF"
timezone_offset_hours = 3

[strategy.session_gap]
k_long = 0.5
k_short = 0.46
wait_hours = 2
k_tp_long = 0.28
k_sl_long = 0.68
k_tp_short = 0.28
k_sl_short = 0.65
long_ex_pct = 2.2
short_ex_pct = 2.2
start_cash = 30000
cash_factor = 0.9
max_entry_hour = 19
close_hour = 23
close_minute = 49
session_gap_min = 60.0
exit_offset_min = 20
work_weekends = false

[runtime]
trade_mode = "paper"
allow_live_orders = false

[paper]
enabled = true
trades_csv = "./trades.csv"
summary_json = "./summary.json"

[replay]
enabled = false
```

### 2.2 Ключевые параметры `[strategy.session_gap]`

| Ключ | Смысл | Тип | Default |
|---|---|---:|---:|
| `k_long` | Порог long-сигнала (множитель диапазона) | `f64` | `0.5` |
| `k_short` | Порог short-сигнала | `f64` | `0.46` |
| `wait_hours` | Сколько часов ждать до оценки сигнала | `i64` | `2` |
| `k_tp_long` / `k_tp_short` | Множитель TP от yesterday range | `f64` | `0.28` |
| `k_sl_long` / `k_sl_short` | Множитель SL от yesterday range | `f64` | `0.68` / `0.65` |
| `long_ex_pct` / `short_ex_pct` | Фильтры экстремальности к `pre_prev_close` | `f64` | `2.2` |
| `start_cash` | Базовый капитал для sizing | `f64` | `30000.0` |
| `cash_factor` | Доля капитала под вход | `f64` | `0.9` |
| `max_entry_hour` | Последний час для новых входов | `u32` | `19` |
| `close_hour` / `close_minute` | Конец сессии | `u32` | `23` / `49` |
| `session_gap_min` | Порог rollover в минутах | `f64` | `60.0` |
| `exit_offset_min` | Принудительный выход до конца сессии | `i64` | `20` |
| `work_weekends` | Торговать в выходные | `bool` | `false` |

> Если секция `[strategy.session_gap]` отсутствует или задана частично, незаданные поля берутся из дефолтов.

---

## 3) Запуск

### 3.1 `strategy_runtime_runner`

**Paper (stream mode):**
```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_session_gap_paper_gateway.toml
```

**Live (stream mode):**
```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_session_gap_live_gateway.toml
```

**Replay (CSV mode):**
```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_replay.toml
```

### 3.2 `session_gap_replay`
```bash
set -a; source strategy-runtime/rt_session_gap_standalone.env; set +a
cargo run -p strategy-runtime --bin session_gap_replay
```

### 3.3 `REPLAY_OUTPUT_DIR`
Для replay артефактов используйте отдельную директорию:
```bash
REPLAY_OUTPUT_DIR=./strategy-runtime/replay_out_local \
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_replay.toml
```

---

## 4) Выходные артефакты
Обычно создаются:
- `trades.csv` — сделки рантайма.
- `summary.json` — агрегированный отчёт.
- `replay_out/parity_report.json` — сводка сравнения runtime/reference в replay.

Ищите путь в `[paper]`, `[backtest]`, `[replay]` (`trades_csv`, `summary_json`, `output_dir`).

---

## 5) Логи и диагностика
Ключевые события, которые стоит искать:
- `restored indicators from runtime state` — применение persisted state.
- `session rollover summary` — rollover по разрыву сессии.
- `live phase transition` — переходы фаз (`Flat`, `PendingEntry`, `InPosition`, `PendingExit`, `Blocked`).
- `state corrected by broker snapshot` — reconcile после bootstrap snapshot.

Рекомендуемый запуск с логами:
```bash
RUST_LOG=info,strategy_runtime=debug \
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_session_gap_runtime.toml
```

---

## 6) Типовые проблемы

### 6.1 «Нет сделок»
Проверить:
1. `strategy_kind = "session_gap_standalone"`.
2. Есть ли `prev_close/yesterday_range` в state после restore.
3. Не срабатывает ли live guard (`allow_live_orders`, `gateway_phase`, origin данных).
4. Время/таймзона: `timezone_offset_hours`, `close_hour`, `max_entry_hour`.

### 6.2 «Дубли сделок»
Проверить:
1. Дедуп по `last_bar_ts`/`last_processed_bar_ts` не сброшен вручную.
2. Не включён ли неожиданный `reset_state_on_start = true`.
3. Поток баров действительно монотонен по `close_time_utc`.

### 6.3 «Runtime != reference в replay»
Проверить:
1. Совпадают ли входные CSV и таймзона.
2. `price_tolerance` и `strict_dedup` в `[replay]`.
3. Приоритет причин выхода на одном баре (например `session_exit` против `tp/sl`) фиксирован и должен быть одинаков в сравниваемых прогонах.

### 6.4 «Конфиг не применился»
Проверить:
1. Переданный `--config`.
2. env overrides (`SESSION_GAP_*`, `REPLAY_*`, `TRADE_MODE` и т.д.).
3. Опечатки в секциях TOML (`[strategy.session_gap]`).

---

## 7) Preflight checklist
Перед запуском в live/paper:
- [ ] Redis доступен.
- [ ] Потоки (bars/orders/trades/positions/snapshots) совпадают с portfolio/source.
- [ ] Режим (`trade_mode`) соответствует ожиданию.
- [ ] Для live: `allow_live_orders=true` только после проверки readiness gateway.
- [ ] Настроен `RUST_LOG` и путь для отчётов/артефактов.
