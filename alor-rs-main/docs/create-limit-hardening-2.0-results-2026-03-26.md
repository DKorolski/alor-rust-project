# Hardening 2.0 Results: Pre-Send Recycle For Idle-Time CWS Control-Path Degradation

Date: 2026-03-26

Related documents:

- `docs/create-limit-tz1.6-results-2026-03-26.md`
- `docs/create-limit-review-submission-2026-03-25.md`
- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-hardening-2.0-rollout-runbook-2026-03-26.md`
- `docs/create-limit-hardening-2.0-results-2026-03-26-artifacts/README.md`

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

## 6. Live Rollout And Acceptance

The hardening line was rolled out on `2026-03-26` as:

- gateway image: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-774b917-diag-20260326`

Rollout scope:

- only `alor-gateway` was recreated
- `sessiongap` and `hybrid` were both updated
- `strategy-runtime` was not recreated

Post-rollout readiness on both stacks exposed the new `TZ 2.0` fields:

- `control_path_stale`
- `control_path_stale_reason`
- `control_path_stale_for_ms`
- `control_path_stale_detected_total`
- `control_path_recycle_total`
- `control_path_recycle_success_total`
- `control_path_recycle_failed_total`
- `control_path_stale_blocked_send_total`

### 6.1 Fresh path passed without recycle

Run directory:

- `/opt/diag-captures/20260326-225749`

Observed:

- fresh `sessiongap` passive `create:limit -> delete:limit` completed cleanly
- `place`:
  - `request_id = 20f9cb69-ac40-41eb-8752-3a56ea7eec94`
  - `broker_order_id = 2023555935792437604`
  - `accepted -> working`
- `cancel`:
  - `request_id = 390f4ea6-8489-4a30-a93d-aba37ce5710d`
  - `accepted -> canceled`
  - `filled = 0.0`

No fresh-path hardening events were emitted:

- no `control_path_stale_detected`
- no `control_path_recycle_start`
- no `control_path_send_after_recycle`

Interpretation:

- fresh entry path remained unchanged operationally;
- the hardening did not introduce unnecessary recycle on healthy fresh control path.

### 6.2 Stale path passed via recycle-before-send

Run directory:

- `/opt/diag-captures/20260326-225923`

Pre-send stale baseline:

- `CWS_CONNECTION_INSTANCE_ID = 0fd446a4-0eae-4707-9a39-b480fb20230d`
- `CWS_CONNECTION_AGE_SEC = 1052`
- `CWS_LAST_TX_AGE_MS = 1052133`
- `CWS_LAST_CONTROL_SUCCESS_AGE_MS = na`
- `CWS_PENDING_COUNT = 0`
- `REQUEST_MAP_SIZE = 0`

First stale-path `place`:

- `request_id = f0e60be3-0778-4459-8023-99846893c015`
- `broker_order_id = 2023555935792442183`

Hardening logs:

- `control_path_stale_detected`
- `control_path_recycle_start`
- `control_path_recycle_success`
- `control_path_send_after_recycle`

Connection switch:

- previous `cws_connection_instance_id = 0fd446a4-0eae-4707-9a39-b480fb20230d`
- fresh `cws_connection_instance_id = 53c26739-3775-4f67-9998-84145da78e9f`

Order lifecycle after recycle:

- `place accepted -> working`
- `cancel request_id = 3fe9596f-2933-4224-ab84-34b1c9ef3d1e`
- `cancel accepted -> canceled`
- `filled = 0.0`

Counters after run:

- `control_path_stale_detected_total = 1`
- `control_path_recycle_total = 1`
- `control_path_recycle_success_total = 1`
- `control_path_recycle_failed_total = 0`
- `control_path_stale_blocked_send_total = 0`

Interpretation:

- the stale path no longer sent blindly into the aged control session;
- the live run validated the intended `recycle-before-send` behavior.

### 6.3 Market path regression passed

Run directory:

- `/opt/diag-captures/20260326-231806-hybrid-market`

Stack:

- `hybrid` paper

Observed:

- market buy:
  - `request_id = 7eed03c0-115c-4536-82a6-28934287e414`
  - `broker_order_id = 2033126085000190190`
  - `filled`
- market sell:
  - `request_id = 996824f7-4515-49f9-81b3-fd83da5328b1`
  - `broker_order_id = 2033126085000190238`
  - `filled`

No hardening recycle events were emitted on the market path.

Interpretation:

- `market` remained out of scope operationally, as intended;
- no fresh regression was introduced into market order routing.

### 6.4 Operational note: one duplicate manual entry was operator-induced

During the first fresh-path acceptance, two separate manual `create:limit` commands were sent:

- `f4711fbe-6174-4cc0-8eb5-57ec907f0bf8`
- `20f9cb69-ac40-41eb-8752-3a56ea7eec94`

These appeared in `cmd.orders.7502MIW` as two distinct stream messages and produced two distinct broker orders:

- `2023555935792437596`
- `2023555935792437604`

This was not a gateway-side duplicate of one request. It was an operator-induced double send during an ambiguous acceptance run. The lingering first order was later canceled successfully:

- cancel request:
  - `2bda3071-0ccb-43d1-a24a-d4f82509d317`
- terminal state:
  - `2023555935792437596 status = canceled`
  - `filled = 0.0`

Interpretation:

- no new hardening bug is indicated by this duplicate;
- the acceptance runbook should avoid `loop 1` style flows that can encourage accidental manual re-entry while a previous helper invocation is still unresolved.

## 7. Strongest Current Conclusion

The hardening has been implemented in the narrow form requested by `TZ 2.0`:

- no keepalive-based mitigation
- no strategy-level semantic change
- no market-path change
- proactive recycle only for stale live limit entry path

Operationally, this is aligned with the strongest `TZ 1.6` conclusion:

- cadence keepalive was insufficient
- reconnect/recycle before the first live limit order after long silence was the strongest workaround

The live rollout now supports the same conclusion directly:

- fresh path passes without recycle;
- stale path passes via explicit recycle-before-send;
- market path stays unaffected.

## 8. Remaining Work

This document now captures both code-complete hardening and first live acceptance.

Still optional after this step:

1. run controlled recycle-failure simulation:
   - expect `control_path_recycle_failed`
   - expect blocked send with controlled error ack;
2. decide whether to keep `TZ 2.0` as the first production hardening line or layer an additional operational wrapper around it.
