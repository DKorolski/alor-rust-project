# VPS Live Observations - 2026-06-15

## RI 7502T0U Controlled Start

The corporate RI emitter for portfolio `7502T0U` was explicitly confirmed
stopped before the VPS contour was enabled.

Pre-start checks:

```text
portfolio = 7502T0U
broker RI position = flat
working RI orders = 0
working RI stop orders = 0
host RAM available ~= 6.3 GiB
disk used ~= 27%
```

The stopped VPS contour still selected the old `RIM6 / RTS-6.26` configuration
and an old runtime image. It was not started in that state.

The prepared controlled-roll candidates were activated instead:

```text
model symbol = RIU6
order symbol = RTS-9.26
qty = 1
execution_path = action_scoped_only
control_cws_mode = action_scoped
gateway image = manual-20260606-perstream-retention
runtime image = manual-20260612-bracket-residual
bars = md.bars.7502T0U.RIU6.10m
reset_state_on_start = true
```

The previous active config, compose file, and environment selection were
archived on the VPS under:

```text
/opt/trading-ri-author41-42-7502t0u/roll-backup-20260615-085327
```

## From-Zero Scope

Only RI operational state was cleared:

```text
runtime.state.ri_author41_42.micro.7502T0U
cmd.orders.7502T0U.ri_author41_42.micro
cmd.acks.7502T0U.ri_author41_42.micro
old RIM6 runtime consumer group
```

Redis, broker streams, reports, and the old RIM6 audit bar stream were
preserved.

## Bootstrap Evidence

The RIU6 gateway was started before the runtime and confirmed:

```text
gateway healthy
Subscribed to RIU6 (bars)
CWS authorization successful
RIU6 history bars loaded = 371
commands = 0
acks = 0
```

The runtime then started from zero and confirmed:

```text
runtime healthy
ri_bootstrap_reconciled_flat
reset_state_on_start enabled; skipping runtime state restore
history warmup bars processed = 371
historical intent emissions = 0
```

Immediately before the first regular-session live bar, the runtime remained
safely blocked with `bootstrap:missing_live_bar` / `SyncingHistory`. This is
the expected pre-session state. The established RIU6 contour on `7502MIW`
reported the same gateway `SyncingHistory / readiness=false` state at the same
time, confirming that the new T0U contour was waiting for the market rather
than failing independently.

Confirm `LiveReady / ALLOWED` and continued zero historical command emission
after the first live RIU6 bar.

## Post-Open Verification

The first regular-session RIU6 live bar arrived at `09:10 MSK`:

```text
close_time_utc = 1781503200
origin = live
close = 107410
volume = 214
```

The T0U RI gateway and runtime then transitioned normally:

```text
gateway: SyncingHistory -> LiveReady
runtime: BLOCKED -> ALLOWED
```

The transition was synchronous with the established RI contour on `7502MIW`.
No historical or post-open command was emitted by the new T0U contour.

Post-open broker truth for `7502T0U`:

```text
positions = {}
working orders = 0
working stop orders = 0
```

All active runtimes reached `LiveReady / ALLOWED`. The post-open scan found no
new `WARN`, `ERROR`, command rejection, or timeout.

Post-open resources remained healthy:

```text
host RAM available ~= 6.3 GiB
disk used ~= 27%
RI T0U Redis used_memory ~= 17.9 MiB / 512 MiB
```

## Morning Trading Read

### RI Author41/42

Both RI contours produced the same valid Author41 MR short decision and
completed the full action-scoped entry/exit path:

```text
model signal = author41_mr short, 09:50 MSK
entry emission = 10:00 MSK
exit decision = stop, 10:00 model bar
exit emission = approximately 10:10 MSK

7502MIW: sell 107900 -> buy 108500, flat
7502T0U: sell 107890 -> buy 108500, flat
```

Every RI entry and exit used a fresh action-scoped `create:market` session and
received broker HTTP `200`. Both runtime states ended:

```text
phase = flat
pending_entry_request_id = null
pending_exit_request_id = null
last_transition_reason = live_position_flat_confirmed
```

`orphan_trade` diagnostics appeared for RI fills because trade events reached
the runtime before the corresponding correlation lifecycle was complete.
They did not cause duplicate commands or stale state in this session, but
remain an observability/correlation watchpoint.

### Alor-USDRUBF Bracket Race

Alor-USDRUBF produced a valid MR short signal and initially followed the
expected bracket path:

```text
09:50 MSK: short entry filled at 72.64
TP buy limit = 72.56
SL buy stop-limit trigger = 72.85
```

The first TP filled at `72.56`, correctly flattening the short. However, before
the flat position event completed bracket cleanup, the runtime emitted a
second protective-repair TP at the same `72.56`.

Exact sequence:

```text
initial TP filled -> broker flat
second TP protective_repair emitted
SL canceled
second TP filled -> unexpected long residual +1
broker_residual_emergency_exit sold residual at 72.55
broker flat restored
```

The action-scoped execution path itself worked correctly and every command was
accepted. The residual safety mechanism also worked and left:

```text
USDRUBF position = 0
working orders = 0
working stop orders = 0
```

This is not a transport failure, but it is a bracket lifecycle race and should
not be classified as fully штатный execution. A follow-up patch must prevent
protective repair after a sibling TP has filled or while flat reconciliation
is in progress.

### IMOEXF

Neither IMOEXF contour emitted an intent during the inspected morning window.
Both remained healthy and flat.

## Alor-USDRUBF Patch Prepared

The bracket lifecycle race was reproduced as a runtime state-machine issue and
patched locally. After an MR TP/SL terminal fill, the runtime now enters an
`exit_intent_inflight` waiting state until broker position truth confirms
flat. Protective repair is suppressed during that interval.

The patch preserves legitimate repair after canceled/rejected/expired
protection and preserves flat-position sibling cleanup.

Regression coverage includes the observed sequence:

```text
TP filled
repair attempt suppressed
repeated non-flat broker snapshot does not re-arm protection
broker-flat snapshot performs sibling cleanup
```

Verification:

```text
Alor-USDRUBF strategy tests: 28 passed
strategy-runtime full test suite: passed
cargo fmt --check: passed
git diff --check: passed
```

`cargo clippy -p strategy-runtime --lib -- -D warnings` remains blocked by 11
pre-existing warnings outside the modified Alor-USDRUBF strategy module.

The patch was intentionally not deployed during the active trading session.
Recommended rollout is a controlled Alor-USDRUBF-only restart from zero in a
safe flat window after confirming no USDRUBF position, working order, or stop
order at the broker.

## RI Rollover Audit Follow-Up Prepared

The June rollover audit confirmed that the existing weekday-only RI model
session guard allowed the special `2026-06-12` DSWD session to become the
previous-session anchor for `2026-06-15`.

A local P0 patch was prepared:

- configured DSWD/holiday dates are excluded from RI model state and live
  intent emission;
- completed previous sessions must pass configurable bar-count and first/last
  bar quality checks before becoming MR/BO anchors;
- internal RI shadow/decision rebuild uses the same eligible-anchor set;
- RI config now records actual expiry plus manual target/fallback roll
  offsets;
- startup logs expose the loaded rollover/session-quality contract.

The RIU6 candidate configs now define:

```text
excluded_model_dates = 2026-06-12, 2026-11-04
min_anchor_bars = 80
first anchor bar <= 09:10 MSK
last anchor bar >= 23:30 MSK
actual_expiry_date = 2026-09-17
target roll = expiry - 1 trading session
fallback roll = expiry - 2 trading sessions
```

Verification:

```text
RI strategy tests: 36 passed
runtime config tests: 26 passed
full strategy-runtime suite: passed
```

The patch was not deployed during the active session. Automatic contract
switching remains intentionally out of scope; the September roll stays a
controlled between-session operation.
