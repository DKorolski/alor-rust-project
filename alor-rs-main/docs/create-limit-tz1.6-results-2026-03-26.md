# TZ 1.6 Results: Final Validation Before Hardening

Date: 2026-03-26

Related documents:

- `docs/create-limit-tz1.5-results-2026-03-26.md`
- `docs/create-limit-review-submission-2026-03-25.md`
- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/README.md`

## 1. Purpose

`TZ 1.6` was the final analytic gate before hardening.

The practical question for this phase was:

- does regular cadence control activity keep the path healthy after a long quiet window;
- or is a proactive reconnect/recycle before the first limit command the more reliable workaround.

## 2. Baseline Entering TZ 1.6

The following should be treated as established going into this phase:

- `idle20 -> PASS`
- `idle30 -> REPRO`
- `idle30 + single keepalive@15m -> REPRO`
- safe active `create:limit -> delete:limit` loops do not themselves poison the path
- readiness and heartbeat can still look healthy before fail
- reconnect can return the path to a healthy immediate post-recovery state

## 3. Build And Test Line

Gateway diagnostic line used for `TZ 1.6`:

- `f1f91e1` `feat(diag): add cws idle-aging telemetry`
- `547be1c` `feat(diag): add tz1.6 cadence validation helper`

Gateway image on VPS:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-f1f91e1-diag-20260326`

Runtime line remained unchanged during the phase.

Stack under test:

- `sessiongap`

Test price used for passive probes:

- `79.00`

## 4. What Was Run

### 4.1 Experiment A: Baseline Idle 30m

Command:

```bash
/opt/limit_diag.sh tz16-baseline sessiongap 79.00 1800
```

Run directory:

- `/opt/diag-captures/20260326-190206`

### 4.2 Experiment B: Cadence Keepalive Every 10m

Command:

```bash
/opt/limit_diag.sh tz16-cadence sessiongap 79.00 600 1800
```

Run directory:

- `/opt/diag-captures/20260326-193241`

### 4.3 Experiment C: Cadence Keepalive Every 5m

Command:

```bash
/opt/limit_diag.sh tz16-cadence sessiongap 79.00 300 1800
```

Run directory:

- `/opt/diag-captures/20260326-200546`

### 4.4 Experiment D: Reconnect Before First Order

Command:

```bash
/opt/limit_diag.sh tz16-reconnect sessiongap 79.00 1800
```

Run directory:

- `/opt/diag-captures/20260326-203720`

## 5. Results

### 5.1 Baseline Idle 30m Reproduced Cleanly

Baseline preflight was operationally clean:

- `conn_id = 5e865502-8eae-4127-9ef4-02133ac6dc85`
- `conn_age_sec = 1808`
- `cws_reconnect_seq = 0`
- `cws_pending_count = 0`
- `request_map_size = 0`
- `last_rx_age_ms = 1807779`
- `last_tx_age_ms = 1807797`
- `last_control_success_age_ms = na`
- `limit_send_total = 0`
- `limit_error_total = 0`

Main probe:

- `request_id = d71b55d4-f019-4d47-8686-98c539d0137e`

Outcome:

- `status = error`
- `error_code = cws_error`
- `error_msg = protocol_reset_without_close_handshake`
- `broker_order_id = null`

Interpretation:

- the `idle30` baseline remained consistent with `TZ 1.5`;
- the failure still presented as immediate transport/control-path failure on the first `create:limit` send.

### 5.2 Cadence Every 10m Did Not Prevent Failure

Both keepalive cycles passed cleanly.

Keepalive at `10m`:

- `place request_id = 02fef0ab-fb7b-43d2-98b3-96ec1a0983ab`
- `order_id = 2023555935792409313`
- `cancel request_id = aa0e4997-553e-4838-a9c2-2ab95d1f36d7`
- result: `accepted -> working -> canceled`, `filled = 0.0`

Keepalive at `20m`:

- `place request_id = 503f0d32-72ca-4a46-99bb-c2107edc87b7`
- `order_id = 2023555935792412195`
- `cancel request_id = 985e130c-7e00-4a83-a2f4-5e0fb9efa224`
- result: `accepted -> working -> canceled`, `filled = 0.0`

Main-probe preflight still looked healthy:

- `conn_id = b1f6f72f-aca3-4e13-b528-a5f7171edcc1`
- `conn_age_sec = 1846`
- `cws_reconnect_seq = 0`
- `cws_pending_count = 0`
- `request_map_size = 2`
- `last_rx_age_ms = 600543`
- `last_tx_age_ms = 600558`
- `last_control_success_age_ms = 600543`
- `limit_send_total = 2`
- `limit_error_total = 0`

Main probe:

- `request_id = dfcac72a-5057-43dd-ba2a-789c78b1bd19`

Outcome:

- `status = error`
- `error_code = cws_error`
- `error_msg = protocol_reset_without_close_handshake`
- `broker_order_id = null`

