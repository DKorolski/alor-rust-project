# Create Limit Topology Diagnostic Runbook

Date: 2026-03-23

## 1. Purpose

This runbook is the next diagnostic phase after the confirmed comparative matrix:

- `M @ T2 = PASS`
- `L1 @ T2 = REPRO`
- `L2 @ T2 = REPRO`
- `L2 @ T1 = PASS`
- `L2 @ T3 = REPRO`

The current goal is narrower than the earlier live review.
We are no longer asking whether `create:limit` can fail.
We are asking which topology/session variable most likely drives the residual failure.

Primary questions:

1. Does the failure depend on multiple concurrent CWS sessions under the same broker identity?
2. Is the failure more likely immediately after reconnect than on an aged stable connection?
3. Does the failure persist if the second stack is present but idle?
4. If a different auth principal is available, does the problem disappear when topology stays the same but the principal changes?

## 2. Working Hypothesis

Current strongest hypothesis:

- shared `create:limit` / CWS transport issue
- with material topology/coexistence sensitivity

This runbook is designed to separate:

- same-principal session competition
- reconnect/send race
- other shared-session transport sensitivity

## 3. Required Telemetry

Before running this matrix, deploy a gateway build that includes:

- `stack_name`
- `gateway_instance_id`
- `auth_principal_fingerprint`
- `connection_age_ms`
- `time_since_last_reconnect_ms`
- `in_flight_pending_count`
- `cws_last_connect_ts_utc`
- `cws_last_transport_failure_ts_utc`
- `cws_last_limit_send_ts_utc`
- `cws_last_limit_error_ts_utc`
- `cws_last_successful_send_ts_utc`
- `cws_last_successful_ack_ts_utc`

These fields should appear in:

- `/readiness`
- `cws_limit_send`
- `cws_transport_failure`
- `cws_fail_pending`

## 4. Stack Naming

Set explicit stack labels before each run:

- `ALOR_STACK_NAME=sessiongap`
- `ALOR_STACK_NAME=hybrid`

This is required so that logs and readiness snapshots remain comparable across runs.

## 5. Required Artifacts

Capture and keep for every run:

- `sessiongap` readiness snapshot before the probe
- `hybrid` readiness snapshot before the probe, if `hybrid` is up
- `sessiongap` readiness snapshot after the probe
- `hybrid` readiness snapshot after the probe, if `hybrid` is up
- `cmd.orders.<portfolio>`
- `cmd.acks.<portfolio>`
- `broker.orders.<portfolio>`
- `broker.positions.<portfolio>`
- gateway logs containing:
  - `command received`
  - `cws_limit_send`
  - `cws_transport_failure`
  - `cws_fail_pending`
  - `command ack published`

Correlation keys to keep for each run:

- `request_id`
- `cws_request_guid`
- `broker_order_id`
- `stack_name`
- `gateway_instance_id`
- `auth_principal_fingerprint`
- `cws_connection_instance_id`
- `connect_seq`
- `reconnect_seq`

## 6. Common Preflight

Use the same preflight before every test group.

### 6.1 Confirm health

```bash
docker compose -p sessiongap exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness
```

If `hybrid` is expected to be up:

```bash
docker compose -p hybrid exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness
```

Expected:

- `readiness=true`
- `ws_connected=true`
- `cws_authorized=true`
- `stack_name` matches the intended stack

### 6.2 Check principal identity

Compare `auth_principal_fingerprint` across the stacks.

Interpretation:

- same fingerprint: same-principal topology test
- different fingerprint: different-principal topology test

### 6.3 Confirm clean order/position baseline

```bash
docker compose -p sessiongap exec -T redis redis-cli --raw \
  XREVRANGE broker.positions.7502MIW + - COUNT 10

docker compose -p sessiongap exec -T redis redis-cli --raw \
  XREVRANGE broker.orders.7502MIW + - COUNT 10
```

Expected:

- no unintended open position
- no active order left from prior probes

### 6.4 Get a fresh bar for passive pricing

```bash
docker compose -p sessiongap exec -T redis redis-cli --raw \
  XREVRANGE md.bars.7502MIW.1m + - COUNT 3
```

Choose a passive buy below market, for example `close - 0.30`, unless current market conditions require a larger gap.

## 7. Test Group P1: Same Principal, Second Stack Idle

This is the highest-priority next probe.

### 7.1 Topology

Goal:

- both `sessiongap` and `hybrid` gateways are up
- `hybrid` exists as a second live CWS client
- `hybrid` does not actively submit commands during the probe

Recommended practical setup:

- keep `sessiongap` gateway up
- keep `hybrid` gateway up
- stop `hybrid` strategy-runtime if needed to reduce noise
- do not inject any manual commands into `hybrid`

### 7.2 Capture preflight snapshots

