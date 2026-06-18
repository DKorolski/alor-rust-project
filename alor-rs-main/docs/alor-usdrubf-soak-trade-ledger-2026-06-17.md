# Alor-USDRUBF Soak Trade Ledger And Version Notes

Date: 2026-06-17

Scope: `alor_usdrubf_hybrid_v1` live/micro soak observations for `USDRUBF`.

This ledger is a best-effort reconstruction from VPS observation documents plus
current VPS docker logs. It is not a broker accounting export. Rows marked
`docs` are reconstructed from daily observation notes. Rows marked `logs` are
confirmed from the currently retained `trading-alor-usdrubf-strategy-runtime-1`
docker log window.

## Broker Terminal Reconciliation Addendum

On 2026-06-17 the broker terminal paired-cycle export was added as a stronger
source than the reconstructed rows below. Treat this section as the canonical
economics read for the paired `USDRUBF` cycles listed by the broker terminal;
keep the reconstructed rows below for operational/version context only.

Summary from the broker paired-cycle table:

- Cycles: `34`.
- Gross result: `+0.64` `USDRUBF` price units before commissions.
- Approximate gross money result for qty 1: `+640 RUB`, using `0.01` price
  point ~= `10 RUB`.
- Win/loss/flat: `18` positive, `15` negative, `1` flat.
- May subtotal: `+1.01` price units.
- June subtotal through 2026-06-16: `-0.37` price units.
- Pre-2026-06-04 subtotal: `+0.02` price units.
- 2026-06-04 and later subtotal: `+0.62` price units.
- 2026-06-12 and later current deployed contour subtotal: `+0.25` price units.

Daily broker paired-cycle totals:

| Date | Cycles | Gross price units | Notes |
| --- | ---:| ---:| --- |
| 2026-05-04 | 1 | +0.02 | Long EOD-style close. |
| 2026-05-06 | 1 | +0.03 | Short quick close. |
| 2026-05-08 | 1 | +0.12 | Short quick close. |
| 2026-05-11 | 1 | -0.07 | Short loss. |
| 2026-05-13 | 1 | +0.01 | Near-flat short. |
| 2026-05-14 | 1 | 0.00 | Flat short. |
| 2026-05-18 | 1 | +0.16 | Short held to 15:00. |
| 2026-05-19 | 1 | +0.32 | Short held to 18:20. |
| 2026-05-20 | 1 | -0.09 | Long EOD-style loss. |
| 2026-05-26 | 1 | +0.72 | Best single cycle in the export. |
| 2026-05-27 | 2 | -0.23 | One win, one larger loss. |
| 2026-05-28 | 1 | -0.08 | Short loss. |
| 2026-05-29 | 2 | +0.10 | One win, one near-flat loss. |
| 2026-06-01 | 1 | -0.12 | Long loss. |
| 2026-06-02 | 3 | +0.14 | Two short losses, one long EOD-style win. |
| 2026-06-03 | 4 | -1.01 | Main negative cluster before the MR bracket fix line. |
| 2026-06-04 | 1 | -0.52 | BO-style long loss; not an MR bracket validation. |
| 2026-06-05 | 1 | -0.30 | BO-style long loss; not an MR bracket validation. |
| 2026-06-08 | 1 | +0.63 | Short EOD-style win. |
| 2026-06-09 | 1 | +0.28 | Short EOD-style win. |
| 2026-06-11 | 2 | +0.28 | Broker ledger shows same-time short close and long open at 10:53. |
| 2026-06-15 | 4 | -0.08 | Includes MR TP fill plus residual/race cleanup row. |
| 2026-06-16 | 1 | +0.33 | Long EOD-style win. |

Operational read from the broker export:

- The old reconstructed ledger undercounted and misattributed several rows,
  especially around portfolio/version transitions. Broker paired cycles should
  supersede those economics.
- The 2026-06-03 cluster is the main negative outlier and predates the later MR
  bracket lifecycle hardening line.
- The 2026-06-15 paired rows confirm that the MR bracket TP did its economic job
  (`+0.08`), but also confirm the residual/race issue (`-0.01`) already captured
  in the watchlist and local patch.
- The 2026-06-11 same-time close/open reversal should remain on the watchlist
  until it is reconciled against runtime intent ownership and model component.

## Version Timeline

