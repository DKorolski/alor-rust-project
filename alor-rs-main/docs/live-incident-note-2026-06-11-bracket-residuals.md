# Live Incident: Bracket Residuals And Duplicate TP, 2026-06-11

## Summary

Two independent MR bracket cycles left non-flat broker residuals:

- `7502MIW / USDRUBF`: duplicate TP fills over-closed the original short and
  created an unexpected long `+1`.
- `7502T0U / IMOEXF`: a TP sized for only part of the broker position filled,
  the paired SL was canceled, and a short `-1` residual remained.

Both affected strategy runtimes were stopped before manual intervention. The
operator manually flattened both positions. Gateway snapshots then confirmed
broker-flat state on both portfolios.

## Root Cause Class

The shared root cause is order-event-first bracket cleanup without sufficient
broker-position reconciliation:

- a terminal TP order event was treated as proof that the whole strategy
  position had exited;
- the paired SL could be canceled before broker position reached zero;
- an unknown TP create outcome could be retried on a later bar;
- an unexpected broker residual could be adopted as a new open position rather
  than flattened.

The `USDRUBF` stop-limit repair path also produced broker rejects for a price
reported as not matching the instrument minimum price increment.

## Implemented Safety Patch

- Unknown TP/SL create outcomes remain pending until an order event resolves
  broker truth; they are not retried merely because the next model bar arrived.
- TP fill no longer cancels the paired SL immediately. Cleanup waits for
  broker-position flat.
- Any non-zero broker position size change or sign flip while a bracket-owned
  position is active enters close-only safety mode, cancels known protection,
  and emits an action-scoped emergency exit for the exact broker residual.
- A fresh unexpected live residual without a pending entry is flattened rather
  than adopted as a normal strategy position.
- Gateway stop-limit price normalization now stabilizes fractional tick values
  before serialization.

This is intentionally conservative. The first patch flattens residuals instead
of attempting an in-place residual bracket rebuild beside potentially stale
broker orders.

## Validation

Added regression coverage for:

- unknown TP outcome not being retried on the next bar;
- TP fill waiting for broker-flat before paired-stop cleanup;
- unexpected live residual triggering an emergency Market exit;
- partial protective fill triggering cancel plus emergency flatten;
- fractional `0.01` stop-limit price normalization.

Targeted tests pass. Full `alor-gateway` and `strategy-runtime` tests pass. The
stale runtime capability expectation was aligned with the already active
`Alor-USDRUBF` stop-order callback contract.

## Rollout Gate

- Keep `trading-alor-usdrubf-strategy-runtime-1` stopped.
- Keep `trading-hybrid-author41-7502t0u-strategy-runtime-1` stopped.
- Build and deploy the patched runtime/gateway only in a controlled flat window.
- Restart affected validation contours `from zero`.
- Validate IMOEXF partial-fill handling at `qty=2`; keep RI and Alor-USDRUBF
  at `qty=1`.
- Return to quantity `1` for patched validation.
- Require at least `3-5` clean sessions before restoring previous quantities.
