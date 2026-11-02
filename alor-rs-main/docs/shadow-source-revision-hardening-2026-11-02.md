# Shadow Source Revision Hardening and Readmission

Status: implemented locally for review/testing; NOT a VPS rollout authorization
or a statement that an existing failed generation has recovered.

## Scope

The authenticated closed-M10 source can observe a late volume correction even
after two identical post-close responses. Rejecting all such corrections stops
an otherwise price-consistent IMOEXF price-only reaction experiment. Conversely,
silently replacing an already-consumed candle invalidates causal evidence.

This patch changes only standalone shadow source/supervisor tooling, associated
tests and engineering guidance. Live strategy/runtime/gateway code, CWS paths,
position sizing, signal parameters, EOD, no-overlap and existing deployments are
unchanged. Production Rust changes are limited to tests. RI's separate context
collector is not wrapped or restarted by this change.

## Contract

1. Source v3 defaults to `strict_ohlcv` for both symbols. Only the reviewed IMOEXF
   Author41-short/BO053 reaction pair can opt into
   `imoexf_author41_short_bo053_volume_audit` in immutable source config.
2. After publication, volume-only corrections are separately audited without
   changing the Redis record, source proof, frozen anchors or either arm's input.
   OHLC revisions and missing previously published rows remain fatal and have
   separate diagnostic classes. Invalid data and audit-storage failure remain
   fatal. Pre-publication confirmation still requires two identical OHLCV reads.
3. The model-invariance tests cover MR, BO, combo, every numerical transition and
   weekday/weekend context. Input hashes/provenance are intentionally NOT asserted
   identical. Do not extend this exception to USD or a future volume filter
   without a separate contract and tests. Existing USD/VWAP paths remain strict.
4. A bounded fsynced `revisions.jsonl` is immutable evidence. Unique corrections
   survive resume without repeated audit entries; corruption or a prior fatal
   event blocks resume. No automatic truncation, history rewrite or repair.
5. Supervisor v2 requires `--source-state` for the closed-M10 namespace. It pins
   source identity, matches source/worker calendar and stream, validates the
   model-specific policy and checks FAILED/manifest/writer-lock liveness. Loss
   of the source stops its owned worker, including while awaiting a response;
   the attempt cannot silently resume. No new broker route is introduced.

See [detailed source contract](shadow-closed-m10-source-2026-11-01.md) for schema,
audit budgets and OS permissions. Checks use local filesystem locks; target-host
smoke must validate permissions/locking under the actual service users.

## Controlled Readmission

1. Preserve failed source and worker generations, revision/receipt journals,
   failure markers, config hashes and the last complete session. Mark the affected
   prospective session incomplete/invalid; never count it toward clean-session GO.
2. Review the scoped diff on a clean engineering base. Run source/supervisor tests,
   Rust numerical invariance and existing model tests. Build/package the reviewed
   Python source+supervisor with a new artifact identity and file checksums. Rust
   workers need not be rebuilt for production logic: this patch changes no worker
   algorithm/protocol. Still pin the approved worker binary/hash on admission.
3. Create NEW source stream/state, worker generation and supervisor state. Do not
   resume v2-source/v1-supervisor state with this patch, remove FAILED, relabel old
   receipts as Live, or overwrite an existing model/source identity.
4. Admit only the affected IMOEXF pair first. Start source-only with bounded
   historical bootstrap and explicitly approved future activation/calendar;
   verify unchanged OHLC and separate volume-revision audit. A source-only
   preparatory run does not create retrospective model trades or clean sessions.
5. Inspect and pin 2-4 complete regular-session bootstrap inputs against that new
   stream. Incomplete current-day input cannot substitute for a complete anchor.
   Before the next approved regular cutoff, admit both worker arms together with
   matching source/worker date, source manifest, policy and `--source-state`.
6. Verify least-privilege read access to state files/lock under the worker user,
   distinct Redis ACLs, denied order writes, armed/caught-up status and no model
   actions from bootstrap. Check both contexts at cutoff. Service changes affect
   only the named pair; preserve other shadow and all live services.
7. In an isolated test generation, reproduce volume-only correction and writer
   loss. Expect audit-and-continue in the first case, fail-closed/no restart in
   the second. Never inject synthetic bars/failures into prospective observation.
8. Observe fresh bars, EOD terminal delivery and paired flat/completion, followed
   by controlled restart/next-session checks. Count only full clean prospective
   sessions of the new pair; old invalid or differently versioned sessions stay
   separately labelled. At least 20 common clean sessions remains a review gate,
   not automatic permission to promote live or delete legacy shadows.

Keep USD v2 source/worker and RI collector running during IMOEXF repair. Applying
the general supervisor guard to USD later requires its own controlled generation
change; do not interrupt a valid observation session merely for version symmetry.

## Rollback and Residual Risk

No automatic rollback or restart. Stop only the new worker, then source; retain
evidence. Reverting binaries does not make a failed or mismatched generation
resumable. Live strategy configuration is not part of this rollback.

The revision recheck window remains the last two published candles on subsequent
REST fetches. This is not continuous full-history finality validation. Volume
audit preserves an as-observed source contract, not the eventual restated broker
dataset. A stalled source can hold its writer lock: missing-bar/freshness checks
still enforce deadlines. Source row/lifetime and revision-audit bounds remain
finite; monitor and admit a successor generation explicitly when needed.

Operational snapshots, broker data, observed economics and target-specific
credentials belong in private evidence, not this engineering publication.

## Local Verification

Verified on 2026-11-02 with Python 3.12 and the scoped Rust worktree:

- Source tests: 31 passed, including the 589 -> 590 volume correction, strict
  default, immutable published prefix, audit durability and failed-resume guards.
- Supervisor tests: 18 passed, including source loss during worker startup,
  polling and a silent response wait; missing source guard blocks admission.
- Real-worker smoke: 2 passed using an owned temporary Redis on an ephemeral
  local port with read-only worker ACLs, not a live or shared Redis instance.
- IMOEXF replay tests: 17 passed; USDRUBF replay tests: 15 passed.
- Reaction-engine tests: 11 passed, 2 explicit artifact exporters ignored.
- Session-context tests: 11 passed, 1 explicit artifact exporter ignored.
- `cargo fmt --all -- --check`, strategy-runtime all-targets/all-features
  `cargo clippy -- -D warnings`, and `git diff --check` passed.

This is focused regression coverage, not a new full-workspace certification.
Target Linux smoke under actual service users, service-file permissions and
prospective admission remain rollout gates. No VPS services were changed or
restarted by this local verification; it does not recover the failed generation.
