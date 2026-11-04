# Inactive USD Worker / Read-Only Preflight

Scope: prepare a new reaction USD model pair behind an explicit admission gate.
No live/RI/IMOEXF changes, source restart, numerical worker activation, bootstrap
selection, historical reaction or generation creation is part of preparation.
See [ordinary processing budget](shadow-ordinary-model-budget-2026-11-04.md).

## Preparation

`scripts/reaction_shadow_preflight.py --stage preparation` checks a SHA-pinned
JSON plan, binaries/supervisor/prepared spec/source manifest, canonical paths,
fresh source dependency, dedicated non-root OS rights, and the existing read-only
Redis reader binding. The socket probe permits only AUTH, PING and exact-stream
bounded XRANGE, not XADD/DEL/XTRIM/SELECT. It receives credentials only through
`REACTION_SHADOW_REDIS_URL`; broker tokens and generic Redis variables are denied.
Errors expose a class/digest, never Redis credentials or raw candle prices.

Source ownership is separate from model ownership. The model OS user receives
read access through the source group, cannot modify source/config/artifact files,
and can write only its dedicated empty model parent. The source's exclusive
writer lock is observed, not acquired or downgraded. Preparation reads one source
row only as a connection/identity probe: it does not choose bootstrap IDs, pin a
bootstrap hash, run numerical code, create an attempt or write any state.

Successful preparation reports `admission_allowed=false`, `bootstrap_pinned=false`,
`worker_started=false` and all six model gates pending. It is not a model GO.

## Plan Contract

The plan has exactly `version`, `files`, `paths`, `redis`. Version is
`reaction_shadow_preflight_v1`.

- `files`: `worker`, `supervisor`, `prepared_spec`, `source_manifest`; each has
  canonical `path` and exact lowercase `sha256`.
- `paths`: `source`, `owner`, `generation`, `approved_spec`, `approval`.
  The generation must be a nonexistent child of the model-owned parent; the
  adjacent `.supervisor` must also be absent. Approval/spec live beside the plan.
- `redis`: exact private numeric `host`, integer `port`, reader `username`.
  The protected URL must match these values and DB0 before a socket is opened.

The prepared spec MUST NOT have `[runner.bootstrap]`. Its approved source calendar,
profile, costs, family, horizon/wait, activation and 360/120/30 service policy are
immutable. The launch spec may later add ONLY the inspected bootstrap block.
Do not edit the plan/model date inside an existing admitted source generation.

```sh
python3 scripts/reaction_shadow_preflight.py \
  --plan /etc/reaction-shadow/plan.json \
  --plan-sha256 APPROVED_PLAN_SHA256 --stage preparation
```

The script emits JSON to stdout only. An operator may retain that output as a
new external private audit file, never inside the active source writer tree.

## Inactive Unit

Template: `configs/examples/reaction-shadow-worker-prepared.service`. Replace all
`@...@` placeholders under review, verify with `systemd-analyze verify`, load with
daemon-reload, but do not start or enable. No `[Install]`, reboot autostart, timer,
`Wants` or `Requires` is provided. The source is not implicitly started.

The installed unit requires all three files, which preparation does NOT create:

- `ADMISSION.json`: explicit fresh operator review.
- `approved-worker.toml`: exact prepared spec plus inspected bootstrap.
- `admission.env`: only the reviewed `SHADOW_READY_BY` value.

It also requires absent model generation/attempt directories. `ExecStartPre`
runs the pinned admission preflight before the supervisor can create anything.
Missing Conditions skip the unit; invalid or stale approval fails preflight.
`Restart=no`, cgroup/process limits, read-only source/config, and network access
only to the dedicated Redis IP apply. Do not add automatic restart as a repair.

## Admission After Natural Evidence

Admission is deliberately limited to the first model morning before 07:00 MSK.
It requires a same-morning review no older than one hour and at least the full
startup budget still available before the explicit readiness deadline, both
before and after inspection. The deadline must be no later than the model cutoff.
For example, a 06:20 review with a 06:55 ready-by leaves room for the 600-second
startup budget; this is an example, not approval of a calendar date.

