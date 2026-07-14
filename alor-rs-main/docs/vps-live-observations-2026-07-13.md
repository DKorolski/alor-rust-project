# VPS live observations - 2026-07-13

Review window: morning and intraday live check on 2026-07-13.
Timezone in this note: Moscow time unless explicitly stated otherwise.

## Executive read

- `RI Author41/42`, `Alor-USDRUBF`, and `Hybrid IMOEXF` containers were healthy.
- `Hybrid IMOEXF` was flat with no active TP/SL or pending state during the check.
- `Alor-USDRUBF` returned flat after an MR short bracket cycle, but the TP partial-fill lifecycle was noisy and is now a watchlist patch item.
- `RI Author41/42` had an active `author42_bo` long position after the 11:00 model signal. This was controlled strategy state, not an orphan exposure.

## Alor-USDRUBF MR bracket observation

At `11:00 MSK`, `Alor-USDRUBF` generated an MR short entry on `7502MIW`:

- entry: sell qty `2` at approximately `76.80`;
- TP: buy limit qty `2 @ 76.72`;
- SL: paired stop accepted by broker;
- TP filled as two separate executions of qty `1`.

Observed lifecycle issue:

- after the first TP partial fill, runtime interpreted the broker quantity
  change as residual drift and emitted a market residual exit for qty `1`;
- the TP cancel then returned `Order to cancel not found`;
- the second TP execution arrived after that as an `orphan_trade`, temporarily
  flipping broker state to long `+1`;
- runtime emitted a second residual emergency exit and returned the portfolio
  flat.

Assessment:

- Final state was safe: broker/runtime ended flat.
- The lifecycle was not clean and matches the known bracket partial-fill /
  residual-reconcile class.
- This is now tracked in `extended-micro-watchlist-2026-06-02.md` as an open
  USDRUBF patch item before any USDRUBF scale-up.

## RI trade list since latest MR hardening

Source: `trading-ri-author41-42-7502miw-strategy-runtime-1` logs from
`2026-07-09T00:00:00Z` onward. Execution timestamps below are broker execution
confirmations from runtime logs; scheduled timestamps are the model-local
schedule carried in the emitted intent.

### 2026-07-09

1. `author41_mr` long
   - entry scheduled `09:40`, executed around `09:50`, buy `1 @ 90320`;
   - exit scheduled `11:30`, executed around `11:40`, sell `1 @ 90710`;
   - model exit reason `take_author_close`, shadow PnL `+398` points.

2. `author41_mr` long
   - entry scheduled `11:40`, executed around `11:50`, buy `1 @ 90440`;
   - exit scheduled `12:20`, executed around `12:30`, sell `1 @ 90780`;
   - model exit reason `take_author_close`, shadow PnL `+388` points.

3. `author42_bo` short
   - entry scheduled `17:00`, executed around `17:10`, sell `1 @ 88980`;
   - exit scheduled `23:00`, executed around `23:10`, buy `1 @ 87880`;
   - model exit reason `time_exit_same_bar_close`, shadow PnL `+978` points.

### 2026-07-10

1. `author41_mr` long
   - entry scheduled `09:00`, executed around `09:10`, buy `1 @ 87700`;
   - exit scheduled `09:20`, executed around `09:30`, sell `1 @ 87960`;
   - model exit reason `take_author_close`, shadow PnL `+268` points.

2. `author41_mr` short
   - entry scheduled `09:30`, executed around `09:40`, sell `1 @ 88100`;
   - exit scheduled `10:00`, executed around `10:10`, buy `1 @ 87480`.

Note: logs also show a prospective `author41_mr` long decision for `09:40 -> 09:50`
with shadow PnL `+48`, but the active broker path at that moment was the prior
short cycle, so it is not listed as a separate executed round.

### 2026-07-13

1. `author41_mr` short
   - entry scheduled `09:20`, executed around `09:30`, sell `1 @ 86530`;
   - exit scheduled `10:30`, executed around `10:40`, buy `1 @ 88250`;
   - model exit reason `stop`, shadow PnL `-1517` points.

