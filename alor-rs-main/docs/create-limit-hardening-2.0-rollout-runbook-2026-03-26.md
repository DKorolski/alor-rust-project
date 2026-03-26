# Hardening 2.0 Rollout Runbook

Date: 2026-03-26

Related documents:

- `docs/create-limit-hardening-2.0-results-2026-03-26.md`
- `docs/create-limit-tz1.6-results-2026-03-26.md`

## 1. Purpose

This runbook applies the `TZ 2.0` hardening line:

- commit: `774b917`
- image tag: `dev-774b917-diag-20260326`

Goal:

- deploy only `alor-gateway`;
- verify fresh-path behavior still works without unnecessary recycle;
- verify stale-path behavior now performs recycle-before-send and then passes.

## 2. Build And Push

Run from the workspace that contains `Dockerfile.gateway` and `alor-rs-main/`.

```bash
cd /path/to/bybit_barter_test

TAG=dev-774b917-diag-20260326
IMG=ghcr.io/dkorolski/alor-rust-project/alor-gateway:$TAG

docker build -f Dockerfile.gateway -t "$IMG" .
docker push "$IMG"
```

## 3. Rollout On VPS

```bash
ssh root@155.212.170.21
```

Backup current env files:

```bash
cp /opt/trading-sessiongap/.env /opt/trading-sessiongap/.env.bak.$(date +%Y%m%d-%H%M%S)
cp /opt/trading-hybrid/.env /opt/trading-hybrid/.env.bak.$(date +%Y%m%d-%H%M%S)
```

Switch only gateway image tag:

```bash
sed -i 's/^IMAGE_TAG=.*/IMAGE_TAG=dev-774b917-diag-20260326/' /opt/trading-sessiongap/.env
sed -i 's/^IMAGE_TAG=.*/IMAGE_TAG=dev-774b917-diag-20260326/' /opt/trading-hybrid/.env
```

Recreate only `alor-gateway`:

```bash
cd /opt/trading-sessiongap
docker compose -p sessiongap up -d --no-deps --force-recreate alor-gateway

cd /opt/trading-hybrid
docker compose -p hybrid up -d --no-deps --force-recreate alor-gateway
```

Wait until both gateways are ready:

```bash
/opt/limit_diag.sh wait-ready sessiongap 180
/opt/limit_diag.sh wait-ready hybrid 180
```

Sanity-check new readiness fields:

```bash
docker exec sessiongap-alor-gateway-1 curl -sS http://127.0.0.1:8081/readiness
docker exec hybrid-alor-gateway-1 curl -sS http://127.0.0.1:8081/readiness
```

Expected new fields:

- `control_path_stale`
- `control_path_stale_reason`
- `control_path_stale_for_ms`
- `control_path_stale_detected_total`
- `control_path_recycle_total`
- `control_path_recycle_success_total`
- `control_path_recycle_failed_total`
- `control_path_stale_blocked_send_total`

## 4. Acceptance A: Fresh Path PASS Without Recycle

Target stack:

- `sessiongap`

Expected:

- immediate passive `create:limit -> delete:limit` passes;
- no `control_path_recycle_start`;
- no `control_path_send_after_recycle`.

```bash
RUN_DIR=$(/opt/limit_diag.sh init-run)
/opt/limit_diag.sh restart-gateway sessiongap 180 >/dev/null
/opt/limit_diag.sh preflight "$RUN_DIR" sessiongap fresh_before >/dev/null
/opt/limit_diag.sh loop sessiongap 79.00 1 1.0 buy 1
```

Inspect:

```bash
docker compose -p sessiongap -f /opt/trading-sessiongap/docker-compose.yml logs --since=5m alor-gateway | \
grep -E 'control_path_stale_detected|control_path_recycle_start|control_path_recycle_success|control_path_send_after_recycle|control_path_send_blocked_due_to_stale' || true
```

Fresh-path acceptance:

- the order cycle passes;
- the grep above is empty.

## 5. Acceptance B: Stale Path Recycle-Before-Send PASS

Target stack:

- `sessiongap`

Configured threshold:

- `control_path_stale_after_sec = 900`

Wait slightly above threshold to avoid edge ambiguity:

- `960` seconds

Expected:

- first passive `create:limit` after idle does not fail with direct stale-path send;
- logs show stale detection and recycle;
- order cycle passes on the fresh connection.

