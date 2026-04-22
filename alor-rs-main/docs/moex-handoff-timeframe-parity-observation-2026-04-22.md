# MOEX Handoff Timeframe Parity Observation (2026-04-22)

## Scope

This note records the current parity read for the two MOEX hybrid lines that were compared against their frozen Python handoff packages:

- `IMOEXF hybrid_intraday`
- `USDRUBF alor_usdrubf_hybrid`

The goal of this note is not to re-open discovery, but to state clearly what is already confirmed, what is drifting today, and what the most practical next soak step is.

## Main observation

For both MOEX hybrid lines, the strongest remaining parity gap is currently a timeframe gap:

- frozen handoff / replay baseline is built on `10m`;
- current Rust paper/live runtime feeds are using `1m`.

This means current live/paper behavior should not be interpreted as exact continuation of the frozen handoff model, even when the Rust core logic itself is close to or fully replay-parity-compatible.

## USDRUBF read

### Confirmed

- The frozen Rust replay path for `USDRUBF` is valid on the handoff bundle and passed parity checks on:
  - `golden`
  - `train`
  - `test`
- The earlier VWAP feature drift was real and has now been corrected to the handoff definition.

### Remaining drift

- The frozen handoff package is `10m`.
- The current runtime feed is `1m`.
- Live entry/exit behavior is still not fully identical to the frozen execution contract because the live runtime is broker-event-driven and not a pure replay-style next-bar simulator.

### Practical read

`USDRUBF` replay parity is now in a much better state than live parity. The remaining mismatch looks dominated by runtime/feed semantics rather than by a broken research-core translation.

## IMOEXF read

### Confirmed

- The frozen handoff package is explicitly `10m`.
- The Rust `hybrid_replay` path exists for the frozen `IMOEXF` bundle.
- Replay parity checks on the frozen bundle completed successfully on:
  - `golden`
  - `train`
  - `test`
- The Rust mean-reversion, breakout, and orchestrator logic appear closely aligned with the Python handoff core.

### Remaining drift

- Current Rust `paper` and `live` configs for `IMOEXF hybrid_intraday` are using `1m` bars, not `10m`.
- No analogous `VWAP`-formula feature drift was found here, because this model does not depend on that `USDRUBF`-specific feature layer.
- So the main drift for `IMOEXF` looks like a timeframe/runtime-contour drift, not a signal-formula drift.

## Current engineering conclusion

At this point the cleanest wording is:

- frozen replay parity: confirmed for the frozen handoff bundles;
- live/paper parity: not yet exact, primarily because runtime is consuming `1m` while the frozen handoff model is `10m`.

This is an important distinction:

- the handoff model transfer is not disproven;
- but the currently soaked runtime should not yet be described as exact frozen-model continuation under the original handoff contract.

## Recommended next step

The most practical next step is to run the next MOEX soak with a `10m` bar feed for these hybrid lines.

Why this looks sufficient as the next move:

- it removes the largest known structural drift without reopening model design;
- it aligns runtime cadence with the frozen replay bundle;
- it gives the best chance of bringing live/paper behavior closer to the already-confirmed replay baseline;
- it is a smaller and cleaner intervention than retuning thresholds or changing signal logic.

## Recommended wording for review

The current evidence suggests that it may be sufficient to provide a `10m` feed to the Rust hybrid runtimes for the next soak phase, and that we should expect materially better parity alignment after this change.

This should still be treated as an engineering expectation, not a final proven result, because after the timeframe is aligned we still need to observe:

- whether paper/live trade cadence converges toward replay;
- whether entry/exit timing differences narrow in practice;
- whether any residual execution-contract drift remains visible after the timeframe mismatch is removed.

## Working decision

Recommended working decision:

1. Keep the frozen handoff packages unchanged.
2. Treat the current replay results as the primary parity truth.
3. Move the next controlled soak for MOEX hybrid lines to `10m` feed.
4. Re-evaluate parity only after that soak, instead of judging transfer quality from the current `1m` runtime contour.

## Bottom line

The simplest current read is:

- `replay parity` is in good shape;
- `live/paper parity` is still distorted by `1m` vs `10m`;
- a `10m` feed is likely the most reasonable next step for continued soak and expected parity convergence.
