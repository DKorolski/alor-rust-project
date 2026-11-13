# Strict Shadow Recovery

Status: code4503b6a local/native qualification PASS. Separate MD-only sources
running; paired numerical models admitted in explicitly authorized diagnostic
late-start mode on November 11. No live deployment or clean-day credit yet.
Source baseline f270443; isolated engineering line, not a live gateway release.
Responds to the November 10 external review of terminal/source/model boundaries.

## Scope

Keep ordinary BarEvent as model input, v4 wire and existing source journal bytes.
The default operational admission is strict/pre-opening. The separately approved
November 11 diagnostic exception is documented below, not a strict grant.
Failed November 9 generations, markers, virtual exposure and evidence are retained.
Live inputs/models/parameters/quantities/action-scoped execution remain untouched.

Implement bounded same-process recovery, a fixed-prefix model admission boundary,
complete-frame terminal cutoff, minimum current-session coverage and safe reasons.
Short/unsupported regular calendars HOLD rather than changing model semantics.
Do not physically migrate proof, add a general intraday resume or weaken freshness.

## Split Ingress And Advancement

The opt-in WS supervisor requires worker capability `split_collect_advance_v1`.
`collect` performs only durable read-side ingress. The supervisor rechecks source
identity/health before `advance`. Advance is synchronous, takes no Redis input and
processes only that collected prefix. A degraded source cannot admit new steps.
If recovery interrupted collection/admission, re-collect from the same durable
cursor to verify overlap and a fresh cutoff poll before re-admission. Original
receipts and queue/model-age checks remain unchanged. No callback from history.

Already admitted synchronous advancement may complete its fixed prefix if the
source changes concurrently. It cannot include newer unadmitted rows. This is
not a claim of atomic transactions across remote WS, Redis and the model process.

## Recovery Bounds

Admission still requires READY. During operation a live writer with degraded WS
enters PAUSED_RECOVERY; no model advancement. Writer heartbeat/identity failure
is fatal, not confused with stale WS RX. One monotonic recovery episode survives
health flapping until a verified progress boundary. The source's 600-second
transport ceiling is capped by the worker's nearest required-M10/EOD/receipt
deadline. No model-age or queue allowance is enlarged to 600 seconds.

Recovery does not restart a worker, clear a failure, resubscribe from Python,
rewrite a cursor or silently import late active-session history. Missing timely
data, changed identity/cursor, unknown publication and deadline expiry stay HOLD /
INCOMPLETE. STOP must remain possible during a recovery pause.

## Terminal And Source Readiness

MD-only text payload and original receipt travel in one internal queue event.
Trading scope retains its old WS-RX/raw event sequence. Terminal proposal follows
all awaited history work and a final drain of complete events with no await
between that boundary and proposal. Late post-cutoff updates remain audit-only.
The existing pending/XADD/readback/commit protocol and v4 evidence stay intact.

Current-session readiness checks mandatory closed labels through the subscription
frontier, not merely ACK and prior-date completeness. Terminal deadlines remain
observable even while history is unready. Full regular sessions and required
warmup are checked; unsupported session overrides fail before model admission.

## Qualification Before Another Real Session

Require source-shaped delayed ordinary fixtures, strict History context-only,
identical Live bars with/without extra metadata, failed enum/clock cases,
reconnect before/during collection, no steps while paused, re-admitted prefix,
cursor change, flapping/deadline exhaustion, source writer failure, STOP in pause,
complete-event queue races, terminal missing, uncertain XADD/readback and full EOD.

Separate component unit passes are not sufficient. Require an owned native
composed source/supervisor/paired-model/Redis scenario with declared fixture clocks,
short recovery, bounded failure, clean STOP and full-day EOD. Synthetic scenario
evidence is not prospective economics or a clean operating session.

Only after that qualification prepare a source/model date-consistent strict grant
for a future cutoff, with capacity/ACL and source readiness rechecked. Existing
strict contracts require source first_live_session to equal model activation;
do not retarget only one date or edit an existing source manifest. A new source
or a separately reviewed distinct-date contract must resolve the binding.

First real day: source terminal count, both model EODs and clean-session count are
separate checks. A healthy day with no reconnect is not recovery PASS. No live
promotion or automatic GO after 20 sessions is included.

## Review Disposition

F1: publication reasons are dispatched explicitly; an ordinary delayed row is
not a terminal exception. F2: split admission and bounded recovery distinguish
transport pause from broken data. F3: shortened sessions HOLD before admission;
general calendar/model support is deferred. F4: terminal drains complete frames
after awaited history writes. F5: broader auxiliary-audit/storage isolation is a
follow-up; this patch does not claim that storage failures are recoverable.
F6: current-session closed-label coverage is required, not just ACK/READY.
F7: supervisor failures expose safe classes and retain worker digests.

