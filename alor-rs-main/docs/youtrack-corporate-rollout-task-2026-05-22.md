# YouTrack Task Text: Corporate Server Rollout

## Summary

Prepare corporate GitLab and corporate server environment for controlled micro
deployment of the Alor Rust trading runtime.

## Background

The project currently runs a Rust-based Alor trading runtime with independent
gateway/runtime/Redis contours for MOEX futures strategies. The sanitized
GitHub `main` branch is CI-green and excludes raw live observation journals,
raw broker ledgers, and secrets.

Current baseline:

- repository: sanitized `main`
- latest validated runtime code baseline before documentation-only handoff:
  `dfe95cf`
- GitHub checks: `test = success`, `docker-build-and-push = success`
- local checks passed:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all`

The project has passed extended micro live soak, but should remain at micro
size for additional observation. The current recommendation is to continue
micro soak for approximately 2-3 months before considering promotion to
`small`.

## Scope

1. Import sanitized repository into corporate GitLab.
2. Configure GitLab CI equivalent to current GitHub CI.
3. Build and publish Docker images to corporate registry.
4. Prepare corporate server deployment layout.
5. Configure one or more controlled micro stacks.
6. Configure Redis maintenance job.
7. Configure weekly resource and log monitoring.
8. Document deployment SHA, image tags, and runtime configs.

## Components

Main services:

- `redis` - durable stream bus and runtime state store.
- `alor-gateway` - Alor WS/CWS gateway, market data, broker events, command
  execution.
- `strategy-runtime` - strategy host, live guard, state restore, command
  emission, reports.

Strategies currently prepared:

- USDRUBF SessionGap, micro.
- USDRUBF Alor Hybrid, micro.
- IMOEXF Hybrid MR/BO RiskGate, currently qty 2 micro validation.
- RI Author41/42, micro.

Important technical requirement:

- Live command execution must use action-scoped CWS configs. Do not downgrade
  to legacy long-lived CWS command paths.

## Acceptance Criteria

Repository and CI:

- Sanitized source imported into GitLab.
- No refresh token, raw broker ledger, `.env`, private SSH keys, or raw live
  observation journals in GitLab.
- GitLab CI passes:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all`
- Docker images build successfully:
  - `alor-gateway`
  - `strategy-runtime`
- Image tags include exact commit SHA.

Server deployment:

- Docker Engine and Compose plugin installed.
- Deployment directories created under `/opt/trading-*`.
- Configs mounted read-only into containers.
- Reports and Redis data mounted as persistent volumes.
- `.env` created on server and protected with restrictive permissions.
- `ALOR_REFRESH_TOKEN` stored only in server secret mechanism or `.env`, never
  in Git.
- Stack starts successfully with `docker compose up -d`.
- `docker compose ps` shows all services healthy/running.
- Gateway and runtime liveness endpoints pass through container healthchecks.

Redis maintenance:

- `redis_safe_trim.sh` installed.
- Dry-run checked.
- Apply mode tested during safe maintenance window.
- Scheduled job configured, preferably systemd timer or corporate scheduler.
- Job runs outside main trading session.
- Logs are written to auditable location.
- `runtime.state.*` and `runtime.riskgate.*` are never trimmed.

Monitoring:

- Weekly resource checklist assigned to an owner.
- Redis memory and stream sizes are reviewed weekly.
- Disk usage is reviewed weekly.
- Gateway/runtime logs are reviewed for warnings/errors.
- Broker flat/non-flat state is checked after relevant sessions.
- Alert thresholds are defined for Redis memory, disk usage, liveness failures,
  stale bars, repeated CWS errors, and unexpected pending orders.

## Deployment Notes

Recommended server layout:

```text
/opt/trading-<stack>/
  docker-compose.yml
  .env
  configs/
  volumes/
    redis/
    reports/

/opt/trading-maintenance/
  redis_safe_trim.sh
```

Do not use broad Redis `FLUSHALL` during normal operations. For `from zero`
restarts, clear only the intended runtime state and command/ack streams after
confirming broker flat and no working orders.

## Redis Maintenance Commands

Dry-run:

```bash
/opt/trading-maintenance/redis_safe_trim.sh --dry-run
```

Apply:

```bash
/opt/trading-maintenance/redis_safe_trim.sh --apply
```

Systemd timer:

```bash
sudo cp alor-rs-main/deploy/systemd/trading-redis-safe-trim.service /etc/systemd/system/
sudo cp alor-rs-main/deploy/systemd/trading-redis-safe-trim.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now trading-redis-safe-trim.timer
systemctl list-timers | grep trading-redis-safe-trim
```

Default schedule: daily at `03:10 Europe/Moscow`.

## Weekly Operations Checklist

```bash
date
uptime
free -h
df -h
docker ps
docker stats --no-stream
docker system df
```

Per stack:

```bash
docker compose exec -T redis redis-cli INFO memory
docker compose exec -T redis redis-cli DBSIZE
docker compose exec -T redis redis-cli --scan | wc -l
docker compose logs --since 24h alor-gateway | egrep -i 'error|warn|reject|timeout|cws|broken pipe|connection refused'
docker compose logs --since 24h strategy-runtime | egrep -i 'error|warn|reject|timeout|blocked|deferred|manual|safe_mode'
```

Safe Docker cleanup:

```bash
docker system df
docker image prune -f
docker builder prune -f
```

Use `docker system prune -a` only with explicit approval.

## Out of Scope

- Increasing live size to `small`.
- Enabling TP bracket/passive limit for RI MR as primary contract.
- Replacing action-scoped CWS with legacy long-lived command path.
- Sharing or committing `ALOR_REFRESH_TOKEN`.
- Importing raw live observation journals or raw broker ledgers into GitLab.

## Links / Reference Docs

- `docs/corporate-handoff-project-overview-2026-05-22.md`
- `docs/live-runtime-service-patterns-anti-regression-checklist-2026-05-07.md`
- `docs/redis-retention-and-cleanup-plan-2026-04-21.md`
- `docs/ri-author41-42-live-contract-2026-05-01.md`
- `docs/ri-author41-42-live-micro-contour-plan-2026-05-01.md`
- `docs/imoexf-primary-runtime-integration-review-handoff-2026-04-26.md`
