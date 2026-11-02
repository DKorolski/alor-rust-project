# Shadow WS Gateway and Terminal-Bar Validation Plan

Date: 2026-11-02.
Status: implementation in progress; isolated smoke and prospective shadow soak
are authorized. The [component progress note](shadow-ws-terminal-implementation-2026-11-02.md)
records the first core/transport slice and its verification boundary. No deployed
WS source, clean prospective session or future live promotion is claimed here.

## Decision and Scope

Use the maintained gateway WebSocket candle path as the operational basis for
the new shadows. Add a narrow opt-in terminal-bar finalizer and a market-data-only
launch role. Do not build another continuously polled REST source or rewrite the
numerical strategies. REST is not a forward liveness/finality dependency.

The existing WS router keeps the latest update of a candle and emits it when a
greater timestamp arrives. The new REST source instead requires repeated equality
and can fail after a broker revises a previously published candle. Stable REST
observations are not an exchange finality certificate. The replacement must use
an explicit as-observed event contract, not promise permanently immutable quotes.

Initial instruments: the approved RI contract, IMOEXF and USDRUBF, canonical M10.
Pin the actual RI contract/expiry per generation; no automatic rollover is added.
Both arms of a model pair must consume the same new stream and receipt sequence.

Preserve all live runtimes/gateways, action-scoped command paths, models, K,
quantities, account allocation, MR/BO windows, ownership and exit deadlines.
Keep the 14 legacy shadows as RUNNING / RETIREMENT_PENDING_REVIEW. No deletion,
shared Redis reset/trim, live stream replacement or model promotion is authorized.
Retain the active RI collector until a reviewed new-source generation is admitted;
do not silently change the source of a running collector.

## WP1: Frozen Source and Terminal Contract

Define versioned policy and provenance before coding:

- Ordinary intraday release remains next-timestamp-driven. The opt-in fallback
  affects only the expected terminal label of an approved instrument/session.
- M10 labels denote interval BEGIN, despite `close_time_utc`. A weekday 23:40
  label closes at 23:50. An approved weekend 18:50 label closes at 19:00. These
  are examples, not a substitute for per-instrument holiday/short-day calendars.
- A monotonic scheduler checks after interval end plus a bounded grace, initially
  5-10 seconds for testing. Publication still fits the existing source/consumer
  freshness budgets. Do not introduce another 10-minute model delay.
- Release only a real, validated buffered WS candle: correct symbol/grid/date,
  current acknowledged subscription/generation, healthy connection/heartbeat,
  stable clock and no unresolved reconnect/backfill gap. Establish which queued
  WS prefix has been processed before committing the terminal snapshot; do not
  race an undrained update queue with the timer.
- Wall clock alone is insufficient. Record the accepted calendar boundary,
  first/last real WS receipt, observation cutoff, connection generation and
  publication time. Healthy WS plus an elapsed interval defines a reviewed
  observation cutoff, NOT proof that all future broker corrections are absent.
  If exchange-certified finality is required, this contract does not meet it.
- Missing/unconfirmed/stale terminal data produces a visible failure/degraded
  state. Never create OHLCV, carry yesterday's price, label History as timely Live
  or infer flat/completion without an input candle.
- Keep raw volume and validation. IMOEXF reaction is price-only. The new USD
  PreviousDay family does not use VWAP for signal levels, unlike live USD MR.
  Preserve this distinction; do not remove volume from live USD or raw evidence.
- After publication, never revise the canonical candle or frozen model context.
  Valid late WS corrections are quarantined as bounded audit events, not another
  model bar and not a blanket source-stop trigger. Keep original/observed values,
  receipt clocks, affected label/context/decision lineage and hashes. An OHLC
  correction affecting used model input makes that session's clean/parity status
  review-pending; continued collection is not reference parity PASS. Volume-only
  changes need the profile's signal-invariance evidence, with raw/auxiliary
  differences explicit. Invalid current input, tampered canonical Redis payload,
  clock faults and unresolved continuity remain failures. No arbitrary price
  tolerance is introduced to hide a material correction.

There are two completion notions: a temporal as-observed candle and eventual
restated history. Keep them separate in reference/source-drift reports. Adding
more REST confirmation reads is not a solution to that distinction.

## WP2: Narrow Shared Gateway Patch

Target `alor-gateway/src/router.rs`, its configuration and WS lifecycle wiring.
Reuse a small tested finalizer core in shadow and any later live release; avoid
a shadow-specific reimplementation that cannot be promoted without rewriting.

1. Add an explicit configuration policy, disabled by default. Existing configs,
   `Router::start` callers and default live behavior retain their semantics.
