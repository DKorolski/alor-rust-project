# IMOEXF Calendar/OI: Stage-One Engineering Progress

Status: E0 verified; E1 pinned loader/reference fixtures implemented;
E2 offline foundation and feature-gated HTTP collector implemented locally.
E3 independent component features and isolated numerical engine implemented;
synthetic and frozen final-history feature/event/state reference parity passed.
Prospective receipt/OI qualification and operational wiring remain pending.
E4 continued 2026-11-16: local durable three-arm session bridge and frozen
qualification bundles implemented; actual source coordinator/admission pending.
Later continuation adds a local one-day read-only Redis driver; multi-day
operation, permanent audit classification and operational admission still pending.
Not full model parity, not admitted for shadow, no VPS deployment.

## Frozen Scope

- Three proposed virtual arms: IMO_AA, IMO_AB and IMO_AB_OI, quantity one.
- Twenty regular sessions after separately accepted parity/warmup/admission.
- Two prospective contrasts: IMO_AB minus IMO_AA and IMO_AB_OI minus IMO_AB.
- Preserve USDRUBF Previous-day07 H4/W4 weekday_only and
  weekend_context_no_trades, including state, costs, cursor and observation count.
- No new USD/H3 profile, live retune, brackets, overnight or broker route.

Both engineering-TZ copies were synchronized with this first-stage scope. The
research package's engineering-document inventory row was refreshed; historical
results, source models and all frozen economic inputs were not rewritten.

## E0 Audit

`scripts/freeze_imoexf_calendar_oi_stage1.py` checks the three supplied research
packages' inventories and frozen dependencies before creating an explicitly
requested baseline. Default verification is read-only; an existing baseline is
never overwritten. Conflicting hashes, missing files and protected-source drift
fail the audit rather than silently resealing a changed model.

The local baseline pins 112 references, validates 107 frozen dependencies and
checks four protected existing files against HEAD: H4/W4 template and the three
live strategy source files. Full evidence is kept outside public engineering
history in ignored `private_docs/imoexf-calendar-oi-stage1-2026-11-15/`.
Hashes are provenance, not proof of alpha or complete event-level parity.

```sh
python3 -B scripts/freeze_imoexf_calendar_oi_stage1.py \
  --workspace /path/to/strategies_list \
  --manifest /path/to/private/baseline-manifest.json
```

Implementation gaps established by inspection:

| Area | Existing path | Required stage-one work |
| --- | --- | --- |
| Calendar | Weekday/exclusion-based frozen-context core | Explicit dated instrument registry and known/unknown session handling |
| MR/BO context | One previous/previous2 pair per numerical session | Independent component selection and feature injection |
| OI | Research one-shot only | Validated immutable capture and daily entry-only BO gate |
| Costs | Existing IMO fee-component policy | Separately versioned research friction accounting |
| USD | Accepted H4/W4 pair | No replacement; H3 statistics are not H4 acceptance targets |
| Admission | Qualified existing source and dated grant | Own identities, resource/isolation checks and reviewed admission for new arms |

## E1 Foundation

`strategy-runtime/src/shadow_calendar.rs` adds an explicit opt-in
calendar API. It is exported by the library but is not called by existing
workers, live strategies, gateway or configuration loading. No new strategy kind
or implicit feature default was added.

Implemented:

- Versioned, bounded, per-symbol definitions with distinct calendar and exchange
  trade dates. Missing dates are UNKNOWN, never inferred from the weekday.
- REGULAR/DSVD/CLOSED/UNKNOWN routing. A regular working Saturday may update
  context without enabling weekend entries. DSVD may update B but not A.
- Separate context, entry and local position-management permissions. The latter
  does not override broker scheduling or authorize an executable fill.
- Explicit full/main-session boundaries and UTC/MSK mapping. Legacy partially
  tradable grid bins are preserved as calendar bins, not executable fill claims.
- Regular exact-grid coverage and frozen DSVD span/count guards: 44/55 or 68/84.
  Eligible context label 09:50 is retained; regular service bars are ignored.
- Independent MR/BO previous and previous2 date selection with calendar-bound
  coverage. Missing expected sessions block dependent selection rather than
  falling back to older valid anchors. Known CLOSED dates preserve warmup.
- A missing BO dependency blocks combo readiness even if MR dependencies exist.
  Old unknown dates outside the newly selected pair do not poison readiness forever.

`combo_ready` describes these calendar dependencies only. It is not an admission
grant, receipt/durability proof, market-data readiness or permission to trade.

### Dated Registry Loading And Reference Checks

`Calendar::load/from_json` requires a separately reviewed `ArtifactPin`: exact
file SHA256, source-reference SHA256, calendar version, symbol and date horizon.
The loader reads at most 4 MiB and rejects malformed/duplicate JSON fields or
an identity mismatch. Dates absent from the artifact, including dates beyond its
horizon, remain UNKNOWN. It does not generate future classifications from a
weekday or infer the calendar from observed candles.

