# MOEX Shadow Report Persistence Rollout

Date: 2026-08-01

## Problem

The USDRUBF and IMOEXF early-session shadow trade CSVs were generated from the
in-memory `TradeLedger`. After a runtime restart, the next report write replaced
the previous file with trades from the new process generation. This made the
files unsuitable as a continuous operational soak record.

The isolated IMOEXF shadow risk-gate ledgers also had no operator-visible
warning for a missing recent weekday row. The July 24 row was absent from both
isolated ledgers, while the production live ledger remained complete.

## Patch Scope

The patch does not change strategy parameters or execution behavior.

- `paper.append = true` is enabled only for USDRUBF and IMOEXF `shadow07` and
  `shadow09` configs.
- In append mode, `TradeLedger` loads the existing CSV, merges current-generation
  closed trades, removes exact replay duplicates and writes the cumulative CSV
  and summary atomically.
- Non-append paper/backtest reports retain the previous replacement behavior.
- Risk-gate startup scans the recent 21-calendar-day ledger tail and logs one
  warning with possible missing Monday-Friday dates.
- A possible weekday gap is diagnostic only. It does not block runtime, modify
  the ledger or assume that an official exchange holiday was a trading day.

No changes are made to:

- live RI, USDRUBF or IMOEXF configs;
- model clocks, K/TP/SL values or quantities;
- action-scoped order routing;
- Redis risk-gate rows, finalized guards or materialized state;
- shadow broker-command isolation.

## Operational History Boundary

The existing Jul31 CSV and summary files are preserved before rollout and form
the first retained generation of the new cumulative reports. Earlier long-range
economics remain reproducible through the frozen MOEX-data replay documented in
`moex-early-session-weekly-review-2026-08-01.md`.

Research replay rows must not be inserted into operational shadow CSVs. Mixing
those sources would hide runtime/replay drift instead of measuring it.

## Tests

Required checks:

```text
cargo test -p strategy-runtime
cargo clippy -p strategy-runtime --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Focused coverage verifies:

- a second runtime generation preserves a trade written by the first;
- a replayed closed trade is deduplicated;
- cumulative summary values include both generations;
- shadow configs remain non-emitting and enable report append mode;
- a missing Friday between Thursday and Monday is reported while weekend dates
  are ignored.

## Safe Rollout

Only these services are recreated:

```text
trading-moex-early-shadow-usdrubf-runtime-shadow07-1
trading-moex-early-shadow-usdrubf-runtime-shadow09-1
trading-moex-early-shadow-imoexf-runtime-shadow07-1
trading-moex-early-shadow-imoexf-runtime-shadow09-1
```

The corresponding Redis and gateway services remain running. RI shadow and all
live services remain untouched.

Before recreation:

1. Archive the four CSV and four summary files.
2. Record the two isolated IMOEXF ledger lengths, tails and materialized states.
3. Confirm all isolated shadow command streams are empty.

After recreation:

1. Confirm all four runtime containers are healthy with zero restart loops.
2. Confirm `paper.append=true` in each mounted config.
3. Confirm the isolated risk-gate ledger lengths and last rows are unchanged.
4. Confirm the July 24 possible-gap warning is visible and does not alter state.
5. Confirm all four shadow command streams remain empty.
6. On the next closed shadow trade, verify that the Jul31 row remains in the CSV
   and the cumulative summary count increases without duplicates.

## Rollback

Restore the previous strategy-runtime image tag and archived configs, then
recreate only the four shadow runtime services. Redis and report backups are not
deleted during rollback.

## Actual Rollout Record

Pending.
