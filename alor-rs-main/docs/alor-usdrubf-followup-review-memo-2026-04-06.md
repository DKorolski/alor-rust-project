# Alor USDRUBF Follow-up Review Memo (2026-04-06)

## Scope of this memo

This memo closes the follow-up hardening slice after the initial live-prep review and documents what is now:

- implemented and verified,
- conditionally allowed,
- not yet equivalent to mature strategies.

## Delivered in follow-up slice

- strict startup guard semantics:
  - `live_ready` is cleared only by fresh `DataOrigin::Live` bar,
  - fresh `history/history_gap/replay` bars no longer clear startup guard.
- non-flat bootstrap adoption:
  - non-flat symbol position from bootstrap snapshot is adopted into strategy open state,
  - blind duplicate entry is prevented immediately after bootstrap adoption.
  - when owner cannot be inferred with confidence at bootstrap, strategy enters conservative owner mode until first live position confirmation.
- terminal reject policy is now explicit:
  - entry reject: clears inflight and defers reissue to next bar (`entry_reject_deferred_retry`),
  - exit reject: preserves open position risk state, clears inflight, and defers retry (`exit_reject_deferred_retry`).
- capability descriptor review:
  - `uses_stop_orders` is now `false` for `AlorUsdrubfHybrid`,
  - descriptor reflects current maturity instead of target maturity.
- docs/runbook synchronization:
  - supported startup profile and unsupported scenarios are explicit,
  - next-run protocol is fixed for isolated controlled execution.

## Test evidence

- `T10`: `bootstrap_adoption_with_non_flat_snapshot_prevents_blind_entry` - PASS.
- `T11`: `fresh_recovered_origin_bar_does_not_clear_live_ready` - PASS.
- `T12`: `restart_with_non_flat_snapshot_keeps_owner_conservative_until_live_confirmation` - PASS.
- `T13`: `terminal_reject_after_entry_intent_clears_inflight_and_defers_retry` - PASS.
- `T14`: `terminal_reject_after_exit_intent_preserves_open_risk_state` - PASS.
- capability consistency check (`registry`): `alor_usdrubf_capabilities_match_followup_hardening_profile` - PASS.

## Anti-regression gate for legacy strategies

The mandatory anti-regression matrix for old strategies is green:

- unit/session gap subset: PASS,
- unit/hybrid intraday runtime subset: PASS,
- integration tests:
  - `e2e_session_gap_restart`: PASS,
  - `e2e_reconnect_blocks`: PASS,
  - `e2e_hybrid_golden`: PASS,
  - `e2e_smoke`: PASS,
  - `live_guard_tests`: PASS,
  - `config_tests`: PASS,
  - `ledger_reports`: PASS.

Full aggregate run:

- `cargo test -p strategy-runtime` - PASS.

## Locked startup profile status

Currently supported:

- clean-start profile only,
- isolated namespace,
- fresh consumer group and runtime-state stream,
- flat startup account.

Not yet proven as fully equivalent:

- restart with open position + complex ownership reconstruction,
- restart with working orders/stop orders full maturity,
- full parity with `session_gap_standalone` / `hybrid_intraday_runtime`.

## Reconcile precedence policy

`live broker events > bootstrap snapshot > restored runtime state`.

This policy is reflected both in runtime behavior and hardening docs.

## Next-run protocol (operational)

Before next micro-run:

1. fresh consumer group,
2. fresh runtime-state stream key,
3. isolated namespace for all relevant streams,
4. first pass in paper or with live orders disabled until target startup scenario is proven.

Mandatory evidence points:

- bootstrap summary,
- replay guard armed/cleared,
- first fresh live-origin bar,
- first `live_guard=ALLOWED`,
- first allowed entry,
- first broker-truth position transition.

## Final verdict for this follow-up

- **Go:** controlled diagnostic bring-up, paper, clean-start live-preflight, clean-start micro-soak under isolation protocol.
- **Conditional-Go:** any restart-focused micro-run only under strict protocol and explicit scenario scope; owner reconstruction remains conservative by design.
- **Not yet equivalent:** mature non-flat restart ownership and stop-order semantics remain a follow-up domain.
