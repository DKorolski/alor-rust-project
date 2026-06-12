# VPS Live Observations - 2026-06-12

## Context

Controlled pre-open rollout was completed on the VPS before the special
MOEX holiday/weekend session on `2026-06-12`.

The MOEX derivatives session runs from `09:50` to `23:50 MSK` and belongs to
trading day `2026-06-15`. This is not a regular weekday session, so all live
decisions and execution results must be reviewed as a separate DSWD case.

## Rollout

Runtime image:

```text
manual-20260612-bracket-residual
sha256:c2a822e85fbfec0c96656befd8b37af1ff197a82c34073974ab4774063e98f46
```

Started contours:

```text
RI Author41/42 RIU6, 7502MIW, qty=1
Alor-USDRUBF hybrid, 7502MIW, qty=1
IMOEXF primary hybrid, 7502MIW, qty=2
IMOEXF author41-short hybrid, 7502T0U, qty=2
```

The temporary RI contour on `7502T0U` remains stopped.

The affected runtime contours were restarted from zero. Only runtime-owned
state, command streams, acknowledgement streams, and bar consumer groups were
reset. Broker history, model bars, reports, and the IMOEXF risk-gate ledger
were preserved.

## Pre-Open Validation

All four runtime containers and their gateways are healthy.

Fresh broker snapshots reported:

```text
open futures positions = 0
open regular orders = 0
open stop orders = 0
```

Resolved sizes and execution contracts:

```text
RIU6 / RTS-9.26: qty=1, execution_path=action_scoped_only
Alor-USDRUBF: qty=1, gateway control_cws_mode=action_scoped
IMOEXF 7502MIW: qty=2, gateway control_cws_mode=action_scoped
IMOEXF 7502T0U: qty=2, gateway control_cws_mode=action_scoped
```

RI roll validation:

```text
bar stream = md.bars.7502MIW.RIU6.10m
RIU6 history bars loaded = 386
bootstrap broker state = flat
historical warmup intents emitted = 0
```

IMOEXF primary risk-gate validation:

```text
ledger rows = 209
last finalized session = 2026-06-10
rolling_sum_lb120 = 165.9
mr_enabled_current_session = true
mr_enabled_next_session = true
```

Before the first fresh DSWD bar, every runtime remains correctly blocked by
the startup live guard.

## Startup Replay Watchpoint

During from-zero replay, old broker trade/position events produced
`orphan_trade` and `unexpected_broker_residual` diagnostics in Alor-USDRUBF
and IMOEXF author41-short.

The new residual safety path attempted to create emergency flatten actions,
but the startup live guard dropped them and reverted strategy state to flat.
No command was emitted and no broker order was created.

This is safe in the observed startup, but remains a watchpoint:

- historical/recovered broker events should not reach residual emergency
  handling as normal live events;
- verify that no replay-origin residual action is emitted after the live guard
  becomes allowed;
- consider an explicit recovered-origin suppression guard in a follow-up patch.

## Resources

After rollout:

```text
host RAM available ~= 5.4 GiB
swap used ~= 426 MiB / 3.9 GiB
disk used ~= 21 GiB / 79 GiB (28%)

hybrid primary Redis ~= 116 MiB / 1 GiB
hybrid author41 Redis ~= 56 MiB / 512 MiB
Alor-USDRUBF Redis ~= 509 MiB / 1 GiB
RI 7502MIW Redis ~= 238 MiB / 768 MiB
```

Alor-USDRUBF Redis remains the main resource watchpoint. The standard
whitelist trim was applied before rollout, but its footprint is still high.

Follow-up Redis inspection showed that stream lengths are bounded and there is
no current unbounded stream growth. The largest streams are:

```text
events.health ~= 10.1k
broker.snapshots ~= 10.1k
broker.positions <= 5k
model bars <= 3k
```

The large Alor-USDRUBF footprint was primarily an old AOF tail plus allocator
accounting after previous growth. Online `BGREWRITEAOF` completed successfully:

```text
Alor-USDRUBF AOF: 402 MB -> 130 MB
Alor-USDRUBF Redis directory: 508 MB -> 249 MB
Alor-USDRUBF process RSS: ~= 138 MB

stopped RI 7502T0U AOF: 58 MB -> 5.4 MB
stopped RI 7502T0U Redis directory: 61 MB -> 11 MB
```

No runtime or gateway stop was required for the online rewrite.

Follow-up hardening watchpoint: Alor-USDRUBF and primary Hybrid Redis currently
report `maxmemory=0`. Docker memory limits still protect the host, but an
explicit Redis `maxmemory` below the container limit should be introduced in a
separate controlled rollout rather than immediately before this DSWD session.

## Session Watchlist

For the first fresh DSWD bars and any live trade:

- confirm transition to `LiveReady / ALLOWED` only after a fresh bar;
- confirm there are no historical/replay-origin intents;
- confirm RI uses `RIU6` bars and `RTS-9.26` orders;
- confirm all order sends use action-scoped CWS;
- monitor IMOEXF partial fills and exact residual emergency flatten behavior;
- monitor Alor-USDRUBF bracket TP/SL lifecycle;
- review all DSWD signals separately from regular-session evidence.
