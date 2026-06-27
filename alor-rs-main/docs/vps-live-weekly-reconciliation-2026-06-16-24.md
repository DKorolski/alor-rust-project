# VPS Live Weekly Reconciliation - 2026-06-16..2026-06-24

Review date: `2026-06-24`

Scope:

- `IMOEXF / 7502T0U`: Author41-short hybrid contour.
- `IMOEXF / 7502MIW`: hybrid riskgate / current IMOEXF contour.
- `USDRUBF / 7502MIW`: Alor-USDRUBF challenger `mr035` hybrid.

Source:

- User-provided broker round-trip tables for `2026-06-16..2026-06-24`.
- Existing local runtime notes, especially `vps-live-observations-2026-06-23.md`.
- Local research artifacts available in `analiz_alpha_si`.

## Data Availability Caveat

The local research replay artifacts are not fully current for this week:

- IMOEXF riskgate replay artifact ends before June 2026.
- IMOEXF Author41 parity artifact ends on `2026-05-25`.
- USDRUBF local broker/model comparison package is complete through `2026-06-16`.

Therefore this note is a live broker/runtime reconciliation, not a full fresh
Python backtest-vs-broker parity pass for the entire week. A strict
model-vs-broker check for `2026-06-16..2026-06-24` requires refreshing the
10-minute data and rerunning the exact Rust/Python replay or exporting runtime
decision journals from VPS.

## IMOEXF / 7502T0U / Author41-Short

Broker-completed rounds in the provided table:

- Completed rounds: `10`.
- Open positions at review time: `1 short x 3` from `2026-06-24 13:10`.
- Winning rounds: `7`.
- Losing rounds: `3`.
- Gross PnL: `+62.0` points.
- Best round: `+75.0` points on `2026-06-16`.
- Worst round: `-135.0` points on `2026-06-23`.

Important runtime confirmations:

- `2026-06-23 09:30`: runtime note confirms MR bracket short entry `qty=3`
  filled as partial `1 + 2` at `2300.0`.
- `2026-06-23 09:49`: TP buy `qty=3` filled at `2278.5`, matching the broker
  round `+64.5` points.
- `2026-06-23 10:00`: runtime note confirms the next MR short entry at
  `2292.5`.
- `2026-06-23 20:10`: runtime note confirms exit buy at `2337.5`, matching the
  broker round `-135.0` points.

Read:

- The `2026-06-23` behavior matches runtime logs and looks like intended
  Author41 MR behavior, not an execution anomaly.
- The large losing trade is a model/market outcome, not a detected runtime
  mismatch.
- The `2026-06-22` cluster of short trades is plausible for Author41 MR with
  multiple entries; the `13:20` losing short should be reviewed against runtime
  ownership if we want component-level attribution.

## IMOEXF / 7502MIW / Hybrid Riskgate

Broker-completed rounds in the provided table:

- Completed rounds: `8`.
- Open positions at review time: `1 short x 3` from `2026-06-24 13:10`.
- Winning rounds: `7`.
- Losing rounds: `1`.
- Gross PnL: `+183.0` points.
- Best round: `+75.0` points on `2026-06-16`.
- Worst round: `-9.0` points on `2026-06-22`.

Read:

- The 7502MIW IMOEXF contour strongly outperformed 7502T0U on this short weekly
  sample.
- The main behavioral difference is fewer extra MR short attempts around
  `2026-06-22..2026-06-23`, which reduced tail loss compared with Author41.
- This is consistent with the current practical stance: keep the riskgate
  version as the live baseline and keep Author41 as challenger/watchlist until
  it proves itself on a longer live window.

Note:

- The existing `2026-06-23` runtime observation says the riskgate-shadow contour
  emitted no live broker fills during that reviewed day. The user-provided table
  also has no completed 7502MIW IMOEXF round on `2026-06-23`, so there is no
  conflict for that date.

## USDRUBF / 7502MIW / Challenger MR035 Hybrid

Broker-completed rounds in the provided table:

- Completed rounds: `9`.
- Winning rounds: `2`.
- Losing rounds: `7`.
- Gross PnL: `-0.60` points.
- Best round: `+0.07` points.
- Worst round: `-0.23` points on `2026-06-22`.

Runtime confirmations available:

- `2026-06-23 09:40`: runtime note confirms MR short entry at `75.10`.
- `2026-06-23 09:57`: runtime note confirms TP/exit buy at `75.03`, matching
  broker `+0.07`.
- `2026-06-23 11:40`: runtime note confirms BO short entry at `74.59`.
- `2026-06-23 12:00`: runtime note confirms exit buy at `74.73`, matching
  broker `-0.14`.

Read:

- At least for `2026-06-23`, broker behavior matches runtime behavior.
- The week is weak economically, but the available evidence points to model
  outcome / market phase rather than a clear runtime mismatch.
- The recent bracket hardening should still stay on the watchlist because
  USDRUBF MR is sensitive to exact fill price and TP/SL anchoring.

## Cross-Contour Read

Short weekly result by provided broker tables:

| Contour | Completed rounds | Gross PnL | Win/Loss | Read |
| --- | ---:| ---:| --- | --- |
| IMOEXF 7502T0U Author41 | 10 | `+62.0` pts | `7/3` | Positive but one large MR loss dominates risk. |
| IMOEXF 7502MIW Riskgate | 8 | `+183.0` pts | `7/1` | Stronger week, fewer tail-like extra MR losses. |
| USDRUBF 7502MIW | 9 | `-0.60` pts | `2/7` | Weak week, but no confirmed behavior mismatch. |

## Preliminary Verdict

`NO CLEAR LIVE-LOGIC MISMATCH DETECTED FROM AVAILABLE LOCAL EVIDENCE`

The strongest confirmed reconciliation is `2026-06-23`, where runtime notes line
up with the broker table across IMOEXF Author41, USDRUBF, and RI. For the rest
of the week, the broker tables are internally plausible, but a strict
model-trade parity check requires fresh replay data or VPS decision journals.

Operational stance:

- Do not replace IMOEXF riskgate with Author41 based on this week.
- Keep IMOEXF riskgate as the stronger live baseline.
- Keep Author41 as challenger/watchlist; its week is still positive, but the
  `2026-06-23` tail loss is exactly the type of event we wanted to observe.
- Keep USDRUBF at current micro size until more post-hardening evidence is
  collected.

## Next Reconciliation Step

To close this as a true model-vs-live comparison:

1. Export or refresh 10-minute IMOEXF and USDRUBF data through `2026-06-24`.
2. Rerun exact model replay for:
   - IMOEXF riskgate hybrid;
   - IMOEXF Author41-short hybrid;
   - USDRUBF challenger `mr035`.
3. Join replay trades to broker rounds by date, side, entry/exit time, and
   price.
4. Mark differences as:
   - expected fill/latency/slippage;
   - component ownership difference;
   - model drift;
   - runtime/execution anomaly.

