## RUNBOOK: VPS deployment (gateway + runtime + redis)

This runbook focuses on operating the Docker Compose deployment on a single VPS.

It complements the detailed internal runbooks in `alor-rs-main/docs/`:

- `docs/alor-gateway-runbook.md`
- `docs/strategy-runtime-runbook.md`
- `docs/devops-paper-runbook.md`

---

### Dev stack (isolated from main)

If `main` already runs in `/opt/trading`, start `dev` with an isolated override:

```bash
cd /opt/trading-dev
docker compose \
  --env-file .env.dev \
  -f docker-compose.yml \
  -f docker-compose.dev.yml \
  -p trading-dev \
  up -d
```

This uses separate container names (`trading_dev_*`) and `.env.dev` values.

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
- Outside an active session (or if the configured instrument is not trading at that time), `readiness=503` can be expected and does not by itself mean the deploy is broken.

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
   - `RUNTIME_CONFIG` set to an existing strategy profile, for example:
     - `/configs/runtime.sessiongap.live.7502MIW.toml`, or
     - `/configs/runtime.hybrid.live.7502SN6.toml`.
   - `RUNTIME_ENABLE_TEST_HOOKS` is **not** set to `true` anywhere.

2. Restart stack after any env/config change:

```bash
cd /opt/trading
docker compose up -d
```

3. Run paper smoke on the same image/tag (as per project docs).

Paper baseline used before live cutover (March 6, 2026):

- `session_gap_standalone` on `USDRUBF`;
- `trades_total=3`, `win_rate=1.0`, `pnl_net_total=204.7248` (from `summary0.json`).

4. Check health as in paper preflight, plus:

- Gateway readiness indicates `LiveReady` phase.
- Runtime readiness is `true` and scheduler state is `Open`.

5. Only after all checks, enable live orders according to the strategy/runtime configuration (see `alor-rs-main/docs/strategy-runtime-runbook.md`).

6. For micro-account live testing, keep strategy sizing behavior explicit:

- `session_gap_standalone` forces live entry size to `1` in code
  (`strategy-runtime/src/strategies/session_gap_standalone.rs`, live call `maybe_generate_signal(..., true)`).
- Do not rely on `[strategy].qty` in TOML for this strategy's live sizing.

---

### 3. Example: switch instrument (`USDRUBF` -> `IMOEXF`)

If you want to test or run on another instrument, change configs first, then recreate containers.

Files to update (strategy-specific profiles):

- session_gap:
  - `alor-rs-main/configs/gateway.sessiongap.live.7502MIW.toml`
  - `alor-rs-main/configs/runtime.sessiongap.live.7502MIW.toml`
- hybrid:
  - `alor-rs-main/configs/gateway.hybrid.live.7502SN6.toml`
  - `alor-rs-main/configs/runtime.hybrid.live.7502SN6.toml`

Minimum changes:

1. In the target gateway profile:

- `symbols = ["IMOEXF"]`
- `log_positions_filter = ["IMOEXF"]`

2. In the target runtime profile(s):

- `[strategy].symbol = "IMOEXF"`

3. Review instrument-specific parameters:

- `[strategy].tick_size`
- `[general].price_step`
- `trading_periods`
- `weekends_off`

Do not assume they are identical across instruments. Validate them against the actual market/instrument contract.

4. Recommended: change runtime state stream name to avoid mixing old state with a previous instrument:

- paper:
  - from `runtime.state.session_gap_standalone.paper.7502T0U`
  - to e.g. `runtime.state.session_gap_standalone.paper.imoexf.7502T0U`
- live:
  - from `runtime.state.session_gap_standalone.live.7502T0U`
  - to e.g. `runtime.state.session_gap_standalone.live.imoexf.7502T0U`

5. After copying updated configs to VPS:

```bash
cd /opt/trading
docker compose up -d
docker compose ps
docker compose logs --tail=200
```

6. Re-check health:

```bash
docker compose exec -T alor-gateway curl -s http://127.0.0.1:8081/readiness || true
docker compose exec -T strategy-runtime curl -s http://127.0.0.1:8091/readiness || true
```

If you switch to an instrument with an active session at the current time, this is the right moment to validate whether `readiness` becomes `200`.

---

### 4. Normal operations

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

### 5. Deployment operations (day-to-day)

Standard deploy flow after a code change:

1. Push changes to `main`:

```bash
git push origin main
```

2. Wait for GitHub Actions workflow `CI and Docker images` to finish successfully.

3. On VPS, pull fresh images:

```bash
cd /opt/trading
docker compose pull
```

4. Recreate containers with the new image:

```bash
docker compose up -d
```

5. Verify:

```bash
docker compose ps
docker compose logs --tail=200
```

For paper, `IMAGE_TAG=latest` is acceptable as a convenience.

For live, use only a fixed tag in `/opt/trading/.env`:

- `IMAGE_TAG=sha-...`
- or `IMAGE_TAG=vX.Y.Z`

Then run:

```bash
cd /opt/trading
docker compose pull
docker compose up -d
```

If `docker compose pull` fails:

- check that GitHub Actions completed successfully,
- confirm GHCR login on VPS (`docker login ghcr.io`),
- confirm that the requested `IMAGE_TAG` exists in GHCR.

---

### 6. Incident snapshot (first diagnostics)

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

### 7. Rollback procedure

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

### 8. Reboot and persistence checks

After planned or unplanned VPS reboot:

```bash
cd /opt/trading
docker compose ps
```

All services should be `Up` thanks to `restart: unless-stopped`.

Repeat preflight health checks (paper or live) and review logs for any startup warnings.

---

### 9. Backups and restore (summary)

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
