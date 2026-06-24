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
- On `2026-06-04`, `trading-hybrid-author41-7502t0u` logged one `orphan_trade` on an `IMOEXF` MR short entry.
- The broker lifecycle still converged cleanly: TP filled, paired SL canceled, and position returned flat.

What to watch:

- `orphan_trade` without later matching ack/order mapping.
- `orphan_trade` followed by stale pending ids or non-flat broker state.
- Rising frequency after scale-up.

Patch candidate:

- Improve broker-truth/order-event reconciliation logs so fill-before-ack is explicitly classified as ordering delay when later ack maps the same broker order id.

### 7. Redis Maintenance And Stream Growth

Status: active maintenance item / systemd timer installed on VPS `2026-06-05`.

Observation:

- `trading-hybrid-author41-7502t0u-redis-1` grew quickly again and was around `372M / 512M` on `2026-06-02`.
- It was around `399M / 512M` during the `2026-06-05` pre-open check.
- Prior safe trim showed large memory savings, especially from high-volume health/snapshot/runtime-state streams.
- Main trading Redis instances are currently manageable but still require weekly checks.
- On `2026-06-05`, safe trim reduced `trading-hybrid-author41-7502t0u-redis-1` from about `393M` Redis-reported memory to about `39M`.
- A systemd timer now runs `/opt/maintenance/redis_safe_trim.sh --apply` on the VPS at `08:10 MSK`, Monday-Friday.
- The local source script is `alor-rs-main/scripts/redis_safe_trim.sh`.
- By `2026-06-06 09:13 MSK`, the author41 Redis had already regrown to about `286M / 512M`, mainly from `events.health`, `broker.snapshots`, and `broker.positions`.
- A flat-state safe trim on `2026-06-06` reduced it to about `64M / 512M` without stopping services or trimming protected runtime/risk-gate state.
- The current Monday-Friday timer leaves a weekend maintenance gap; daily scheduling or lower source-side retention remains an operations candidate.

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
- Add regular weekend or pre-open trim job for known high-volume streams. Current VPS implementation uses systemd timer `redis-safe-trim-live-soak.timer`, but its Monday-Friday schedule is not sufficient for the observed author41 weekend growth rate.
- Source-side per-stream gateway retention was deployed as an author41 canary
  on `2026-06-06`. Health heartbeat remains periodic, while health,
  snapshots, and positions are now bounded independently at
  `1500/2000/2000`. Observe the canary before rolling it to other contours.

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

### 9. IMOEXF Hybrid Stale BO Marketable-Limit Entries

Status: stale passive-entry risk mitigated and first live validation completed
on `2026-06-09`; BO retry-policy and action-scope open-timeout follow-up remain
open.

Observation:

- On `2026-06-08`, both IMOEXF hybrid contours emitted the same BO short entry
  from the `15:00 MSK` model bar:
  - `7502MIW`: sell `4` at `2547.5`, broker order
    `2033126304043449002`.
  - `7502T0U`: sell `2` at `2547.5`, broker order
    `2033126304043448923`.
- The signal reference close was `2548.0`. The runtime applied one aggressive
  tick and submitted sell limits at `2547.5`.
- By the time the broker accepted the commands, the market had moved below the
  limit. Both intended marketable limits became passive working orders and
  remained live for several hours.
- Both orders had `ttl_ms = null` and broker time-in-force `OneDay`.
- The runtime did not cancel them on the next model bar because a working order
  prevents stale pending-entry garbage collection.
- The operator manually canceled both orders on `2026-06-08`; broker events
  confirmed `status=canceled`, `filled=0`.
- On the next `10m` model bar at `17:30 MSK`, both runtimes cleared the old
  pending entry through `hybrid_pending_gc_entry` and generated
  `BreakoutShort` again because the signal condition was still true.
- `7502MIW` was live-ready and the repeated sell `4` entry filled immediately
  at `2526.5`, opening `IMOEXF qty=-4`.
- `7502T0U` was temporarily blocked by gateway reconnect during that model bar,
  so its repeated signal was not emitted and the portfolio remained flat.
- Before the `2026-06-09` session, both IMOEXF hybrid contours were switched
  from `live_order_style = "marketable_limit"` to
  `live_order_style = "market"`.
- Both corresponding gateways now explicitly enable
  `action_scope_enable_market = true` while retaining
  `control_cws_mode = "action_scoped"`.
- The rollout was config-only and preserved Redis runtime/risk-gate state.
  Startup verification confirmed both runtimes resolved `Market`, both
  gateways resolved action-scoped Market from file, both containers were
  healthy, and both IMOEXF portfolios were flat with no working entry orders.
- During the `2026-06-09` regular session, both contours completed their first
  action-scoped Market BO entry/exit cycles after the rollout:
  - author41-short `7502T0U` entered and exited cleanly;
  - primary `7502MIW` entered cleanly, then its first exit attempt encountered
    an action-scope `open timeout`;
  - the primary contour subsequently handled a closed-window retry through
    deferred exit and reissued successfully after reopen;
  - both contours ended broker-flat without a passive working-order tail.