`scripts/export_shadow_calendar_reference.py` produces the synthetic
native fixture from the pinned, unmodified `exchange_calendar.py` and the exact
`inspect_sessions/build_ledger` function bodies from pinned `session_data.py`.
Only those two pure function bodies are extracted by AST, avoiding research
model-loader/plotting side effects. This is not a run of the full research model.
Both source hashes are checked before execution; output creation is exclusive.

The fixture has 38 cases for IMOEXF/USDRUBF: working Saturday, late opening,
morning/main-session transitions, product-specific DSVD admission, extended
holiday and incomplete scheduled sessions. Rust matches expected label grids,
coverage acceptance and A/B previous/previous2 selection/readiness. Prices are
synthetic; no broker observation, PnL or proprietary receipt journal is included.
USD fixture coverage does not introduce or change a USD model/profile.

Reproduce into a NEW temporary output and compare to the repository test fixture:

```sh
python3.12 -m venv /tmp/alor-calendar-reference
/tmp/alor-calendar-reference/bin/pip install pandas==2.2.3 numpy==2.1.3
/tmp/alor-calendar-reference/bin/python -B scripts/export_shadow_calendar_reference.py \
  --reference /path/to/analiz_alpha_si/moex_calendar_context_2026_11_15 \
  --output /tmp/calendar-reference-new.json
cmp /tmp/calendar-reference-new.json strategy-runtime/tests/fixtures/calendar_reference_v1.json
```

## E2 Offline Foundation

`strategy-runtime/src/shadow_oi.rs` is the pure validator and append-only local
evidence store. Its default build has no HTTP client, scheduler, trading
credentials, runtime integration or broker-command path. A separate opt-in
collector is described below. No real ISS capture or OI service installation
was performed by these development slices.

Implemented:

- IMOEXF regular07 weekday plan bound to model/calendar identities and exact
  previous two REGULAR dates. DSVD B price context does not change OI dates;
  an unknown source frontier fails rather than falling back to an older day.
- Exact public dated-history URL/params; HTTP/schema/pagination checks; SECID,
  board, dates, duplicate columns/rows, finite/nonnegative values and positive
  denominator validation. Current/static OI cannot substitute for dated history.
- Ordered request/receipt/durable timestamps; 06:45..06:55 MSK window, inclusive
  deadline and strictly negative change. Local defaults reject missing clock
  evidence, offset above 1000 ms, HTTP duration above 30 s or server-Date skew
  above 300 s. These conservative transport limits need review at collector wiring.
- Immutable UUID-named raw captures/manifests with hashes and UTC/MSK metadata.
  A daily decision is no-replace published via a same-filesystem hard link after
  file fsync; its directory is fsynced. A second writer cannot overwrite it.
- Retryable errors do not freeze a missing day before the deadline. At deadline
  missing/invalid OI disables new BO; a first late freeze cannot enable it.
  Restart reuses the frozen result, not a newer vintage, and rechecks referenced
  raw/manifest hashes. Changed model/calendar identity requires explicit rebuild.

**Boundary:** receipt, raw durability, clock health and freeze time are supplied
by the caller for these offline tests. Filesystem publication does not attest the
real final-commit deadline. The future collector must own those clocks/barriers,
check actual completion time and fail closed on an uncertain/late commit. Do not
use fixture timestamps or these files alone as a prospective PIT/admission proof.
Crash-before-freeze is not silently reconstructed from an arbitrary newer raw
capture. Reopening an existing decision never enables BO after a missing day.

Only a new BO entry may consult this result after the separate integration;
MR, BO observation, existing positions and exits must continue. That integration
and its OI-off/event/state parity are not implemented or claimed yet.

## E2 Opt-In HTTP Collector

`strategy-runtime/src/shadow_oi/collector.rs` and `collector/io.rs` implement a
manually invoked one-shot; `src/bin/oi_shadow_capture.rs` is its CLI. Both require
`shadow-oi-http`. The normal runtime build does not enable this feature or build
this binary. The existing locked reqwest/libc packages become optional direct
dependencies; no locked package version was upgraded.

This is a separate daily OI input, NOT a replacement REST candle feed, gateway
or strategy runtime. WS bar sources, terminal finalizer, BarEvent, current
live/shadow workers and USDRUBF H4/W4 remain unchanged.

### Transport And Clock Rules

- GET only to the fixed public dated IMOEXF history URL with the two exact
  REGULAR dates. No configurable host, authentication, cookies, redirect or
  environment proxy. Unknown config fields, including trading keys, are rejected.
- Total HTTP timeout up to 30 seconds, connect timeout 5 seconds, response cap
  1 MiB. Partial/oversized responses are marked incomplete; a valid-looking JSON
  prefix can never qualify. Request metadata, exact available response bytes,
  response hash and UTC/MSK clocks are retained for each recorded attempt.
