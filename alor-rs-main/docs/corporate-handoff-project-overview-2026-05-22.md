# Corporate Handoff: Alor Rust Trading Runtime

Date: 2026-05-22

Status: ready for corporate server preparation. The current `origin/main`
sanitized baseline is CI-green and intentionally excludes raw live observation
journals, raw broker ledger dumps, and refresh tokens.

## Executive Summary

This repository contains a Rust-based live/paper trading runtime for MOEX
futures strategies through Alor market data and command WebSocket APIs.

The system has passed an extended micro live soak on several independent
strategy contours. The soak confirmed that the runtime/gateway contour is
operationally viable, but the trading size should remain micro for additional
observation. The recommended plan is to continue micro live observation for
approximately 2-3 months before considering promotion to `small`.

Current promotion posture:

- `GO` for continued controlled micro operation.
- `NO-GO` for immediate broad `small` scale-up.
- Corporate migration is appropriate if the target server has equivalent or
  better resource monitoring, Redis retention jobs, secret handling, and
  deployment discipline.

## Repository Structure

Top-level:

- `.github/workflows/deploy.yml` - GitHub CI and Docker image build workflow.
- `docker-compose.yml` - production-style compose template for one
  gateway/runtime/Redis stack.
- `docker-compose.dev.yml`, `docker-compose.local.yml` - local/dev variants.
- `env.example` - deployment environment template. Must be copied to `.env`.
- `Dockerfile.gateway` - Docker image for `alor-gateway`.
- `Dockerfile.runtime` - Docker image for `strategy-runtime`.

Rust workspace under `alor-rs-main/`:

- `alor-types` - shared types, scheduler/session helpers.
- `alor-protocol` - Redis command/ack protocol and execution intent schema.
- `alor-gateway` - Alor WS/CWS gateway, broker stream publishing, command
  consumer, action-scoped CWS command path.
- `strategy-runtime` - strategy host/runtime, live guard, state restore,
  command emission, paper reports, strategy implementations.
- `standalone-cws-stability-probe` - diagnostic CWS probe utility.

Operational/config folders:

- `alor-rs-main/configs/` - runtime and gateway TOML configs.
- `alor-rs-main/scripts/` - diagnostic and Redis maintenance helpers.
- `alor-rs-main/deploy/systemd/` - safe Redis trim systemd timer/service.
- `alor-rs-main/docs/` - architecture, model handoff, parity, and rollout docs.

## Runtime Components

### Redis

Redis is the shared durable stream bus:

- market data bars: `md.bars.*`
- broker orders/trades/positions: `broker.orders.*`, `broker.trades.*`,
  `broker.positions.*`
- broker snapshots: `broker.snapshots.*`
- strategy commands: `cmd.orders.*`
- command acknowledgements: `cmd.acks.*`
- runtime state: `runtime.state.*`
- hybrid risk-gate ledger/state: `runtime.riskgate.*`
- health events: `events.health*`

Important rule: maintenance scripts must not trim `runtime.state.*` or
`runtime.riskgate.*` keys.

### alor-gateway

Responsibilities:

- connects to Alor market-data WebSocket;
- publishes normalized bars and broker events to Redis;
- consumes `cmd.orders.*` command streams;
- sends orders through Alor CWS;
- publishes command acknowledgements to `cmd.acks.*`;
- exposes health/liveness endpoints.

Current live configs use the action-scoped CWS path. This is a critical
stability requirement discovered during live soak. Do not revert to a generic
long-lived CWS command path unless a new validation line proves it safe.

Key action-scoped config fields:

```toml
control_cws_mode = "action_scoped"
action_scope_enable_create_limit = true
action_scope_enable_market = true
action_scope_enable_delete_limit = true
action_scope_enable_exit = true
action_scope_force_token_refresh_before_authorize = true
```

### strategy-runtime

Responsibilities:

- reads Redis bars, broker events, snapshots and acks;
- runs strategy logic;
- applies live guard and bootstrap/warmup checks;
- persists runtime state;
- emits order intents into Redis;
- writes paper/live report artifacts where configured;
- exposes health/liveness endpoints.

Key runtime protections:

- bootstrap waits for broker snapshots and live bars;
- stale recovered/history bars are suppressed in live mode;
- exact emitted `request_id` is fed back to strategy-owned state;
- closed-window exits can be deferred before broker emit;
- transient CWS errors use recovery/reconcile paths instead of blind re-entry;
- position/order snapshots are used to block unsafe dirty starts.

## Strategies

### 1. USDRUBF SessionGap

Primary live config:

