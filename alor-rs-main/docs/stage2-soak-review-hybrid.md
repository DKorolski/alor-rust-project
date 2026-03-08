# Stage-2 Hybrid Soak Review

## 1) Что сделано по Stage-2

1. Выполнена интеграция `hybrid_intraday` в `strategy-runtime` без изменения core-логики стратегии.
2. Добавлены `intent_class` и intent-aware gating:
   - `Entry` фильтруется по окнам/guard,
   - `Exit/CancelCleanup/ProtectiveRepair` не режутся целиком при blocked-состоянии.
3. Реализован execution/recovery контур:
   - pending request tracking (`entry/exit/tp/sl`),
   - MR bracket (TP/SL),
   - OCO cleanup,
   - repair/backoff/deadline,
   - safe-mode.
4. Добавлены tagging + ownership:
   - формат `HYB|sid=<sid>|c=<cycle>|o=<MR|BO>|r=<ENTRY|TP|SL|EXIT|CANCEL>`,
   - persist/recover `cycle_id`,
   - filter и adopt только "наших" ордеров/stop-ордеров.
5. Подключен `stopLimit` plumbing и `StopOrders` WS/snapshot delivery в runtime.

## 2) Результаты тестов и smoke

1. Локальные unit/integration прогоны по этапам проходили (`PASS`) на рабочих ветках этапов.
2. Live smoke `stop_limit_smoke`:
   - create: `orderNumber=114979079` (`httpCode=200`),
   - delete: `orderNumber=114979079` (`httpCode=200`),
   - итог: `PASS`.
3. Live smoke `stop_orders_ws_smoke`:
   - subscribe ack: `PASS`,
   - после create получен WS статус `working`,
   - после delete получен WS статус `canceled`,
   - итог: `PASS`.
4. Replay parity (`hybrid_replay --check --strict`) ранее подтверждалась как `PASS` на `golden/train/test` (без регрессий в рамках этапов интеграции).

## 3) Наблюдения в soak (live/paper)

1. На старте live runtime ожидаемо уходит в `BLOCKED` до завершения bootstrap:
   - `bootstrap:missing_live_bar`,
   - `bootstrap:not_ready`,
   - `gateway_ready=false`,
   - `phase=SyncingHistory`.
2. В paper был операционный кейс с путями отчётов:
   - `failed to flush trade ledger reports: No such file or directory`,
   - требуется валидировать/создавать каталоги outputs до запуска.
3. Вне торговой фазы (выходные/closed) система не должна исполнять входы до разрешения guard/readiness.

## 4) Кейс pending entry (для анализа)

Из runtime snapshot наблюдалось:

- `pending_entry_owner = mean_reversion`
- `pending_entry_side = short`
- `pending_entry_cycle_id = 69a7ca6000`
- `pending_entry_request_id = 5591204f-2197-5ec8-8a0c-42a288c686f1`
- `entry_ready = false`

Интерпретация:

1. Зафиксирован хвост entry-attempt в persisted runtime state.
2. По ручной проверке stream-ов явной корреляции ack/order по этому UUID на момент проверки не найдено.
3. При запуске с `reset_state_on_start = true` состояние сбрасывается и хвост не наследуется.

## 5) Рекомендации на доработку (observability + troubleshooting)

1. Добавить structured-логи в hybrid strategy:
   - `on_bootstrap_snapshot` (owner/side/cycle/tag adopt),
   - `on_runtime_state_restored`,
   - причины блокировки входа (`warmup/silence/guard/phase`).
2. Добавить корреляционный трейс:
   - `intent -> command.request_id -> ack -> order/position`.
3. Добавить диагностический блок по pending:
   - `pending_created_ts`,
   - `last_seen_ack_status`,
   - `timeout/cooldown decision`,
   - причина очистки/удержания pending.
4. Расширить persisted state hybrid индикаторными полями прогрева (day-aggregates), чтобы после рестарта не терять контекст `FeatureBuilder`.
5. Обновить runbook разделом:
   - как разбирать `pending_entry_*`,
   - какие Redis streams смотреть,
   - когда применять `reset_state_on_start=true`.

## 6) Статус

1. Stage-2F принят как сильный инкремент.
2. Для финального `Stage-2 accepted` рекомендуется закрыть согласованные P0 hotfix/observability пункты и повторить soak с артефактами.

## 7) Canary Window (expanded)

1. Для детерминированного canary используем warmup+history начиная с `2026-03-05 09:00 MSK`:
   - это зафиксировано в `configs/gateway.paper.canary.7502MIW.toml` через `from_ts = 1772690400`.
2. Целевой canary-фрагмент для оценки сделок:
   - `2026-03-06 14:10-14:55 MSK`.
3. Запуск:
   - gateway: `cargo run -p alor-gateway --bin alor_gateway_transport_runner -- --config ./configs/gateway.paper.canary.7502MIW.toml`
   - runtime: `cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config ./configs/runtime.hybrid.paper.canary.7502MIW.toml`
4. Acceptance-check:
   - `reports/paper_hybrid_canary_7502MIW.jsonl` создан,
   - `reports/trades_hybrid_canary_7502MIW.csv` не пустой,
   - `reports/summary_hybrid_canary_7502MIW.json` содержит `trades_total > 0`,
   - в `cmd.orders.7502MIW` нет публикаций (для `trade_mode=paper`, `allow_live_orders=false`).
