# Session Gap via alor-gateway runbook

This runbook covers stream-driven runs (no CSV replay) for `session_gap_standalone`.

## 0) Prerequisites
- Redis is running and reachable at `redis://127.0.0.1/`.
- `alor-gateway` is running and publishing to the configured streams.
- Gateway/account identifiers in stream names match your portfolio.

## 1) Paper mode (stream feed, no live orders)

```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_session_gap_paper_gateway.toml
```

Behavior:
- Reads bars/orders/trades/positions from gateway streams.
- Simulates order execution in runtime (`trade_mode = paper`).
- Writes paper outputs to:
  - `strategy-runtime/trades.csv`
  - `strategy-runtime/summary.json`

## 2) Live mode (stream feed + real commands)

```bash
cargo run -p strategy-runtime --bin strategy_runtime_runner -- --config strategy-runtime/rt_session_gap_live_gateway.toml
```

Behavior:
- Reads stream feed from gateway.
- Emits real order commands (`trade_mode = live`, `allow_live_orders = true`).
- Uses live guard and ack/order/trade feedback loop.

## Notes
- Both configs intentionally keep some schema-required fields that `session_gap_standalone` does not consume directly.
- `replay.enabled = false` in these profiles to force stream mode instead of CSV replay mode.