1. Audit the complete previous eligible weekday source session, original terminal
   and Redis/journal prefix. Review revisions and actual overnight recovery.
   Missing calendar sessions cannot be hidden by selecting older anchors.
2. After terminal/recovery review, use the pinned worker's existing
   `inspect-bootstrap` command on an EXPLICIT reviewed source-ID range. Keep 2-4
   complete sessions plus intervening context, at most 1,024 rows / 14 days.
   Inspection is context-only and cannot backdate original receipt clocks.
3. Retain the inspected preview privately; approve anchors/skipped rows. Add its
   exact six bootstrap fields to `approved-worker.toml`. No retuning is allowed.
4. Stage reviewed, bounded reports under the config's `evidence/` directory,
   readable but not writable by the model user. Create the admission record only
   after human review, and bind each report by SHA256.
5. Run admission preflight with the same OS user, supplementary source group,
   reader secret and readiness environment as the unit. This still does not
   create a generation or start a worker. Only then explicitly start the unit.

Approval fields (exact schema):

```json
{
  "version": "reaction_shadow_admission_v1",
  "plan_sha256": "EXACT_PLAN_SHA256",
  "approved_spec_sha256": "EXACT_APPROVED_SPEC_SHA256",
  "reviewed_at_utc": 0,
  "ready_by": "REVIEWED_OFFSET_AWARE_WHOLE_SECOND_TIMESTAMP",
  "gates": {
    "complete_session": false,
    "terminal": false,
    "overnight_recovery": false,
    "revisions": false,
    "source_prefix": false,
    "bootstrap": false
  },
  "evidence": [
    {"kind": "natural_session", "path": "/etc/reaction-shadow/evidence/session.json", "sha256": "REVIEWED_SHA256"},
    {"kind": "overnight_recovery", "path": "/etc/reaction-shadow/evidence/recovery.json", "sha256": "REVIEWED_SHA256"}
  ]
}
```

This is intentionally invalid/non-launchable. Do not populate true gates or
invent timestamps/hashes merely to satisfy the parser. An operator's true gates
are a human attestation bound to reviewed files, not an independent semantic
verification of those reports. Preflight verifies their identity and freshness;
it does not reinterpret arbitrary reports as automatic GO.

Admission additionally verifies the previous session's exact 07:00..23:40 M10
labels, a real forward terminal structural proof within the unchanged deadline,
and equality of its Redis/journal envelope. Bootstrap/history-only terminal is
not enough. Timely `next_timestamp` and `session_boundary_ws` are supported;
`next_timestamp_delayed` is forbidden for terminal. Price-review dates block this
first-launch path and require separate review/readmission, not flag removal.
These are structural source checks, not a second implementation of Rust proof
hashing or a substitute for the reviewed source audit.

Finally, the actual pinned worker re-runs read-only `inspect-bootstrap` on the
approved bounds. Its count/hash/dates must match exactly and both arms must have
anchors. This is the existing Rust bootstrap verifier, not a Python model clone.
Any failure keeps the unit/worker inactive and preserves all evidence.

## Verification Scope

Python fixtures cover preparation without writes/pinning, explicit missing or
expired approval, changed inputs/evidence/model parameters, existing attempts,
readiness/queue policy binding, historical/missing terminal, canonical mismatch,
price-review flags, source failure, allowed terminal race reasons, inspector
failure, RESP bounds/command restrictions and credential-safe errors. Numerical
inspector results and OS ownership are mocked only in pure policy unit tests.
They are NOT original-receipt or live-model parity claims.

Repeat preparation on Linux under the real dedicated non-root reader with its
actual source group and Redis ACL. Also run the negative admission path with
approval absent; prove no generation/attempt exists and no source/live process
was restarted. Actual positive admission remains blocked until natural future
terminal/recovery data and a reviewed final bootstrap exist.
