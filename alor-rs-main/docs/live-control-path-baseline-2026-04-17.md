# Live Control-Path Baseline

Date: 2026-04-17

## Purpose

This memo freezes the current operator-facing live baseline after the extended micro soak.

It exists to reduce ambiguity between:

- historical phase documents,
- older candidate config paths,
- stale project-name references,
- and the actual live contour used during the soak.

Historical notes and phase rollout documents remain valid as evidence, but they should not be treated as the primary source of truth for the current live contour.

## Current Live Stacks

### `trading-sessiongap`

- host directory: `/opt/trading-sessiongap`
- compose project: `trading-sessiongap`
- strategy-runtime container: `trading-sessiongap-strategy-runtime-1`
- alor-gateway container: `trading-sessiongap-alor-gateway-1`
- runtime config: `/configs/runtime.sessiongap.live.7502MIW.toml`
- gateway config: `/configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`

### `trading-hybrid`

- host directory: `/opt/trading-hybrid`
- compose project: `trading-hybrid`
- strategy-runtime container: `trading-hybrid-strategy-runtime-1`
- alor-gateway container: `trading-hybrid-alor-gateway-1`
- runtime config: `/configs/runtime.hybrid.live.7502SN6.action-scoped.toml`
- gateway config: `/configs/gateway.hybrid.live.7502SN6.action-scoped.toml`

### `trading-alor-usdrubf`

- host directory: `/opt/trading-alor-usdrubf`
- compose project: `trading-alor-usdrubf`
- strategy-runtime container: `trading-alor-usdrubf-strategy-runtime-1`
- alor-gateway container: `trading-alor-usdrubf-alor-gateway-1`
- runtime config: `/configs/runtime.alor_usdrubf.live.7502T0U.toml`
- gateway config: `/configs/gateway.alor_usdrubf.live.7502T0U.toml`

## Shared Control-Path Invariants

The live soak baseline confirmed the following operator-level invariants:

- the active live contour is `action_scoped`, not `legacy_long_lived`;
- action-scope authorization is used with forced token refresh before authorize;
- the current VPS uses `trading-*` compose project names for all three stacks;
- Redis memory expansion to `1024m` is part of the current VPS operational baseline.

## What Is Historical, Not Current

The following remain useful as historical evidence, but should not be read as the current live baseline:

- older `legacy_long_lived` live TOML references for `sessiongap` and `hybrid`,
- Phase 1 candidate docs for `sessiongap` action-scoped create/delete only,
- older `alorusdrubf` project-name references for the USDRUBF stack,
- older pre-upgrade Redis memory assumptions (`512m`) from before the VPS / memory expansion work.

Also note:

- the top-level local deploy artifact `bybit_barter_test/.env.sessiongap` still reflects an older local deploy shape and must not be treated as the current VPS source of truth.

## Practical Operator Rule

For current live operations, use this memo together with:

- [alor-gateway-runbook.md](./alor-gateway-runbook.md)
- [strategy-runtime-runbook.md](./strategy-runtime-runbook.md)
- stack-specific live observation memos from the April 2026 soak window

Do not infer the live baseline from older phase documents without checking this memo first.
