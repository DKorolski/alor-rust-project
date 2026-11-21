# IMOEXF Component Session Bridge

Status: local implementation and controlled fault tests. Not a worker, source
admission, live promotion or permission to deploy. Scope is IMO_AA, IMO_AB and
IMO_AB_OI, quantity one, frozen Author41-short plus existing BO053 reaction
contract. Existing live profiles, old shadow pairs and USD H4/W4 are unchanged.

## Boundaries

`shadow_component_session::Session` processes ONE explicitly dated session and
three matched arms. It borrows the existing owned `RiIngressJournal`; it does
not open Redis, create groups, acknowledge/trim streams, issue HTTP requests,
schedule work or create broker commands. It reuses the generic single-writer
fsync/transaction-HEAD storage without changing its format or existing runners.

`Spec` pins the exact original source identity, reviewed producer-manifest hash,
fixed model hash, date, copied calendar JSON plus independent artifact pin, and
execution freshness budget of at most 300 seconds. Unknown config fields are
rejected. The copied calendar is not reread from a mutable file at restart.
Creation is pre-open; freeze requires the original journal's sealed calendar
cutoff, same source identity, and processing before the first M10 bar closes.
There is no diagnostic late-start mode or automatic next-day/root selection.

The producer hash is a DECLARED reviewed identity, not independent evidence of
WS ACK, exchange completeness, M10 aggregation or producer health. Likewise, a
sealed local journal barrier alone does not prove a drained upstream source.
The future coordinator must supply the separately qualified read-only M10
collector and its source checks. Never substitute matching labels for aggregation
provenance or mint a cutoff from a maximum bar label.

Pin stable producer identity/config/timeframe, not a mutable heartbeat or WS
connection generation. A new WS generation in the approved process may recover
normally after ACK, coverage and published-prefix checks; never encode a fixed
nightly disconnect time. `ingress.generation` is the owned receipt-journal
generation, NOT the WS connection generation. Source/process replacement,
stream recreation or changed committed prefixes require the provider's separate
review/recovery policy, not a silent new journal cursor.

Every public output/status has `order_emission_allowed=false` and
`operational_admission_required=true`. Numerical readiness/completion does not
clear that second flag, and cannot count as prospective operational acceptance.

## Original Receipts And Frozen Context

- Original source IDs, sequence, origin, received/durable clocks and payloads
  remain in the existing durable raw journal. Replay-origin input is rejected.
  Warmup history gets actual collector clocks, not backdated candle clocks.
- Pre-cutoff context uses first accepted receipts, sorted by M10 label. This
  canonical projection has its own contiguous sequence namespace and hash;
  original source IDs and the original journal barrier remain bound in the
  transaction. Projection sequence is NOT the raw journal cursor.
- Identical nightly republication is audited/deduplicated. Changed raw quotes
  are retained in audit without replacing the accepted canonical quote.
- Before freeze, a revision intersecting fields actually consumed by a selected
  component holds the MATCHED three-arm group. Previous-session high/low inputs
  and final close are relevant; previous2 contributes only its final close.
  Volume, open, nonterminal context closes, service labels and changes outside
  selected dependencies stay audit-only. High/low changes are conservatively
  held per selected input label even if the aggregate extremum happens to match.
  Coverage, selected previous/previous2 dates and calendar eligibility are still
  checked independently; daily OHLC equality alone is not an admission rule.
- After freeze, later history revisions cannot alter those anchors/scales or
  rerun a decision. Current-day History remains context/audit-only. Only an
  original, fresh closed Live bar may enter the model; already-observed labels
  never execute twice. Unsupported qualified-history-forward recovery is not
  silently invented by this slice.
- A missing/nonconsecutive, malformed or stale new model bar fails the local
  generation. It cannot consume pending state or publish a partial arm result.
  Unknown/incomplete selected context produces a documented common HOLD, not
  fallback to an older good anchor or a clean empty session.

## OI Once Per Session

