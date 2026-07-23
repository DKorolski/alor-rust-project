# Alor-USDRUBF BO Partial-Fill Hardening - 2026-07-23

## Status

`IMPLEMENTED_LOCAL / PENDING_SAFE_LIVE_ROLLOUT`

## Incident Class

On 2026-07-23 a valid Alor-USDRUBF BO short entry for quantity two arrived as
two broker fills of one contract. The first position callback was treated as a
complete BO entry because the BO pending state carried `target_qty = 1`, while
the emitted market command carried quantity two. The second fill was then seen
as broker residual drift, so the runtime issued an emergency market flatten.

The signal and action-scoped CWS send path were valid. The defect was strictly
in the strategy-owned partial-entry lifecycle.

## Patch Contract

1. The quantity in the emitted entry command becomes the authoritative pending
   target quantity for both MR and BO.
2. A partial entry of either owner remains pending until broker position truth
   reaches that target quantity.
3. While pending, the runtime creates neither an open strategy position nor MR
   protection and does not emit a residual emergency exit.
4. If the remaining entry quantity does not settle within three seconds, the
   runtime cancels tracked working entry orders and market-flattens only the
   known partial position.
5. Existing MR TP/SL behavior, BO signal rules, fixed quantity configuration,
   model time windows, and action-scoped CWS routing remain unchanged.

## Tests

`strategy-runtime/src/strategies/alor_usdrubf_hybrid.rs` covers:

- emitted BO quantity overwriting a legacy/stale pending target;
- partial BO fill waiting for the full target before activation;
- partial BO fill timeout cancelling the remainder and flattening the residual;
- existing MR partial-entry and protective-bracket reconciliation paths.

## Rollout

Deploy only at a confirmed account-flat boundary with no working orders or stop
orders. Restart the Alor-USDRUBF runtime from clean operational state, retain
the current action-scoped gateway configuration, and observe the next multi-lot
BO entry through broker-flat reconciliation.
