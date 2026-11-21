# Context-Aware Shadow Admission

Status: P0 scoped controller installed and qualified on 15 November;
actual model admission pending 16 November. P1 permanent procedure specified,
not implemented or accepted by the P0 tests.
This changes admission procedure only. Live strategies, terminal finalizer,
BarEvent, source/WAL formats, signal parameters and execution paths are out of scope.

## P0: One Reviewed Start

For the first regular session on 16 November, retain the existing strict v3
preflight and grant launcher. An operational controller may accept only the
operator-confirmed, externally reviewed as-observed snapshot. Without that exact
review, the existing conservative audit veto remains the default.

Bind the exception to the reviewed archive/document, activation date and expiry,
qualified release, model specs, plans, streams/manifests, host boot, source
PID/start, immutable reviewed source prefix, and exact revision/quarantine
journals. Preserve original market data and audit history. No historical
back-insertion, from-zero reset, clock override, late-start fallback, or invented
gates is permitted.

Regular pre-cutoff source appends may extend the reviewed prefix. Re-read the
complete required context, choose anchors again with the actual Rust worker, and
bind the final bootstrap digest on the admission morning; do not freeze an old
Sunday row count. Compare the Redis reader's full claimed range to the durable
canonical records, including contents and ordering, not only first/last IDs.

Repeat process identity checks after preparation, before every model start and
during confirmation. WS reconnect generations within the same process remain
valid after ACK/history coverage/readiness recover; they are not new processes.

Use one confirmation deadline with cleanup reserve before the model cutoff.
Record ownership durably before each start. An external oneshot timeout or kill
must invoke independent cleanup of only claimed new units; a successful STARTED
outcome must retain both workers. Preserve failed attempts; no automatic retry.
Test timeout cleanup with isolated dummy units, never with trading services.

The grant is an as-of boundary, not an atomic lock on the writer. A subsequent
audit is retained and does not rewrite accepted observations. Strictly atomic
writer-to-spawn coordination would be a separate change, not an implied guarantee.

## P1: Permanent Procedure

Nightly disconnect, resubscription and repeated history are ordinary recovery.
Do not encode a fixed disconnect time or require manual intervention merely
because WS generation or the audit journal grew. The current dated one-shot is
not a permanent scheduler; future daily admission requires its own controlled
generation lifecycle and acceptance.

1. Recover source connection, subscription ACK, expected coverage and reader
   cursor before the profile deadline. Verify the already-published prefix by
   content; source-process changes still require reviewed reauthorization.
2. Obtain an explicit context dependency trace from each model arm: selected
   sessions/anchors, previous/previous2, scale, count/first/last, relevant feature
   fields, skipped candidates and the selection frontier. Version it with the
   model/profile/calendar. A journal row being outside the currently selected
   anchors is not enough if it could change candidate eligibility or selection.
3. Classify each new audit against that dependency trace. Irrelevant historical
   restatements remain audit-only and do not block a healthy session. Relevant
   ones require automated checks of completeness, anchor selection and every
   field actually used by the model. Equal daily OHLC is not full equivalence.
4. Preserve as-observed inputs: do not replace accepted bars or invoke the model
   again for a historical correction. A revised-history experiment needs another
   identity and a separate replay comparison.
5. HOLD for source not recovered by deadline, missing mandatory input, mutated
   canonical data, identity mismatch, or unresolved material disagreement in the
   model's dependency context. Report the precise reason and affected arm/session,
   not simply audit_present. Maintain scoped review watermarks and provenance.

The current weekend context contract admits the 09:50 M10 label. Do not silently
reclassify it as service data, or change completeness/calendar while implementing
admission. Likewise, do not add a blanket allow-price-revisions flag or discard
audit history to make a gate pass. Audit/export capacity is a separate robustness
follow-up and must not introduce unsafe data-path rewrites.

## Required Regression

Mandatory end-to-end fixture:

```text
healthy source -> WS disconnect -> generation increase in the same process
-> resubscription ACK -> repeated historical bars / corrected past bars
-> complete immutable published prefix -> context classification
-> automatic morning admission when model dependencies are valid
```

Cover both instruments and both paired arms. Include irrelevant old corrections,
relevant but resolved as-observed revisions, unknown relevant context, incomplete
mandatory bars, changed anchor selection, same-ID content mutation, new PID/boot,
expired review, appends during inspection and deadline exhaustion. Assert no
duplicate model call, historical back-insertion, state rewrite or broker command.

Unit/mock reconnect coverage in P0 is not this full permanent-regime acceptance.
Require a native recovery fixture and then prospective observation for P1.

## Observation And Acceptance

After first admission, verify actual matching source cursors and ordered terminal
consumption in both arms. Require last_complete_session/session_complete,
durable state, no virtual position or pending, and no duplicate processing.
Source terminal publication, model clean EOD, live broker-flat and live promotion
are distinct facts. A passing preflight or healthy process is not a clean session.

Keep the first start narrowly scoped. Implement the permanent dependency-aware
classifier separately with a model-specific review; do not broaden P0 exceptions
to all future sessions or retune strategies under an operational patch.
