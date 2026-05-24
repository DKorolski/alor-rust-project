# Pre-Live Config Audit

Date: 2026-03-27

## 1. Goal

Before the next controlled live window, confirm that the VPS launch path points to the real trading contour, not to a diagnostic or acceptance-only scenario used for limit/cancel, single, loop, idle/cadence/recycle checks.

## 2. Scope

In scope:

- `sessiongap` as the candidate contour for the next controlled live window;
- `hybrid` only as a role check: confirm whether it is still paper/diagnostic and therefore out of the live window.

Out of scope:

- new `create:limit` engineering audit;
- new restart/ownership soak;
- new payload experiments;
- additional limit/cancel checks inside the working window.

## 3. Evidence Used

- local runbooks and configs:
  - `docs/strategy-runtime-runbook.md`
  - `docs/create-limit-hardening-2.0-rollout-runbook-2026-03-26.md`
  - `configs/runtime.sessiongap.live.7502MIW.toml`
  - `configs/gateway.sessiongap.live.7502MIW.toml`
  - `configs/runtime.hybrid.paper.7502SN6.toml`
- read-only server verification on the target host:
  - `docker ps`
  - `.env`
  - `docker-compose.yml`
  - `docker compose config`
  - `docker inspect`
  - mounted config files
  - `/readiness`
  - startup log excerpts
  - grep for automated helper references in compose/systemd/cron

Artifacts bundle:

- Historical pre-live audit artifacts are intentionally omitted from the
  sanitized corporate handoff branch.

## 4. Exact Stack Answer

### 4.1 Candidate live contour

The intended live contour is:

- compose project: `sessiongap`
- services: `sessiongap-alor-gateway-1`, `sessiongap-strategy-runtime-1`
- runtime config path from `.env`: `/configs/runtime.sessiongap.live.7502MIW.toml`
- gateway config path from `.env`: `/configs/gateway.sessiongap.live.7502MIW.toml`

The mounted runtime config is a real live strategy config:

- `strategy_id = "session_gap_standalone"`
- `strategy_kind = "session_gap_standalone"`
- `trade_mode = "live"`
- `allow_live_orders = true`
- `allow_paper_orders = false`
- `portfolio = "7502MIW"`
- `exchange = "MOEX"`
- `symbol = "USDRUBF"`
- `tf_sec = 60`
- `reset_state_on_start = false`

The runtime startup logs confirm the same launch identity:

- loaded config: `/configs/runtime.sessiongap.live.7502MIW.toml`
- strategy started: `session_gap_standalone`
- `trade_mode = Live`
- `allow_live_orders = true`

### 4.2 Hybrid contour role

`hybrid` is not part of the next live window in its current VPS state.

The effective launch path is:

- compose project: `hybrid`
- runtime config path from `.env`: `/configs/runtime.hybrid.paper.7502SN6.toml`
- gateway config path from `.env`: `/configs/gateway.hybrid.live.7502SN6.toml`

The runtime startup logs and readiness confirm:

- `strategy_id = "hybrid_intraday"`
- `strategy_kind = HybridIntraday`
- `trade_mode = Paper`
- `allow_live_orders = false`
- runtime readiness is `false`
- `live_guard_reasons = ["trade_mode=Paper", "allow_live_orders=false"]`

Conclusion for `hybrid`:

- keep treating it as a separate paper diagnostic contour;
- do not use it as evidence that `hybrid live` is ready or selected for the next live window.

## 5. What Was Confirmed

### 5.1 Runtime really points to the trading strategy

For `sessiongap`, this is confirmed by three layers at once:

1. `.env` points to `runtime.sessiongap.live.7502MIW.toml`.
2. bind mount maps `/opt/trading-sessiongap/configs -> /configs`.
3. runtime startup logs explicitly say:
   - loaded config `/configs/runtime.sessiongap.live.7502MIW.toml`
   - `strategy_id = "session_gap_standalone"`
   - `trade_mode = Live`
   - `allow_live_orders = true`

### 5.2 Gateway runs as the normal transport process

For `sessiongap`, the compose command is the normal runner:

- `alor_gateway_transport_runner --config /configs/gateway.sessiongap.live.7502MIW.toml --redis-url redis://redis:6379/`

The gateway startup logs confirm:

- config path `/configs/gateway.sessiongap.live.7502MIW.toml`
- portfolio `7502MIW`
- symbol `USDRUBF`
- startup path is the normal transport runner, not a helper script

### 5.3 Diagnostic helpers are not wired into the live launch path

No automatic references were found in:

- `/opt/trading-sessiongap`
- `/opt/trading-hybrid`
- `/etc/systemd/system`
- `/etc/cron.d`
- root crontab

The search specifically checked for:

- `limit_diag.sh`
- `fresh_probe`
- `stale_probe`
- `single`
- `loop`
- `create:limit`
- `delete:limit`