```bash
RUN_DIR=$(/opt/limit_diag.sh init-run)
/opt/limit_diag.sh restart-gateway sessiongap 180 >/dev/null
/opt/limit_diag.sh preflight "$RUN_DIR" sessiongap stale_baseline >/dev/null
sleep 960
/opt/limit_diag.sh preflight "$RUN_DIR" sessiongap stale_before >/dev/null
/opt/limit_diag.sh loop sessiongap 79.00 1 1.0 buy 1
/opt/limit_diag.sh capture post "$RUN_DIR"
```

Inspect readiness before and after:

```bash
sed -n '1,120p' "$RUN_DIR/sessiongap.preflight.stale_before.summary.txt"
sed -n '1,120p' "$RUN_DIR/sessiongap.readiness.post.json"
```

Inspect logs:

```bash
grep -nE 'control_path_stale_detected|control_path_recycle_start|control_path_recycle_success|control_path_send_after_recycle|control_path_send_blocked_due_to_stale' \
  "$RUN_DIR/sessiongap.gateway.post.log"
```

Stale-path acceptance:

- `control_path_stale_detected` is present;
- `control_path_recycle_start` is present;
- `control_path_recycle_success` is present;
- `control_path_send_after_recycle` is present;
- order cycle ends `accepted -> working -> canceled`, `filled = 0.0`.

## 6. Acceptance C: Recycle Failure Controlled Error

This is optional and should only be run if you are comfortable doing a temporary gateway-network disruption during the recycle window.

Expected:

- no send into stale path;
- ack error with:
  - `error_code = control_path_recycle_failed`
- log:
  - `control_path_recycle_failed`
  - `control_path_send_blocked_due_to_stale`

Recommended approach:

1. restart `sessiongap` gateway;
2. wait `960` seconds;
3. right before the first stale-path probe, break outbound network for the gateway long enough to make recycle timeout;
4. send one passive `create:limit`.

Do not run this unless you specifically want the failure-path proof.

## 7. Acceptance D: Market Path Regression

Target stack:

- `hybrid` paper

Expected:

- `market` path still behaves as before;
- no hardening recycle logs, because `market` is out of scope.

Example manual market entry payload:

```bash
REQ_ID=$(cat /proc/sys/kernel/random/uuid)
TS=$(date +%s)

PAYLOAD=$(cat <<JSON
{"schema_version":1,"ts_utc":$TS,"source":"manual-market-regression","msg_type":"command","payload":{"request_id":"$REQ_ID","created_ts_utc":$TS,"strategy_id":"manual.market.hybrid.entry","portfolio":"7502SN6","exchange":"MOEX","symbol":"IMOEXF","action":{"market":{"qty":1.0,"side":"buy","comment":"market_entry_$REQ_ID"}},"intent_class":"entry","ttl_ms":600000}}
JSON
)

docker compose -p hybrid -f /opt/trading-hybrid/docker-compose.yml exec -T redis \
  redis-cli XADD cmd.orders.7502SN6 "*" payload "$PAYLOAD"
```

Check logs:

```bash
docker compose -p hybrid -f /opt/trading-hybrid/docker-compose.yml logs --since=5m alor-gateway | \
grep -E 'control_path_stale_detected|control_path_recycle_start|control_path_recycle_success|control_path_send_after_recycle|control_path_send_blocked_due_to_stale' || true
```

Market-path acceptance:

- order goes through normal market path;
- no hardening recycle logs are emitted for this command.

## 8. Rollback

Restore the previous `IMAGE_TAG` in both stack `.env` files and recreate only `alor-gateway` again:

```bash
cp /opt/trading-sessiongap/.env.bak.<timestamp> /opt/trading-sessiongap/.env
cp /opt/trading-hybrid/.env.bak.<timestamp> /opt/trading-hybrid/.env

cd /opt/trading-sessiongap
docker compose -p sessiongap up -d --no-deps --force-recreate alor-gateway

cd /opt/trading-hybrid
docker compose -p hybrid up -d --no-deps --force-recreate alor-gateway
```

## 9. Success Criteria

`TZ 2.0` rollout is operationally validated when:

1. fresh-path passive limit probe passes with no recycle;
2. stale-path passive limit probe passes with explicit recycle-before-send evidence;
3. market path remains unaffected;
4. readiness and logs expose the stale/recycle decision clearly enough for post-incident review.
