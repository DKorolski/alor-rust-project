# Extended Micro Watchlist - 2026-06-02

This note consolidates active watchlist and engineering-backlog items from the recent
VPS extended micro soak journals. It is intended as the first page to check before
daily live-log review.

## Status Summary

Current overall status: extended micro remains operationally acceptable.

No current item below is classified as uncontrolled position risk on the VPS contours.
The main open themes are service observability, bracket/cleanup edge cases, Redis
maintenance, and corporate mirror confirmation.

## Active Watchlist

### 1. Corporate `7502T0U` USDRUBF Overnight Carry

Status: open / needs corporate logs.

Observation:

- VPS observer saw external/corporate `USDRUBF qty = 1` on `7502T0U` after the reported corporate deployment.
- VPS `Alor-USDRUBF` is on `7502MIW`, so this is not a VPS-originated position.
- The position looked consistent with a corporate `USDRUBF` long that did not close by EOD.

What to check:

- Mounted corporate runtime config:
  - `portfolio = "7502T0U"`.
  - `strategy_kind = "alor_usdrubf_hybrid"`.
  - `bars = "md.bars.7502T0U.10m"`.
  - `bo_eod_exit_time = "23:30:00"`.
  - `timezone_offset_hours = 3`.
- Mounted corporate gateway config:
  - `tf_sec = 600`.
  - `control_cws_mode = "action_scoped"`.
  - `timezone_offset_hours = 3`.
- Corporate logs around entry and EOD:
  - `signal_generated`.
  - `intent_emitted`.
  - `command acknowledged`.
  - `execution_confirmed`.
  - `position_transition`.
  - `live_guard_changed`.

Patch decision:

- Do not infer a VPS code regression until corporate mounted config and logs are checked.

### 2. `IMOEXF hybrid 7502MIW` Weekend-Gap BO Attribution

Status: open / engineering review.

Observation:

- On `2026-06-01`, BO long entry was flattened at `12:10 MSK` by `breakout_no_overnight_guard_exit`.
- The guard fired early because active/reference cycle day remained `2026-05-29` after the weekend gap.
- Risk outcome was safe because the position was flattened quickly.
- Semantics are not ideal because a same-day BO entry should not be attributed to a stale pre-weekend cycle.

What to check:

- Whether active BO cycle day is reset/reassigned correctly after non-tradable gaps.
- Whether no-overnight guard should compare against the live action day rather than stale reference day for newly emitted BO entries.

Patch decision:

- Keep as watchlist/patch candidate before treating BO weekend-gap path as fully clean.

### 3. `IMOEXF hybrid 7502MIW` MR Protective TP Action-Scoped `open_timeout`

Status: open / frequency watch.

Observation:

- On `2026-06-02`, MR short bracket entry succeeded.
- TP command was `place buy 2 @ 2582.5`, comment `HYB|...|o=MR|r=TP`, `intent_class=protective_repair`.
- Gateway confirmed action-scoped routing:
  - `control_cws_mode = "action_scoped"`.
  - `action_scope_session_open_start`, `primary_opcode="create:limit"`.
  - failure: `action_scope_session_open_error`, `error="open timeout"`.
- SL command immediately after used action-scoped `create:stopLimit` and succeeded.
- Runtime safety path flattened the position via generic `MR|r=EXIT` and canceled the remaining SL.

Interpretation:

- This is not a regression to legacy long-lived CWS.
- The failure class is transient action-scoped websocket open timeout on `create:limit`.
- Safety outcome was correct, but operator visibility could be clearer.

What to watch:

- Repeated `action_scope_session_open_error open timeout` on protective TP create-limit.
- Whether repair flatten appears more than occasionally.
- Whether any remaining stop order survives after repair flatten.

Patch candidates:

- Increase `action_scope_open_timeout_ms` if open timeouts repeat under otherwise healthy VPS/network conditions.
- Add bounded retry for protective TP before repair deadline flatten.
- Improve operator log/comment for `repair_deadline_force_flatten` so it does not appear as a generic `MR|r=EXIT`.

### 4. `IMOEXF hybrid` Size-2 Partial Fills And Cleanup Idempotency

Status: watch / no uncontrolled state observed.

Observation:

- On `2026-05-21`, size-2 TP filled in two partial fills.
- Runtime did not prematurely treat the first partial fill as full flat; the second fill completed the TP and cleanup followed.
- Later cleanup produced `Order to cancel not found` for the TP-side cancel while paired `delete_stop_limit` succeeded.
- Runtime/broker state ended flat and no active stop tail was observed.

What to watch:

- Repeated `Order to cancel not found` after TP fills.
- Delayed or failed `delete_stop_limit`.
- `stop_order_active_while_flat`.
- Any non-empty `broker.stop_orders.*` for strategy-owned stop orders after runtime and broker are flat.

Patch candidate:

- Treat cleanup-side `Order to cancel not found` as benign/idempotent when broker position is already flat and paired `delete_stop_limit` succeeds.
- Improve log wording to distinguish stale TP cancel from a genuinely active stop cleanup failure.

Scale-up note:

