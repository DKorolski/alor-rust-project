# Diagnostic Status Update: `create:limit` / `delete:limit` / `marketable limit`

Date: 2026-03-25

Related documents:

- `docs/create-limit-and-sessiongap-review-ready-2026-03-23.md`
- `docs/create-limit-delete-limit-instrumented-interim-2026-03-25.md`
- `docs/create-limit-delete-limit-formal-chronology-2026-03-25.md`
- `docs/create-limit-delete-limit-chronology-memo-2026-03-25.md`
- `docs/create-limit-delete-limit-post-fix-clean-loop-2026-03-25.md`
- `docs/create-limit-tz1.5-results-2026-03-26.md`
- `docs/session-gap-b2-runbook.md`

Follow-on note:

- later `TZ 1.5` idle/keepalive-specific live results were captured on `2026-03-26` and are documented separately in:
  - `docs/create-limit-tz1.5-results-2026-03-26.md`
  - `docs/create-limit-tz1.5-results-2026-03-26-artifacts/README.md`

## 1. Purpose

This note records the current diagnostic position after the next narrowing round completed on `2026-03-25`.

The goal of this round was not to close root cause.

It was to answer four narrower questions:

1. does the fresh gateway tracing remain operationally usable on live runs;
2. does the post-incident CWS `401` loop look more like cached-token reuse than dead `refresh_token` state;
3. does the passive `create:limit -> delete:limit` path remain stable on the current line across repeated iterations;
4. does the `marketable limit` B2 path still work cleanly both before and after a forced gateway reconnect.

## 2. Scope Completed In This Round

### 2.1 Gateway auth-cache fix and token-usage diagnostics

Implemented and deployed:

- invalidate cached `access_token` on explicit CWS `401`;
- add safe `access_token_fingerprint` diagnostics;
- expose token source / consumer / age in readiness and logs.

Relevant commits:

- `0735d62` `fix(gateway): invalidate cached token on cws 401`
- `3d43422` `feat(gateway): trace access token usage across reconnects`

### 2.2 Passive limit repeated-loop verification

Executed on `sessiongap`:

- clean loop:
  - `20 / 20 PASS`
- follow-on stress loop:
  - `50 / 50 PASS`

Documented separately in:

- `docs/create-limit-delete-limit-post-fix-clean-loop-2026-03-25.md`

### 2.3 `marketable limit` B2 verification

Executed on:

- `hybrid` paper stack
- `sessiongap` live stack
- `sessiongap` again immediately after forced gateway restart

These runs were used to check whether the marketable `create:limit` fill-confirm-exit path still behaves cleanly on the current line.

## 3. Build And Environment

Gateway diagnostic line used in this round:

- `0735d62` + `3d43422`

