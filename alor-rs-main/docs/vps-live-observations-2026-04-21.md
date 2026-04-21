# VPS Live Observations - 2026-04-21

Date: 2026-04-21

Scope:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

Context:

- `trading-hybrid` is running patched runtime `manual-084308c` after clean `from zero` restart.
- `trading-hybrid` and `trading-alor-usdrubf` gateway hotfix rollout remains active on `alor-gateway:manual-5430299`.
- This note summarizes:
  - weekend / overnight behavior
  - trading session on 2026-04-20
  - trading session on 2026-04-21 up to observation time

## Executive Summary

The most important result is mixed but informative:

- `trading-hybrid` produced a clean patched validation signal on 2026-04-20:
  - clean `IntradayBreakout` entry
  - clean `BreakoutEodExit` at `23:30`
  - no stale `pending_exit_active`
  - no visible `trading_window_closed` reject / skew repeat in the observed slice
- `trading-alor-usdrubf` remained materially cleaner on the target hot path:
  - repeated `create:market` intents on 2026-04-20 and 2026-04-21 went through the action-scoped path
  - forced token refresh before authorize was visible
  - broker accepted and execution confirmed
- a new separate operational incident appeared on 2026-04-21:
  - `trading-sessiongap` entered a restart loop
  - root symptom is `sessiongap-redis` memory-cgroup OOM during AOF/RDB load and rewrite
  - runtime now repeatedly dies on `BusyLoadingError`, `Connection refused`, and later name-resolution failures while Redis is flapping

So the current reading is:

- Patch validation for `hybrid` and `alor-usdrubf` is encouraging.
- `sessiongap` did hit a real Redis memory incident during the day, but it was later recovered with a clean `from zero` Redis restart.
- The next operational priority is preventive reduced-trim rollout for `hybrid` and `alor-usdrubf` in a safe pre-open window.

## 1. Weekend / Overnight Path

For `trading-hybrid` and `trading-alor-usdrubf`, weekend and overnight behavior looked operationally normal:

- end-of-day `ALLOWED -> BLOCKED` transitions occurred at the usual boundary
- overnight reconnect noise appeared as isolated long-lived CWS transport resets:
  - `protocol_reset_without_close_handshake`
- both stacks returned through:
  - `SyncingGap`
  - `SyncingHistory`
  - then back to `LiveReady / ALLOWED` around `06:00 UTC` on 2026-04-20 and 2026-04-21

This overnight noise remained gateway-side and did not show the old trade-path failures on the patched validation slices.

## 2. `trading-hybrid`

### 2026-04-20 trading session

Observed clean live sequence:

- `18:30 local`
  - `hybrid actions generated`
  - `submit_entry owner=IntradayBreakout side=Long style=Market reason=BreakoutLong`
- gateway action-scoped limit path:
  - fresh action-scope session open
  - forced token refresh
  - authorize
  - `create:limit`
  - `Accepted`
- runtime:
  - `command acknowledged outcome="accepted"`
  - `execution confirmed`

Later, a clean EOD cycle also appeared:

- `23:30 local`
  - `hybrid actions generated`
  - `submit_exit owner=IntradayBreakout reason=BreakoutEodExit`
- runtime:
  - `command acknowledged outcome="accepted"`
  - `execution confirmed`

Most important reading:

- the patched runtime did not reproduce the old 2026-04-17 failure mode
- there was no visible stale `pending_exit_active`
- there was no visible `trading_window_closed` reject on this observed path
- there was no visible repeat of the request-id skew incident in the pulled slice

This does not yet prove every exit case is fixed, but it is a real positive patched live signal.

### 2026-04-21 status

Today the stack shows only ordinary overnight lifecycle:

- `ALLOWED -> BLOCKED` at session end
- `SyncingGap -> SyncingHistory -> ALLOWED` by `06:00 UTC`

Current runtime state snapshot:

- `last_position_qty = 0.0`
- `current_owner = null`
- `current_side = null`
- `pending_entry_* = null`
- `pending_exit_* = null`
- `deferred_entry_* = null`
- `deferred_exit_* = null`
- `tp/sl` fields are null

Current verdict for `trading-hybrid`:

- patched validation signal: positive
- target incident recurrence in observed slice: not seen
- still needs more sessions, but today’s evidence is encouraging

## 3. `trading-alor-usdrubf`

### 2026-04-20 trading session

The target path remained clean and repeated multiple times.

Observed sequences included:

- clean short entry around `07:04 UTC`
- clean `mr_take` exit around `07:07 UTC`
- clean short entry around `07:09 UTC`
- clean `mr_stop` exit around `07:30 UTC`
- clean short entry around `07:55 UTC`
- clean `mr_take` exit around `07:56 UTC`
- clean `bo_eod_exit` around `20:31 UTC`

