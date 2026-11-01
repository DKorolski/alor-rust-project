# Shadow Observation and Safe Live Promotion

This is the engineering handoff for the isolated RI, IMOEXF and USDRUBF shadow
line. It preserves model, data and execution boundaries for a possible future
live implementation. It is not live approval, a trading recommendation or a
copy of private soak results. No live runtime or gateway is changed by publishing
this code. All deployment decisions require a separate reviewed release.

## Observation and Retirement Policy

Keep the 14 legacy shadows running during the new observation window. Their
documentary status is `RUNNING / RETIREMENT_PENDING_REVIEW`. Do not change service
labels by recreating containers merely to apply this status.

| Family | Deferred retirement candidates | Temporary bridge |
| --- | --- | --- |
| RI | legacy09, BO max2, BO last-entry17 | prospective shadow07 |
| USDRUBF | native VWAP09, MR1040/BO2, MR1140/BO2 | native VWAP07 |
| IMOEXF | High18007/09, weekend legacy09-Author41, compromise MR1059/BO4, BO-only weekend09 | weekend live07-phase |

These are 11 candidates plus 3 bridges, not an approved deletion list. Old and
new models are not necessarily execution-equivalent controls. Review after at
least **20 common clean prospective sessions per relevant comparison pair**.
Both arms must have valid frozen context, complete input/decision coverage,
resolved EOD position/pending state and intact recovery evidence. A process
uptime day, history replay, warmup date or invalid half-pair does not count.
Different pairs can start on different dates; do not combine their counts.

Twenty sessions permit a review, not automatic GO. Extend the sample for sparse
trades, too few affected weekends, unresolved drift or unstable resources.
The reaction supervisor's `max_sessions=20` is a bounded generation limit,
not a clean-session score. Respect source/storage limits and explicit admission
when another generation is needed; never reset a failed run to make it clean.

Before any later retirement: approve the exact services, archive bounded final
reports plus config/image/state/cursor hashes, verify archive readback and
virtual flat/no pending, then disable only those services' autostart. Keep shared
source gateways and Redis until all dependent readers are accounted for. Deleting
obsolete state/groups/volumes is a separate approval after retention review.
Never erase open virtual exposure or drop a High180 ledger to manufacture flat.

## Model Identity and Current Scope

Three isolated paired processes provide six comparison arms. Two additional
market-data source processes serve the reaction pairs; they are not strategies.
Live configurations, signal parameters and order sizes remain unchanged.

| Pair | Frozen model and policy | Explicit exclusions |
| --- | --- | --- |
| IMOEXF | Author41 short MR .16/.020/.58, previous-LOW range normalization; MR-first combo with Hybrid BO K .53; weekday vs weekend context | Not High180, not previous-close Author41/bracket semantics, not Author42 BO |
| USDRUBF | Previous-day07 H4/W4; short MR .035/.16/.43; BO .45/.51/.35; weekday vs weekend context | Not native intraday-VWAP/bracket live execution |
| RI | canonical07, `micro_regular_v1`; runtime-accepted vs confirmed-closing context; MR K .20/.11, BO K .42; weekday anchors, Friday BO off | No weekend anchors, no inherited BO max2/last-entry17 filter |

RI MR keeps its shared maximum of two entries across both sides. IMOEXF MR
also permits at most two accepted entries per session. USDRUBF must NOT inherit
that quota: its reaction reference has no such MR entry limit. Preserve each
engine's MR-first/no-overlap ownership and BO observation while MR is busy.

Reaction exits are closed-bar decisions with next-open model proxies, not
broker limit/stop brackets. Distinguish the bar-BEGIN label, decision-ready time,
logical action time, proxy-known time, actual receipt and actual broker fill.
Historical proxy price is not a price the live adapter is guaranteed to obtain.

IMOEXF MR signal labels through 10:00 are inclusive, last entry action 10:10;
MR time-exit label 20:00 means action 20:10, not a 10:00 forced flatten.
USDRUBF H4/W4 MR forced action is 11:00, first BO action 11:10. RI retains its
configured 10:00 MR entry cutoff, 20:00 MR time exit and 23:00 BO exit contract;
do not infer forced MR close from the end of its entry window. IMOEXF/USD BO
reaction EOD is decision label 23:30 and next-open action 23:40. Source receipt
of the complete 23:40 candle happens after 23:50; it must not delay a live
23:40 action merely to learn a retrospective fill proxy.

