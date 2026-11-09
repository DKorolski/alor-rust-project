# Shadow Review Repair And Durable Grant

Scope: narrow follow-up to the da9047c review, not exploratory/degraded model
execution, live command hardening or a signal/cost/calendar/position-size change.
No rollout is authorized by this document. Historical source/admission identities
and failed evidence must remain intact.

## Remote Connection Termination

WsHub now distinguishes requested Shutdown, event-consumer disappearance,
remote normal Close and stream EOF. Only MarketDataOnly reconnects after the two
remote endings, using existing bounded backoff, fresh connection generation and
subscriptions. Errors retain their existing recovery path. Requested STOP and
receiver/owner termination stop rather than reconnect. Shared Trading normal-close
behavior is intentionally unchanged and requires separate qualification for any
future change; deploying the separate source does not upgrade live gateway.

Coverage includes scope/termination policy tests, owned Rust Close1000/1001 plus
subscription/Shutdown behavior, and composed WS/Redis cases for both symbols:
normal Close1000/1001, transport EOF and existing protocol-reset/revision cases.
The composed cases assert stable canonical Redis contents, no duplicate appends,
explicit stop/resume behavior and no account/order opcodes or command writes.

## Policy Fixtures And Diagnostics

Pure policy fixture reader UID is selected relative to the fixture source owner,
not hard-coded1000. Root rejection and shared-owner rejection are independent
cases, including simulated owner1000/1007 coverage. These mocks do not replace
actual non-root ownership/ACL qualification on Linux. Production guards remain
unchanged. `historical_bootstrap` remains a distinct worker failure reason.

## Opt-In Grant Boundary

Use `configs/examples/reaction-shadow-worker-granted.service` only after review
and substitution of all template tokens. It invokes `reaction_shadow_launch.py`
inside ExecStart; the old prepared template remains available for compatibility
and is not retroactively described as a fenced/granted launch.

1. The launcher accepts only a pinned v3 plan and performs the same fresh strict
   admission, actual Rust inspection, source/audit/terminal and startup-margin checks.
2. Final v3 output contains `admission_grant`: checked time, reader identity,
   model/ready-by dates, exact approval/config/spec/source/worker/supervisor hashes,
   bootstrap and both reviewed audit snapshots. No file is written by preflight.
3. The launcher rechecks pinned inputs, source health/identity and startup margin.
   It fsyncs a `reaction_shadow_supervisor_v3` manifest containing the grant,
   its SHA and launcher/preflight hashes before supervise can spawn the worker.
4. Successful durability of that supervisor manifest is the grant commit. Its
   `checked_at_utc` is the final check sample, not a claim to measure fsync end.
   The persisted event is emitted only after the manifest commit succeeds.
5. The policy is `as_observed_reviewed_snapshot_grant_v1`: valid append-only
   audits after their final successful check, including between that check and
   manifest fsync, do not invalidate earlier immutable inputs or this receipt.
   The contract does not freeze the source writer or promise simultaneous
   snapshots/current audit equality at manifest commit or Popen. Source identity,
   health, durability and model freshness guards remain active throughout.

An audit/tail change observed by the final preflight rereads still rejects it;
those rereads are not a global publication fence. The grant binds the approved
prefix and the actual reviewed snapshot bytes, not a claim that no writer can
append immediately after a read. Late audit observations are never imported
into model history. A stronger until-spawn fence remains a different contract.

Grant/fsync, pinned input, source health or budget failure blocks spawn.
The granted path requires the full startup margin at supervisor entry and again
after the durable started reservation, so slow manifest/reservation fsync cannot
silently consume the approved budget. Existing legacy calls retain their prior
bounded-window behavior unless this flag is explicitly enabled by the launcher.
Operator stop before grant commit does not reserve a model attempt. A stop or
crash after reservation leaves forensic evidence; there is no automatic cleanup,
retry or pretend clean STOP. Worker environment still contains only its dedicated
read-only Redis URL and TZ, never broker tokens or generic Redis credentials.

The new entrypoint deliberately supports initial `create` only. It is not a new
resume/recovery contract; existing units are unchanged. The legacy CLI cannot
silently resume its v3 supervisor manifest as v2. A stopped/failed granted run
requires separate reviewed recovery work, not deletion of control files.

## Controlled Qualification Before Any Rollout

Verify unit/policy tests, owned WS/Redis behavior, source readiness and no
canonical rewrite. Test the exact granted launcher with the actual worker under
the target non-root reader, including source/audit identities, denied command
writes, immutable script/config permissions and timing margin. Mock launch tests
alone are not native target qualification.

