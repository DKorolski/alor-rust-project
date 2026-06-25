# RI Next-Bar-Open Timing Hardening - 2026-06-24

## Summary

RI Author41/42 live micro uses market entry/exit intents, not bracket TP/SL
lifecycle. The relevant parity issue is therefore timing, not protective-order
cleanup.

The frozen RI execution contract is:

```text
closed-bar signal -> next-bar-open execution proxy
```

A live signal finalized by a fresh completed bar must emit the market intent on
that bar boundary. It must not wait for one additional completed `10m` bar.

This is the same timing class as the Alor-USDRUBF patch
`a96a80a Align USDRUBF live entry with next-bar model timing`, but applied at
the shared runtime order-gating layer because the observed RI drop happened
after the RI strategy had already generated a candidate intent.

## Observed issue

On `2026-06-23`, both RI micro-live contours generated the first morning
candidate after overnight sync, but runtime order gating dropped it:

- `intent_dropped_bar_silence`;
- `intent_dropped_by_trading_window`;
- strategy state reverted to `flat`.

The next RI cycle emitted and filled normally, but one completed bar later.
Operationally this was safe and ended broker-flat, but it is a P1 parity issue
before RI scale-up because it can shift intended exposure by one `10m` bar.

## Root cause

`StrategyRuntime::handle_bar` called the strategy with a context whose
`last_bar_ts` intentionally points to the previous processed bar. That is useful
inside strategy callbacks: the strategy can compare the incoming bar against
persisted state.

However, live intents returned by that same `on_bar` callback were then gated
using the same previous-bar context. On the first fresh bar after an overnight
reconnect/history sync, the previous bar can be many hours old. The current bar
is fresh, but the order gate only saw the stale previous timestamp and rejected
the intent through `bar_silence`.

## Patch

For intents produced by a live bar callback, runtime order gating now uses an
emit context whose `last_bar_ts` is the current bar timestamp.

The strategy callback still receives the previous-bar context.

This keeps stale-bar protection for non-bar callbacks and real reconnect gaps,
while allowing a fresh current live bar to serve as the freshness proof for the
next-bar-open proxy intent it just generated.

## Regression coverage

Added runtime test:

```text
live_bar_intents_use_current_bar_for_silence_gate
```

The test reproduces the morning-open shape:

- previous processed bar is an old overnight timestamp;
- a fresh `DataOrigin::Live` RI bar arrives at the session open;
- the stale callback context would block an entry by bar-silence;
- the live-bar emit context allows the entry using the current bar timestamp.

## Rollout gate

Roll out only when both RI stacks are broker-flat or the operator explicitly
accepts restarting shared runtime while another symbol is open:

- `7502MIW / RTS-9.26 = 0`;
- `7502T0U / RTS-9.26 = 0`;
- no working RI strategy orders;
- no open strategy-managed positions on the same runtime stack unless an
  explicit manual exception is recorded.

Restart only the two RI strategy-runtime containers. Gateways and Redis do not
need to restart.

## Pre-rollout check — 2026-06-24 19:58 MSK

Patch is committed locally as `89c5967 Harden live bar intent freshness gate`.
Runtime image on both RI stacks before rollout:
`strategy-runtime:manual-20260618-lifecycle-68d1cd1`.

VPS resources were normal:

- load average about `0.79 / 0.41 / 0.33`;
- memory available about `6.5 GiB`;
- root disk `28%` used;
- both RI runtime, gateway, and Redis containers healthy.

Broker snapshots:

- `7502MIW`: `RTS-9.26 = 0`, but `IMOEXF = -3`;
- `7502T0U`: `RTS-9.26 = 0`, but `IMOEXF = -3`.

RI runtime logs for the last 30 minutes had no `warn`, `error`, `panic`,
`safe_mode`, `manual_intervention`, `orphan`, `rejected`, `intent_dropped`,
`partial`, or `emergency` records.

Decision: rollout held. The RI instrument itself is flat, but the runtime stack
is not fully flat because both portfolios currently hold an open IMOEXF short.
Deploy in the next fully-flat window, or only after an explicit operator
decision to restart shared runtime while IMOEXF is open.

## Rollout — 2026-06-25 07:52-07:57 MSK

Operator confirmed the safe window. Repeated preflight at `07:52-07:56 MSK`
showed both RI stacks fully flat:

- `7502MIW`: `RTS-9.26 = 0`, `IMOEXF = 0`, `USDRUBF = 0`;
- `7502T0U`: `RTS-9.26 = 0`, `IMOEXF = 0`.

VPS resources were normal before rollout:

- load average about `0.63 / 0.36 / 0.31`;
- memory available about `6.5 GiB`;
- root disk `28%` used;
- both RI runtime, gateway, and Redis containers healthy.

Built and loaded runtime image:

```text
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260625-ri-timing-b5b010a
```

Both stack `.env` files were backed up with suffix
`.env.bak.20260625-075642`, then `RUNTIME_IMAGE_TAG` was changed from
`manual-20260618-lifecycle-68d1cd1` to
`manual-20260625-ri-timing-b5b010a`.

Rollout command:

```text
docker compose up -d strategy-runtime
```

Note: in these compose files, this also recreated the `alor-gateway`
dependency for each RI stack. Redis was not restarted. The recreated gateways
came back healthy and CWS authorization succeeded immediately.