- runtime: `configs/runtime.sessiongap.live.7502MIW.toml`
- gateway: `configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`
- symbol: `USDRUBF`
- timeframe/feed: `10m`
- quantity: `1`
- style: one-shot entry/exit, no overnight, forced session flatten.

Current engineering read:

- clean baseline contour;
- can have several sessions without trades; this is expected when signal
  conditions are not met;
- should remain micro while observing portfolio interaction with RI on the same
  portfolio.

### 2. USDRUBF Alor Hybrid

Primary live config:

- runtime: `configs/runtime.alor_usdrubf.live.7502T0U.toml`
- gateway: `configs/gateway.alor_usdrubf.live.7502T0U.toml`
- symbol: `USDRUBF`
- timeframe/feed: `10m`
- quantity: `1`
- CWS: action-scoped, including market command support.

Current engineering read:

- resilient broker-truth/retry behavior;
- market path was hardened after live CWS observations;
- should not be scaled before more clean micro sessions are collected.

### 3. IMOEXF Hybrid MR/BO RiskGate

Primary live config:

- runtime: `configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml`
- gateway: `configs/gateway.hybrid.live.7502SN6.action-scoped.toml`
- symbol: `IMOEXF`
- timeframe/feed: `10m`
- quantity: `2`
- profile: `imoexf_primary_riskgate_high180_lb120`
- MR variant: `high180`
- risk gate: `shadow_pnl_lb120_positive`
- risk-gate mode: `normal_append`

Current engineering read:

- high180 MR + BO profile is integrated in existing `hybrid_intraday` runtime;
- risk-gate history is handled through runtime-owned Redis ledger/state;
- BO gap flatten is accepted as live safety behavior, not replay parity;
- partial fills at qty `2` are on the watchlist and need continued observation.

### 4. RI Author41/42

Primary micro config:

- runtime: `configs/runtime.ri_author41_42.micro.7502MIW.pending.toml`
- gateway: `configs/gateway.ri_author41_42.micro.7502MIW.pending.toml`
- internal symbol: `RIM6`
- broker order symbol: `RTS-6.26`
- timeframe/feed: `10m`
- quantity: `1`
- profile: `ri_author41_42_primary_combo_cost2`
- live adapter: enabled
- order emission: enabled

Current engineering read:

- isolated contour, but reuses runtime/gateway service patterns;
- all live orders must use action-scoped paths;
- MR exit contract remains close-bar/model-condition plus marketable exit;
- TP bracket/limit is not enabled as primary contract;
- SL bracket may be reviewed later as a separate safety overlay.

## Test and CI Status

Latest validated runtime code baseline before this documentation-only handoff
update:

```text
dfe95cf Fix clippy warnings on stable Rust
```

GitHub Actions status:

- `test`: success
- `docker-build-and-push`: success

Local CI-equivalent commands passed on stable Rust 1.95:

```bash
cd alor-rs-main
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --quiet
```

Test inventory:

- total Rust tests listed by `cargo test --workspace -- --list`: 437
- `strategy-runtime`: 346
- `alor-gateway`: 77
- `alor-types`: 9
- `alor-protocol`: 5
- doctests: 0

Important integration/e2e groups:

- `strategy-runtime/tests/config_tests.rs`: 25
- `strategy-runtime/tests/e2e_hybrid_golden.rs`: 5
- `strategy-runtime/tests/e2e_session_gap_restart.rs`: 5
- `strategy-runtime/tests/e2e_smoke.rs`: 8
- `strategy-runtime/tests/e2e_buy_and_close.rs`: 4
- `alor-gateway/tests/ws_integration.rs`: 5, one timing-dependent ignored test
- `alor-gateway/tests/redis_transport.rs`: 4
- `alor-gateway/tests/action_scope_cws.rs`: 2

## Extended Micro Soak Summary

The extended micro soak tested several independent live contours:

- SessionGap USDRUBF
- Alor USDRUBF hybrid
- IMOEXF hybrid MR/BO riskgate
- RI Author41/42

Key outcomes:

- live gateway/runtime contour is stable enough for continued micro operation;
- action-scoped CWS path is required for reliable command execution;
- Redis retention must be actively managed;
- no immediate promotion to `small` is recommended;
- strategy behavior is still being observed for live execution edge cases such
  as partial fills, broker margin interaction, and cross-strategy portfolio use;
- additional 2-3 months of micro observation is recommended before small-size
  promotion decisions.

Known operational watchlist:

- Redis memory growth and stream lengths;
- command reject/error paths, especially CWS transport errors;
- stale pending request/order states after reconnects;
- broker truth vs runtime state after manual interventions;
- partial fills when increasing quantity;
- RI contract rollover/expiry handling;
- portfolio margin when multiple strategies share a portfolio;
- risk-gate ledger finalization for IMOEXF hybrid.

## Corporate Deployment Requirements

Recommended minimum server profile:

- Linux server with Docker Engine and Docker Compose plugin;
- 4 vCPU or more;
- 8 GB RAM or more;
- 80 GB disk or more;
- outbound HTTPS/WSS access to Alor API endpoints;
- NTP/time synchronization enabled;
- log rotation enabled;
- backup policy for configs and selected reports;
- secure secret storage for `.env`.

Network access:

- outbound WebSocket/HTTPS to Alor market-data and command APIs;
- outbound access to the selected container registry;
- no public inbound exposure is required for trading containers unless internal
  monitoring explicitly needs it.

Secrets:

- `ALOR_REFRESH_TOKEN` must not be committed or shared in documentation;
- other deployment parameters may be shared unless corporate policy says
  otherwise;
- place secrets only in `.env` or corporate secret manager;
- restrict `.env` file permissions, for example `chmod 600 .env`.

## Deployment Options

### Option A: Use GHCR Images

Use this if the corporate server can pull images from GitHub Container Registry.

Images:

```text
ghcr.io/dkorolski/alor-rust-project/alor-gateway:<IMAGE_TAG>
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:<IMAGE_TAG>
```

Recommended `IMAGE_TAG`:

- exact git SHA from approved `main`, not `latest`, for controlled deployment;
- for the current handoff: use the latest code commit validated by CI before
  documentation-only handoff commits, or the full SHA from `git rev-parse HEAD`
  after internal mirror import.

### Option B: Rebuild in GitLab CI / Corporate Registry

Use this if corporate policy requires internal images.

Steps:

1. Import sanitized `origin/main` into GitLab.
2. Configure GitLab CI to run the same checks:

```bash
cd alor-rs-main
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

3. Build images from repository root:

```bash
docker build -f Dockerfile.gateway -t <registry>/alor-gateway:<tag> .
docker build -f Dockerfile.runtime -t <registry>/strategy-runtime:<tag> .
```

4. Push images to corporate registry.
5. Update `.env` / compose image names if corporate registry names differ from
   GHCR.

## Server Directory Layout

Recommended layout per stack:

```text
/opt/trading-<stack>/
  docker-compose.yml
  .env
  configs/
    gateway.*.toml
    runtime.*.toml
  volumes/
    redis/
    reports/

/opt/trading-maintenance/
  redis_safe_trim.sh
