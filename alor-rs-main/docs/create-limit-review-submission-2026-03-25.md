# Review Submission: `create:limit` / `delete:limit` / `marketable limit`

Date: 2026-03-25

Primary artifacts:

- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-tz1.5-results-2026-03-26.md`
- `docs/create-limit-tz1.5-results-2026-03-26-artifacts/README.md`
- `docs/create-limit-tz1.6-results-2026-03-26.md`
- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/README.md`
- `docs/create-limit-hardening-2.0-results-2026-03-26.md`
- `docs/create-limit-hardening-2.0-results-2026-03-26-artifacts/README.md`
- `docs/create-limit-and-sessiongap-review-ready-2026-03-23.md`
- `docs/create-limit-delete-limit-formal-chronology-2026-03-25.md`
- `docs/create-limit-delete-limit-chronology-memo-2026-03-25.md`
- `docs/create-limit-delete-limit-post-fix-clean-loop-2026-03-25.md`

## 1. Current Submission Position

Recommended review posture:

- ready for specialist and project review;
- materially narrowed;
- not root-cause closed.

This package is now strong enough for formal review because it contains both:

- fresh clean-pass evidence;
- fresh repro evidence;
- and fresh immediate-retry-after-reconnect evidence on the same diagnostic line.

## 2. What Was Added Since The Earlier Review Package

Since the earlier `2026-03-23` review-ready report, this package now also includes:

1. gateway auth-cache mitigation:
   - invalidate cached `access_token` on explicit CWS `401`;
2. live token-usage diagnostics:
   - `access_token_fingerprint`
   - token source / consumer / age;
3. post-fix passive limit repeated-loop verification:
   - `70 / 70 PASS` on `sessiongap`;
4. clean `marketable limit` B2 verification:
   - `hybrid PASS`
   - `sessiongap PASS`
   - `sessiongap after forced gateway restart PASS`;
5. fresh passive `create:limit` repros on both stacks:
   - `hybrid REPRO`
   - `sessiongap REPRO`;
6. immediate retry after reconnect on both stacks:
   - `hybrid PASS`
   - `sessiongap PASS`.
7. `TZ 1.5` idle/control-path silence aging results:
   - `idle 20m PASS`
   - `idle 30m REPRO`
   - `idle 30m + mid-window keepalive still REPRO`.
8. `TZ 1.6` final validation before hardening:
   - `idle30 baseline REPRO`
   - `10m cadence REPRO`
   - `5m cadence REPRO`
   - `reconnect before first order PASS`.
9. `TZ 2.0` hardening rollout validation:
   - fresh path `PASS` without recycle
   - stale path `PASS` via explicit recycle-before-send
   - market path regression `PASS`

## 3. Strongest Current Conclusions

### 3.1 The residual issue remains intermittent

The reviewed line is now proven capable of:

- clean passive `create:limit -> delete:limit`;
- clean `marketable limit` entry and exit;
- clean immediate retry after reconnect.

Therefore the residual incident class is not best described as:

- blanket limit-path failure;
- blanket marketable-limit failure;
- or a deterministic post-reconnect poison state.

### 3.2 Fresh repros on both stacks still point to transport/control-path failure

Fresh passive `create:limit` repros were observed on:

- `hybrid`
- `sessiongap`

Both fresh repros showed:

- immediate `protocol_reset_without_close_handshake`;
- `cws_transport_failure`;
- `cws_fail_pending`;
- `broker_order_id = null`.

That keeps the strongest incident framing in the transport/control-path class.

### 3.3 Reconnect recovery currently looks healthier than before

After the fresh repros:

- both stacks recovered CWS connectivity;
- token diagnostics showed fresh token refresh on recovery;
- the immediate retry on the recovered session passed cleanly.

This materially weakens the hypothesis that the first failure necessarily leaves the stack in a broken follow-on state.

### 3.4 The auth-cache fix improved recovery interpretability

The package now contains direct evidence that:

- restart/recovery can obtain a fresh `access_token`;
- earlier post-incident `401 Invalid JWT token!` loops are better explained by stale cached-token reuse during recovery than by a permanently invalid `refresh_token`.

This narrows the auth issue, but it does not by itself close the primary root cause for the transport reset incidents.

### 3.5 `TZ 1.5` narrows the idle/keepalive hypothesis further

The new `2026-03-26` `TZ 1.5` package now shows:

- `idle 20m` can pass cleanly;
- `idle 30m` can fail cleanly on the first `create:limit`;
- one successful mid-window keepalive at ~15m does not prevent the later ~30m `REPRO`.

This strengthens the view that the residual issue is tied to longer-lived, mostly idle CWS/control-path session state more than to ordinary safe create/delete activity, while also weakening the idea that one small keepalive is enough to clear it.

### 3.6 `TZ 1.6` resolves the practical hardening discriminator

The new `2026-03-26` `TZ 1.6` package now shows:

- `idle30 baseline -> REPRO`
- `idle30 + cadence 10m -> REPRO`
- `idle30 + cadence 5m -> REPRO`
- `idle30 + reconnect before first order -> PASS`

This is the strongest current practical conclusion in the whole package:

- regular safe cadence keepalive is insufficient;
- proactive reconnect or recycle before the first limit command after a long quiet window is the strongest current workaround direction.

### 3.7 `TZ 2.0` validates the chosen hardening path live

The first hardening rollout now shows the intended split directly:

- fresh `sessiongap` passive limit path passes without recycle;
- stale `sessiongap` passive limit path is detected as stale, recycled, and then passes on a fresh connection;
- `hybrid` market path passes without entering the new recycle flow.

This moves the package from:

- "diagnostically strong and ready for hardening"

to:

- "first hardening line implemented and live-validated".

## 4. Practical Readout For Review

What this package now supports with confidence:

1. the gateway diagnostics are working and useful;
2. the reviewed line can behave cleanly for both passive and marketable limit paths;
3. fresh repros still occur on the current line;
4. those repros remain intermittent;
5. reconnect recovery can be clean immediately afterwards.

What this package still does not prove conclusively:

1. whether the residual failure is purely broker-side transport instability;
2. whether a client-side ordering/correlation weakness is also required to explain the full incident class;
3. how the path behaves under a real network-driven reconnect rather than restart-based recovery.

## 5. Remaining Open Work

The remaining work is now comparatively narrow:

1. translate the `TZ 1.6` conclusion into a hardening implementation;
2. optionally add one controlled recycle-failure validation;
3. optionally add one real network-driven reconnect comparison if engineering still wants broker-vs-client confidence beyond the current operational gate.

## 6. Requested Review Focus

Requested focus for engineering review:

- accept `TZ 1.6` as the final analytic gate before hardening;
- treat the strongest current workaround as:
  - proactive reconnect or recycle before the first live `limit` order after a long quiet window;
- review whether the hardening should live:
  - in gateway session management,
  - in runtime orchestration,
  - or in both.

Requested focus for project review:

- accept the package as materially narrowed and review-ready;
- accept the analytic phase as sufficiently resolved for hardening;
- accept the first hardening line as live-validated for fresh-path, stale-path, and market-regression checks.

## 7. Bottom Line

The package is now substantially stronger than the earlier `2026-03-23` review set.

It contains:

- old and new repro evidence;
- old and new clean-pass evidence;
- repeated-loop stability evidence;
- marketable-limit evidence;
- immediate post-reconnect retry evidence on both stacks;
- and a final `TZ 1.6` hardening discriminator that favors reconnect/recycle over cadence keepalive.

Recommended overall position:

- ready for review;
- ready to move from hardening validation into rollout hardening/operationalization.
