# IMOEXF Shadow Paper-State Isolation - 2026-07-25

## Problem

The `canonical07` and `legacy09` IMOEXF shadow runtimes shared the live
portfolio snapshot stream for `7502MIW`. Although paper mode already ignored
external position *events*, bootstrap imported the broker snapshot into
runtime state. A subsequent synthetic shadow fill was therefore added to the
real broker position, producing quantities such as `12` instead of the
configured shadow quantity `6` and a false `broker_residual_emergency_exit`.

This invalidates paper PnL and makes the 07:00 versus 09:00 comparison
non-diagnostic.

## Fix

- Non-live runtime bootstrap acknowledges snapshot availability for readiness,
  but clears external orders, stops and positions before they enter runtime
  state.
- `HybridIntraday` clears only broker lifecycle state on a paper/backtest
  bootstrap. Model features and the High180 risk-gate state remain intact.
- A regression test covers an external broker snapshot plus stale lifecycle
  fields and requires a flat paper state after bootstrap.

## VPS Rollout

- Image: `manual-20260725-paper-shadow-isolation`.
- Scope: only `runtime-shadow07` and `runtime-shadow09` in
  `/opt/trading-moex-early-shadow-imoexf`.
- The two `runtime.state.hybrid_intraday.shadow*.imoexf.7502MIW` streams were
  reset for a clean paper start. The risk-gate session ledgers and materialized
  state were retained.
- Pre-fix reports were archived under
  `volumes/reports/archive-20260725-paper-shadow-isolation/`.

## Acceptance

- Both runtimes log `bootstrap: ignored external broker state for non-live
  runtime` and `paper_bootstrap_broker_state_ignored`.
- Both remain `trade_mode=Paper` with `allow_live_orders=false`.
- No records may appear in either shadow command stream.
- New reports must show the configured shadow quantity, not a quantity summed
  with the live broker position.