The existing qualified OI validator is reused through a reader abstraction; its
archive/schema and deadline/clock rules remain unchanged. The new bounded
`shadow_oi::collector::snapshot::Snapshot` copies exactly the files read by that
validator, then validates the frozen copy again. Bound: eight files, at most
1 MiB per file and 3 MiB total. Unused/unbound files, missing bytes, changed hashes,
wrong plan, offline-fixture evidence or broken clock chains fail verification.
Neither snapshot capture nor recovery makes HTTP requests.

Freeze stores that verified bundle, or a fixed missing/invalid/not-compiled
decision, INSIDE the committed transaction. Restore validates the stored bundle
in memory, never the current external OI directory. Deleting/replacing the
external producer store or delivering a later positive certificate cannot select
a newer vintage or enable BO mid-session. Invalid OI disables only NEW BO in
IMO_AB_OI; MR, observation and exits continue. A corrupted already-committed
bundle fails recovery instead of silently changing an enabled gate to disabled.

The feature `shadow-oi-http` currently contains the qualification reader as well
as the separate capture tool. A consumer build without it records a closed
`oi_feature_not_compiled` decision; adding the feature after freeze cannot reopen
that day's gate. A qualified transaction cannot be restored by an incompatible
feature-off build. This is not permission to run the HTTP collector in a worker.

`oi_input_valid` is separate from `oi_allows_new_bo` and `oi_status`. An accepted
nondecreasing OI snapshot is VALID with BO disabled; missing/corrupt/failed
capture is NOT valid OI input. A complete price grid with invalid OI must not be
counted as a clean OI paired-comparison day. The twenty-session coordinator and
quality counters remain downstream work.

The observed engine gate is opaque and bound to the exact context/evidence hash.
Checkpoint JSON is witness data, not a permission-minting route. The component
engine checkpoint is now v2 and rejects experimental v1 checkpoints explicitly.
No deployed strategy/runtime snapshot uses this isolated checkpoint format.

## Commit And Recovery

One transaction stages all three engines, then writes/fsyncs transaction and
HEAD before exposing any outbox result. An error in one arm cannot advance or
publish either of the other arms. Outputs contain only new events/trades with
stable IDs derived from generation, date, arm, record kind and ordinal. Consumers
deduplicate by ID; never sum cumulative checkpoint trade arrays.

On open, the exact original raw prefixes/barriers and frozen OI decisions are
replayed; context snapshots, output witnesses and engine checkpoints must match.
Only committed outputs can be read. Rehashed wrong witnesses, changed spec,
parallel writers, missing source prefixes, torn/uncommitted tails and failed
generations are refused. Do not delete/reset a HEAD, tail or source cursor to
make recovery pass. There is no automatic truncate/repair/fallback path.

Status exposes readiness, HOLD reason, completion, source observation frontier,
transaction count/bytes, OI quality and failed-generation state. A complete
session requires the entire same-day grid and all engines flat; no missing
terminal bar is replaced by a favorable invented fill.

The inherited journal is bounded by 10,000 records/64 MiB; transaction storage
is bounded by 20,000 transactions/256 MiB, 16 MiB per file. Resource exhaustion
fails closed. No trimming, rotation or periodic disk job was installed.

## Local Dated Driver

`shadow_component_session::coordinator::Driver` now wraps ONE dated session,
its original receipt journal and the existing read-only `RiRedisSource`.
It is an opt-in library API, not an installed worker or continuous scheduler.
Existing workers, sources and their serialized contracts are unchanged.

The separate driver plan pins the source manifest/root, inclusive bootstrap ID,
Linux PID/start ticks/boot identity, reviewed pre-freeze audit hashes, readiness
deadline and bounded collection/queue budgets. The source's original activation
date may precede the explicit model date; that is an explicit source binding,
not permission to replay old model decisions. The older paired-worker v3
admission procedure does NOT yet implement this new binding.

The driver reads the exact pinned v4 M10 manifest and current health, requiring
ACK/subscription/history completion, a drained queue and fresh heartbeat/RX.
Source-config hashing and collection-plus-queue headroom are checked. Linux
process replacement, boot change, manifest mutation and WS generation rollback
fail; reconnect in the same process can wait/recover before required deadlines.
No fixed overnight disconnect time is encoded. Host NTP, filesystem isolation,
writer qualification and operator authorization remain separate admission gates.

