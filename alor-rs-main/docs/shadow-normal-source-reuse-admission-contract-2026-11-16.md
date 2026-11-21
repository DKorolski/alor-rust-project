# Normal Shadow Source Reuse: Recovery Contract

Status: opt-in v4 Python procedure implemented and tested. New dated plans passed
target-host preparation under the actual model UIDs; new units are registered
INACTIVE and conditioned on absent admission artifacts. No grant/model generation
or timer created. The calendar/OI dated driver remains separate.

## Why A New Date Alone Is Insufficient

The existing strict `ClosedSourceGuard` deliberately requires the prepared model
date to match the source's original activation date. A healthy, continuously
running MD-only source cannot therefore be reused for a later first model day by
simply changing `first_decision_session`. The failed attempt must remain retained;
its date, grants, hashes and admission lock must not be rewritten.

Restarting a qualified source solely to change its manifest date also discards
useful recovery continuity. Diagnostic late-start is not an alternative normal
contract and must not be used to bypass this mismatch.

## Compatibility Boundary

The separately versioned, opt-in NORMAL source-continuity binding extends the paired
preflight/launcher/supervisor procedure, rather than changing strict defaults.

- Preserve v1/v2/v3 plans and their exact source-date equality when no new binding
  is present. Existing live and shadow configurations do not opt in implicitly.
- A new reviewed plan binds the original source activation, new model activation,
  exact source manifest/stream/symbol, qualified release, boot and source process
  identity. Original source activation must precede or equal the new model day.
- Preserve the current H4/W4 and IMOEXF numerical config, costs, first-receipt
  context, terminal proof and broker-command prohibition. No Rust signal retune
  or old history relabeled Live is needed for this operational date binding.
- The fresh model generation has a new explicit owner path and prepared config;
  no bootstrap/grant is copied from the failed generation. A fresh bootstrap
  digest and real model dependency trace must be recomputed before admission.
- Carry the validated binding into the durable supervisor/grant manifest and
  revalidate it on restart. An unrelated diagnostic mode, caller boolean or
  unpinned source path cannot manufacture the binding.
- Stage root:model-group 0750 evidence with 0440 files, recheck actual model-UID
  readability under the service sandbox, then run both preflights before starting
  either model. Pin the permission helper in the new operational release.

Old v1/v2/v3 plans retain their original behavior and exact field sets. An old
plan cannot smuggle a continuity field or silently opt into another identity.

## Implemented Versions

| Artifact | New opt-in version |
| --- | --- |
| Preflight plan | `reaction_shadow_preflight_v4` |
| Morning approval | `reaction_shadow_admission_v4` |
| Durable grant | `reaction_shadow_admission_grant_v2` |
| Supervisor manifest | `reaction_shadow_supervisor_v4` |
| Continuity binding | `reaction_shadow_source_continuity_v1` |

The v4 plan adds an inline `source_continuity` and a pinned
`files.continuity_helper` entry. `scripts/shadow_source_continuity.py` validates
canonical source/prepared paths, exact manifest and prepared-spec hashes,
symbol/stream, original activation <= new model day, Linux PID/start ticks/boot,
and a minimum WS generation. On an approved spec, only the separately verified
bootstrap section may differ from the prepared spec. Diagnostic late-start is
explicitly incompatible. No caller boolean relaxes these checks.

The guard rechecks process/boot on every dependency check. A reconnect generation
increase in the SAME pinned process remains ordinary recovery after source
readiness returns. A replacement process requires reauthorization, not a new
unreviewed PID inferred from whichever process is now running.

Both v4 preparation and admission read the entire claimed canonical source range
under the dedicated reader, comparing IDs/order and all envelope fields. The
existing source manifest/heartbeat/writer-lock, collection/model budgets, current
audit snapshots, terminal proof, bootstrap, morning freshness and read-only
permissions remain required. Preparation reports pending gates, never a grant.

The v4 approval explicitly includes `source_continuity_sha256`. The durable grant
retains the full binding/hash, helper hash, original source identity, reviewed
bootstrap/audit snapshots and reader UID. The supervisor manifest fsync precedes
any worker spawn. Worker input remains the existing Rust spec/protocol; no Rust
source, signal formula, transport schema or new strategy kind is introduced.

## Create And Resume

```sh
python3 -B scripts/reaction_shadow_preflight.py \
  --plan /reviewed/new/plan.json --plan-sha256 EXACT_PLAN_SHA --stage preparation

# Only after the separately reviewed model-morning approval exists:
python3 -B scripts/reaction_shadow_launch.py \
  --plan /reviewed/new/plan.json --plan-sha256 EXACT_PLAN_SHA \
  --ready-by YYYY-MM-DDT06:50:00+03:00
```