Also confirmed:

- runtime command is `strategy_runtime_runner`, not a helper wrapper
- gateway command is `alor_gateway_transport_runner`, not a helper wrapper
- `RUNTIME_ENABLE_TEST_HOOKS=false`
- no helper strings were found in the checked startup log excerpts

### 5.4 Hardening 2.0 is active as a protective layer

The effective gateway process exposes:

- `control_path_stale_after_sec = 900`
- `control_path_pre_entry_recycle_enabled = true`
- `control_path_recycle_timeout_ms = 5000`
- `control_path_hardening_log_only = false`

This confirms hardening is active in the running gateway process and is not a separate synthetic loop.

Important nuance:

- on VPS, the mounted gateway config file does not explicitly contain `control_path_*` keys;
- gateway startup logs show these values are currently coming from defaults in the gateway build, not from the mounted `.toml`.

So hardening is active, but not declaratively pinned in the mounted config file.

## 6. Excluded Leftovers

### Confirmed absent from automatic launch

- `limit_diag.sh` is not referenced by compose command, systemd, cron, or root crontab
- no automatic `single` / `loop` helper launch was found
- no acceptance-only probe strings were found in the checked startup logs
- no compose entrypoint/command was found that injects synthetic limit/cancel probes

### Present, but not part of the active launch path

- old `.env.bak*` files exist under both `/opt/trading-sessiongap/` and `/opt/trading-hybrid/`
- these backup files are world-readable (`0644`)

This means:

- they are not the active compose input we observed;
- but they are still risky leftovers and should be cleaned up or locked down before the next live window.

## 7. Critical Drift Found

### 7.1 The current running runtime image is not the same as the compose-resolved runtime image

For `sessiongap`:

- current running container:
  - `sessiongap-strategy-runtime-1 -> ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`
- current `docker compose config` resolution:
  - `strategy-runtime -> ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-774b917-diag-20260326`

The same mismatch exists for `hybrid`.

Interpretation:

- the VPS is currently running an older runtime container than the one implied by the current `.env` and compose resolution;
- if operators do a standard recreate or `docker compose up -d` with the current files, runtime may switch to a different image than the one currently running;
- therefore the answer to "what exact stack will go into the window" is not yet immutable.

This is the main blocker for closing this pre-live audit as `ready`.

### 7.2 The gateway image tag still carries a diagnostic-looking name

Current gateway image:

- `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-774b917-diag-20260326`

What this does and does not mean:

- it does not, by itself, prove that helper scripts are wired into the live loop;
- the process actually launched is still the normal transport runner;
- however, the tag name is confusing for a production gate and should be explicitly approved or renamed/pinned before the next controlled live window.

### 7.3 Hardening is effective, but not config-pinned

This is not the same as a diagnostic helper being in the trading loop.

But it does mean:

- the live protective behavior currently depends on gateway build defaults;
- the mounted gateway file on VPS is not sufficient on its own to explain the active hardening behavior.

For a pre-live gate, that is a configuration clarity issue.

## 8. Strongest Conclusion

Current verdict:

- `sessiongap` is confirmed as the intended live strategy contour at the config level;
- `hybrid` is confirmed as paper-only and out of the next live window;
- hardening 2.0 is active and diagnostic helpers are not wired into the live launch path;
- but the stack is **not ready to be called fully pre-live-clean**, because config/image drift still exists.

Recommended status:

- `not ready / config drift found`

Reason:

- there is not yet one fully unambiguous answer for the next window if a normal compose recreate happens, because the current running runtime image and the compose-resolved runtime image are different.

## 9. Required Closure Before The Window

1. Freeze the exact runtime image for `sessiongap`.
   - either keep the current runtime intentionally and do not recreate it;
   - or pin the runtime image/tag explicitly and recreate once, then re-verify.
2. Decide whether the gateway image tag with `-diag-20260326` is the approved production hardening build name.
3. Pin `control_path_*` behavior explicitly in the mounted gateway config, or document that build defaults are the approved source of truth.
4. Lock down or remove `.env.bak*` leftovers and fix permissions on active env files.
5. Run one short read-only verification after the final image/tag decision:
   - `docker ps`
   - `docker compose config`
   - runtime `/readiness`
   - gateway `/readiness`
   - first startup log lines after the final recreate, if a recreate is performed

## 10. Bottom Line

The audit answered the main question, but with one important caveat:

- the live strategy is indeed `session_gap_standalone` in `live` mode with `allow_live_orders=true`;
- hardening is present;
- diagnostic helpers are not auto-injected into the trading contour;
- `hybrid` remains paper;
- however, the runtime image currently running on VPS is not the same image that the present compose files would resolve on recreate.

Until that drift is removed, the pre-live answer is still "close, but not yet clean enough to sign off as exact and final".