The gateway evidence is especially important:

- `primary_opcode="create:market"`
- action-scoped session open
- forced token refresh before authorize
- authorize ok
- `create:market`
- `http_code=Some(200)`
- clean close

Runtime side matched that cleanly:

- `intent_emitted action="market"`
- `command acknowledged outcome="accepted"`
- `execution confirmed`
- correct `flat_to_open` / `open_to_flat` position transitions

### 2026-04-21 trading session

Today the same improved path is still visible:

- strategy generated `bo_short_signal`
- emitted short market entry
- gateway again used action-scoped `create:market`
- accepted ack arrived
- execution confirmed
- strategy moved to broker-confirmed open short

Current runtime state snapshot:

- `hybrid_state = "open"`
- `open_position_owner = "day_breakout_waitfix"`
- `open_position_side = "short"`
- `open_position_qty = 1.0`
- `pending_request_ids = []`
- `tracked_order_ids = []`
- `entry_intent_inflight = false`
- `exit_intent_inflight = false`

Current verdict for `trading-alor-usdrubf`:

- target `create:market` hot path remains materially cleaner after the patch
- no repeated old burst/defer storm was visible in the observed 2026-04-20 / 2026-04-21 slice
- this remains the strongest positive Patch A validation signal

## 4. `trading-sessiongap`

### 2026-04-20 / overnight

No clean core trading signal was captured in the pulled slice.
The visible pattern was mostly repeated startup / guard churn and later a much more serious runtime availability issue.

### 2026-04-21 incident

At observation time the stack is not healthy:

- `trading-sessiongap-strategy-runtime-1`
  - `Restarting`
  - `RestartCount=10123`
- `trading-sessiongap-redis-1`
  - repeatedly restarting
  - `RestartCount=3748`
  - memory usage around `936 MiB / 1 GiB`

Runtime symptoms:

- `xreadgroup failed error=Connection refused`
- `BusyLoadingError: Redis is loading the dataset in memory`
- later `Temporary failure in name resolution`

Redis evidence:

- repeated AOF / RDB reload on startup
- `RDB memory usage when created 1396.65 Mb`
- repeated background AOF rewrite start
- background AOF rewrite terminated by signal `9`

Kernel evidence from `dmesg`:

- repeated `memory cgroup out of memory`
- `Killed process ... (redis-server)`
- multiple kills between roughly `12:06:14` and `12:07:02 MSK`

Reading:

- this is not a strategy-logic anomaly
- this is a live operational Redis memory incident
- it is stack-local to `sessiongap` in the observed slice
- it is materially different from the cleaner state of `hybrid` and `alor-usdrubf`

### Later Recovery On 2026-04-21

After the incident triage, `trading-sessiongap` was recovered with a controlled `from zero` Redis restart.

Actions performed:

- reduced live trim on VPS to:
  - `bars = 10000`
  - `orders = 5000`
  - `trades = 5000`
  - `positions = 20000`
  - `commands = 5000`
  - `acks = 5000`
  - `health = 5000`
  - `runtime_state = 500`
- stopped only the `trading-sessiongap` stack
- moved old Redis persistence aside
- created a fresh empty Redis data directory
- started the stack again on clean persistence

Post-recovery validation:

- `trading-sessiongap-redis-1`
  - `restart=0`
  - memory around `10 MiB / 1 GiB`
- `trading-sessiongap-strategy-runtime-1`
  - `restart=0`
  - `readiness=true`
  - `runtime_phase="LiveReady"`
  - `live_guard="ALLOWED"`
- Redis startup logs showed fresh AOF creation instead of old AOF/RDB replay
- runtime no longer showed `BusyLoadingError`, `broken pipe`, or `Connection refused` restart churn

Operational verdict after recovery:

- the root cause still matters and should drive retention hardening
- but `sessiongap` itself ended the maintenance window in a healthy recovered state

## Final Verdict

For the current micro live soak:

- `trading-hybrid`
  - positive patched signal
  - clean observed entry/EOD-exit sequence
  - no visible recurrence of the stale deferred-exit incident in the pulled slice
- `trading-alor-usdrubf`
  - positive patched signal
  - repeated clean action-scoped `create:market` behavior across yesterday and today
- `trading-sessiongap`
  - real Redis memory / persistence incident during the day
  - later recovered successfully with `from zero`
  - no longer in restart-loop after recovery, but still needs retention hardening to avoid recurrence

Most important next step:

- keep observing `hybrid` and `alor-usdrubf` as the patched validation line
- perform preventive reduced-trim rollout for `hybrid` and `alor-usdrubf` in the next safe pre-open window
- keep `sessiongap` under resource watch so the Redis incident does not recur