- The primary entry produced one fill-before-ack `orphan_trade` warning, which
  later converged through acknowledgement and broker truth.

Risk:

- A stale BO entry can execute much later, after the original breakout signal is
  no longer actionable.
- The same behavior occurred simultaneously on both hybrid profiles, so this is
  a shared runtime execution-contract issue rather than a profile-specific
  signal issue.
- Canceling a stale broker order alone does not invalidate or mark the original
  BO opportunity as attempted. The runtime can therefore re-enter on a later
  bar at a materially different price.

Implemented mitigation:

- Use action-scoped Market for hybrid entry and marketable-exit intents, matching
  the already validated RI and Alor-USDRUBF one-shot execution path.
- Keep MR protective TP limit and SL stop-limit commands on their existing
  action-scoped protective paths.

Remaining patch/watch candidate:

- Define BO entry retry semantics explicitly. Preferred safe default: one
  bounded execution attempt per BO signal/cycle; do not automatically re-emit
  the same signal after transport failure or broker rejection unless a
  separately validated retry policy permits it.
- Add explicit events that distinguish Market command transport failure,
  broker rejection, unknown outcome, and retry eligibility.

Validation focus:

- Continue confirming hybrid entry/exit uses `primary_opcode=create:market`
  through the action-scoped path.
- Confirm no new passive working BO entry can remain after a one-shot intent.
- Confirm Market command timeout/unknown-outcome handling converges through
  broker truth without duplicate entry.
- Confirm a failed/rejected Market command does not silently create a later,
  worse-price re-entry for the same BO signal.
- Track action-scope session-open timeout frequency. The first live validation
  recovered safely, but this transport class is not considered closed.

## Scale-Up Implications

### 10. Confirmed Bracket Residual Shared-Class Bug, 2026-06-11

Status: patched locally / affected VPS runtimes stopped / controlled rollout
required.

Observed:

- `7502MIW / USDRUBF`: two TP buy limits filled against one short and created an
  unexpected long `+1`.
- `7502T0U / IMOEXF`: TP closed only part of the short, paired SL cleanup
  followed, and short `-1` remained without broker protection.
- Both residuals were manually flattened after stopping the affected runtimes.

Patch posture:

- do not treat TP order fill as broker-flat;
- do not retry an unknown protective create outcome on the next model bar;
- on non-zero broker quantity change or sign flip, enter close-only, cancel
  known protection, and flatten the exact residual;
- stabilize fractional stop-limit tick normalization.

Rollout:

- deploy only while both portfolios are broker-flat;
- affected contours restart `from zero`; validate IMOEXF partial-fill handling
  at quantity `2`, while RI and Alor-USDRUBF remain at quantity `1`;
- require `3-5` clean validation sessions before restoring previous size.

Detailed incident note:

- `live-incident-note-2026-06-11-bracket-residuals.md`

### 11. Bracket Terminal Fill vs Sibling Cleanup, 2026-06-17

Status: open / patch required before scale-up.

Observed:

- `7502MIW / IMOEXF hybrid` entered an MR long, qty `2`.
- TP limit filled and broker position became flat.
- The paired SL stop-limit remained `working` after TP fill.
- Runtime emitted `delete_stop_limit` for the sibling SL, but the cleanup command
  failed with `cws_error` because Alor OAuth refresh returned `502 Bad Gateway`.
- Runtime logged:
  `cleanup_ack_error_with_active_stop_while_flat`, `working_stop_orders_count=1`.
- Operator manually canceled the stale SL. Broker stream later confirmed
  `stop_order_id=121741481 status=canceled`.

Risk:

- A stale sibling SL after broker-flat can reopen the opposite position if it
  triggers later.
- The failure was transport/auth-path related, but the risk sits in runtime
  lifecycle semantics: terminal TP fill must not leave a working sibling stop
  without a robust retry/reconcile path.

Patch posture:

- Deploy and validate the bracket residual/race patch before any lot-size
  increase.
- Treat terminal TP/SL fill and broker-flat reconciliation as separate states.
- Keep cleanup retry/reconcile active until sibling protection is confirmed
  canceled or broker truth proves no strategy-owned stop remains.
- Emit operator-visible events for `flat_with_active_stop`,
  `sibling_cleanup_failed`, `sibling_cleanup_retry`, and
  `sibling_cleanup_confirmed`.

Scale-up gate:

- Do not increase `USDRUBF` lot size now.
- Do not increase `IMOEXF hybrid` size above the current validation size while
  this class remains open.
- After patch rollout, collect at least `30-50` complete broker rounds or
  `15-20` active trading days before revisiting `USDRUBF` lot-size increase.

Daily audit requirement:

- Compare broker rounds vs runtime intents vs model replay, not PnL alone.
- Required fields for the daily audit:
  - `signal_ts` / model component;
  - entry intent and broker fill;
  - protective TP/SL install acks;
  - exit reason and broker fill;
  - sibling cleanup result;
  - final position reconcile.
