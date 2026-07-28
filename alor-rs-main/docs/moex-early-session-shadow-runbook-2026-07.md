# MOEX Early Session Shadow Runbook

Date: `2026-07-17`

Status:

```text
SHADOW_ONLY
NO_LIVE_CONFIG_CHANGE
NO_ORDER_EMISSION
```

## Purpose

Run isolated `legacy09` and `canonical07` decision-only contours after the
MOEX futures session moved to continuous trading from `07:00 MSK`.

The active live/micro systems stay on the frozen `09:00` contract until a
separate reviewed promotion decision.

## Session Policy

```text
opening_auction_excluded = 06:50..06:59
legacy09_model_start     = 09:00:00
canonical07_model_start  = 07:00:00
model_session_end        = 23:49:59
weekends                 = excluded
timeframe                = closed 10m bars
```

The `06:50` auction bar must not update model state, anchors, risk-gate state,
signals, exits or volume diagnostics used by the model.

## Required Contours

Mandatory shadow pair per strategy:

```text
RI Author41/42:        legacy09 + canonical07
Alor-USDRUBF Hybrid:   legacy09 + canonical07
IMOEXF Hybrid Riskgate: legacy09 + canonical07
```

Optional diagnostic only:

```text
canonical07_legacy_bo_clock
```

The optional contour must not be pooled with the primary A/B result.

## Safety Checklist

Before starting any shadow service, verify:

```text
trade_mode = Paper or Shadow
allow_live_orders = false
allow_paper_orders = false
strategy_order_emission = false
command stream is isolated or disabled
runtime_state key is unique
consumer_group is unique
health identity is unique
journal path is unique
```

Do not attach a command-capable gateway command consumer to these contours.

If a market-data gateway is reused, it must be demonstrably read-only for the
shadow topology. A merely unused command stream is not sufficient unless a test
proves every command class is rejected.

## Clock Translation

Opening-phase clocks move by exactly `-2h`.

Late-day and EOD exits do not move.

```text
RI Author41 entry_end:       12:00 -> 10:00
RI Author41 time_exit:       unchanged 20:00
RI Author42 exit_time:       unchanged 23:00
USDRUBF MR last_entry:       11:40 -> 09:40
USDRUBF MR forced_exit:      11:50 -> 09:50
USDRUBF BO wait2 earliest:   11:00 -> 09:00
IMOEXF MR session_end:       11:59 -> 09:59
IMOEXF MR forced model exit: 11:50 -> 09:50
IMOEXF BO wait3 earliest:    12:00 -> 10:00
```

## Riskgate Isolation

IMOEXF canonical07 risk-gate ledger must use a separate key.

The active live ledger remains:

```text
runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120
```

The canonical07 shadow ledger must be a new key and must not append to the
active live ledger.

Seed metadata for the canonical07 ledger:

```text
legacy09_history_through_2026-07-13
canonical07_runtime_sessions_begin_2026-07-14
```

## Bring-Up Sequence

1. Export active VPS config manifest.
2. Generate `legacy09` and `canonical07` configs as controlled diffs.
3. Run offline replay for all complete sessions from `2026-07-14`.
4. Verify no-order and clock tests.
5. Start read-only market-data publisher if required.
6. Start shadow runtimes.
7. Confirm journals advance and command streams remain empty.
8. Observe at least five complete sessions before preliminary review.
9. Make no promotion decision before at least ten complete sessions.

## Daily Operator Checks

Record once per session:

```text
all shadow services healthy
journals advanced
zero broker commands emitted
zero broker orders/positions from shadow services
legacy09/canonical07 daily files produced
IMOEXF riskgate ledger isolated
auction bars excluded
pre-09 volume diagnostics populated
```

## RI Shadow Path Semantics

RI shadow writes two layers of decision evidence:

```text
shadow_recorded       raw finalized model decision observed by incremental runtime replay
shadow_path_active    decision currently included in the recomputed chronological no-overlap path
shadow_path_superseded previously active decision removed by a later path recompute
```

For `legacy09` vs `canonical07` economics, reduce the append-only journal to
one final status per `decision_key`:

1. order path events in append order;
2. ignore `shadow_recorded` for economic aggregation;
3. let the latest `shadow_path_active` or `shadow_path_superseded` event define
   the current status;
4. count each final-active `decision_key` exactly once.

Do not sum every `shadow_path_active` row. The in-memory path index is rebuilt
after a runtime restart, so history warm-up may append the same active
`decision_key` again without creating a new economic trade.

Do not treat raw `shadow_recorded` rows alone as the portfolio path. They are
kept for audit and can include decisions that were valid under an earlier
partial replay but were later displaced by an earlier overlapping trade.

## Paper Trading-Window Semantics

Shadow runtimes are diagnostic paper contours, but their virtual execution must
still obey the same trading-window entry filter used by live.

In particular:

```text
18:50-19:04:59 Break2 entries must not create paper positions
19:00 BO entry signals may be observed/journaled
19:00 BO entry signals must not advance paper execution
19:10 or later entry signals may advance paper execution after reopen
```

This keeps shadow reports from creating false virtual positions during exchange
breaks. The raw signal remains visible in logs/journals, but the paper position
ledger only reflects entries that live could have emitted.

## Rollback

Stop and remove only the new shadow services and their isolated consumer
groups/state keys.

Rollback must not require live strategy restart, live ledger restore, broker
position changes or broker order cancellation.
