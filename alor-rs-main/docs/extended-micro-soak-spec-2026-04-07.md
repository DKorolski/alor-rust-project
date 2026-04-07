# Сокращённое ТЗ перед extended micro soak (2026-04-07)

Документ фиксирует цели, блоки работ, критерии приёмки и план soak. Реализация блока A (логирование AlorUsdrubfHybrid) ведётся в коде стратегии и согласованных местах runtime; блок B опирается на уже задокументированный hybrid baseline; блок C — только по результатам soak.

Связанные материалы:

- Action-scoped stop cleanup (hybrid): [`hybrid-cws-stop-cleanup-observation-2026-04-07.md`](./hybrid-cws-stop-cleanup-observation-2026-04-07.md)
- Ранее: [`hybrid-action-scoped-live-soak-runbook-2026-04-02.md`](./hybrid-action-scoped-live-soak-runbook-2026-04-02.md)

---

## 0. Цель

Не разворачивать новый крупный hardening-проект, а:

1. Нормализовать логирование **AlorUsdrubfHybrid**.
2. Зафиксировать уже внедрённый **action-scoped stop cleanup** для hybrid как **validated baseline**.
3. Пройти **extended micro soak** на **5–10 сессий** на **frozen release**.
4. По итогам soak решить, нужен ли узкий follow-up по **retry state machine** (блок C).

---

## 1. Блок A — логирование AlorUsdrubfHybrid

### Цель

Привести новую стратегию по уровню и стилю логирования ближе к `session_gap_standalone` и `hybrid_intraday_runtime`: меньше шума, проще post-trade forensic.

### Что сделать

#### A1. Meaningful status transitions по broker position

Логировать на **INFO** только:

- flat → open  
- open → flat  
- смена направления  
- существенное изменение qty / avg_price  

Повторные идентичные подтверждения: **DEBUG** или подавление по fingerprint.

#### A2. Единообразный набор INFO-событий (поля / имена)

Ориентир по событиям (часть обеспечивается runtime audit, часть — стратегией):

- `signal_generated`
- `intent_emitted`
- `command_acknowledged` / отклонения (runtime + стратегия для reject-policy контекста)
- `execution_confirmed` (runtime)
- `position_transition` (стратегия)
- `bootstrap_processed`
- `runtime_state_restored`
- `replay_guard_armed` / `replay_guard_cleared`
- `risk_state_changed` (при смене defer / inflight риска)

#### A3. Market fills: не путать цену ордера и цену исполнения

В **INFO** для исполнения:

- `exec_price`, `qty`, `commission`, снимок позиции из runtime state (может отставать от брокера до position stream)  
- поле цены из записи ордера именовать как **reference** (не как «цена исполнения»)

#### A4. Change events, а не повтор состояния

Для `live_ready`, lifecycle stage, reconcile, reject policy, entry/exit inflight — логировать **на переходе**, а не на каждом тике.

### Критерий приёмки (A)

- Хвосты логов новой стратегии читаются без спама.  
- За один день можно быстро восстановить жизненный цикл сделки.  
- Стиль сопоставим с двумя зрелыми стратегиями.

---

## 2. Блок B — action-scoped stop cleanup как baseline (hybrid)

### Цель

Формально закрепить validated operational baseline для **Hybrid intraday** (`trading-hybrid`, **IMOEXF**), не переделывая уже внедрённое.

### B1. Зафиксировать в docs / soak-plan

Baseline включает:

- `DeleteStopLimit` через **action-scoped CWS**  
- short-lived control session для stop cleanup  
- fresh token / authorize по политике  
- runtime observability:  
  - `cleanup_ack_error_with_active_stop_while_flat`  
  - `stop_order_active_while_flat`  

Детали и пост-роллаут validation: [`hybrid-cws-stop-cleanup-observation-2026-04-07.md`](./hybrid-cws-stop-cleanup-observation-2026-04-07.md).

### B2. Evidence во время soak (каждый день)

По hybrid фиксировать:

- был ли stop cleanup  
- был ли transport reset  
- был ли retry / second attempt  
- остался ли stop after flat  
- потребовалось ли manual cleanup  

### Критерий приёмки (B)

Action-scoped stop cleanup считается **внедрённым и validated**; soak **наблюдает** этот путь.

---

## 3. Блок C — optional residual follow-up (только если soak покажет)

### Цель

Не добавлять лишнюю механику заранее.

### Делать только при повторяющихся проблемах stop cleanup + transport

- bounded retry state machine  
- счётчики: `stop_cleanup_transport_error_total`, `stop_cleanup_success_after_retry_total`, `stop_cleanup_retry_total`  
- финальный audit event `cleanup_outcome`  

### Пока не делать

- безусловный recycle на каждый stop intent  
- агрессивный token refresh перед каждым control action  
- крупный refactor control-plane  

---

## 4. План extended micro soak (5–10 сессий)

### 4.0 Freeze

На период soak: **code / configs / topology** заморожены; без opportunistic правок mid-soak.

### 4.1 Supported режим

| Стратегия | Режим |
|-----------|--------|
| `session_gap_standalone` | current micro size, без изменения operational semantics |
| `hybrid_intraday_runtime` | current micro size, action-scoped `DeleteStopLimit`, отдельное наблюдение stop cleanup path |
| `AlorUsdrubfHybrid` | micro only, **clean-start profile**, без non-flat restart, без startup с working/stop orders |

### 4.2 Сбор каждый день

**Все три:** first live bar, first entry, first exit, final flat, rejects, retries, health/readiness, manual actions yes/no.

**AlorUsdrubfHybrid:** replay guard armed/cleared, все reject-policy решения, broker-truth transitions, duplicate/stale startup, EOD flat confirmation.

**Hybrid:** stop cleanup attempted / transport reset / active stop after flat / auto recovery / manual stop intervention.

### 4.3 Daily reconcile

После сессии:

- **Брокер:** positions flat, orders 0, stop orders 0  
- **Runtime:** нет stale pending ids, нет нерешённого degraded risk, нет скрытого non-flat  
- **Док:** короткий дневной note (summary, anomalies, operator actions, verdict)

### 4.4 Success criteria (5–10 сессий)

**Общие:** 0 overnight non-flat, 0 unresolved working/stop orders, 0 manual flatten, 0 false startup-tail, 0 infinite retry loops.

**Hybrid:** stop cleanup детерминирован, нет working stop после flat, transport flakes закрываются штатно.

**AlorUsdrubfHybrid:** нет blind entries, нет broker/runtime divergence, нет повторных duplicate emissions от stale state, exit path доходит до broker flat.

### 4.5 Abort / hold

Не повышать confidence / не двигаться к small, если хотя бы одно из:

- manual intervention  
- overnight non-flat  
- active stop после flat  
- runtime flat при несовпадении с брокером  
- повторные stop cleanup failures  
- нарушение clean-start у новой стратегии  

### 4.6 Решение после soak

- **5–10 чистых сессий:** session_gap и hybrid — обсуждение перехода к small; AlorUsdrubfHybrid — отдельный verdict (continue micro или условный small позже).  
- **Проблемы только hybrid stop cleanup:** не откатывать весь релиз автоматически; оценить узкий retry follow-up.  
- **Проблемы только AlorUsdrubfHybrid:** две старые оставить running; новую держать на micro / harden отдельно.
