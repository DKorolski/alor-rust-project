# Strict Shadow Recovery

Status: implementation and qualification in progress. No deployment/admission.
Source baseline f270443; isolated engineering line, not a live gateway release.
Responds to the November 10 external review of terminal/source/model boundaries.

## Scope

Keep ordinary BarEvent as model input, v4 wire and existing source journal bytes.
The next operational generation is strict/pre-opening, not diagnostic late start.
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
