# VPS Live Observations - 2026-06-07

## Sunday Pre-Session Check

All active containers were healthy.

VPS resources:

- load average: approximately `0.19 / 0.35 / 0.31`;
- available RAM: approximately `5.8 GiB / 7.7 GiB`;
- swap used: approximately `221 MiB / 3.9 GiB`;
- root disk used: `27%`;
- no kernel OOM or killed-process event was found.

Broker truth:

- `7502T0U`: no instrument positions, regular orders, or stop orders;
- `7502MIW`: `IMOEXF = 0`, `USDRUBF = 0`, `RTS-6.26 = 0`.

## Overnight Gateway Reconnects

Several gateways experienced synchronized WS/CWS resets during the Sunday
service window around `06:24-06:35 MSK`.

Observed classes:

- peer EOF without TLS close notification;
- protocol reset without close handshake;
- transient positions subscription ack timeout.

The gateways reconnected and CWS authorization succeeded. No command was in
flight, no reject followed, and broker truth remained flat. Treat this as a
convergent weekend transport event, not a trading incident.

## RI `7502T0U` Per-Stream Retention Rollout

Before rollout:

- RI Redis had grown from approximately `1.6 MiB` to `73.7 MiB` in about
  `17` hours;
- main growth sources:
  - `events.health.ri_author41_42.7502T0U = 12073`;
  - `broker.snapshots.7502T0U = 6039`;
- positions, orders, stops, commands, and trades were empty or minimal.

Applied gateway-only image:

- `manual-20260606-perstream-retention`.

Applied limits:

- bars: `3000`;
- orders/trades/commands/acks: `5000`;
- positions/snapshots: `2000`;
- health: `1500`.

Validation:

- gateway and runtime remained healthy;
- `control_cws_mode = action_scoped`;
- CWS authorization succeeded;
- transient `gateway_health_stale` cleared after the first fresh heartbeat;
- no runtime restart or from-zero reset was performed;
- no historical or fresh `intent_emitted` appeared;
- latest broker snapshot remained empty for positions, orders, and stops.

After rollout:

- health stream: `1500`;
- snapshots stream: `2000`;
- positions stream: `4`;
- bars stream: `985`, bounded by `3000`;
- RI Redis memory: approximately `10.83 MiB / 512 MiB`.

Status: rollout healthy. Continue observing both `7502T0U` contours during the
next regular session.
