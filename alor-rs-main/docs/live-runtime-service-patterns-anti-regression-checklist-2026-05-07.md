# Live Runtime Service Patterns And Anti-Regression Checklist

Date: 2026-05-07

Status: `ENGINEERING_MEMO / APPLY_TO_RI_AND_NEW_LIVE_CONTOURS`

## Purpose

This memo captures the service-layer lessons learned from the already working
live strategies:

- `session_gap_standalone`
- `hybrid_intraday_runtime`
- `alor_usdrubf_hybrid`
- `ri_author41_42`

The goal is not to retell every incident. The goal is to make the protective
runtime patterns explicit so that RI and future strategies do not repeat bugs
already discovered during the extended micro/live soak work.

## Short Verdict

The stable live systems are not stable only because their signal logic is good.
They are stable because the signal layer is surrounded by service invariants:

- action-scoped order emission
- broker-truth bootstrap and restart reconciliation
- explicit pending/deferred lifecycle
- closed-window pre-emit deferral
- request-id parity or request-id skew detection
- rollback after host-dropped intents
- close-only passthrough when the live guard is blocked
- startup replay guards
- conservative safe-mode/manual-intervention branches
- operator-readable audit events

RI already has several of these protections. The 2026-05-07 service-maturity
patches move RI live entry emission into `live_pending_entry` and promote it to
`live_in_position` only after broker position confirmation. RI exits now move
through `live_pending_exit`, confirm flat only from broker position updates, and
use `live_deferred_exit` for recoverable `trading_window_closed` exit rejects.
The runtime also treats RI `order_symbol` as part of the strategy symbol set, so
routed broker events are not filtered out before strategy callbacks.

## Proven Service Patterns

### 1. Action-scoped live order emission is the baseline

Evidence:

- `docs/live-control-path-baseline-2026-04-17.md`
- `docs/intent-path-unification-fix-plan-2026-04-17.md`
- `alor-gateway/src/action_scope_cws.rs`
- `strategy-runtime/src/strategies/ri_author41_42_live.rs`

Rules:

- Live orderable strategies must not silently fall back to legacy long-lived CWS.
- Config validation should reject legacy execution paths for new live contours.
- Tests should prove all emitted live roles use the action-scoped path.

Existing tests:

- `ri_author41_42_live::legacy_execution_path_is_rejected`
- `ri_author41_42_live::candidate_adapter_keeps_all_roles_action_scoped_only`
- gateway action-scope tests around `control_cws_mode = "action_scoped"`

### 2. Runtime host owns live guard, trading-window blocking, and close-only passthrough

Evidence:

- `strategy-runtime/src/runtime.rs`

Important host behavior:

- `maybe_defer_exit_before_emit` converts known closed-window exits into a
  synthetic rejected ack before broker emission.
- `restore_strategy_state_after_dropped_intents` reverts strategy state when all
  intents are dropped by guard/window before emission.
- `guard_allows_intent_when_blocked` allows only exit/cancel/protective repair
  passthrough, and only when a broker position exists.

Existing tests:

- `runtime::closed_window_exit_is_deferred_before_emit`
- `runtime::guard_close_only_path_allows_exit_cancel_repair_only_with_open_position`
- `runtime::silence_gap_blocks_entry_on_first_gap_bar`
- `runtime::market_intent_classified_as_exit_against_open_position`

Rule for new strategies:

- A blocked/dropped intent must not leave strategy-owned state believing a live
  order or live position exists.

### 3. Strategy-local hidden state must be persisted or cleared on restore

Incident driver:

- RI 2026-05-07 blocked-entry state skew.

Pattern:

- If strategy state is mutated while producing intents, and the host later
  restores the previous `StrategyState`, then every non-persisted operational
  field must be reverted too.

RI mitigation:

- `RiAuthor4142LiveStrategy::set_state` clears `live_mr.position`,
  `live_bo.position`, and `live_bo.pending` whenever restored phase is not
  `live_in_position`.
- RI live entry emits now persist `live_pending_entry` first; broker position
  updates promote that phase to `live_in_position`.
