# ТЗ: Календарь, PIT OI И Замороженные Shadow-Профили MOEX

Дата: 2026-11-15.
Статус: `READY_FOR_ENGINEERING_REVIEW / NO_DEPLOYMENT_AUTHORIZATION`.
Плановый горизонт: ноябрь-декабрь 2026; фактический старт зависит от приёмки.
Текущий prospective цикл: 20 полных обычных сессий, затем отдельный review.
Заказчик отдельно утверждает запуск на VPS и любые изменения production.

## 1. Цель И Границы

Первая очередь разработки: календарно-корректные контексты, prospective-сбор OI
и три проверяемых Rust shadow-профиля IMOEXF. USDRUBF H4/W4 сохранить без изменений;
его возможный A/B требует отдельного matched reference и решения. Не искать новые K.
Отделить исправление доступности/допуска данных от изменения alpha-контекста.
Получить воспроизводимый parity и будущую выборку, не заменяя текущий live.

Обязательный scope:

- Версионированный календарь и раздельные контексты MR/BO.
- PIT-коллектор OI IMOEXF, ежедневная фиксация решения и аудит доступности.
- Три изолированных виртуальных arms: IMO_AA, IMO_AB, IMO_AB_OI.
- Сохранение существующих USDRUBF H4/W4 A/A и B/B без нового rollout/reset.
- Native Rust replay, golden fixtures, рестарты, admission и будущий shadow.
- Дневная отчётность, checkpoint и описательный отчёт на 20 сессиях,
  portfolio incremental analysis без автоматического продления наблюдения.

Вне scope: ретюн, новые часы/квоты/стопы, изменение лотов, broker brackets,
новые weekend-входы, overnight, RI OI gate и автоматическое live promotion.
Сбор USD/RI OI допустим позднее как read-only наблюдение, не новый фильтр.
Ни один shadow arm не должен иметь маршрут выставления брокерских заявок.

## 2. Источники И Текущее Состояние

Все пути ниже относительно корня workspace `strategies_list`, не cwd документа.

| Назначение | Источник |
| --- | --- |
| Календарь, допуск сессий, H/L/C | `analiz_alpha_si/moex_calendar_context_2026_11_15/` |
| Раздельные контексты, Python reference | `analiz_alpha_si/moex_component_context_2026_11_15/` |
| OI gate, исходные captures и инструкция | `analiz_alpha_si/moex_oi_gate_overnight_2026_11_15/` |
| Целевой Rust проект | `alor_project/bybit_barter_test/alor-rs-main/` |

Основное историческое окно parity: 2026-07-20..2026-11-13, 84 обычные сессии.
Не объявлять это новым OOS. Старые версии и исходные данные не перезаписывать.
Подробные определения: `protocol.json`, `PROTOCOL.md`, `README_RU.md`,
`frozen_inputs.json` и `artifact_manifest.csv` соответствующих пакетов.
Их файлы/хеши должны входить в инженерный manifest, даже если пока не tracked Git.

Точки интеграции, а не разрешение изменять их без анализа:

- `strategy-runtime/src/shadow_session_context.rs` и его collector.
- `strategy-runtime/src/shadow_reaction.rs`.
- `strategy-runtime/src/bin/imoexf_shadow_replay/{model,tests,trace}.rs`.
- `strategy-runtime/src/bin/usdrubf_shadow_replay/{model,tests,trace}.rs`.
- `strategy-runtime/src/bin/reaction_shadow_worker.rs`.
- `docs/shadow-context-aware-admission-work-plan-2026-11-15.md`.

На момент осмотра уже существует scoped P0 admission-controller с планом первой
сессии 16 ноября. Это не приёмка нового A/B/OI-кода и не постоянная P1-процедура.
Не обходить существующий strict preflight, review/grant, source identity,
неизменяемый source-prefix, карантин и deadline. Согласовать изменения с автором
admission-плана. Новые model/config/calendar hashes требуют собственного допуска.

