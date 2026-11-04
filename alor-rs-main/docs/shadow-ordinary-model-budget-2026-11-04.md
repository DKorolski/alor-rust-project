# Shadow Ordinary-Bar Processing Budget

Follow-up to [source v3 collection](shadow-ws-ordinary-delay-hardening-2026-11-04.md).
Scope: opt-in reaction shadow service only. No live gateway/runtime, RI collector,
CWS route, signal formula, costs, order emission or terminal source policy changes.

## Separate Limits

The USD launch template now declares:

| Check | Seconds | Scope |
| --- | ---: | --- |
| Ordinary source collection | 300 | Genuine greater-timestamp observation only |
| Ordinary model processing | 360 | Close to processing, including source/delivery/queue |
| Model receipt queue | 30 | Original collector receipt to processing |
| Terminal source publication | 90 | Existing terminal proof; unchanged |
| Terminal model processing | 120 | Also retains existing startup/cutoff deadlines |

The new optional `max_ordinary_model_lateness_seconds` is bounded to 360 and
cannot be less than `max_model_lateness_seconds`. When absent, all old model
limits and serialized policy shape remain unchanged. IMOEXF keeps its existing
120-second model policy; no blanket increase across strategies is implied.

The 360-second value is a controlled shadow allowance, not proof that every
future bar arrives in time or that delayed reactions are equivalent to live
execution. The 60-second difference from collection allows queue/delivery margin;
the independent 30-second queue guard is not relaxed. Both the pre-advance age
check and the expected ordinary-bar deadline use the new allowance. The 23:40
terminal label, EOD missing-bar deadline, readiness, first/next cutoff and
incomplete-session resume checks retain the original model limit.

Health reports ordinary/terminal ages and effective limits separately, retaining
the aggregate model age for compatibility. Actual receipts and publication
timestamps are never backdated; late rows remain identifiable source evidence.
Changing this allowance cannot repair past failed or incomplete sessions.
Source health's `consumer_owned_unchanged` means the source does not waive the
consumer's configured checks; the separately pinned worker policy, not that
source-health string, supplies its effective numerical age limits.

For a v3 source the supervisor rejects admission unless ordinary model budget
covers source collection plus queue budget, and terminal model budget covers
terminal publication plus queue budget. Existing v1/v2 guard contracts remain
unchanged. The raw launch spec, worker binary and source manifest remain pinned
by the supervisor's immutable generation/attempt binding.

## Checks

Required coverage: legacy absent-field serialization/defaults, invalid limits,
125/291/300-second ordinary delays, 330 seconds including queue 30, 360/361
boundary, queue 31 rejection, terminal 120/121 boundary, missing ordinary and
missing EOD deadlines, and complete paired EOD with delayed ordinary input.
Run the owned PID-verified random-port Redis service integration explicitly;
never run arbitrary ignored tests against a default Redis endpoint.

Repeat source gateway tests, both owned source pipelines, actual worker and
supervisor tests, symbol-specific fake WS/Redis CLI smoke and native Linux checks
from a clean pinned artifact. The native smoke must use the rebuilt worker, not
an older installed binary which cannot parse the new optional policy.

## Staged Rollout

1. Install only a new USD source generation after the native gate. Retain failed
   USD v2, working IMOEXF v1/v2, the existing RI comparison and all live services.
2. Keep new stream, source writer, reader ACL, state directory and launch spec
   separate. Reader cannot write; source cannot access order/live namespaces.
3. Prepare the worker spec with 360/120/30 but do not start a numerical worker
   solely because source health says ready. Source-only readiness is not model GO.
4. Obtain a natural complete collection/terminal cycle and recovery evidence;
   review price revisions, actual prefix and warmup/cutoff provenance. Select an
   explicit future first model session with enough time to arm before cutoff.
5. Admit the pair separately, then count only complete eligible clean sessions.
   Never auto-resume intraday, reinterpret a partial collection day as a model
   session, or substitute historical prices for original prospective receipts.

Activation dates in examples are placeholders, not schedule approval. Existing
experimental holiday exclusions stay explicit and separate from exchange hours.
If the selected date cannot pass admission in time, do not edit an existing
manifest: preserve it and review a new generation/date instead.

## Local Checkpoint

502 runtime library tests passed, with 14 intentionally ignored integrations/
exports; the owned-Redis service integration was separately invoked and passed.
It exercises a full paired EOD with delayed ordinary inputs, the missing ordinary
360/361 boundary and the unchanged missing terminal 120/121 boundary. Gateway
library: 107 passed, two owned source pipelines intentionally excluded from that
default suite (run explicitly in native verification). Supervisor: 25 passed;
closing audit: seven passed; composed actual-worker WS/Redis CLI: four passed.
Runtime all-target clippy with warnings denied, fmt and diff checks passed.
Native artifact verification and natural operational admission are separate gates.

## Native Checkpoint

Clean artifact `67ef210913a9518277814f00e0401ef52d8febc6` passed on Linux:
107 gateway library tests, both separately invoked owned source pipelines,
eight service unit tests, the full owned-Redis service integration, seven audit,
25 supervisor and four composed WS/Redis CLI smoke cases. The CLI cases used
the newly rebuilt worker. This native scope is not a rerun of all 502 runtime
library tests; those passed locally.

The probe used a private network, owned PID-verified Redis, CPU 50%, memory 2 GiB,
no broker credentials or service Redis. It completed in about 710 seconds and
left existing Docker identity/start/restart/health metadata unchanged.
Source SHA256:
`5a97e11fc9227e2c16802f7b595b60d526957952364004500cbe5b6809d8ee67`.
Rebuilt worker SHA256:
`74446c54f45705fbc56c1e553634929d6607bb13b09e5a349a3657994b75f02b`.

A fresh USD-only source canary was installed after that gate. Its captured
bootstrap prefix matched Redis objects with unique IDs; source health was ready,
with no pending publication or automatic restart. Old sources, failed evidence,
RI and live deployments were preserved. The model spec is prepared, NOT admitted.
Bootstrap/source readiness is not a natural terminal/recovery proof, prospective
model output, clean-session credit or permission for automatic promotion.
