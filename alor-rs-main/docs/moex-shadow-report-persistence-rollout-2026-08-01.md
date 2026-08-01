# MOEX Shadow Report Persistence Rollout

Date: 2026-08-01

## Problem

The USDRUBF and IMOEXF early-session shadow trade CSVs were generated from the
in-memory `TradeLedger`. After a runtime restart, the next report write replaced
the previous file with trades from the new process generation. This made the
files unsuitable as a continuous operational soak record.

The isolated IMOEXF shadow risk-gate ledgers also had no operator-visible
warning for missing recent weekday rows. The production live ledger remained
complete.

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

The patch was committed and deployed on 2026-08-01 during the weekend window.

```text
commit: 5b394ae
image: ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260801-shadow-reports-5b394ae
image id: sha256:6ee6f6a7eeea3a3cb7fc9ed042120ad6528c642f7b89d9f96f3193d876267042
architecture: amd64
```

Pre-rollout resources were approximately 5.9 GiB available RAM and 48 GiB free
disk space. The image was built on the VPS from a clean `git archive` of the
commit. No environment or token files were included in the build context.

Backups and Redis exports were stored under:

```text
/opt/rollout-candidates/5b394ae
```

Only the two USDRUBF and two IMOEXF shadow runtime services were recreated.
Their Redis and gateway services, both RI shadow runtimes and all live services
were left running.

Post-rollout checks:

- all four target containers became healthy on the first start;
- mounted configs resolved with `paper.append=true`;
- Jul31 CSV line counts were unchanged by the restart;
- both isolated risk-gate streams remained at 186 rows;
- shadow07 retained last finalized `2026-07-30`, rolling `152.7`, gate enabled;
- shadow09 retained last finalized `2026-07-30`, rolling `161.8`, gate enabled;
- startup refreshed only `current_shadow_session_date` to `2026-08-01`;
- no risk-gate record was inserted or removed;
- all isolated shadow command streams remained absent/empty;
- all nine live/shadow runtime containers were healthy after rollout.

The continuity warning reported these absent weekday rows:

```text
2026-07-09, 2026-07-10, 2026-07-13, 2026-07-14, 2026-07-15,
2026-07-16, 2026-07-17, 2026-07-20, 2026-07-21, 2026-07-24
```

The Jul9-Jul21 interval predates continuous ledger ownership by the isolated
shadow generations and is classified as a historical observation gap. Jul24 is
the gap inside the active isolated-ledger observation interval. None of these
dates were automatically backfilled, and the complete production live ledger
was not modified.