**Не копировать существующие шаблоны без сверки и не заменять текущий H4/W4.**
В последних инженерных документах operational USDRUBF shadow выбран как
Previous-day07 H4/W4: weekday_only и weekend_context_no_trades. Это намеренный
профиль, не ошибка шаблона. Его наблюдение и накопленную выборку сохранить.
H3/W4 ниже - другой frozen reference последнего исследования, вне первой очереди.
Он не является разрешением новых operational arms или миграции H4 -> H3.
У H4 last MR signal10:40, forced action11:00; у H3 last signal09:40,
forced action10:00. BO W4 в обоих случаях: first signal11:00/action11:10.

Документально выбранные ветки существующего H4/W4 сохранить без переименования:

| Ветка H4/W4 | MR context | BO context | Новые входы в ДСВД/выходные |
| --- | --- | --- | --- |
| `weekday_only` | Обычные сессии, A | Обычные сессии, A | Запрещены |
| `weekend_context_no_trades` | Обычные + допустимые ДСВД, B | Обычные + допустимые ДСВД, B | Запрещены |

Это A/A и B/B, не mixed A/B. Сохранить mr_horizon=4, bo_wait=4, состояния,
журналы и согласованный 20-сессионный отсчёт существующего operational запуска;
не сбрасывать накопленные наблюдения из-за отдельного H3-исследования.
H3/W4 ниже остаётся отдельным исследовательским reference, а не выбранной
заменой H4/W4; этот документ не разрешает его добавление в operational shadow
или переключение существующих веток.

`configs/examples/reaction-shadow.usdrubf-h4-w4.toml` нельзя просто переименовать
в H3; нужны иной config_hash и отдельная квалификация. IMOEXF example использует
другую cost-формулу; её нельзя выдавать за исследованный cost x1.
Старый single-context API сам по себе не доказывает поддержку A/B.
Существующие shadow/live конфиги не перезаписывать. Применение календарного
контракта к H4 требует его собственного reference/parity; контрольные H3 totals
из этого ТЗ к нему не относятся. Выбор H3 вместо H4 требует отдельного matched
сравнения при одинаковых calendar/context/costs и решения заказчика.

## 3. Эталонные Модели И Arms

| model_id для новой версии | Эталон | MR context | BO context | OI |
| --- | --- | --- | --- | --- |
| imoexf_author41_bo053_AA_v1 | IMO_AA | A | A | off |
| imoexf_author41_bo053_AB_v1 | IMO_AB | A | B | off |
| imoexf_author41_bo053_AB_oi_negative_lag1_v1 | IMO_AB + negative_oi_lag1 | A | B | gate |

Архивные USD_AA/USD_AB относятся к previous-day07 H3/W4 и сохраняются только
как исследовательский reference. Новые USD arms в первую очередь не входят.
Нельзя переносить их статистику на H4/W4 или переименовывать текущую B/B в A/B.

У каждого arm отдельные state, journal, checkpoints и виртуальный портфель.
Один общий слот MR-first **внутри** arm; независимые arms не делят позиции,
квоты или доступность средств. Quantity=1, без реинвестирования.

A = предыдущие завершённые REGULAR-сессии.
B = предыдущие завершённые REGULAR + допущенные DSVD.
A не равно Mon-Fri: рабочая суббота может быть REGULAR-контекстом.
Исходный запрет субботних/воскресных новых входов при этом не снимается.
Известная DSVD в будний праздник не становится REGULAR по номеру дня недели.

### 3.1 IMOEXF

Author41 MR short: k=.16, k2=.020, stop_k=.58; существующий BO k=.53.
Это не High180 riskgate и не Author42 BO.

Контрольные свойства текущего frozen reference:

- MR использует свой prev_close/prev_range/prev_low; максимум 2 входа в день.
- Допуск range: .005 < prev_range/prev_low < .075.
- Short signal: prev_close < closed_bar.close < prev_close+.16*prev_range.
- Signal cutoff MR: 10:00 включительно при открытии 07:00.
- MR stop: high >= prev_close+.58*prev_range; take: close < prev_close-.020*prev_range.
- Time-exit signal MR: 20:00; breakeven после bars_held >20 по исходному правилу.
- BO: первый signal не ранее start+3h; абсолютный min_range=1.01.
- BO использует собственные prev_close, prev_range и два prev_close для return gate.
- BO big-move gate .025, stop1 .51, stop2 .35; исходные side quotas сохраняются.
- EOD BO signal label 23:30; в next-open reference выход на 23:40.
- Все сравнения, порядок stop/TP/time, округления и re-entry брать из compiler,
  а не восстанавливать приблизительно по этой сводке.