```

For multiple independent strategy stacks, use separate compose project names
and separate host directories. Avoid sharing a Redis container between unrelated
stacks unless explicitly designed and tested.

## Base Deployment Procedure

1. Install packages:

```bash
sudo apt-get update
sudo apt-get install -y docker.io docker-compose-plugin git curl ca-certificates
sudo systemctl enable --now docker
```

2. Create deployment directories:

```bash
sudo mkdir -p /opt/trading/configs
sudo mkdir -p /opt/trading/volumes/redis
sudo mkdir -p /opt/trading/volumes/reports
sudo mkdir -p /opt/trading-maintenance
```

3. Copy compose and configs:

```bash
cp docker-compose.yml /opt/trading/docker-compose.yml
cp alor-rs-main/configs/<gateway-config>.toml /opt/trading/configs/
cp alor-rs-main/configs/<runtime-config>.toml /opt/trading/configs/
```

4. Create `.env` from `env.example`:

```bash
cp env.example /opt/trading/.env
chmod 600 /opt/trading/.env
```

5. Fill `.env`:

```text
IMAGE_TAG=<approved image tag or sha>
CONFIG_DIR=/opt/trading/configs
REDIS_DATA_DIR=/opt/trading/volumes/redis
REPORTS_DIR=/opt/trading/volumes/reports
REDIS_URL=redis://redis:6379/
GATEWAY_CONFIG=/configs/<gateway-config>.toml
RUNTIME_CONFIG=/configs/<runtime-config>.toml
ALOR_REFRESH_TOKEN=<secret value, never commit>
```

6. Start the stack:

```bash
cd /opt/trading
docker compose pull
docker compose up -d
docker compose ps
```

7. Verify logs:

```bash
docker compose logs --tail 200 redis
docker compose logs --tail 200 alor-gateway
docker compose logs --tail 200 strategy-runtime
```

8. Verify health:

```bash
docker compose ps
docker stats --no-stream
```

9. Verify Redis stream sizes:

```bash
docker compose exec -T redis redis-cli INFO memory
docker compose exec -T redis redis-cli --scan | sort | head -100
docker compose exec -T redis redis-cli XLEN broker.snapshots.<PORTFOLIO>
docker compose exec -T redis redis-cli XLEN md.bars.<PORTFOLIO>.10m
docker compose exec -T redis redis-cli XLEN cmd.orders.<PORTFOLIO>
docker compose exec -T redis redis-cli XLEN cmd.acks.<PORTFOLIO>
```

## Strategy-Specific Config Mapping

SessionGap USDRUBF:

```text
GATEWAY_CONFIG=/configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml
RUNTIME_CONFIG=/configs/runtime.sessiongap.live.7502MIW.toml
```

Alor USDRUBF hybrid:

```text
GATEWAY_CONFIG=/configs/gateway.alor_usdrubf.live.7502T0U.toml
RUNTIME_CONFIG=/configs/runtime.alor_usdrubf.live.7502T0U.toml
```

IMOEXF hybrid riskgate:

```text
GATEWAY_CONFIG=/configs/gateway.hybrid.live.7502SN6.action-scoped.toml
RUNTIME_CONFIG=/configs/runtime.hybrid.live.7502SN6.riskgate-shadow.toml
```

RI Author41/42:

```text
GATEWAY_CONFIG=/configs/gateway.ri_author41_42.micro.7502MIW.pending.toml
RUNTIME_CONFIG=/configs/runtime.ri_author41_42.micro.7502MIW.pending.toml
```

If corporate portfolio IDs differ, configs must be reviewed and updated across:

- `portfolio`
- Redis stream names
- `runtime_state`
- strategy IDs where portfolio-specific
- reports paths if needed
- gateway symbols and runtime symbols/order symbols

## Safe Restart Procedure

Preferred restart window:

- outside active trading sessions;
- after broker account is flat for the target instrument;
- after open/working orders and stop orders are confirmed absent.

Procedure:

```bash
cd /opt/trading
docker compose ps
docker compose logs --tail 200 strategy-runtime
docker compose logs --tail 200 alor-gateway
docker compose restart strategy-runtime alor-gateway
docker compose ps
docker compose logs --tail 200 strategy-runtime
```

For `from zero` validation:

- stop runtime/gateway first;
- verify broker flat and no working orders;
- clear only the intended runtime state and command/ack streams;
- do not delete market data history unless warmup/history source is available;
- do not delete risk-gate ledger unless intentionally rebuilding it.

Example patterns, to be adapted per stack:

```bash
docker compose stop strategy-runtime alor-gateway
docker compose exec -T redis redis-cli DEL runtime.state.<strategy>
docker compose exec -T redis redis-cli DEL cmd.orders.<portfolio>
docker compose exec -T redis redis-cli DEL cmd.acks.<portfolio>
docker compose up -d
```

Do not run broad `FLUSHALL` in production unless the whole stack is intentionally
being rebuilt and all history/warmup implications are understood.

## Redis Maintenance

Redis growth was a real operational issue during micro soak. Maintenance is
mandatory.

Script:

```text
alor-rs-main/scripts/redis_safe_trim.sh
```

Safe behavior:

- trims only explicit stream prefixes;
- never trims `runtime.state.*`;
- never trims `runtime.riskgate.*`;
- supports `--dry-run` and `--apply`.

Current default stream retention limits:

```text
events.health.ri_author41_42.* = 2000
events.health = 10000
broker.snapshots.* = 10000
broker.positions.* = 5000
broker.orders.* = 5000
broker.trades.* = 5000
cmd.orders.* = 5000
cmd.acks.* = 5000
md.bars.* = 3000
```

Install maintenance script:

```bash
sudo mkdir -p /opt/trading-maintenance
sudo cp alor-rs-main/scripts/redis_safe_trim.sh /opt/trading-maintenance/
sudo chmod +x /opt/trading-maintenance/redis_safe_trim.sh
```

Manual dry-run:

```bash
/opt/trading-maintenance/redis_safe_trim.sh --dry-run
```

Manual apply:

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

Default schedule:

```text
03:10 Europe/Moscow daily
```

Corporate environment may replace this with:

- systemd timer;
- cron;
- Jenkins/GitLab scheduled job;
- internal job runner.

The job should run outside active trading hours and should write logs to an
auditable location.

## Weekly Operations Checklist

Run at least weekly, and also after any unexpected restart, memory spike, or
manual trading intervention.

Server:

```bash
date
uptime
free -h
df -h
docker ps
docker stats --no-stream
docker system df
```

Redis per stack:

```bash
docker compose exec -T redis redis-cli INFO memory
docker compose exec -T redis redis-cli DBSIZE
docker compose exec -T redis redis-cli --scan | wc -l
docker compose exec -T redis redis-cli --scan --pattern 'md.bars.*' | sort
docker compose exec -T redis redis-cli --scan --pattern 'broker.snapshots.*' | sort
docker compose exec -T redis redis-cli --scan --pattern 'runtime.state.*' | sort
docker compose exec -T redis redis-cli --scan --pattern 'runtime.riskgate.*' | sort
```

Logs:

```bash
docker compose logs --since 24h alor-gateway | egrep -i 'error|warn|reject|timeout|cws|broken pipe|connection refused'
docker compose logs --since 24h strategy-runtime | egrep -i 'error|warn|reject|timeout|blocked|deferred|manual|safe_mode'
```

Resource thresholds requiring attention:

- Redis memory consistently above 60-70% of configured limit;
- disk usage above 70%;
- repeated `broken pipe`, `connection refused`, or CWS transport errors;
- runtime stuck in `BLOCKED` during expected live session;
- pending command/ack streams growing unexpectedly;
- risk-gate ledger not finalizing after regular session;
- broker is non-flat when runtime believes it is flat.

Docker cleanup:

```bash
docker system df
docker image prune -f
docker builder prune -f
```

Use `docker system prune -a` only with explicit approval, because it can remove
images needed for rollback.

## Monitoring Recommendations

Minimum monitoring:

- container health: Redis, gateway, runtime;
- memory per container;
- host memory and disk;
- Redis `used_memory_human`;
- Redis stream lengths for bars, snapshots, commands, acks;
- last health event timestamp;
- gateway/runtime log warnings and errors;
- broker flat/non-flat status after session close.

Recommended alert classes:

- Redis memory > 70% for more than 15 minutes;
- disk usage > 75%;
- runtime liveness failed;
- gateway liveness failed;
- no fresh bars during expected trading time;
- repeated command rejects;
- action-scoped CWS authorization/open timeout bursts;
- strategy safe mode/manual intervention required;
- pending orders/positions after expected EOD flatten.

## Corporate GitLab Migration Checklist

1. Import sanitized GitHub `main` into GitLab.
2. Confirm no raw live journals, raw broker ledgers, or refresh tokens are in
   the imported repository.
3. Add GitLab CI equivalent to GitHub workflow:

```yaml
stages:
  - test
  - build

