# Paired Shadow Admission V2

For opt-in source v4 late-context recovery use the separately versioned
[admission v3 contract](shadow-late-context-recovery-2026-11-08.md). V1/v2
plans continue to reject v4; this document's strict existing semantics remain.

Scope: isolated IMOEXF and USDRUBF reaction pairs. No live/legacy deployment,
signal formula, cost, execution route, position size or Rust numerical change.
The existing `reaction_shadow_preflight_v1` / `reaction_shadow_admission_v1`
remain strict and USD-only; existing units and failed evidence are not upgraded.

## As-Observed Inputs

The numerical model consumes the immutable canonical bars actually supplied to
it. A later valid broker restatement is audited separately, never substituted
into a consumed bar, frozen context or historical decision. This applies to
volume too: a VWAP model is not retroactively recalculated from revised volume.
Original receipt clocks, source proofs and canonical IDs remain unchanged.

Separate operational session completeness from agreement with later downloaded
history. A late audited correction is not itself an outage or proof that the
model made a wrong decision. Keep restated-history differences visible; no
blanket claim of numerical invariance or exact restated-history parity follows.
Large/frequent changes still require review. No arbitrary numerical tolerance
or automatic live promotion is introduced.

Plan v2 opts into `as_observed_audit_v1`. The approval record must additionally
contain `revision_review`, exactly equal to the preflight's current snapshot:

```json
{
  "policy": "as_observed_audit_v1",
  "journal_sha256": "REVIEWED_EXACT_FILE_SHA256",
  "rows": 0,
  "price_rows": 0,
  "volume_only_rows": 0,
  "review_dates": []
}
```

Use actual values, not this non-launchable example. The snapshot binds the
entire bounded, newline-complete revision journal, including the empty-journal
case. Every event must reference an unchanged canonical original, the same
symbol/label, valid OHLCV, the correct revision class and a real post-close
receipt. Audit counts/review dates must agree with fresh source health. Source
publication must be drained with no pending write. A changed/torn/unreviewed
journal, missing original or malformed correction blocks admission.

The raw file SHA binds exact Rust-owned bytes. Python checks structural binding;
it does not duplicate Rust proof hashing or authenticate upstream prices anew.
The source writer and its original hash chain remain the canonical authority.
Do not erase `review_dates`, rewrite source bars or clear FAILED to pass a gate.

Approval v2 retains the v1 identity, gates, deadline and exact bootstrap-only
spec rules. It uses `version="reaction_shadow_admission_v2"` and requires three
SHA-bound reviewed evidence kinds: `natural_session`, `overnight_recovery`,
`source_revisions`. True gates are human attestation, not automatic semantic
certification of arbitrary JSON reports. A changed audit during numerical
inspection invalidates the approval and requires a fresh review.

After actual model admission the maintained source continues audit-and-collect
without replaying old decisions. Missing/stale input, source loss, invalid data,
unknown publication, corrupt state and all numerical freshness/EOD guards remain
fail-closed. This admission policy does not relax those runtime guards.

## Symbol And Weekend Boundaries

Plan v2 supports only exact pairs `IMOEXF/imoexf` and `USDRUBF/usdrubf`, each
with its own pinned spec/source/reader/unit. It still requires source v3 and the
source/model budget headroom. IMOEXF remains ordinary120/terminal120/queue30;
USD remains ordinary360/terminal120/queue30 with source ordinary300.

The previous eligible weekday must have the full 07:00..23:40 grid and a real
timely terminal proof, identical in Redis and the source journal. A historical
terminal is not forward evidence. No source or model time is backdated.

For a Monday activation, bootstrap must ALSO include the actual received
weekend-context tail. It must end at the final canonical pre-cutoff source ID,
not blindly at Friday's terminal. The range must cover the preceding complete
weekday from its opening label. No weekend candle is fabricated if none was
observed. The actual Rust `inspect-bootstrap` must accept the exact bounded
range/count/hash/dates, two arms and their anchors/skipped context. An arriving
tail row during inspection blocks admission. This changes neither the frozen
context model nor its weekend completeness rules.

## Controlled Preparation And Launch

1. Use a clean scoped commit and verified source/worker binaries. Stage new
   source stream/state/ACLs and dedicated non-root source/model users. Keep all
   live, legacy, old canary and failed generations intact.
2. Start only the new MD-only sources; prepare static/inactive model units with
   absent approval/spec/env gates and no automatic start/restart/timer.
3. Run Linux fixtures without broker credentials or service network. Run actual
   preparation under each least-privilege reader; verify denied writes with ACL
   DRYRUN, not real write attempts. A context-only inspector preview is not GO.
4. Retain natural terminal/recovery evidence for the new source and the final
   pre-cutoff prefix, including relevant weekend context. Review revisions.
5. On the selected first model morning before07:00 MSK, select/pin the final
   bootstrap and freshly review all evidence. Approval must be no older than one
   hour and leave the full600second startup budget before the explicit ready-by,
   both before and after actual inspection. Example review06:20/ready-by06:55.
6. Only after real admission preflight passes, explicitly start the two workers
   separately. They remain technically unable to emit broker commands. Count
   only complete common model sessions after actual admission, not source days.

Future dates are conditional, not permission to generate future review
timestamps. If a cutoff is missed, preserve that identity and prepare a reviewed
future generation. Do not edit activation inside an admitted source or enable an
unconditional scheduler. Use `KillMode=mixed` for acknowledged supervisor STOP.

Required regression coverage: strict-v1 compatibility, both v2 symbols,
approved/unreviewed/new revisions, volume-only audit, corrupt/bad-binding events,
source pending writes, genuine terminal proof, actual Rust inspector invocation,
Monday context tail and empty weekends, and changes during inspection.

Linux fixture packaging must include both reaction model TOMLs, the USD WS JSON
used by policy tests, the service template and all selected test modules. Pure
policy fixtures mock OS ownership/inspector results; actual reader probes remain
separate. The optional `reaction-shadow-preflight-smoke.Dockerfile` supplies
Python3.11+ and real Redis tools without installing packages on the host. Build
it without secrets, then run as a non-root UID with no network, read-only
artifact/worker mounts and an isolated writable tmpfs. The two actual-worker
smokes own and PID-verify a random-port Redis inside that test namespace only.

## Preparation Verification

Admission code `14c2e1b` and fixture packaging `1d19f91` passed31 preflight and25
supervisor policy tests locally. The two actual-worker integrations were separate
from that local run. Linux then passed all58 tests, including both integrations,
under a non-root UID with no external network and only read-only artifact/worker
mounts. This is focused Python/service coverage, not a new full Rust-workspace run.

Two actual symbol-specific preparation probes used distinct model UIDs, their
source groups and dedicated read-only Redis ACLs. Both passed without model
generation creation, bootstrap pinning or approval. Negative admission with
missing approval was rejected for both. Sources alone were activated; model
units remained static/inactive with no gate files and no automatic launch.

Target checks also confirmed the bootstrap-date binding is enforced before
source-state creation: gateway `from_ts` and source `bootstrap_start` must agree.
Never relax this guard; stage a corrected new identity instead. An actual
numerical bootstrap inspection before completion of the preceding regular day
was correctly rejected as missing history. Preserve that result as a pending
gate, not a numerical PASS or a reason to omit the incomplete weekday.

Final positive admission remains contingent on complete preceding-session and
weekend evidence, actual final Rust inspection and fresh first-morning approval.
The preparation checkpoint does not certify those future inputs or start the
20-session model counter. Existing live/legacy and failed evidence were retained.