| Period | Portfolio | Version / contour | MR exit contract | Notes |
| --- | --- | --- | --- | --- |
| 2026-04 to 2026-06-03 | mostly `7502T0U`, later `7502MIW` | pre-bracket live contour | MR `mr_take` / `mr_stop` emitted market/action-scoped exits after model-bar condition | Functional, but MR take could slip because the exit was sent after the bar instead of resting as TP. BO used market/action-scoped exits. |
| 2026-06-04 to 2026-06-11 | `7502MIW` | `manual-20260604-mrbracket-guard` | MR entry installs TP limit + SL stop-limit; MR time cutoff and BO still market | Deployed with runtime-only from-zero. First checked post-rollout trades were BO, so no bracket validation until the next MR. |
| 2026-06-12 to current VPS | `7502MIW` | `manual-20260612-bracket-residual`, config `/configs/runtime.alor_usdrubf.live.7502MIW.challenger_mr035.toml` | MR bracket enabled | Current active version. Params observed in config: `mr_k_short=0.035`, `mr_take_k_short=0.16`, `mr_stop_k_short=0.43`, `mr_force_exit_time=11:50`, `bo_k=0.45`, `bo_wait_hours=2`, qty 1. |
| Prepared, not deployed as of 2026-06-17 morning | local repo only | bracket lifecycle repair patch | suppress protective repair after sibling TP/SL terminal fill until broker-flat truth | Prepared after 2026-06-15 bracket race. Tests pass, but rollout still pending. |

## Confirmed / Reconstructed Trade Cycles

Gross is in `USDRUBF` price units before fees. For `USDRUBF`, `0.01` price
point is approximately `10 RUB`, so `1.00` price unit is approximately
`1000 RUB` per contract. Commission is not subtracted unless noted.

| Date | Source | Version class | Component | Entry MSK | Exit MSK | Side | Qty | Entry | Exit | Gross | Operational notes |
| --- | --- | --- | --- | --- | --- | --- | ---:| ---:| ---:| ---:| --- |
| 2026-04-15 | docs | pre-bracket | BO | 11:02 | 13:14 | short | 1 | 75.03 | 75.40 | -0.37 | Several `protocol_reset` retries; final broker flat. |
| 2026-04-15 | docs | pre-bracket | BO | 13:44 | 13:51 | long | 1 | 75.49 | 75.56 | +0.07 | `bo_stop1_long` exit, final broker flat. |
| 2026-04-16 | docs | pre-bracket | MR | 09:07 | 09:45 | short | 1 | 76.03 | 75.96 | +0.07 | `mr_take` market/action-scoped exit after one rejected attempt. |
| 2026-04-16 | docs | pre-bracket | BO | 11:03 | 23:32 | long | 1 | 76.50 | 76.19 | -0.31 | Many `bo_stop1_long` market exits rejected; EOD retry flattened. |
| 2026-05-04 | docs | pre-bracket | BO | 11:30 | 23:40 | long | 1 | 75.54 | 75.67 | +0.13 | EOD exit, broker flat. |
| 2026-05-05 | docs | pre-bracket | BO | 11:10 | 12:00 | short | 1 | 75.38 | 75.55 | -0.17 | Stop-style BO exit, broker flat. |
| 2026-05-07 | docs | pre-bracket | BO | 11:30 | 12:00 | long | 1 | 74.81 | 74.78 | -0.03 | `bo_stop1_long`, broker flat. |
| 2026-05-11 | docs | pre-bracket | BO | 11:20 | 12:00 | short | 1 | 73.93 | 74.00 | -0.07 | `bo_stop1_short`, broker flat. |
| 2026-05-12 | docs | pre-bracket | BO | 11:10 | 12:00 | long | 1 | 74.00 | 73.86 | -0.14 | `bo_stop1_long`, broker flat. |
| 2026-05-14 | docs | pre-bracket | BO / uncertain duplicate day slice | n/a | n/a | short | 1 | 73.56 | 73.66 | -0.10 | Observation note records clean market entry/exit; exact component not restated in the excerpt. |
| 2026-05-14 | docs | pre-bracket | BO | 11:10 | 23:40 | short | 1 | 73.36 | 73.19 | +0.17 | EOD exit, broker flat. |
| 2026-05-15 | docs | pre-bracket | BO | 12:40 | 23:40 | short | 1 | 73.13 | 73.07 | +0.06 | EOD exit, broker flat. |
| 2026-05-19 | docs | pre-bracket | BO | 11:10 | 23:40 | short | 1 | 72.56 | 71.93 | +0.63 | Action-scoped market path, broker flat. |
| 2026-05-20 | docs | pre-bracket | BO | 11:10 | 23:40 | short | 1 | 71.35 | 70.56 | +0.79 | One `orphan_trade` on EOD fill; converged flat. |
| 2026-05-20 | docs | pre-bracket | BO | 11:10 | 23:40 | long | 1 | 71.05 | 71.40 | +0.35 | From 2026-05-21 cycle summary; broker flat. |
| 2026-05-21 | docs | pre-bracket | MR | 09:50 | 10:00 | short | 1 | 71.59 | 71.28 | +0.31 | `mr_take` market exit; one `orphan_trade` ordering warning, broker flat. |
| 2026-05-21 | docs | pre-bracket | BO | 11:10 | 12:00 | short | 1 | 71.03 | 70.95 | +0.08 | Broker flat. |
| 2026-05-27 | docs | pre-bracket | BO | 11:10 | 23:40 | short | 1 | 72.02 | 71.30 | +0.72 | EOD exit, broker flat. |
| 2026-05-27 | docs | pre-bracket | BO | n/a | 12:00 | short | 1 | 71.03 | 71.37 | -0.34 | From same observation file follow-up; treat as separate broker/log slice until full ledger is reconciled. |
| 2026-06-01 | docs | pre-bracket | BO | 11:10 | 12:00 | long | 1 | 71.63 | 71.51 | -0.12 | `bo_stop1_long`, broker flat. |
| 2026-06-05 | docs | post-2026-06-04 bracket version | BO | 11:20 | 12:00 | long | 1 | 73.92 | 73.62 | -0.30 | No bracket expected because this was BO, not MR. |
| 2026-06-09 | docs | post-2026-06-04 bracket version | BO | 11:20 | 23:40 | short | 1 | 71.99 | 71.71 | +0.28 | No bracket expected because this was BO. |
| 2026-06-15 | logs | current VPS `manual-20260612-bracket-residual` | MR | 09:50 | 09:57 | short | 1 | 72.64 | 72.56 | +0.08 | MR bracket TP filled. Then a repair race emitted a second TP before flat reconciliation. |
| 2026-06-15 | logs | current VPS `manual-20260612-bracket-residual` | MR residual safety | 09:57 | 09:57 | long residual | 1 | 72.56 | 72.55 | -0.01 | Not a model trade. Residual was created by duplicate TP protective repair and then flattened by emergency exit. |
| 2026-06-15 | logs | current VPS `manual-20260612-bracket-residual` | BO | 11:20 | 12:00 | long | 1 | 72.82 | 72.66 | -0.16 | `bo_stop1_long`, market/action-scoped, broker flat. |
| 2026-06-15 | logs | current VPS `manual-20260612-bracket-residual` | BO | 16:00 | 17:00 | short | 1 | 72.13 | 72.12 | +0.01 | `bo_stop1_short`, market/action-scoped, broker flat. |
| 2026-06-16 | logs | current VPS `manual-20260612-bracket-residual` | BO | 16:50 | 23:40 | long | 1 | 72.46 | 72.79 | +0.33 | `bo_eod_exit`, market/action-scoped, broker flat. |

