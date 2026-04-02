# Hybrid Action-Scoped Live Soak Runbook

Date: 2026-04-02

## Goal

Move `hybrid` onto the control path that can actually use:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`

without reusing stale startup Redis backlog.

## What Changed

Runtime-side:

- `hybrid` startup replay guard suppresses stale `origin=Live` backlog bars after restart
- stale pending tail is cleared on boot when flat and no working orders exist
- live `Entry` and live `Exit` can emit `Place` intents when `live_order_style = "marketable_limit"`
- emergency live exits (`sl_triggered_escalation`, `repair_deadline_force_flatten`) also use `Place`

Config-side:

- gateway candidate: `/configs/gateway.hybrid.live.7502SN6.action-scoped.toml`
- runtime candidate: `/configs/runtime.hybrid.live.7502SN6.action-scoped.toml`

## Why This Rollout Is Different

The previous live attempt failed because `hybrid` restarted into old Redis `origin=Live` backlog
and re-emitted a stale signal.

This rollout uses:

- a fresh runtime consumer group
- a fresh runtime state stream
- runtime startup replay suppression
- marketable-limit `Place` intents so gateway action-scoped routing is actually exercised

## VPS Target State

Environment under `/opt/trading-hybrid/.env`:

- `GATEWAY_CONFIG=/configs/gateway.hybrid.live.7502SN6.action-scoped.toml`
- `RUNTIME_CONFIG=/configs/runtime.hybrid.live.7502SN6.action-scoped.toml`
- `GATEWAY_IMAGE_TAG=<existing action-scoped capable gateway image>`
- `RUNTIME_IMAGE_TAG=<new hybrid runtime image>`

## Rollout Steps

1. Backup current hybrid env.
2. Copy both candidate config files into `/opt/trading-hybrid/configs/`.
3. Switch only:
   - `GATEWAY_CONFIG`
   - `RUNTIME_CONFIG`
   - `RUNTIME_IMAGE_TAG`
4. Recreate `strategy-runtime`.
5. Recreate `alor-gateway` only if gateway image/config changed.
6. Wait until both `gateway` and `runtime` are healthy.

## Preflight Checks

Before leaving the stack in soak:

- gateway `/readiness` is `LiveReady`
- runtime `/readiness` is `LiveReady / ALLOWED` or an expected warmup-ready state
- first startup bars do not emit immediate stale commands
- runtime logs show startup replay suppression first, then release on fresh live bar
- any live `Entry/Exit` commands land in `cmd.orders.7502SN6` as `place`
- gateway logs show action-scoped authorize/send path for those commands

## Evidence To Capture

- `/readiness` from gateway and runtime
- `cmd.orders.7502SN6`
- `cmd.acks.7502SN6`
- `broker.orders.7502SN6`
- `broker.positions.7502SN6`
- runtime logs:
  - `hybrid_startup_replay_guard_armed`
  - `hybrid_startup_replay_bar_suppressed`
  - `hybrid_startup_replay_guard_released`
- gateway logs:
  - `invalidated cached alor access token`
  - `refreshed alor access token consumer="action_scope_cws_authorize"`
  - `action_scope_authorize_ok ... access_token_source="refreshed"`

## Rollback

If startup emits stale intents or gateway/runtime readiness regresses:

- restore `/opt/trading-hybrid/.env` from backup
- recreate `strategy-runtime`
- recreate `alor-gateway` if needed
- confirm `RUNTIME_CONFIG` is back on paper baseline