Collection and model advancement are distinct calls. Collection uses only
`MULTI`/`EXEC` and `XRANGE`, then compares EVERY accepted source ID and normalized
envelope, not just the resume cursor. Payloads include `source_proof`; publisher,
schema and label-derived IDs are checked. A separately typed local source-prefix
WAL preserves full-envelope hashes across restart, including publication-time
fields absent from the inherited receipt payload. A prefix checksum is an
integrity witness, not a certificate of upstream exchange correctness.

Advancement requires the same checked local prefix, revalidated upstream health
and an unexpired queue budget. No Redis await occurs during model advancement.
Ready-before-open is persisted in `ARMED.json`, bound to the complete driver
plan and original durable receipt prefix. Restart after ready_by but before
opening can restore that witness, but still needs a NEW qualified source poll;
it cannot reuse volatile caught-up status or infer readiness from a directory.
The witness is not a model-admission grant.

The first post-cutoff drained poll seals the original cutoff, then freezes all
three arms once. Ordinary and terminal required-bar deadlines are enforced even
when a healthy Redis returns no new rows. Incomplete next-day recovery fails;
there is no auto-roll, from-zero fallback, late-start replay or invented fill.
A transient Redis transport failure does not itself fabricate a missing row or
poison a healthy generation before its required deadline. Source/clock/content
faults, exhausted deadlines, stale queued prefixes and torn storage fail closed.

This driver deliberately retains EXACT reviewed audit pins BEFORE freeze.
It does not yet classify v4 quarantine/revision journals against model-specific
selection dependencies. New unresolved pre-freeze audit rows require review;
after freeze historical audit growth does not rewrite accepted model inputs.
This conservative temporary boundary is NOT the permanent P1 classifier and
must not be advertised as its acceptance or deployed as an endless daily veto.

Bounds: 2048 accepted source receipts, 128 rows per read, one poll timeout at
most 30 seconds, queue age at most 120 seconds, at most eight 16-row model
transactions per advancement. The source-prefix WAL has a separate 256-record /
32 MiB bound, exposed in status. Existing receipt/session WAL limits also apply.
No upstream trim or local rotation is performed on resource exhaustion.

`clean_price_session` and `clean_oi_comparison_session` are LOCAL numerical quality
indicators. The latter additionally requires valid frozen OI evidence. Neither
increments an accepted VPS-session counter: every status/output still requires
operational admission and forbids broker order emission.

The owned-Redis tests use a PID-verified temporary server and an ACL permitting
only one fixture stream's read operations. They cover full-day EOD, restart and
stable outbox, same-process reconnect, early-payload and envelope-only mutation,
missing bars and durable pre-open readiness. Controlled clocks/process callbacks
are not actual MD-source, NTP or production Linux sandbox qualification.

## Before A VPS Worker

1. Review dated registry, producer manifest, bootstrap source-ID boundary and
   actual M10/source/ACK/reconnect checks. Use the existing read-only polling
   pattern, not REST candles or raw late history relabeled Live.
2. Review and qualify the local dated driver, then add the explicit multi-day
   coordinator and owned generation retention. Resolve pre-freeze quarantined
   audits using the permanent dependency-aware procedure; do not replace review
   with audit deletion or an unrestricted bypass. No skipped session or implicit
   generation rotation is permitted.
3. Qualify actual host clock/filesystem and one separately authorized real ISS
   capture. Test fixtures with kernel-shaped clocks are NOT host/PIT evidence.
4. Run an isolated Linux/read-only-Redis integration with stable outbox recovery
   and source faults, then review configs, budgets, restart policy and admission.
5. Deploy only after separate authorization. Count twenty clean regular sessions
   only on the VPS, separately reporting source/OI exclusions. No automatic GO,
   live parameter change, K/TP/stop retune or USD H3 replacement follows.

Local evidence and current test counts are in the stage-one progress note and
ignored `private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-session/`.
Keep price/capture/checkpoint/outbox artifacts private; this contract is the
portable engineering information for the future integration/review.
