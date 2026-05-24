# Documentation Index

This sanitized documentation set is intended for corporate DevOps and project
handoff. Historical observation journals, raw live incident notes, replay
result dumps, VPS-specific logs, and ad-hoc research artifacts are intentionally
omitted to avoid operational confusion.

## Start Here

- `corporate-handoff-project-overview-2026-05-22.md` - project overview,
  deployment model, runtime components, strategy list, testing status, and
  maintenance procedures.
- `youtrack-corporate-rollout-task-2026-05-22.md` - ready-to-copy task text for
  corporate rollout tracking.
- `devops-paper-runbook.md` - paper/pre-live operational runbook.
- `strategy-runtime-runbook.md` - strategy runtime operations.
- `alor-gateway-runbook.md` - gateway operations.

## Runtime And Operations

- `state-and-restarts.md` - restart and state semantics.
- `redis-runtime-state-and-snapshots.md` - Redis runtime state and snapshot
  layout.
- `redis-retention-and-cleanup-plan-2026-04-21.md` - Redis retention and safe
  cleanup policy.
- `live-runtime-service-patterns-anti-regression-checklist-2026-05-07.md` -
  shared live service patterns and anti-regression checklist.
- `failure-test-matrix.md` - failure scenarios and expected behavior.

## Strategy And Deployment Contracts

- `live-control-path-baseline-2026-04-17.md` - live action-scoped control path
  baseline.
- `intent-path-unification-fix-plan-2026-04-17.md` - shared command path
  unification plan.
- `request-id-skew-and-deferred-exit-fix-plan-2026-04-18.md` - request-id and
  deferred-exit safety plan.
- `ri-author41-42-live-contract-2026-05-01.md` - RI live contract.
- `ri-author41-42-live-micro-contour-plan-2026-05-01.md` - RI micro contour
  plan.
- `imoexf-hybrid-mr-bo-handoff-2026-04-26.md` - IMOEXF hybrid MR/BO profile
  handoff.

## Seed And Config Artifacts

Runtime-required seed files are kept in `../configs/`, not in `docs/`:

- `../configs/riskgate_high180_lb120_seed_2026-04-26.csv`
- `../configs/riskgate_high180_lb120_seed_2026-04-26_metadata.json`

Do not place refresh tokens, `.env` files, raw broker ledgers, private SSH keys,
or raw live observation journals in this repository.
