# Устойчивость новых shadow контуров и перенос опыта на FINAM

Дата: 19 ноября 2026 года; статус дополнен утром 20 ноября.
Получатель: разработчик отдельного FINAM-контура.

Это операционное дополнение к [handoff моделей](finam-new-shadow-models-handoff-2026-11-13.md).
Оно объясняет, какие изменения позволили новым ALOR shadow обрабатывать данные
без постоянных диагностических перезапусков и какие проверки нужны на FINAM.
Численные модели, параметры стратегий и разрешение на live здесь не меняются.

**Статус на утро 20 ноября:** после общего recovery-патча от 19 ноября обе
reaction-пары прошли настоящий terminal/EOD и ночные reconnect без restart
и потери допущенного префикса. Сегодняшние бары обрабатываются. Подтверждён
первый реальный ночной цикл, но многосессионная проверка устойчивости ещё
продолжается; это не завершённый soak и не разрешение на live.

## 1. Главный результат

Устойчивость достигалась не ослаблением всех проверок и не увеличением прогрева.
Мы разделили получение данных, неизменяемую публикацию, расчёт модели и допуск.
Для транспорта использовали уже отработанные механизмы gateway, но в отдельном
MD-only процессе, без торгового контура. Штатное восстановление источника теперь
не должно уничтожать исправное состояние модели.

```text
Broker market data + history
    -> MD-only source: subscribe / reconnect / history / candle finalizer
    -> durable immutable closed-M10 stream + source audit
    -> read-only reader -> durable ingress with real receipt timestamps
    -> supervisor: readiness / context / deadlines / admitted prefix
    -> deterministic model -> state / decisions / virtual trades
```

Для FINAM важна эта схема ответственности, а не обязательное наличие WebSocket.
Если его gateway уже имеет устойчивый native streaming transport, следует
использовать его. REST остаётся пригодным для bootstrap, истории и аудита;
не нужно каждому shadow независимо опрашивать историю вместо общего
сопровождаемого источника текущих закрытых баров.

## 2. Какие модели относятся к этому опыту

| Модель | Две независимые ветки | Существенное отличие от live |
| --- | --- | --- |
| IMOEXF Author41-short MR + Hybrid BO053, reaction07 | `weekday_only`, `weekend_context_no_trades` | Не High180/lb120, без seed riskgate и без broker brackets |
| USDRUBF Previous-day07 H4/W4 | `weekday_only`, `weekend_context_no_trades` | Не live VWAP challenger035 и не H3/W2 |
| RI Author41/42 prospective07 | `runtime_accepted_only`, `ri_confirmed_closing_context_v1` | Отдельный prospective engine, без weekend anchors |

Каждая ветка имеет собственные position, pending, counters и frozen context.
Последний Python recovery-патч относится к двум reaction-парам IMOEXF/USDRUBF;
RI нельзя автоматически считать проверенным этим патчем. У RI сохраняются
no-overlap и общий MR max2; отсутствие BO max2 не отменяет ограничение MR.

## 3. Почему первые версии останавливались

| Проблема | Что изменили | Урок для FINAM |
| --- | --- | --- |
| Отдельный REST-путь новых моделей имел другую availability и обработку истории, чем старые gateway-fed shadow | Выделили общий MD-only источник с проверенными subscription/recovery механизмами | Сначала переносить зрелую операционную модель транспорта, не писать упрощённый второй gateway |
| Последний M10 бар не закрывался без следующего бара и мог появиться лишь на следующей сессии | Добавили source-side terminal finalizer настоящего накопленного бара | Отдельно проверить session boundary, не ждать завтрашнюю котировку |
| История могла прийти раньше финального subscription ACK | Буферизуем кадры, но readiness даём только после явного ACK и проверки покрытия | Пришедшие данные сами по себе не доказывают успешную подписку |
| После нескольких часов работы reconnect проверял лимит от возраста процесса | Отсчитываем ограниченный период unready монотонными часами от потери readiness | Десятичасовой процесс не должен исчерпывать recovery за первую секунду разрыва |
| Проверка источника после combined poll могла обнаружить сбой уже после шага модели | Разделили `collect` и `advance`, проверяем источник между ними | Не смешивать сетевое чтение и разрешение модели принять новые данные |
| Diagnostic late-start навсегда отключал recovery | Recovery включается для каждого совместимого WS worker, независимо от способа первого старта | Качество первого дня и способность процесса восстанавливаться являются разными признаками |
| Новая дата модели, права на evidence или рост audit создавали лишние HOLD | Явно связываем source/model generations, проверяем права реальным UID, рассматриваем конкретный контекст | Не исправлять проблему подменой manifest, удалением audit или запуском под root |

