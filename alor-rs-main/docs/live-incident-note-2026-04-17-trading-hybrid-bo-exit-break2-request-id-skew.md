# Live Incident Note: `trading-hybrid` — BO exit at `Break2`, request-id skew, stale `pending_exit_active`

Date: 2026-04-18

Incident date:

- 2026-04-17

Stack / container:

- `trading-hybrid-strategy-runtime-1`
- `trading-hybrid-alor-gateway-1`

Symbol / portfolio:

- `IMOEXF`
- `7502SN6`

## 1. Executive summary

On 2026-04-17, `trading-hybrid` opened an `IntradayBreakout` short and later generated a valid `BreakoutStop1Short` exit exactly at the start of the evening trading break:

- `dt_local = 2026-04-17 18:50:00`
- scheduler state on gateway = `Break2`

The gateway correctly rejected the exit with:

- `error_code = trading_window_closed`

This reject should have been treated as a recoverable exit-window event:

- clear live `pending_exit`
- move exit into deferred state
- reissue exit after the break

Instead, the strategy remained stuck in:

- `pending_exit_active`

for the rest of the session, suppressing:

- repeated `BreakoutStop1Short` exit attempts
- the later `BreakoutEodExit` at `23:30 MSK`

The root cause is not the break-window reject itself.

The root cause is a request-id mismatch between:

- the `pending_exit_request_id` stored inside `hybrid_intraday_runtime`
- and the actual `OrderCommand.request_id` emitted by `strategy-runtime`

Because of that skew, the reject ack did not match the strategy-owned pending exit record, so the runtime never entered the intended:

- `exit_deferred_trading_window_closed`

path.

## 2. What happened

### 2.1 Signal and first exit attempt at break start

At:

- `2026-04-17T15:51:03.164947Z`
- local time `2026-04-17 18:50:00 MSK`

the strategy generated:

- `submit_exit owner=IntradayBreakout reason=BreakoutStop1Short`

This is logically correct.

With:

- `close_prev = 2740.0`
- `day_range_prev = 30.0`
- `bo_stop1_range = 0.51`

the short `stop1` level is:

- `2740.0 - 0.51 * 30.0 = 2724.7`

Observed close:

- `2726.5`

So:

- `2726.5 > 2724.7`

and `BreakoutStop1Short` should fire.

Relevant code:

- [intraday_breakout.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/strategies/hybrid_intraday/intraday_breakout.rs#L218)

### 2.2 Gateway reject at `Break2`

The gateway received:

- `request_id = 42cea307-31eb-5489-8e4b-9e3944c1471d`
- `action = place`

and rejected it with:

- `trading_window_closed`
- `scheduler_state = Some(Break2)`

Relevant code:

- [command_consumer.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/alor-gateway/src/services/command_consumer.rs#L885)

This reject is expected and correct for a break-window boundary.

## 3. Root cause

### 3.1 Strategy-side pending request id

Inside `hybrid_intraday_runtime`, exit pending state is set from:

- `self.live_request_id(ctx, created_ts_utc, side)`

Relevant code:

- [hybrid_intraday_runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/strategies/hybrid_intraday_runtime.rs#L1223)
- [hybrid_intraday_runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/strategies/hybrid_intraday_runtime.rs#L356)

For:

- `strategy_id = hybrid_intraday`
- `portfolio = 7502SN6`
- `symbol = IMOEXF`
- `action = place`
- `created_ts_utc = 1776441000`
- `seq = 0`

the deterministic request id is:

- `e33f24f1-53e9-54b2-a4f6-cc15d6189b96`

That is exactly what later appears in:

- `pending_exit_active`

logs and runtime state.

### 3.2 Runtime-emitted request id

But `strategy-runtime` does not always emit commands using the raw bar timestamp.

It passes intents through:

- `normalize_event_ts(...)`

and then builds `OrderCommand.request_id` from the normalized `created_ts_utc`.

Relevant code:

- [runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/runtime.rs#L1837)
- [runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/runtime.rs#L3101)
- [runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/runtime.rs#L3297)
- [runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/runtime.rs#L3499)

In this incident, the actually emitted request id was:

- `42cea307-31eb-5489-8e4b-9e3944c1471d`

That id corresponds to:

- `place`
- `created_ts_utc = 1776441055`

So strategy-owned pending state and emitted command request id diverged.

### 3.3 Why deferred exit did not happen

The recoverable exit-window path in `hybrid_intraday_runtime` is keyed by exact request-id match:

- `if Some(ack.request_id) == self.pending_exit_request_id { ... }`

Relevant code:

- [hybrid_intraday_runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/strategies/hybrid_intraday_runtime.rs#L3415)

Because:

- pending exit id = `e33f24f1-53e9-54b2-a4f6-cc15d6189b96`
- gateway reject ack id = `42cea307-31eb-5489-8e4b-9e3944c1471d`

the match never happened.

So the runtime did not:

- clear `pending_exit_request_id`
- create `deferred_exit`
- log `exit_deferred_trading_window_closed`
- reissue after the break

## 4. Operational consequence

After the `18:50` reject, the system remained in a false live-order state:

- `pending_exit_request_id.is_some()`

and `hybrid` treats that as:

- `has_live_orders = true`

Relevant code:

- [hybrid_intraday_runtime.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/strategies/hybrid_intraday_runtime.rs#L229)

Therefore all later exits were suppressed as:

- `exit_suppressed`
- `reason = pending_exit_active`

Affected later points:

- `19:50 MSK` — `BreakoutStop1Short`
- `20:50 MSK` — `BreakoutStop1Short`
- `21:50 MSK` — `BreakoutStop1Short`
- `22:50 MSK` — `BreakoutStop1Short`
- `23:30 MSK` — `BreakoutEodExit`

## 5. Important timing note

For `hybrid` breakout, `BreakoutEodExit` currently triggers at:

- `23:30 MSK`

This is currently hardcoded in the breakout engine, not derived from session close:

- [intraday_breakout.rs](/Users/denisq/Documents/from_mac/projects/strategies_list/alor_project/bybit_barter_test/alor-rs-main/strategy-runtime/src/strategies/hybrid_intraday/intraday_breakout.rs#L192)

So the `23:30` line in logs is expected under the current strategy behavior.

## 6. Classification

This incident is best classified as:

- `shared request-id skew / stale pending state class`

not as:

- pure trading-window policy issue
- pure gateway issue
- pure breakout signal issue

The same class may affect other strategy-owned deferred / pending workflows if they:

- precompute request ids inside strategy state
- while `strategy-runtime` later emits commands with a different normalized timestamp

## 7. Resolution status

Status:

- incident analyzed
- fix line agreed
- implementation not yet applied

## 8. Agreed direction

Agreed decisions for the fix line:

1. keep `BreakoutEodExit` at the current effective time:
   - `23:30 MSK`
2. for window-closed exits, prefer deferring in `runtime` before emit instead of relying on gateway reject + ack reconciliation
3. treat this as a shared bug class, not a one-off `hybrid` quirk
4. plan rollout as a controlled rebuild / restart decision, with explicit consideration of whether a `from zero` restart is required for clean validation
