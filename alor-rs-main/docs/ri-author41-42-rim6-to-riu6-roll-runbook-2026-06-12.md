# RI Author41/42 RIM6 To RIU6 Roll Runbook - 2026-06-12

## Decision

Prepare both VPS RI micro-live contours for a controlled futures roll:

```text
model symbol: RIM6 -> RIU6
order symbol: RTS-6.26 -> RTS-9.26
RIM6 expiry: 2026-06-18
RIU6 expiry: 2026-09-17
target preparation/switch window: before 2026-06-12 session
```

The active RIM6 configs remain unchanged until the explicit rollout GO.

Prepared candidates:

```text
configs/runtime.ri_author41_42.micro.7502MIW.RIU6.roll-2026-06-12.toml
configs/gateway.ri_author41_42.micro.7502MIW.RIU6.roll-2026-06-12.toml
configs/runtime.ri_author41_42.micro.7502T0U.RIU6.roll-2026-06-12.toml
configs/gateway.ri_author41_42.micro.7502T0U.RIU6.roll-2026-06-12.toml
```

Rollout quantities:

```text
7502MIW = 1
7502T0U = stopped / not part of this rollout
```

## Preparation Checkpoint - 2026-06-10

Candidate files were staged on the VPS without changing the active contours:

```text
/opt/trading-ri-author41-42-7502miw/configs/*RIU6.roll-2026-06-12.toml
/opt/trading-ri-author41-42-7502miw/docker-compose.RIU6.roll-2026-06-12.yml
/opt/trading-ri-author41-42-7502t0u/configs/*RIU6.roll-2026-06-12.toml
/opt/trading-ri-author41-42-7502t0u/docker-compose.RIU6.roll-2026-06-12.yml
```

Validation completed:

- both candidate compose files pass `docker compose config`;
- runtime candidate config tests pass for both portfolios;
- gateway candidate config tests pass for both portfolios;
- candidates resolve `RIU6`, `RTS-9.26`, and action-scoped execution;
- active containers remain healthy and still resolve their RIM6 streams.

No active config, compose file, runtime state, Redis stream, or running
container was changed during preparation.

## Calendar And Liquidity Gate

On `2026-06-10` the MOEX ISS snapshot showed:

```text
RIM6 volume ~= 73.4k, open interest ~= 52.8k
RIU6 volume ~= 4.4k, open interest ~= 11.2k
RIU6/RIM6 volume ratio ~= 6%
```

Do not perform the final switch solely because the account is flat. Re-check
RIU6 volume, open interest, and spread immediately before rollout.

Important `2026-06-12` calendar nuance:

- the MOEX futures market runs `09:50-23:50 MSK`;
- the session is an additional weekend/holiday session;
- its trading day is attributed to `2026-06-15`;
- the current RI runtime calendar sees Friday and accepts its bars as an
  eligible weekday model feed;
- Author42 BO explicitly excludes Friday and will not enter;
- Author41 MR does not exclude Friday and can enter if its morning signal
  conditions are met.

Preferred conservative rollout:

1. Complete the contract switch before the `2026-06-12` session.
2. Start only both RIU6 gateways on `2026-06-12` so they publish and retain the
   new contract's canonical `10m` feed/history.
3. Keep both RI strategy runtimes stopped during the holiday session unless
   DSWD MR trading is explicitly accepted.
4. Start both RIU6 runtimes from zero before the regular `2026-06-15` session;
   they will warm model state from the retained RIU6 history.

If the operator explicitly accepts DSWD trading, record that decision in the
live observation journal before starting the runtimes on `2026-06-12`.

## Invariants

The roll changes only the futures contract and its bar stream:

```text
RIM6 -> RIU6
RTS-6.26 -> RTS-9.26
md.bars.<portfolio>.RIM6.10m -> md.bars.<portfolio>.RIU6.10m
```

The following remain unchanged:

- strategy profile and decision logic;
- quantities;
- action-scoped-only execution contract;
- command and acknowledgement streams;
- runtime state key names;
- portfolio-specific Redis instances;
- no-overnight behavior.

