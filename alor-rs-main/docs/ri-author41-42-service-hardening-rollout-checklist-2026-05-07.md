# RI Author41/42 Service Hardening Rollout Checklist

Date: `2026-05-07`

Status: `SERVICE_HARDENING_PATCH_READY / REBUILD_REQUIRED / FROM_ZERO_VALIDATION`

## Purpose

This checklist captures the post-shadow service-hardening line for the isolated
RI Author41/42 micro contour on portfolio `7502MIW`.

It is not a new model decision. The frozen model contract remains:

```text
profile_id = ri_author41_42_primary_combo_cost2
symbol = RIM6
order_symbol = RTS-6.26
timeframe = 10m
qty = 1
execution_path = action_scoped_only
```

## Patch Line Included

The following commits form the RI live service-maturity line:

```text
9df9842 Harden RI live entry lifecycle
7214a6c Harden RI live exit recovery
9c0a5e2 Track exact RI live request ids
baabca2 Cover RI guard rollback lifecycle
f69669e Reduce RI model bar log noise
```

Covered behavior:

- entry moves through `live_pending_entry` and becomes `live_in_position` only
  after broker position confirmation;
- exit moves through `live_pending_exit` and returns to `flat` only after
  broker-flat confirmation;
- recoverable `trading_window_closed` exits enter `live_deferred_exit` and
  reissue on the next eligible model bar;
- runtime provides the exact final `request_id` via `on_command_prepared`;
- RI stores exact `pending_entry_request_id` / `pending_exit_request_id`;
- ack/reject handlers ignore mismatched pending ids and log
  `ri_pending_request_id_skew_detected`;
- host-dropped RI entries restore hidden strategy-local live state before the
  next bar;
- `ri_model_bar_observed` is debug-only heartbeat noise.

## Validation Already Run

Local validation before rollout:

```text
cargo test -p strategy-runtime ri_author41_42_live
cargo test -p strategy-runtime
```

Latest full result:

```text
strategy-runtime: 287 unit tests passed
e2e/replay/config/doc-tests passed
```

## Rollout Preconditions

Before restarting the VPS RI micro contour:

- Broker account `7502MIW` is flat for RI/RTS.
- No working RI/RTS regular orders.
- No working RI/RTS stop orders.
- Existing RI micro runtime and gateway containers are stopped.
- New image/binary includes the service-hardening commits listed above.
- Config is based on
  `configs/runtime.ri_author41_42.micro.7502MIW.pending.toml`.
- Gateway config uses action-scoped CWS only.
- `RIM6` model bars are present on `md.bars.7502MIW.RIM6.10m`.
- `order_symbol = "RTS-6.26"` remains configured unless the active contract
  full Alor symbol changes.

## From-Zero Runtime Reset

Use from-zero for operational live state, not for model bar history.

Clear or recreate only RI micro operational state:

```text
runtime.state.ri_author41_42.micro.7502MIW
cmd.orders.7502MIW.ri_author41_42.micro
cmd.acks.7502MIW.ri_author41_42.micro
consumer group: strategy-runtime-ri-author41-42-micro-7502MIW
decision journal: archive or rotate before restart
```

Do not delete the canonical model bar stream as part of normal from-zero:

```text
md.bars.7502MIW.RIM6.10m
```

The runtime should warm model state from retained/history bars and then wait for
the next live bar under the live guard.

## Expected Startup Logs

Expected healthy sequence:

```text
bootstrap flat / no working orders / no working stop orders
warmup completed
live_guard BLOCKED while syncing/bootstrap
live_guard ALLOWED after gateway and snapshots are ready
```

Expected RI-specific `INFO` events:

```text
ri_bootstrap_reconciled_flat
ri_model_decision
ri_candidate_intent_suppressed
ri_intent_emitted
ri_command_prepared
ri_live_entry_rejected_rolled_back
ri_live_exit_rejected_deferred
ri_manual_intervention_required
ri_pending_request_id_skew_detected
```

`ri_model_bar_observed` is no longer expected in normal `INFO` logs. It is a
debug-level heartbeat for cadence diagnostics only.

## NO-GO Signals

Stop the rollout or keep the stack flat if any of the following appears:

- legacy or long-lived CWS path is used for an RI command;
- broker rejects due to wrong `order_symbol` / unknown instrument;
- non-flat bootstrap enters anything other than manual-intervention mode;
- working order or stop-order bootstrap does not enter manual-intervention mode;
- `pending_entry_request_id` or `pending_exit_request_id` remains stale while
  broker truth is flat;
- RI emits an exit after a guard-dropped entry without a broker position;
- BO/MR overlap creates simultaneous live exposure;
- any position risks overnight carry without a safety/model exit.

## Post-Rollout Observation

Keep size at `1` and observe at least `3-5` additional trading sessions after
the service-hardening rebuild.

Daily observation should record:

- broker position at open/midday/EOD;
- number of RI commands emitted and accepted/rejected;
- entry/exit component attribution: `author41_mr` vs `author42_bo`;
- any deferred exit and its reissue result;
- any `ri_pending_request_id_skew_detected`;
- Redis memory after the session;
- whether `INFO` logs remain readable without `ri_model_bar_observed` noise.

Promotion beyond conservative micro requires a separate GO/NO-GO note after the
patched observation window.
