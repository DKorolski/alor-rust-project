# Integration Report: `main` + `dev`

Date: 2026-03-11  
Branch: `integration/unify-main-dev`

## 1. Branching workflow executed

1. Branch point detected:
- `merge-base(main, dev) = c07b738`

2. Integration branch created from branch point:
- `integration/unify-main-dev` from `c07b738`

3. `dev` merged into integration base:
- merge commit: `58490ea` (`merge dev into integration base`)

4. Production fixes from `main` applied on top:
- `0444494` (partially, conflict resolved intentionally)
- `65df9fc`
- `b661310`
- `18e9121`

## 2. Main fixes transfer status

| Main commit | Topic | Status | Notes |
|---|---|---|---|
| `18e9121` | session_gap warmup indicators + false-ready guard | Transferred | cherry-pick successful |
| `65df9fc` | session_gap market -> marketable execution | Transferred | cherry-pick with manual conflict resolution in `session_gap_standalone.rs` |
| `0444494` | MIW live rollout + OneDay TIF | Transferred (partial) | logic transferred; profile split clarified via explicit strategy-specific config names |
| `b661310` | ws reconnect test stabilization | Transferred | cherry-pick successful |
| `6da7747` | merge wrapper commit | Not cherry-picked directly | merge commit wrapper, relevant functional change already present via `18e9121` |

## 3. Conflict zones and decisions

### `configs/gateway.*.7502*.toml` naming split (former add/add conflict zone)
- Conflict source: both branches had different intent for similarly named live configs.
- Decision: adopt explicit strategy-specific names:
  - `configs/gateway.sessiongap.live.7502MIW.toml`
  - `configs/runtime.sessiongap.live.7502MIW.toml`
  - `configs/gateway.hybrid.live.7502SN6.toml`
  - `configs/runtime.hybrid.live.7502SN6.toml`
- Rationale: remove ambiguity and prevent accidental profile reuse across strategies.

### `strategy-runtime/src/strategies/session_gap_standalone.rs`
- Conflict source: marketable `Place` migration vs newer `Intent::Place` shape in dev line.
- Decision: preserve marketable execution path (`Intent::Place`) and keep `comment` field; remove stale `fill_price` assignment in `Place`.
- Rationale: keep main production behavior while remaining compatible with current runtime intent schema.

## 4. Shared-core areas reviewed during integration

- `strategy-runtime/src/runtime.rs`
- `strategy-runtime/src/config.rs`
- `strategy-runtime/src/lib.rs`
- `strategy-runtime/src/strategies/session_gap_standalone.rs`
- `strategy-runtime/src/strategies/hybrid_intraday_runtime.rs`
- `alor-gateway/src/cws_client.rs`
- `alor-gateway/src/router.rs`
- `alor-gateway/src/services/command_consumer.rs`
- `alor-gateway/src/ws_hub.rs`
- `alor-gateway/src/supervisor.rs`

## 5. Verification results

Executed on integration branch:

1. `cargo test -p strategy-runtime --quiet`
- Result: PASS (all tests green)

2. `cargo test -p alor-gateway --quiet`
- Result: PASS (all tests green, one ignored in suite as expected)

## 6. Session-gap specific acceptance notes

- Warmup/false-ready logic from `main` is present (`18e9121`).
- Marketable execution path for live session_gap preserved (`65df9fc`).
- Reconnect test stabilization present (`b661310`).

## 7. Hybrid specific acceptance notes

- Full dev hybrid line retained (strategy + runtime/gateway support + replay/tests/configs).
- Recent paper lifecycle safety fixes retained:
  - duplicate exit suppression,
  - close-only exit semantics in paper simulator,
  - fill-time recalc guard,
  - dropped queued exit terminal ledger status.

## 8. Remaining follow-up before merge to `main`

1. Final config policy decision for live profile naming:
- strategy-specific names are now used for session_gap/hybrid live configs.
- if additional portfolios/instruments are added, keep the same naming scheme `<gateway|runtime>.<strategy>.live.<portfolio>.toml`.

2. Run environment smoke checks on target deployment profile(s):
- session_gap live smoke,
- hybrid paper/live-only soak smoke.

## 9. Conclusion

Integration branch now contains both strategy lines on shared core:
- `session_gap` production fixes are preserved,
- `hybrid` functionality is preserved,
- runtime/gateway tests are green.

Branch is suitable as merge candidate to `main` after final config naming decision and deployment smoke confirmation.