Последняя строка не означает, что уже реализован универсальный автоматический
классификатор всех revisions. Разовые рассмотренные допуски и постоянная
context-aware процедура имеют разные границы приёмки.

## 4. Время, прогрев и контекст

Canonical input: external M10 OHLCV с UTC-меткой **начала** интервала и календарём
Москвы UTC+3. Например, label `07:00` относится к `07:00..07:10` и обычно становится
доступен после `07:10`. Историческое имя `BarEvent.close_time_utc` в этих ALOR
адаптерах означает bar-BEGIN; это нужно явно перевести в схему FINAM.

Полная regular weekday session имеет labels `07:00..23:40`, всего 101 бар.
Аукционный/service label `06:50` не входит в модельный weekday state.
В истории до 14 июля 2026 года действуют прежнее начало `09:00` и
transition-aware правила полноты. Выходные могут быть контекстом только
соответствующей IMOEXF/USD ветки, но не разрешают ей торговать в выходные.
У RI weekend anchors исключены. Исключения календаря и контракт фьючерса
задаются явно; weekday-проверки недостаточно для всех праздничных расписаний.

Нужны два квалифицированных предыдущих контекста, не 120 дней истории.
При текущем ALOR bootstrap использовали три полные обычные сессии, 303 бара.
Число 120 относится к другому live High180/lb120 riskgate, не к новым shadow.

Контекст замораживается на установленном calendar cutoff, а не на первом
задержавшемся обработанном event. При допуске проверяются previous/previous2,
completeness, scale, selection frontier и используемые моделью поля.
Restart не должен заново выбирать другой anchor из уточнённой истории.

Раздельно сохраняются bar label, signal/scheduled action, original receipt,
durable receipt и processing time. Поздняя история не становится своевременным
Live event. Пограничный случай запуска требует повторной проверки актуального
источника и cutoff, а не backdate timestamps ради прохождения допуска.

## 5. Последний бар и EOD

IMOEXF/USDRUBF reaction engine: сигнал по closed bar T исполняется в модели
по open строго следующего M10 T+10m. Если источник отдаёт только закрытые
свечи, этот open достоверно известен после закрытия следующей свечи.

Пример: EOD signal label `23:30` -> action label `23:40` -> доступность
последней свечи после `23:50`. Это model timing, не broker fill по этим часам.
RI имеет другой prospective contract и BO EOD `23:00`; переносить на него
reaction fill semantics нельзя.

Source-side terminal finalizer ALOR работает так:

1. Хранит реальный текущий candle buffer; обычный бар закрывает при следующей метке.
2. После session boundary и grace 10 секунд проверяет календарь, ACK, WS/writer freshness, историю и обработку очереди полных входных событий.
3. Дожидается завершения history writes и дренирует очередь; таймер не обгоняет уже полученные обновления.
4. Публикует buffered terminal bar через durable pending -> canonical append -> commit watermark.
5. При потерянном подтверждении публикации читает точный ID и payload, а не делает слепой повторный append.

Timer/next-label race не должен дать две свечи. Таймер является основанием
закрыть настоящий буфер, но не источником выдуманной цены, пустой свечи или fill.
Если требуемого terminal нет, сессия неполная: закрытие на следующем дне нельзя
выдать за соблюдение no-overnight. Это as-observed cutoff, не гарантия, что
брокер больше никогда не уточнит свечу.

На FINAM сначала проверить, не выдаёт ли native gateway корректный terminal
уже сейчас. Если выдаёт, дополнительный finalizer не нужен. Новые ALOR
`source_proof`/grant поля не являются обязательным FINAM `BarEvent` contract:
достаточно эквивалентного проверяемого evidence в adapter-owned sidecar/journal.

