# Изолированный сбор OI IMOEXF

Это сбор данных для будущего shadow, не запуск трёх моделей и не изменение live.
Источник OHLCV/WS, terminal finalizer, Redis, параметры и позиции действующих
стратегий не меняются. У сборщика нет торговых ключей, Redis client path или
доступа к отправке заявок. Используется один фиксированный публичный ISS endpoint.

Исторический статус 20 ноября: `STANDBY_CLOCK_HOLD`. Collect-only release и disabled timer
установлены из scoped Linux build. Настоящий GET под nonroot/systemd sandbox
получил HTTP 200 и прошёл dated schema/HTTP Date проверки, но native clock
не уложился в max_error<=1000 ms. Production store пуст, PIT capture ещё нет,
timer не включён. Лимиты часов и qualification не ослаблялись.

Оператор разрешил изменить только NTP poll ceiling до 128 секунд и повторить
проверки **после подтверждённого flat maintenance window**. До этого общая служба
времени не меняется. После настройки нужны actual kernel clock, успешный полный
launcher probe и повторная проверка текущих контуров; лишь затем enable timer.
Первый запланированный collect 23 ноября не является обещанием допуска без
закрытия clock gate. Смена NTP-сервера, ручной set-time/makestep и ослабление
uncertainty bounds этим решением не разрешены.

Обновление 21 ноября: `ARMED_COLLECT_ONLY / AWAITING_FIRST_PIT_CAPTURE`.
После проверенного flat/no-orders maintenance выполнено именно согласованное
ограничение poll: 32..128 секунд, без смены NTP provider или ручного изменения
времени. Пять native samples за 375 секунд прошли существующий clock gate;
полный launcher probe под service UID/systemd sandbox также PASS. Trading/source/
model process inventory не изменился; production store остался пуст.

Датированный timer enabled/active, Persistent=false; следующий collect
23 ноября в 06:45 МСК, всего 20 дат до 18 декабря. Своевременный PIT capture
ещё не получен: успешный diagnostic probe не заменяет первый утренний collect.
Три будущие model arms не запущены, OI не подключён к текущим стратегиям.

## Два разных режима

`oi_shadow_capture probe CONFIG` делает один диагностический GET к датированной
истории, проверяет native Linux clock, HTTP Date и существующий строгий schema
validator. Raw bytes и SHA256 входят в результат. Probe явно имеет
`diagnostic_only=true`, `qualified=false`, `allow_new_bo=false`; он не пишет
collector store и не может быть загружен как Qualification. Его можно выполнить
после закрытия утреннего окна, не подделывая своевременность данных.

`clock CONFIG` проверяет фактический read-only adjtimex, не делает HTTP и не
создаёт store. `plan CONFIG` проверяет pinned calendar и точные source dates.
Проверка `timedatectl NTPSynchronized` сама по себе недостаточна.

`collect CONFIG` сохраняет настоящий snapshot только в разрешённом окне
06:45..06:55 МСК с commit reserve и исходными clock/durable checks. Повторный
запуск использует frozen qualification; retries/reservations не обнуляются.
Положительный/нулевой OI допустим как успешно собранный snapshot с закрытым
data gate, это не HTTP failure. Missing/late/error не превращается в разрешение.

## Dated job

[Launcher](../scripts/oi_shadow_job.py) принимает manifest и его явный SHA256.
Manifest закрепляет бинарник, calendar artifact, model hash, каждый датированный
config и отдельные store/report roots. Больше 20 разрешённых дат не принимается.
Нет генерации следующей даты по weekday или автоматического продления горизонта.
Native subprocess получает только минимальные PATH/LANG, не broker environment.

Нормальный job выполняет plan -> native clock -> повторную проверку окна ->
collect. Запоздалый запуск не делает HTTP. Probe проходит отдельным режимом
и пишет только отдельный non-PIT report. Reports append-only, с file/directory
fsync; stdout содержит короткий статус, без многократного дампа HTTP body.

До установки job нужны scoped source build/hash inventory, native/Linux tests,
реальный probe под service UID и sandbox, проверка неизменности работающих
процессов, ресурсов и отсутствия созданных production qualifications от probe.
Source bundle нельзя собирать из произвольного смешанного dirty runtime.

[Шаблон service](../configs/examples/oi-shadow-capture.service) рендерится с
root-owned read-only release/config paths и отдельным nonlogin UID. Для probe
используются те же ограничения, отличается только mode. ProtectClock не включён:
он может блокировать даже adjtimex(modes=0). Отсутствие CAP_SYS_TIME и nonroot UID
не позволяют сборщику менять часы. Системное время программой не корректируется.

Timer содержит только явно разрешённые даты в Europe/Moscow, AccuracySec=1s,
без random delay, Persistent=false и без Restart loop. `enable --now` не должен
создать catch-up HTTP после cutoff. Установка timer разрешается только после
успешного native probe; неизвестное будущее расписание требует нового review.

## Календарь и первый горизонт

Для начального dated rollout 23 ноября..18 декабря выбраны 20 обычных сессий.
18..20 ноября служат явными preceding REGULAR dates; 21/22 ноября, 5/6 и 12/13
декабря классифицированы как DSVD, 28/29 ноября как CLOSED. OI использует только
две предыдущие REGULAR даты: для 23 ноября это 20 и 19 ноября, не выходные.
Для диагностического probe 20 ноября используются 19 и 18 ноября.

Календарный артефакт закрепляется отдельно от старого research calendar.
Источник расписания: [ноябрь MOEX](https://www.moex.com/n104837?nt=106),
[изменение ДСВД](https://www.moex.com/n103931?nt=112) и
[годовой регламент](https://www.moex.com/n96571?nt=101). Аукцион 06:50 не становится
модельным открытием: сохранено 07:00. В runtime нет weekday fallback; за пределами
артефакта даты UNKNOWN. Изменение календаря не перезаписывает старые qualifications.

## Наблюдение и обслуживание

После каждого утра проверить service result, machine-readable capture report,
source dates, actual receipt/durable/deadline, raw hashes и qualified result.
Отказ не скрывать и не удалять ради повторного допуска. Успешный probe не является
первым PIT snapshot; первым считается только реальный своевременный collect.

Default collector-store budget 64 MiB/4096 entries, min free disk 256 MiB;
launcher reports дополнительно ограничены 32 MiB/256 файлами. При исчерпании
не удаляются active evidence или reservations: job явно требует обслуживания.
Store не расположен в Redis и не требует Redis trim/BGREWRITEAOF.

Контрольные состояния логов: `oi_capture_finished`, `okay=true`,
`models_connected=false`, `order_emission_allowed=false`. Проверять return code
и report, а не только наличие процесса. `oi_capture_job_failed` требует разбора.
Отдельный внешний alert receiver этим job не устанавливается.

Rollback collector: остановить/отключить только его timer, дождаться или штатно
остановить только его service. Store/reports и failed attempts сохранить.
Не перезапускать live/MD и не очищать Redis. Подключение `IMO_AA / IMO_AB /
IMO_AB_OI` требует отдельного E4/admission и будущего 20-сессионного наблюдения;
нынешние model clean counters не меняются и OI-сбор не считается model-soak.