rust-test:
  stage: test
  image: rust:latest
  services:
    - redis:7-alpine
  script:
    - cd alor-rs-main
    - cargo fmt --all -- --check
    - cargo clippy --all-targets --all-features -- -D warnings
    - cargo test --all

docker-build:
  stage: build
  image: docker:latest
  services:
    - docker:dind
  script:
    - docker build -f Dockerfile.gateway -t $CI_REGISTRY_IMAGE/alor-gateway:$CI_COMMIT_SHA .
    - docker build -f Dockerfile.runtime -t $CI_REGISTRY_IMAGE/strategy-runtime:$CI_COMMIT_SHA .
    - docker push $CI_REGISTRY_IMAGE/alor-gateway:$CI_COMMIT_SHA
    - docker push $CI_REGISTRY_IMAGE/strategy-runtime:$CI_COMMIT_SHA
```

4. Configure protected variables:

```text
ALOR_REFRESH_TOKEN
CI_REGISTRY credentials if needed
```

5. Deploy only from a protected branch/tag.
6. Record deployed git SHA and image tags in the operations journal.
7. Start with micro sizing only.
8. Continue micro soak observation for 2-3 months before small promotion.

## Information That Must Not Be Shared Publicly

Do not share:

- `ALOR_REFRESH_TOKEN`;
- raw broker ledger exports;
- raw live observation journals with account-level details;
- `.env` files;
- private SSH keys;
- internal server passwords.

Can be shared with the corporate deployment team:

- sanitized repository;
- Dockerfiles;
- compose templates;
- TOML configs after portfolio review;
- CI instructions;
- Redis maintenance scripts;
- architecture/runbook docs;
- non-secret image tags and commit SHAs.

## Recommended Next Steps

1. Import sanitized `main` into GitLab.
2. Recreate CI and confirm green `fmt`, `clippy`, `test`, Docker build.
3. Prepare corporate server directories and secret storage.
4. Deploy one stack first in paper/shadow or controlled micro mode.
5. Confirm Redis trim job.
6. Confirm weekly monitoring checklist ownership.
7. Continue micro observation.
8. Revisit small promotion only after 2-3 months of clean micro evidence.
