# Replay / Backtest Guide

**Правило №1:** все актуальные TOML-конфиги лежат в `./configs/`. Другие `.toml` в репозитории считаются legacy.

## Config file path
- `./configs/runtime.replay.toml` — базовый replay-конфиг для `strategy_runtime_runner`.
- `./configs/session_gap.replay.toml` — отдельный replay-сценарий SessionGapStandalone.
- `./configs/strategy_runtime_runner.replay.toml` — алиас-конфиг для runner replay сценариев.

## Run command
```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.replay.toml
```

```bash
REPLAY_BARS_CSV_PATH=./data_samples/paper_bars_2.csv \
REPLAY_OUTPUT_DIR=./replay_out \
cargo run -p strategy-runtime --bin session_gap_replay
```

> Важно: команды выше предполагают запуск из корня `alor-rs-main`.
> Если запускаете из другого `cwd`, используйте абсолютные пути к `data_samples/*`.

### Пример для набора `paper_bars_3`

Если нужно сверять именно индикаторы по новому набору:

```bash
REPLAY_BARS_CSV_PATH=./data_samples/paper_bars_3.csv \
REPLAY_OUTPUT_DIR=./replay_out_3 \
cargo run -p strategy-runtime --bin session_gap_replay
```

Для этого набора `paper_indicators_3.csv` используется как reference для тестов стратегии,
а не как вход `session_gap_replay` (replay-бинарь читает бары и опционально сравнивает сделки с reference trades CSV, если он задан/существует).

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
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.replay.toml
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
