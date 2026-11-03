# Parallel Shadow Preparation and Remaining Gates

Scope: read-only RI closing audit, USDRUBF source templates and owned symbol-
specific smoke, while the reconnect-hardened IMOEXF source-only canary runs.
No live, legacy shadow, active collector source, model parameter or broker route
is changed. Template/preflight completion is not admission or promotion.

## RI Closing Receipt Audit

`scripts/audit_ri_closing_receipts.py` reads an explicit generation directory.
It snapshots committed transaction HEAD, verifies stored payload hashes and
transaction/ingress chain binding, and reports only that immutable prefix.
It never locks/opens the active writer, accesses Redis, repairs a tail, changes
cutoff or recreates original receipt timestamps. Its output must be a new file
outside the generation. Future/uncommitted writer tails are not audited as if
they had been part of the captured HEAD.

```sh
python3 scripts/audit_ri_closing_receipts.py \
  --generation "$RI_GENERATION" \
  --date 2026-10-30 --date 2026-11-02 \
  --output "$NEW_PRIVATE_REPORT"
python3 -m unittest discover -s scripts -p test_audit_ri_closing_receipts.py -v
```

The report joins each 23:40 receipt to BOTH actual recorded arm dispositions,
rather than reimplementing strategy state. Distinguish:

- Timely closing input common to both arms, e.g. ordinary History/Live.
- Actual closing HistoryGap recovery accepted by candidate but rejected by
  control (`RecoveryDisabled`). Only this exercises the declared contrast.
- Missing/late closing input, unobserved committed receipts, a cutoff still in
  the future, and an excluded/non-model date.

Receipt is first observation inside this collector, not gateway publication or
broker receipt. Durability must be strictly before the next eligible model-open
cutoff; equality is late. The report is not independent semantic state replay,
broker-execution parity, a completeness certificate or clean-session credit.
`context_ready` can be true with a 23:30 last label and does not prove 23:40 input.

Keep the current RI collector running on its unchanged source. A future source
with timely terminal Live delivery may make both context arms identical; that
is not a reason to silently move the current comparison or merge observations.
Reassess the experimental identity, preserve controls, and use a separately
admitted generation with genuine warmup/receipt evidence for any migration.

## USDRUBF Source Preparation

Templates: `configs/examples/shadow-md-ws.usdrubf.gateway.toml` and
`shadow-md-ws.usdrubf.json`. Paths/hash/generation are deliberately non-launchable.
They preserve M10, weekday 07:00..23:40 labels, context close 19:00, separate
observation/model activation, explicit exclusions and MD-only sentinel.
Do not choose calendar boundaries by copying another instrument blindly.
Raw source evidence/calendar approval remain separate deployment inputs.

The owned CLI harness now runs IMOEXF and USDRUBF independently. USD history
has sparse weekend context slots (no fabricated candles); its actual worker
inspects bootstrap and arms both H4/W4 comparison branches. Both symbols test
ACL/opcode denial, history-before-ACK, forced owned-WS reconnect with unchanged
XADD/prefix, stop/resume, changed-prefix refusal and uncertain-ACK readback.
This proves symbol-specific transport/reader/bootstrap behavior, not future
weekend-forward correctness or numerical model selection.

## V1 Blocker: Sparse Forward Context

The admitted v1 WS adapter sends context-day observations through the same pure
finalizer as timely tradable Live bars. If no 10:00/10:10 candle exists and the
next real timestamp after 09:50 is 10:20, the earlier candle is already outside
the 90-second publication deadline. This is a legitimate sparse context case,
not permission to invent missing bars or enlarge ALL Live deadlines.

Diagnostic Rust test
`md_source::tests::sparse_context_forward_is_not_certified_by_history_bootstrap`
documents the rejection and absence of a committed watermark. It is a KNOWN
LIMITATION / admission blocker, not a completed fix or passing forward gate.
Historical sparse-context bootstrap passing does not close this blocker.

Before new source/model-pair admission, prepare a narrow context-only path:

Preferred bounded design: opt-in finalization of an already observed NON-TRADABLE
context buffer at its own interval end plus grace, with healthy ACK/generation /
heartbeat and drained-prefix evidence. No buffer means no candle. This avoids
waiting for a next context timestamp that may legitimately be absent, without
increasing regular Live lateness limits. Introduce an explicit context-only
release reason/provenance and test its restoration. This is a proposed next
patch relative to v1, not a behavior present in the admitted artifact.
The separate [v2 implementation](shadow-ws-context-interval-hardening-2026-11-03.md)
now implements this opt-in policy; deployment/real-forward gates remain distinct.

1. Keep weekday tradable freshness/late-history/failure limits unchanged.
2. Give eligible non-tradable context observations explicit receipt/cutoff
   provenance and bounded publication semantics. Do not relabel them timely Live.
3. Preserve only actual quotes, deterministic IDs, ordering, uncertain-append
   handling and canonical immutability. Late corrections remain audit events.
4. Reconnect cannot resurrect an old forming buffer or silently insert a quote
   behind the committed prefix. Context arriving after the model cutoff cannot
   rewrite a frozen anchor.
   Specify bounded actual-history context import separately from active weekday
   Live history: preserve real arrival/origin, ordering and strict next-model
   cutoff. Do not globally bypass the existing active-history rejection.
5. Keep terminal close separate: a healthy clock/WS without the expected real
   terminal candle must not synthesize one or certify a complete session.
6. Test sparse ordinary context, last context label, duplicate/correction,
   reconnect, unknown append outcome, and same-day/next-model-cutoff boundaries.
   Prove unchanged behavior for regular tradable input.
7. Pin/version any changed provenance contract. Use a new source generation;
   never edit a running/failed manifest or reconnect active collectors in place.

Until that gate passes, USD remains PREPARED / NOT DEPLOYED on this WS line.
The isolated IMOEXF weekday canary may continue collecting under its existing
contract, but no weekend-forward or full model-pair acceptance is inferred.
Live gateway changes remain a later separately reviewed integration/flat rollout.
Activation dates are conditional; never force admission to meet a calendar date.

## Verification Checkpoint

Clean preparation commit `e7f475578a285c73b7960e6007a0c653c5f48a5d` passed local
and native Linux scoped checks: 99 gateway library tests, 7 audit tests,
20 supervisor tests and 4 composed symbol-specific smoke cases. The 99 include
the explicit known-limitation diagnostic; that count does NOT close the sparse
forward-context blocker. Local gateway all-target clippy, fmt and diff checks
passed. No whole-workspace or independent numerical parity claim is made.

Native checks used offline Cargo with forced current-crate rebuild, owned fake
WS/RESP and PID-verified random-port Redis in a private network namespace, CPU
50% / memory 2 GiB limits, no credentials or service Redis. Original deployed
container identities and health were unchanged. This preparation changes no
production Rust path; the rebuilt source binary matched the already approved
recovery artifact byte-for-byte. It was not redeployed or rebound to collectors.

The read-only RI report was additionally exercised on an active collector's
captured committed HEAD under a private-network, read-only-system cgroup. Real
receipts/results remain private; no source cursor, writer lock, file or service
was modified by that audit. Future audits can capture a later HEAD without
relabeling earlier observations or supplying synthetic original receipts.