```bash
docker compose -p sessiongap exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness

docker compose -p hybrid exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness
```

What to compare immediately:

- `auth_principal_fingerprint`
- `stack_name`
- `gateway_instance_id`
- `cws_connection_instance_id`
- `connect_seq`
- `reconnect_seq`
- `cws_last_connect_ts_utc`

### 7.3 Send one passive `L2` create command on `sessiongap`

Template:

```bash
REQ_ID=$(cat /proc/sys/kernel/random/uuid)
TS=$(date +%s)
PASSIVE_PRICE=<close_minus_0.30_or_similar>

PLACE_PAYLOAD=$(cat <<JSON
{"schema_version":1,"ts_utc":$TS,"source":"manual-l2-p1","msg_type":"command","payload":{"request_id":"$REQ_ID","created_ts_utc":$TS,"strategy_id":"manual.limit.l2.p1","portfolio":"7502MIW","exchange":"MOEX","symbol":"USDRUBF","action":{"place":{"price":$PASSIVE_PRICE,"qty":1.0,"side":"buy","comment":"l2_p1_$REQ_ID"}},"intent_class":"entry","ttl_ms":600000}}
JSON
)

docker compose -p sessiongap exec -T redis redis-cli \
  XADD cmd.orders.7502MIW "*" payload "$PLACE_PAYLOAD"

echo "REQ_ID=$REQ_ID"
```

### 7.4 Capture artifacts

```bash
docker compose -p sessiongap exec -T redis redis-cli --raw \
  XREVRANGE cmd.acks.7502MIW + - COUNT 20 | grep -A8 -B3 "$REQ_ID"

docker compose -p sessiongap logs --since=5m alor-gateway | \
  grep -E "$REQ_ID|cws_limit_send|cws_transport_failure|cws_fail_pending|command ack published"

docker compose -p sessiongap exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness

docker compose -p hybrid exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness
```

### 7.5 Interpretation

If this reproduces the reset while both stacks show the same `auth_principal_fingerprint`, the same-principal coexistence hypothesis becomes much stronger.

If this passes cleanly, coexistence alone is less likely to be sufficient, and reconnect timing or intermittent broker behavior remains in scope.

## 8. Test Group P2: Different Principal, Same Topology

Run this only if a second valid auth principal is operationally available.

Goal:

- keep the same topology as `P1`
- change only the broker auth identity of the second stack

Expected value:

- if `P1` reproduces and `P2` passes, this is strong evidence for same-principal session competition

Artifacts and commands should match `P1` exactly, with the only deliberate change being the second stack principal.

## 9. Test Group R: Reconnect Timing

This group checks whether `create:limit` failures cluster near reconnect.

### R1: Immediate after reconnect

Trigger or wait for a reconnect, then send the passive `L2` probe within roughly `1-5` seconds after `cws_last_connect_ts_utc` updates.

### R2: Short stable session

Wait `30-60` seconds after reconnect, then send the same probe.

### R3: Aged stable session

Wait several minutes after reconnect, then send the same probe.

For each run compare:

- `connection_age_ms`
- `time_since_last_reconnect_ms`
- `cws_last_successful_send_ts_utc`
- `cws_last_successful_ack_ts_utc`
- `in_flight_pending_count`

Interpretation:

- repeated failures in `R1` but not `R2/R3` strongly suggest reconnect/send race
- failures spread across all three windows suggest a broader topology/session factor

## 10. Classification Table

### Outcome A

- `P1` repro
- `P2` pass

Interpretation:

- strongest evidence for same-principal session competition

### Outcome B

- `P1` repro
- `P2` unavailable
- `R1` much worse than `R2/R3`

Interpretation:

- reconnect/send race is a leading candidate

### Outcome C

- `P1` repro
- `R1/R2/R3` mixed
- second stack really idle

Interpretation:

- coexistence matters, but the exact trigger remains open

### Outcome D

- `P1` pass repeatedly

Interpretation:

- topology effect is not deterministic enough yet
- continue with repeated probes and timing comparison before changing code

## 11. Operational Guardrails

- Do not run a broad new live campaign during this phase.
- Keep probes narrow and low-frequency.
- Use passive limits unless a narrower hypothesis explicitly requires marketable behavior.
- Keep `hybrid` recovery/ownership work separate from this track.
- If any passive order is accepted, either cancel it cleanly or verify terminal state before the next probe.

## 12. Acceptance For This Phase

This phase is successful if it narrows the incident class to one of these buckets:

1. same-principal session competition
2. reconnect/send race
3. topology-sensitive but still not principal-specific shared-session issue
4. still intermittent enough that a repeated probe series is required before code changes

At that point the next step should be a targeted fix or a broker-side escalation, not another broad exploratory rerun.
