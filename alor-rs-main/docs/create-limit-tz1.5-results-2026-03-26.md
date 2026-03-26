# TZ 1.5 Results: Idle / Control-Path Silence Aging

Date: 2026-03-26

Related documents:

- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-review-submission-2026-03-25.md`
- `docs/create-limit-tz1.4-preflight-and-activity-aging-2026-03-25.md`
- `docs/create-limit-tz1.4-results-2026-03-25.md`
- `docs/create-limit-tz1.5-results-2026-03-25.md`

Review bundle:

- `docs/create-limit-tz1.5-results-2026-03-26-artifacts/README.md`

## 1. Purpose

This note records the live results for `TZ 1.5`.

The goal of `TZ 1.5` was to test the narrower hypothesis that the residual intermittent `create:limit` incident is associated with:

- idle or mostly quiet CWS control-path aging;
- longer-lived latent CWS/session degradation not reflected early enough in the older readiness fields;
- and whether a small safe mid-window keepalive can materially reduce that `REPRO`.

All runs in this document were executed on the `sessiongap` live stack after deployment of the `TZ 1.5` telemetry line.

## 2. Baseline Carried Forward

The following should be treated as already established before this phase:

- passive `create:limit -> delete:limit` loops can pass cleanly;
- active aging `5 / 10 / 15` passive cycles passed cleanly;
- `idle ~30m` had already produced a clean `create:limit REPRO`;
- preflight immediately before the earlier idle fail was operationally clean;
- after reconnect, the path can recover and pass immediately again;
- the simple hypothesis that ordinary safe `create/delete` activity itself causes the failure had already been weakened materially.

## 3. Telemetry Used In This Phase

Gateway line deployed on VPS:

- `f1f91e1` `feat(diag): add cws idle-aging telemetry`

Gateway image:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-f1f91e1-diag-20260326`

The `TZ 1.5` line added or exposed:

- `cws_last_rx_ts_utc`
- `cws_last_rx_age_ms`
- `cws_last_tx_ts_utc`
- `cws_last_tx_age_ms`
- `cws_last_control_success_ts_utc`
- `cws_last_control_success_age_ms`
- `cws_last_control_failure_ts_utc`
- `cws_last_control_failure_age_ms`
- `cws_last_ping_ts_utc`
- `cws_last_pong_ts_utc`
- `cws_last_ping_pong_age_ms`
- `cws_pending_guids`
- `cws_oldest_pending_age_ms`
- `request_map_size`
- per-op counters for:
  - `create:limit`
  - `delete:limit`
  - `replace:limit`
- `/debug/cws`
- compact preflight snapshots in `limit_diag.sh`

## 4. Experiments And Results

## 4.1 Idle 20m Control Probe

Capture:

- `/opt/diag-captures/20260326-100944`

Preflight immediately before the probe was clean:

- `READINESS=true`
- `CWS_AUTHORIZED=true`
- `CWS_CONNECTION_INSTANCE_ID=48b87d1a-00d8-444f-bacf-e60441b4d327`
- `CWS_CONNECTION_AGE_SEC=1289`
- `CWS_LAST_RX_AGE_MS=1288187`
- `CWS_LAST_TX_AGE_MS=1288596`
- `CWS_PENDING_COUNT=0`
- `REQUEST_MAP_SIZE=0`
- `CWS_PROTOCOL_RESET_TOTAL=0`

Probe:

- `place request_id=815c9781-56f6-4c5f-998b-f4306b37c7ac`
- `broker_order_id=2023555935791832109`
- `cancel request_id=85eb900f-61a4-4774-8d5c-5656b601c2c4`

Observed behavior:

- `place accepted`
- order reached `working`
- `cancel accepted`
- order reached `canceled`
- `filled=0.0`

Post-state remained healthy:

- `readiness=true`
- `cws_authorized=true`
- `cws_protocol_reset_total=0`
- `cws_limit_error_total=0`

Conclusion:

- `idle 20m` did not reproduce the incident.

## 4.2 Idle 30m Control Probe

Capture:

- `/opt/diag-captures/20260326-103237`

Preflight immediately before the probe was also clean:

- `READINESS=true`
- `CWS_AUTHORIZED=true`
- `CWS_CONNECTION_INSTANCE_ID=7ac90405-adb0-4042-9816-18b068cb94c2`
- `CWS_CONNECTION_AGE_SEC=1887`
- `CWS_LAST_RX_AGE_MS=1886805`
- `CWS_LAST_TX_AGE_MS=1886810`
- `CWS_PENDING_COUNT=0`
- `REQUEST_MAP_SIZE=0`
- `CWS_PROTOCOL_RESET_TOTAL=0`
- `CWS_LAST_CONTROL_SUCCESS_AGE_MS=na`
- `CWS_LAST_CONTROL_FAILURE_AGE_MS=na`

Probe:

- `place request_id=c884803f-3f3c-49aa-8ac9-d543ec5b0027`

Observed behavior:

- first `create:limit` send failed immediately;
- `status=error`
- `error_code=cws_error`
- `error_msg=protocol_reset_without_close_handshake`
- `broker_order_id=null`

Gateway trace showed:

- `cws send opcode=create:limit`
- immediate `cws_transport_failure`
- immediate `cws_fail_pending`
- then `command ack published status=Error`

Conclusion:

- `idle 30m` reproduced cleanly on the first `create:limit`.

## 4.3 Idle 30m With Mid-Window Keepalive

Capture:

- `/opt/diag-captures/20260326-114756`

### 4.3.1 Keepalive At ~15m

Preflight before keepalive:

- `READINESS=true`
- `CWS_AUTHORIZED=true`
- `CWS_CONNECTION_INSTANCE_ID=fbfeccce-33d0-4843-9bad-c6f477821f7a`
- `CWS_CONNECTION_AGE_SEC=912`
- `CWS_LAST_RX_AGE_MS=912164`
- `CWS_PENDING_COUNT=0`
- `REQUEST_MAP_SIZE=0`

Keepalive operations:

- `place request_id=0d3842d9-593b-4d71-b6c2-97ab9e83c9ae`
- `broker_order_id=2023555935791931457`
- `cancel request_id=b0d3e63c-27b4-4357-a3c1-2b9fec8f82e2`

Observed behavior:

- keepalive `place accepted`
- order reached `working`
- keepalive `cancel accepted`
- order reached `canceled`
- `filled=0.0`

### 4.3.2 Main Probe At ~30m From Same Restart

Preflight before the main probe:

- `READINESS=true`
- `CWS_AUTHORIZED=true`
- same `CWS_CONNECTION_INSTANCE_ID=fbfeccce-33d0-4843-9bad-c6f477821f7a`
- `CWS_CONNECTION_AGE_SEC=1815`
- `CWS_LAST_RX_AGE_MS=900114`
- `CWS_LAST_TX_AGE_MS=900126`
- `CWS_LAST_CONTROL_SUCCESS_TS_UTC=1774515792`
- `CWS_LAST_CONTROL_SUCCESS_AGE_MS=900114`
- `CWS_CREATE_LIMIT_SUCCESS_TOTAL=1`
- `CWS_DELETE_LIMIT_SUCCESS_TOTAL=1`
- `CWS_PENDING_COUNT=0`
- `REQUEST_MAP_SIZE=1`
- `CWS_PROTOCOL_RESET_TOTAL=0`

Main probe:

- `place request_id=1680e796-0a44-4d4a-8e54-a398c97af8f4`

Observed behavior:

- first `create:limit` still failed;
- `status=error`
- `error_code=cws_error`
- `error_msg=protocol_reset_without_close_handshake`
- `broker_order_id=null`

Immediate post-fail snapshot captured the reconnect in flight:

- `readiness=true`
- `cws_authorized=false`
- `cws_reconnect_seq=1`
- `cws_protocol_reset_total=1`
- `cws_limit_error_total=1`
- `cws_pending_failed_total=1`
- `cws_create_limit_failure_total=1`
- `cws_last_control_failure_ts_utc=1774516694`

Follow-up after recovery showed:

- `readiness=true`
- `cws_authorized=true`
- new `cws_connection_instance_id=47f3d09f-b47c-4fb6-9ea9-ad4bc1f5b049`

Conclusion:

- one successful safe mid-window keepalive at ~15m did not prevent the later ~30m `create:limit REPRO`.

## 5. Comparison

| Case | Connection age before main probe | `last_rx_age_ms` before main probe | `last_control_success_age_ms` before main probe | Pending / request map | Result |
| --- | ---: | ---: | ---: | --- | --- |
| `idle 20m` | `1289s` | `1288187` | `na` | `pending=0`, `request_map=0` | `PASS` |
| `idle 30m` | `1887s` | `1886805` | `na` | `pending=0`, `request_map=0` | `FAIL` |
| `idle 30m + keepalive@15m` | `1815s` | `900114` | `900114` | `pending=0`, `request_map=1` | `FAIL` |

## 6. Strongest Current Conclusion

The `TZ 1.5` results materially strengthen the following reading:

- `idle 20m` does not by itself reproduce;
- `idle 30m` does reproduce on the first `create:limit`;
- a single successful safe keepalive at ~15m does not materially eliminate that later `REPRO`.

This narrows the hypothesis further.

The best current framing is no longer:

- simple wall-clock decay after restart;
- nor pure absence of all control activity since connect.

The current stronger framing is:

- the incident correlates with longer-lived, mostly idle CWS/control-path session state;
- and one safe successful `place -> cancel` in the middle of the window is insufficient to reset or prevent that latent failure.

Another important outcome is that the expanded `TZ 1.5` readiness/debug telemetry did improve post-fail explanation, but it still did not expose a clear deterministic latent fail marker before the failing send.

Immediately before both failing probes, preflight was still operationally clean by:

- readiness;
- authorization;
- pending count;
- request-map size in the pure idle case;
- and protocol-reset counters.

## 7. Review Bundle

Compact review artifacts are stored next to this report:

- `docs/create-limit-tz1.5-results-2026-03-26-artifacts/`

That bundle contains, for each key run:

- preflight summary
- preflight readiness snapshot
- preflight `/debug/cws`
- post readiness snapshot
- post `/debug/cws`
- gateway post log

The bundle is intended to be small enough for convenient review while still preserving the evidence behind the conclusions above.
