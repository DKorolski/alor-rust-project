# VPS USDRUBF Stack Drift (2026-04-08)

## Scope

Operational review of the live `AlorUsdrubfHybrid` VPS layout after the successful 2026-04-07 soak.

## Findings

### 1. Repo config drift vs live VPS config

`configs/gateway.alor_usdrubf.live.7502T0U.toml` in the repository had drifted behind the live VPS contour.

Live VPS config currently uses:

- `control_cws_mode = "action_scoped"`
- `action_scope_enable_create_limit = true`
- `action_scope_enable_delete_limit = true`
- `action_scope_enable_replace_limit = false`
- `action_scope_enable_exit = true`
- `action_scope_force_token_refresh_before_authorize = true`

This contour matches:

- `docs/alor-usdrubf-7502T0U-smoke-checklist.md`
- `docs/alor-usdrubf-local-bringup-report-2026-04-06.md`
- observed VPS logs from 2026-04-07

Repository config was updated on 2026-04-08 to match the live contour.

### 2. Duplicate compose projects from the same host directory

On VPS, two compose projects are currently active from the same working directory `/opt/trading-alor-usdrubf`:

- project `alorusdrubf`
- project `trading-alor-usdrubf`

Observed containers:

- `alorusdrubf-strategy-runtime-1`
- `alorusdrubf-alor-gateway-1`
- `trading-alor-usdrubf-alor-gateway-1`
- `trading-alor-usdrubf-redis-1`

This is not a safe steady state for further rollout work because:

- the duplicate gateway stack can subscribe/publish against the same broker and Redis contour,
- the duplicate project creates ambiguity about the canonical operational target,
- updating only one project may leave the other live and partially shadowing the expected behavior.

### 3. Canonical live stack choice before next rollout

The current active runtime for `AlorUsdrubfHybrid` is under project `alorusdrubf`:

- `alorusdrubf-strategy-runtime-1`
- `alorusdrubf-alor-gateway-1`

This stack showed:

- healthy runtime logs,
- flat end-of-day state after the 2026-04-07 exit retry,
- persisted runtime state under `runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U`.

The extra `trading-alor-usdrubf` gateway/redis pair should be treated as cleanup debt before the next production rollout.

## Recommended cleanup order

1. Confirm `alorusdrubf` is the canonical project name for the live USDRUBF stack.
2. Stop and remove only the duplicate `trading-alor-usdrubf` project.
3. Re-check:
   - `docker ps`
   - gateway/runtime health
   - Redis runtime-state tail
   - current image tags
4. Only after that perform the next rollout against the canonical `alorusdrubf` project.

## Rollout note

For the next rollout, prefer an explicit image tag already proven by CI/main merge rather than relying on floating tags.
This keeps the deployment deterministic even if the publish workflow lives outside the repo snapshot currently checked out locally.