- Classify any mismatch as model drift, Rust state-machine drift, broker
  execution drift, or bracket lifecycle drift.

Current scale-up posture:

- `RI`: continue extended micro; no bracket TP change for MR unless research explicitly validates it.
- `Alor-USDRUBF`: continue micro observation at qty `1`; do not increase lot
  size until bracket residual/race hardening is deployed and the post-patch
  audit window is clean.
- `IMOEXF hybrid`: do not increase current quantities until partial-fill,
  cleanup, near-zero churn, protective TP open-timeout, and stale BO-entry
  watch items remain clean or are patched.

Small-readiness conditions:

- No repeated uncontrolled position tails.
- No repeated stale protective stop orders after flat.
- `orphan_trade` remains only a convergent fill-before-ack class.
- Redis growth stays within maintenance envelope.
- Corporate mirror config/logging is confirmed clean before transferring confidence from VPS to corporate infrastructure.

## 2026-06-22 Partial-Entry Follow-Up

The first post-patch quantity-3 partial entry was observed on both IMOEXF
portfolios at `13:20 MSK`:

- the broker filled the entry as `1 + 2` within milliseconds;
- target quantity `3` was reached well inside the `3000 ms` timeout;
- no residual recovery, emergency flatten, reject, or stale protection
  followed;
- the later full-size exits returned both portfolios to flat.

Watch items:

- The observed entry belonged to `BO`, while the diagnostic text said
  `MR entry partially filled`.
- Review whether accumulation/timeout handling should be explicitly limited to
  `EntryStyle::Bracket`, leaving BO/ordinary market semantics untouched.
- Until that review, treat current behavior as operationally convergent but the
  diagnostic/scope wording as imprecise.
- Continue waiting for the first genuine MR bracket partial fill before marking
  the MR-specific production acceptance complete.
- Continue monitoring gateway disconnect/gap-sync events. The `15:35-15:40
  MSK` event blocked trading through the live guard and recovered to
  `ALLOWED / LiveReady` without order activity.

## 2026-06-23 RI Morning-Open Freeze-Contract Latency

Status: P1 parity watch before RI scale-up / local patch prepared / no
uncontrolled state observed.

Observation:

- On both RI micro-live contours (`7502MIW` and `7502T0U`), the first
  `09:00 MSK` candidate was generated after overnight sync but dropped by the
  runtime guard:
  - `intent_dropped_bar_silence`;
  - `intent_dropped_by_trading_window`;
  - state transition reverted back to `flat`.
- The next actionable cycle used scheduled entry `09:10 MSK`, but commands were
  prepared and emitted on the next processing pass around `09:20 MSK`.
- Fills then completed normally:
  - `7502MIW`: short entry filled at `94910`, exit at `93400`;
  - `7502T0U`: short entry filled at `94890`, exit at `93400`.
- Later broker snapshots showed `RTS-9.26 = 0` on both portfolios.

Interpretation:

- The intended RI freeze contract is: closed-bar signal -> next-bar-open
  execution proxy.
- A compliant live path for a signal on the `09:00-09:10 MSK` bar is market
  intent emitted immediately after that signal bar closes, with broker fill
  around `09:10 MSK`.
- The observed `09:10 -> 09:20 MSK` behavior would be a one-bar-late execution
  drift relative to the freeze contract and should be treated as a P1 parity
  issue before RI scale-up.
- `entry_price = bar.close` in runtime state does not by itself prove a
  violation; the audit must compare `model_signal_ts`, scheduled entry,
  `ri_command_prepared` / `intent_emitted`, and broker fill timestamps.

What to watch:

- Repeated `09:00 MSK` `intent_dropped_bar_silence` / trading-window drops on
  otherwise healthy morning starts.
- Whether a `09:00-09:10 MSK` signal emits market intent immediately after
  `09:10 MSK` or drifts to the `09:20 MSK` processing pass.
- Any mismatch between `model_signal_ts_local`, `scheduled_ts_local`,
  `created_ts_utc`, `intent_emitted`, and broker `execution_confirmed`.
- Any divergence between `7502MIW` and `7502T0U` RI entry timing after morning
  sync.

Patch candidate:

- Review RI morning-start readiness and freeze-contract enforcement so a fresh
  closed signal bar can execute on the next-bar-open proxy immediately after
  `ALLOWED / LiveReady`, without weakening stale-bar protection after real
  reconnect gaps.
- Local patch prepared on `2026-06-24`: live bar callback still receives the
  previous-bar context, but runtime order gating for intents produced by that
  current live bar uses the current bar timestamp as freshness proof. See
  `ri-next-bar-open-timing-hardening-2026-06-24.md`.

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
- `vps-live-observations-2026-06-04.md`
- `vps-live-observations-2026-06-05.md`
- `vps-live-observations-2026-06-06.md`
- `vps-live-observations-2026-06-07.md`
- `vps-live-observations-2026-06-09.md`
- `vps-live-observations-2026-06-15.md`
- `vps-live-observations-2026-06-20.md`
- `alor-usdrubf-soak-trade-ledger-2026-06-17.md`