Gateway image on VPS:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-3d43422-diag-20260325`

Runtime line left unchanged during this round:

- `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`

Stacks under test:

- `sessiongap`
- `hybrid`

Auth setup:

- separate broker principals / tokens remained in place
- readiness fingerprints stayed distinct:
  - `sessiongap auth_principal_fingerprint = sha256:2dcaa06a8677f87a`
  - `hybrid auth_principal_fingerprint = sha256:c5a5ef042fbb04da`

## 4. What Was Verified

### 4.1 Cached-token hypothesis was narrowed materially

Observed earlier in failing conditions:

- after transport incidents, gateways could enter repeated:
  - `401 Invalid JWT token!`
  - `cws_authorized = false`
- operationally, restarting only `alor-gateway` with the same unchanged `refresh_token` restored service.

This round added direct token-fingerprint evidence.

#### Soft restart observation on `sessiongap`

Before restart:

- `access_token_fingerprint = sha256:449e19d97495894d`

After restart:

- `access_token_fingerprint = sha256:fea30ab2c4154b3a`

Interpretation:

- process restart did not keep using the same in-memory access token;
- after restart the gateway really obtained a fresh access token;
- therefore the earlier post-incident `401` loop is better explained by reuse of a stale cached token during reconnect handling than by restart preserving the same token.

Important boundary:

- this narrows the auth-recovery issue;
- it does not by itself prove that `cws_error` incidents were originally caused by token invalidity.

### 4.2 Passive `create:limit -> delete:limit` path was stable on the current line

From the two loop series on `sessiongap`:

- total completed iterations:
  - `70`
- total failed iterations:
  - `0`

Across these `70` iterations:

- `create:limit` was accepted;
- order reached `working`;
- `filled = 0.0`;
- `delete:limit` was accepted;
- order reached `canceled`.

No iteration produced:

- `cws_error`
- `protocol_reset_without_close_handshake`
- `cws_transport_failure`
- `cws_fail_pending`
- `401 Invalid JWT token!`

Interpretation:

- on the current post-fix gateway line, the passive limit control-path is not behaving like a constant defect;
- the incident class remains intermittent, not universal.

### 4.3 `marketable limit` B2 passed on `hybrid`

Capture:

- `/opt/diag-captures/20260325-162620`

Entry:

- `request_id = 4a773127-bcf6-47eb-adbf-a3719d8c65ec`
- `order_id = 2033126080705045731`
- `status = accepted`
- order lifecycle:
  - `working`
  - `filled`

Exit:

- `request_id = a61eb08e-5c26-40ff-9a48-d4a666eec292`
- `order_id = 2033126080705045733`
- `status = accepted`
- order lifecycle:
  - `working`
  - `filled`

Final position:

- `IMOEXF qty = 0.0`

Focused trace for this run showed:

- `create:limit` response matched pending request cleanly;
- `command_ack` published cleanly on both entry and exit;
- no transport reset markers for the fresh run;
- `event_request_id = null` still appeared on some order events, but `request_map` restoration remained correct and non-disruptive.

### 4.4 `marketable limit` B2 passed on `sessiongap`

Capture:

- `/opt/diag-captures/20260325-163311`

Entry:

- `request_id = f766e38a-f7d2-4670-a268-2495ac125b49`
- `order_id = 2023555931497142573`
- `status = accepted`
- order lifecycle:
  - `working`
  - `filled`

Observed resulting position:

- `USDRUBF qty = 1.0`
- `avg_price = 80.98`

Exit:

- `request_id = c53d6158-7ae4-43cb-9ecb-9f928aa26445`
- `order_id = 2023555931497142613`
- `status = accepted`
- order lifecycle:
  - `working`
  - `filled`

Final position:

- `USDRUBF qty = 0.0`

### 4.5 `marketable limit` B2 also passed immediately after forced gateway restart

Capture:

- `/opt/diag-captures/20260325-163411`

The gateway was restarted first:

- only `sessiongap-alor-gateway` was recreated;
- `strategy-runtime` was not recreated.

After restart readiness showed:

- `readiness = true`
- `cws_authorized = true`
- new `gateway_instance_id`
- new `cws_connection_instance_id`
- fresh `access_token_fingerprint = sha256:fea30ab2c4154b3a`

Entry after reconnect:

- `request_id = c0ebccad-2bea-4c98-957d-b59a123434f7`
- `order_id = 2023555931497142981`
- `status = accepted`
- order lifecycle:
  - `working`
  - `filled`

Exit after reconnect:

- `request_id = 0e55a98a-60e5-4e54-9484-e35cdbc4a917`
- `order_id = 2023555931497143068`
- `status = accepted`
- order lifecycle:
  - `working`
  - `filled`

Final position:

- `USDRUBF qty = 0.0`

Focused grep on both new `sessiongap` B2 captures showed no fresh:

- `cws_transport_failure`
- `cws_fail_pending`
- `protocol_reset_without_close_handshake`
- `error_code = cws_error`

Interpretation:

- the marketable entry/exit path stayed healthy immediately after a forced gateway reconnect;
- this does not prove network-driven reconnect behavior under a real transport break, but it does show that soft restart recovery is currently clean on the reviewed line.

### 4.6 `hybrid` fresh `create:limit` repro followed by clean immediate retry after reconnect

Fresh failing probe:

- capture:
  - `/opt/diag-captures/20260325-182648`
- `request_id = e7b8dbdf-5fd5-41ec-86a6-f7a2821625f6`
- action:
  - passive `create:limit`
- result:
  - `status = error`
  - `error_code = cws_error`
  - `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
  - `broker_order_id = null`