- Keep `IMOEXF hybrid` at qty `2` until this stays clean for several more sessions or the cleanup-idempotency patch is made.

### 5. `IMOEXF hybrid` Near-Zero MR Bracket Churn

Status: patched on VPS `2026-06-04` / validate in extended micro.

Observation:

- On `2026-05-28`, an MR short bracket opened and closed almost immediately at the same effective broker price.
- Transport/lifecycle path was clean and action-scoped.
- Working classification: `near_zero_mr_bracket_churn`.
- Likely mechanism: after tick rounding and actual fill price, TP distance became too close to the entry level.

What to watch:

- Repeated MR entries where rounded TP is less than a minimum useful distance from expected/actual entry.
- Commission/slippage drag from near-flat churn, especially at higher quantity.

Patch applied:

- Suppress MR bracket entry when rounded TP distance from expected entry is below a threshold, for example `1-2` ticks.
- Log explicit event: `mr_entry_suppressed reason=take_too_close_after_rounding`.
- Keep BO behavior and action-scoped execution unchanged.

Validation focus:

- Confirm suppressed MR entries are rare and explainable.
- Confirm no repeated near-flat MR bracket churn after size increase.
- Confirm BO behavior is unchanged.

### 6. Fill-Before-Ack `orphan_trade` Warnings

Status: known noisy class / keep monitoring.

Observation:

- Seen across RI, Alor-USDRUBF, and IMOEXF hybrid.
- In checked cases, later ack/order-map reconciliation converged and broker/runtime state ended flat.
- Current classification: service observability issue, not position-risk incident.

What to watch:

- `orphan_trade` without later matching ack/order mapping.
- `orphan_trade` followed by stale pending ids or non-flat broker state.
- Rising frequency after scale-up.

Patch candidate:

- Improve broker-truth/order-event reconciliation logs so fill-before-ack is explicitly classified as ordering delay when later ack maps the same broker order id.

### 7. Redis Maintenance And Stream Growth

Status: active maintenance item.

Observation:

- `trading-hybrid-author41-7502t0u-redis-1` grew quickly again and was around `372M / 512M` on `2026-06-02`.
- Prior safe trim showed large memory savings, especially from high-volume health/snapshot/runtime-state streams.
- Main trading Redis instances are currently manageable but still require weekly checks.

What to watch:

- Redis memory close to `512M` cap on smaller containers.
- Fast growth in:
  - `events.health*`.
  - `broker.snapshots.*`.
  - `runtime.state.*`.
  - high-frequency broker market/order streams.

Maintenance rule:

- Use whitelist safe trim only.
- Do not run broad `FLUSHALL`.
- Prefer safe window / flat state for larger maintenance.

Patch/ops candidate:

- Reduce `TRIM_MAXLEN` / health retention for noisy trial contours.
- Add regular weekend or pre-open trim job for known high-volume streams.

### 8. RI And Alor-USDRUBF MR Exit Contract Research

Status: Alor-USDRUBF MR bracket patch deployed on VPS `2026-06-04`; RI remains unchanged.

Observation:

- RI accepted live micro contract remains closed-bar condition / marketable exit to preserve validated parity.
- Alor-USDRUBF current behavior relies on validated action-scoped path and broker-truth convergence.
- There was a research idea to evaluate whether MR exits can move toward limit/bracket-style exits similar to IMOEXF hybrid to reduce commissions/slippage.
- On `2026-06-04`, Alor-USDRUBF MR was patched to use protective TP limit and SL stop-limit after MR entry confirmation.
- BO exits and MR time-cutoff / forced flatten remain marketable exit paths.

Constraints:

- This is an execution-contract change, not a small config tweak.
- Alor-USDRUBF validation must confirm action-scoped protective TP/SL install, paired cleanup, and no stale stop tail after flat.
- RI production live behavior remains unchanged until separate replay/economics review validates any bracket-style MR exit.

## Scale-Up Implications

Current scale-up posture:

- `RI`: continue extended micro; no bracket TP change for MR unless research explicitly validates it.
- `Alor-USDRUBF`: continue observation; do not alter MR exit contract yet.
- `IMOEXF hybrid`: keep qty `2` for now; do not move toward `IMOEXF 10` until partial-fill, cleanup, near-zero churn, and protective TP open-timeout watch items remain clean or are patched.

Small-readiness conditions:

- No repeated uncontrolled position tails.
- No repeated stale protective stop orders after flat.
- `orphan_trade` remains only a convergent fill-before-ack class.
- Redis growth stays within maintenance envelope.
- Corporate mirror config/logging is confirmed clean before transferring confidence from VPS to corporate infrastructure.

## Source Journals

- `vps-live-observations-2026-05-19.md`
- `vps-live-observations-2026-05-20.md`
- `vps-live-observations-2026-05-21.md`
- `vps-live-observations-2026-05-22.md`
- `vps-live-observations-2026-05-27.md`
- `vps-live-observations-2026-05-28.md`
- `vps-live-observations-2026-05-29.md`
- `vps-live-observations-2026-05-30.md`
- `vps-live-observations-2026-06-02.md`