При старте 07:00 MR и BO потенциально пересекаются по разрешённым сигналам
с 10:00, а открытый MR может сохраняться до 20:00. Это **свойство baseline**:
не вводить forced MR flatten в 10:00, независимый BO-слот, reversal или новый
same-bar handoff под видом календарной задачи.

### 3.2 USDRUBF: Отложенный Reference

Ниже описан архивный previous-day07 H3/W4 reference, не scope первой очереди,
не intraday-VWAP live и не замена существующего H4/W4 shadow.
MR short: k=.035, take_k=.16, stop_k=.43.
context_hour=7, clock_hour=7, mr_horizon=3, bo_wait=4, mr_start_delay=0.

Контрольные свойства:

- MR signal labels 07:00..09:40; force-exit signal 09:50 -> действие 10:00.
- Range gate: .006 < MR prev_range/current close < .050.
- MR signal: 0 < close-MR prev_close < .035*MR prev_range.
- MR TP/SL от фактического next-open entry с исходным round_tick(.01).
- BO signals не ранее 11:00; k=.45, stop1=.51, stop2=.35, return gate=.020.
- Для BO B весь его scale/anchor/return, включая stop levels, берётся из B.
- Первичный MR predicate имеет приоритет над BO; position/pending блокируют новый вход.
- EOD BO signal 23:30 -> next-open exit 23:40; исходные terminal guards неизменны.

Не переносить в этот профиль live MR bracket execution и не заменять previous-day
anchor текущим VWAP/open/range. Детальную нормировку и приоритеты задаёт
`component_models.py`, компилирующий замороженный `reference_model.py`.

## 4. Календарный Контракт

Ввести явный классификатор минимум REGULAR / DSVD / CLOSED / UNKNOWN.
Сервисность бара - отдельный признак, не новая торговая сессия.
Хранить `calendar_version`, инструмент, calendar session date, exchange trade
date, UTC boundaries и их MSK представление. Не группировать выходной бар
автоматически в понедельник только из-за exchange trade date.

Разделить `context_update_allowed`, `new_entry_allowed` и
`position_management_allowed`. Классификация REGULAR сама по себе не отменяет
исходные календарные/weekday/seasonal gates модели.

Правила:

1. REGULAR: обновляет A/B после принятого завершения и проверки данных.
2. DSVD: обновляет только B после принятого завершения; новых входов нет.
3. CLOSED/известное исключение: не синтезировать бар, не стирать валидный warmup.
4. UNKNOWN ожидаемой сессии: зависимый контекст не готов; нельзя молча взять
   более старый anchor и считать это обычным пропуском выходного.
5. Для enabled MR и BO должны быть готовы оба выбранных контекста.
   Неизвестный BO-context блокирует admission combo, даже если MR-context готов.
6. При HOLD продолжаются безопасное наблюдение и управление уже существующей
   позицией; admission failure не должен отключать exits/cleanup.
7. Readiness требует две выбранные завершённые предыдущие сессии, а не новый
   median/lookback10. MR/BO snapshots и readiness должны быть раздельными.

Повторить исследованный grid/span/coverage contract, не устанавливать новый
порог полноты по результатам PnL. В частности, DSVD guard исходного исследования:
44/55 bins обычной и 68/84 продлённой праздничной сессии плюс first/last span.
Это исследовательский допуск, не сертификат полной ленты.
В frozen DSVD-context 09:50 M10 допускается: не объявлять его служебным при
переносе. Для торговых решений и исполнения аукционный бар не использовать.
Сервисные бары обычной сессии фильтровать по утверждённому session contract,
не по универсальному правилу вырезания всех :59 или всех pre-09:00.

Календарь должен быть инструментальным и датированным: USDRUBF получает
DSVD-context только после своего допуска. Для parity учесть переходы 14 июля,
18 июля, основную сессию с 14 сентября, 4/5 ноября, известные закрытые выходные.
Рабочие субботы, праздники и shortened/break sessions покрыть fixture.
Не менять календарный контракт старых live-профилей неявно через shared helper.