## Observed Totals From This Best-Effort Ledger

The rows above sum to approximately:

- Pre-bracket rows: `+1.73` USDRUBF price units gross before fees, with several
  repeated transport/retry incidents in April.
- Post-2026-06-04 but before the first MR bracket validation: `-0.02` gross,
  both rows were BO and therefore did not exercise MR TP/SL.
- Current 2026-06-12+ log-confirmed rows: `+0.25` gross if the residual safety
  row is included, or `+0.26` gross excluding the residual bug row.

These totals are only directional because the ledger is reconstructed from
observation notes rather than a complete broker statement.

## MR TP/SL Transition Read

The MR bracket transition itself is visible on 2026-06-15:

```text
09:50 MSK: MR short entry filled at 72.64
TP buy limit: 72.56
SL buy stop-limit trigger: 72.85
09:57 MSK: TP filled at 72.56
```

The intended economic effect worked: TP rested at the model take level instead
of waiting for a later market exit. However, the current deployed runtime then
repaired protection before broker-flat reconciliation completed:

```text
TP filled -> broker flat event in progress
second TP repair emitted at 72.56
second TP filled -> unexpected long residual
residual emergency exit sold at 72.55
final broker state flat
```

This is why the local follow-up patch is important before treating the bracket
MR path as production-clean.

## Current Recommendation

Keep `Alor-USDRUBF` at qty `1` until the bracket lifecycle patch is deployed and
one or more fresh MR bracket cycles complete without:

- duplicate TP/SL repair after terminal sibling fill;
- residual emergency exit;
- stale working order or stale stop order after broker flat;
- repeated orphan/fill-before-ack state confusion.

BO cycles on 2026-06-15 and 2026-06-16 looked operationally clean on the current
VPS logs, but they do not validate the MR bracket path.
