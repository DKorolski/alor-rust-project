# Перенос Новых Shadow-Моделей На FINAM

Инженерный контракт для реализации аналогов на отдельном FINAM-контуре.
Это не live-rollout, не разрешение отправлять заявки и не замена работающих
стратегий. Полный переносимый handoff передаётся отдельно: исходные freeze
bundles/fixtures/архивы не дублируются в engineering repository.

## Выбранные Профили

| Модель | Независимые context policies | Что не переносится |
| --- | --- | --- |
| IMOEXF Author41-short + Hybrid BO053, reaction07, combo | weekday_only / weekend_context_no_trades | High180/lb120 seed, Author42 BO, broker brackets |
| USDRUBF Previous-day07 H4/W4 | weekday_only / weekend_context_no_trades | Live VWAP challenger035, live brackets, H3/W2 |
| RI Author41/42 prospective07 | runtime_accepted_only / ri_confirmed_closing_context_v1 | Weekend anchors, BO max2/last-entry17 experiments |

Итого шесть read-only виртуальных веток с отдельными position/pending/counters,
frozen context и generation/checkpoint. Для нового IMOEXF shadow riskgate seed
не нужен. У RI общий MR max2 сохраняется; отсутствие BO max2 его не отменяет.

## Данные И Исполнение

Canonical external M10 OHLCV, начальная метка бара UTC, модельный календарь
Москва UTC+3. Историческое имя BarEvent.close_time_utc в этих адаптерах означает
bar-BEGIN; окончание бара и фактические receipt/durable/processing timestamps
учитываются отдельно. Не заменять M10 произвольной M1-агрегацией или получать
фиктивное original receipt time из позднего history fetch.

Regular weekday labels07:00..23:40; до14.07.2026 history starts09:00.
Service/auction bars не входят в weekday model state. Нужны два допустимых
prior context sessions, не120дней прогрева. Для IMO/USD weekend policy меняет
только anchors, не разрешает weekend trading. RI weekends всегда исключены.
Calendar exclusions и фактический экспирационный контракт задаются явно.

Context замораживается на calendar cutoff открытия, не на первом обработанном
event. Поздние revisions идут в audit, без переписывания уже использованных
баров и решений. Restart сохраняет frozen view и idempotency; изменение
profile/symbol/config означает новый согласованный generation.

IMO/USD используют reaction-next-open: closed-bar signal T -> next consecutive
M10 open T+10m; при closed-bar-only источнике proxy становится известен после
закрытия следующего бара. Signal/action/availability различаются. Stop/take/
breakeven здесь не означают broker limit/stop fill по уровню. EOD23:30 ->
action23:40 требует настоящего terminal label23:40 после23:50; missing evidence
не заменяется fill на следующей сессии. У RI отдельный prospective MR/BO
contract и BO EOD23:00; не унифицировать его с reaction engine.

Frozen costs IMOEXF: point10RUB, tick5RUB, rate/component/side
0.00006575119054451348, components2. Старый synthetic rate0.001не подходит.
Для USD сохраняются previous-day anchor/scale и выбранные H4/W4, не intraday
VWAP. Реальные FINAM fees/fills впоследствии учитываются отдельно от model PnL.

## FINAM Adapter И Приёмка

Реализовать FINAM-native closed-M10 adapter с instrument mapping, честной
availability/closure evidence, durable ingress, isolation и local decision/state
audit. ALOR-specific source_proof/grants/Redis bindings не являются обязательным
FINAM wire contract. Численная модель переносится отдельно от broker adapter.
Shadow не должен иметь торговый client/credential или command-producer path.

Сначала одинаковые synthetic inputs и independent expected_state_trace:
previous/previous2/scale, quotas, BO observation/levels, owner/position/pending,
suppressed branches, scheduled times, events/trades и restart-prefix. Одних
PnL/trade counts недостаточно. Затем native FINAM terminal/EOD/restart/readiness
smoke и отдельное prospective наблюдение; локальный model PASS не сертифицирует
FINAM production и не означает GO live. Неполный/catch-up день не включается
в clean-session count только по имени journal disposition.

Reference implementation:

- [IMOEXF numerical core](../strategy-runtime/src/bin/imoexf_shadow_replay/model.rs).
- [USDRUBF numerical core](../strategy-runtime/src/bin/usdrubf_shadow_replay/model.rs).
- [RI prospective facade](../strategy-runtime/src/strategies/ri_author41_42_live/frozen_shadow.rs).
- [Independent parity tools](../scripts/shadow_state_parity/README.md).
- [Isolation and promotion gates](shadow-observation-and-live-promotion.md).

Отдельный handoff содержит русские model passports, selected profile JSON,
исходники, неизменённые frozen manifests, inputs/expected state traces и
comparator. Его bounded numerical qualification:52scenarios/4148state rows,
с отдельными negative/fee arithmetic checks. Это engineering qualification,
не результаты live-soak или cross-broker historical parity.

Обмен журналами между VPS, transport/schema ежедневной сверки и решения о
promotion будут отдельным ТЗ. Здесь они не реализуются. В публичный engineering
repository не добавляются handoff ZIP, source clones, broker exports, soak
journals, credentials или реальные server/account deployment bindings.
