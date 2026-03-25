# Post-Fix Clean And Stress Loops: `create:limit` / `delete:limit`

Date: 2026-03-25

Related documents:

- `docs/create-limit-delete-limit-instrumented-interim-2026-03-25.md`
- `docs/create-limit-delete-limit-chronology-memo-2026-03-25.md`
- `docs/create-limit-and-sessiongap-review-ready-2026-03-23.md`

## 1. Purpose

This note records the first clean repeated-loop results collected after:

- the fresh transport-ordering instrumentation rollout;
- the separate-token live discriminator;
- the CWS auth-cache fix:
  - invalidate cached `access_token` on explicit CWS `401`.

The goal of these runs was still not to broaden scope.

They were meant to answer a narrower operational question:

- on the new gateway build, does the passive `create:limit -> delete:limit` control path remain stable across repeated iterations on `sessiongap`?

## 2. Build And Environment

Gateway commit:

- `0735d62` `fix(gateway): invalidate cached token on cws 401`

Gateway image on VPS:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-0735d62-diag-20260325`

Runtime image left unchanged:

- `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`

Stack under test:

- `sessiongap`

Auth context:

- separate broker principal remained in place
- readiness showed:
  - `auth_principal_fingerprint = sha256:2dcaa06a8677f87a`

Capture sources:

- first clean loop:
  - run directory:
    - `/opt/diag-captures/20260325-153325`
  - summary:
    - `/opt/diag-captures/20260325-153325/sessiongap.loop.summary.txt`
- follow-on stress loop:
  - run directory:
    - `/opt/diag-captures/20260325-153754`
  - summary:
    - `/opt/diag-captures/20260325-153754/sessiongap.loop.summary.txt`

## 3. Executed Scenario

Helper:

- `/opt/limit_diag.sh loop`

### 3.1 First clean loop

Parameters:

- stack:
  - `sessiongap`
- price:
  - `80.10`
- iterations:
  - `20`
- qty:
  - `1.0`
- side:
  - `buy`
- sleep:
  - `2` seconds

Observed effective passive order price in broker stream:

- `80.09`

For each iteration the helper executed:

1. `create:limit`
2. wait for `accepted`
3. confirm `working`
4. `delete:limit`
5. wait for `accepted`
6. confirm terminal `canceled`

### 3.2 Follow-on stress loop

Parameters:

- stack:
  - `sessiongap`
- price:
  - `80.00`
- iterations:
  - `50`
- qty:
  - `1.0`
- side:
  - `buy`
- sleep:
  - `1` second

Observed effective passive order price in broker stream:

- `80.00`

## 4. Observed Result

### 4.1 First clean loop result

- `20 / 20 PASS`

For all `20` iterations:

- `create:limit` was accepted;
- broker order entered:
  - `working`
- `filled = 0.0`
- `delete:limit` was accepted;
- broker order entered:
  - `canceled`

No iteration produced:

- `cws_error`
- `protocol_reset_without_close_handshake`
- `cws_transport_failure`
- `cws_fail_pending`
- `401 Invalid JWT token!`

Representative example from the clean loop:

- place:
  - `request_id = 4d6bf989-c6d9-4ec9-8a75-7864fbbd6463`
  - `order_id = 2023555931497110203`
  - `status = accepted`
  - order reached:
    - `working`
- cancel:
  - `request_id = a77e7728-308e-4792-9d11-eb9fb38bfb54`
  - `status = accepted`
  - order reached:
    - `canceled`

Final iteration:

- place:
  - `request_id = 5cbcf436-9c6c-4bd5-a4cd-13c1f80b5310`
  - `order_id = 2023555931497111008`
- cancel:
  - `request_id = 77522830-020e-4b08-9548-4e7f3d4dc1ac`
  - terminal:
    - `canceled`

### 4.2 Follow-on stress loop result

- `50 / 50 PASS`

For all `50` iterations:

- `create:limit` was accepted;
- broker order entered:
  - `working`
- `filled = 0.0`
- `delete:limit` was accepted;
- broker order entered:
  - `canceled`

No iteration produced:

- `cws_error`
- `protocol_reset_without_close_handshake`
- `cws_transport_failure`
- `cws_fail_pending`
- `401 Invalid JWT token!`

Representative first iteration:

- place:
  - `request_id = ed0daaeb-0476-4dad-ace1-d0756e40c648`
  - `order_id = 2023555931497111870`
- cancel:
  - `request_id = b6a1868b-9c5d-47d9-8a00-6a002df13241`
  - terminal:
    - `canceled`

Final iteration:

- place:
  - `request_id = 01e721c9-bc56-4b15-86e1-48401b5f27ca`
  - `order_id = 2023555931497112329`
- cancel:
  - `request_id = 4e943a1d-e8ae-4687-a0e5-577769326f22`
  - terminal:
    - `canceled`

### 4.3 Combined readout

Across the two post-fix repeated-loop runs:

- total completed iterations:
  - `70`
- total failed iterations:
  - `0`

## 5. Gateway State After The Loop

Readiness after the stress loop:

- `readiness = true`
- `gateway_phase = LiveReady`
- `cws_authorized = true`

CWS counters after the stress loop:

- `cws_protocol_reset_total = 0`
- `cws_limit_error_total = 0`
- `cws_pending_failed_total = 0`
- `cws_limit_send_total = 71`

Recent transport health:

- `cws_last_transport_failure_ts_utc = null`
- `cws_last_limit_error_ts_utc = null`

Position / safety state:

- no filled live position was opened during the loop
- recent broker order stream showed terminal `canceled` states for the looped orders

## 6. Important Correlation Note

The clean loop still showed the already-known correlation behavior:

- some `working` lifecycle events arrived with:
  - `request_id = null`
- gateway recovered request identity from local state / `request_map`

In this clean run that behavior did not cause operational failure.

Interpretation:

- the correlation nuance remains real;
- however, under this run it coexisted with a fully clean control path.

## 7. Practical Interpretation

This run materially strengthens the following points:

- the residual incident class is still intermittent, not deterministic;
- on the post-fix build, `sessiongap` passive `create:limit -> delete:limit` can execute repeatedly without transport reset;
- the auth-cache invalidation fix did not introduce regressions in the repeated clean path;
- no post-failure `401 Invalid JWT token!` loop appeared during either repeated series;
- the same gateway process / same live line sustained:
  - one `20 / 20` clean loop
  - one `50 / 50` clean loop
  - with `70 / 70` completed iterations in total.

This run does **not** prove that the earlier incident class is gone.

It does prove that:

- the current gateway line is capable of sustained clean repeated limit-order control-path execution under the tested conditions.

## 8. Recommended Next Step

Use this run as the clean comparison artifact.

Then choose one narrow follow-up:

1. a longer or tighter repeated loop on the same stack;
2. the same loop on `hybrid`;
3. wait for the next natural `REPRO` and compare it directly against this clean series on the same gateway build.