2. Track the buffered candle, WS receipt/generation evidence and per-symbol
   release watermark. Supply connection/subscription/gap state explicitly; do
   not equate a surviving process with a healthy subscription.
3. Add the terminal scheduling branch and explicit health/audit events. Keep all
   ordinary bars, price fields, volume, model calendar and command code unchanged.
4. Publish once through the normal validation/publisher path. Advance the durable
   release identity only after confirmed publication. Retain or reconcile an
   uncertain append; a timeout must not cause blind duplicate delivery.
5. Define bounded watermark restoration from the owned source evidence after
   restart. Handle timer/next-bar races, duplicate/out-of-order frames and
   reconnect generation changes without releasing stale buffered Live data.
6. Use a dedicated market-data-only role/runner. The current `WsHub::subscribe_all`
   also subscribes account data and the full supervisor expects CWS readiness;
   launching the trading supervisor unchanged is NOT a broker-free MD role.
   Reuse auth/reconnect/bars subscription code, but instantiate no CWS client,
   command consumer, order publisher or portfolio subscriptions in the MD role.
   Model workers receive neither source credentials nor a broker network route.

No giant gateway/runtime refactor, generic timer-driven strategy callbacks,
signal retune or lifecycle normalization is included in this patch.

## WP3: Isolation, Identity and Bootstrap

- New gateway artifact/image, source schema/policy ID, stream keys, source state
  and reader generations; pin code/config/binary/calendar/instrument hashes.
  Keep original WS receipt time distinct from Redis IDs/publication clocks.
- Source writer ACL: only the new allowlisted MD/health/audit keys. Worker ACL:
  read-only on its approved source keys. Deny live-state reads, command writes,
  group/trim/delete access as appropriate. Test denial, not just configuration.
- Bind worker readiness to the new source identity and real health/freshness.
  Extend the supervisor's source guard explicitly for the WS source version;
  do not bypass the existing guard by choosing a different stream prefix.
- Historical bootstrap may use authenticated WS history. An explicit offline
  REST import is permitted only as labelled/pinned History, never as a silent
  forward fallback or a fabricated original receipt. OAuth refresh HTTP remains
  necessary; "no REST load" means no routine forward candle polling.
- Reaction bootstrap requires 2-4 complete contiguous approved regular sessions
  through the last eligible day before activation, plus relevant context. It is
  not enough to have any two old days while omitting a more recent regular day.
  Inspect with the actual worker and pin bounds/rows/hash before activation.
- Preserve source-consistent weekday/weekend paired context, cutoff/freeze,
  suppression/counter/owner trace, costs and no-overlap. Downloaded history must
  not generate historical forward decisions. RI keeps its own warmup contract.
- REST-source failed generations remain immutable evidence. Cancel obsolete
  scheduled REST admission; retain units/configs/FAILED markers and journals.
  No in-place rename of old results into WS observations.

## WP4: Deterministic and Integration Smoke

All fixtures run against owned clocks, fake/owned WS and owned ephemeral Redis.
Never run default-localhost integration tests against a service Redis instance.

| Check | Required result |
| --- | --- |
| Feature disabled | Existing intraday/terminal behavior unchanged |
| Identical ordinary WS frame prefix | Old/new nonterminal bars equal and ordered |
| Repeated current-bar updates | Latest real update buffered, no early model bar |
| Terminal without next tick | One observed terminal bar after boundary/grace |
| Before/exact boundary, grace, clock step | No premature release; clock fault visible |
| Timer and next-bar race | One release, no reordered/fabricated model input |
| Missing candle or unresolved WS gap | No synthetic terminal or completion |
| Disconnect/resubscribe around close | Old generation cannot finalize as current Live |
| Duplicate/late correction | Canonical input unchanged; bounded audit and policy outcome |
| Weekend, excluded holiday, short session | Instrument-specific boundary; no forbidden trades |
| Publication timeout/crash/restart | Reconciled identity; no blind append/release duplication |
| Late history after frozen context | No retroactive anchor/decision rewrite |
| Volume perturbation | Proven profile signal/decision invariance; raw/auxiliary differences explicit |
| MD-only role | Only MD/auth requests; no CWS/account/order opcode or command publisher |
| Credentials/ACL | Worker has no token; forbidden Redis operations denied |
| Both numerical arms | No historical intents, correct context/owner/quota and complete EOD |

Run gateway/router/config compatibility tests, existing reaction/RI tests,
independent expected-state/reference checks, fmt and clippy. A synthetic smoke
is not a clean prospective session or an economic result. Capture manifests,
test versions and exit statuses without tokens/authorization headers.

