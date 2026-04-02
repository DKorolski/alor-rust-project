# Restart From Zero Indicator Warmup Validation

Date: 2026-04-02

## Goal

Validate that the runtime patch for:

- `sessiongap` history warmup reconstruction,
- `hybrid` signal-state persistence / restore,
- `hybrid` MR `close_prev = previous day close`,

works correctly after a full `restart from zero` rollout.

Operationally, `restart from zero` means:

1. deploy new runtime image,
2. stop both stacks,
3. clear Redis data directories,
4. recreate `redis`, `alor-gateway`, and `strategy-runtime`,
5. let gateway cold-backfill recent bars again.

## Runtime Build

Code commit:

- `0b605ac` `Fix strategy indicator warmup and persistence`

Runtime image:

- `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-0b605ac-indwarm-20260402-213111`

Gateway image remained:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-71c09ac-actionscope2-20260402-131941`

## Rollout Performed

Stacks:

- `sessiongap` at `/opt/trading-sessiongap`
- `trading-hybrid` at `/opt/trading-hybrid`

Executed steps:

1. backed up both `.env` files,
2. switched `RUNTIME_IMAGE_TAG` in both stacks to `dev-0b605ac-indwarm-20260402-213111`,
3. ran `docker compose down` for both stacks,
4. removed:
   - `/opt/trading-sessiongap/volumes/redis/*`
   - `/opt/trading-hybrid/volumes/redis/*`
5. ran `docker compose up -d --force-recreate` for both stacks.

## Gateway Result

Both gateways completed a cold history sync and reached `LiveReady`.

Observed in logs:

- `sessiongap` gateway:
  - `history backfill complete; live stream started ... history_count=3300`
  - `gateway phase transition current=SyncingHistory next=LiveReady`
- `hybrid` gateway:
  - `history backfill complete; live stream started ... history_count=3291`
  - `gateway phase transition current=SyncingHistory next=LiveReady`

## Runtime Result

After first live bars arrived, both runtimes reached:

- `runtime_phase="LiveReady"`
- `live_guard="ALLOWED"`
- `readiness=true`

This is important because immediately after cold boot they were temporarily blocked only by bootstrap gates such as:

- `bootstrap:not_ready`
- `bootstrap:missing_live_bar`

That temporary state cleared on fresh live-bar arrival without manual intervention.

## Sessiongap Validation

Persisted runtime state after cold restart contained fully reconstructed signal fields:

- `prev_close = 81.32`
- `yesterday_range = 0.8800000000000097`
- `pre_prev_close = 81.22`
- `first_min_high = 81.4`
- `first_min_low = 81.4`
- `first_hour_price = 81.27`
- `phase = "Flat"`

Observed runtime evidence:

- `session gap history warmup applied`
- later `signal warmup complete`

Important interpretation:

- `sessiongap` no longer stayed stuck with `prev_close=null`, `yesterday_range=null`, `pre_prev_close=null`
- the earlier warmup failure was successfully removed in the clean-start contour

## Hybrid Validation

Persisted runtime state after cold restart contained warmed signal fields:

- `entry_ready = true`
- `prev_day_close = 2776.5`
- `prev_day_range = 38.0`
- `prev_day_return = -0.010160427807486631`
- `current_day_high = 2790.5`
- `current_day_low = 2769.0`
- `current_day_close = 2781.0`
- `today_start_local = "2026-04-01T08:59:00"`
- `was_long_today = false`
- `was_short_today = false`

Observed runtime evidence:

- `hybrid history warmup applied ... processed=911`
- `signal warmup complete ... entry_ready=true`

Important interpretation:

- `hybrid` no longer stayed at `prev_day_range=null`
- the runtime now persists enough signal context to reconstruct the live signal baseline after cold restart
- MR is now aligned with the research contour because `close_prev` is restored as previous-day close rather than previous intraday close

## Residual Observation

`sessiongap` startup still saw historical broker artifacts during bootstrap:

- old order/trade events were replayed from broker streams
- one runtime log line showed `orphan_trade`

Current reading:

- this did not prevent warmup completion,
- this did not block `ALLOWED`,
- but it remains worth monitoring during soak as restart/replay noise.

## Conclusion

The patch is validated in the intended operational contour.

What is now confirmed:

1. clean Redis restart no longer leaves `sessiongap` with missing prior-session indicators,
2. clean Redis restart no longer leaves `hybrid` with missing previous-day signal context,
3. both runtimes recover from cold bootstrap to `LiveReady / ALLOWED`,
4. the systems are suitable for continued live soak on the new runtime baseline.

## Soak Decision

Decision:

- keep both stacks on this runtime image,
- continue soak through tomorrow and the following week,
- monitor:
  - runtime readiness,
  - signal warmup logs,
  - any `trading_window_closed` defer/reissue sequences,
  - any replay/orphan warnings during subsequent restarts.