В bootstrap dependency trace включать previous/previous2 каждого компонента,
H/L/C/range/return, source dates, coverage, skipped candidates и selection frontier.
Старое исправление данных вне выбранных двух дней может менять их отбор:
одного сравнения итоговых OHLC недостаточно для admission.

## 5. PIT-Коллектор OI

Первый обязательный инструмент IMOEXF. Переиспользовать как исследовательский
пример `capture_oi.py` и `OI_CAPTURE_RUNBOOK.md`, но доработать operational
валидацию: это сейчас one-shot, не готовый daemon.

Расписание: запуск 06:45 Europe/Moscow в ожидаемый ordinary decision day;
bounded retries/backoff только до 06:55; commit принятого snapshot до deadline.
В 07:00 нельзя обращаться к более позднему snapshot как к утреннему решению.
Перед запуском проверить часы хоста; timestamps сохранять UTC и MSK.
Сервис read-only, без торговых ключей и доступа к endpoint создания заявок.

Источник: dated ISS history OPENPOSITION двух точных предыдущих REGULAR-дат.
`securities.PREVOPENPOSITION` - дополнительная сверка.
Текущий `marketdata.OPENPOSITION` - диагностический changing value, не подмена
вчерашнего EOD. DSVD price-context B не меняет REGULAR source policy для OI.

Минимальная валидация: HTTP/schema, ровно одна запись на каждую выбранную дату,
конечные неотрицательные значения и положительный знаменатель, ожидаемый SECID,
точные source dates, receipt/durable timestamps <=deadline.
Согласовать размерность OPENPOSITION с эталоном, не делить OI на два только
ради иной трактовки количества контрактов: sign rule должен воспроизводиться.

На каждый запрос сохранять raw bytes, URL/params, HTTP status/Date, received_at,
durable_at, SHA256 и schema version. Не писать секреты в URL или журнал.
Никакого overwrite принятой версии. Новые revisions получают новый vintage;
торговое решение ссылается на конкретный неизменяемый snapshot.

Успешный сбор не означает разрешение торговли. При missing/late/schema error,
дубликате, неизвестной source date или invalid clock создать явный status и alert.
Не заменять предыдущую запись более старой и не переключаться на lag2.

Хранилище: append-only raw captures + атомарно сохраняемый validated manifest;
deduplicate только идентичные payload по hash с сохранением receipt evidence.
Рестарт должен восстановить уже frozen решение без повторного выбора vintage.
Установить бюджеты HTTP timeout/retries, disk/retention, метрики размера и
свободного места; не копировать многогигабайтную OHLCV-историю в каждый arm.

После 20 сессий можно заявить coverage полученных будущих snapshots.
Нельзя заявлять 100% historical PIT certification или восстановление
первоначальной публикации старых 84 дней из сегодняшнего ISS history.

## 6. Daily OI Gate

Для ordinary session D выбрать две предыдущие REGULAR-даты r1/r2 и вычислить:

```text
delta_oi = OPENPOSITION(r1) / OPENPOSITION(r2) - 1
allow_new_BO = timely_valid_snapshot && delta_oi < 0
```

Зафиксировать gate на всю сессию до открытия, включая source dates/hash/reason.
Нулевое изменение запрещает новые BO. Неполный/поздний snapshot запрещает
только новые BO в IMO_AB_OI, но не MR, BO observation или exits.
Не включать BO позже в течение дня после запоздалой публикации.

**Это не то же самое, что UNKNOWN price-context.** Неготовый обязательный
ценовой контекст блокирует combo admission; OI unavailable - предусмотренный
дневной BO gate, а не повод отбрасывать день из оценки результата.

Gate проверять непосредственно перед новым BO entry, не до обновления
bar_index/opening levels/day H/L, не вокруг всего BO update и не как остановку
engine. MR-журнал при OI off/on должен быть идентичен эталонному.
Сохранять именно shared-slot replay, а не постфактум удалённые BO trades.

В historical replay использовать final-history proxy с явной маркировкой,
не выдавать эти данные за prospective captures. Future-erasure не доказывает
своевременность исторической публикации.

## 7. Execution, Costs И State

Базовый timeframe 10m, bar label = начало интервала в MSK.
Сигнал рассчитывается по закрытому бару; action = open следующего
**последовательного** 10m бара. Например, label 07:00 означает интервал
07:00..07:10, следовательно action 07:10, а не дополнительное ожидание до 07:20.