## WP5: Native Canary and Admission

1. Record existing live/legacy container identities, source cursors and resources.
   Build/package from a clean scoped commit and pin the target binary/image.
2. Native no-network/fake-input smoke under real source/worker OS users, with
   cgroup limits and owned temporary files/Redis. Inject faults only there.
3. Start the isolated WS gateway source-only. Verify auth/ACK, bounded history,
   normal closed-bar cadence, reconnect and terminal publication on the VPS.
   Compare normal bars with retained WS controls; separately classify genuine
   source/receipt differences. No new model decisions are counted yet.
4. Confirm at least one real terminal cycle per enabled session policy before
   claiming it observed. Weekday success does not certify weekend/holiday close.
5. Admit IMOEXF/USD pairs to fresh WS-source generations before their next approved
   cutoff, after complete bootstrap and source/worker identity checks. No fixed
   launch date overrides a failed readiness gate.
6. Move RI only through a separate source-pinned collector generation after its
   own smoke/warmup checks. Keep the current RI collector/legacy controls intact
   while this is incomplete; never splice their clean-session counters.
7. Verify fresh input, armed/caught-up, matching frozen contexts in both arms,
   broker emission technically denied, no pending inherited from REST and unchanged
   live/legacy identities. Preserve explicit source/storage capacity headroom.

Real WS/auth reads and isolated shadow smoke/soak are authorized within these
boundaries. Broker order/account mutation, live gateway restart, shared input
replacement and synthetic prospective input remain prohibited.

## WP6: Twenty Clean Sessions and Operations

Maintain separate gateway and per-model-pair counters. A clean gateway session
requires the expected input cadence, one timely terminal bar, intact publication
and restart evidence, no unexplained duplicates/gaps and acceptable resource use.
A clean model-pair session additionally requires valid context, complete decisions
and resolved EOD state for both arms. Uptime, bootstrap or half a session does
not count. A feed revision requiring exclusion remains visible, not erased.

Review after the first complete session, 5 sessions and at least 20 common clean
sessions for each affected pair. Monitor weekend/holiday cases and both RI sides;
extend observation if the required edge cases or meaningful decisions are sparse.
Gateway acceptance is independent of choosing a profitable model winner.

Daily checks: source generation/ACK/connection, last received/published label and
age, terminal reason/latency/count, model context/cursor/owner/pending, EOD, denied
broker emission and audit health. Weekly checks: RAM/CPU/disk, Redis used/maxmemory,
evictions, persistence, source/audit/storage growth and bounded generation limits.
Do not persist every in-progress WS update indefinitely. Predeclare record/byte
budgets, warning/maintenance thresholds and explicit archive/rollover procedure;
no trim of data needed by a reader, automatic deletion or unbounded revision log.

## WP7: Separate Live Gateway Decision

After the required evidence, review the gateway patch independently of model
promotion. Twenty clean sessions are necessary review evidence, not automatic GO.
Require no unexplained ordinary-bar drift, terminal and recovery acceptance,
relevant calendar cases, source/provenance agreement and bounded resources.

A later live release may enable the same tested terminal policy while leaving
all live model parameters, quantities, instruments and action-scoped execution
unchanged. Audit every consumer of a changed stream. Deploy in an explicitly
approved flat/order-clean maintenance window, one writer per source, with a
pinned last-good artifact and checkpoint/source continuity plan. Stage affected
stacks rather than restarting all live systems as part of shadow admission.

The final 23:40 candle is available after 23:50; it cannot be the trigger for an
earlier required live flatten. A live EOD watchdog/reconcile path is a separate
safety responsibility. Never infer broker-flat from a model's terminal input or
blindly retry an uncertain Market exit. Do not change exit deadlines in this work.

Rollback: stop only new shadow workers and then the new source, preserve evidence
and mark affected sessions invalid. No automatic restart of failed generations,
REST fallback or overwrite of shared streams. A later live rollback has its own
flat/reconcile gate. Keep engineering docs public; target configs, credentials,
actual receipts/economics and operational journals stay private.

## Planning Baseline Verification

November 2 checks against this engineering branch: existing gateway router
tests 6 passed, WS subscription builder tests 1 passed, and data-quality tests
4 passed, using `cargo test --locked -p alor-gateway --lib` with the respective
module filters. Local document links and `git diff --check` passed.

These 11 existing tests establish a pre-change baseline only. They do not test
the unimplemented terminal policy, MD-only role, new supervisor integration,
native WS canary or future live suitability. No new WS-soak count has started.