- Linux clock health comes from read-only `adjtimex(modes=0)`. TIME_OK is required,
  without UNSYNC/CLOCKERR, with offset and maximum-error bounds of 1000 ms each.
  The offset's micro/nanosecond distinction and maxerror's microsecond unit follow
  the [Linux API](https://man7.org/linux/man-pages/man2/adjtimex.2.html).
  Unsupported/denied/unsynchronized queries are not replaced with a made-up zero.
- Each important boundary samples UTC and a process-monotonic clock. A mismatch
  above 250 ms, backwards time or lost synchronization closes the day's BO gate.
  The decision-durability deadline includes clock uncertainty, not only the
  displayed wall time. No time is adjusted by this program.
- No query starts outside dated 06:45..06:55 MSK or without a 10-second commit
  reserve. Premature invocation does not freeze a future day as failed. A normal
  first attempt after the deadline is refused; it cannot create a late success.

### Persistence And Recovery

The collector measures raw durability AFTER raw file and directory fsync. After
freezing the data decision, it again samples actual time AFTER the data barrier
and rechecks clock health, monotonic consistency and the uncertainty-adjusted
06:55 deadline. An initially timely response followed by a late data fsync
produces `late_commit`, never BO permission.

`qualified/<date>.json` binds the plan/model/calendar, exact data-decision hash,
selected HTTP-attempt hash, raw response/capture identity and measured clock
chain. This certificate audits an already completed DATA barrier; its own write
is not backdated as that barrier and it does not select a newer OI vintage.
It must be present and validated at future model initialization. An absent or
uncertain certificate disables new BO. A crash leaving only an offline data
draft is explicitly `unqualified_existing_decision`, not a recovered success.

**Consumer rule:** use `collector::load_qualified`, not an offline
`DailyDecision::allow_new_bo()` alone. Missing, corrupt, late or unqualified
evidence means BO-disabled for that session. The future adapter must freeze that
choice once at session initialization, never enable BO mid-session after a late
certificate. MR, BO observation, existing-position management and exits continue.
This gate does not override signals, MR priority, no-overlap or other readiness
requirements. Offline-fixture certificates are rejected by the production loader.

`attempts/<date>/<n>.reserved.json` is persisted BEFORE a request. Default budget
is four attempts with 30-second retry delays; restart cannot reset the count or
silently change that policy. A reservation with no completion consumes an
attempt, preserving crash evidence. Source errors retain their records, and a
retryable outage is not converted into an early successful/missing freeze.

A store-wide OS file lock prevents concurrent collectors and is released by
process exit/crash. Frozen qualified results are reused byte-for-byte without
network access. Later revisions cannot overwrite them. Failures are machine-
readable statuses and the CLI exits nonzero; valid zero/positive OI is an ordinary
successful snapshot with BO disabled, not a transport alarm.

### Resource Budget And Invocation

Default dedicated-store limits: 64 MiB, 4096 filesystem entries and 256 MiB free
disk, plus a conservative 3 MiB/12-entry reservation before a request. The scan
is bounded and rejects symlinks/special files. Budget exhaustion disables new BO
and emits an explicit status. Nothing is automatically deleted or trimmed; never
remove the current day's reservation/capture/decision records to reset a budget.
The OI root must be separate from Redis, strategy snapshots and other services.

```sh
cargo build -p strategy-runtime --bin oi_shadow_capture \
  --features shadow-oi-http --locked --offline
target/debug/oi_shadow_capture plan /absolute/reviewed-config.json
# Only after explicit capture/host authorization, inside the dated window:
target/debug/oi_shadow_capture collect /absolute/reviewed-config.json
```

The bounded JSON config requires `calendar_path`, reviewed `calendar_pin`,
`decision_date`, `model_sha256`, absolute `store_root` and `policy` matching the
typed fields. `plan` only validates/pins the calendar and prints source dates;
it does not create a store, sample host NTP or issue HTTP. A scheduler/launcher
must later supply dated reviewed configs; no timer or continuous restart loop
was installed. PREVOPENPOSITION/current-marketdata diagnostics are not yet
collected and are never a fallback for missing dated history.

These local fixtures and a network-disabled Linux smoke are not future PIT
coverage, historical publication proof or permission to start the three arms.
The twenty-session observation count has not started for this implementation.

## Verification

- 26 Rust calendar/selection tests, including 38 Python-reference cases.
- 20 OI validator/store tests, including deadline, restart, tamper and writer race.
- 25 collector tests: transport limits, interruption/resume, durable deadline,
  clock uncertainty, unsupported/stepped clocks, qualification, locking and quotas.
- Isolated Linux/amd64 Docker smoke: all 45 OI/core+collector tests passed with
  networking disabled, no capabilities, read-only source and disposable targets.
- 8 Python baseline/export-tool unit tests and an actual frozen-input audit.
- Strategy-runtime library with `shadow-oi-http`: 604 passed, 17 explicitly ignored.
- Clippy/all targets passes both feature-on and feature-off; reqwest is absent
  from the feature-off runtime's normal dependency tree.
- The native CLI builds and its synthetic-fixture `plan` smoke succeeds without
  creating a store or issuing HTTP requests.
- Existing context-core tests: 11 passed, one explicit export ignored.
- Existing numerical-reaction tests: 11 passed, two explicit exports ignored.
- Scoped clippy with warnings denied, formatting and diff whitespace checks.

The original calendar and two pure session functions were executed in an
isolated pandas 2.2.3 environment. Calendar/reference success is not full historical
model event/state parity or receipt certification. External WS recovery, the full
workspace/Redis integration suite and prospective soak are not claimed here.
Ignored evidence directories are
`private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-e1-e2/` and
`private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-http/`.
The two earlier Docker launch failures (rustup writable-home expectation and
tmpfs noexec) are retained separately; the final pinned-toolchain/exec-tmpfs
Linux run passed. No real HTTP request or healthy-VPS-clock claim follows from it.

## E3 Component Feature Foundation

`strategy-runtime/src/shadow_component_context.rs` builds an immutable per-session
value for IMO_AA, IMO_AB and IMO_AB_OI. It is exported as an opt-in library API,
not invoked by existing workers, and does not modify `shadow_reaction`, original
replay engines, strategy kinds, current runtime configuration or H4/W4.

- MR always selects A. BO selects A for IMO_AA and B for IMO_AB/IMO_AB_OI.
  Both components retain independent previous/previous2 dates, OHLC/range,
  main-session range diagnostics, return and entry/stop scales. Primary scales
  remain the full previous-session range; no new model tuning was added.
- Input is a bounded, ordered, unique canonical receipt prefix bound to the
  declared stream, hash, sequence frontier and completeness barrier. This checks
  internal consistency, NOT the operational authority of a caller's attestation.
  The later bridge must bind the actual source manifest and canonical M10 feed;
  matching timestamp labels alone cannot certify OHLC aggregation provenance.
- Receipt and durability must be strictly BEFORE the calendar opening cutoff.
  Delayed first-event processing does not relax it. Source appends after cutoff
  cannot alter model features; their enclosing audit/barrier metadata may differ.
- Service bars are excluded from features using the dated grid, while DSVD 09:50
  remains eligible B context. Incomplete/unknown selected days block dependent
  readiness instead of choosing an older good anchor. Missing BO dependencies
  block combo readiness without hiding the independently ready MR trace.
- Session trace includes calendar kind, expected/observed/required counts,
  missing/ignored labels and per-component selected/older/excluded disposition.
  Anchor provenance includes original IDs/sequences, receipt/durability bounds
  and a hash of the exact selected rows.
- Numerical fixtures are explicitly marked and may not masquerade as observed
  input. A complete zero-range session stays data-ready but does not satisfy a
  positive-range signal predicate. OI does not alter either price feature set.

Eleven focused synthetic unit tests pass locally and in a network-disabled,
read-only-source Linux/amd64 container. This feature-only slice did not establish
engine-state parity; the separate numerical slice below now supplies synthetic
`expected_state_trace` checks. Full historical acceptance and once-only
operational freeze/persistence remain downstream integration responsibilities.
No existing signal, fill, EOD or bracket contract changed.

Full runtime library regression after this slice: 604 passed, 17 explicitly
ignored. The four protected baseline files and 112/107 frozen reference/dependency
checks still pass. Evidence is retained in ignored
`private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-component-context/`.

## E3 Isolated Numerical Reaction Engine

`strategy-runtime/src/shadow_component_reaction.rs` is a separate opt-in library
path. It has no Strategy adapter, broker commands, Redis or HTTP access. Existing
`shadow_reaction`, replay workers, live strategy sources and H4/W4 are unchanged.

- Independent MR A and BO A/B close/range/return are used in both entry and exit
  conditions. The Author41 short branch keeps its two-entry total quota, morning
  entry cutoff, stop-before-close-take precedence, breakeven and 20:00 time exit.
  A morning entry cutoff is NOT a forced morning position close.
- Shared-slot MR-first/no-overlap, pending next-open fills, BO side quotas,
  wait-three-hours, K=0.53 and frozen stop conditions are preserved. Daily/sixth-
  bar BO observation precedes ownership, MR and OI early returns. Observer-only
  author eligibility is not accidentally turned into a hybrid BO trading ban.
- OI-off is the control. A fixed numerical allow/deny switch is accepted only
  for the explicitly marked IMO_AB_OI fixture arm. Denial affects new BO entries,
  not MR, observation or exits. This is NOT the real qualified-OI bridge and
  cannot substitute for `collector::load_qualified` at future admission.
- Each closed M10 input must be the next expected label, valid and observed no
  earlier than its close; processing clocks cannot regress. Invalid/gapped input
  is transactional and cannot consume a pending entry/exit. The kernel currently
  accepts full consecutive weekday 07:00/09:00..23:50 execution grids only.
  Shortened/break grids fail explicitly pending a separately matched execution
  contract; they do not get an invented next-open fill or flatten rule.
- BO EOD remains 23:30 signal -> 23:40 next-open fill, with no late new entry.
  `is_complete()` additionally requires every expected bar and a flat final
  state; being flat earlier is not a complete session. Numerical completion is
  not an operational clean-EOD certificate or a prospective session count.
- Research friction is separate from production fee components: quantity one,
  point value 10, per leg `1 + price * 10 * 0.000066 + 0.5 * 10` RUB. The x2 stress
  doubles total friction, not prices. These values come from the pinned research
  cost function/protocol, not the old diagnostic template commission.

### Reference And Recovery Checks

`scripts/export_component_reaction_reference.py` pins seven original
calendar/session/compiler/model/protocol sources. It executes the exact selected
compiler/function bodies, supplying only the model-directory path dependency
without research cache/plot imports. Original compiled loop predicates and control
sites are instrumented for observation, not replaced by handwritten expected
signals. Unknown function/compile/control-site inventories fail the exporter.
For every admitted case, events and trades are also checked against the same
compiler WITHOUT trace instrumentation before writing expected values.

The synthetic fixture contains 50 cases: A/A, A/B, OI-off, fixed allow/deny,
flat-range context, missing DSVD, MR/BO ownership transitions, both BO sides,
quotas, EOD suppression, ambiguous MR stop/TP and breakeven. Four incomplete-
context cases HOLD rather than becoming empty successful sessions. The remaining
46 cases produce 4,646 per-bar states, compared across 28 fields including
counters, BO observation, position/pending owner, predicates and decision
suppression. Anchors/scales, dates, events, fills, exit reasons, trades and costs
are checked separately; integer/enum values are exact, float tolerance is
absolute 1e-9.

The versioned bounded checkpoint binds the ORIGINAL frozen component-context
hash and gate, then retains the input prefix and a state/event/trade witness.
Restore reexecutes that prefix and compares the witness; a matching envelope hash
alone cannot bless forged state. Restart tests cover pending entry, held position,
pending exit and EOD without duplicate fills/events. The future host must durably
retain that original context, not recompute it from revised history at restart.
This API does not yet provide the host filesystem/Redis transaction boundary.

```sh
/path/to/pandas-2.2.3-venv/bin/python -B scripts/export_component_reaction_reference.py \
  --research /path/to/analiz_alpha_si \
  --output /new/path/component-reference.json
cmp strategy-runtime/tests/fixtures/component_reaction_reference_v1.json \
  /new/path/component-reference.json
cargo test -p strategy-runtime --lib shadow_component --locked --offline
```

Both this fixture and the earlier calendar fixture reproduce byte-for-byte.
The new exporter has four stdlib unit tests, in addition to the existing eight
baseline/calendar-export tests. Seven new kernel tests and eleven feature tests
pass both natively and in an isolated Linux/amd64 Docker run with no network,
read-only source, no capabilities and disposable targets. The complete runtime
library run with `shadow-oi-http` passes 611 tests, with 17 explicit ignored
exports/integrations. Clippy/all targets passes with and without that feature;
formatting and whitespace checks pass. E0 still verifies all 112 references,
107 dependencies and four protected existing sources read-only. Full logs and
reproduced fixtures are retained under ignored
`private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-reaction/`.
No research sources/results, actual OI capture, scheduler, VPS source or worker
was modified. That synthetic suite alone is NOT historical model parity or GO;
the separately scoped historical evidence is described below.

## E3 Frozen Historical Parity

Status: `HISTORICAL_NUMERICAL_PARITY_PASS_NOT_PIT` for the declared frozen
IMOEXF Author41-short/BO053 reaction contract. This is not a live High180 parity
claim, not historical receipt/publication proof and not deployment permission.

`scripts/export_component_reaction_history.py` loads the exact pinned
`sources/read_bars/raw_python` function bodies without research import side
effects. The six price vintages, source manifest, protocols, compiler and existing
frozen trade/event/OI-gate tables are hash-checked before execution. Inputs are
rechecked before writing the final completeness record. Nothing is downloaded,
updated or silently resealed. The exporter also checks every reproduced Python
trade/event/cost against those saved tables, not just another fresh model run.

| Control | Planned Regular Dates | Numerically Ready | Trades | Per-Bar States |
| --- | ---: | ---: | ---: | ---: |
| IMO_AA, 2026-03-23..2026-11-13 | 167 | 165 | 241 | 15,741 |
| IMO_AB, same full interval | 167 | 165 | 258 | 15,741 |
| IMO_AB_OI_OFF, diagnostic A/B equality control | 167 | 165 | 258 | 15,741 |
| IMO_AB_OI_LAG1, frozen 2026-07-20..2026-11-13 cohort | 84 | 84 | 96 | 8,484 |

The total is 585 arm/date cases, 579 ready cases and 55,707 per-bar state rows,
with zero detected differences. The OI-off control is deliberately duplicated;
853 compared trade rows are NOT 853 independent economic observations. The six
HOLD cases are the same 2026-03-23/24 context-warmup dates across three controls,
not six prospective failed sessions. No current-day-grid exclusion occurred in
this frozen interval. Missing data/context would be shown separately, never
counted as an empty successful day. There is no shortened/break execution day in
this declared interval; the kernel's explicit unsupported-grid guard remains.

The ignored opt-in Rust test compares independent source dates, H/L/C/range/
return/scales, all 28 state fields, events, fills, reasons and x1/x2 costs with
the synthetic suite's exact enum/integer and absolute 1e-9 float rules. It checks
the full dated control inventory rather than accepting arbitrary omitted cases,
file bounds and the final JSONL prefix hash. A missing footer/truncated case list
cannot be called complete. Restart replay covers the sixth-bar observer, first
pending entry/exit and final state of each ready case. Every ready case ends flat
after its complete same-day grid. A/B equals OI-off in state/events/trades/costs;
the final-history lag1 gate preserves MR trades/costs and BO observation.

### Reproduce Privately

```sh
python3.12 -m venv /tmp/alor-component-history
/tmp/alor-component-history/bin/pip install \
  pandas==2.2.3 numpy==2.1.3 pyarrow==18.1.0
/tmp/alor-component-history/bin/python -B scripts/export_component_reaction_history.py \
  --research /path/to/analiz_alpha_si \
  --output /new/private/path/component-history.jsonl
ALOR_COMPONENT_HISTORY_REFERENCE=/absolute/private/path/component-history.jsonl \
  cargo test -p strategy-runtime --lib \
  shadow_component_reaction::tests::history::historical_component_reference_event_state_parity \
  --locked --offline -- --ignored --exact --nocapture
```

The full qualified test name is intentional: combining a short filter with
`--exact` runs zero tests and is NOT a successful historical check. The expected
historical invocation must report `running 1 test` and `1 passed`. Keep this
11-MiB final-history artifact, price data, logs and any economic/operational
retrospectives under ignored private evidence, not public fixtures or the
engineering-only distribution. The small synthetic fixtures remain portable.

Evidence directory:
`private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-history/`.
The new history exporter adds four stdlib tests, plus a default Rust inventory
test; the historical check stays opt-in and cannot read external prices during
ordinary CI. Final verification: 16 Python tool tests pass; the full runtime
library with `shadow-oi-http` passes 612 tests with 18 explicit ignored checks;
both feature-on/off all-target Clippy and formatting/whitespace checks pass.
The final Linux/amd64 run passes all 20 component tests INCLUDING the historical
check, using no network, read-only sources/evidence, no capabilities and disposable
build files. Native and Linux historical counts match. Calendar, synthetic and
historical exports reproduce byte-for-byte in the documented fully pinned
pandas/NumPy/pyarrow environment. E0 again verifies 112 references, 107 dependencies
and four protected files unchanged. Live, old shadows, USDRUBF H4/W4, sources,
schedulers and VPS state were not touched by this local work.

### Limits Before Integration

- Historical receipt/durable clocks are explicitly numerical Replay evidence.
  Neither a synthetic prefix nor a final daily OI value proves publication before
  that day's cutoff. The OI gate table is a final-history lag proxy, not PIT.
- The reference's full-current-day coverage mask is ex-post numerical eligibility,
  not an invented pre-open readiness check. Actual source freshness, incremental
  missing-bar handling and immutable freeze/checkpoint durability still need a
  separately reviewed operational bridge and fault tests.
- Next-open times/prices are frozen virtual-fill semantics, not proof that a
  live order would have filled at those prices. Operational journals must retain
  model and actual receipt/processing clocks separately; no command is backdated
  or authorized by a numerical fill.
- Broader older/exceptional execution calendars, actual host clock/filesystem/ISS
  qualification and prospective resources/isolation/admission are not certified
  by this replay. Twenty clean VPS sessions have not started for these new arms.

## E4 Local Session Bridge, 2026-11-16

`shadow_component_session` adds a new opt-in, explicitly dated three-arm session
API on the existing owned receipt journal and generic fsync/HEAD transaction log.
It is not a running worker, scheduler, source admission or broker route.

Implemented:

- A copied/pinned calendar and exact source/spec identity, original sealed local
  barrier and actual receipt clocks; no mutable history re-selection on restart.
- Canonical first-receipt projection plus separate raw revision audit. Normal
  nightly duplicate history does not rerun models. Before freeze, affected
  selected model fields hold the matched group; service/volume/nonterminal
  context-close changes and irrelevant previous2 fields stay audit-only.
- One-time qualified OI bundle with complete offline revalidation on recovery.
  Missing/invalid evidence closes new BO in the OI arm only. Later external
  arrival, replacement or deletion cannot change the day's committed choice.
  OI-input validity is reported separately from gate allow/deny and price-grid
  completion, so an unknown OI day is not a fake clean paired-comparison day.
- Atomic staging of three engines and stable-ID incremental outbox records,
  released only after the shared transaction HEAD. Fault/restart tests include
  pending entry/exit, one-arm failure, torn tail, parallel writer, changed spec,
  rehashed witness, missing/stale bar and repeated/updated quotes.
- Observed OI gates cannot be minted from checkpoint JSON. The isolated component
  checkpoint is now v2; experimental v1 inputs fail explicitly. This is not a
  change to any deployed strategy snapshot or the original OI archive format.

The precise trust boundaries, budgets, quality fields, recovery rules and work
remaining before a VPS worker are in
[the session bridge contract](imoexf-component-session-bridge-contract-2026-11-16.md).
Controlled test clocks/HTTP bodies do not prove real host NTP health or ISS PIT
coverage. No actual HTTP capture, Redis poll, VPS restart, live/shadow config or
scheduled job was performed by this development slice.

Ignored evidence directory:
`private_docs/imoexf-calendar-oi-stage1-2026-11-15/verification-session/`.
The directory follows the original stage date; this continuation is 2026-11-16.
Twenty new prospective regular sessions have NOT started for these three arms.

Verification of the final local slice:

- 12 session bridge tests, including three controlled OI integration cases.
- Two qualification-bundle tests and an observed-gate/checkpoint capability test.
- Full runtime library with `shadow-oi-http`: 627 passed, 18 explicitly ignored.
- Feature-off component suite: 29 passed, one explicit historical check ignored;
  all-target Clippy passes with and without the feature.
- Final Linux/amd64 network-disabled, read-only-source/evidence run: 80 passed,
  INCLUDING the full historical replay, all component and OI tests. Earlier
  78-test Linux output is an intermediate slice, not the final inventory.
- Historical parity is unchanged: 585 arm/date cases, 55,707 states and the same
  frozen trade/event/cost counts. No signal, fill, EOD or cost retune was needed.
- Formatting/whitespace checks pass; E0 still verifies all 112 references,
  107 dependencies and four protected existing files read-only.

Native library/workspace integration distinction remains explicit: this is not
a run of every workspace/owned-Redis integration or real VPS/NTP/HTTP validation.
The generic old runners/storage/ingress, live sources, gateway and BarEvent schema
are unchanged. Only the new opt-in modules and their pure OI-reader abstraction
participate in this bridge.

## Operational Prerequisite Check, 2026-11-16

A separate read-only VPS check found the existing live/legacy-shadow processes
healthy, but the previously planned strict IMOEXF/USD model-pair admission did
not start its workers. Evidence staging used umask-sensitive directory creation;
the actual model UID could not traverse the staged evidence. This is outside
the new calendar/OI numerical/session bridge, not a model-parity failure.

A narrow local staging/secret-safe diagnostic candidate and Linux cross-UID
regressions are documented in
[the evidence permissions hardening note](shadow-admission-evidence-permissions-hardening-2026-11-16.md).
No host permission, source, approval, worker or live process was changed. The
failed admission and any later audit require explicit review before the next
safe dated launch. Do not count that failed attempt as a prospective clean day
or infer operational readiness from the new bridge's local test results.

## E4 Dated Read-Only Driver, 2026-11-16

`shadow_component_session::coordinator` adds an explicitly dated local driver
around the existing journal/session engine. It reuses read-only polling, validates
the pinned source manifest/health and Linux process identity, and durably binds
the entire accepted Redis envelope prefix. No shared source/BarEvent/old worker
format was changed.

Collection and model advancement are separated by source revalidation and queue
expiry. Pre-open readiness has its own durable, plan-bound witness. A fresh
post-cutoff poll is still mandatory after restart; no volatile caught-up flag or
old cutoff is accepted as a new source check. Missing required bars exhaust a
deadline even if the source stream returns no new records. Source reconnection
can recover in the same process, but identity changes and accepted-prefix mutation
fail the experimental generation without repair or an extra model decision.

The source-prefix WAL records full-envelope hashes, not only OHLCV or the last
cursor. Owned-Redis tests catch both an early quote mutation with an unchanged
tail and an envelope-only publication-time mutation after process recovery.
This complements, rather than changes, the inherited durable raw receipt journal.

This is not a running daily service. Exact reviewed pre-freeze audit hashes are
temporary conservative pins; the permanent model-dependency classifier is NOT
implemented by this driver. Valid OI and complete price-grid quality remain
separate; neither grants operational admission or starts a VPS observation count.
The session bridge contract documents the new limits and source/clock boundaries.

Ten focused native tests passed, INCLUDING five owned Redis integration tests
normally marked ignored. The fixture server's PID is checked and its reader ACL
denies writes/unrelated streams; no REDIS_URL or VPS Redis is used. Test clocks
and source-process callbacks are controlled, not a real Linux MD qualification.
The earlier 80-test Linux run predates this driver; it must not be cited as
qualification of these new files. All-target feature-on Clippy passes.

Final focused driver verification: ten native tests, including all five owned
Redis cases, passed. Full feature-on runtime library: 632 passed, 23 explicitly
ignored; ignored integrations are not silently counted as passes. The historical
reference was rerun explicitly: 585 arm/date cases, 55,707 states, same events,
853 compared trade rows including the deliberately duplicated OI-off control.
This remains numerical final-history parity, not PIT or a prospective result.

A NEW Linux/amd64, network-disabled/read-only-source build passed six driver
unit tests, including the actual boot/start-ticks process pin check. Its five
owned-Redis integration tests were explicitly ignored because that container
does not contain redis-server; their native owned-server run above passed.
This is not actual Linux MD/source/sandbox or full Linux Redis qualification.
Build artifacts lived only in a bounded temporary filesystem; no VPS runtime
image or background service was built/updated by this verification.

Separately, the storage-permission helper was qualified on the target host in a
STANDBY directory, including real-model-UID read/no-write fixtures. The active
controller and all protected processes remained unchanged; no timer or grant was
installed. A fresh normal start also needs an explicit reviewed source-date reuse
binding, specified in
[the recovery contract](shadow-normal-source-reuse-admission-contract-2026-11-16.md).
Do not reschedule the expired one-shot or use diagnostic late-start to hide the
old paired guard's source/model date mismatch.

## Next Slices

### Collect-Only Standby, 2026-11-20

An independently scoped Linux build now installs only the OI collector and
dated Python job, not the experimental three-arm coordinator or a model worker.
The shared runtime/live/shadow processes remain unchanged. Native calendar/OI
tests passed 26/51; eight Python job tests passed locally and under the target
nonroot UID. All twenty dated plans validate. The bounded timer is installed
DISABLED and has no catch-up behavior. No prospective PIT session is claimed.

A one-shot non-PIT transport probe under the target systemd sandbox received
HTTP 200 with a valid dated body and current HTTP Date. Its native kernel clock
reported max_error above the existing 1000 ms limit, so the overall probe failed
closed and the collector remains `STANDBY_CLOCK_HOLD`. No production qualification
or morning snapshot was created. The runtime clock and deadline guards were not
relaxed, and the current 20-session model counts were not changed.

The operator approved lowering the existing NTP poll ceiling to 128 seconds
ONLY after verified flat maintenance. That global clock-service operation is
not performed while positions are open. Then repeat native clock/probe and
protected-process checks before enabling the collector timer. This does not
authorize a different time server or manually setting/stepping the host clock.
See [collect-only runbook](oi-shadow-collect-only-runbook-2026-11-20.md).

**Update, 2026-11-21:** the authorized flat-only poll change has now completed.
The existing NTP provider is unchanged; no manual clock setting was used.
Five native clock readings across multiple ordinary poll cycles passed the
original bounds, followed by a successful full nonroot/systemd launcher probe.
Protected trading/source/model process inventory was unchanged. The bounded
collect-only timer is enabled, next run 2026-11-23 06:45 Europe/Moscow,
Persistent=false. Status is `ARMED_COLLECT_ONLY / AWAITING_FIRST_PIT_CAPTURE`:
production store is still empty, the diagnostic probe is not a PIT snapshot,
and none of the future three model arms has been admitted.

The dated-driver E4 gates below still apply to future MODEL admission, not to
the separate collect-only installation.

The older paired-worker source-date reuse gap is now addressed separately by an
opt-in Python v4 plan/guard/grant/launcher with explicit pre-open resume. Old
strict defaults are unchanged. Native/Linux compatibility suites and real
model-UID target preparation passed; new dated pair units are registered but
INACTIVE, with no grant/state/timer. The next initial admission still requires
the actual previous EOD and fresh reviewed context, not a copied old approval.
This operational fix does not activate the experimental calendar/OI driver,
count a prospective day or implement the permanent P1 dependency classifier.

1. Prepare a separately reviewed prospective dated registry and validate the
   actual feature/frontier provenance. Do not activate this API inside an old
   profile merely because synthetic reference fixtures pass.
2. Review the collector/qualification contract and validate actual host clock,
   filesystem and a separately authorized real ISS capture. Prepare a dated
   launcher and monitoring; scheduler installation still requires separate
   authorization. Do not treat local fixtures as prospective PIT coverage.
3. Preserve the now-verified frozen historical event/state and OI-off controls.
   If a broader historical window is later requested, explicitly match its
   exceptional execution grids instead of silently excluding or filling them.
4. Complete Linux owned-Redis/source/sandbox qualification, the multi-day
   coordinator, source-audit classification, resource/isolation/admission and
   actual host/source fault checks. The library driver is not an admitted worker.
5. Prospective observation occurs only on the VPS after explicit deployment
   authorization. The existing paired-shadow admission is not that authorization.
