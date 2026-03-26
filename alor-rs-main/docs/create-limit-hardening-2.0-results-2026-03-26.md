# Hardening 2.0 Results: Pre-Send Recycle For Idle-Time CWS Control-Path Degradation

Date: 2026-03-26

Related documents:

- `docs/create-limit-tz1.6-results-2026-03-26.md`
- `docs/create-limit-review-submission-2026-03-25.md`
- `docs/create-limit-diagnostic-status-update-2026-03-25.md`

## 1. Purpose

This change implements the first hardening step after the `TZ 1.4` to `TZ 1.6` diagnostic series.

Target behavior:

- do not send the first live `create:limit` or marketable-limit entry command blindly after long control-path silence;
- proactively recycle the CWS session before sending that command when the control path is stale;
- expose the decision and outcome through readiness, structured logs, and counters.

## 2. What Was Implemented

### 2.1 Control-path stale projection

A new computed control-path stale state is now projected from gateway health using:

- `cws_last_control_success_age_ms`
- `cws_last_tx_age_ms`

with configurable threshold:

- `control_path_stale_after_sec`

The stale decision is intentionally independent from:

- `readiness`
- `cws_authorized`
- ping/pong alone

### 2.2 Pre-send recycle for live limit entry path

For:

- `CommandAction::Place`
- with effective `IntentClass::Entry`

`command_consumer` now performs a stale check before send.

If the control path is stale and hardening is enabled:

1. log `control_path_stale_detected`
2. trigger a controlled CWS recycle
3. wait for:
   - fresh `cws_connection_instance_id`
   - `cws_authorized = true`
   - non-stale control-path projection
4. only then send the original command

If recycle does not complete in time:

- the live limit command is not sent into the stale session
- a controlled `CommandAck Error` is published with:
  - `error_code = control_path_recycle_failed`

### 2.3 Serialized recycle guard

`cws_client` now owns a serialized recycle guard so that stale-entry protection cannot launch multiple overlapping recycle attempts.

### 2.4 Observability additions

`/readiness` and `/debug/cws` now expose:

- `control_path_stale_after_sec`
- `control_path_stale`
- `control_path_stale_reason`
- `control_path_stale_for_ms`
- existing `cws_last_control_success_*`
- existing `cws_last_control_failure_*`
- existing `cws_last_tx_*`
- existing `cws_last_rx_*`

New counters added to health:

- `control_path_stale_detected_total`
- `control_path_recycle_total`
- `control_path_recycle_success_total`
- `control_path_recycle_failed_total`
- `control_path_stale_blocked_send_total`

Structured logs added:

- `control_path_stale_detected`
- `control_path_recycle_start`
- `control_path_recycle_success`
- `control_path_recycle_failed`
- `control_path_send_after_recycle`
- `control_path_send_blocked_due_to_stale`

## 3. Configs Added

Gateway config/env now supports:

- `control_path_stale_after_sec`
- `control_path_pre_entry_recycle_enabled`
- `control_path_recycle_timeout_ms`
- `control_path_hardening_log_only`

Recommended first-rollout values were also written explicitly into:

- `configs/gateway.sessiongap.live.7502MIW.toml`
- `configs/gateway.hybrid.live.7502SN6.toml`

Current configured values:

- `control_path_stale_after_sec = 900`
- `control_path_pre_entry_recycle_enabled = true`
- `control_path_recycle_timeout_ms = 5000`
- `control_path_hardening_log_only = false`

## 4. Files Changed

Primary implementation files:

- `alor-gateway/src/config.rs`
- `alor-gateway/src/cws_client.rs`
- `alor-gateway/src/health.rs`
- `alor-gateway/src/health_server.rs`
- `alor-gateway/src/services/command_consumer.rs`
- `alor-gateway/src/supervisor.rs`
- `configs/gateway.sessiongap.live.7502MIW.toml`
- `configs/gateway.hybrid.live.7502SN6.toml`

## 5. Acceptance Tests Executed Locally

Executed:

```bash
cargo test -p alor-gateway --lib --quiet
```

Result:

- `59 passed`

The local test set now covers:

- stale projection from health state
- no recycle on fresh control path
- successful recycle on stale path
- timeout/failure path with controlled block
- scope filter for `Entry + Place` only

## 6. Strongest Current Conclusion

The hardening has been implemented in the narrow form requested by `TZ 2.0`:

- no keepalive-based mitigation
- no strategy-level semantic change
- no market-path change
- proactive recycle only for stale live limit entry path

Operationally, this is aligned with the strongest `TZ 1.6` conclusion:

- cadence keepalive was insufficient
- reconnect/recycle before the first live limit order after long silence was the strongest workaround

## 7. Remaining Work

This document captures code-complete local hardening.

Still pending after this step:

1. deploy the new gateway build
2. run live acceptance checks:
   - fresh path `PASS`
   - stale path `recycle-before-send PASS`
   - optional recycle-failure simulation
3. collect the first rollout review bundle
