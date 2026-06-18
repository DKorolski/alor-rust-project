# VPS Live Observations - 2026-06-18

Observation window: pre-open review at `07:47-08:00 MSK`.

## Pre-Open State

All active runtime, gateway, and Redis containers were healthy.

Broker snapshots:

- `7502MIW`: `USDRUBF=0`, `IMOEXF=0`, `RTS-9.26=0`.
- `7502T0U`: `IMOEXF=0`, `RTS-9.26=0`.
- All stop orders present in the latest snapshots had terminal
  `status=canceled`.
- No uncontrolled working TP/SL tail was present before rollout work.

Resources:

- RAM: about `6.5 GiB` available from `7.7 GiB`.
- Swap: about `125 MiB` used from `3.9 GiB`.
- Root disk: `20 GiB / 79 GiB`, `27%`.
- Active Redis instances: about `17-31 MiB`.

The overnight `SyncingGap` / `SyncingHistory` transitions occurred outside the
regular session and were not accompanied by position risk.

## 2026-06-17 Session Read

### Primary IMOEXF Hybrid, 7502MIW

Three observed MR bracket cycles completed broker-flat:

1. Long `2` around `10:00 MSK`, TP filled around `10:03 MSK`.
2. Short `2` around `10:40 MSK`, TP filled around `11:39 MSK`.
3. Long `2` around `11:40 MSK`, TP filled around `13:38 MSK`.

The first cycle exposed a bracket sibling-cleanup incident:

- TP filled and broker position became flat.
- Runtime emitted `delete_stop_limit` for SL `121741481`.
- Action-scoped authorization failed because OAuth refresh returned HTTP `502`.
- Runtime logged `cleanup_ack_error_with_active_stop_while_flat`.
- The operator manually canceled the stale SL.
- Broker event later confirmed `status=canceled`.

The later checked IMOEXF cycles completed flat with their cleanup path.

### IMOEXF Author41-Short, 7502T0U

- MR short `2` entered around `10:50 MSK`.
- TP filled around `11:39 MSK`.
- Broker snapshot was flat before the 2026-06-18 session.

### Alor-USDRUBF, 7502MIW

Two observed cycles completed broker-flat:

- MR short entered near `72.89` and exited by the `11:50` model cutoff near
  `72.98`.
- BO long entered near `73.15`; the first stop-style exit fell inside a closed
  trading window and was rejected as `trading_window_closed`; a later exit
  reissue filled near `73.09`.

One fill-before-ack `orphan_trade` warning was observed on the MR entry. Broker
truth and runtime state subsequently converged.

### RI Author41/42

Both MIW and T0U contours produced Author41 MR decisions and finished
broker-flat. The pre-open broker snapshot showed no `RTS-9.26` position.

## Patch Work

The local patch line now includes:

- Alor-USDRUBF: terminal TP/SL fill blocks protective repair until broker-flat
  reconciliation.
- HybridIntraday: failed sibling stop cleanup while broker-flat emits a bounded
  retry, with a maximum of three `delete_stop_limit` retries.
- HybridIntraday operator events:
  - `sibling_cleanup_retry`;
  - `sibling_cleanup_retry_exhausted`;
  - `sibling_cleanup_confirmed`.

Regression test:

```text
failed_flat_stop_cleanup_retries_are_bounded_and_reset_after_cancel
```

Verification:

- `cargo fmt`: passed.
- `cargo test -p strategy-runtime`: passed.
- `306` library tests passed, plus all strategy-runtime integration and e2e
  tests.

## Rollout Gate

Rollout is allowed only while both portfolios remain broker-flat and have no
working strategy-owned TP/SL orders.

After rollout:

- keep Alor-USDRUBF at qty `1`;
- keep current IMOEXF validation quantities;
- monitor the first MR bracket terminal fill for cleanup retry/confirmation;
- continue the daily broker-round vs runtime-intent vs model-replay audit.

## Rollout Result

Rollout completed at `08:34:55 MSK`, before the regular session.

Images:

- runtime:
  `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260618-lifecycle-68d1cd1`;
- gateway:
  `ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-20260618-oauth-68d1cd1`.

Updated contours:

- `trading-hybrid`;
- `trading-hybrid-author41-7502t0u`;
- `trading-alor-usdrubf`;
- `trading-ri-author41-42-7502miw`;
- `trading-ri-author41-42-7502t0u`.

The rollout was performed sequentially with per-contour `.env` backups and
health checks. Redis containers and persisted runtime state were not restarted
or cleared.

Post-rollout checks:

- all updated gateway and runtime containers were healthy;
- all gateways completed CWS authorization successfully;
- runtime bootstrap found zero open positions, orders, and stop orders;
- no new `WARN` or `ERROR` event was observed during startup;
- RAM available: about `6.6 GiB`;
- root disk usage remained `27%`.

The primary IMOEXF risk-gate resumed from the existing Redis ledger:

- startup decision: `UseExistingLedger`;
- loaded rows: `212`;
- last finalized session: `2026-06-16`;
- `rolling_sum_lb120=131.9`;
- current and next-session MR gate: enabled.

Before the first regular live bar, runtimes remained conservatively blocked by
the expected bootstrap reasons (`missing_live_bar`, `SyncingHistory`). The next
validation point is the transition to `LiveReady` after the session feed starts.