## 6. Recovery без потери состояния

Начальный допуск строгий: готовый источник, подтверждённая подписка, покрытие,
неизменённый опубликованный префикс и пригодный контекст. Разрешение продолжить
уже допущенный процесс после разрыва является отдельным механизмом.

```text
READY -> connection/readiness lost -> PAUSED_RECOVERY
      -> ACK + coverage + same canonical prefix restored
      -> collect/revalidate durable cursor -> admitted advance -> READY

identity/canonical corruption or deadline exhausted -> INCOMPLETE/FAILED
```

Источник восстанавливает broker transport. Supervisor модели сам к брокеру
не переподключается: сохраняет worker/state, приостанавливает новые шаги и
проверяет восстановление. Новое поколение подписки в том же разрешённом процессе
допустимо; rollback поколения или неподтверждённая замена процесса недопустимы.

Протокол `split_collect_advance_v1`:

1. `collect` сохраняет полученные входы в durable ingress, не выполняя модель.
2. Supervisor повторно проверяет source readiness и сроки; при разрыве ждёт, затем повторяет collect/проверку overlap от сохранённого cursor.
3. `advance` синхронно обрабатывает только допущенный фиксированный префикс, без Redis read/await и без добавления receipts.
4. Supervisor сверяет неизменность префикса; recovery считается завершённым после подтверждённого advance при готовом источнике.

Если связь пропала во время уже допущенного синхронного advance, он может
закончить свой фиксированный префикс, но не взять новые входы. Это не атомарная
транзакция между брокером, Redis и моделью. Worker без split capability
не должен молча возвращаться к небезопасному combined poll.

## 7. Разные таймауты решают разные задачи

Параметры ALOR ниже являются reference, не готовыми настройками для FINAM.
Его интервалы heartbeat/keepalive и распределение задержек нужно измерить отдельно.

| Проверка | ALOR reference | Смысл |
| --- | --- | --- |
| Writer heartbeat | 15 секунд | Жив ли и обновляет ли состояние сам источник; просрочка остаётся hard failure |
| WS RX age | 45 секунд | Readiness threshold; после допуска превышение ведёт в pause/recovery, не сразу убивает модель |
| Transport recovery | До 600 секунд | Ограничение периода восстановления, но раньше может истечь model/queue deadline |
| Ordinary source collection | IMOEXF 90, USD 300 секунд | Допустимое получение ordinary candle, не срок WS reconnect |
| Ordinary model lateness | IMOEXF 120, USD 360 секунд | Отдельный лимит своевременного расчёта модели |
| Queue age | 30 секунд | Локальная очередь не должна бесконечно удерживать уже полученный вход |
| Terminal model lateness | 120 секунд | Ordinary USD budget не расширяет EOD window |

Recovery считается монотонными часами; flapping не обнуляет его до
подтверждённого прогресса. Потеря источника вне торгового окна также не разрешает
бесконечное ожидание. Проверяется более ранний из действующих deadlines.
Общее увеличение всех лимитов не исправляет неверную state machine.

В status нужны source identity/generation, ACK/history readiness, writer/RX age,
last published bar, reader cursor, admitted prefix, last model bar и
`next_guard_deadline_utc`. На переходах логировать причину pause, elapsed recovery,
resume и его prefix; отдельно показывать `session_complete`, clean counter,
position/pending и запрет orders. Существование процесса или отсутствие ERROR
не доказывают, что cursor движется и модель получила все обязательные бары.

## 8. Что именно исправили 19 ноября

Обе reaction-пары имели один supervisor и одинаковый WS RX budget. IMOEXF
пересёк 45-секундный порог до завершения восстановления, USDRUBF успел раньше.
Это независимые socket/readiness интервалы, а не доказательство лучшего recovery
у USDRUBF или влияние его 360-секундного model budget.

Причина: `late_start` использовался не только для классификации неполного
первого дня, но и исключал процесс из split recovery на всём сроке жизни.
После первого чистого дня эта ошибка не исчезала сама.

