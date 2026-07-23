# MR Bracket Partial-Entry Hardening, 2026-06-22

## Historical Scope

This document records the MR-bracket-only hardening released on 2026-06-22.
It has been extended by the generic trade-ordering and multi-lot BO entry
contract in [Partial Entry Reconciliation Hardening, 2026-07-23](partial-entry-reconciliation-hardening-2026-07-23.md).

## Original Scope

The patch applies to MR entries that create broker TP/SL protection after the
entry. BO and ordinary market entry/exit behavior are unchanged.

## P0 Contract

- A position transition toward the configured MR target is treated as entry
  accumulation, for example `0 -> -1 -> -3`.
- No TP or SL is created for an intermediate partial position.
- Exactly one bracket generation is created after broker position reaches the
  full target quantity.
- A sign mismatch, overfill, or position reduction during accumulation enters
  close-only recovery and flattens broker truth.
- A partial entry that does not reach target within `3000 ms` cancels known
  working entry orders and flattens the partial broker position.
- The timeout is driven by the runtime poll timer and is independent of the
  model bar interval.

## Rollout

- Keep both IMOEXF live configurations at quantity `3`.
- Deploy only in a confirmed flat window with no working TP/SL or entry orders.
- Restart affected runtimes from zero.
- Confirm the first MR lifecycle has either a direct full fill or logged
  `partial_entry_progress` followed by one full-size TP and SL generation.
- Treat `partial_entry_timeout_emergency_exit`, sign mismatch, or overfill as a
  rollout stop condition.
