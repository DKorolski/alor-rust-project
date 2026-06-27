# Live micro closure and broker migration note — 2026-06-27

## Context

On 2026-06-25, the Bank of Russia decided to annul the depositary license of ООО «АЛОР+». Public materials indicated that the broker license itself was not the same license event, but the depositary-license decision materially changed the operational and counterparty-risk profile for live trading through Alor.

On 2026-06-27, after the user initiated funds withdrawal, all order-emitting live systems were intentionally stopped.

## Operational closure

Pre-stop broker REST checks showed no open non-RUB positions:

- `7502MIW`: non-RUB open positions = 0.
- `7502T0U`: non-RUB open positions = 0.
- `7502SN6`: non-RUB open positions = 0.

Stopped services:

- `trading-ri-author41-42-7502t0u`: `strategy-runtime`, `alor-gateway`.
- `trading-ri-author41-42-7502miw`: `strategy-runtime`, `alor-gateway`.
- `trading-alor-usdrubf`: `strategy-runtime`, `alor-gateway`.
- `trading-hybrid-author41-7502t0u`: `strategy-runtime`, `alor-gateway`.
- `trading-hybrid`: `strategy-runtime`, `alor-gateway`.

Redis containers were intentionally left running to preserve operational history and allow reconciliation.

Post-stop checks:

- all `strategy-runtime` containers exited with code `0`;
- all `alor-gateway` containers exited with code `0`;
- Redis containers remained healthy;
- broker REST re-check still showed non-RUB open positions = 0 for `7502MIW`, `7502T0U`, and `7502SN6`.

## Live micro result

Broker-truth gross PnL across reviewed live micro systems was positive:

```text
+22,451.63 RUB gross
```

The result is before commissions and exchange fees because the Alor REST trade-history response returned `commission=null`.

High-level result by group:

| Group | Round-trips | Gross PnL | Read |
|---|---:|---:|---|
| USDRUBF | 104 | +5,450.00 | Stable modest positive; good simple-market gateway validation candidate. |
| IMOEXF | 139 | +4,295.00 | Positive, but requires MR/BO attribution and bracket lifecycle care. |
| RI / RTS | 70 | +12,706.63 | Strongest alpha evidence, with explicit tail-risk warning. |
| All systems | 313 | +22,451.63 | Positive live micro, not yet scale-up-ready. |

RI detail:

- `RTS-6.26`: +21,571.02 RUB, strong result.
- `RTS-9.26`: -8,864.39 RUB including the 2026-06-16 Hormuz/news-shock tail event.
- Excluding that explicit event-shock day, `RTS-9.26` was +3,660.72 RUB with profit factor about 2.05.

IMOEXF detail:

- The current best carry-forward candidate is the no-overlap hybrid / MR-priority line.
- The author41/MR-first overlap challenger is positive but less attractive on broker-truth economics and operational complexity.

## Decision read

The live micro work is considered successful as a research and operational validation phase:

- systems were able to trade real broker micro size;
- broker-truth gross PnL was positive;
- multiple hardening patches were validated live;
- the final state before broker-risk shutdown was flat and controlled.

However, the Alor depositary-license event changes the next priority from scale-up to broker migration and risk reduction.

## Carry-forward plan for a new Finam or T-Bank gateway

Port first:

1. Broker-neutral auth, portfolio, orders, trades, and positions adapters.
2. Current-session and historical broker-truth import.
3. Readiness/live-guard contract:
   - no trading until broker state is ready;
   - readiness wait rather than dropping first valid intent;
   - explicit missed-intent classification only after timeout.
4. Simple market entry/exit system, preferably USDRUBF-like micro, as first live validation.

Port second:

1. IMOEXF no-overlap hybrid / MR-priority line.
2. IMOEXF bracket lifecycle after order-id correlation and stop/limit behavior are proven.
3. RI MR micro after freeze-intent semantics are validated on the new broker.

Do not scale until:

- broker-specific replay/order/trade semantics are characterized;
- event-risk pause/kill-switch exists for RI MR;
- fees and commissions are included in net PnL;
- broker-truth reconciliation can attribute trades by strategy owner;
- MR/BO and partial-fill semantics are verified on the new gateway.

## References

- `docs/micro-effectiveness-analysis-2026-06-27.md`
- `docs/vps-live-observations-2026-06-27.md`
- `docs/vps-live-weekly-reconciliation-2026-06-16-24.md`