Reference TP/SL touch на high/low закрытого бара - это predicate, не обещание
fill по самому уровню. Для этой работы сохранить next-open reaction.
USD ambiguous TP+SL bar имеет исходный stop-first и отдельный флаг.
Не вводить limit fills, market-on-close, bracket или иной порядок приоритетов.

Раздельно логировать bar start, bar close, receipt, signal label, scheduled action,
actual processing и paper fill. Исследовательский next-open fill не доказывает
доступность той же цены при позднем operational receipt. Запоздалую реакцию
помечать; не подставлять благоприятную прошлую цену как реальное исполнение.
Operational freshness/readiness gates не смягчать, чтобы добиться совпадения PnL.

Не материализовать stale entry после его scheduled exit, terminal cutoff или
через неподтверждённый gap. Сохранить MR-first/side quotas/re-entry ordering;
не вводить новую политику same-bar handoff без отдельной модели и проверки.
BO observer обновляется и при занятом MR слоте.

Все завершённые обычные сессии должны заканчивать paper-flat, без pending;
no-overnight. Source terminal publication, model session_complete и broker-flat
различаются. Healthy process или preflight PASS не доказывают clean EOD.

Costs - отдельный версионированный accounting layer:

```text
friction_per_leg = 1 RUB + raw_fill_price * point_value_RUB * 0.000066
                  + 1 * tick_size * point_value_RUB
net_x1 = gross - entry_friction - exit_friction
net_x2 = gross - 2 * (entry_friction + exit_friction)
```

IMOEXF point_value=10, tick=.5; USDRUBF point_value=1000, tick=.01.
Не списывать adverse tick одновременно в fill и ещё раз как стоимость.
Хранить raw reference fill и modeled executable/friction отдельно.
Прямой funding у принятого no-overnight book нулевой; overnight cash не добавлять.

Все настройки содержатся в config/frozen model manifest: model_id, config_hash,
calendar_hash, cost_hash, executable/source hashes и reference_input_hashes.
Default новых функций off. Checkpoint привязан к этим identities; mismatch
не разрешает продолжить старое состояние как новую модель.

## 8. Parity И Regression Matrix

Сравнивать не только net, но и порядок событий, даты предыдущего контекста,
signal/action timestamps, component/side, entry/exit prices и exit reasons.
Совпадение net без event parity не принимается.

Идентичность дат/сторон/причин/порядка обязательна. Execution prices сравнивать
на исходной tick-grid; разрешённая арифметическая погрешность уровней <=1e-9,
денежных агрегатов <=1e-6 RUB. Tolerance не меняет торговые predicates.
Расхождения у threshold отдельно разобрать, не расширять epsilon до успеха.

Обязательные controls:

| Проверка | Ожидаемый результат |
| --- | --- |
| IMO_AA | Исходные frozen parent events и trades |
| IMO_AB | Точное совпадение со mixed-context Python reference |
| Диагональный B/B | Контроль правильного выбора двух контекстов, не новый prospective arm |
| IMO_AB_OI gate=off | Полное совпадение с IMO_AB |
| IMO_AB_OI gate=lag1 | Эталонные OI replay events/trades; MR неизменен |
| Отключённый feature flag | Старые профили/конфиги и state semantics неизменны |

Контрольные totals x1 на 84 сессиях, один контракт, только smoke-check:

| Arm | Trades | Net RUB |
| --- | ---: | ---: |
| IMO_AA | 125 | 913.61273 |
| IMO_AB | 133 | 2533.50425 |
| IMO_AB_OI | 96 | 3609.02511 |
| USD_AA | 106 | 1874.05766 |
| USD_AB | 111 | 3440.73428 |

USD totals приведены только для неизменности архивного H3/W4 reference.
Они не являются acceptance targets текущего H4/W4 или первой очереди IMOEXF.

Полный reference: component `results/trade_log.csv`, `results/events.csv`,
calendar `results/*_prior_context.csv`, OI `results/oi_trades.csv`,
`results/oi_events.csv`, `results/oi_gate_days.csv`.
Списки исходных файлов проверять по manifests, не переизобретать котировки.

Fixtures должны покрыть:

- 4 ноября DSVD в среду -> 5 ноября REGULAR; понедельник после DSVD.
- Closed weekend, рабочую субботу REGULAR без новых weekend orders, July transition.
- UNKNOWN/missing/incomplete selected prior session; readiness и сохранение warmup.
- DSVD09:50 как контекст, сервисный regular bar и отсутствие фиктивного auction fill.
- Отдельные MR/BO previous2 close и return gate, включая смену anchor selection.
- MR pending/position в первых барах, BO observer без сдвига внутреннего индекса.
- MR занимает слот при BO trigger, штатный выход и исходная последовательность re-entry.
- Stop+TP на одном баре, EOD signal/action distinction, последний бар, stale entry.
- OI <0, =0, >0, missing, duplicate, zero denominator, late receipt, changed revision.
- OI deadline граница, immutable day decision, no delayed enabling и no lag2 fallback.
- Независимость MR и exit lifecycle от закрытого OI gate.
- Restart до/после freeze, pending/position restore, no duplicate event/model call.
- WS reconnect того же process, resubscribe/history replay, immutable source cursor.
- Relevant/irrelevant revisions, новая PID/boot, deadline exhaustion и scoped cleanup.
- Future-erasure OHLCV/context/OI и иной future revision без изменения прошлых решений.

Не стирать исходный as-observed journal и не делать historical back-insertion.
Для P1 recovery требуется native end-to-end fixture, а не только unit/mock тест.
Если perpetual unattended admission не принят, разрешён только отдельно
reviewed запуск с оператором; существующий dated P0 grant не распространяется
автоматически на будущие сессии или новые arms.

## 9. Deployment И Изоляция

После локальной приёмки подготовить, но не активировать без отдельного решения:
новые versioned configs, service units/containers, resource budgets и rollback.
Shadow должен быть технически неспособен отправить broker intent: отсутствие
broker credentials/producer route плюс тест попытки отправки. Одного имени
shadow в config или logging mode недостаточно.

Existing live RI/IMOEXF/USDRUBF, gateway и текущие shadow units не менять.
Новые prefixes/namespaces/checkpoints/journals должны исключать перезапись.
Общий read-only ingress допускается при независимых курсорах и подтверждённом
одинаковом input content/order для paired arms.

Оценочное окно этого задания ограничено 20 обычными сессиями. Не повышать
max_sessions сверх 20 и не продлевать lifetime существующих source manifests
ради отменённого 30-сессионного этапа. Перед допуском всё равно проверить, что
20-сессионный календарь вместе с warmup укладывается в bounds/row/disk budgets.
Продление или новый цикл требует отдельного решения, manifest и допуска.

До старта зафиксировать activation timestamp, release/image digest, actual config
hashes, reviewed source identity, bootstrap digest и календарь. Проверить context
coverage **после** последних source appends, а не использовать старый row count.
HOLD при неготовности к cutoff; не стартовать с нуля или с опоздавшим warmup.

Unattended recovery допускается только после отдельной приёмки P1 процедуры.
Сохранять действующие queue/model-lateness/deadline guards.
Rollback shadow: остановить только принадлежащие этому запуску новые units,
сохранить source/raw captures/journals и результаты проваленного старта.
Calendar production rollout - отдельный PR/решение после replay/shadow;
исправление может менять допустимые торговые дни, несмотря на неизменность K.

## 10. Prospective Протокол И Выходные Данные

До запуска подписать frozen experiment manifest: arms, contrasts, start
conditions, оценочные окна, costs, coverage rules и decision labels.
Начало отсчёта только после accepted parity, warmup и документированного запуска.
Backfill и historical replay не увеличивают будущую выборку.

Два заранее объявленных primary contrasts первой очереди:

```text
IMO_AB - IMO_AA
IMO_AB_OI - IMO_AB
```

Архивный USD_AB - USD_AA не входит в этот prospective цикл. Новый H4/W4
contrast потребует собственного reference, scope и допуска, без изменения
уже наблюдаемых H4/W4 A/A и B/B.

Одинаковые даты и feed для контроля/кандидата. OI-эффект не сравнивать только
с A/A: это смешало бы эффект weekend context и gate.
Не добавлять BO-only/B/B/другие OI-пороги в prospective primary family после старта.
Если старты различаются, каждый contrast имеет собственный зафиксированный
старт; данные до готовности всех его arms не называются парным prospective OOS.