RIM6 bars and reports remain retained for audit. They are not deleted during
the roll.

## Pre-Roll GO Checklist

Run immediately before touching either stack:

1. Confirm both portfolios are broker-flat for `RTS-6.26` and `RTS-9.26`.
2. Confirm no working regular or stop orders exist for either contract.
3. Confirm RI runtimes report flat with no pending entry/exit request.
4. Confirm RIU6 still has acceptable live quotes, spread, volume, and open
   interest.
5. Confirm the active gateway/runtime images are unchanged and healthy.
6. Decide and record whether the `2026-06-12` DSWD session is live-enabled or
   observation-only.

Any failed check is a NO-GO.

## VPS Stack Mapping

```text
/opt/trading-ri-author41-42-7502miw
/opt/trading-ri-author41-42-7502t0u
```

Active config names inside each stack remain:

```text
configs/gateway.ri_author41_42.micro.<portfolio>.toml
configs/runtime.ri_author41_42.micro.<portfolio>.toml
```

The candidate files must replace those active names only during the controlled
roll. The compose files also hard-code `STREAM_BARS`; change them from RIM6 to
RIU6 in the same rollout. Changing only TOML is not sufficient.

## Controlled Roll Procedure

Perform the procedure one portfolio at a time, starting with `7502T0U`.

For each stack:

1. Re-run the full pre-roll GO checklist.
2. Stop only `strategy-runtime` and `alor-gateway`; keep Redis running.
3. Archive:
   - active runtime and gateway TOML;
   - `docker-compose.yml`;
   - current RI decision journal.
4. Install the matching RIU6 candidate TOML under the active config names.
5. Change the compose gateway override:

```text
STREAM_BARS: md.bars.<portfolio>.RIM6.10m
```

to:

```text
STREAM_BARS: md.bars.<portfolio>.RIU6.10m
```

6. Clear only RI operational state:

```text
runtime.state.ri_author41_42.micro.<portfolio>
cmd.orders.<portfolio>.ri_author41_42.micro
cmd.acks.<portfolio>.ri_author41_42.micro
```

7. Destroy the runtime consumer group on the old RIM6 bar stream if present.
   Do not delete the old RIM6 bar stream.
8. Start `alor-gateway` first.
9. Confirm RIU6 history was published to
   `md.bars.<portfolio>.RIU6.10m`.
10. Start `strategy-runtime` from zero.
11. Wait for history warmup, broker-flat reconciliation, and
    `LiveReady / ALLOWED`.
12. Repeat for the second portfolio only after the first contour is clean.

## Required Post-Start Evidence

The resolved runtime/gateway state must show:

```text
symbol = RIU6
order_symbol = RTS-9.26
bars = md.bars.<portfolio>.RIU6.10m
control_cws_mode = action_scoped
execution_path = action_scoped_only
reset_state_on_start = true
broker position RTS-6.26 = 0
broker position RTS-9.26 = 0
working orders = 0
working stop orders = 0
```

Expected startup sequence:

```text
Subscribed to RIU6 (bars)
bootstrap: snapshots loaded
reset_state_on_start enabled; skipping runtime state restore
bootstrap: warmup from history bars
ri_bootstrap_reconciled_flat
live_guard ... LiveReady / ALLOWED
```

Stop immediately if:

- any resolved stream still contains `RIM6`;
- the runtime emits a historical intent during warmup;
- bootstrap sees a non-flat RI position or working order;
- legacy/long-lived CWS path appears;
- RIU6 history is missing or insufficient;
- the holiday-session decision is not explicit.

## Rollback

Rollback is between sessions only:

1. Stop RI runtime and gateway.
2. Restore archived RIM6 TOML and `docker-compose.yml`.
3. Clear the same RI operational state and command/ack streams.
4. Keep both RIM6 and RIU6 historical bar streams.
5. Start gateway, confirm RIM6 history, then start runtime from zero.
6. Confirm broker-flat and action-scoped-only state before leaving the stack
   running.

Do not perform an intraday cross-contract rollback or transfer.