Do not use daily clean restarts to hide new failures. Qualify combinations in
the composed bench first, preserve failed generations, and classify each real
failure against its original receipt/deadline. Recovery must not turn missing
history into a timely prospective event or reset the observation-day counter.

## Local Qualification Checkpoint

Local library suites: gateway112 passed/5 explicit opt-in ignored; runtime507
passed/16 opt-in ignored. Python policy/process104 passed. All6 owned composed
scenarios passed: both instruments' complete synthetic session/paired EOD,
same-process recovery, STOP in pause and unextended missing-bar deadlines.
USD includes the source-shaped95-second ordinary delay. The fixture also forces
opening between collect/advance, verifying a new post-cutoff read rather than
backdating a barrier. Test clocks exist only in ignored test executables.

An additional owned Redis test injected a complete frame while canonical history
append was awaiting an actual paused Redis writer. The queued updated quote was
used for the single terminal publication. It is not a production observation.

All-target clippy for gateway/runtime with warnings denied, fmt and diff checks
passed. Native Linux qualification and any actual future admission are still
separate gates; this checkpoint does not approve deployment. Initial fixture
development failures were corrected before this result: bootstrap must be pinned,
USD's360-second budget must not be tested as120, and bridge STOP must reap cleanly.

## Initial Native Checkpoint (Historical)

Clean implementation `818606e` was exported from Git objects with per-file hashes.
Offline Linux builds completed for both library test executables and production
`alor_shadow_md_ws` / `reaction_shadow_worker`. All26 terminal unit tests passed.

The first owned source-pipeline invocation failed before exercising the pipeline:
the qualification helper selected a host Redis executable path that does not
exist. Its log is retained. No host packages or working Redis are changed to fix
the fixture. A separate follow-up helper runs those tests, policy/process tests,
composed cases and production-protocol probes inside the existing isolated Redis
fixture image. Full native qualification is not PASS until those results and
the before/after protected-service comparison are available. New SSH connections
subsequently timed out during banner exchange, delaying that follow-up.

Do not infer a trading-service failure from this SSH symptom, or deployment GO
from successful compilation. The patch has not replaced any active binary or
source/model generation. Keep the previous review archive and source evidence.

## November 10 Continuation

On a compatible Ubuntu24/glibc2.39 isolated fixture, terminal26, source pipeline4,
recovery regression5, Python policy104 and composed6 passed. One of17 production
protocol smokes exposed a real diagnostic-only cutoff race. The source read was
fresh at collection, but bootstrap/ingest could cross a wall-clock second before
the diagnostic as-of freeze. That freeze correctly required a newer poll and
previously poisoned the generation. The failure reproduced5/8 times; original
markers and logs are retained. It is not a synthetic model or exchange failure.

The retained collector digest `aac9804489bdef34e2f042d509f9d099986b43f05ff48a188c4b3748133efd43`
matches `cutoff requires a fresh source poll`; nested worker digest is
`f75c3e24e38015aa79c5015f15092299cdd3d8e1555f4749f87599e7dd6f7cf4`.

The follow-up keeps bootstrap/ingest durable, returns unarmed before diagnostic
freeze, and re-polls the unchanged source prefix. Only a genuinely fresh read
can freeze/arm. Original receipts, deadline and clean-session eligibility do not
change. Strict pre-opening and RI logic are unchanged; diagnostic admission is
still not planned. A PID-verified owned-Redis regression for both symbols was
RED on the old code and PASS after this fix.

Follow-up code commit: `4503b6a`. Full local requalification PASS: gateway112,
runtime507/17 opt-in ignored, Python policy104, composed6, queued-frame race1
and the new owned-Redis cutoff regression1. All-target clippy with warnings
denied, fmt and diff checks also passed. The native rebuild must use the clean
Git-object export of4503b6a and a compatible Ubuntu24/glibc2.39 fixture, include
the sixth recovery regression and8 repeated IMO production-protocol cases.

The next explicit admission remains a separate operation. Start before opening,
with matching future source/model activation dates, a pinned fresh bootstrap,
read-only model ACL and a full readiness window. An inactive artifact may be
staged only after native PASS; it is not a grant or an enabled service. No
current live/legacy source, failed generation or marker is replaced by this work.

## Final Native Checkpoint

Clean code4503b6a passed171 native Linux checks, including8 repeated executions:
terminal26, source pipeline4, recovery/diagnostic6, Python policy104, composed6,
production protocol17 and repeated IMO diagnostic protocol8/8. No skipped Python
tests. Both production executables were rebuilt offline in a bounded sandbox;
test execution used a compatible Ubuntu24/glibc2.39 image, non-root UID, owned
Redis and no external network. The before/after comparison confirmed unchanged
pre-existing container identities and protected service PIDs/restart counters.

