# Shadow Admission Evidence Permissions Hardening

Status: local implementation and Linux cross-UID regression passed.
The helper is staged and verified on the target host as a STANDBY artifact only.
The active pinned controller is unchanged; no new timer or grant was installed.
No source altered, worker started or live path changed.

## Failure Boundary

A reviewed admission controller stages root-owned evidence for a separate
non-root model account. `mkdir(mode=0o750)` does not guarantee `0750`: under the
service's `UMask=0077` it creates `0700`. Changing the group with `chown` does not
restore group traversal. Evidence files may correctly be `0440` yet remain
unreadable to the model account because of the enclosing directory.

The observed attempt completed reader-prefix and bootstrap inspection, then
failed at the first model-account admission preflight. The original controller
kept only `subprocess_failed_systemd-run`; the original child stderr/error class
was not retained. Read-only inspection confirmed an inaccessible `0700` evidence
directory. Replaying the actual staging method reproduces that sufficient
blocker; this is not a claim to have recovered the discarded child exception or
excluded every possible later preflight failure.

Source-only readiness and root-run preparation checks cannot prove cross-UID
admission readiness. These are separate checks.

## Narrow Fix

`scripts/shadow_admission_storage.py` provides two opt-in helpers. Importing it
does not change permissions, create evidence or call any service.

`create_reviewed_directory` creates a NEW evidence directory using:

1. An absolute path without parent traversal, within a pre-reviewed root-owned
   configuration tree. The immediate parent is opened as a directory without
   following a symlink. Ancestor ownership and traversal still require host
   validation; this helper is not a general untrusted-path resolver.
2. Initial mode `0700`, followed by descriptor-based `fchown(root, model_gid)`
   and explicit `fchmod(0750)`, independently of the process umask.
3. Exact owner/group/mode verification and directory/parent fsync.
4. Refusal of existing directories and symlinks. A failed staging attempt is
   retained for investigation, not repaired or overwritten automatically.

Evidence files remain no-replace, durable `root:model_group 0440` publications.
The model account gets read/traverse, not directory or file write access. Do not
use `0777`, make the model the evidence owner, remove the restrictive umask or
recursively chmod the configuration tree.

`command_failure_details` retains only the command basename, exit status,
stdout/stderr lengths and SHA256 hashes. A bounded, recognized failure JSON may
contribute its safe `child_error_class`; raw output, argv, environment,
credentials and arbitrary exception text are never echoed. These fields are
diagnostics, never a permission or admission decision.

## Controller Integration

The dated host controller is an operational artifact, separate from the Rust
worker. The locally tested candidate changes only its staging and failure
diagnostics:

```python
from shadow_admission_storage import (
    command_failure_details,
    create_reviewed_directory,
)

# Replace mkdir(mode=0o750) followed by chown in Host.stage:
create_reviewed_directory(evidence_dir, gid)

# Replace the generic nonzero-return branch in Host.command:
if result.returncode != 0:
    details = command_failure_details(
        args[0], result.returncode, result.stdout, result.stderr,
    )
    raise Hold(
        'subprocess_failed_' + Path(args[0]).name
        + ':' + json.dumps(details, sort_keys=True)
    )
```

Deliver the helper beside the reviewed controller and explicitly pin both file
hashes in the next approved operational release. The helper alone does not fix
an already staged attempt. Do not replace a pinned controller in place or
silently reseal an existing approval.

## Verification

```sh
PYTHONPATH=scripts python3 -B -m unittest test_shadow_admission_storage -v
```

The suite covers strict/permissive umasks, the original defect, existing and
symlinked targets, a symlinked immediate parent, failed ownership changes,
invalid identities and secret-safe diagnostics. On non-root/macOS runs the
cross-UID test is explicitly skipped, not counted as a Linux isolation pass.

A local Linux/amd64 container run passed all nine tests with networking disabled,
a read-only filesystem and scripts, temporary fixture storage and only
CHOWN/SETUID/SETGID capabilities. The separate reader UID can read `0440` evidence
through `0750` as the authorized group, cannot write it or its directory, and
cannot read it as another group. This is a kernel DAC test, not a real systemd
sandbox/admission qualification.

The captured host controller's actual `Host.stage` was also replayed against a
temporary policy fixture: original `0700`, candidate `0750`, three `0440` files,
no group write. That local replay uses fixture ownership and grants no authority.
The actual `Host.command` failure path preserves only safe diagnostic fields.

## Safe Rollout Follow-Up

- Retain the failed attempt and its admission lock for audit. Never backdate a
  grant, delete the lock to retry, or edit a first-session date in place.
- Review any newly arrived source revisions against the selected context and
  consumed fields. The permission fix does not approve later audit records.
- Reconnect/new WS generation within the same qualified source process is an
  expected recovery scenario. It is not by itself a reason to HOLD; restoration,
  ACK, coverage and the unchanged canonical prefix must still be checked.
- Use a new explicitly reviewed dated plan/generation and a full pre-session
  startup window. No diagnostic in-session start is implicit in this patch.
- Before grant/start, test evidence readability as the real model UID under the
  actual systemd sandbox; verify the source remains non-writable and broker
  credentials remain inaccessible. Record this read-only check's outcome.
- Recheck current source prefix, terminal evidence, bootstrap, audit scope,
  resource budget and pinned release files, then confirm both workers armed and
  caught up before opening. A failed check leaves the pair unadmitted.
- Live strategies, old shadow generations, terminal finalization and `BarEvent`
  remain unchanged. This patch does not implement the separate context-aware
  permanent admission classifier or the calendar/OI session coordinator.

Raw host logs, account identities, exact audit records and economic observations
stay in ignored operational evidence, not this engineering document.

## Recovery-Date Binding

Inspection also found a separate scheduling dependency: the older paired
supervisor's strict source guard requires `source.first_live_session` to equal
`runner.first_decision_session`. The running sources and failed plan share their
original date. Reusing those healthy sources for a later, fresh model generation
will fail that default check even after the evidence permission defect is fixed.

Do not schedule a renamed copy of the expired one-shot and assume it can pass.
The preferred follow-up is an explicitly reviewed NORMAL source-continuity
binding: original activation date, exact manifest/stream/release/process identity,
new model activation date, fresh bootstrap and reviewed context. Preserve current
strict defaults without that binding. Do not misuse diagnostic late-start, change
an existing source manifest, backdate a model grant or replay missed decisions.
The new calendar/OI dated driver has a separate explicit source binding; that
does not authorize or silently upgrade the older paired worker's v3 procedure.

Target-host standby qualification exercised all nine storage tests plus separate
read/no-write fixtures under BOTH actual model UIDs. Container IDs/start times,
systemd source/model states and the active controller hash were identical before
and after. No actual staged admission directory was repaired or overwritten.
