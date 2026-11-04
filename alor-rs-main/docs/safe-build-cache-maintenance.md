# Safe Build-Cache Maintenance

Scope: reclaim verified unused build/cache storage without changing running
services, broker orders, strategy state, Redis data or shadow admission gates.
This is separate from [Redis retention](redis-retention-and-cleanup-plan-2026-04-21.md).
Resource pressure is not permission to erase recovery or observation evidence.

## Weekly Inventory

Record actual host disk/RAM, Docker usage, Redis memory/AOF status, process and
service health. Compare resource trends rather than nominal image sizes alone.
Read-only inventory can be scheduled; deletion needs a separately reviewed exact
allowlist and should not be a blind recursive weekend job.

```sh
df -h
free -h
docker system df
docker ps -a --no-trunc
docker image ls --no-trunc --filter dangling=true
```

Never print secret EnvironmentFile contents, Docker environment arrays, broker
refresh/access tokens or Redis credential URLs in inventory logs.

## Target Is Not Always A Cache

A Rust `target` directory can contain an executable used directly by a live
systemd unit or a retained replay/test binary. Its name alone does not make it
disposable. Do not run a global find/delete or cargo-clean across host projects.

For EACH candidate exact path:

1. Require a canonical nonsymlink path under an explicitly reviewed build root.
   Confirm that this is a completed build, not active compilation or ingestion.
2. Inspect all process executable/cwd, open-file and mapped-library references,
   including host services outside Docker. Check systemd, timers and cron.
3. Inspect bind mounts of ALL containers, including stopped rollback containers.
   A parent-directory mount can also expose the target to a running process.
4. Preserve independent deployment/rollback binaries and verify their SHA256
   against the pinned artifact/result manifest. A symlink into target is not an
   independent copy. Keep source, lockfiles, source archives, test results and logs.
5. Retain the current reproduction target and any target still needed for an
   active parity/price-sensitivity investigation. Keep one deliberate rebuild
   cache and necessary offline Cargo/toolchain dependencies.
6. Recheck references immediately before deletion; delete only the reviewed paths.
   An unexpected process, path, symlink or artifact mismatch blocks the operation.

Cache removal may require recompilation for a future historical test rerun. It
must not require rebuilding a binary just to restart an already admitted service.
Record which build directories were removed and which pinned artifacts remain.

## Python Environments

A `.venv`/`venv` is a dependency installation, not a disposable download cache.
The absence of an active Python process does not prove it unused: systemd,
cron, scheduled analysis, shell shebangs, rollback procedures and external jobs
may need it later. Preserve active and rollback environments until the owner
has approved retirement and a reproducible dependency lock/install procedure.

Pip/uv download caches are different. They can often be regenerated but may be
needed for offline maintenance; inspect installer activity, connectivity and the
space benefit before removing them. Do not delete working environments or Cargo
registries to achieve a cosmetic disk-use target when there is ample headroom.

## Docker Images

Protect images referenced by ANY running or stopped container, all current
deployment/rollback tags, and deliberately retained offline build bases.
Retain containers, bind-mounted data and volumes; do not use broad system prune.

For an image candidate, inspect its full immutable ID, tags, creation date,
container references and possible ongoing builds. A conservative initial policy
is an untagged dangling image older than seven days with no container references.
Capture the exact candidate list first; do not sweep new concurrent build output.

After review, remove one explicit immutable ID WITHOUT force or parent pruning:

```sh
docker image rm --no-prune "$APPROVED_DANGLING_IMAGE_ID"
```

If Docker refuses because of child images or references, retain it and record the
skip. Do not escalate to force. Do not sum nominal sizes as recovered space:
layers are shared. Measure actual `df` free-space change and Docker usage after
garbage collection. A zero-size builder cache does not require a prune command.

## Before / After Guards

Capture container IDs/images/start times/restarts/health, source/collector PIDs,
independent binary hashes, retained FAILED markers, and deployment image tags.
Compare containers by ID and mounts as sets; Docker enumeration order is not a
state change. Do not weaken identity, image or restart checks to ignore a real
difference. Preserve the original before snapshot and reviewed deletion plan.

For live gateway/runtime, verify current readiness and guard state, fresh WS/CWS
health, action-scoped control path, command-consumer status, pending/error counters
and absence of newly required operator intervention. Parse log LEVELS rather than
counting the word `error` inside an ordinary WARN message as a separate ERROR.
Separate recovered overnight transport resets from a current persistent outage.

For shadow sources, check heartbeat, writer health, canonical/pending status,
revision review flags and unchanged failed evidence. An inactive prepared worker
must remain inactive, with no newly created model generation or admission files.
Source readiness is still not model GO or clean-session credit.

Check Redis eviction counters, AOF write status and memory after the operation,
without trim, DEL, consumer-group resets, FLUSH or persistence rewrites. Check
host-runtime services in other projects as well as the ALOR Docker stacks.

Retain a private report with allowlisted paths/image IDs, guards, skipped items,
actual reclaimed bytes and after-check results. Stop on an unexpected difference;
do not conceal it by restarting services or resetting state.

## Git And Publication

Publish reusable engineering policy/runbooks without raw host inventories,
private observation/retrospective data, broker ledgers or secret files. Keep raw
before/after reports in the approved private evidence store.

Use a clean scoped engineering worktree for maintenance documentation. An older
dirty worktree or a diverged local main is not repaired by `git add .`, forced
reset, blind rebase, or publishing private historical commits. Inventory it,
preserve unrelated work, and review synchronization separately from host cleanup.
