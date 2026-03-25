# TZ 1.5 Results: Idle / Control-Path Silence Aging

Date: 2026-03-25

Related documents:

- `docs/create-limit-tz1.4-results-2026-03-25.md`
- `docs/create-limit-tz1.4-preflight-and-activity-aging-2026-03-25.md`

## 1. Purpose

This note is the working result file for `TZ 1.5`.

The purpose of `TZ 1.5` is to test the narrower hypothesis that the residual intermittent limit-path incident is associated with:

- idle or quiet CWS control-path aging;
- or latent CWS/session degradation that is not surfaced early enough by the older readiness fields.

## 2. Implementation Completed First

Before running the next live experiments, the gateway and helper were extended with new telemetry and artifact capture.

Implemented in code:

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
- per-op control counters for:
  - `create:limit`
  - `delete:limit`
  - `replace:limit`
- recent inbound CWS frame ring buffer
- recent outbound CWS frame ring buffer
- new gateway debug endpoint:
  - `/debug/cws`

Implemented in helper:

- preflight now saves:
  - `/readiness`
  - `/debug/cws`
  - stream tails
  - gateway/runtime log tails
  - expanded summary fields
- pre/post capture now also saves `cws.debug.*.json`
- loop preflight compact line now includes:
  - `last_rx_age_ms`
  - `last_tx_age_ms`
  - `last_control_success_age_ms`
  - `last_control_failure_age_ms`
  - `request_map_size`
  - `oldest_pending_age_ms`

## 3. Files Updated

- `alor-gateway/src/health.rs`
- `alor-gateway/src/health_server.rs`
- `alor-gateway/src/cws_client.rs`
- `alor-gateway/src/services/command_consumer.rs`
- `scripts/limit_diag.sh`

Relevant local validation:

- `bash -n scripts/limit_diag.sh`
- `cargo test -p alor-gateway --lib --quiet`
- result:
  - `52 passed`

## 4. What This Enables Next

With these telemetry additions in place, the next `TZ 1.5` experiments can now compare:

1. `idle 20m / 30m / 45m`
2. `idle 30m + mid-window keepalive`
3. if feasible later, observation-only keepalive

with direct visibility into:

- whether the session was truly quiet before the probe;
- whether recent inbound/outbound CWS activity had gone silent;
- whether control success/failure ages were already abnormal before the first failing send;
- whether pending or request-map state was already drifting before `REPRO`.

## 5. Current Status

At this point in `TZ 1.5`:

- implementation is complete locally;
- validation is green locally;
- live `TZ 1.5` experiments are still pending.

## 6. Strongest Current Interim Conclusion

`TZ 1.5` telemetry is now in place to test the idle/control-path silence hypothesis more directly than before.

The next meaningful result will depend on whether:

- `idle 30m` or `45m` still reproduces;
- and whether a single safe mid-window keepalive materially changes that outcome.
