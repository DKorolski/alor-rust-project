# Soak Start State: 2026-03-30

Date: 2026-03-30

Purpose:

- freeze the observed VPS state at the start of the March 30, 2026 soak window;
- record what is currently known about `sessiongap`, `hybrid`, and CWS transport behavior;
- record the practical implication for hardening 2.0 under the currently deployed `sessiongap` path.

## 1. Exact VPS State At Soak Start

Observed on VPS after market open on 2026-03-30:

- `sessiongap` stack:
  - gateway image: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-774b917-diag-20260326`
  - runtime image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`
  - runtime config: `/configs/runtime.sessiongap.live.7502MIW.toml`
  - gateway config: `/configs/gateway.sessiongap.live.7502MIW.toml`
  - mode: `live`
  - runtime readiness: `true`
  - `live_guard = ALLOWED`
  - `scheduler_state = Open`

- `hybrid` stack:
  - gateway image: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-774b917-diag-20260326`
  - runtime image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`
  - runtime config: `/configs/runtime.hybrid.paper.7502SN6.toml`
  - gateway config: `/configs/gateway.hybrid.live.7502SN6.toml`
  - mode: `paper`
  - runtime readiness: `false`
  - blocking reasons:
    - `trade_mode=Paper`
    - `allow_live_orders=false`

Both stacks were previously restarted from clean local VPS state:

- Redis persistence cleaned;
- reports cleaned;
- backup env files removed;
- compose changed to explicit per-service image tags;
- runtime and gateway relaunched on the fixed pair above.

## 2. What Has Been Stable So Far

At the start of soak on 2026-03-30:

- both stacks are up and healthy at the container level;
- `sessiongap` re-entered `LiveReady` after market open;
- `sessiongap` runtime is again `ALLOWED`;
- `hybrid` remains cleanly isolated in `paper`;
- no fresh `runtime`-side `orphan_trade` was observed after the clean restart;
- no fresh `runtime`-side `command rejected` / `cws_error` was observed after the market opened on 2026-03-30.

## 3. What Is Still Repeating

The repeating noise remains on the gateway transport side:

- `disconnect_kind="eof"`
- `disconnect_kind="protocol_reset_without_close_handshake"`
- `ws hub error; reconnecting`
- CWS reconnect / reauthorize cycles

This was observed repeatedly on both stacks during March 29-30, 2026.

Current reading:

- the transport issue is still alive as an intermittent gateway/CWS event class;
- it is not currently showing up as a new runtime-state corruption event after the clean restart;
- it remains most visible as a reconnect/recovery story rather than an immediate runtime crash story.

## 4. Most Important Finding For Hardening

The currently deployed `sessiongap` path is not the limit-entry path.

Current code/config behavior:

- deployed `sessiongap` runtime currently emits `Intent::Market` for entry and exit;
- current live config does not explicitly enable `marketable_limit`;
- gateway hardening 2.0 pre-entry recycle is currently wired to `Place` entry commands, not `Market` entry commands.

Practical implication:

- if the transport issue repeats under the currently deployed `sessiongap` strategy path, the event may be observable;
- but the specific `control_path_stale -> recycle -> send_after_recycle` hardening path is unlikely to be exercised by a normal `sessiongap` live entry on the current line;
- therefore a repeated failure may still be only partially diagnosable with respect to hardening 2.0.

## 5. What The Documents Support

The current documentation set points more strongly to:

- baseline `create:market` path stable;
- residual failure reproduced on `create:limit` / `marketable_limit`;
- main transport hypothesis centered on the shared limit/CWS path, not on market baseline behavior.

However, there is now a drift between:

- some integration notes that describe a `sessiongap` marketable execution migration;
- and the current deployed/configured code path, which is presently behaving as `Market`.

This means the historical hardening narrative and the current live `sessiongap` execution path are no longer perfectly aligned.

## 6. What We Can Diagnose If The Error Repeats

If the issue repeats during soak, the current instrumentation should still let us capture:

- exact transport-failure class;
- failure timestamp;
- affected gateway stack;
- `cws_connection_instance_id` before/after reconnect;
- reconnect / protocol-reset counters;
- whether runtime readiness was dropped;
- whether any runtime-side `command rejected`, `orphan_trade`, or ownership tail reappears.

## 7. What We Probably Cannot Fully Validate Under The Current Path

Without a real entry path that generates `Place`:

- we probably will not fully validate `control_path_recycle_start`;
- we probably will not fully validate `control_path_recycle_success`;
- we probably will not validate `control_path_send_after_recycle` on the natural `sessiongap` trading path;
- therefore a repeat transport incident may confirm the problem still exists, but may not prove that hardening 2.0 would or would not save the natural `sessiongap` live entry path.

## 8. Working Conclusion

Recommended interpretation at soak start on 2026-03-30:

- the environment is stable enough to continue passive soak observation;
- the clean restart improved observability and removed the earlier stale local state tail;
- the repeating transport issue is still a realistic recurrence candidate;
- if it recurs, it may not exercise the currently implemented hardening path because the deployed `sessiongap` execution mode is currently `Market`, while hardening 2.0 protection is implemented for stale `Place` entry sends.

Short form:

- continue soak;
- expect transport noise may recur;
- do not assume a recurrence will automatically validate hardening;
- treat any future recurrence as strong transport evidence, but only partial hardening evidence under the current `sessiongap` execution mode.
