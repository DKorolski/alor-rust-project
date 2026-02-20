# Replay / Backtest Guide

## 1) Когда использовать
- **Replay**: проверка детерминизма и паритета runtime на фиксированном наборе CSV.
- **Backtest**: быстрая проверка стратегии на исторических данных без live-инфраструктуры.

---

## 2) Запуск replay через `strategy_runtime_runner`

```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_replay.toml
```

Рекомендуется задавать отдельный output dir:
```bash
REPLAY_OUTPUT_DIR=./strategy-runtime/replay_out_ci \
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_replay.toml
```

Проверьте:
- `replay.enabled = true`,
- `replay.bars_csv_path` указывает на корректный CSV,
- при необходимости `replay.reference_trades_csv_path` для сравнения.

---

## 3) Запуск replay через `session_gap_replay`

```bash
set -a; source strategy-runtime/rt_session_gap_standalone.env; set +a
cargo run -p strategy-runtime --bin session_gap_replay
```

Используйте этот путь для быстрых проверок SessionGapStandalone без полного runtime-конфига.

---

## 4) Как сравнивать результаты

Ищите артефакты:
- `trades.csv` (runtime сделки),
- `summary.json`,
- `parity_report.json` (если включён report в replay-процессе).

Сверяйте в `trades.csv` минимум:
- timestamp/время сделки,
- side,
- qty,
- price,
- reason (если присутствует в формате отчёта).

### Tolerance и ожидания
- `price_tolerance` задаёт допустимую погрешность цены.
- `strict_dedup=true` усиливает фильтрацию дублей.
- Полный матч live и backtest не всегда ожидаем (разные источники/latency/ack path).

---

## 5) Быстрый минимальный отчёт

1. Прогоните replay:
```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_replay.toml
```
2. Проверьте, что `trades.csv` не пуст.
3. Проверьте `summary.json` на PnL/count.
4. Если есть `parity_report.json`, проверьте mismatch count и max price diff.

---

## 6) Troubleshooting

### 6.1 Пустой `trades.csv`
- Неверный `strategy_kind`.
- Неподходящий период данных (нет условий входа).
- Ошибочная таймзона/время сессии в конфиге.

### 6.2 Сильное расхождение с reference
- Проверьте те же входные CSV.
- Проверьте `price_tolerance`.
- Проверьте, что один и тот же build/commit для сравниваемых прогонов.

### 6.3 Нестабильный replay между прогонами
- Проверьте, что не смешиваются output директории.
- Убедитесь, что replay читает одинаковый вход и одинаковый конфиг.