The inactive release pins executable/script hashes and qualification evidence.
It is not a live gateway image, model admission or a completed natural session.
Original failed fixtures/generations and the prior review archive are retained.

## MD-Only Preparation

The operator selected November11 with morning review at06:20 MSK. Two separate
MD-only sources started November10 after native PASS, without changing existing
live/legacy services. Source/model activation is November11; the distinct source
terminal observation date is November10. Today's records remain History/context
only. The new sources must collect their own genuine previous-session terminal;
an imported historical terminal alone cannot satisfy strict admission.

Both sources reached READY with two complete101-bar regular context sessions.
Separate OS identities and exact-stream writer/read-only Redis ACL are used;
broker refresh tokens are not copied. Reader-role checks confirmed no source
write permission or access to MD environment/gateway files, and a supported full
session calendar. Prepared model units remain inactive, with no approval files,
bootstrap pin, model generation or supervisor attempt. Admission without approval
was explicitly rejected. An operator helper initially read an absent `reason`
field in that negative response; it was corrected to verify the exact safe error
digest. The healthy source was not restarted and no model state was created.

Next review is manual, not a scheduled/autonomous GO: verify natural terminal,
overnight continuity/recovery evidence, current source readiness, revisions,
context tail, read ACL and capacity; then pin fresh bootstrap and grant only
inside the full pre-opening window. Do not clear failures or use late-start
fallback if any gate is missing. Source terminal, paired EOD and clean-session
credit remain separate facts; twenty clean sessions are not automatic promotion.

## Authorized Diagnostic Admission

On November 11 the strict pre-opening window expired without a model grant.
The operator then explicitly authorized a same-day start with relaxed startup
conditions. This is a distinct diagnostic admission, not an automatic fallback
or a rewritten strict approval. The already qualified diagnostic late launcher
and worker from4503b6a were used without modifying model or supervisor code.

The exception covers pre07 startup and strict pre-opening recovery qualification
for the diagnostic day only. Evidence states that no controlled recovery smoke
was performed; READY plus an increased WS generation is not relabeled as that
test. Actual previous-session terminal evidence is retained and checked.

Unchanged requirements include complete context, a pinned canonical recovery
prefix, genuine receipts, source readiness/identity, reviewed revisions, separate
non-root read-only model identities and technical prohibition of broker orders.
Startup600seconds, queue30seconds, ordinary model lateness120seconds for IMO and
360seconds for USD, and terminal120seconds are unchanged. Existing strict model
plans, source manifests, failed attempts, live and legacy shadow units remain
untouched. New configurations, owners, receipts and grants have distinct paths.

Both numerical weekday/weekend pairs armed with pair_ready=true, loaded202
historical context rows, caught up to their source and reported no failure.
November11 is diagnostic as-of recovery, not a full prospective day: reconstructed
events must be separated from newly observed events in economics/decision audit.
The first possible clean session is November12, conditional on uninterrupted
valid coverage, both model EODs and all normal data/deadline checks. The active
process can continue into that session; do not recreate it daily or clear its
state. A failed generation still requires retained evidence and explicit review.

The launch before/after check confirmed unchanged protected container identities,
live/legacy service PIDs and source PIDs. Source terminal publication, paired EOD
and clean-session count still require separate observation at the actual close.

Post-start verification observed the next genuine Live M10 bar in both paired
model states, not only an advancing source cursor. Startup recovery was marked
diagnostic_replayed and the new Live event model_observed; these are distinct
from diagnostic_forward_observed for eligible History-origin inputs. Both pairs
remained armed/caught-up without failure or restart, with no broker environment
and Redis-only network access. Today's clean count remained zero.

## Reusing Qualified Admission

After the November12 provider-maintenance pause, the operator explicitly allowed
same-day numerical shadow admission without waiting for another night or another
terminal observation by each new source generation. Existing successful native
qualification and genuine prior-session terminal evidence are reusable engineering
evidence, not mandatory daily tests. Record their exact provenance: a terminal
witness from a prior generation must not be presented as a forward publication
by the current generation. Current source health, canonical prefix and actual
post-admission observations still require independent checks.

Use the already qualified diagnostic late-start launcher for the incomplete day,
with a new immutable spec/grant/generation and explicit authorization. Do not
rewrite an existing source manifest to relabel History as Live, or call the
reconstructed morning a prospective observation. A future source activation date
may match the first clean session, as supported by the diagnostic contract;
proof-backed forward History is audited separately from bootstrap recovery.

A successfully running diagnostic pair continues into its first full eligible
session without a fresh morning grant, daily restart or another overnight wait.
Morning inspection is monitoring, not recurring manual admission. Keep normal
source readiness, isolation, receipt provenance, queue/required-bar deadlines,
terminal handling and durable failure behavior. This authorization is not a
general bypass for stale/missing data, failed generations or live-order promotion.
