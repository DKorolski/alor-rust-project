# Session Gap replay runbook

## 1) Runtime replay (`strategy_runtime_runner`)
Uses runtime config + `strategy_kind = "session_gap_standalone"`.

```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_session_gap_runtime.toml
```

Outputs:
- `strategy-runtime/trades.csv`
- `strategy-runtime/summary.json`
- `strategy-runtime/replay_out/parity_report.json`

## 2) Standalone replay (`session_gap_replay`)
Uses environment variables (not runtime TOML):

```bash
set -a; source strategy-runtime/rt_session_gap_standalone.env; set +a
cargo run -p strategy-runtime --bin session_gap_replay
```

Outputs:
- `strategy-runtime/replay_out/trades_runtime.csv`
- `strategy-runtime/replay_out/parity_report.json`

## Notes
- `session_gap_replay` is a separate binary and is **not** a `strategy_kind` value.
- Runtime replay and standalone replay are close but not identical simulators; compare via parity report and trade deltas.
