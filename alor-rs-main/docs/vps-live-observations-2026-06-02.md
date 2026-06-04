# VPS Live Observations - 2026-06-02

## Checkpoint

Time checked: `2026-06-02 09:12-09:25 MSK`.

Context:

- Regular morning health/log review after the `2026-06-01` live session.
- Corporate `7502T0U` note: reported `Alor-USDRUBF` did not exit by end of day; corporate logs are not available yet.
- VPS is still running the local extended micro soak on `7502MIW` plus the trial `hybrid-author41` observer/strategy on `7502T0U`.

## VPS Resource Snapshot

Containers:

- `trading-ri-author41-42-7502miw-*`: healthy.
- `trading-alor-usdrubf-*`: healthy.
- `trading-hybrid-*`: healthy.
- `trading-hybrid-author41-7502t0u-*`: healthy.

Host resources:

- RAM: `7.7 GiB` total, about `5.7 GiB` available.
- Swap: `3.9 GiB` total, about `42 MiB` used.
- Disk `/`: `79G` total, `36G` used, `40G` available, `48%`.

Redis memory:

- `trading-ri-author41-42-7502miw-redis-1`: about `184.7M / 512M`.
- `trading-alor-usdrubf-redis-1`: about `168.2M`.
- `trading-hybrid-redis-1`: about `164.4M`.
- `trading-hybrid-author41-7502t0u-redis-1`: about `372.3M / 512M`.

Interpretation:

- Host resources are healthy.
- `trading-hybrid-author41-7502t0u-redis-1` is again growing quickly and remains on the Redis maintenance watchlist.
- The likely growth source remains service/snapshot streams, not trading command streams.

## 2026-06-01 Session - VPS Trading Read

### RI Author41/42 `7502MIW`

Observed path:

- `09:10 MSK`: `author41_mr` short entry emitted.
- Execution path: `action_scoped_only`.
- Order symbol: `RTS-6.26`.
- Entry fill: sell `1` at `113770`.
- `10:40 MSK`: scheduled/closed-bar exit emitted.
- Exit fill: buy `1` at `113290`.
- `10:50 MSK`: `author41_mr` long entry emitted.
- Entry fill: buy `1` at `113500`.
- `11:10 MSK`: scheduled/closed-bar exit emitted.
- Exit fill: sell `1` at `113720`.

Runtime state after session:

- `phase = flat`.
- `current_component = null`.
- `pending_entry_request_id = null`.
- `pending_exit_request_id = null`.
- `last_transition_reason = live_position_flat_confirmed`.

Notes:

- One `orphan_trade` warning appeared around the first RI exit, consistent with the known broker-truth/order-event ordering nuance.
- No command reject, timeout, or CWS error path was observed for RI.

### Alor-USDRUBF `7502MIW`

Observed path:

- `11:00 MSK`: `bo_long_signal` accepted into pending entry.
- `11:10 MSK`: live entry emitted, buy `USDRUBF` qty `1`.
- Entry fill: `71.63`.
- `12:00 MSK`: exit emitted with reason `bo_stop1_long`.
- Exit fill: sell `1` at `71.51`.

Runtime state after session / morning check:

- `hybrid_state = flat`.
- `open_position_owner = null`.
- `open_position_qty = 0`.
- `entry_intent_inflight = false`.
- `exit_intent_inflight = false`.
- `bo_was_long_today = true` for `2026-06-01`.

Interpretation:

- VPS `Alor-USDRUBF` behaved coherently and was flat after the stop exit.
- The VPS contour is running on `7502MIW`, not `7502T0U`.

### IMOEXF Hybrid Riskgate `7502MIW`

Observed path:

- `09:10 MSK`: riskgate finalized previous regular session row:
  - `session_date = 2026-05-29`.
  - `shadow_pnl_points = 0.0`.
  - `ledger_rows_count = 202`.
  - `rolling_sum_lb120 = 173.8`.
  - `mr_enabled_current_session = true`.
