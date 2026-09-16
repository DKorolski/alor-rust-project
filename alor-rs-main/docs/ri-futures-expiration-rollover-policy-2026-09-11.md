# RI Futures Expiration Rollover Policy - 2026-09-11

## Decision

RI Author41/42 live micro rollover is a manual operational action. The current
policy is to roll in the final flat maintenance window immediately before the
current contract's expiry / last circulation date, not 7 calendar days before
expiry.

For the active RIU6 contour:

```text
current model symbol: RIU6
current order symbol: RTS-9.26
current expiry / last circulation date: 2026-09-17
final intended RIU6 live session: 2026-09-16
next roll timing: 2026-09-16 evening flat maintenance window
```

Do not switch to the next RI contract early by default. An earlier roll is
allowed only as an explicit operator exception with a recorded reason, for
example exchange-calendar changes, broker availability, liquidity collapse in
the expiring contract, or abnormal spread/market-data quality.

For RIU6 specifically, the planned operational sequence is: let the contour
finish the `2026-09-16` session, confirm evening broker-flat / no working orders,
then switch to the next contract from zero before the `2026-09-17` session. The
expiring RIU6 contour should not be restarted for normal live trading on
`2026-09-17` unless an explicit exception is recorded.

## Why Final-Flat-Window Roll

The live RI contract is no-overnight by design. We therefore do not need a
multi-day cross-contract transition while carrying positions. Rolling too early
can create unnecessary model/feed drift because the recent live soak evidence,
shadow comparisons, and operator monitoring are tied to the current contract's
actual traded path.

The safest default is:

- keep trading the active contract through its intended observation window;
- avoid cross-contract overlap;
- roll only after broker-flat confirmation;
- restart from zero on the next contract.

## Required Roll Conditions

Before switching:

- broker position in current RI contract is flat;
- broker position in next RI contract is flat;
- no working regular orders exist for either contract;
- no working stop orders exist for either contract;
- runtime has no pending entry, pending exit, pending BO, or unresolved
  lifecycle state;
- action-scoped gateway path is healthy;
- current decision journal and logs are archived or at least retained for audit.

If any condition fails, the roll is a NO-GO until the state is reconciled.

## Roll Procedure

Perform the roll in a flat maintenance window, preferably after the final
intended live session of the expiring contract. For the current RIU6 cycle this
window is the evening of `2026-09-16`. If that window is missed, perform the
switch before the `2026-09-17` session starts and do not start the runtime until
the checklist is clean.

Steps:

1. Stop only the RI runtime and RI gateway for the target portfolio.
2. Archive the active runtime TOML, gateway TOML, compose file, and decision
   journal path.
3. Update runtime and gateway configs to the next RI contract.
4. Update both model symbol and broker order symbol.
5. Update the bar stream from the old contract to the new contract.
6. Clear only RI operational runtime state, command stream, ack stream, and
   runtime consumer group.
7. Keep old contract bar streams and reports for audit.
8. Start the gateway first and confirm new-contract bars/history.
9. Start the runtime from zero.
10. Confirm warmup, broker-flat reconciliation, and `LiveReady / ALLOWED`.

## From-Zero Requirement

Every RI contract roll must be a from-zero runtime restart:

- do not restore old-contract runtime snapshot;
- do not reuse old-contract pending request ids;
- do not transfer old-contract indicators directly into the new contract;
- load warmup/history from the new contract's own `10m` stream;
- let the first new-contract session establish its own model state.

This avoids hidden drift from carrying old-contract state into a different
futures series.

## Config Field Note

Current RI configs contain:

```text
actual_expiry_date = "2026-09-17"
roll_target_sessions_before = 1
roll_fallback_sessions_before = 2
```

These fields are currently validated and logged as policy metadata. They do not
perform an automatic instrument switch. They must not be interpreted as approval
to roll one or two sessions before expiry. The operator decision above is the
source of truth: final-flat-window manual roll unless an explicit exception is
recorded.

If future code turns these fields into active scheduling logic, the semantics
must be revisited before deployment so that "final-flat-window roll" remains the
default.

## Next Expected Roll

For RIU6 -> next RI contract, use the `2026-09-16` evening flat window as the
operational rollover checkpoint ahead of the `2026-09-17` expiry / last
circulation date. Before that window, prepare candidate configs and verify the
broker symbol for the next series, but keep live trading on RIU6 unless a
documented exception is approved.
