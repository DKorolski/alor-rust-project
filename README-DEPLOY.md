## Deploy guide (VPS / Docker Compose)

This document describes how to deploy the trading gateway + runtime + Redis on a single VPS using Docker Compose.

It is aligned with the main project README and the internal runbooks in `alor-rs-main/docs/`.

Local validation note:

- the stack was validated locally with `docker-compose.local.yml`;
- all three containers (`redis`, `alor-gateway`, `strategy-runtime`) reached `healthy`;
- `liveness` endpoints responded correctly;
- `readiness` can legitimately return `503` outside an active trading session or when the configured instrument is not trading at the current time.

Paper validation snapshot (March 6, 2026):

- strategy: `session_gap_standalone` (`USDRUBF`);
- reports file: `/opt/trading/volumes/reports/summary0.json`;
- result: `trades_total=3`, `win_rate=1.0`, `pnl_net_total=204.7248`.

Live micro-account sizing note:

- for `session_gap_standalone`, live order size is forced to `1` in strategy code
  (`strategy-runtime/src/strategies/session_gap_standalone.rs`, live path calls `maybe_generate_signal(..., true)`).
- `strategy.qty` in runtime TOML is not the sizing source for this strategy.

---

### 1. Prerequisites on VPS

- Ubuntu LTS.
- Non-root user `deploy` with SSH key access and `sudo` (minimal).
- Docker + Docker Compose installed.
- Basic hardening applied (SSH without password login, firewall, fail2ban, chrony for time sync).
- GHCR pull access prepared (`docker login ghcr.io` with read-only token).

Directory layout on VPS:

- `/opt/trading/`
  - `docker-compose.yml` (from this repo)
  - `.env` (copied from `env.example` and filled with real values)
  - `configs/` (TOML configs copied from `alor-rs-main/configs/`)
  - `volumes/redis/`
  - `volumes/reports/`

---

### 2. Prepare configs and env

On your local machine (or directly on VPS if you prefer):

1. Copy example env:

```bash
cp env.example .env
```

2. Edit `.env`:

- Set `IMAGE_TAG` to a fixed tag (git SHA or `vX.Y.Z`) of the Docker images you want to run.
- Adjust `CONFIG_DIR`, `REDIS_DATA_DIR`, `REPORTS_DIR` if needed.
- Set `GATEWAY_CONFIG` and `RUNTIME_CONFIG`:
  - Paper mode: `RUNTIME_CONFIG=/configs/runtime.paper.toml`
  - Live mode (session_gap): `RUNTIME_CONFIG=/configs/runtime.sessiongap.live.7502MIW.toml`
  - Live mode (hybrid): `RUNTIME_CONFIG=/configs/runtime.hybrid.live.7502SN6.toml`
- Fill `ALOR_REFRESH_TOKEN` and any other secrets.
- Set per-stream `TRIM_MAXLEN_*` values. Keep health heartbeat enabled: the
  runtime uses fresh gateway health as a live guard. Reduce retention instead
  of changing health publication to change-only.

Recommended live-micro retention baseline:

```dotenv
TRIM_MAXLEN_BARS=3000
TRIM_MAXLEN_ORDERS=5000
TRIM_MAXLEN_TRADES=5000
TRIM_MAXLEN_POSITIONS=2000
TRIM_MAXLEN_SNAPSHOTS=2000
TRIM_MAXLEN_COMMANDS=5000
TRIM_MAXLEN_ACKS=5000
TRIM_MAXLEN_HEALTH=1500
```

The legacy `TRIM_MAXLEN` remains a fallback for any per-stream value that is
not set. Older gateway images only understand the legacy global value, so
deploy a gateway image containing per-stream retention support before adding
these variables.

Tag policy:

- `paper`: `latest` is acceptable if you intentionally allow frequent updates.
- `live`: use only fixed tags (`:<git-sha>` or `:vX.Y.Z`) to make rollback deterministic.

CI/CD behavior:

- test job runs on `main` and `devops/vps-setup`;
- image build/push to GHCR runs only on `main` and release tags (`v*.*.*`).

3. Configure GHCR pull auth on VPS (one-time):

```bash
echo "$GHCR_READ_TOKEN" | docker login ghcr.io -u <ghcr-user> --password-stdin
```

Use a dedicated token with minimal scope (`read:packages`), do not use personal developer tokens with broad permissions.

4. Copy configs from workspace:

From the project root on your local machine:

```bash
scp -r alor-rs-main/configs deploy@<VPS_HOST>:/opt/trading/
```

Or clone the repo on VPS and copy `alor-rs-main/configs` into `/opt/trading/configs`.

---

### 3. Deploy with Docker Compose (paper profile)

On VPS, as `deploy` user:

```bash
cd /opt/trading
docker compose pull
docker compose up -d
```

This will start:

- `redis`
- `alor-gateway`
- `strategy-runtime` (by default with `RUNTIME_CONFIG` from `.env`, usually paper mode)

Startup behavior:

- `redis` has an explicit healthcheck (`redis-cli PING`);
- `alor-gateway` waits for healthy Redis before starting;
- `strategy-runtime` also waits for healthy Redis before starting.

Security/network notes:

- Compose defines two networks:
  - `app_net` with `internal: true` (Redis and internal traffic only),
  - `ext_net` as a normal network (gateway/runtime use it for outbound WS/CWS to Alor).
- No service ports are published externally by default.
- Access health endpoints via `docker compose exec`, SSH tunnel, or VPN.

Check container status:

```bash
docker compose ps
```

Check health endpoints from VPS (inside containers):

```bash
cd /opt/trading
docker compose exec -T alor-gateway curl -fsS http://127.0.0.1:8081/liveness
docker compose exec -T alor-gateway curl -fsS http://127.0.0.1:8081/readiness

docker compose exec -T strategy-runtime curl -fsS http://127.0.0.1:8091/liveness
docker compose exec -T strategy-runtime curl -fsS http://127.0.0.1:8091/readiness
```

Interpretation:

- `liveness` should return `200`;
- `readiness` should return `200` when the service is fully ready and the configured market/session is active;
- `readiness=503` can be expected outside an active session and does not by itself indicate a deploy failure.

---

### 4. Switching between paper and live

- **Paper**:
  - `RUNTIME_CONFIG=/configs/runtime.paper.toml`
  - `IMAGE_TAG` can follow `latest` if you accept more frequent updates.

- **Live**:
  - Use a fixed, tested `IMAGE_TAG` (git SHA or `vX.Y.Z`).
  - `RUNTIME_CONFIG` points to an existing strategy profile, e.g.:
    - `/configs/runtime.sessiongap.live.7502MIW.toml`
    - `/configs/runtime.hybrid.live.7502SN6.toml`
  - Ensure `RUNTIME_ENABLE_TEST_HOOKS=false` (enforced in compose).

After changing `.env`, reload:

```bash
cd /opt/trading
docker compose up -d
```

---

### 5. Rollback

To rollback to a previous image tag:

1. Edit `/opt/trading/.env` on VPS:

```dotenv
IMAGE_TAG=<previous_stable_tag>
```

2. Apply:

```bash
cd /opt/trading
docker compose pull
docker compose up -d
```

3. Verify:

- Containers are healthy (`docker compose ps`).
- Health endpoints return expected status.

Details for pre-live checks and incident handling are in `RUNBOOK.md`.

---

### Redis stream retention verification

After deploying a gateway image with per-stream retention support, verify that
the resolved limits appear in the gateway startup log:

```bash
docker compose logs --tail 100 alor-gateway | grep 'gateway mode: transport-only'
```

Then check the largest streams:

```bash
docker compose exec -T redis redis-cli --scan --pattern 'events.health*'
docker compose exec -T redis redis-cli XLEN <health-stream-name>
docker compose exec -T redis redis-cli XLEN <snapshot-stream-name>
```

Expected behavior:

- health remains periodic and fresh for the runtime live guard;
- health length converges to at most `TRIM_MAXLEN_HEALTH`;
- snapshot and position lengths converge to their own limits;
- command/order/trade history keeps its independently configured limit.

Rollback only the gateway image and remove the per-stream variables if a
problem appears. The runtime and Redis do not require a from-zero restart for
this retention-only change.

---

### Optional: manage compose with systemd (recommended)

Example unit (`/etc/systemd/system/trading.service`):

```ini
[Unit]
Description=Trading stack (docker compose)
After=docker.service network-online.target
Requires=docker.service

[Service]
Type=oneshot
WorkingDirectory=/opt/trading
ExecStart=/usr/bin/docker compose up -d
ExecStop=/usr/bin/docker compose down
RemainAfterExit=yes
TimeoutStartSec=0

[Install]
WantedBy=multi-user.target
```

Then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now trading
sudo systemctl status trading
```
