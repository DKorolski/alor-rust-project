## Deploy guide (VPS / Docker Compose)

This document describes how to deploy the trading gateway + runtime + Redis on a single VPS using Docker Compose.

It is aligned with the main project README and the internal runbooks in `alor-rs-main/docs/`.

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
  - Live mode: `RUNTIME_CONFIG=/configs/runtime.live.toml`
- Fill `ALOR_REFRESH_TOKEN` and any other secrets.

Tag policy:

- `paper`: `latest` is acceptable if you intentionally allow frequent updates.
- `live`: use only fixed tags (`:<git-sha>` or `:vX.Y.Z`) to make rollback deterministic.

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

---

### 4. Switching between paper and live

- **Paper**:
  - `RUNTIME_CONFIG=/configs/runtime.paper.toml`
  - `IMAGE_TAG` can follow `latest` if you accept more frequent updates.

- **Live**:
  - Use a fixed, tested `IMAGE_TAG` (git SHA or `vX.Y.Z`).
  - `RUNTIME_CONFIG=/configs/runtime.live.toml`
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