Additional USDRUBF H3/W4, H3/W2 attribution, 09/VWAP reaction and standalone
IMOEXF controls exist in numerical tooling but are NOT all active services in
the initial paired deployment. Add each only with an explicit identity/scope.
Do not use High180's lb120 seed/ledger in the Author41 reaction experiment.

## Code and Configuration Map

| Layer | Code / example |
| --- | --- |
| IMOEXF numerical engine and trace | `strategy-runtime/src/bin/imoexf_shadow_replay/` |
| USD numerical engine and trace | `strategy-runtime/src/bin/usdrubf_shadow_replay/` |
| Paired context/cutoff | `strategy-runtime/src/shadow_session_context.rs` and submodules |
| Continuous reaction state/outbox | `strategy-runtime/src/shadow_reaction.rs`, context collector/runner |
| Reaction worker and supervisor | `src/bin/reaction_shadow_worker.rs`, `scripts/reaction_shadow_service.py` |
| Closed M10 REST source | `scripts/shadow_closed_bar_source.py` |
| RI collector and frozen strategy facade | `src/ri_context/`, `src/bin/ri_context_shadow.rs`, `src/strategies/ri_author41_42_live/frozen_shadow.rs` under `strategy-runtime/` |
| Reference/state comparators | `scripts/shadow_state_parity/` |
| Approved-shape templates, not deploy defaults | `configs/examples/reaction-shadow.*.toml`, `configs/examples/ri-context-shadow.toml` |

Pin reference bundle/manifest, source/config/schema, calendar, instrument and
binary hashes per generation. Research bundles and generated state traces are
private inputs, not distributed deployment artifacts. Never build live images
wholesale from this standalone-shadow publication: unrelated lifecycle fixes
may live in a separately reviewed runtime release line.

## Data and Restart Contract

Use closed M10 as the canonical accounting layer. Both comparison arms consume
the same price source and actual receipt sequence, with independent model state,
positions, quotas and journals. A weekend-context arm may use eligible weekend
bars as anchors, never generate weekend orders or spend entry quotas.

Freeze prior/previous2 context at model open, not at first received bar: 07:00
Europe/Moscow for these pairs. Only durably received data strictly before cutoff
can qualify. A correction at/after cutoff cannot rewrite the day's context even
if the first decision is delayed. Preserve raw OHLCV, selected session dates,
completeness, source identity, receipt times and context hash across restart.
Do not reconstruct missing original receipt evidence from download time.

RI uses its transition-aware regular-session completeness guards (pre-transition
09:00 vs canonical07), same-contract history and weekday-only anchors. The
reaction weekend extension uses its separately versioned >=44 of 55 labels
09:50..18:50, first <=10:00 and last >=18:50 acceptance. That 09:50 context bar
is not a tradable model bar. Changing this guard/calendar is a separate model
version, not a silent market-data cleanup.

Reaction bootstrap is an explicit one-time import of 2-4 complete eligible
sessions, typically three, with relevant intervening context. It seeds context
only, no historical decisions. Pin inspected source-ID bounds/row count/hash;
resume uses the durable cursor and frozen state, not a fresh history overwrite.
RI's initial collector uses prospective context and has its own readiness gate.

Missing/late/conflicting data, source revisions, torn state and incomplete
supervisor attempts fail visibly. Preserve FAILED evidence; no auto-clear,
silent cursor skip, in-place profile switch or synthetic flat. Live broker
reconciliation is not implemented by these shadow fail-stop mechanisms.

## Last Candle and EOD Responsibilities

The existing gateway router buffers a Live bar until a later timestamp arrives.
Without a next timestamp, the terminal candle may appear only next morning as
History/HistoryGap. The isolated shadow fix uses an authenticated REST source
and new streams, without changing shared gateway or live inputs. See the
[source contract](shadow-closed-m10-source-2026-11-01.md).

These are separate engineering responsibilities:

1. **Data finalization:** publish a genuinely closed terminal bar, with real
   receipt provenance, without requiring a next-session tick. Do not relabel old
   history as timely Live or finalize an unconfirmed buffered candle by clock alone.
2. **Live position safety:** honor the approved exit deadline and prove broker-flat,
   even when the market-data source is delayed or missing. A timer/risk exit is
   an explicit safety overlay, not a fabricated model candle/fill or parity PASS.

