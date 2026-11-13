# Diagnostic Shadow Recovery Boundary

## Dependency Graph

New IMOEXF/USDRUBF reaction pairs use this independent market-data chain:

```text
ALOR market-data WS
  -> alor_shadow_md_ws (MD-only gateway hub and terminal finalizer)
       -> dedicated closed-M10 Redis stream -> reaction_shadow_worker (two arms)
       -> HEALTH/manifest/journal -> reaction_shadow_service supervisor
                                      -> guard/control of the owned worker
```

The source binary reuses gateway WS/history/reconnect components, but is not the
live gateway container or an old paper-shadow gateway container. The worker reads
canonical Redis input; the supervisor guards source health and owns the worker
process. It cannot restart or repair that source.
The RI context worker is a separate contour. None of these read-only paths emits
broker orders.

## Current Operational Distinction

The strict WS supervisor requires split_collect_advance_v1. It can pause model
advancement while a live writer reconnects, re-collect the durable prefix and
continue the same worker after a verified readiness boundary. The worker's
required-bar/queue deadline caps the transport recovery ceiling.

Diagnostic late-start deliberately selects the older poll path:

- ClosedSourceGuard.diagnostic is derived from runner.late_start.
- supervise enables split recovery only for a non-diagnostic WS source binding.
- enable_recovery explicitly rejects diagnostic generations.
- A diagnostic source RX age above its heartbeat budget raises WS_RX_STALE,
  even if the writer process remains alive and can reconnect successfully.
- The supervisor terminates its own worker and retains an incomplete attempt.
  Source recovery alone does not restart that worker or clear its failure.

This distinction remains true on the first clean-eligible day after a diagnostic
start. The passage of a date does not change the launch protocol into strict
recovery. A healthy source process is therefore not evidence of a running model.

## Failure Classification

An observed sequence was source ping timeout, diagnostic worker WS_RX_STALE,
then successful source publication of the next closed M10 bar. The old paper
gateway also reconnected independently in that interval. This identifies the
stopped layer as the diagnostic model/supervisor, not a permanent gateway/source
shutdown or the paper simulator. It does not establish whether the initial WS
timeout originated in the broker, network or host.

Keep distinct alerts for source process/writer heartbeat, transport freshness,
model pause/failure, required-bar completeness and full paired EOD. Docker health
alone cannot establish model continuity. Mark the affected observation interval
incomplete; it does not earn a clean session even if source EOD later succeeds.

## Controlled Follow-Up

This document and its regression test do not enable diagnostic recovery or
authorize service changes. The narrower operational option is a reviewed new
strict generation admitted before opening, using the already-qualified split
recovery path. Its source/worker dates, complete context and admission evidence
must agree; changing the name of a diagnostic grant does not make it strict.
Before extending recovery to the diagnostic protocol instead:

1. Define an explicit versioned diagnostic-recovery capability/admission contract.
   Preserve pinned bootstrap/recovery boundaries, as-observed receipts and the
   exclusion of the diagnostic day. Do not merely remove the diagnostic check.
2. Qualify split collection/advancement during and after late-start bootstrap,
   including a reconnect between collect and advance. While paused there must
   be no new model advancement; recovery must recheck the durable prefix.
3. Keep writer loss, corrupt identity, missing required bars and model/queue/EOD
   deadline expiry fatal. Do not raise the RX budget to conceal disconnects or
   waive age checks under the longer transport ceiling.
4. Test short recovery, flapping, stale writer, deadline exhaustion, STOP in pause,
   replay overlap/deduplication and both model arms' same-day EOD in an owned
   native composed fixture, not only mocked Python tests.
5. Start a reviewed new generation or use an explicitly supported strict grant
   before opening. Current source/worker activation dates must satisfy that
   contract; do not retarget only one manifest. A failed generation is evidence,
   not a candidate for blind systemctl restart or marker deletion.

See [strict recovery](shadow-strict-recovery-work-plan-2026-11-10.md),
[diagnostic launch](shadow-diagnostic-late-start-2026-11-09.md) and
[paired service runbook](reaction-shadow-supervisor-runbook-2026-10-31.md).
Operational paths, timestamps, source-health snapshots and service journals are
kept in private evidence, not the portable engineering publication.