На 20 полных обычных сессиях - operational/risk checkpoint и описательный
парный отчёт. Обязательный этап на 30 сессиях отменён; дальнейшее наблюдение
не включается автоматически и требует отдельного решения после checkpoint20.
Сохранять все запланированные даты, HOLD/missing/late/parity faults; не отбирать
только прибыльные или удобные полные дни. Показывать own/common masks и coverage.
UNKNOWN market inputs не превращать в нулевой counterfactual PnL.
Предусмотренный OI missing -> BO disabled день остаётся результатом gate-policy,
а не удаляется как неудачный день. Не перезапускать отсчёт из-за плохого PnL.

Метрики: net x1/x2, MR/BO и long/short, trades/active days, expectancy, Sharpe,
win rate/payoff, top1/3/5 absolute и incremental concentration, MTM DD/DDD,
текущая незавершённая просадка, execution lateness и полная event parity.
DDD считать в обычных торговых сессиях; OHLC bound не называть точным intrabar DD.
Лимит max DDD=60 сохраняется; короткое окно 20 не доказывает соблюдение лимита
на будущей истории. При коротком sample inference descriptive/insufficient.

В текущем 20-сессионном цикле для paired blocks5/20 статус INSUFFICIENT_WINDOW:
p/CI/q не рассчитывать и не выдавать описательные итоги за statistical PASS.
Минимум max(30, 2*block_length) не снижается до 20. Для возможного отдельного
расширенного эксперимента первой очереди заранее зафиксировать seed, draws и BH family2 для
объявленных contrasts; block20 потребует минимум 40 сессий, не 20 или 30.
Локальная family2 не покрывает исходные USD-контрасты и предыдущий исследовательский поиск.
Не выбирать более выгодную длину блока, подокно или raw p вместо q.
Даже q<.05 не является автоматическим production GO.

Минимальные выдаваемые таблицы/журналы:

| Файл | Минимальное содержание |
| --- | --- |
| engineering_manifest.json | release, source/config/calendar/cost hashes, reference hashes, schema versions |
| calendar_sessions.csv | symbol, calendar_date, exchange_trade_date, type, boundaries, provenance, version |
| context_snapshots.jsonl | arm/date, MR/BO previous/previous2, HLC/range/return, readiness, coverage, skipped/frontier, source hashes |
| oi_capture_manifest.jsonl | decision date, source dates, receipt/durable/deadline, raw hashes, values, validity/revision status |
| daily_gate_decisions.csv | arm/date, gate enabled/allowed, delta OI, snapshot hash, source dates, reason, frozen_at |
| events.jsonl | arm/config, bar start/close/receipt, signal/action/process times, component/side, level, fill, suppress/exit reason, state owner |
| trade_log.csv | trade_id, arm, component/side, signal/action/exit times/prices, reason, gross, costs, net x1/x2, context/gate hashes |
| daily_pnl.csv | date/arm, eligibility/unknown/HOLD, MR/BO, realized and EOD MTM, net x1/x2, position/pending, completeness |
| parity_summary.csv | fixture/arm, reference/actual counts, exact matches, missing/extra, field deltas, pass/fail |
| shadow_session_health.csv | planned/admitted/complete dates, source cursors, restart/lag/faults, warmup, EOD and cleanup proof |
| candidate_comparison.csv | fixed paired contrasts, common/own masks, PnL/cost/risk/concentration, uncertainty status |
| portfolio_incremental.csv | frozen lots, Author/RI reference source, corr, DD/DDD, overlap and paired portfolio delta |

Все записи имеют schema_version, run_id и model/config identity либо
однозначную ссылку на immutable manifest. Таймзона дат/времени явно описана.
Для instrument-specific optional fields использовать null + applicability reason,
не подставлять 0. Числовой 0 reserved для реально известного нулевого результата.

Portfolio comparison делать с неизменными согласованными лотами RI и остальных
систем, на общих датах и одинаковом friction/accounting. Не оптимизировать веса
на этих же 20 сессиях. Live broker PnL и reference proxy сохранять отдельными
сериями; не выдавать смешанный portfolio replay за доходность брокера.
Если нужного RI-журнала/MTM или утверждённых весов нет, результат
PORTFOLIO_PENDING_INPUTS, а не придуманный control и не окончательная приёмка.

