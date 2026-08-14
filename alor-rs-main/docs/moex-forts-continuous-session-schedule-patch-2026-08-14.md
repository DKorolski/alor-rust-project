# MOEX FORTS continuous-session schedule patch, 2026-08-14

## Summary

MOEX currently publishes the FORTS trading day as three continuous weekday
blocks:

- Morning additional session: 07:00-10:00 MSK.
- Main session: 10:00-19:00 MSK.
- Evening additional session: 19:00-23:50 MSK.

Source: https://www.moex.com/torgovye-sessii-na-srochnom-rynke

The published page no longer lists the old intraday breaks around 14:00 and
18:50. Keeping those breaks in runtime/gateway admission creates an artificial
local closed-window state and can produce false `trading_window_closed` rejects
or suppress valid live intents.

## Patch scope

The patch updates active live07/canonical07 contours and their paired
shadow07/shadow09 diagnostics to use continuous weekday admission:

- `session_start = "07:00:00"` for active canonical07/live07 runtime/gateway
  configs.
- `session_end = "23:49:00"` or `23:49:59`, preserving the existing contour
  convention.
- `break_start_1/break_end_1/break_start_2/break_end_2 = "00:00:00"`.

The `TradingPeriods` schema currently requires break fields, so the patch keeps
the fields present but moves them outside the live session. With the current
scheduler this avoids `Break1`/`Break2` during normal weekday trading.

## Weekend policy

This patch does not enable weekend trading for live strategies.

MOEX also describes an additional weekend session, but live weekend behavior is
a separate strategy decision. Existing `weekends_off` settings are preserved for
runtime configs. Any future weekend-trading decision must be handled by a
separate research and rollout line.

## Affected repo configs

Active live/canonical configs:

- `configs/gateway.ri_author41_42.micro.7502MIW.RIU6.roll-2026-06-12.toml`
- `configs/runtime.ri_author41_42.micro.7502MIW.RIU6.live07.toml`
- `configs/gateway.alor_usdrubf.live.7502MIW.toml`
- `configs/runtime.alor_usdrubf.live.7502MIW.canonical07.challenger_mr035.toml`
- `configs/gateway.hybrid.live.7502MIW.action-scoped.canonical07.toml`
- `configs/runtime.hybrid.live.7502MIW.riskgate-canonical07.toml`

Shadow diagnostics:

- `configs/runtime.ri_author41_42.shadow07.7502MIW.toml`
- `configs/runtime.ri_author41_42.shadow09.7502MIW.toml`
- `configs/runtime.alor_usdrubf.shadow07.7502MIW.toml`
- `configs/runtime.alor_usdrubf.shadow09.7502MIW.toml`
- `configs/runtime.hybrid_imoexf.shadow07.7502MIW.toml`
- `configs/runtime.hybrid_imoexf.shadow09.7502MIW.toml`

## Rollout plan

Do not deploy while any live strategy has an open broker position or working
orders.

Safe rollout sequence:

1. Confirm broker-flat for `RTS-9.26`, `USDRUBF`, and `IMOEXF`.
2. Confirm no working orders and no stop orders.
3. Copy updated configs to the VPS.
4. Recreate affected gateway/runtime containers.
5. Check health streams for `scheduler_state = "Open"` during former break
   windows.
6. Watch for `trading_window_closed`, stale pending intents, and broker-flat
   reconciliation through the next session.

Preferred rollout window: after all systems are flat today, or before the next
session opens.

## Acceptance checks

- No local `Break1`/`Break2` during 14:00-14:05 and 18:50-19:05 MSK on a normal
  weekday session.
- Action-scoped market path remains active for RI and Alor-USDRUBF.
- Hybrid IMOEXF keeps existing BO-only/live riskgate behavior; this patch does
  not re-enable MR.
- Shadow07/shadow09 remain order-emission disabled.
- No change to strategy K/TP/SL/quantity parameters.