Before promotion choose and document the data solution independently of model
selection: keep the isolated confirmed source, or implement a reviewed gateway
terminal-bar finalizer. A gateway fix needs per-instrument/session-calendar
boundaries, authoritative close confirmation, duplicate/conflict handling,
durable ordering, reconnect/startup recovery and compatibility tests for every
consumer of the changed stream. Shared stream replacement is NOT config-only.

Gateway finalization does not remove the need for a live EOD watchdog. Runtime
timer/event-loop work needs persistent exit lineage, pending-ACK reconciliation,
reentrancy/dedup, stale-feed/clock handling and broker checks. Never wait until
23:50 confirmation of the final M10 candle to start an earlier required exit.
Never blindly resubmit an uncertain Market ACK: reconcile orders/fills/position
first to avoid a duplicate exit or reversal. A gateway scheduler reject remains
a safety net, not the preferred normal deferred-exit path.

Required fixtures: no next tick after terminal label; exact session close;
weekend/holiday/short session; missing and revised terminal bar; clock skew;
duplicate/out-of-order REST/WS delivery; restart across close/cutoff; late history
after a frozen anchor; an open position during feed loss; partial fill and unknown
ACK during EOD. Assert no fake bars, no historical broker intents, no duplicate
exits, no new entry after terminal cutoff and no unexplained overnight exposure.

## Promotion Work Packages

1. Select a model and context policy after observation review. Freeze the full
   feed/calendar/execution/cost identity; keep source drift separate from model
   and broker-execution drift. No winner selection from a single week.
2. Re-run independent Rust/Python state parity, not only trades/PnL. Compare
   anchors/previous2, scales, quotas, BO observations, pending/position owner,
   suppressions and exact decision keys. Preserve signal vs execution drift classes.
3. Design a narrow live adapter/profile integrated with the maintained host.
   The worker currently has no broker route. Reuse runtime-owned final request IDs,
   pre-emit window deferral, residual reject handling, action-scoped fresh control
   transport, symbol-scoped broker truth and partial-fill/cancel-race safeguards.
   Preserve MR/BO attribution and observation while another component owns risk.
4. Do not silently copy legacy brackets into a reaction model. Adding TP-limit
   or stop orders changes the execution contract and needs a separate replay,
   rounding/tick/cost comparison and bracket lifecycle review. Entry-filled vs
   requested quantity and in-flight protection/exit ownership must be explicit.
5. Resolve terminal-bar delivery and live EOD safety as above. Prove readiness,
   monitoring, resource bounds, restart and operator recovery on the target host.
6. Build a clean scoped artifact with pinned provenance. Run unit/config/parity,
   lifecycle/failure tests, fmt/clippy and owned integration probes without live
   Redis/order credentials. Review before any controlled micro rollout.
7. Roll only in an approved maintenance window with broker positions flat and
   no working/stop orders. Stop the previous writer before enabling the new one.
   Isolate new model state/group/generation; preserve raw evidence and any
   unrelated riskgate ledger. Size, account and rollout require explicit GO.
8. Reconcile startup against broker truth, verify one writer and action-scoped
   routing, then monitor entries, partial fills, exits and EOD for a separately
   agreed micro validation window. No simultaneous size increase or model retune.

Rollback is also flat/order-clean gated. Stop new emission, reconcile unresolved
ACK/fills, preserve the failed generation and use the reviewed last-good artifact.
Do not restore an old non-flat snapshot or start a second account/symbol writer.

## Publication and Ownership

Publish engineering source, tests, templates and these operating contracts.
Keep raw broker exports, actual receipts/trades, economics/equity, account dumps,
host-specific deployment files, credentials and retrospective journals private.
Ignore rules do not remove already-tracked files or private commit ancestors.
Use a clean publication branch based on the current remote tip, with an explicit
file allowlist and no private-history merge. Existing public history is not rewritten.

Retain model/reference manifests and run evidence in access-controlled storage
so a future engineer can reproduce acceptance. An engineering release needs its
own CI/test results; a private historical report is not a deployable artifact.
Source/worker services are deliberately bounded and not an unattended live
platform. Consult the [service runbook](reaction-shadow-supervisor-runbook-2026-10-31.md)
and [parity tools](../scripts/shadow_state_parity/README.md) before starting them.

The repository's main-branch CI can publish gateway/runtime images to GHCR after
tests. Publication is not deployment or live promotion: no VPS restart step is
included. Keep approved deployments pinned to reviewed image tags/digests; do not
auto-pull `latest` as a substitute for the flat/order-clean rollout gates above.
