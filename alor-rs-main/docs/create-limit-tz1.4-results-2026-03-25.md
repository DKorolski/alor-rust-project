# TZ 1.4 Results: Preflight Snapshot, Idle Aging, And Active Aging

Date: 2026-03-25

Related documents:

- `docs/create-limit-tz1.4-preflight-and-activity-aging-2026-03-25.md`
- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-review-submission-2026-03-25.md`
- `docs/create-limit-tz1.4-results-2026-03-25-artifacts/README.md`

## 1. Purpose

This note records what was actually implemented and observed during the `TZ 1.4` diagnostic round.

`TZ 1.4` was introduced after the earlier restart-based checks produced:

- immediate `PASS`
- `10m PASS`
- `16m PASS`

on the same clean `sessiongap` CWS session.

That materially weakened the simple framing:

- "the path deterministically breaks after roughly `10-15` minutes"

The goal of `TZ 1.4` was therefore to distinguish more clearly between:

- idle session aging;
- active control-path aging;
- and rarer sequence/state degradation.

## 2. Scope Completed

### 2.1 Preflight snapshot support was added

Implemented in this round:

- expanded readiness fields for preflight comparison;
- added `limit_diag.sh preflight`;
- added automatic per-iteration preflight snapshot during loop runs.

Relevant commit:

- `005697a` `feat(diag): add preflight snapshots for limit aging checks`

Implemented files:

- `alor-gateway/src/health_server.rs`
- `scripts/limit_diag.sh`
- `docs/create-limit-tz1.4-preflight-and-activity-aging-2026-03-25.md`

### 2.2 Two explicit branches were tested

1. `idle aging`
2. `active aging`

The experiments were run on `sessiongap`.

### 2.3 Local review bundle was assembled

For review convenience, a compact local artifact bundle was copied next to this report:

- `docs/create-limit-tz1.4-results-2026-03-25-artifacts/`

That bundle contains:

- baseline preflight summaries;
- selected iteration preflight summaries;
- loop summaries;
- selected readiness JSON snapshots;
- the failing `sessiongap.gateway.post.log` for the `idle 30m` repro branch.

The full raw capture trees remain on the VPS under `/opt/diag-captures/...`.

## 3. Preflight Snapshot Coverage

The new preflight capture recorded, for each baseline or loop iteration:

- readiness state
- gateway / CWS identity
- access-token fingerprint and source
- CWS connection age
- reconnect counters
- limit send / error counters
- pending count
- subscription / WS freshness indicators
- command-consumer health counters
- supporting stream and log tails

This made it possible to compare:

- clean `PASS` pre-state
- against the pre-state immediately before the first `FAIL`

without relying only on post-incident logs.

## 4. Idle-Aging Result

### 4.1 Clean baseline

Baseline capture:

- `/opt/diag-captures/20260325-211853`

Key baseline fields:

- `gateway_instance_id = 600068d7-1047-48dc-9500-6263912e0c1f`
- `cws_connection_instance_id = 0694ae06-63ff-49a0-b89d-3ee7b528fe7e`
- `cws_connection_age_sec = 6`
- `cws_reconnect_seq = 0`
- `cws_protocol_reset_total = 0`
- `cws_limit_send_total = 0`
- `cws_limit_error_total = 0`
- `cws_pending_count = 0`
- `readiness = true`
- `cws_authorized = true`

### 4.2 Probe after `30m` idle

Probe capture:

- `/opt/diag-captures/20260325-214901`

Failing command:

- `request_id = 6faa6b3d-dede-44ad-9c15-de06fefa3440`
- `opcode = create:limit`
- `status = error`
- `error_code = cws_error`
- `error_msg = "cws disconnected: protocol_reset_without_close_handshake"`
- `broker_order_id = null`

Preflight immediately before the failing probe still showed a clean-looking session:

- same `cws_connection_instance_id = 0694ae06-63ff-49a0-b89d-3ee7b528fe7e`
- `cws_connection_age_sec = 1808`
- `readiness = true`
- `cws_authorized = true`
- `cws_reconnect_seq = 0`
- `cws_protocol_reset_total = 0`
- `cws_limit_send_total = 0`
- `cws_limit_error_total = 0`
- `cws_pending_count = 0`

Gateway chronology:

1. `cws send`
2. immediate `cws_transport_failure`
3. `disconnect_kind = protocol_reset_without_close_handshake`
4. `cws_fail_pending`
5. `command ack published status=Error`

Interpretation:

- a long mostly idle session can still fail on the first fresh `create:limit` probe;
- the session did not show an obvious degraded preflight signature immediately beforehand;
- idle aging therefore remains a strong current suspect.

## 5. Active-Aging Results

### 5.1 Pilot: `5` cycles

Baseline:

- `/opt/diag-captures/20260325-215351`

Loop:

- `/opt/diag-captures/20260325-215400`

Result:

- `5 / 5 PASS`

Session identity stayed constant through the whole run:

- `cws_connection_instance_id = ff854256-722c-4328-b4fb-af5cf045200f`
- `cws_reconnect_seq = 0`
- `access_token_fingerprint = sha256:80ae4578143435d3`

Observed connection ages:

- iter1: `9s`
- iter5: `501s`

### 5.2 Main run: `10` cycles

Baseline:

- `/opt/diag-captures/20260325-221647`

Loop:

- `/opt/diag-captures/20260325-221656`

Result:

- `10 / 10 PASS`

Session identity stayed constant:

- `cws_connection_instance_id = 85f458e5-44b2-416a-9838-7e0b40cc5a27`
- `cws_reconnect_seq = 0`
- `access_token_fingerprint = sha256:b8285f7e179f466a`

Observed connection ages:

- iter1: `11s`
- iter10: `1115s`

### 5.3 Extended run: `15` cycles

Baseline:

- `/opt/diag-captures/20260325-224713`

Loop:

- `/opt/diag-captures/20260325-224747`

Result:

- `15 / 15 PASS`

Session identity stayed constant:

- `cws_connection_instance_id = 9d1eb0e1-4a7a-4925-8f0d-22b220032078`
- `cws_reconnect_seq = 0`
- `access_token_fingerprint = sha256:9c7b47ede7cd2282`

Observed connection ages:

- iter1: `35s`
- iter10: `1139s`
- iter15: `1753s`

### 5.4 Shared active-aging behavior

Across all active-aging runs:

- every `create:limit` was accepted;
- every order reached `working`;
- every `delete:limit` was accepted;
- every order reached `canceled`;
- `filled = 0.0` throughout;
- no reconnect occurred inside the loop;
- `cws_error = 0`;
- `protocol_reset_without_close_handshake = 0`.

Combined active-aging total:

- `30 / 30 PASS`

## 6. What TZ 1.4 Changed

The earlier working suspicion was:

- elapsed time alone may be enough to explain the residual failure

`TZ 1.4` sharpens that picture.

What is now materially stronger:

1. short and medium active control activity does not by itself reproduce the incident;
2. safe passive `create:limit -> delete:limit` cycling remained healthy for:
   - `5` cycles
   - `10` cycles
   - `15` cycles
3. a clean-looking session can still fail after a long idle window.

What is now materially weaker:

- the idea that ordinary safe control activity accumulation is the primary trigger

What is still not proved:

- whether a longer active-aging series would eventually fail;
- whether the real trigger is pure idle age or a rarer hidden state that becomes more likely during idle periods;
- whether the same pattern holds under forced network-driven reconnect rather than restart-based clean baselines.

## 7. Strongest Current Conclusion

The strongest current `TZ 1.4` readout is:

> The residual `create:limit` failure class correlates more strongly with longer idle session aging than with short-to-medium safe control-path activity. On the current line, a clean `sessiongap` CWS session reproduced a transport reset after approximately `30` minutes of idle age, while active safe cycling on separate clean sessions completed `5/5`, `10/10`, and `15/15` without transport failure or reconnect.

This does not yet prove a simple deterministic "break after `30m`" rule.

But it does materially narrow the next search space toward:

- idle-age-related degradation;
- or a rarer latent condition that is not triggered by ordinary safe control cycling.

## 8. Recommended Next Step

The next highest-value checks after `TZ 1.4` are:

1. repeat `idle aging` with a longer interval such as `45m`;
2. if needed, extend `active aging` further only after the next idle check;
3. keep collecting preflight snapshots immediately before the first probe on any clean session intended for comparison.