В [supervisor](../scripts/reaction_shadow_service.py) убрали diagnostic
исключение из выбора split-пути и разрешения recovery. Проверка
`split_collect_advance_v1`, строгий начальный допуск, prefix/deadline guards
сохранены. Численная модель, Rust worker, gateway, finalizer и live не менялись.
Патч применили к обеим парам, а не только к успевшему упасть инструменту.

Диагностический старт означает неполный первый день, а не начало модели
в 09:00 и не запрет восстанавливаться в следующие дни.

## 9. Уточнения истории и допуск

Первый принятый canonical bar остаётся неизменным. Поздние OHLC/volume revisions
записываются отдельно, не вызывают модель повторно и не исправляют прошлый PnL.
Для revised-history эксперимента нужен отдельный replay и другая identity.

Сам reconnect или рост audit не должны автоматически создавать HOLD.
Проверяется влияние на dependency trace обеих веток: набор допустимых sessions,
выбор anchors, coverage и все используемые features. Совпадение дневного OHLC
не доказывает state parity. Volume нельзя глобально считать несущественным:
в частности, для VWAP-моделей он влияет на расчёт.

HOLD нужен при отсутствии обязательных данных к deadline, изменении canonical
prefix, нарушении identity или неразобранном существенном изменении выбранного
контекста. Если автоматический classifier ещё не принят, нужен явный scoped
review, а не blanket allow. В ALOR постоянная dependency-aware admission policy
выделена отдельно от последнего runtime recovery-патча.

## 10. Изоляция, restart и ресурсы

Shadow технически не имеет trading client, order producer или broker trading
credentials. MD-only source пишет только свой market-data stream; model reader
имеет только необходимые read/transaction permissions на этот stream. Ни одна
роль не может писать broker commands. Проверка denial обязательна, одного
`order_emission_allowed=false` в логе недостаточно.

Запускать под отдельными nonroot UID, с проверенными владельцами evidence,
минимальными filesystem/network permissions и ограничением ресурсов. Preflight
выполнять реальным service UID под его sandbox, а не только root-проверкой.
Evidence и бинарники должны иметь закреплённые версии/hashes.

Healthy source можно продолжать использовать для новой согласованной model
generation: source activation и model activation не обязаны совпадать. Но это
явная continuity binding, а не редактирование старого manifest или копирование
чужого grant. Начальный контекст и reader prefix проверяются заново.

Штатный reconnect восстанавливается внутри процесса. Crash/reboot, FAILED и
ручной stop имеют отдельный restart contract; обычный `Restart=always` его
не заменяет. Failed generation сохраняется. Новый диагностический запуск
получает новые identities/counters, а не поддельный clean STOP старого процесса.
From-zero модели не означает FLUSH Redis и не требует сбрасывать healthy source.

Контролировать RSS, диск, stream rows/bytes, journal/audit size, Redis persistence
errors/evictions и запас до установленного capacity limit. Health достаточно
держать в небольшом обновляемом state с логами по изменениям, а не бесконечно
дублировать heartbeat в Redis. Архивирование/retention не должны удалять входы
под активным cursor или evidence, нужный для restart. Планировать обслуживание
между сессиями; не применять общий trim ко всем ключам без разбора владельцев.

## 11. Приёмка на FINAM

| Проверка | Требуемый результат |
| --- | --- |
| Frozen numerical/state fixtures | Совпадают previous/previous2, scale, counters, BO observation, owners/pending, suppressions, decisions и trades; не только итоговый PnL |
| История до ACK | Данные сохранены, но READY не выставлен до подтверждения и полного покрытия |
| Разрыв после нескольких часов работы | Один ограниченный recovery interval от разрыва, не от запуска процесса |
| Reconnect дольше RX threshold, но в пределах model deadline | PAUSED_RECOVERY -> ACK/history -> same prefix -> один resume без restart и повторных решений |
| То же после diagnostic late-start | Качество первого дня не отключает последующее recovery |
| Flapping, missing bar, stale writer, replacement source | Deadline не продлевается, нарушения identity/liveness останавливают модель |
| Разрыв между collect и advance | Модель не получает недопущенные receipts; prefix mutation обнаруживается |
| Terminal timer/next-label race и потерянный append ACK | Одна настоящая свеча, readback по точному ID/payload, без duplicate append |
| Поздняя correction и повторная история | Audit сохранён, принятый prefix/state не переписан, решения не повторены |
| Stop во время recovery, crash/reboot и explicit resume | Состояние и причины сохранены, незавершённая попытка не превращена в clean |
| Реальный terminal/EOD обеих веток | Same-session terminal, session_complete, durable state, flat и no pending |
| Order isolation | Торговые команды технически запрещены на всех входах |

