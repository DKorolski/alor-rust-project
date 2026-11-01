# Isolated Shadow Service Runbook

Engineering instructions for the standalone paired reaction worker and RI
context collector. Examples are NOT approved deployment configurations. This
publication does not change or deploy live runtimes, gateways or existing shadows.
See [observation and promotion gates](shadow-observation-and-live-promotion.md).

## Scope and Isolation

One reaction process runs two independent arms: `weekday_only` and
`weekend_context_no_trades`. Supported service profiles are IMOEXF reaction
MR/BO/combo and USDRUBF Previous-day07 H3/W4 or H4/W4. Numerical support for other
profiles does not imply service admission. RI uses its separate paired collector.

The reaction worker does not instantiate the live Runtime, order publishers or
gateway clients. Use a distinct unprivileged OS account, private generation
directory, dedicated source-only Redis ACL and network access only to that Redis.
Allow `XRANGE/MULTI/EXEC/PING` on the approved input key; verify authentication
and ACL denial on the installed Redis version. No XADD, groups, trim, DEL,
live-state keys or broker credentials in the model worker.

The optional [closed-M10 source](shadow-closed-m10-source-2026-11-01.md) has a
separate account and secret. It writes only its new source key, never commands.
Neither source nor worker may mutate a shared legacy stream or ledger.

## Build and Identity

Python 3.11+ on Unix is required for the supervisor (`tomllib`). Build from a
clean reviewed source commit with the checked-in lockfile, on a target-compatible
toolchain. Pin source commit, binary SHA256, spec SHA256, model/reference hashes,
calendar, instrument/contract and input-stream identity in release records.

```sh
cargo build --locked --release -p strategy-runtime \
  --bin reaction_shadow_worker --bin ri_context_shadow
```

Use `configs/examples/reaction-shadow.imoexf-combo.toml`,
`reaction-shadow.usdrubf-h4-w4.toml` and `ri-context-shadow.toml` only as shapes.
Replace placeholder streams, generation IDs, activation and excluded dates after
calendar/source review. Keep frozen costs: IMOEXF rate per component per side
`0.00006575119054451348`, two components, 10 RUB/point, 5 RUB/tick. The synthetic
`0.001` fixture rate is not the frozen economic policy.

## Bootstrap and Readiness

Reaction modes: prospective context warmup, or explicit bounded history import.
The model needs previous/previous2, not an arbitrary 10,000-bar replay. Choose
2-4 complete eligible regular sessions, normally three, plus intervening context.
Do not import High180's lb120 seed/ledger. RI has separate prospective warmup.

1. Approve future activation, source, symbol, calendar and source-ID range. Inspect
   candle labels too: stream IDs need not represent receipt or market time. The
   bootstrap permits at most 1,024 records over 14 calendar days. A missing regular
   day is not skipped unless explicitly excluded by the approved calendar.
2. Inspect with the reviewed worker and dedicated `REACTION_SHADOW_REDIS_URL`:

```sh
reaction_shadow_worker inspect-bootstrap approved-pair.toml FIRST_ID LAST_ID
```

3. Review anchors/completeness/skipped rows. Copy the exact inspected
   `first_source_id`, `last_source_id`, `rows`, `source_sha256`, `first_session`
   and `last_session` into `[runner.bootstrap]`. Never invent bounds/hash or
   backdate inspection as original delivery. Inspection writes no generation.
4. Create before the first cutoff. The verified prefix seeds both context stores
   atomically, without retrospective model trades. Check bootstrap loaded, caught
   up, armed and zero historical decisions. `pair_ready` is checked at freeze;
   armed alone does not certify model readiness or a clean session.
5. On resume use the durable cursor and original evidence. A missing/changed resume
   row, spec/hash mismatch or failed attempt blocks startup, not a fresh history
   overwrite. A clean bootstrap is not proof that all future bars will arrive.

## Start and Stop

Pass `REACTION_SHADOW_REDIS_URL` through the approved secret mechanism, never
CLI arguments or tracked files. The generation parent must exist; paths must be
canonical without symlink indirection. Use a new directory for `create`:

```sh
python3 scripts/reaction_shadow_service.py create \
  --worker /opt/reaction-shadow/bin/reaction_shadow_worker \
  --spec /etc/reaction-shadow/approved-pair.toml \
  --generation /var/lib/reaction-shadow/approved-generation \
  --ready-by "$APPROVED_READY_BY"
```

