## RUNBOOK: VPS deployment (gateway + runtime + redis)

This runbook focuses on operating the Docker Compose deployment on a single VPS.

It complements the detailed internal runbooks in `alor-rs-main/docs/`:

- `docs/alor-gateway-runbook.md`
- `docs/strategy-runtime-runbook.md`
- `docs/devops-paper-runbook.md`

---

### 1. Preflight checklist (paper)

From VPS (`deploy` user):

```bash
cd /opt/trading
docker compose ps
```

Check that containers are up, then check health **inside containers**:

```bash
docker compose exec -T alor-gateway curl -fsS http://127.0.0.1:8081/liveness
docker compose exec -T alor-gateway curl -fsS http://127.0.0.1:8081/readiness

docker compose exec -T strategy-runtime curl -fsS http://127.0.0.1:8091/liveness
docker compose exec -T strategy-runtime curl -fsS http://127.0.0.1:8091/readiness
```

Expected:

- `liveness` returns 200.
- `readiness` eventually returns 200 when gateway/runtime are ready.

Redis basic check (if CLI installed on host or in a helper container):

```bash
docker exec -it trading_redis redis-cli PING
```

Expected: `PONG`.

---

### 2. Preflight checklist (live)

Before enabling live trading:

1. Confirm that `.env` on VPS has:
   - `IMAGE_TAG` set to a fixed, tested tag.
   - `RUNTIME_CONFIG=/configs/runtime.live.toml`.
   - `RUNTIME_ENABLE_TEST_HOOKS` is **not** set to `true` anywhere.

2. Restart stack after any env/config change:

```bash
cd /opt/trading
docker compose up -d
```

3. Run paper smoke on the same image/tag (as per project docs).

4. Check health as in paper preflight, plus:

- Gateway readiness indicates `LiveReady` phase.
- Runtime readiness is `true` and scheduler state is `Open`.

5. Only after all checks, enable live orders according to the strategy/runtime configuration (see `alor-rs-main/docs/strategy-runtime-runbook.md`).

---

### 3. Normal operations

- View container list:

```bash
cd /opt/trading
docker compose ps
```

- View recent logs:

```bash
docker compose logs --tail=200 alor-gateway
docker compose logs --tail=200 strategy-runtime
```

- Restart a single service:

```bash
docker compose restart alor-gateway
docker compose restart strategy-runtime
```

---

### 4. Incident snapshot (first diagnostics)

When something looks wrong (alerts, no trades, repeated reconnects, etc.), take a quick snapshot **before** changing anything:

```bash
cd /opt/trading
docker compose ps

docker compose logs --tail=200 alor-gateway
docker compose logs --tail=200 strategy-runtime

docker compose exec -T alor-gateway curl -fsS http://127.0.0.1:8081/readiness || true
docker compose exec -T strategy-runtime curl -fsS http://127.0.0.1:8091/readiness || true

df -h
free -m
top -b -n 1 | head -n 30
```

Save this output with a timestamp; it will greatly speed up postmortem and debugging.

Optionally, capture Docker events:

```bash
docker events --since 10m > /tmp/docker-events-$(date +%s).log &
```

---

### 5. Rollback procedure

If a new deployment causes issues and you need to quickly roll back:

1. Edit `/opt/trading/.env` and set `IMAGE_TAG` to the previous known-good tag.

2. Apply:

```bash
cd /opt/trading
docker compose pull
docker compose up -d
```

3. Run paper or minimal smoke tests as per project docs.

4. Confirm health endpoints and that no unexpected open orders/positions exist.

---

### 6. Reboot and persistence checks

After planned or unplanned VPS reboot:

```bash
cd /opt/trading
docker compose ps
```

All services should be `Up` thanks to `restart: unless-stopped`.

Repeat preflight health checks (paper or live) and review logs for any startup warnings.

---

### 7. Backups and restore (summary)

- Regularly back up:
  - `/opt/trading/.env`
  - `/opt/trading/docker-compose.yml`
  - `/opt/trading/configs/`
  - `/opt/trading/volumes/redis/` (if persistence is enabled)

- Test restore at least once:
  - On a clean VPS:
    - install Docker/Compose,
    - restore `/opt/trading`,
    - run `docker compose up -d`,
    - perform paper preflight checks.

Document results of the restore test (date, image tag, success/fail) in this file or a separate ops log.

