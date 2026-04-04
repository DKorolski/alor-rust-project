# Review Memo: Soak Audit Findings

Date: 2026-03-27

Primary artifacts:

- `docs/strategy-runtime-runbook.md`
- `docs/state-and-restarts.md`
- `docs/stage2-soak-review-hybrid.md`
- `docs/create-limit-hardening-2.0-rollout-runbook-2026-03-26.md`
- `docs/AUDIT_AND_ROADMAP_GATEWAY_RUNTIME.md`
- `strategy-runtime/src/runtime.rs`
- `strategy-runtime/src/strategies/hybrid_intraday_runtime.rs`
- `alor-gateway/src/services/command_consumer.rs`
- read-only VPS inspection on `2026-03-27` (`root@155.212.170.21`)

Important:

- this memo records review findings only;
- no code changes are included in this document;
- the goal is to turn the audit into a reviewable decision package before the next soak step.

## 1. Current Review Position

Recommended review posture:

- review is required before continuing the next soak phase;
- `sessiongap` is not blocked from all further testing, but restart/recovery semantics now need explicit engineering review;
- `hybrid` is not ready for live conclusions from the current VPS, because the deployed stack is `paper` and still carries a stale ownership/reconciliation tail;
- one operational security issue is immediate and should be treated as `P0`.

## 2. Scope Of Evidence

This memo is based on:

1. static code audit of:
   - runtime state restore
   - bootstrap snapshot handling
   - ownership tracking
   - health/readiness semantics
2. document audit of:
   - runtime runbooks
   - restart/state notes
   - hybrid soak review
   - gateway/runtime audit notes
3. local verification:
   - `cargo test --workspace --all-targets`
   - `cargo build --workspace`
4. read-only VPS inspection:
   - `docker ps`
   - runtime/gateway health endpoints
   - deployed `.env` and compose files
   - Redis `XREVRANGE` / `XINFO`
   - container logs for the last 24h

## 3. Strongest Current Conclusions

1. The codebase is mature enough for focused engineering review; the next risk cluster is not "basic mechanics" but restart semantics, ownership correlation, deployment drift, and monitoring blind spots.
2. VPS inspection confirmed that at least one code-level concern is operationally real: restart/recovery can still produce `orphan_trade` after transport errors.
3. The current `hybrid` VPS stack must be treated as a paper diagnostic stack, not as a live canary reference.
4. Current "green" signals are insufficient by themselves:
   - Docker `healthy` does not imply runtime `readiness=true`;
   - gateway `readiness=true` does not imply `control_path_stale=false`.
5. The project is in a good place for review, but not yet in a "just continue soak unchanged" place.

## 4. Confirmed Findings

### P0-1. Secrets Exposure On VPS

Observed on VPS:

- `/opt/trading-hybrid/.env` permissions: `0644`
- `/opt/trading-sessiongap/.env` permissions: `0644`
- both files contain live refresh-token material

Why this matters:

- any local user or process on the host can read live auth material;
- backup files (`.env.bak*`) multiply exposure;
- this is an operational security issue independent of application correctness.

Required action:

- immediately restrict permissions;
- rotate the affected tokens after cleanup;
- move secrets out of world-readable env files.

---

### P0-2. Restart Correlation Gap Is Real, Not Theoretical

Code evidence:

- `strategy-runtime/src/runtime.rs`: `pending_request_ids()` returns `Vec::new()` for `HybridIntradayRuntime`
- `strategy-runtime/src/runtime.rs`: ack-to-order ownership relies on `our_request_ids`

Operational evidence:

- on `2026-03-27 07:23 UTC`, the restarted `sessiongap` runtime logged multiple:
  - `command rejected ... cws disconnected: protocol_reset_without_close_handshake`
  - followed by multiple `orphan_trade`

Interpretation:

- recovery paths are still capable of losing ownership correlation across restart/recovery boundaries;
- the audit finding is therefore not only code-theoretical but already observable in a live-like contour.

Review consequence:

- restart/recover semantics should be treated as a blocking review area before expanding live soak.

---

### P1-1. Ownership Model Is Internally Inconsistent For `hybrid`

Code evidence:

- bootstrap currently inserts every working order for the symbol into runtime ownership tracking;
- `hybrid` strategy itself is designed to adopt only tagged "our" orders.

Docs evidence:

- `docs/stage2-soak-review-hybrid.md` frames tagging/ownership as "filter and adopt only our orders/stop-orders".

Risk:

- foreign or manual same-symbol orders can contaminate:
  - runtime ownership,
  - ledger classification,
  - trade attribution,
  - recovery semantics.

This is especially relevant on shared live accounts or during manual operator intervention.

---

### P1-2. `hybrid` Deployment On VPS Is `paper`, Not `live`

Observed on VPS:

- `/opt/trading-hybrid/.env` points to `RUNTIME_CONFIG=/configs/runtime.hybrid.paper.7502SN6.toml`
- runtime readiness returned `503`
- runtime reasons were:
  - `trade_mode=Paper`
  - `allow_live_orders=false`

Consequence:

- the current VPS `hybrid` stack is a paper/live-only execution contour;
- it must not be used as evidence that `hybrid live` is currently accepted or rejected;
- paper and live conclusions need to stay clearly separated in review.

---

### P1-3. `hybrid` Persisted State Is Stale Against Broker Snapshot

Observed on VPS:

- runtime state stream persists:
  - `last_position_qty = -1.0`
  - `safe_mode_close_only = true`
  - `safe_mode_reason = "recovered_position_owner_unknown"`
- current broker snapshot for the same stack shows:
  - `IMOEXF qty = 0.0`

Interpretation:

- an ownership/reconciliation tail remains unresolved, or at minimum insufficiently observable;
- this matches the earlier known issue line and should still be treated as open.

Operational consequence:

- the stack can look "running" while still carrying stale strategic state that invalidates conclusions.

---

### P1-4. Docker Healthchecks Are Liveness-Only

Observed on VPS compose:

- gateway healthcheck uses `/liveness`
- runtime healthcheck uses `/liveness`

Operational evidence:

- `hybrid-strategy-runtime-1` was Docker-healthy while runtime `/readiness` returned `503`.

Risk:

- orchestration and dashboards can report green while the strategy is intentionally blocked;
- operators can infer "stack healthy" when the trading contour is not actually ready.

Required review decision:

- decide whether container `healthy` should mean:
  - process alive only,
  - or operationally ready.

---

### P1-5. `sessiongap` Still Shows Transport Instability In The Live Contour

Observed on VPS gateway/runtime logs within the last 24h:

- repeated CWS `protocol_reset_without_close_handshake`
- EOF disconnects
- WS subscribe retry exceeded
- runtime replay/recovery against old ack/trade material on restart

Interpretation:

- instrumentation is materially better than before;
- the transport layer is still not "boringly stable";
- restart/recovery and operational guardrails remain part of the real risk surface.

---

### P2-1. Readiness Does Not Capture Stale Limit-Path Risk

Observed on VPS:

- both gateways returned `readiness=true`;
- both gateways simultaneously reported `control_path_stale=true`.

Interpretation:

- `HTTP 200` readiness alone is not enough to assert first-post-idle limit-path safety;
- the hardening path may still recover on send via recycle, but operator semantics must reflect that.

Operational note:

- "gateway ready" and "limit control path fresh" are not currently the same statement.

---

### P2-2. Redis Consumer Lists Accumulate Stale Runtime Consumers

Observed on VPS:

- `strategy-runtime-sessiongap-local` retains many inactive consumer identities with zero pending entries.

Interpretation:

- not a correctness defect by itself;
- but it adds operator noise and complicates stream forensics during incidents.

This should be treated as operational cleanup debt.

---

### P2-3. Documentation Drift Remains Around Restore Flow

Docs currently describe one restore order, while runtime bootstrap invokes snapshot notification before `on_runtime_state_restored`.

Impact:

- restart analysis and operator reasoning can diverge from actual execution order;
- this is lower priority than security/recovery, but still worth fixing in the same review wave.

## 5. What Is Already Strong

- local workspace build and test suite are green;
- gateway hardening line exists and is observable;
- `sessiongap` live stack is up and currently `ALLOWED`;
- `hybrid` paper stack is up and emitting runtime-state snapshots;
- health/readiness endpoints exist and are useful;
- Redis/runtime-state inspection is feasible in the deployed environment.

These are meaningful strengths. The review is about narrowing the next blocking risks, not about starting from zero.

## 6. Review Questions To Settle

1. Is the next planned phase:
   - `sessiongap live soak`,
   - `hybrid paper soak`,
   - or a coordinated move of `hybrid` into live canary?
2. Is the project willing to treat restart/ownership fixes as blocking `P1` before broader live expansion?
3. Should container `healthy` mean:
   - process alive,
   - or strategy operationally ready?
4. Is `control_path_stale=true` acceptable behind `readiness=true`, or should it be elevated into the operational gate?
5. Who owns VPS secret hygiene and secret rotation after the review?

## 7. Recommended Gate Before The Next Soak Window

Require all of the following:

- `P0` secrets fix completed and tokens rotated;
- explicit decision on `hybrid paper` vs `hybrid live` stack role;
- review sign-off on restart/ownership findings;
- documented monitoring semantics for liveness vs readiness;
- targeted re-test plan for:
  - `sessiongap` restart after old ack/trade material,
  - `hybrid` ownership reconciliation,
  - first-post-idle limit command on stale control path.

## 8. Immediate Actions

### 8.1 Ops

- restrict permissions on env files and backups;
- rotate exposed refresh tokens;
- decide whether readiness must be part of container health semantics;
- document the actual role of the current `hybrid` VPS stack.

### 8.2 Code And Review

- fix pending-request restoration for `hybrid` runtime;
- fix ownership bootstrap rules to respect tag-based adoption;
- add focused restart/recovery e2e coverage for `hybrid` and `sessiongap`;
- align docs with the real bootstrap/restore order.

### 8.3 Retest

- rerun `sessiongap` restart/recovery scenario on VPS;
- rerun `hybrid` from clean state, or with controlled `reset_state_on_start=true` for diagnostic purposes only;
- re-evaluate soak gate only after fresh artifacts are captured.

## 9. Bottom Line

This package is review-worthy now because the key risks are concrete, reproducible, and already split into:

- code-level recovery/ownership issues;
- deployment drift;
- operational monitoring gaps;
- and VPS secret hygiene.

Recommended overall position:

- do not continue as if the system is fully soak-ready;
- do continue with a formal review and a short blocking-fixes wave;
- treat the current VPS findings as sufficient evidence to prioritize recovery, ownership, and secret hygiene ahead of broader live expansion.