Post-rollout check at `07:56-07:57 MSK`:

- both RI runtime containers healthy on
  `strategy-runtime:manual-20260625-ri-timing-b5b010a`;
- both gateways healthy on `alor-gateway:manual-20260618-oauth-68d1cd1`;
- Redis containers remained healthy and were not restarted;
- runtime bootstrap reported zero open positions/orders/stops and
  `ri_bootstrap_reconciled_flat`;
- runtime state restored clean on both portfolios;
- latest broker snapshots were empty/flat after gateway restart;
- no `warn`, `error`, `panic`, `safe_mode`, `manual_intervention`, `orphan`,
  `rejected`, `intent_dropped`, or `emergency` records appeared in the
  immediate post-rollout logs.

At `07:57 MSK`, live guard was still expectedly `BLOCKED` on both runtimes while
waiting for the first live bar / session readiness before market open. This is
normal pre-session behavior; acceptance remains the `09:10 MSK` next-bar-open
parity check below.

## 09:10 acceptance check and follow-up patch — 2026-06-25

At `09:10:00 MSK`, both RI runtimes emitted the intended entry intent for the
`09:00-09:10` closed bar. This confirmed that the previous-bar freshness gate
patch fixed the original `bar_silence` drift.

However, both intents were still dropped by the runtime live guard:

```text
intent_dropped_by_guard reasons=["phase=SyncingHistory", "gateway_ready=false"]
```

Gateway reached `LiveReady` at about `09:10:00.25 MSK`, but runtime observed the
health/readiness transition only several seconds later (`09:10:08-09:10:11`).
The remaining issue is therefore a first-live-bar readiness propagation race,
not a model/freeze-bar parity issue.

Follow-up patch:

- when a live intent is blocked only by gateway readiness / phase / health
  freshness reasons, runtime now performs a bounded readiness grace loop;
- the loop force-refreshes the latest gateway health event from Redis up to
  eight times with a `250 ms` pause;
- if the guard becomes allowed inside that grace window, the original intent is
  emitted on the same bar and logs `live_guard_readiness_grace_allowed`;
- the grace path is deliberately not used for operator intervention, stale-bar
  restart waits, safe-mode/risk reasons, or other non-readiness blocks.

Regression coverage added:

```text
guard_readiness_grace_is_limited_to_gateway_readiness_race
```

## Freeze-intent hardening plan — 2026-06-25

The `09:10 MSK` check also exposed a stricter freeze-semantics problem. The
first RI MR intent was not merely late:

- `09:10:00.305 MSK` on `7502T0U`: Author41 MR `long / Buy` intent emitted,
  then dropped by guard with `phase=SyncingHistory`, `gateway_ready=false`;
- `09:10:00.878 MSK` on `7502MIW`: same `long / Buy` shape and same guard
  drop;
- runtime later observed `ALLOWED` only around `09:10:08-09:10:11`;
- at `09:20 MSK`, RI generated and executed a fresh `short / Sell`.

This is worse than a one-bar-late execution: the original frozen MR decision was
lost, the strategy was reverted to `flat`, and the next bar was allowed to
replace the missed `long` with an opposite-side `short`.

Patch semantics:

- keep the runtime readiness wait, but extend it to `240 x 250 ms`, i.e. up to
  about `60 s`, when the only guard reasons are gateway readiness / phase /
  health propagation;
- before invoking a strategy on a fresh live bar, perform the same readiness
  wait so hybrid/IMOEXF-style strategy publish gates can see `LiveReady` before
  deciding whether to emit bracket intents;
- if an RI entry intent is still blocked by readiness-only guard after the wait,
  do not silently roll strategy state back to `flat`;
- instead, RI records `live_entry_missed_runtime_not_ready`, preserves the
  frozen metadata (`component`, `side`, `cycle_id`, scheduled entry timestamp),
  clears any tentative internal live position, marks operator intervention
  required in runtime health, and blocks fresh replacement entries;
- BO receives the same defensive missed/blocking behavior if an already-frozen
  entry reaches this path, but no model-specific BO auto-retry semantics are
  introduced here.

Regression coverage added:

```text
micro_live_readiness_blocked_entry_marks_missed_and_blocks_opposite_replacement
ri_readiness_blocked_entry_hook_keeps_missed_state_and_blocks_replacement
```

Observability follow-up:

- `/readiness` now includes an `observability` block with counters for readiness
  waits, readiness timeouts, strategy-kept blocked intents, and `orphan_trade`;
- `/metrics` now exposes the same counters as Prometheus-style text when
  `health.expose_metrics=true`;
- the rollout acceptance check can therefore use both logs and fast health
  probes to detect any new `live_entry_missed_runtime_not_ready`,
  readiness-wait timeout, or fill-before-ack correlation event.

## Post-rollout acceptance

For the next RI morning session, compare:

- `model_signal_ts_local`;
- `scheduled_ts_local`;
- `ri_intent_emitted`;
- `ri_command_prepared.created_ts_utc`;
- broker `execution_confirmed`.

Expected behavior: a signal finalized at the `09:00-09:10 MSK` bar boundary
emits immediately after `09:10 MSK` and does not drift to the `09:20 MSK`
processing pass.