## 11. Порядок Работ И Приёмка

| Этап | Deliverables | Exit gate |
| --- | --- | --- |
| E0: audit | source/config/dependency matrix, baseline hashes, conflict list, approved scope | Три IMO arms; H4/W4 сохранён; нет подмены family/cost |
| E1: calendar | versioned calendar/router, trace, unit/native fixtures | Known exclusion сохраняет контекст; UNKNOWN fail-closed; старый feature-off неизменен |
| E2: PIT | collector, schemas, failure tests, scheduler candidate, recovery/runbook | Timely immutable snapshot либо явный отказ; нет подмен/торгового доступа |
| E3: model/parity | three IMO configs, full native replay, event diff, state/restart tests | IMO parity PASS; прежние H4/W4 и live paths неизменны |
| E4: admission | source/identity/recovery proof, isolation/resource tests, deployment/rollback notes | Согласованный operational допуск; отдельное разрешение VPS |
| E5: future shadow | immutable journals, coverage/health, checkpoint20 и описательный отчёт | Только новые сессии; нет подгонки и скрытых пропусков |
| E6: review | candidate/portfolio/risk report, limitations, unresolved findings | Отдельное решение аналитика/оператора; никаких автоматических live изменений |

E1 и E2 можно реализовывать параллельно; OI arm не запускается до E2+E3+E4.
A/B без OI не обязан ждать полного 20-сессионного OI checkpoint, но требует
своего parity и admission. Любая новая логика после просмотра будущих результатов получает
новый model_id/config_hash и отдельный цикл подтверждения.

Обязательные локальные проверки: `cargo fmt --all -- --check`, scoped unit и
integration tests для изменённых модулей, native replay fixtures, feature-off
backward compatibility; scoped clippy с `-D warnings`. Полный workspace test
при возможности; отдельно указать, что не запускалось или упало вне scope.
Не заявлять новую native parity на основании старого Python audit PASS.

Инженерные статусы:
`ENGINEERING_REVIEW` -> `LOCAL_PARITY_PASS` -> `APPROVED_FOR_SHADOW` ->
`SHADOW_OBSERVING` -> `CHECKPOINT_20` -> `REVIEW_20`.
Незавершённые gates дают `HOLD` с причиной, а не общий PASS.
Аналитическое решение: WATCHLIST / CONTINUE_SHADOW / REJECT_OR_ARCHIVE /
READY_FOR_SEPARATE_LIVE_REVIEW. Последняя метка не разрешает live самостоятельно.

Приёмка разработки не равна подтверждённому edge и не равна успешному deployment.
Закрытие конкретной prospective session требует terminal/model/EOD proofs,
а не только зелёного preflight или запущенного контейнера.

## 12. RI И Сохранение Baseline

RI micro07/context A остаётся неизменным контрольным контуром. Не добавлять
ему DSVD-context, OI gate, median range, новые часы, стопы или overnight.
Календарные safety tests и контроль исполнения сохраняются.

Исторические модельные показатели next-open proxy и cost sensitivity сохраняются
в отдельном аналитическом пакете, не в инженерной публикации. Они не являются
доказательством, что любое изменение RI обязательно хуже. В этом ТЗ нет
подтверждённого основания менять его alpha.

## 13. Пакет Передачи

Канонический текст:
`analiz_alpha_si/moex_oi_gate_overnight_2026_11_15/ENGINEERING_TZ.md`.
Публикуемая инженерная версия без результатов экономики:
`alor_project/bybit_barter_test/alor-rs-main/docs/moex-calendar-context-oi-shadow-engineering-tz-2026-11-15.md`.
Полный аналитический текст и публикуемая версия имеют разные границы содержимого;
не требовать совпадения их файловых SHA256. Отдельные pinned hashes календаря,
модели, входов и протоколов остаются обязательными и этим не изменяются.

ТЗ описывает ожидаемую разработку. На момент его создания не изменены Rust,
live/shadow configs, scheduler или VPS; новые arms не запущены.
Инженер сдаёт PR/commit, manifest, configs, tests/parity results, runbook,
rollout/rollback и список незакрытых gates. Коммит и deployment этого задания
не выполняются автоматически.