- RI live exit emits now persist `live_pending_exit`; broker-flat position
  updates clear strategy-owned live positions and return the phase to `flat`.
- RI `trading_window_closed` exit rejects with a broker position enter
  `live_deferred_exit` and reissue the exit on the next eligible model bar.
- Runtime strategy event matching includes RI `order_symbol`, preserving
  callbacks for routed full-symbol instruments such as `RTS-6.26`.
- Runtime now calls `on_command_prepared` after constructing the exact
  `OrderCommand`, and RI persists that exact `request_id` into
  `pending_entry_request_id` / `pending_exit_request_id` before state
  persistence.
- RI ack handling only clears pending entry/exit state when the ack
  `request_id` matches the current persisted pending id; mismatches log
  `ri_pending_request_id_skew_detected` and keep the pending state intact.
- Runtime-level guard rollback is covered for RI: if a live entry intent is
  dropped by the host before broker emit, strategy state and hidden live
  positions are restored before the next model bar.
- `ri_model_bar_observed` is a debug-level heartbeat only; operational `INFO`
  remains focused on decisions, candidate intents, command preparation, rejects,
  recovery, and manual-intervention events.

Existing test:

- `ri_author41_42_live::restored_flat_state_clears_unpersisted_live_positions`
- `ri_author41_42_live::micro_live_promotes_pending_entry_to_in_position_on_position_update`
- `ri_author41_42_live::micro_live_trading_window_closed_exit_reject_enters_deferred_exit_and_reissues`
- `ri_author41_42_live::micro_live_entry_ack_with_request_id_skew_does_not_clear_pending_entry`
- `runtime::notify_command_prepared_updates_strategy_state_with_exact_request_id`
- `runtime::ri_guard_dropped_entry_restores_hidden_live_state_before_next_bar`
- `runtime::live_accepts_position_events_for_ri_order_symbol`

Required future checklist item:

- Every strategy must have either no hidden live state, or an explicit
  `set_state` invariant test that proves hidden state cannot survive a host
  rollback.

### 4. Pending request ids must be exact, persisted, and auditable

Evidence:

- `docs/request-id-skew-and-deferred-exit-fix-plan-2026-04-18.md`
- `strategy-runtime/src/strategies/hybrid_intraday_runtime.rs`

Patterns:

- Strategy-owned pending request ids must equal the emitted command request id.
- Hybrid uses `effective_created_ts_utc` to align pending ids with host event
  timestamp semantics.
- Hybrid logs `pending_request_id_skew_detected` if an ack arrives for a related
  path but does not match the current pending id.
- RI receives the final host-built command id through `on_command_prepared`
  instead of deriving it from model/bar timestamps.
- RI exposes these ids through `pending_request_ids()` so runtime restore keeps
  in-flight entry/exit acks attached after restart.

Existing tests:

- `hybrid_intraday_runtime::pending_exit_request_id_uses_effective_created_ts`
- `hybrid_intraday_runtime::ack_reject_clears_only_matching_pending_entry`
- `ri_author41_42_live::micro_live_entry_ack_with_request_id_skew_does_not_clear_pending_entry`
- `runtime::notify_command_prepared_updates_strategy_state_with_exact_request_id`
- runtime tests for `normalize_event_ts_is_monotonic_and_bootstrap_safe`

Rule for new strategies:

- Do not let each strategy guess final emitted request ids from raw bar time if
  the host may normalize timestamps.

### 5. Closed-window exits need two layers: pre-emit defer and reject safety net

Evidence:

- `strategy-runtime/src/runtime.rs`
- `strategy-runtime/src/strategies/session_gap_standalone.rs`
- `strategy-runtime/src/strategies/hybrid_intraday_runtime.rs`

Patterns:

- Preferred normal path: runtime detects closed window before broker emission and
  invokes strategy ack/defer handling with a synthetic `trading_window_closed`.
- Safety-net path: if gateway still rejects with `trading_window_closed`, the
  strategy clears pending live exit and enters deferred exit.

Existing tests:

- `runtime::closed_window_exit_is_deferred_before_emit`
- `session_gap_standalone::trading_window_closed_exit_reject_enters_deferred_phase`
- `session_gap_standalone::deferred_exit_reissues_after_trading_resumes_until_flat`
- `hybrid_intraday_runtime::trading_window_closed_exit_reject_enters_deferred_state`
- `hybrid_intraday_runtime::deferred_exit_reissues_after_live_ready_returns_until_flat`
- `ri_author41_42_live::micro_live_trading_window_closed_exit_reject_enters_deferred_exit_and_reissues`

Rule for new strategies:

- A closed-window exit must never become stale `pending_exit_active` forever.

### 6. Bootstrap and restart must reconcile against broker truth first

Evidence:

- `strategy-runtime/src/runtime.rs`
- `strategy-runtime/src/strategies/session_gap_standalone.rs`
- `strategy-runtime/src/strategies/hybrid_intraday_runtime.rs`
- `strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs`
- `strategy-runtime/src/strategies/ri_author41_42_live.rs`

Patterns:

- Runtime loads order/position/stop-order snapshots before declaring live ready.
- SessionGap reconciles phase from broker snapshot.
- Hybrid can adopt tagged working MR brackets and skip unnecessary repair.
- Hybrid enters safe mode if an open position exists but owner is unknown.
- Alor-USDRUBF adopts non-flat snapshot conservatively and blocks blind entry.
- RI requires manual intervention on non-flat position, working orders, working
  stop orders, restored pending requests, or restored known order ids.

Existing tests:

- `session_gap_standalone` restore/reconcile tests around runtime state and
  bootstrap snapshot
- `hybrid_intraday_runtime::bootstrap_adopts_working_mr_bracket_and_skips_repair`
- `hybrid_intraday_runtime::bootstrap_open_position_without_owner_enters_safe_mode_even_with_cycle`
- `alor_usdrubf_hybrid::bootstrap_adoption_with_non_flat_snapshot_prevents_blind_entry`
- `ri_author41_42_live::bootstrap_non_flat_position_requires_manual_intervention`
- `ri_author41_42_live::bootstrap_working_orders_require_manual_intervention`
- `ri_author41_42_live::bootstrap_working_stop_orders_require_manual_intervention`

Rule for new strategies:

- Startup flat is a broker fact, not a strategy assumption.

### 7. Rejected entry and rejected exit have different risk semantics

Evidence:

- `strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs`
- `strategy-runtime/src/strategies/ri_author41_42_live.rs`

Patterns:

- Rejected entry with broker flat may roll back to flat or defer retry.
- Rejected exit while broker still has a position must preserve open-risk state
  and show operator-visible recovery status.

Existing tests:

- `alor_usdrubf_hybrid::terminal_reject_after_entry_intent_clears_inflight_and_defers_retry`
- `alor_usdrubf_hybrid::terminal_reject_after_exit_intent_preserves_open_risk_state`
- `ri_author41_42_live::micro_live_rejected_entry_rolls_back_to_flat_when_broker_flat`

Rule for new strategies:

- Entry failure can be flat-safe. Exit failure is risk-active until broker truth
  proves flat.

### 8. History and startup replay can warm indicators, but must not emit stale live orders

Evidence:

- `strategy-runtime/src/strategies/session_gap_standalone.rs`
- `strategy-runtime/src/strategies/hybrid_intraday_runtime.rs`
- `strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs`
- `strategy-runtime/src/strategies/ri_author41_42_live.rs`

Patterns:

- `warmup_from_history` updates indicators without live emission.
- Hybrid startup replay guard suppresses stale live bars until the runtime
  reaches the current boundary.
- SessionGap preserves `last_bar_ts` as a strict dedup marker.

Existing tests:

- `hybrid_intraday_runtime::startup_replay_guard_warms_stale_live_bar_without_emitting_intents`
- `session_gap_standalone::restored_last_bar_ts_prevents_reprocessing_last_bar`
- `session_gap_standalone` runtime restore indicator tests

Rule for new strategies:

- From-zero warmup must build model state, not replay broker actions.

### 9. Model feed guards are part of the contract, not cosmetic filters

Evidence:

- `docs/ri-author41-42-live-contract-2026-05-01.md`
- `docs/imoexf-primary-runtime-integration-review-handoff-2026-04-26.md`
- IMOEXF frozen model/risk-gate details are summarized in
  `docs/imoexf-hybrid-mr-bo-handoff-2026-04-26.md`; raw research artifact
  bundles are intentionally omitted from the sanitized corporate handoff
  branch.

Patterns:

- RI and IMOEXF frozen models are 10m contracts.
- Weekend and service/pre-session bars may exist in raw/audit data, but must not
  enter model state.
- BO/MR overlap and no-overnight behavior are live contract constraints, not
  optional monitoring notes.

Rule for new strategies:

- Keep raw/audit feed and model/tradable feed separate.

### 10. Risk-gate historical memory belongs in a ledger, not in strategy snapshot

Evidence:

- `strategy-runtime/src/risk_gate_store.rs`
- `strategy-runtime/src/strategies/hybrid_intraday/risk_gate.rs`
- `docs/imoexf-primary-runtime-integration-review-handoff-2026-04-26.md`

Patterns:

- Seed is one-time bootstrap or controlled rebuild artifact.
- Redis stream ledger is the canonical session history.
- Redis hash/materialized state is a fast cache.
- Strategy snapshot keeps only current operational state.

Existing tests:

- `risk_gate_store::startup_config_from_strategy_builds_identity_and_mode`
- `risk_gate_store::startup_config_from_strategy_requires_seed_for_bootstrap`
- `risk_gate_store::startup_config_from_strategy_validates_ledger_key_identity`
- `hybrid_intraday_runtime::risk_gate_shadow_finalizes_previous_regular_session_on_rollover`
- `hybrid_intraday_runtime::risk_gate_shadow_ack_clears_pending_finalization_and_state`
- `hybrid_intraday_runtime::enforced_risk_gate_blocks_mr_without_state`

Rule for new strategies:

- Long-lived risk memory must survive operational state resets without becoming
  a hidden mutable config file.

## Mandatory Checklist For RI And Future Live Strategies

Before a strategy moves from shadow/dry-run to micro-live, verify:

- Config rejects non-action-scoped live execution.
- Model symbol and broker order symbol are tested separately when they can
  differ, e.g. `RIM6` model vs `RTS-6.26` order routing.
- Warmup/history cannot emit broker commands.
- Live guard or trading-window dropped intents cannot leave hidden strategy-local
  positions, pending flags, or emitted journals that imply broker exposure.
- Every non-persisted live field is either derivable from `StrategyState` or
  explicitly cleared in `set_state`.
- Entry lifecycle does not mark broker exposure as real unless emission/ack/fill
  semantics justify it, or there is a rollback test proving safe recovery.
- Exit lifecycle preserves open-risk state until broker truth confirms flat.
- Closed-window exits defer before emit and still converge on residual gateway
  reject.
- Pending request ids are produced by a single source of truth, preferably the
  runtime's final prepared command hook, or checked against emitted command ids.
- Bootstrap with non-flat broker position, working order, or working stop order
  enters safe/adopt/manual-intervention mode rather than blind entry.
- Runtime restore with pending requests or known order ids is explicitly handled.
- Model feed excludes service/weekend bars from model state.
- No-overnight/gap-flatten behavior is explicitly tested if the model can hold
  across a non-tradable gap.
- Operator logs include enough fields to reconstruct component, role, side,
  request id, order symbol, broker order id, position before/after, and
  suppress/defer/emit decision.

## RI-Specific Follow-Up Recommendation

The immediate RI micro patch line is acceptable for controlled observation after
the 2026-05-07 rollback fix, provided account state remains flat and logs remain
clean.

For promotion beyond the current conservative micro contour, continue the
remaining P1 lifecycle polish slice:

- keep the existing strategy-level `set_state` hidden-state cleanup test as a
  regression backstop;
- keep monitoring live logs for any new high-volume `INFO` event that crowds out
  execution/recovery diagnostics.

This keeps the current micro contour usable while moving RI toward the service
contract already proven by SessionGap, Hybrid IMOEXF, and Alor-USDRUBF.
