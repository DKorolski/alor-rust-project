# Partial Entry Reconciliation Hardening, 2026-07-23

## Purpose

Prevent a valid broker partial fill from being treated as an orphan trade or a
broker-residual failure while the gateway ACK and order metadata are still in
flight. The incident motivating this patch was a multi-lot IMOEXF BO market
entry whose broker fills arrived as `1 + 5`; the old runtime emitted an
immediate residual close after the first fill.

This is an execution-lifecycle fix. It does not change model signals, entry
levels, risk-gate decisions, or position sizing.

## Runtime Contract

### Generic trade ordering

- A broker trade received while an owned command is awaiting its ACK is logged
  as `trade_buffered_pending_ack`, not as an orphan.
- A trade received after ACK but before order metadata is logged as
  `trade_buffered_pending_order`.
- Buffered trades are replayed only after the runtime has both command
  ownership and order metadata.
- A still-unmatched buffered trade becomes
  `orphan_trade_after_ack_settled` only after all owned requests have settled.

This generic layer applies to every runtime command path. It removes an ACK /
trade ordering race without weakening unmatched-trade detection.

### Hybrid multi-lot entries

- Any Hybrid entry with target quantity greater than one, including BO market
  entries and MR bracket entries, remains in entry accumulation until broker
  position reaches the full target.
- MR TP/SL brackets are created only once, for the complete target quantity.
- A BO entry is not activated from an intermediate partial position.
- Sign mismatch, overfill, or position reduction while accumulating enters
  close-only recovery and flattens broker truth.
- If a complete target is not reached within
  `partial_entry_fill_timeout_ms` (currently 3000 ms), the runtime cancels
  known working entry orders, market-flattens the actual partial position, and
  enters safe close-only mode.

## Rollout Record

The release was built from commit `369e007` as image:

```
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260723-partial-entry-369e007
```

It was deployed in a confirmed broker-flat window to Hybrid IMOEXF and
Alor-USDRUBF live micro contours. Only the affected `strategy-runtime`
services were recreated; their gateway and Redis services were not restarted.
The prior image tag is retained in a timestamped `.env` backup for rollback.

After a controlled restart the expected initial state is
`waiting_for_next_bar_after_restart`: no command may be emitted until the
first newly received 10-minute bar. The service must be `healthy` before the
next tradeable interval.

## Validation

The release passed:

- `cargo fmt --all --check`;
- `cargo test -p strategy-runtime --lib` (336 tests);
- full `cargo test -p strategy-runtime` (336 library, 30 configuration, and
  23 integration tests).

`cargo clippy -p strategy-runtime --all-targets -- -D warnings` still reports
pre-existing lints outside this patch; no new clippy diagnostic was introduced
by the changed paths.

The targeted regression coverage includes:

- a partial trade before ACK, replayed after ACK and order metadata;
- an unmatched pre-ACK trade that is eventually recorded as orphan only after
  request settlement;
- a `1 + remainder` multi-lot BO entry that activates only at the full target;
- a genuine incomplete multi-lot BO entry that cancels the remainder and
  flattens the partial broker position.

## Soak Watch Items

Expected diagnostic events during a broker ordering race are
`trade_buffered_pending_ack` and `trade_buffered_pending_order`; they are not
incidents by themselves.

Treat any of the following on a new entry as a rollout stop condition pending
broker and runtime reconciliation:

- `partial_entry_timeout_emergency_exit`;
- `partial_entry_sign_mismatch`;
- `partial_entry_overfill`;
- `broker_residual_emergency_exit`;
- `orphan_trade_after_ack_settled` for an owned entry.

Rollback is permitted only in a confirmed broker-flat window with no working
entry, TP, or SL orders. Restore the preceding `RUNTIME_IMAGE_TAG` from the
timestamped `.env` backup and recreate only the affected runtime service.
