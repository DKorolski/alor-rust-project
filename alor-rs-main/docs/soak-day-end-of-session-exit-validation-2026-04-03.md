# Soak Day End-Of-Session Exit Validation

Date: 2026-04-03

## Goal

Verify that both live contours:

- `sessiongap`
- `hybrid`

can:

1. enter a real position during the trading day,
2. keep the position through the session,
3. emit a real end-of-session exit,
4. return to `Flat` without hanging in exit-timeout or recovery paths.

## Stack Baseline

Runtime image:

- `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-0b605ac-indwarm-20260402-213111`

Gateway image:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-71c09ac-actionscope2-20260402-131941`

Both stacks remained on the post-`restart from zero` baseline from `2026-04-02`.

## Sessiongap Result

Entry:

- `2026-04-03T12:00:02.772282Z`
  - live phase transition `Flat -> PendingEntry`
  - runtime emitted `intent_emitted action="place"`
- `2026-04-03T12:00:33.071435Z`
  - phase transition `PendingEntry -> InPosition`
- `cmd.acks.7502MIW`
  - request `71c5b46f-f18f-5091-b45a-fa94e0df3557`
  - status `accepted`
  - broker order `2023555961561723768`

Exit:

- `2026-04-03T20:31:09.438586Z`
  - live phase transition `InPosition -> PendingExit`
  - runtime emitted `intent_emitted action="place"`
- `2026-04-03T20:31:09.818550Z`
  - phase transition `PendingExit -> Flat`
- `cmd.acks.7502MIW`
  - request `af4a62a1-9065-56a4-874c-c59adf1ac76c`
  - status `accepted`
  - broker order `2023555961561837608`

Final position state:

- `broker.positions.7502MIW` ended at `qty=0.0`

## Hybrid Result

Entry:

- `2026-04-03T09:13:02.286204Z`
  - `hybrid actions generated`
  - `submit_entry owner=IntradayBreakout side=Short`
  - runtime emitted `intent_emitted action="place"`
- `cmd.acks.7502SN6`
  - request `cf7e08bb-427f-53c4-8928-d96f385e66e9`
  - status `accepted`
  - broker order `2033126110769774494`

Exit:

- `2026-04-03T20:31:57.056954Z`
  - `hybrid actions generated`
  - `submit_exit owner=IntradayBreakout reason=BreakoutEodExit`
  - runtime emitted `intent_emitted action="place"`
- `cmd.acks.7502SN6`
  - request `34a26c68-cdb6-5c35-a123-2fb0366875f0`
  - status `accepted`
  - broker order `2033126110769916175`
- `2026-04-03T20:31:58.560083Z`
  - exit order fill observed in runtime logs

Final position state:

- `broker.positions.7502SN6` ended at `qty=0.0`

## Timeout / Recovery Check

No evidence was seen for:

- `exit timeout`
- `deferred exit`
- `exit recovery`
- `close_only_degraded`
- stuck `PendingExit`

Both systems closed successfully within the normal session-end contour.

## Readiness After Close

At the time of inspection, both runtimes reported:

- `readiness=true`
- `runtime_phase="LiveReady"`
- `live_guard="ALLOWED"`

This indicates the end-of-session exit path did not leave either stack degraded.

## Conclusion

The `2026-04-03` soak day supports the current live baseline.

Confirmed:

1. `sessiongap` can enter, hold, and flatten by session-end without exit-timeout pathology,
2. `hybrid` can enter, hold, and flatten by session-end without exit-timeout pathology,
3. both stacks remain healthy and `ALLOWED` after the close cycle.
