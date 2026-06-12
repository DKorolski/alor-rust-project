# VPS Live Observations - 2026-06-09

## Review Scope

Context:

- VPS: `155.212.170.21`, host `nektodk.ispvds.com`.
- Reviewed regular session: `2026-06-09`.
- Current-state check: `2026-06-10 09:20 MSK`.
- Scope: all active trading contours, action-scoped Market rollout validation,
  warning/error review, broker truth, Redis maintenance, and VPS resources.

Executive read:

- all `15` containers were healthy;
- all `2026-06-09` strategy cycles converged to broker-flat;
- no stale regular order or stop-order tail was found;
- the new IMOEXF action-scoped Market path worked on both hybrid contours;
- the primary IMOEXF contour exercised and survived both an action-scope open
  timeout and a closed-window deferred-exit path;
- a broad evening Alor/OAuth/WS disruption blocked runtimes safely and later
  recovered without a trading incident.

## Session Results

### RI Author41/42

Both RI contours completed the same MR-long cycle:

- entry: approximately `09:20 MSK`;
- exit: approximately `09:40 MSK`;
- `7502MIW`: buy `2` near `108530`, sell `2` near `108770`;
- `7502T0U`: buy `1` near `108540`, sell `1` near `108760`.

Interpretation:

- entries and exits used action-scoped Market;
- the `7502MIW` quantity-2 entry arrived as two quantity-1 execution events and
  reconciled correctly;
- both portfolios returned flat;
- approximate gross result was `+240` points per contract on `7502MIW` and
  `+220` points on `7502T0U`, before commissions.

### Alor-USDRUBF On `7502MIW`

Observed BO-short cycle:

- entry: sell `1` around `11:20 MSK` at approximately `71.99`;
- EOD exit: buy `1` around `23:40 MSK` at approximately `71.71`;
- broker position returned flat.

Interpretation:

- the cycle belonged to BO, so no MR protective bracket was expected;
- action-scoped entry and exit completed normally;
- approximate gross result was `+0.28` price units, or about `+280 RUB` before
  commissions.

### IMOEXF Author41-Short On `7502T0U`

Observed BO-short cycle:

- entry: sell `2` around `12:20 MSK` at `2486.5`;
- exit: buy `2` around `13:00 MSK` at `2509.5`,
  reason `BreakoutStop1Short`;
- broker position returned flat.

Interpretation:

- both commands used the new action-scoped Market path;
- no passive working entry, stale order, or stop-order tail appeared;
- approximate gross result was `-23` points per contract, about `-460 RUB`
  total before commissions.

### Primary IMOEXF Hybrid On `7502MIW`

Observed BO-short cycle:

- entry: sell `4` around `12:20 MSK` at `2486.5`;
- the execution event arrived before the acknowledgement and produced one
  convergent `orphan_trade` warning;
- first exit attempt around `13:00 MSK` failed before emit while opening the
  action-scoped CWS session: `action_scope_session_open_error/open timeout`;
- the next model retry around `14:00 MSK` occurred during Break1 and was
  rejected with `trading_window_closed`;
- runtime correctly created a deferred exit;
- deferred exit was reissued after reopen around `14:10 MSK`, filled at
  `2515.0`, and returned the broker position flat.

Interpretation:

- action-scoped Market removed the previous stale passive marketable-limit
  entry risk;
- closed-window defer/reissue recovery worked as designed;
- the action-scope open timeout remains a frequency-watch item;
- the fill-before-ack warning converged through later broker truth;
- approximate gross result was `-28.5` points per contract, about `-1,140 RUB`
  total before commissions.

## Transport And Recovery Review

Around `16:28-17:20 MSK`, several gateways experienced a synchronized external
transport disruption:

- websocket ping timeout;
- OAuth refresh failures;
- repeated subscription retry exhaustion;
- protocol resets and reconnects.

Runtimes moved to blocked states while gateway readiness was unavailable and
returned to `LiveReady` after recovery. No active command was identified as
lost, and no uncontrolled position resulted.

Additional overnight EOF/reset events occurred without in-flight command
opcodes and converged normally.

## Morning State - 2026-06-10

All runtimes reached `LiveReady / ALLOWED`.

Broker truth around `09:20 MSK`:

- `7502MIW`: `IMOEXF = 0`, `USDRUBF = 0`, `RTS-6.26 = +2`;
- `7502T0U`: `IMOEXF = 0`, `RTS-6.26 = +1`;
- regular and stop-order snapshots did not show stale protective-order tails.

The two RI long positions were fresh same-session model entries. Both used
action-scoped Market and were accepted and filled. They are controlled live
positions, not stale overnight tails.

A final `09:23 MSK` scan found no fresh `WARN`, `ERROR`, reject, failure,
`orphan_trade`, or dropped-intent event in any active runtime or gateway during
the preceding `15` minutes.

## VPS Resources And Redis

Host resources at the morning check:

- load average: approximately `0.21 / 0.32 / 0.37`;
- RAM: approximately `2.2 GiB / 7.7 GiB` used, `5.5 GiB` available;
- swap: approximately `84 MiB` used;
- root disk: approximately `20 GiB / 79 GiB`, `27%`;
- no resource-pressure or OOM condition was identified.

The scheduled whitelist safe trim completed successfully at
`2026-06-10 08:10 MSK`.

Post-trim Redis memory:

- primary IMOEXF hybrid: approximately `94.66 MiB`;
- Alor-USDRUBF: approximately `261.49 MiB`;
- RI `7502MIW`: approximately `219.26 MiB / 512 MiB`;
- IMOEXF author41 `7502T0U`: approximately `17.21 MiB / 512 MiB`;
- RI `7502T0U`: approximately `14.59 MiB / 512 MiB`.

All values remained inside the current maintenance envelope. Alor-USDRUBF and
RI `7502MIW` remain the largest Redis instances and should stay in the weekly
resource review.

## Current Read

- Extended micro soak remains operationally acceptable.
- The hybrid action-scoped Market rollout has now passed its first live
  entry/exit validation on both IMOEXF contours.
- The primary IMOEXF recovery path correctly handled open timeout, closed
  trading window, deferred exit, and later broker-flat convergence.
- Keep action-scope open timeout frequency, fill-before-ack warnings, and BO
  retry semantics on the watchlist.
- No immediate patch or emergency maintenance is required before continuing
  the session.
