# VPS Live Observations - 2026-04-17

Date: 2026-04-17

Scope:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

Context:

- Patch A gateway hotfix was rolled out as a controlled live validation.
- Rollout scope was intentionally narrow:
  - `trading-hybrid` gateway only
  - `trading-alor-usdrubf` gateway only
  - runtime containers were not changed
- New gateway image tag on VPS:
  - `ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-5430299`

## Executive Summary

The most important result of the day is that `trading-alor-usdrubf` produced an immediately useful post-rollout signal:

- first observed post-rollout `create:market` entry went through `action_scope_cws`,
- token refresh before authorize happened,
- broker accepted the order,
- execution confirmed,
- and the strategy opened a short position cleanly.

That is exactly the hot path the patch was meant to improve.

`trading-hybrid` did not yet provide a clean post-rollout protective validation case. The most visible `TP/SL` failures in the log slice happened before the gateway recreate window and belong to the `MeanReversion` component. The current runtime state shows an open short owned by `intraday_breakout`; for that owner, absence of broker-side `tp/sl` ids is not itself anomalous. The main live watchpoint remains the next fresh `MeanReversion` protective install after the patch.

`trading-sessiongap` remained the quiet baseline through the day; no transport anomaly was visible in the pulled window and the strategy stayed flat by session end.

## Stack Snapshot

At observation time:

- `trading-hybrid-alor-gateway-1` -> `alor-gateway:manual-5430299`
- `trading-alor-usdrubf-alor-gateway-1` -> `alor-gateway:manual-5430299`
- `trading-hybrid-strategy-runtime-1` -> unchanged previous runtime image
- `trading-alor-usdrubf-strategy-runtime-1` -> unchanged previous runtime image

Both patched stacks returned to:

- `readiness=true`
- `runtime_phase="LiveReady"`
- `live_guard="ALLOWED"`

after the short expected `BLOCKED -> ALLOWED` transition during gateway restart / resync.

## 1. `trading-sessiongap`

Observed behavior in the latest operational slice:

- previous session end exit was clean:
  - `InPosition -> PendingExit -> Flat`
  - `intent_emitted`
  - `execution confirmed`
  - no transport reject on the core one-shot path
- overnight / morning path showed only normal guard transitions:
  - `ALLOWED -> BLOCKED -> ALLOWED`
  - caused by websocket / history sync state

Gateway evidence remains consistent with the clean baseline:

- action-scoped session open
- forced token refresh before authorize
- `create:limit`
- accepted ack
- clean close

Reading:

- no new anomaly in the observed slice
- still the cleanest operational baseline
- day-end reading remains clean and flat

## 2. `trading-hybrid`

### Intraday behavior before the gateway rollout

The strategy again reproduced the known protective asymmetry:

- **06:01 UTC**
  - entry `place` intent emitted
  - entry execution confirmed
  - then protective intents emitted:
    - `place`
    - `create_stop_limit`
  - both protective requests were rejected:
    - one with `cws disconnected: protocol_reset_without_close_handshake`
    - one with `cws response timeout`
  - runtime entered temporary `BLOCKED`
  - later returned to `ALLOWED`

- **07:40 UTC**
  - another entry / protection cycle repeated the same pattern:
    - entry execution confirmed
    - follow-up protective legs rejected
    - same transport-failure class

- **08:00 UTC**
  - `orphan_trade` appeared again for the historical stop-order path

### Current state at observation time

Latest runtime state shows:

- `last_position_qty = -1.0`
- `current_owner = "intraday_breakout"`
- `current_side = "short"`
- `pending_* = null`
- `tp_order_id = null`
- `sl_stop_order_id = null`
- `sl_exchange_order_id = 2033126153719430281`

This means:

- the stack is not flat now,
- the currently visible state belongs to `intraday_breakout`,
- absence of active broker-side TP / SL ids is expected for that owner,
- and the old stop-related residue is still present in state.

### Post-rollout reading

The gateway hotfix was applied successfully and the stack returned to `ALLOWED`, but the pulled slice does not yet prove that the patched protective path has now behaved cleanly on a fresh live setup event.

Important ownership note:

- broker-side `TP/SL` installation in `hybrid` applies only to the `MeanReversion` component
- `IntradayBreakout` uses ordinary entry / exit flow without broker-side `tp/sl` protective installs
- therefore, the current open `intraday_breakout` short without `tp_order_id` / `sl_stop_order_id` is not itself an anomaly
- the unresolved question is whether the next `MeanReversion` position will install protection cleanly via the patched path

Current status:

- patch applied: yes
- clean protective validation after patch: not yet proven
- live watchpoint remains active

### End-of-day reading

By the end of the observed session:

- the stack still carried an open `intraday_breakout` short
- there was no evidence of a new post-patch `MeanReversion` protective install succeeding cleanly
- a late `place` intent was rejected with `trading_window_closed`, which is consistent with the session boundary rather than a transport defect

So the day was operationally mixed:

- ordinary entry / exit flow still worked
- but the specific patched validation target for `MeanReversion` protection remained unresolved

## 3. `trading-alor-usdrubf`

### Pre-rollout behavior earlier in the day

Before the gateway recreate window, the strategy still showed the old familiar exit-path noise:

- repeated market exit intents
- `command rejected`
- `error_code = cws_error`
- `error_msg = cws disconnected: protocol_reset_without_close_handshake`
- defer to next bar
- temporary `BLOCKED -> ALLOWED` churn

This is consistent with the pre-patch soak pattern.

### First useful post-rollout signal

After rollout, the first observed fresh entry sequence was materially better:

- **14:06:05 UTC**
  - strategy emitted a short market entry intent
  - gateway logs show:
    - `action_scope_session_open_start`
    - `action_scope_session_open_success`
    - token invalidation / forced refresh
    - `action_scope_authorize_ok`
    - `action_scope_send_start ... opcode=\"create:market\"`
    - `action_scope_send_result ... http_code=Some(200)`
    - clean action-scope session close
  - gateway published `Accepted` ack
  - runtime logged:
    - `command acknowledged outcome="accepted"`
    - `execution confirmed`
    - `position_transition = flat_to_open`

This was the first strong positive post-rollout signal.

### Post-rollout reading

For `alor-usdrubf`, the patch produced two concrete live success cases on the exact target path:

- post-rollout entry:
  - `create:market`
  - action-scoped
  - forced token refresh
  - accepted
  - executed
- post-rollout exit:
  - `create:market`
  - action-scoped
  - forced token refresh
  - accepted
  - executed

This means the patched contour already showed a full clean live round-trip:

- `flat_to_open`
- then `open_to_flat`

on the target `create:market` path.

### End-of-day state

Latest runtime state now shows:

- `hybrid_state = "flat"`
- `open_position_owner = null`
- `open_position_side = null`
- `open_position_qty = 0.0`
- `pending_request_ids = []`
- `tracked_order_ids = []`
- `entry_intent_inflight = false`
- `exit_intent_inflight = false`

So the day ended flat and without leftover inflight garbage.

## 4. Overall Assessment For 2026-04-17

### Positive

- both patched gateways restarted cleanly
- both patched stacks returned to `ALLOWED`
- `trading-alor-usdrubf` produced a clean post-rollout market-entry success through the new path
- `trading-alor-usdrubf` also produced a clean post-rollout market-exit success through the same new path
- `trading-sessiongap` remained clean and flat
- no new infrastructure incident was observed during the pulled slice

### Still problematic

- `trading-hybrid` continues to show protective-order fragility
- `trading-hybrid` still has `orphan_trade` / stop-order ambiguity in the observed day
- `trading-hybrid` still lacks a clean post-patch `MeanReversion` protective validation case

## Immediate Watchpoints

1. `trading-hybrid`

- next live protective install after the gateway patch
- whether TP / SL now get broker accept instead of:
  - `protocol_reset_without_close_handshake`
  - `cws response timeout`
- whether current short position exits cleanly and returns to flat

2. `trading-alor-usdrubf`

- confirm the same clean action-scoped behavior repeats on the next session, not just once on entry and once on exit
- whether repeated `create:market` transport resets materially decrease

3. `trading-sessiongap`

- keep as regression baseline
- confirm it remains clean while shared gateway logic has changed for the other stacks

## Current Verdict

The patched rollout is operationally healthy.

But the live validation result is asymmetric:

- `trading-alor-usdrubf`: first meaningful positive patched round-trip already observed (`entry` and `exit`)
- `trading-hybrid`: patch applied, but the main protective validation case remains unresolved and still needs close observation
