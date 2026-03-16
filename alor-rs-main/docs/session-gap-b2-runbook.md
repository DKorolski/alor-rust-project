# Session Gap B2 Runbook

This runbook verifies the full `session_gap` lifecycle after the residual CWS transport-failure fix:
- marketable limit entry,
- fill confirmation,
- controlled flatten,
- return to `Flat` without orphan state.

## Preconditions
- Deployment contains the transport observability fix for `cws_guid` preservation and `cws_transport_failure` logging.
- `alor-gateway` and `strategy-runtime` are both healthy.
- Runtime is in `LiveReady` and `live_guard=ALLOWED`.
- Redis streams are reachable.
- Operator is ready to flatten immediately if fill behavior diverges from the plan.

## Required artifacts
Capture and keep:
- gateway logs,
- runtime logs,
- `cmd.orders.<portfolio>`,
- `cmd.acks.<portfolio>`,
- `broker.orders.<portfolio>`,
- `broker.positions.<portfolio>`,
- `runtime.state.session_gap_standalone.live.<portfolio>`,
- request correlation:
  - `request_id`,
  - `cws_guid`,
  - `broker_order_id`.

## Step 1: Preflight
Confirm health before sending commands:

```bash
docker compose -p sessiongap exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness
docker compose -p sessiongap exec -T strategy-runtime curl -s http://127.0.0.1:8091/readiness
docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE runtime.state.session_gap_standalone.live.7502MIW + - COUNT 1
```

Expected:
- gateway `readiness=true`,
- runtime `readiness=true`,
- `live_guard=ALLOWED`,
- runtime state `phase="Flat"`.

## Step 2: Send marketable limit entry
Use the production command path with the smallest safe size.
Choose a limit price that should cross the book immediately but still respects exchange price limits.

Example template:

```bash
REQ_ID=$(cat /proc/sys/kernel/random/uuid)
TS=$(date +%s)

PLACE_PAYLOAD=$(cat <<JSON
{"schema_version":1,"ts_utc":$TS,"source":"manual-b2","msg_type":"command","payload":{"request_id":"$REQ_ID","created_ts_utc":$TS,"strategy_id":"manual.limit.b2","portfolio":"7502MIW","exchange":"MOEX","symbol":"USDRUBF","action":{"place":{"price":<MARKETABLE_PRICE>,"qty":1.0,"side":"buy","comment":"b2_$REQ_ID"}},"intent_class":"entry","ttl_ms":600000}}
JSON
)

docker compose -p sessiongap exec -T redis redis-cli XADD cmd.orders.7502MIW "*" payload "$PLACE_PAYLOAD"
echo "REQ_ID=$REQ_ID"
```

## Step 3: Confirm accept and fill
Collect the command ack, gateway send/ack logs, and order/position effects.

```bash
docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE cmd.acks.7502MIW + - COUNT 200 | grep -A3 -B3 "$REQ_ID"

docker compose -p sessiongap logs --since=10m alor-gateway | \
grep -E "$REQ_ID|cws_limit_send|cws_limit_ack|cws_transport_failure|command ack published"

docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE broker.orders.7502MIW + - COUNT 20

docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE broker.positions.7502MIW + - COUNT 20
```

Expected:
- `cmd.ack` is `accepted`,
- `broker_order_id != null`,
- an order event appears for the same symbol,
- a position opens,
- no `ack_failed`,
- no `Blocked` tail.

## Step 4: Controlled flatten
Send the exit command through the same production path.
Use the team-approved flatten method for `session_gap` in the current environment.

If flatten is a market order:

```bash
EXIT_REQ_ID=$(cat /proc/sys/kernel/random/uuid)
TS2=$(date +%s)

EXIT_PAYLOAD=$(cat <<JSON
{"schema_version":1,"ts_utc":$TS2,"source":"manual-b2","msg_type":"command","payload":{"request_id":"$EXIT_REQ_ID","created_ts_utc":$TS2,"strategy_id":"manual.limit.b2.exit","portfolio":"7502MIW","exchange":"MOEX","symbol":"USDRUBF","action":{"market":{"qty":1.0,"side":"sell","comment":"b2_exit_$EXIT_REQ_ID"}},"intent_class":"exit","ttl_ms":600000}}
JSON
)

docker compose -p sessiongap exec -T redis redis-cli XADD cmd.orders.7502MIW "*" payload "$EXIT_PAYLOAD"
echo "EXIT_REQ_ID=$EXIT_REQ_ID"
```

If flatten in your environment uses another command type, substitute the approved exit command but keep the same evidence collection.

## Step 5: Verify flat / no orphan state

```bash
docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE cmd.acks.7502MIW + - COUNT 200 | grep -A3 -B3 "$EXIT_REQ_ID"

docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE broker.orders.7502MIW + - COUNT 20

docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE broker.positions.7502MIW + - COUNT 20

docker compose -p sessiongap exec -T redis redis-cli --raw \
XREVRANGE runtime.state.session_gap_standalone.live.7502MIW + - COUNT 1
```

Expected:
- exit ack accepted,
- position returns to zero,
- no active orphan order remains,
- runtime state returns to `phase="Flat"`,
- no `Blocked`,
- no `ack_failed`.

## Failure handling
If a transport incident occurs:
- keep the exact `request_id`,
- capture the matching `cws_transport_failure` and `cws_fail_pending` logs,
- capture the published `cmd.ack`,
- verify whether `cws_request_guid` was preserved,
- verify whether reconnect returned the gateway to `LiveReady`,
- inspect `broker.orders` and `broker.positions` before any manual cleanup.

## Acceptance
B2 passes only if:
- entry is accepted and filled,
- flatten is accepted and executed,
- position returns to zero,
- runtime returns to `Flat`,
- there is no orphan order,
- there is no orphan position,
- there is no residual `Blocked` state.
