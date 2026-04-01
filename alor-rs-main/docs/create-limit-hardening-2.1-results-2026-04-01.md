# Hardening 2.1 Results: Exit Recovery Lane After `control_path_recycle_failed`

Date: 2026-04-01

Related documents:

- `docs/create-limit-hardening-2.0-results-2026-03-26.md`
- `docs/soak-start-state-2026-03-30.md`
- `docs/live-incident-note-2026-03-31-sessiongap-exit-recycle-timeout.md`

## 1. Purpose

This change implements the first exit-specific hardening step after the live `sessiongap` incident of 2026-03-31.

Target behavior:

- keep stale-path send blocking for `Entry`;
- introduce a distinct recovery lane for risk-reducing `Exit`;
- avoid the old dead-end `PendingExit -> Blocked` path on `control_path_recycle_failed`;
- expose the degraded exit state through runtime readiness and state snapshots.

## 2. Policy Implemented

### 2.1 Entry policy remains fail-closed

For stale live `Entry` sends:

- stale detection still runs in gateway;
- recycle-before-send still protects the command path;
- failed recycle still blocks the send.

No relaxed entry semantics were introduced in `2.1`.

### 2.2 Exit policy now uses a separate recovery lane

For stale live `Exit` sends:

1. gateway still detects stale control-path and tries recycle-before-send;
2. if recycle succeeds, the exit is sent normally;
3. if recycle fails with `control_path_recycle_failed`, runtime does not go directly to ordinary `Blocked`;
4. runtime enters `ExitRecoveryPending`;
5. runtime emits one bounded retry for the same exit intent;
6. if the retry also fails with `control_path_recycle_failed`, runtime enters `CloseOnlyDegraded`;
7. runtime marks the strategy as operator-required instead of silently dead-ending.

Current conservative policy choice:

- no market-flatten fallback was enabled;
- no unsafe stale-path send was allowed for exit;
- the system now prefers explicit degraded visibility over silent dead-end blocking.

### 2.3 Current retry and timeout values

Current implemented values on this line:

- `EXIT_RECOVERY_MAX_RETRIES = 1`
- `control_path_pre_entry_recycle_enabled = true`
- `control_path_pre_exit_recycle_enabled = true`
- `control_path_recycle_timeout_ms = 5000`
- `control_path_recycle_timeout_ms_exit = 10000`

Important note:

- `control_path_recycle_timeout_ms_exit = 10000` is currently coming from code/default resolution on this candidate line;
- `configs/gateway.sessiongap.live.7502MIW.toml` still explicitly pins only the entry timeout fields from `2.0`;
- the retry count is still code-local, not config-driven.

## 3. What Was Implemented

Primary implementation areas:

- `strategy-runtime/src/state.rs`
- `strategy-runtime/src/runtime.rs`
- `strategy-runtime/src/health_server.rs`
- `strategy-runtime/src/lib.rs`
- `strategy-runtime/src/strategies/session_gap_standalone.rs`
- `alor-gateway/src/config.rs`
- `alor-gateway/src/services/command_consumer.rs`
- `alor-gateway/src/supervisor.rs`

New runtime states:

- `ExitRecoveryPending`
- `CloseOnlyDegraded`

New runtime readiness fields:

- `exit_recovery_active`
- `close_only_degraded`
- `operator_intervention_required`
- `open_risk_position_unflattened`

Structured runtime logs added for the exit lane:

- `exit_recovery_started`
- `exit_recycle_retry_started`
- `exit_recycle_retry_failed`
- `exit_close_only_degraded_entered`
- `exit_operator_intervention_required`

## 4. Local Verification

Executed locally:

```bash
cargo build --workspace
cargo test --workspace --all-targets
```

Additional direct coverage was added in `session_gap_standalone` for:

- first `control_path_recycle_failed` on exit enters `ExitRecoveryPending`;
- second `control_path_recycle_failed` enters `CloseOnlyDegraded` with operator-required semantics.

## 5. Candidate Rolled To VPS

Rolled line on 2026-04-01:

- gateway image:
  - `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-cf913bd-exit21-20260401-005046`
- runtime image:
  - `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-cf913bd-exit21-20260401-005046`

Deployment scope:

- `sessiongap` only;
- `hybrid` remained on the previous fixed pair and stayed in `paper`.

Clean-start actions performed before the rollout:

- `sessiongap` containers recreated;
- `sessiongap` Redis volume contents removed;
- `sessiongap` reports directory cleared.

## 6. Clean-Deploy Observations On VPS

### 6.1 Startup was not broker-clean even after Redis cleanup

Immediately after the clean deploy, startup logs still replayed historical broker events:

- `orphan_trade` for an old `existing=true` sell;
- `order filled awaiting execution` for the same historical order.

Interpretation:

- local Redis cleanup was effective;
- but the live broker/event contour still replayed historical order/trade tails;
- therefore the VPS was not a fully sterile broker-level clean slate.

### 6.2 Natural live entry succeeded

Observed on 2026-04-01:

- `09:00:00 UTC` (`12:00:00 MSK`)
- `Flat -> PendingEntry`
- `intent_emitted action="place"`
- command accepted:
  - `request_id = da403120-5869-534a-9edc-fd43dc8f74ae`
  - `broker_order_id = 2023555952971770062`
- `PendingEntry -> InPosition`

Broker evidence:

- order filled `sell 1.0 @ 80.57`
- trade id:
  - `2023555952971623175`

### 6.3 Exit recovery lane was exercised live

Later on 2026-04-01:

- `15:04:03 UTC` (`18:04:03 MSK`)
- strategy emitted a natural risk-reducing exit;
- state transition:
  - `InPosition -> PendingExit`
- exit request id:
  - `75bc4462-1118-5587-8303-01db5c868627`

First failure:

- `15:04:14 UTC`
- `command rejected`
- `error_code = control_path_recycle_failed`
- `error_msg = fresh cws session was not ready before recycle timeout`

New 2.1 behavior then happened exactly as designed:

- `exit_recovery_started`
- `exit_recycle_retry_started`
- state transition:
  - `PendingExit -> ExitRecoveryPending`
- retry request id:
  - `94911821-5284-5c4c-a07e-63be062aaf77`

Second failure:

- `15:04:25 UTC`
- retry also failed with:
  - `error_code = control_path_recycle_failed`

Final runtime outcome:

- `exit_recycle_retry_failed`
- `exit_close_only_degraded_entered`
- `exit_operator_intervention_required`
- state transition:
  - `ExitRecoveryPending -> CloseOnlyDegraded`

### 6.4 Runtime no longer dead-ended in ordinary `Blocked`

This is the most important confirmed behavior change versus the 2026-03-31 incident.

Observed current runtime readiness:

- `runtime_phase = "CloseOnlyDegraded"`
- `close_only_degraded = true`
- `operator_intervention_required = true`
- `open_risk_position_unflattened = true`

Observed current `runtime.state`:

- phase:
  - `CloseOnlyDegraded`
- side:
  - `buy`
- qty:
  - `1.0`
- `reason = "ack_failed:Error:tp"`
- `retry_attempts_exhausted = 1`
- `last_error_code = "control_path_recycle_failed"`

Conclusion:

- `2.1` succeeded in replacing the old silent dead-end with an explicit exit-risk degraded state.

### 6.5 Operationally the position still remained open

Broker stream inspection after the failed exit showed:

- `USDRUBF qty = -1.0`
- `avg_price = 80.57`

Orders/trades evidence showed:

- the live entry was sent and filled;
- no exit order reached the broker after the recycle failures;
- gateway `create:limit` send count remained consistent with entry only.

Conclusion:

- `2.1` improved state-machine behavior and observability;
- `2.1` did not yet achieve unattended safe flatten when recycle failed twice.

## 7. Hybrid Logging Observation

The clean `hybrid` stack is not writing to `journald`.

Observed on VPS:

- Docker logging driver:
  - `json-file`
- `trading-hybrid-strategy-runtime-1` log config:
  - `type=json-file`
- `trading-hybrid-alor-gateway-1` log config:
  - `type=json-file`
- `journalctl` produced no useful application log hits for the active `hybrid` stack.

Practical implication:

- `hybrid` logs should be read via `docker logs`;
- `journald` is not the primary source for the current containerized `hybrid` deployment.

## 8. Strongest Conclusion

`Hardening 2.1` passed the main semantic goal of this stage:

- a failing stale exit no longer goes straight into ordinary `Blocked`;
- the system now exposes a deterministic `ExitRecoveryPending -> CloseOnlyDegraded -> operator_required` outcome.

But the line has not yet solved the full unattended live safety problem:

- the risk-reducing exit still did not reach the broker after repeated recycle timeout;
- the broker position remained open;
- operator intervention was still required.

Short form:

- state machine improved;
- observability improved;
- unattended live exit safety is still not fully solved.

## 9. Recommended Next Step

Before the next unattended `sessiongap` live run:

- preserve this rollout as the reference `2.1` outcome;
- treat `CloseOnlyDegraded + operator_required` as the new correct degraded state;
- decide whether `2.2` should introduce:
  - more than one configurable retry,
  - a stronger flatten-only fallback,
  - or a separate operator-assisted close-only dispatch policy.
