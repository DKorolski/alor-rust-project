# VPS Live Observations - 2026-06-04

## Pre-Open Runtime Patch Rollout

Context:

- VPS: `155.212.170.21`, host `nektodk.ispvds.com`.
- Target portfolio: `7502MIW`.
- Rollout window: pre-open, before regular trading session.
- Broker/runtime state before rollout: flat for `USDRUBF`, `IMOEXF`, and `RTS-6.26`.

Patch scope:

- `Alor-USDRUBF`: MR exit contract changed from closed-bar marketable exit on `mr_take` / `mr_stop` to protective bracket after MR entry confirmation.
- `Alor-USDRUBF`: MR TP is emitted as protective limit; MR SL is emitted as protective stop-limit.
- `Alor-USDRUBF`: MR time-cutoff / forced flatten and BO exits remain marketable exit paths.
- `IMOEXF hybrid`: added pre-entry guard for near-zero MR bracket churn after tick rounding.
- `IMOEXF hybrid`: BO behavior and riskgate ledger contract unchanged.

Deployment:

- Built runtime image on VPS:
  - `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260604-mrbracket-guard`.
- Restarted only:
  - `trading-alor-usdrubf-strategy-runtime-1`.
  - `trading-hybrid-strategy-runtime-1`.
- Gateway and Redis containers were not restarted.
- Runtime-only from-zero was applied:
  - deleted `runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502MIW`.
  - deleted `runtime.state.hybrid_intraday.live.riskgate_shadow.imoexf.7502MIW`.
- Preserved riskgate ledger:
  - `runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120`.

Post-rollout checks:

- Both restarted runtime containers became `healthy`.
- `Alor-USDRUBF` bootstrap showed:
  - strategy position open count `0`.
  - strategy working orders `0`.
  - strategy working stop orders `0`.
- `IMOEXF hybrid` bootstrap showed:
  - strategy position open count `0`.
  - strategy working orders `0`.
  - strategy working stop orders `0`.
- `IMOEXF hybrid` riskgate store used existing ledger:
  - decision `UseExistingLedger`.
  - `ledger_rows_count = 204`.
  - `rolling_sum_lb120 = 165.40000000000018`.
  - `mr_enabled_current_session = true`.
  - `mr_enabled_next_session = true`.
- Both runtimes entered expected `waiting_for_next_bar_after_restart` guard after startup.
- At `08:11 MSK`, before regular market open, both runtimes were still waiting for the next new `10m` live bar.
- The repeated warning `bars stream has data but runtime reads none` was observed during this pre-open wait. Current interpretation: expected after runtime-only restart before new market bars arrive; re-check after first regular session bars.

Resource check after rollout:

- Redis memory remained within current operating envelope:
  - `trading-alor-usdrubf-redis-1`: about `283MiB / 1GiB`.
  - `trading-hybrid-redis-1`: about `351MiB / 1GiB`.
  - `trading-ri-author41-42-7502miw-redis-1`: about `222MiB / 768MiB`.
  - `trading-hybrid-author41-7502t0u-redis-1`: about `204MiB / 512MiB`.
- Runtime memory after restart was low and normal.

Validation focus for the next sessions:

- `Alor-USDRUBF`: first MR trade should install TP/SL via action-scoped protective path after entry fill.
- `Alor-USDRUBF`: `mr_take` / `mr_stop` should no longer emit direct marketable exits while protective bracket is active.
- `Alor-USDRUBF`: TP fill should delete paired SL; SL trigger/fill should cancel paired TP.
- `IMOEXF hybrid`: watch for `mr_entry_suppressed reason=take_too_close_after_rounding`.
- `IMOEXF hybrid`: confirm no repeat near-zero bracket churn after the guard.
- All systems: confirm no stale working orders or stop orders after flat.