- `12:00 MSK`: BO long entry emitted.
- Entry fill: buy `2` IMOEXF at `2584.0`.
- `12:10 MSK`: `breakout_no_overnight_guard_exit` fired and forced BO exit.
- Exit fill: sell `2` IMOEXF at `2584.0`.

Important watchlist item:

- The `breakout_no_overnight_guard_exit` fired very early in the day because the active/reference cycle day remained `2026-05-29` after the weekend gap:
  - `dt_local = 2026-06-01 12:10:00`.
  - `reference_day = 2026-05-29`.
  - `active_cycle_day = 2026-05-29`.
  - `previous_day_local = 2026-06-01`.
- This was safe from a risk perspective because it flattened the BO position quickly.
- However, it is not the intended semantic for a same-day BO entry on `2026-06-01`; keep as a patch/watchlist item for weekend-gap cycle attribution.

Morning `2026-06-02` riskgate read:

- `risk_gate_shadow_session_finalized` for `2026-06-01`.
- `shadow_pnl_points = 0.0`.
- `shadow_trade_count = 0`.
- `ledger_rows_count = 203`.
- `rolling_sum_lb120 = 164.6`.
- `mr_enabled_current_session = true`.

### IMOEXF Hybrid Author41-Short `7502T0U`

Observed VPS strategy path:

- `12:00 MSK`: BO long entry emitted.
- Entry fill: buy `1` IMOEXF at `2584.0`.
- `14:50 MSK`: BO stop exit emitted with reason `BreakoutStop1Long`.
- Exit fill: sell `1` IMOEXF at `2571.5`.

Runtime state after session / morning check:

- `active_cycle_id = null`.
- `current_owner = null`.
- `current_side = null`.
- `pending_entry_request_id = null`.
- `pending_exit_request_id = null`.
- `last_position_qty = 0.0`.

Interpretation:

- The `hybrid-author41` VPS strategy itself is flat and has no pending intent.
- It does not use riskgate seed/ledger; `risk_gate_*` fields are null/zero as expected.

## Corporate `7502T0U` External Observation

The VPS `trading-hybrid-author41-7502t0u` gateway sees the full broker portfolio snapshot for `7502T0U`.

Latest snapshot during the check showed:

- `USDRUBF qty = 1.0`.
- `avg_price = 71.90`.
- `RUB qty` around `93k`.

Related broker stream evidence visible from the VPS observer:

- `2026-06-01 13:21 MSK` approximately:
  - `USDRUBF` buy `1`.
  - order type `limit`.
  - fill price `71.90`.
  - order comment is `null`.
- The same `USDRUBF qty=1` was still present in `7502T0U` snapshots during the `2026-06-02 09:12 MSK` check.

Interpretation:

- This position does not come from the active VPS `Alor-USDRUBF` contour, because VPS `Alor-USDRUBF` is currently on `7502MIW`.
- This is consistent with the reported corporate `7502T0U` long that did not close by end of day.
- Since corporate logs are not available yet, root cause is still pending.

Working hypotheses for the corporate issue:

- Corporate runtime may have been deployed with default Helm config instead of explicit `--set-file runtime.configBody=...` and `gateway.configBody=...`.
- Corporate runtime may have started/synced later and built different live state.
- Corporate logs or UI may be displayed in UTC/browser time, creating time interpretation drift.
- Corporate runtime may not have processed EOD/exit bars due to feed/sync/readiness issue.
- Need corporate `signal_generated`, `intent_emitted`, `command_acknowledged`, `execution_confirmed`, `runtime_state`, and mounted config snippets to confirm.

Requested corporate checks:

- Confirm mounted runtime config:
  - `portfolio = "7502T0U"`.
  - `strategy_kind = "alor_usdrubf_hybrid"`.
  - `bars = "md.bars.7502T0U.10m"`.
  - `bo_eod_exit_time = "23:30:00"`.
  - `timezone_offset_hours = 3`.
- Confirm mounted gateway config:
  - `tf_sec = 600`.
  - `control_cws_mode = "action_scoped"`.
  - `timezone_offset_hours = 3`.