`APPROVED_READY_BY` is an explicit future offset-aware ISO time no later than the
next model-open cutoff. `resume` uses the same syntax, exact binary/spec and
existing generation. It is permitted only after an acknowledged operator STOP
and before initial cutoff, or after complete paired EOD and before the next
eligible cutoff. Intraday automatic recovery is NOT supported by this wrapper.

SIGINT/SIGTERM requests a bounded acknowledged STOP of the owned worker group.
The adjacent `<generation>.supervisor` directory is part of recovery evidence:
pinned manifest, exclusive lock and fsynced numbered started/stopped attempts.
Crash, failed stop, missing acknowledgment or incomplete attempt blocks relaunch.
No `--force`, marker deletion, automatic fresh generation or hidden cursor skip.

RI uses `RI_SHADOW_REDIS_URL` and its own state directory:

```sh
ri_context_shadow create approved-ri.toml /var/lib/ri-shadow/new-generation
```

Its `resume` path validates persisted source/receipt/context state. Do not assume
the reaction supervisor's lifecycle protocol applies to this separate executable.
Review signals/service-manager shutdown behavior and a real restart on the target
host before unattended collection. Neither launcher is an approved live process.

Use `Restart=no`, explicit admission rather than reboot autostart, cgroup CPU/RAM
bounds and an external watchdog. Kill only the owned process group. Redis ACL
persistence/reprovision is an operator responsibility; do not assume runtime-only
ACLs survive Redis restart. No generic systemd unit is installed by this code.

## Limits and Diagnostics

| Required policy | Example budget |
| --- | ---: |
| startup_timeout_seconds | 600 |
| poll_timeout_seconds | 60 |
| poll_interval_seconds | 5 |
| shutdown_timeout_seconds | 15 |
| max_queue_age_seconds | 30 |
| max_model_lateness_seconds | 120 after label + 600 seconds |
| max_sessions | 20 paired sessions |

These are engineering starting budgets, not capacity guarantees. Storage bounds:
10,000 receipts, 64 MiB ingress, 128 cutoffs, 20,000 transactions, 256 MiB
transactions and 16 MiB/file. Warn at 60% of any bound; stop/audit at 80%.
Session-budget completion is a clean stop, not permission for automatic rollover.
Source bounds are separate. Never trim evidence or raise limits silently.

Observe source cursor/caught-up, original receipt age, missing/late expected M10,
bootstrap, frozen anchors, pair readiness, completed sessions, position/pending
state, storage usage, heartbeat and `order_emission_allowed=false`. Preserve
FAILED generations. A missing terminal candle must fail visibly, not manufacture
EOD flat. Deduplicate economic records by durable outbox identity, not cumulative
trade lists repeated in heartbeats. Keep actual session evidence private.

## Validation and Rollback

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p strategy-runtime --test config_tests
cargo test --locked -p strategy-runtime \
  --bin imoexf_shadow_replay --bin usdrubf_shadow_replay
python3 -m unittest discover -s scripts -p test_reaction_shadow_service.py
python3 -m unittest discover -s scripts -p test_shadow_closed_bar_source.py
```

Independent reference/state parity requires the access-controlled frozen handoffs;
see [parity tools](../scripts/shadow_state_parity/README.md). Self-comparison and
similar aggregate PnL are not sufficient. Target-host smoke must include ACL
denial, complete session/EOD, bounded cold restore, missing cursor, crash and
new-process recovery of the exact release. Keep build/test provenance separate
from prospective clean-session counts.

Redis integration tests need owned isolated endpoints. Unsetting `REDIS_URL`
alone does NOT make an unfiltered library run safe if a test defaults to local
6379. Use a reviewed network/owned-Redis harness; do not run it against a real
service. `REACTION_WORKER_BIN` enables the separate CLI smoke suite; absence
means that suite is explicitly skipped, not passed.

### Local Linux Release Probe

`scripts/shadow_state_parity/linux_release.py` builds an allowlisted, manifest-
pinned source snapshot and uses explicitly supplied independent fixture inputs.
It is a compatibility/recovery probe, not deployment or prospective acceptance.
Run its `--help` and record the exact clean commit, source/input/binary hashes,
toolchain, resource limits and owned test environment. Emulation results do not
replace native-host timing/memory acceptance. Generated reports stay private.

Rollback stops only the new owned worker/source services and preserves evidence.
Keep legacy controls and shared Redis/gateways running. Never reconnect a failed
generation to an unapproved source or delete supervisor state to bypass admission.