Use the dedicated model account and the reviewed service sandbox. These commands
are not authority to invent approval fields, copy an expired grant or run late.

Explicit v4 resume uses the SAME plan/spec/generation and a fresh pre-open startup
deadline, after a clean `operator_stop` only:

```sh
python3 -B scripts/reaction_shadow_launch.py --resume \
  --plan /reviewed/new/plan.json --plan-sha256 EXACT_PLAN_SHA \
  --ready-by YYYY-MM-DDT06:50:00+03:00
```

Resume verifies stored grant/manifest, current code/helper/spec/approval hashes,
original reader UID, exact process/manifest/binding and the canonical reader
prefix again. It never reruns CREATE preflight, renews the grant, changes the
original model date, reimports bootstrap or resets counters. A missing/failed
attempt, completed generation, changed identity or midday/insufficient startup
window fails. Legacy v2 supervisor resume cannot drop a v4 manifest's grant.

The original create unit remains create-only. Recovery must use the explicit
reviewed resume command/service after stopping the old parent, not an automatic
Restart policy or two processes sharing the same state directory.

## Admission Morning Inputs

The target day's previous complete session must really have ended. A check during
that previous session cannot certify its future terminal or completed context.
This is an actual data dependency, not a requirement to wait an arbitrary extra
night after already qualified recovery.

For the new launch, validate source ACK/history/catch-up, mandatory bars, the
entire immutable published prefix, both arms' selected anchors/dependencies,
reviewed audit scope, fresh bootstrap and resource/clock/isolation gates. Use the
same process across ordinary WS reconnect after successful recovery. New material
or unresolved relevant audit is HOLD; audit growth alone is not the permanent
decision rule. The permanent dependency-aware classifier remains a separate P1
acceptance, not something the storage permission patch implements.

## Verification And Launch Boundary

The native procedure suite passed 104 tests (one Linux-only identity check
explicitly skipped on macOS); Linux/amd64 passed all 104, including the owned
Linux process/start-ticks/boot check. New fixtures cover both instruments, default
strict rejection, preparation without a model call, explicit approval, whole-range
mutation, process replacement, durable binding/grant and clean pre-open resume.
These are controlled policy/protocol fixtures, not a prospective clean session.

Target-host preparation then ran under both actual model UIDs and the service
sandbox, verifying each current 636-row source prefix. Model config and bounded
policy were compared to the deployed originals and remained equal. Only fresh
model activation/generation/receipt identities changed. Existing source processes,
manifests, Redis streams, live containers and legacy shadow processes remained
untouched. Neither a future terminal nor a completed previous session was claimed.

The dated root orchestration has separate offline/target-host tests: both
preflights precede any start; ownership is claimed before start; partial-start
failure stops only the new pair; cleanup handles a torn outcome/claim and one
failed stop without skipping another valid claim. Success retains both units.
It requires a new root-owned morning review and a controlled oneshot with external
`ExecStopPost` cleanup. The entrypoint rejects the wrong date BEFORE reservation.
Root job/unit templates are operational artifacts, not installed launch authority.

Registered model units remain inactive, lack `[Install]`, use `Restart=no` and
require the new approval/spec/env files. They have no implicit `Requires`/`BindsTo`
activation of an upstream source. Loading their definitions does not start them.
No automatic timer was installed: the dated review must be completed first.

Continue to require unchanged old strict behavior plus an owned fixture covering:

Require unchanged old strict behavior plus an owned, non-broker fixture covering:

1. Older qualified source, new model date, new paths, complete previous terminal
   and explicitly reviewed continuity: normal pre-open admission, no late-start.
2. The same date mismatch WITHOUT the new binding: default rejection.
3. Process/boot/manifest/stream/release change, incomplete mandatory context,
   changed prefix, stale readiness, unresolved relevant audit: no grant/start.
4. Night reconnect in the same source process, restored ACK and immutable prefix:
   no forced source reset or invented extra observation delay.
5. Restart retains the same binding/bootstrap and source cursor; the failed old
   attempt remains untouched. No duplicate model decision or broker command.

Before actual admission, review the previous completed session/terminal, actual
Rust-selected context/anchors, all relevant price/quarantine audits, source prefix,
fresh bootstrap and resources, then perform morning non-root admission preflights.
The full model-dependency-aware P1 audit classifier is still separate and NOT
claimed by this source-date fix. Accepted as-observed history is not rewritten.