Stage new pinned artifacts and reviewed unit/config bindings; do not overwrite
the executable/config of a running immutable identity. Current source sessions,
natural terminal/recovery evidence and morning approval remain governed by the
existing strict contract. If the chosen replacement changes qualifying evidence,
re-evaluate the intended admission date rather than backdating it. Count clean
model sessions only after actual admission, not from source-only collection.

## Explicit Follow-Up

- F4: reproduce delayed Raw after WsRx on terminal boundary, then choose processed
  prefix versus physical socket watermark semantics. No producer barrier added here.
- F6: existing late-context durable timestamp is a pre-fsync sample, not a latency
  measurement; preserve old journal bytes. A new timing schema is separate work.
- Source observation lifecycle versus model first-decision date remains coupled.
  Decoupling and exploratory grant policy need an explicitly new opt-in contract.
- VALID/DEGRADED/RECOVERED_REPLAY segments, unresolved virtual exposure and next
  session recovery are not implemented by extending timeouts or this grant.
- Numerical next-open proxy is not receipt-time executable PnL. Preserve that
  distinction and quality-independent selection of reportable sessions.
- Live execution crash window (broker execute / ACK / XACK, process-local dedup,
  stable broker guid and uncertain outcomes) remains a separate P1 review task,
  not a confirmed duplicate incident or a blocker for isolated numerical shadow.

## Local Verification Checkpoint

- Workspace library tests passed, including112 gateway tests and502 runtime
  tests; opt-in ignored cases are not counted as default passes.
- All3 explicit owned-Redis source runner cases passed serially with their
  unchanged30s deadlines. An initial parallel run alongside the fsync-heavy
  workspace suite timed out; it is retained as a verification limitation,
  not called a successful run or repaired by increasing timeouts.
- All12 composed source WS/Redis/actual-worker cases passed on macOS.
- Python reaction suite passed86 tests including the two owned real-worker/ACL
  integrations. Final pure policy/grant suite passed84 tests on Linux/amd64
  under actual UID1000 and UID1007, read-only artifact mount and no network.
- Workspace all-target clippy with warnings denied, fmt check and diff check passed.

The local checkpoint above preceded the separate native qualification below.
It did not itself qualify Linux artifacts or real ownership isolation.

## Native Linux Qualification Checkpoint

Clean source commit `9f1e393` was exported with a per-file hash manifest and
built offline on Linux/x86_64. No production executable or service definition
was replaced. The bounded build used a private network, one compiler job,
0.5 CPU and a 2 GiB memory limit; Python fixtures used non-root readers, read-only
artifacts, dropped capabilities and an owned loopback network only.

- All 112 gateway tests, 3 owned source-runner cases, 8 service tests and the
  explicitly enabled owned Redis/service integration passed. Runner cases
  remained serial with their original deadlines.
- Python reaction suite passed 86 tests, including two actual-worker/ACL
  integrations. All 12 composed source WS/Redis/actual-worker cases passed.
- Each symbol's separate source/model UID fixture exercised the exact granted
  launcher and Linux worker without mocking filesystem permissions, Redis or
  inspection. Each loaded 202 bootstrap receipts, recorded the grant before worker
  start, stopped cleanly, rejected duplicate create and preserved canonical
  source bytes. Direct Redis writes/consumer-group reads were denied by ACL.
- Both positive launches used an explicitly synthetic calendar and injected
  admission clock, not real forward-source evidence. The unmodified CLI with
  actual wall time rejected those fixtures. This is protocol qualification, not actual
  morning admission or prospective clean-session credit.
- For both current symbol sources, the new read-only preparation ran under
  their actual dedicated reader UIDs with production source mounts and reader
  connections. ACL DRYRUN confirmed read-only permissions; absent actual
  approval blocked admission. Existing model-owner directories remained empty,
  pinned inputs unchanged and source/model services unchanged.

Qualified Linux binary SHA256 values for that source commit:

| Binary | SHA256 |
|---|---|
| alor_shadow_md_ws | b50482da8a4cf2a0896d0d5acb09fa1ddd26b9e28da7fca4999b834283244aa1 |
| reaction_shadow_worker | 74446c54f45705fbc56c1e553634929d6607bb13b09e5a349a3657994b75f02b |

Artifacts were staged separately, not bound into running service identities.
The long SSH transport closed during qualification; the owned process remained
running and wrote its successful native result. No duplicate attempt was
started. Logs and original transport outcome are retained privately.

Remaining gates: controlled replacement/source identity and unit/plan bindings,
actual full-session/terminal/overnight checks and fresh morning approval of the
exact target. The synthetic fixture and preparation-only checks do not remove
those gates. No live strategy, broker command, Redis ACL/retention or actual
model admission was changed by qualification.
