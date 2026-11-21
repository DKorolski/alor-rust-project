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
- `testing-redis-isolation.md` - isolated test environments, opt-in Redis E2E
  coverage and the explicitly retained legacy baseline test gap.
- `safe-build-cache-maintenance.md` - reviewed target/venv/Docker cache cleanup,
  rollback protection and before/after guards; separate from Redis retention.

## Isolated Shadow Preparation

These documents describe opt-in engineering/validation, not automatic promotion
or a replacement for the production live gateway:

- `shadow-observation-and-live-promotion.md` - isolated observation and promotion
  boundaries, including legacy contour retention.
- `shadow-md-ws-source-runbook-2026-11-02.md` - MD-only WS source and terminal
  collection, with separate source/model readiness.
- `shadow-ordinary-model-budget-2026-11-04.md` - ordinary collection/processing,
  queue and unchanged terminal budgets.
- `reaction-shadow-inactive-worker-preflight-2026-11-04.md` - inactive worker,
  read-only preparation, reviewed bootstrap and explicit future admission.
- `reaction-shadow-paired-admission-v2-2026-11-06.md` - separate IMOEXF/USD
  admission with immutable as-observed bars, reviewed revisions and weekend tail.
- `shadow-late-context-recovery-2026-11-08.md` - opt-in source v4 / admission v3,
  audited late context without back-insertion or model replay; successor gates.
- `shadow-review-repair-and-grant-2026-11-09.md` - MD-only remote Close/EOF repair,
  portable policy fixtures and opt-in durable admission grant; rollout boundaries.
- `shadow-diagnostic-late-start-2026-11-09.md` - separately granted same-day
  diagnostic recovery, original receipt clocks and first-day clean-count exclusion.
- `shadow-source-version-index-2026-11-09.md` - WS/REST source generations,
  admission entrypoints and historical vs current validation contracts.
- `shadow-strict-recovery-work-plan-2026-11-10.md` - bounded source recovery,
  strict versus separately approved diagnostic admission and evidence reuse.
- `finam-new-shadow-models-handoff-2026-11-13.md` - broker-neutral porting
  contract for the three paired shadow models; private bundles stay external.
- `finam-shadow-operational-stability-handoff-2026-11-19.md` - recovery, terminal,
  immutable input and acceptance patterns for a separately operated broker contour.
- `shadow-admission-evidence-permissions-hardening-2026-11-16.md` - cross-UID
  read-only evidence staging and safe failure diagnostics.
- `shadow-context-aware-admission-work-plan-2026-11-15.md` - reviewed context
  dependencies; transport reconnect or audit growth alone is not a HOLD rule.
- `shadow-normal-source-reuse-admission-contract-2026-11-16.md` - opt-in v4
  pre-open source reuse and same-generation resume without grant renewal.
- `reaction-shadow-ws-recovery-contract-2026-11-19.md` - split collect/advance,
  required-bar deadlines and recovery for diagnostic as well as strict workers.

## Experimental Calendar And OI

These opt-in libraries/tools do not alter live strategies or admit new model
arms. A successful build/probe is not prospective model or live acceptance:

- `moex-calendar-context-oi-shadow-engineering-tz-2026-11-15.md` - frozen model,
  calendar, separate MR/BO contexts, OI and prospective acceptance requirements.
- `moex-calendar-oi-stage1-progress-2026-11-15.md` - implemented slices, executed
  verification boundaries and remaining model-admission work.
- `imoexf-component-session-bridge-contract-2026-11-16.md` - immutable receipt
  journal, component state/event traces and calendar-aware session coordinator.
- `oi-shadow-collect-only-runbook-2026-11-20.md` - optional `shadow-oi-http`
  collector, non-PIT probe, dated capture schedule and strict host-clock checks.

Synthetic native fixtures required by these tests are in `strategy-runtime/tests/fixtures`.
Full historical exports and raw OI/soak reports remain external or ignored.

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