- Provide logs around entry and EOD:
  - `signal_generated`.
  - `intent_emitted`.
  - `command acknowledged`.
  - `command rejected`.
  - `execution_confirmed`.
  - `position_transition`.
  - `live_guard_changed`.

## Morning 2026-06-02 Status

Live guard:

- All active VPS runtimes returned to `LiveReady / ALLOWED` after morning history/gap sync.

Broker state:

- `7502MIW` snapshots show target instruments flat:
  - `RTS-6.26 qty = 0`.
  - `USDRUBF qty = 0`.
  - `IMOEXF qty = 0`.
- `7502T0U` snapshot shows external/corporate `USDRUBF qty = 1`, while VPS `hybrid-author41` own IMOEXF state is flat.

No new `2026-06-02` strategy entries were observed in the checked morning window.

## Intraday Follow-Up 2026-06-02

### IMOEXF Hybrid `7502MIW` MR Bracket / Repair Path

Observed path around `10:00-10:10 MSK`:

- `10:00 MSK`: MR short bracket entry emitted and accepted.
- Entry fill: sell `2` IMOEXF at `2583.5`.
- Runtime emitted protective TP and SL:
  - TP command: `place buy 2 @ 2582.5`, comment `HYB|sid=hybrid_imoexf|c=6a1e7d1805|o=MR|r=TP`, `intent_class=protective_repair`.
  - SL command: `create_stop_limit buy 2`, trigger `2590.5`, price `2591.0`, comment `HYB|sid=hybrid_imoexf|c=6a1e7d1805|o=MR|r=SL`, `intent_class=protective_repair`.
- Gateway config on VPS confirmed:
  - `control_cws_mode = "action_scoped"`.
  - `action_scope_enable_create_limit = true`.
  - `action_scope_force_token_refresh_before_authorize = true`.
- Gateway log for the TP confirmed action-scoped routing:
  - `action_scope_session_open_start`, `primary_opcode="create:limit"`.
  - failure was `action_scope_session_open_error`, `error="open timeout"`.
- Gateway log for the SL confirmed action-scoped routing and success:
  - `primary_opcode="create:stopLimit"`.
  - fresh token refresh / authorize succeeded.
  - broker stop order `121563338` created.
- Because TP failed while SL remained working, runtime later flattened via the MR repair safety path:
  - exit command `place buy 2`, comment `HYB|sid=hybrid_imoexf|c=6a1e7d1805|o=MR|r=EXIT`.
  - fill: buy `2` IMOEXF at `2583.0`.
  - remaining SL cleanup via `delete_stop_limit order_id=121563338` was accepted.

Interpretation:

- This is not a regression to legacy long-lived CWS; the TP went through the intended action-scoped path.
- The failure class is a transient action-scoped websocket `open_timeout` on `create:limit`.
- Risk outcome was safe: the repair deadline / incomplete-bracket safety path flattened the position and cleaned up the working SL.
- Operator readability is weaker than desired because the forced repair flatten appears as a generic `MR|r=EXIT`; consider adding a more explicit `repair_deadline_force_flatten` log/comment marker if this repeats.

## Watchlist

- Corporate `7502T0U` `USDRUBF qty=1` overnight carry: pending corporate logs/config confirmation.
- Main `IMOEXF hybrid 7502MIW` weekend-gap BO attribution:
  - early no-overnight rescue at `12:10 MSK` on `2026-06-01`;
  - likely needs engineering review before treating this path as clean.
- Main `IMOEXF hybrid 7502MIW` MR bracket protective TP action-scoped `open_timeout`:
  - observed once on `2026-06-02` for TP `create:limit`;
  - confirmed not legacy-path regression;
  - safety repair flattened the position and cleanup canceled the remaining SL;
  - watch frequency before deciding whether to patch `action_scope_open_timeout_ms`, add protective TP retry, or improve repair-flatten observability.
- `trading-hybrid-author41-7502t0u-redis-1` memory:
  - about `372M / 512M`;
  - schedule safe trim / reduce gateway `TRIM_MAXLEN` in a safe window.
- Continue watching known benign-but-noisy `orphan_trade` warnings caused by broker event ordering.
