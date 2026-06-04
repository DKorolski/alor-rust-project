# VPS Live Observations - 2026-05-28

## Temporary RI Author41/42 Micro On 7502T0U

Context:

- Decision: temporarily raise a second `RI author41/42 micro` contour on portfolio `7502T0U`.
- Purpose: allow a temporary mirror/parallel RI live micro contour while the main `7502MIW` one-portfolio soak continues.
- The stack was created as a separate VPS contour, not by reusing the existing `7502MIW` Redis volume.

Deployment:

- VPS path: `/opt/trading-ri-author41-42-7502t0u`.
- Source contour: `/opt/trading-ri-author41-42-7502miw`.
- New Redis volume: `/opt/trading-ri-author41-42-7502t0u/volumes/redis`.
- Gateway config: `/configs/gateway.ri_author41_42.micro.7502T0U.toml`.
- Runtime config: `/configs/runtime.ri_author41_42.micro.7502T0U.toml`.
- Portfolio: `7502T0U`.
- Symbol feed: `RIM6`.
- Order symbol: `RTS-6.26`.
- Quantity: `1`.
- Runtime mode: `micro_live`.
- Execution path: `action_scoped_only`.

Streams:

- Bars: `md.bars.7502T0U.RIM6.10m`.
- Commands: `cmd.orders.7502T0U.ri_author41_42.micro`.
- Acks: `cmd.acks.7502T0U.ri_author41_42.micro`.
- Runtime state: `runtime.state.ri_author41_42.micro.7502T0U`.
- Health: `events.health.ri_author41_42.7502T0U`.

Pre-runtime broker snapshot:

- Gateway was started before runtime.
- Broker snapshot for `7502T0U` showed no positions.
- Broker snapshot showed no working orders.
- Broker snapshot showed no stop orders.

Startup result:

- `trading-ri-author41-42-7502t0u-redis-1`: healthy.
- `trading-ri-author41-42-7502t0u-alor-gateway-1`: healthy.
- `trading-ri-author41-42-7502t0u-strategy-runtime-1`: healthy.
- Runtime bootstrap reconciled flat:
  - `positions_open_strategy=0`.
  - `orders_open_strategy=0`.
  - `stop_orders_open_strategy=0`.
  - `ri_bootstrap_reconciled_flat`.
- Warmup processed `454` historical bars.
- No command was emitted at startup.
- `cmd.orders.7502T0U.ri_author41_42.micro` was empty at the checkpoint.
- `cmd.acks.7502T0U.ri_author41_42.micro` was empty at the checkpoint.

Current state at pre-open checkpoint:

- Runtime is healthy but live guard is still `BLOCKED`.
- Reasons are expected for pre-open startup:
  - `bootstrap:missing_live_bar`.
  - `bootstrap:not_ready`.
  - `gateway_ready=false`.
  - `phase=SyncingHistory`.
- Interpretation: contour is up, isolated, flat, and waiting for the first live bar before becoming live-ready.

Resource snapshot after adding the stack:

- `trading-ri-author41-42-7502t0u-redis-1`: about `3.9 MiB / 768 MiB`.
- `trading-ri-author41-42-7502t0u-alor-gateway-1`: about `3.6 MiB / 768 MiB`.
- `trading-ri-author41-42-7502t0u-strategy-runtime-1`: about `2.7 MiB / 768 MiB`.
- Existing stacks remained healthy.

Watchlist:

- Confirm transition to `LiveReady / ALLOWED` after the first regular live bar.
- Confirm no startup replay/historical decision emits a live order.
- If a live order is emitted on `7502T0U`, verify it stays on action-scoped path and receives broker `Accepted`.
- Monitor shared exposure: this temporary contour can duplicate RI exposure if the main `7502MIW` RI contour is also active.

## Hybrid IMOEXF Author41-Short Micro On 7502T0U

Context:

- Decision: deploy a trial `hybrid_intraday` IMOEXF contour on portfolio `7502T0U`.
- Goal: keep the validated hybrid BO and operational contour, but replace only the MR sleeve with `author41_boundary_short`.
- Implementation commit: `09c4739 Add IMOEXF hybrid author41 short variant`.
- Runtime variant: `mr_variant = "author41_boundary_short"`.
- BO parameters were kept aligned with the current live IMOEXF hybrid baseline:
  - `bo_k = 0.53`.
  - `bo_wait_hours = 3.0`.
  - `bo_stop1_range = 0.51`.
  - `bo_stop2_range = 0.35`.
- Quantity: `1`.

Deployment:

- VPS path: `/opt/trading-hybrid-author41-7502t0u`.
- Runtime image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-09c4739-hybrid-author41-20260528`.
- Gateway image: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-5430299-protplace-20260428`.
- Gateway config: `/configs/gateway.hybrid.live.7502T0U.action-scoped.toml`.
- Runtime config: `/configs/runtime.hybrid.live.7502T0U.author41-short.toml`.
- Execution contour: action-scoped CWS.

Streams:

- Bars: `md.bars.7502T0U.10m`.
- Commands: `cmd.orders.7502T0U.hybrid_author41_short`.
- Acks: `cmd.acks.7502T0U.hybrid_author41_short`.
- Runtime state: `runtime.state.hybrid_intraday.live.author41_short.imoexf.7502T0U`.
- Health: `events.health.hybrid_author41_short.7502T0U`.

Startup result:

- `trading-hybrid-author41-7502t0u-redis-1`: healthy.
- `trading-hybrid-author41-7502t0u-alor-gateway-1`: healthy.
- `trading-hybrid-author41-7502t0u-strategy-runtime-1`: healthy.
- Runtime bootstrap snapshot was clean for the target contour:
  - `positions_open_all=0`.
  - `orders_open_all=0`.
  - `stop_orders_open_all=0`.
- Gateway observed existing filled RI orders on the same portfolio, but no open IMOEXF position/order/stop-order tail was present.
- Startup replay guard armed with `tolerance_sec=60`.
- First live checkpoint:
  - `09:40` local bar was suppressed by startup replay guard.
  - Runtime transitioned to `LiveReady / ALLOWED` at `2026-05-28 09:50:08 MSK`.
  - No `intent_emitted` event was produced during startup.

Resource snapshot:

- New Redis memory at startup: about `1.42 MiB / 512 MiB`.
- VPS still had healthy headroom after stack bring-up.

Watchlist:

- Verify first author41-short MR entry, if generated, emits as `MeanReversion` bracket entry.
- Verify protective TP/SL installation remains on the action-scoped path.
- Verify BO behavior remains unchanged versus the existing IMOEXF hybrid baseline.
- Monitor combined exposure on `7502T0U`, because RI, alor-USDRUBF, and this IMOEXF trial contour can now coexist on the same portfolio.

## Watchlist - IMOEXF Hybrid Near-Zero MR Bracket Churn On 7502MIW

Context:

- Portfolio: `7502MIW`.
- Stack: existing `trading-hybrid` IMOEXF hybrid contour, not the new `7502T0U` author41-short trial.
- Quantity: `2`.
- Time window checked: around `2026-05-28 11:05-11:10 MSK`.
- Broker ledger showed:
  - `11:05` sell `2` IMOEXF at `2596`, closing the prior long MR cycle.
  - `11:10` sell `2` IMOEXF at `2593`, opening a new short MR cycle.
  - `11:10` buy `2` IMOEXF at `2593`, closing that short MR cycle almost immediately.

Runtime/gateway read:

- The `11:10` sequence was generated by the existing hybrid MR sleeve:
  - `submit_entry owner=MeanReversion side=Short style=Bracket reason=MorningMeanReversionShort`.
  - Entry command was accepted and filled via the action-scoped CWS path.
  - Protective TP and SL were installed via the action-scoped path.
  - The TP buy limit filled almost immediately at the same effective broker price as the short entry.
  - The residual SL stop order was cancelled via action-scoped `delete_stop_limit`.
- No evidence of legacy long-lived CWS fallback was observed for this episode.
- No uncontrolled IMOEXF position tail was observed after the sequence.

Interpretation:

- This is not currently classified as a transport or lifecycle incident.
- The working classification is `near_zero_mr_bracket_churn`.
- The likely mechanism is that after tick rounding and actual fill price, the short MR take-profit became too close to the expected/actual entry level.
- With quantity `2`, the behavior is more visible operationally because a near-flat churn produces two broker trades and consumes commissions/slippage budget without meaningful model edge.

Watchlist / possible patch:

- Monitor whether similar MR cycles repeat, especially after the move from `1` to `2` contracts.
- If repeated, add a hybrid MR bracket sanity guard before emitting the entry:
  - suppress MR bracket entry when rounded TP distance from expected entry is below a minimum threshold, for example `1-2` ticks;
  - log an explicit event such as `mr_entry_suppressed reason=take_too_close_after_rounding`;
  - keep BO behavior and the existing action-scoped execution contour unchanged.
- Keep this as an engineering watchlist item until at least several more clean sessions confirm whether it is isolated or systematic.
