# RI Author41/42 RIU6 To RIZ6 Roll Runbook - 2026-09-16

## Decision

Roll the RI Author41/42 `7502MIW` live micro contour from the September 2026
contract to the December 2026 contract in the final RIU6 flat maintenance
window.

```text
model symbol: RIU6 -> RIZ6
order symbol: RTS-9.26 -> RTS-12.26
RIU6 last circulation / expiry date: 2026-09-17
RIZ6 last circulation / expiry date: 2026-12-17
roll window: 2026-09-16 evening, after RI broker-flat confirmation
```

This is a contract-only roll. Strategy logic, live07 session policy,
Author41/Author42 parameters, quantity, and action-scoped execution remain
unchanged.

## Prepared Candidate Files

```text
configs/gateway.ri_author41_42.micro.7502MIW.RIZ6.roll-2026-09-16.toml
configs/runtime.ri_author41_42.micro.7502MIW.RIZ6.live07.toml
```

The active VPS compose path must also change:

```text
STREAM_BARS=md.bars.7502MIW.RIZ6.10m
GATEWAY_CONFIG=/configs/gateway.ri_author41_42.micro.7502MIW.RIZ6.roll-2026-09-16.toml
RUNTIME_CONFIG=/configs/runtime.ri_author41_42.micro.7502MIW.RIZ6.live07.toml
```

Changing only TOML is not sufficient because `STREAM_BARS` is set in
`docker-compose.yml`.

## Pre-Roll GO Checklist

Run immediately before touching the RI stack:

- broker position `RTS-9.26` is flat;
- broker position `RTS-12.26` is flat;
- no working regular orders exist for `RTS-9.26` or `RTS-12.26`;
- no working stop orders exist for `RTS-9.26` or `RTS-12.26`;
- RI runtime state is `phase=flat`;
- RI runtime has no pending entry or pending exit request;
- current RI gateway path is healthy or intentionally stopped;
- RIZ6 is visible in MOEX/Alor reference data as `RTS-12.26`;
- operator accepts that RIU6 will not be restarted for normal live trading on
  `2026-09-17`.

Any failed check is a NO-GO.

## Controlled Roll Procedure

1. Stop only the RI `strategy-runtime` and RI `alor-gateway` containers.
2. Keep the RI Redis container running.
3. Archive active runtime config, gateway config, compose, `.env`, and current
   RIU6 decision journal.
4. Install the RIZ6 gateway/runtime candidate files on the VPS.
5. Update `.env` to point to the RIZ6 configs.
6. Update `docker-compose.yml` `STREAM_BARS` to `md.bars.7502MIW.RIZ6.10m`.
7. Clear only RI operational state:
   - `runtime.state.ri_author41_42.micro.live07.7502MIW`;
   - `runtime.state.ri_author41_42.micro.live07.RIZ6.7502MIW`;
   - `cmd.orders.7502MIW.ri_author41_42.micro`;
   - `cmd.acks.7502MIW.ri_author41_42.micro`;
   - consumer groups on `md.bars.7502MIW.RIU6.10m` and
     `md.bars.7502MIW.RIZ6.10m` if present.
8. Start the RI gateway first.
9. Confirm RIZ6 bars/history are published to `md.bars.7502MIW.RIZ6.10m`.
10. Start the RI runtime from zero.
11. Confirm startup reconciliation:
    - `symbol=RIZ6`;
    - `order_symbol=RTS-12.26`;
    - `execution_path=action_scoped_only`;
    - `phase=flat`;
    - `LiveReady / ALLOWED`;
    - no startup replay intent is emitted for historical bars.

## Stop Conditions

Stop the RI runtime/gateway and do not continue if:

- any config/log still points to `RIU6` or `RTS-9.26` after the RIZ6 switch;
- RIZ6 history is missing or too short for warmup;
- broker snapshot shows non-flat `RTS-9.26` or `RTS-12.26`;
- any working order remains on either RI contract;
- gateway emits via legacy/long-lived CWS path instead of action-scoped path;
- runtime emits historical/startup replay live intent.

## Rollback

Rollback is between sessions only:

1. Stop RI runtime and gateway.
2. Restore archived RIU6 `.env`, `docker-compose.yml`, and RIU6 TOML files.
3. Clear the same RI operational state and command/ack streams.
4. Start gateway, confirm RIU6 bars/history, then start runtime from zero.
5. Confirm broker-flat and action-scoped-only before leaving the stack running.

Do not run both RIU6 and RIZ6 live runtimes simultaneously.
