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

## Blocker: Sparse Forward Context

The current WS adapter sends context-day observations through the same pure
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

1. Keep weekday tradable freshness/late-history/failure limits unchanged.
2. Give eligible non-tradable context observations explicit receipt/cutoff
   provenance and bounded publication semantics. Do not relabel them timely Live.
3. Preserve only actual quotes, deterministic IDs, ordering, uncertain-append
   handling and canonical immutability. Late corrections remain audit events.
4. Reconnect cannot resurrect an old forming buffer or silently insert a quote
   behind the committed prefix. Context arriving after the model cutoff cannot
   rewrite a frozen anchor.
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
