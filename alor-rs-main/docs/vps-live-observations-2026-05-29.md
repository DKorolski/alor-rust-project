# VPS Live Observations - 2026-05-29

## Pre-Open Health Check

Checkpoint:

- Time checked: `2026-05-29 08:47 MSK`.
- VPS: `155.212.170.21`.
- Context: corporate server is preparing deployment for `7502T0U`; temporary VPS `RI Author41/42` on `7502T0U` was stopped on `2026-05-28`.

Active containers:

- `trading-ri-author41-42-7502miw-*`: healthy.
- `trading-alor-usdrubf-*`: healthy.
- `trading-hybrid-*`: healthy.
- `trading-hybrid-author41-7502t0u-*`: healthy.
- `trading-ri-author41-42-7502t0u-*`: not running, expected after handoff preparation.

Resource snapshot:

- RAM: `7.7 GiB` total, about `5.9 GiB` available.
- Swap: `3.9 GiB` total, about `44 MiB` used.
- Disk `/`: `79G` total, `36G` used, `40G` available, `48%`.
- Redis memory:
  - `trading-ri-author41-42-7502miw-redis-1`: about `160M / 512M`.
  - `trading-alor-usdrubf-redis-1`: about `145M`.
  - `trading-hybrid-redis-1`: about `253M`.
  - `trading-hybrid-author41-7502t0u-redis-1`: about `152M / 512M`.

Log read:

- Overnight/morning CWS and WS reconnects were observed across gateways.
- Reconnects had no command in flight:
  - `opcode_in_flight=None`.
  - `request_id=None`.
  - `pending_count=0`.
- No runtime `command rejected` or panic/error path was observed in the checked window.
- Runtime guards moved into normal overnight/pre-open blocked states:
  - `phase=SyncingGap`.
  - `phase=SyncingHistory`.
  - temporary `gateway_ready=false` / `ws_connected=false`.

Broker-state read:

- Latest broker position snapshots show flat for the traded instruments:
  - `RTS-6.26 qty=0`.
  - `USDRUBF qty=0`.
  - `IMOEXF qty=0`.
- No active stop-order tail was visible in the checked Redis streams.
- Latest order/trade rows in the broker streams are yesterday's completed events.

Interpretation:

- Systems are healthy before the new session.
- The `7502T0U` temporary RI contour is correctly stopped for corporate deployment preparation.
- The remaining `7502T0U` VPS contour is only the trial `IMOEXF hybrid author41-short`; this matches the current decision to leave it running while corporate deploys `RI + Alor-USDRUBF`.
- Redis broker streams are portfolio-level, so cross-symbol rows may appear in each stack's Redis. This is expected and does not by itself mean that the wrong strategy emitted those orders.

Watchlist:

- Continue monitoring `IMOEXF hybrid` near-zero MR bracket churn after the move to quantity `2` on `7502MIW`.
- Continue monitoring `IMOEXF hybrid` partial-fill behavior at higher quantity. Current status: observed as operationally manageable, but still important before any move toward `IMOEXF 10`.
- After corporate `7502T0U` deployment starts, verify there is no second VPS order-emitting RI contour on `7502T0U`.
- Re-check after the first regular live bars that all live guards return to `ALLOWED` where expected.

## Scale-Up Watchlist / Engineering Ideas

Extended micro / small-readiness watchlist:

- `IMOEXF hybrid`:
  - Watch for repeated `near_zero_mr_bracket_churn`, especially where rounded TP is too close to the expected/actual MR entry price.
  - Watch for partial fills after the move from `1` to `2` contracts and before any larger quantity.
  - Potential guard before scale-up: suppress MR bracket entry when rounded TP distance from expected entry is below a minimum threshold, for example `1-2` ticks.
- `RI Author41/42` and `Alor-USDRUBF`:
  - Research idea: evaluate whether MR exits can be moved from current closed-bar/marketable exit semantics toward a limit/bracket exit style similar to `IMOEXF hybrid`.
  - Motivation: potentially reduce commissions/slippage and avoid unnecessary marketable exits.
  - Constraint: this is an execution-contract change, not just a config tweak. It must be tested against parity/economics before live use.
  - For `RI`, current accepted live micro contract remains closed-bar condition / marketable exit for parity with the validated model.
  - For `Alor-USDRUBF`, any bracket/limit MR exit change should preserve the already validated action-scoped CWS path and broker-truth convergence behavior.

Decision:

- Keep these items in watchlist/engineering backlog during extended micro.
- Do not block current live observation on them.
- Revisit before deciding on `small` portfolio sizing, especially if the target package is around `RI 2 / USDRUBF 2 / IMOEXF 10`.