Gateway trace showed:

- `cws send`
- immediate:
  - `cws_transport_failure`
  - `cws_fail_pending`
- no broker order lifecycle for the fresh request

Readiness immediately after recovery showed:

- `cws_reconnect_seq = 1`
- `cws_protocol_reset_total = 1`
- refreshed token state:
  - `access_token_fingerprint = sha256:dfba7f25cf6b8c7a`
  - `access_token_last_source = refreshed`

Immediate retry after that reconnect:

- capture:
  - `/opt/diag-captures/20260325-183136`
- place:
  - `request_id = fa2635ee-a173-4c1a-90eb-93ecbab2873b`
  - `order_id = 2033126080705083412`
  - `status = accepted`
  - order reached:
    - `working`
- cancel:
  - `request_id = 049ea5b5-a137-4c5c-b71c-519fbc09a52f`
  - `status = accepted`
  - order reached:
    - `canceled`
  - `filled = 0.0`

Interpretation:

- the fresh `hybrid` incident reproduced the transport-reset class cleanly;
- the path did not remain stuck after reconnect;
- the immediate next probe on the recovered CWS session passed end-to-end.

### 4.7 `sessiongap` fresh `create:limit` repro followed by clean immediate retry after reconnect

Fresh failing probe:

- capture:
  - `/opt/diag-captures/20260325-183504`
- `request_id = cb970197-3cd8-4b15-928a-a9af74b2b71d`
- action:
  - passive `create:limit`
