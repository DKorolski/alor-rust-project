# Market Buy And Close Diagnostic Runbook

Date: 2026-03-17

## 1. Purpose

`market_buy_and_close` now supports two live execution styles:

- `live_order_style = "market"`
- `live_order_style = "marketable_limit"`

This gives a controlled comparison path for:

- baseline `create:market`
- controlled `create:limit` via marketable live limits
- `session_gap_standalone` native `create:limit`

The goal is to separate:

- residual `create:limit` / CWS transport issues
- from `session_gap`-specific timing, state-machine, or recovery behavior.

## 2. Config Fields

Add these fields under `[strategy]` for `strategy_kind = "market_buy_and_close"`.

```toml
[strategy]
strategy_id = "market_buy_and_close"
strategy_kind = "market_buy_and_close"
symbol = "USDRUBF"
qty = 1.0
side = "buy"
close_trigger = "next_bar"

live_order_style = "market"
marketable_limit_offset_ticks = 0
```

Semantics:

- `live_order_style = "market"`
  - preserves old behavior
  - live entry/flatten use `Intent::Market`
- `live_order_style = "marketable_limit"`
  - live entry/flatten use `Intent::Place`
  - runtime computes a marketable price from the current live bar and `tick_size`
- `marketable_limit_offset_ticks`
  - extra aggressiveness in ticks before gateway normalization
  - runtime still keeps one extra tick internally so a normalized limit does not become passive

Environment overrides are also supported:

```bash
LIVE_ORDER_STYLE=marketable_limit
MARKETABLE_LIMIT_OFFSET_TICKS=0
```

## 3. Marketable Limit Pricing

For `live_order_style = "marketable_limit"`:

- `buy`: `reference_price + (offset_ticks + 1) * tick_size`
- `sell`: `reference_price - (offset_ticks + 1) * tick_size`

Reference price:

- entry / next-bar flatten: current live bar close
- position-update flatten: last seen live bar close, with broker average price as fallback

## 4. Structured Logs

When the strategy emits a live command, logs include:

- `strategy = "market_buy_and_close"`
- `live_order_style`
- `request_id`
- `side`
- `qty`
- `reason = entry | flatten`
- `price` for `marketable_limit`

This is intended for apples-to-apples comparison with `session_gap` gateway/runtime logs.

## 5. Test Group A: Baseline Market Path

Use:

```toml
[strategy]
strategy_kind = "market_buy_and_close"
live_order_style = "market"
```

Expected outcome:

- entry accepted
- fill observed
- flatten accepted
- fill observed
- runtime returns to `Flat/Done`
- no `Blocked`
- no orphan state

## 6. Test Group B: Controlled Marketable Limit Path

Use:

```toml
[strategy]
strategy_kind = "market_buy_and_close"
live_order_style = "marketable_limit"
marketable_limit_offset_ticks = 0
```

Recommended progression:

1. B1: 1-2 controlled cycles in calm conditions
2. B2: 3-5 full entry -> fill -> flatten -> Flat cycles

For each cycle capture:

- runtime logs
- gateway logs
- `cmd.orders.*`
- `cmd.acks.*`
- `broker.orders.*`
- `broker.positions.*`
- `runtime.state.*`

Acceptance per cycle:

- `accepted` ack
- `broker_order_id`
- `cws_request_guid`
- `working -> filled`
- position opens
- position closes
- state returns cleanly
- no `Blocked`
- no `safe_mode`

## 7. Test Group C: Compare With Session Gap

Compare `market_buy_and_close` in `marketable_limit` mode against `session_gap_standalone`.

Focus comparison on:

- request shape
- `create:limit` send/ack timing
- transport resets around send
- `cws_transport_failure`
- `cws_fail_pending`
- runtime transition into `Blocked`
- reconnect proximity

## 8. Interpretation Guide

Scenario 1:

- A stable
- B stable
- `session_gap` still occasionally fails

Interpretation:

- issue is more likely in `session_gap` timing, state machine, or recovery semantics

Scenario 2:

- A stable
- B also shows residual resets

Interpretation:

- issue is more likely around `create:limit`, CWS transport, or broker handling of live limit requests

Scenario 3:

- A stable
- B stable
- `session_gap` also stable over repeated runs

Interpretation:

- residual incident is likely intermittent and needs longer-haul soak plus topology/session diagnostics

## 9. Minimal Live Checklist

Before each run:

1. confirm gateway `readiness=true`
2. confirm runtime is not already `Blocked`
3. confirm no residual open position from prior cycle

During run:

1. watch runtime `intent_emitted`
2. watch gateway `command received`, `cws_limit_send` or market path equivalent, `command ack published`
3. watch `broker.orders` and `broker.positions`

After run:

1. confirm final position is `0`
2. confirm runtime state is clean
3. classify any failure as:
   - market-path only
   - marketable-limit-path only
   - shared transport symptom

## 10. Current Conclusion

This diagnostic mode is intended to answer one narrow question:

Can a minimal live strategy using the same `Intent::Place -> create:limit` path as `session_gap` reproduce the same residual transport behavior, or does the problem remain isolated to `session_gap` timing/recovery context?
