# Cleanup Inventory (Step 7)

## Scope and method
- Checked repository hygiene for generated artifacts and system files.
- Reviewed duplicate runbooks and consolidated docs entry points to `/docs`.
- Ran dependency inventory commands:
  - `cargo tree -e normal`
  - `cargo tree -i reqwest`
  - `cargo tree -i tokio`
  - `cargo machete` (installed as `cargo-machete`)
- Reviewed strategy-runtime/alor-gateway binaries and config files for risky deletions.

## Remove now (safe)
- Tracked runtime artifacts removed:
  - `strategy-runtime/trades.csv`
  - `strategy-runtime/trades0.csv`
  - `strategy-runtime/summary.json`
  - `trades.csv`
  - `summary.json`
  - `backtest_trades.log`
- Tracked system files removed:
  - `barter-data/.DS_Store`
  - `barter/examples/.DS_Store`
- Duplicate legacy runbooks removed (single source of truth kept in `/docs`):
  - `strategy-runtime/GATEWAY_RUNBOOK.md`
  - `strategy-runtime/REPLAY_RUNBOOK.md`
- `.gitignore` expanded with recursive ignores for replay/runtime outputs and editor/system metadata.

## Keep (required)
- Current `/docs` runbooks:
  - `docs/strategy-runtime-runbook.md`
  - `docs/alor-gateway-runbook.md`
  - `docs/replay-backtest-guide.md`
  - `docs/state-and-restarts.md`
- Active binaries in runtime/gateway remain unchanged:
  - `strategy_runtime_runner`, `session_gap_replay`
  - `alor_gateway_runner`, `alor_gateway_transport_runner`, `alor_gateway_limit_cancel`
- Existing config templates/profiles retained pending runtime-owner confirmation.

## Needs coordination (risk)
- `cargo machete` flagged potentially-unused dependencies (may include macro/test/feature false positives):
  - `barter-macro`: `proc-macro2`
  - `alor-protocol`: `serde_json`
  - `alor-scalping`: `futures`, `rust_decimal`, `uuid`
  - `alor-gateway`: `barter-data`, `futures`, `tokio-stream`
  - `barter`: `prettytable-rs`
  - `barter-data`: `vecmap-rs`
- Decision: do **not** remove these in this cleanup step without crate-owner confirmation + targeted compile/test per crate.

## Post-cleanup safety checks
- Workspace build/test were executed (see final report).
- Replay and gateway compile sanity commands executed; generated outputs are ignored by git.