Interpretation:

- a `10m` cadence of successful safe control activity did not materially eliminate the residual incident.

### 5.3 Cadence Every 5m Also Did Not Prevent Failure

All five keepalive cycles passed cleanly.

Successful keepalive places:

- `748ab952-c377-4f1f-9f52-a2b577e212b0`
- `6a76fa2e-a735-43cd-8a53-8fada12b6ab9`
- `93682f3f-9e79-4893-ac11-bbb9945a7fd5`
- `035cfde3-3f6e-475b-b855-bd28fc403865`
- `eaffdb24-e481-42c8-ba7d-e6d933e71191`

All matching cancels were accepted and all orders ended `canceled` with `filled = 0.0`.

Main-probe preflight remained healthy:

- `conn_id = a4a72b4d-8967-4fe4-8b1f-d1aef0685156`
- `conn_age_sec = 1841`
- `cws_reconnect_seq = 0`
- `cws_pending_count = 0`
- `request_map_size = 5`
- `last_rx_age_ms = 300769`
- `last_tx_age_ms = 300860`
- `last_control_success_age_ms = 300769`
- `limit_send_total = 5`
- `limit_error_total = 0`

Main probe:

- `request_id = 7605face-00fb-4822-b5e4-29c0f827dc11`

Outcome:

- `status = error`
- `error_code = cws_error`
- `error_msg = protocol_reset_without_close_handshake`
- `broker_order_id = null`

Interpretation:

- even a denser `5m` cadence of successful safe `create:limit -> delete:limit` activity did not keep the `30m` main probe healthy.

### 5.4 Reconnect Before First Order Passed Cleanly

Before controlled recycle:

- `conn_id = a0a08080-3dc6-49c8-b542-5c559b9cf334`
- `conn_age_sec = 1845`
- `cws_reconnect_seq = 0`
- `cws_pending_count = 0`
- `request_map_size = 0`
- `last_rx_age_ms = 1844770`
- `last_tx_age_ms = 1845411`

After reconnect:

- new `conn_id = f8842c37-e393-4b0f-aea8-927401f1c2e9`
- `conn_age_sec = 4`
- `cws_pending_count = 0`
- `request_map_size = 0`
- new `access_token = sha256:5275ce2cdce2f9b0`

Main probe:

- `place request_id = f8cae750-182a-4247-95c1-dda5dae4670f`
- `order_id = 2023555935792429672`
- `cancel request_id = fed3413b-d459-426a-925b-058911a8cebc`

Outcome:

- `place accepted -> working`
- `cancel accepted -> canceled`
- `filled = 0.0`
- overall scenario result: `PASS`

Interpretation:

- a proactive controlled recycle before the first limit command after the long idle window restored the path to a clean working state.

## 6. Comparison Table

| Scenario | Connection age at main probe | Control cadence before main probe | Main probe result | Key note |
| --- | ---: | --- | --- | --- |
| `idle30 baseline` | `1808s` | none | `FAIL` | first `create:limit` hit immediate `cws_error` |
| `idle30 + keepalive every 10m` | `1846s` | `10m`, `20m` | `FAIL` | both keepalives passed, main probe still failed |
| `idle30 + keepalive every 5m` | `1841s` | `5/10/15/20/25m` | `FAIL` | five keepalives passed, main probe still failed |
| `idle30 + reconnect before order` | `6s` on new conn | controlled recycle at end of idle window | `PASS` | `place -> cancel` passed cleanly |

## 7. Strongest Current Conclusion

`TZ 1.6` resolves the practical question in favor of `Variant 2` from the task definition.

Current strongest conclusion:

- regular safe cadence keepalive is insufficient;
- even `5m` cadence did not materially eliminate the residual incident class;
- the strongest current operational workaround is a proactive gateway/CWS reconnect or recycle immediately before the first live `limit` command after a long quiet window.

This also tightens the interpretation of the `idle/control-path degradation` hypothesis:

- the problem is not cured simply by sprinkling occasional successful `create/delete` traffic into the idle window;
- the later failing state appears compatible with stale long-lived connection state that is cleared more reliably by reconnect than by cadence activity.

## 8. Recommended Hardening Path

Recommended next step:

1. treat the research phase as sufficiently narrowed;
2. move to hardening with a deliberate pre-order recycle/reconnect strategy after long idle windows;
3. keep the new `TZ 1.5` / `TZ 1.6` telemetry in place while the hardening change is validated.

Most practical hardening direction from the evidence collected so far:

- before the first live `create:limit` after a long quiet period, proactively recycle the gateway/CWS session and wait for a fresh ready state.

## 9. Review Bundle

Curated review artifacts are stored alongside this report:

- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/README.md`
- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/idle30-baseline/`
- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/idle30-keepalive-10m/`
- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/idle30-keepalive-5m/`
- `docs/create-limit-tz1.6-results-2026-03-26-artifacts/idle30-reconnect-before-order/`