2. `author42_bo` long
   - entry scheduled `11:00`, executed around `11:10`, buy `1 @ 88430`;
   - status at review time: open controlled `author42_bo` long, qty `1`.

## Current follow-up

- Keep RI observation active through the current BO cycle and confirm scheduled
  exit returns broker flat.
- Patch Alor-USDRUBF MR bracket partial-fill reconcile before any USDRUBF size
  increase. Local patch was prepared on 2026-07-13 and awaits flat-window VPS
  rollout.
- Continue daily audit: runtime intent, broker fill, protective TP/SL lifecycle,
  cleanup result, final broker position.

## Model-vs-live comparison read

The 2026-07-13 follow-up comparison did not indicate that the live strategy
logic was broadly broken. It separated into three classes:

- `RI Author41/42`: mostly confirms the model/live execution contract.
- `Alor-USDRUBF`: confirms the trading idea, but exposes bracket lifecycle and
  timing/execution-contract drift.
- `IMOEXF`: no current conflict in the checked fragment; flat live state was
  consistent with the model being flat after the morning move.

### RI Author41/42

The cleanest read is RI.

For 2026-07-09, all three live rounds matched the model by component, side,
scheduled time, exit reason, and shadow PnL:

- `author41_mr` long `09:40 -> 11:30`, `take_author_close`, `+398`.
- `author41_mr` long `11:40 -> 12:20`, `take_author_close`, `+388`.
- `author42_bo` short `17:00 -> 23:00`, `time_exit_same_bar_close`, `+978`.

Live broker execution occurred around the next `10m` processing bar, for example
scheduled `09:40` executed around `09:50`, and scheduled exit `11:30` executed
around `11:40`. This is consistent with the intended close-bar / next-bar live
execution contract rather than a runtime error.

For 2026-07-13, the main MR round also matched:

- model: `author41_mr` short `09:20 -> 10:30`, exit reason `stop`, shadow PnL
  `-1517`;
- live: entry scheduled `09:20` and executed around `09:30`; exit scheduled
  `10:30` and executed around `10:40`; exit reason `stop`, shadow PnL `-1517`.

The `author42_bo` long at `11:00` also existed in the model. A local replay that
closed it at `14:00` was limited by the fresh MOEX feed cutoff and should be
treated as a data-cutoff artifact, not as a real model EOD/exit claim.

Open RI contract question:

- On 2026-07-10, raw candidates included:
  - MR long `09:00 -> 09:20`, `+268`;
  - MR short `09:20 -> 10:00`, `+548`;
  - MR long `09:40 -> 09:50`, `+48`.
- Live executed the long, then the short. The prospective `09:40` long was only
  logged and not executed because the broker path was occupied by the short
  cycle.
- Local `combo_nooverlap` replay kept the `09:00` long and `09:40` long but
  removed the `09:20` short because the short entry timestamp matched the prior
  long exit timestamp.
- This is a same-bar handoff / reversal-rule contract question, not an obvious
  live execution bug.

### Alor-USDRUBF

The USDRUBF trading idea matched at a high level: the live system took an MR
short and the model also expected an MR take-style short. The mismatch is in
timing/execution details:

- live: MR short around `11:00`, sell qty `2` near `76.80`, TP buy limit qty `2`
  at `76.72`;
- refreshed model replay: MR short `10:10 -> 11:10`, entry `76.73`, exit
  `76.538`, reason `mr_take`.

This should be investigated as a focused timing replay:

- whether live and research use the same feed cut/state;
- whether live entry is one bar later than the refreshed model;
- whether bracket-entry timing differs from the signal close-bar timing;
- whether close-bar interpretation differs between runtime and replay.

The main operational defect remains bracket partial-fill reconciliation: the TP
filled as two separate qty-`1` fills, while runtime reacted to the first partial
fill as residual drift and created extra market churn before converging flat.

### IMOEXF

IMOEXF was flat during the checked live state, which did not conflict with the
model fragment. The local Author41-short replay for 2026-07-13 saw:

- short `09:10 -> 10:10`;
- exit reason `stop`.

After `10:10`, that model fragment was flat, so a later flat live state is
expected. This is only a sanity check for the Author41-short fragment and does
not replace full hybrid/riskgate parity review.