Рекомендуемый порядок: сначала independent model/state parity; затем composed
source + storage + real worker fault-injection tests на собственных тестовых
сервисах; затем FINAM-native source-only terminal/reconnect canary; только после
этого pre-open допуск моделей и prospective soak. Не инъецировать сбои в live.

Цель наблюдения новых пар ALOR: 20 чистых сессий, затем review, без автоматического
GO live. Diagnostic/catch-up дни не включаются. Для FINAM нужен собственный
prospective счётчик; ALOR clean sessions не являются его квалификацией.

## 12. Что уже доказано и что ещё наблюдаем

До последнего supervisor-патча на реальном ALOR источнике подтверждены
same-session terminal publication и полный paired EOD. На 19 ноября общий
recovery release прошёл 134 Linux procedure/supervisor/admission tests и
18 controller tests. Это 152 проверки ограниченного операционного scope,
не повторный полный Rust workspace и не cross-broker parity.

После перехода обе reaction-пары продолжают обработку свежих баров без
перезапуска. День перехода диагностический; clean EOD предыдущей generation
сохраняется в явной lineage, новые counters не переписываются.

Дополнение от 20 ноября: после патча обе пары обработали terminal label 23:40
за 19 ноября и завершили paired EOD около 23:50:17, с 101 модельным баром в
каждой ветке, flat и no pending. Ночью IMOEXF прошёл три, USDRUBF один цикл
`PAUSED_RECOVERY -> READY`. Сохранены source/model PID, нет restart; resume
подтверждён на том же допущенном префиксе 404 receipts. Обе пары затем приняли
сегодняшние model bars и заморозили контексты 19 и 18 ноября. Это реальное
подтверждение проверенной recovery-ветки, не только synthetic fixture.

19 ноября по-прежнему не считается чистой сессией. Утром 20 ноября новые clean
counters равны нулю; сегодняшний день только eligible и может быть засчитан
после полного EOD. Один успешный ночной цикл не заменяет последующее наблюдение
задержек, revisions, terminal/EOD и restart по отдельному контракту.

## 13. Инженерные ориентиры в ALOR

- [MD-only entry point](../alor-gateway/src/bin/alor_shadow_md_ws.rs): изоляция от trading/CWS.
- [Source runner](../alor-gateway/src/md_source/runner.rs) и [storage](../alor-gateway/src/md_source/storage.rs): readiness, публикация и durable prefix.
- [Terminal finalizer](../alor-gateway/src/terminal_bar.rs): session-boundary закрытие настоящего буфера.
- [Reaction worker](../strategy-runtime/src/bin/reaction_shadow_worker.rs) и [runner service](../strategy-runtime/src/shadow_session_context/collector/runner/service.rs): split collect/advance.
- [Supervisor regressions](../scripts/test_reaction_shadow_service.py) и [composed tests](../scripts/test_shadow_strict_composed.py): recovery, deadlines, terminal, stop.
- [State parity tools](../scripts/shadow_state_parity/README.md): независимая проверка модели.
- [Recovery contract от 19 ноября](reaction-shadow-ws-recovery-contract-2026-11-19.md): последняя поправка supervisor.
- [Source runbook](shadow-md-ws-source-runbook-2026-11-02.md) и [source reuse](shadow-normal-source-reuse-admission-contract-2026-11-16.md): launch/resume/continuity; датированные статусы ранних этапов не равны текущему допуску.
- [Context-aware admission](shadow-context-aware-admission-work-plan-2026-11-15.md): граница scoped review и будущего постоянного classifier.

Переносить нужно закреплённую проверенную ревизию и согласованные model configs,
а не произвольный dirty checkout. Для FINAM не нужны реальные ALOR accounts,
server bindings, credentials, operational grants и журналы экономики.
Повторяется контракт и тестовый путь, а не история временных ручных исключений.
