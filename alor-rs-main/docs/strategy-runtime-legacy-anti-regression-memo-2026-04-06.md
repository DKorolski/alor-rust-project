# Strategy Runtime Legacy Anti-Regression Memo (2026-04-06)

## Scope

This memo records the mandatory anti-regression gate for legacy strategies after shared runtime hardening for `AlorUsdrubfHybrid`.

Validated legacy strategies:

- `session_gap_standalone`
- `hybrid_intraday_runtime`

## Why this gate was mandatory

Shared code paths changed in:

- strategy host/lifecycle routing,
- registry + adapters,
- capabilities wiring,
- state restore/bootstrap callbacks.

Because of that, legacy operational semantics needed explicit re-verification before next live soak phase.

## Gate execution

### Unit/per-strategy gate

- `cargo test -p strategy-runtime --lib session_gap_standalone` - PASS
- `cargo test -p strategy-runtime --lib hybrid_intraday_runtime` - PASS

### Integration/e2e gate

- `cargo test -p strategy-runtime --test e2e_session_gap_restart` - PASS
- `cargo test -p strategy-runtime --test e2e_reconnect_blocks` - PASS
- `cargo test -p strategy-runtime --test e2e_hybrid_golden` - PASS
- `cargo test -p strategy-runtime --test e2e_smoke` - PASS
- `cargo test -p strategy-runtime --test live_guard_tests` - PASS
- `cargo test -p strategy-runtime --test config_tests` - PASS
- `cargo test -p strategy-runtime --test ledger_reports` - PASS

### Aggregate gate

- `cargo test -p strategy-runtime` - PASS

## Observed result

No regression was detected in core legacy semantics during this gate:

- restart/recovery scenarios remained green for `session_gap_standalone`,
- startup replay and golden behavior remained green for `hybrid_intraday_runtime`,
- shared lifecycle routing and guard behavior remained consistent with prior expectations.

## Practical verdict

- **Gate status:** PASS
- **Residual risk:** full behavioral equivalence across all possible live broker edge cases is not claimed by this memo.
- **Decision impact:** anti-regression prerequisite for next controlled live-soak stage is satisfied.
