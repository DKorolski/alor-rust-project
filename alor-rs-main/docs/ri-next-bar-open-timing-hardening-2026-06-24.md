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

Roll out only when both RI portfolios are broker-flat:

- `7502MIW / RTS-9.26 = 0`;
- `7502T0U / RTS-9.26 = 0`;
- no working RI strategy orders.

Restart only the two RI strategy-runtime containers. Gateways and Redis do not
need to restart.

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
