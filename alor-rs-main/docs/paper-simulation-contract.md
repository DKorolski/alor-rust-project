# Paper Simulation Contract

## Scope

The shared Paper/Backtest simulator is not the live broker dispatcher and is
not the standalone reaction or RI context worker. Importing this implementation
does not deploy a runtime, migrate a consumer group, restore an order book or
change a live configuration. Action-scoped CWS execution is unchanged.

Two flags are explicit opt-ins; omitted flags and older serialized PaperConfig
values preserve false defaults:

```toml
[paper]
market_fill_on_close = false
simulate_stop_limits = false
```

`market_fill_on_close=true` applies only to Paper Market intents without an
explicit fill price. They settle on the current accepted bar's close. Explicit
model fill prices take precedence. Otherwise Market fills remain next-bar open;
the flag does not change Backtest pricing. Stop-limit simulation requires its
own flag and does not become enabled merely by selecting close-fill.

## Fill And Ownership Lifecycle

Immediate and queued fills update the trade ledger, runtime position and strategy
callbacks through one path. Terminal order feedback precedes position feedback,
allowing sibling TP/SL cleanup. Working and canceled synthetic orders also notify
the strategy. Callback-generated cleanup intents are processed before the next
original action after an immediate fill, and reports include those actions.

Pending fills settle before the strategy observes the next bar. A current-bar
context is used for paper intent freshness checks, while the on_bar callback
still receives the previous processed-bar context. In Paper live_only mode,
history bars cannot advance simulated execution.

Queued Exit and ProtectiveRepair fills are reduce-only and capped to the current
position. Flat or wrong-sided protection cannot open a reverse position. Orders
only use bars for their own symbol, and queued orders cannot fill on their
creation bar.

## Prices And Stops

Paper limit placement follows the gateway's directional grid policy: floor for
buy, ceil for sell. Replacement and stop trigger/limit prices round to the nearest
tick, including decimal stabilization. The strategy tick size must agree with the
gateway instrument price step. Raw bars and Market fill prices are not rounded.
The small helpers in paper_prices.rs mirror command_consumer's current rules;
changes to either side require an explicit contract comparison and tests.

Stop-limit orders report working, triggered, canceled and expired lifecycle
events and expose their exchange-limit child. A buy cannot fill above its limit,
nor a sell below it. A gap beyond the cap can leave that child working.

OHLC does not reveal intrabar ordering. Triggered stops are considered before
sibling TP orders, and paper_stop_triggered records that assumption. Strict or
inverted intrabar triggers do not invent a post-trigger retracement. This is an
explicit approximation, not tick-level execution parity. No partial-fill, queue
priority or market-impact model is introduced.

## EOD And Recovery Boundaries

Hybrid SameDay orchestration rejects new entries at/after 23:30 local time, after
allowing existing-position exits. This is shared strategy code, including live
Hybrid, and is therefore a separate change from the simulator. Overnight mode
retains its existing behavior; no forced-exit time is moved earlier.

Close-fill removes next-morning dependency only if a genuine eligible EOD bar
reaches the model and produces an exit. It does not manufacture missing bars,
add timers, repair historical overnight results or guarantee delivery by wall
clock. Model completeness and terminal-bar delivery remain separate gates.

The in-memory simulator order book is not made restart-durable here. Deploy only
in a verified-flat maintenance window with explicitly reviewed state/group/report
generations. Preserve canonical risk-gate ledgers; do not reset shared Redis or
silently reimport a seed. Keep old economics separate from corrected execution
generations, and do not treat liveness as fill/EOD acceptance.

## Verification

Regression fixtures cover real Hybrid BO ownership/EOD, MR TP/SL/time exits,
sibling cancellation, gap-limit caps, ambiguous touches, reduce-only protection,
history isolation, grid rounding, defaults and legacy configuration decoding.
Deployment-specific portfolio configurations and operational journals are not
required by the portable tests.

Run fmt, Clippy and the relevant package/workspace suites on the integrated
revision. Redis/native tests require their documented isolated opt-in setup;
see [Redis isolation and known E2E gap](testing-redis-isolation.md). An archive's
past deployment evidence is not qualification of a newly integrated image.