- result:
  - `status = error`
  - `error_code = cws_error`
  - `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
  - `broker_order_id = null`

Readiness immediately after recovery showed:

- `cws_reconnect_seq = 1`
- `cws_protocol_reset_total = 1`
- refreshed token state:
  - `access_token_fingerprint = sha256:b87fcfaff01542b1`
  - `access_token_last_source = refreshed`

Immediate retry after that reconnect:

- capture:
  - `/opt/diag-captures/20260325-183532`
- place:
  - `request_id = 21113d99-0d7e-47c7-83e6-724f2b4479d5`
  - `order_id = 2023555931497215624`
  - `status = accepted`
  - order reached:
    - `working`
- cancel:
  - `request_id = 20f0af8b-441c-41ee-88eb-54d74977edf4`
  - `status = accepted`
  - order reached:
    - `canceled`
  - `filled = 0.0`

Interpretation:

- `sessiongap` now shows the same practical pattern as `hybrid`;
- a fresh transport-reset repro can be followed by a clean immediate retry on the recovered session;
- the path is therefore intermittent rather than deterministically broken after the first incident.

## 5. Strongest Current Conclusions

### 5.1 What is now materially stronger

1. The auth-recovery issue is narrower than the main incident class.

What is now better supported:

- explicit CWS `401` recovery problems were plausibly aggravated by stale cached `access_token` reuse;
- the new invalidate-on-`401` fix is the correct minimal mitigation for that auth loop;
- restart obtains a fresh access token and currently restores service cleanly.

2. Passive limit and marketable-limit paths both work cleanly on the current line.

What is now directly observed:

- passive `create:limit -> delete:limit` can run cleanly across repeated loops;
- marketable entry and marketable exit can complete cleanly on both `hybrid` and `sessiongap`;
- marketable entry/exit also stayed clean immediately after a forced gateway restart.

3. The residual incident class is still intermittent.

The fresh clean runs materially weaken any framing that says:

- all valid `create:limit` are broken;
- all valid `delete:limit` are broken;
- all marketable-limit entry/exit sequences are broken;
- restart/reconnect recovery is currently broken in every case;
- the first repro necessarily poisons the immediately following request.

### 5.2 What remains true from earlier failing evidence

The earlier fresh failing evidence still matters.

Already documented earlier:

- passive `delete:limit` could reproduce `cws_error`;
- fresh `create:limit` could reproduce `protocol_reset_without_close_handshake`;
- final downstream cancel attribution in the older ambiguous run remained non-conclusive because of local request-map overwrite plus manual terminal cancellation.

These earlier failures are not overturned by the new clean runs.

They are now better interpreted as:

- intermittent control-path incidents on a path that otherwise can operate cleanly;
- not as a deterministic every-request defect.

## 6. Current Review Position

Recommended current framing:

- instrumentation rollout: complete and effective;
- auth-cache fix: implemented and live;
- passive limit post-fix stability check: passed in repeated-loop form;
- marketable-limit B2 verification: passed on `hybrid`, passed on `sessiongap`, passed again after forced gateway restart;
- fresh passive `create:limit` repros were also observed on both `hybrid` and `sessiongap`;
- on both stacks those repros were followed by clean immediate retry after reconnect;
- overall: materially narrowed, operationally stronger, still not root-cause closed.

## 7. What Is Still Open

The main open items are now narrower.

1. A real network-driven reconnect test still remains.

What is already done:

- soft restart recovery test.

What is not yet done:

- long enough forced external disconnect to trigger an actual CWS reconnect path rather than a trivial short detach.

2. A fresh failing run on the current token-diagnostics line is still desirable.

This is now narrower than before, because the current line already contains fresh repros on both stacks.

What is still desirable is not just "another repro", but:

- a repro with full comparison against the immediate clean retry on the same recovered connection;
- or a repro under a real network-driven reconnect rather than a naturally recovered transport reset.

The strongest next comparison would now be:

- current clean pass artifacts on the same line;
- current failing artifact on the same line;
- current immediate post-reconnect pass artifact on the same line;
- then direct `PASS vs REPRO` comparison using:
  - `request_id`
  - `order_id`
  - `send_seq`
  - `recv_seq`
  - `access_token_fingerprint`
  - `handler/state` chronology.

3. It is still not proved whether the residual failing incident is:

- purely broker-side transport/control instability;
- purely client-side ordering/correlation weakness;
- or a combined issue where an intermittent external failure is made harder to interpret by local correlation semantics.

## 8. Practical Next Step

Recommended next-step order:

1. preserve the current clean-state evidence package;
2. run one longer network-driven reconnect test;
3. after recovery, repeat a short B2 or passive limit probe;
4. wait for the next fresh `REPRO` on the same diagnostic line rather than broadening scope again;
5. compare the next `REPRO` directly against the current clean `PASS` captures.

## 9. Bottom Line

The diagnostic picture is materially stronger than it was before this round.

What we now have in hand:

- working token-fingerprint diagnostics;
- live proof that restart acquires a fresh access token;
- `70 / 70 PASS` passive limit loops on `sessiongap`;
- clean `marketable limit` B2 on `hybrid`;
- clean `marketable limit` B2 on `sessiongap`;
- clean `marketable limit` B2 again immediately after forced gateway restart.
- fresh `hybrid` passive `create:limit REPRO` followed by clean immediate retry after reconnect;
- fresh `sessiongap` passive `create:limit REPRO` followed by clean immediate retry after reconnect.

What this means:

- the system is capable of clean limit-order and marketable-limit behavior on the reviewed line;
- the auth-cache fix improved the interpretability and recovery story around post-incident `401` behavior;
- the freshest incidents on both stacks still present as transport-reset class failures, not as persistent auth-dead states;
- the next request after reconnect can succeed cleanly, so the post-incident state is not deterministically poisoned;
- the residual live issue should still be treated as intermittent and not yet root-caused;
- the next best evidence is a fresh `REPRO` on the same diagnostic line or a real network-driven reconnect experiment.
