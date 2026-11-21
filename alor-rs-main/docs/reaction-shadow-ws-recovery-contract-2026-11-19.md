# Reaction Shadow WS Recovery Contract

## Scope

Both paired reaction models use the same WS recovery lifecycle, including a
generation initially created through the explicitly authorized late-start path.
This changes supervision only. Numerical models, costs, source publication,
terminal finalization, BarEvent and live execution remain unchanged.

`late_start` describes output quality and the initial recovery prefix. It must
not permanently opt a process out of transport recovery after the first clean
session.

## Lifecycle

1. Admission remains fail-closed: require the pinned source identity, active
   writer, fresh writer heartbeat, WS readiness, subscription and history ACK.
2. Require `split_collect_advance_v1` from every WS-backed worker before enabling
   recovery. An old worker cannot silently fall back to combined polling.
3. On temporary disconnect or stale WS RX, enter `PAUSED_RECOVERY`; keep the
   existing worker and its committed model state, and do not advance the model.
4. After source recovery, collect/revalidate the durable cursor before admitting
   the fixed receipt prefix. `advance` must not add receipts or perform a Redis
   read. A changed prefix remains fatal.
5. Emit `reaction_shadow_recovery_resumed` only after admitted model progress.
   A reconnect generation increase is allowed; rollback is not.

The 45-second WS RX budget remains a readiness threshold, not a reason to kill an
already admitted process immediately. The 15-second writer heartbeat remains a
hard integrity/liveness check. Recovery has the existing 600-second transport
ceiling, capped by the worker's earlier required-bar/queue deadline. Repeated
flapping does not reset the ceiling before confirmed progress.

Source manifest/identity changes, missing writer, stale writer heartbeat,
generation rollback, failed source and required-bar deadlines still stop the
consumer. This patch does not waive model freshness, terminal deadlines, audit
review or the absolute prohibition on broker commands.

## Explicit Cutover After A Failed Attempt

Never forge a clean STOP, rewrite a manifest/grant, delete the failed generation,
or backdate receipts to make an interrupted day appear prospective. Existing
resume rules still require a clean operator STOP, complete paired EOD, identical
identity and a valid pre-open deadline.

If those conditions cannot be met intraday, an operator may authorize a new
diagnostic generation using the unchanged canonical source and model config.
Recompute and review bootstrap/recovery digests and selected contexts, qualify
both nonroot readers, and confirm both new pairs before retiring a healthy
predecessor. Retain the old generation and the completed-session evidence.

For the explicitly approved 2026-11-19 cutover, 2026-11-18 remains one completed
clean session per pair; 2026-11-19 is diagnostic and does not count. New generations
start their own clean counters at zero on 2026-11-20 with 19 sessions remaining.
Combine this lineage explicitly in observation reports, never by rewriting the
new worker's counter. Runtime reset is not Redis reset.

## Acceptance

Regression coverage must include diagnostic IMOEXF and USDRUBF reconnect past the
WS RX threshold, subscription/history readiness before resume, one worker/model
advance only, no prefix change, unchanged required-bar deadline, stale writer
failure and rejection of workers without split capability. Existing strict
admission, incomplete-attempt and no-order tests must continue to pass.

Controlled tests are not prospective soak evidence. After deployment, observe a
real reconnect and paired terminal/EOD before claiming the recovery path has
passed an overnight soak.
