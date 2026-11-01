# Shadow State Parity Gate

For current service admission, observation and future live migration use the
[engineering handoff](../../docs/shadow-observation-and-live-promotion.md) and
[supervisor runbook](../../docs/reaction-shadow-supervisor-runbook-2026-10-31.md).
The dated numerical implementation checkpoints below are historical scope and
test-method notes, not current release certification or prospective soak results.
Generated artifacts and private input bundles are not part of this publication.

Offline engineering tools for the frozen IMOEXF Author41 reaction07 MR,
MR-first/BO053 combo and USDRUBF previous-day/VWAP reaction shortlist.
They do not connect to Redis, publish commands or change
deployment configs. The first scope is numerical state comparison, not live
receipt/restart validation and not broker bracket equivalence.

## Reference Export

Run from the Rust workspace with Python 3.9+ and the frozen handoff's pandas/numpy
dependencies. `HANDOFF` is the original `engineer_handoff` directory; `OUTPUT`
must be a NEW directory outside that bundle.

```sh
python3 scripts/shadow_state_parity/export_imoexf.py \
  --handoff "$HANDOFF" --output "$OUTPUT"
```

The exporter verifies the entire frozen manifest and pins the reference model
SHA256. It adds observer calls to an in-memory AST copy at reviewed source sites:
no trading condition, state assignment or execution expression is replaced.
A changed reference hash or unknown branch fails closed and requires review.
This avoids editing the freeze or maintaining a second copy of its signal engine.
The reference and verifier are compiled in memory rather than imported through
caching file loaders, so Python cannot create bytecode files inside the freeze.
The observed replay must exactly reproduce the uninstrumented trades/events;
the latter must also pass the existing frozen CSV verifier.

Each of `mr/` and `combo/` contains `expected_state_trace.jsonl.gz` and a manifest.
`verification.json` is written only after both arms finish and pass. It marks
reference integrity, NOT Rust state parity. A partial directory from a failed
run is not an accepted bundle; use a new output path for a rerun.

Gzip headers are deterministic. The manifests bind model/config/feed/execution
identity, reference/input/exporter/comparator hashes and every ordered input.
Private full-history traces belong beside research artifacts, not inside a
public deploy repository or the original handoff. No original file is rewritten.

## State Contract

`compare.py` defines separate typed `imoexf_reaction_state_v1` and
`usdrubf_reaction_state_v1` schemas. The IMOEXF schema is unchanged. Each input has
`bar_start -> [pending_applied] -> observed -> bar_end`, with contiguous event
sequence numbers. All no-trade/suppressed inputs are present. `observed` is after
updating running observations but before position/entry decisions; it advances
even while MR owns the shared slot. `bar_end` includes any newly scheduled action.

The trace includes previous/previous2 dates and context, range/scale, provenance,
MR/BO counters, BO observations, position/pending ownership, held bars, derived
rule levels, logical decision keys, raw price conditions and the taken branch.
`suppression_reasons` records the executed blocking branch, not an invented trade
or proof that every blocked branch had a valid raw candidate. Identity carries
the full frozen parameter hash. Unused Author42-only sixth-bar levels/calendar
flags are intentionally excluded: this is Hybrid BO053, not Author42 BO.

`levels` are deterministic diagnostics derived from the same reference context,
not evidence of broker TP/SL orders. Pending decisions never contain a future
fill price. Decision IDs are model keys, not broker request IDs. Next-open proxy
becomes known when the subsequent complete M10 input is processed, represented
by `entry_proxy_known_ts_local = entry_bar_label + 10m`. Logical action time and
proxy-known time therefore remain distinct; no real arrival latency is invented.

All local timestamps are Europe/Moscow bar-BEGIN labels. `decision_ready_ts_local`
is label + 10m. Prior context uses the frozen numerical reference; the stated
context cutoff is 09:00 before the schedule transition and 07:00 afterwards.
It does NOT certify that the inputs were durably received before that cutoff.
Receipt and processing timestamps are null for `numerical_history`; this schema
rejects fabricated receipt values. Operational cutoff/restart fixtures need a
separate schema, a durable ingestion barrier and persisted original provenance.

## USDRUBF Reference Export

`USD_HANDOFF` is the frozen `usdrubf_previous_day_shadow_selection_2026_10_30`
directory. The exporter pins both reference source and full contract hashes,
validates the complete file manifest, and preserves the outer replay wrapper's
complete-session/missing-execution-bar assertions.

```sh
python3 scripts/shadow_state_parity/export_usdrubf.py \
  --handoff "$USD_HANDOFF" --output "$USD_OUTPUT"
```

Each frozen `model_id/` gets its own compressed trace and manifest. All SIX
profiles are checked offline: Previous-day 07 H3/W4, 07 H4/W4, 09 H3/W2,
VWAP07 H3/W2 reaction control, Previous-day07 H3/W2 attribution control and
the optional VWAP09 reaction control. Exporting six profiles is not a decision
to deploy six services. The work plan requires the H3/W2 attribution control.

The state distinguishes yesterday's frozen MR/BO anchor, range and return from
current-session VWAP, open, running range, volume and numerator. Feature capture
reads the reference's own intermediates. In this vectorized reference the
current COMPLETE bar's prefix features already exist at `bar_start`; that phase
is pre-pending-application, not pre-bar-arrival. Observation count is therefore
`i + 1` in all phases, including while MR owns the position. Only current and
prior sessions are projected; no future session context is included.

Position state contains captured signal scale/anchor and actual rounded MR
stop/take, separately from the currently recomputed features. Those levels are
reaction thresholds, NOT working broker brackets. Both-touch MR bars select
stop before take; exit price remains the next open. Pending entries contain
signal-time scale/anchor but never a future fill price or future MR TP/SL.
BO stop1/stop2 fields are exported as stored by the reference, including its
unused BO diagnostics on MR positions; they do not participate in MR exits.

`mr_scheduled_total` is an observer audit counter of accepted entry schedules,
not a new model limiter: `mr_limit=null`. USDRUBF must not inherit RI/IMOEXF's
two-entry MR quota. BO used-side flags come directly from the model's `used`
state and change at entry application. Windows explicitly distinguish H3/H4,
W2/W4 and the pre-transition 09:00 clock.

The reference's missing-prev2 return fallback of zero is preserved and labelled
`warmup_return_fallback`; `two_prior_sessions_available` is separate. This does
not approve operational trading before the two-session readiness gate. Negative
volume is clipped to zero by the reference, and zero cumulative volume falls
back to current close. These semantics are tested, not silently redefined.

Manifests also bind the raw input hash/count and excluded raw input IDs/hashes.
The state comparator covers every PREPARED model input, including warmup and
no-trade bars. The excluded-input list is a filter audit, not runtime receipt
evidence or proof that Rust selected the same raw feed. A Rust harness must
separately verify raw selection before comparing states. This generation is
weekday-only; weekend-context variants still require their own reference and
operational cutoff fixtures.

USDRUBF full-history acceptance on 2026-10-30 retained 629 frozen trade rows and
1,258 events across six profiles, with exact instrumented/uninstrumented output
equality. There are 8,056 inputs per 07 profile and 7,120 per 09 profile;
140,682 state rows in total. These are overlapping experiment arms, not unique
portfolio trades or prospective soak observations.

## Candidate Comparison

The Rust exporter must project its OWN state at the same boundaries. Do not
derive candidate state from reference trades, expected rows or final PnL.
Validate the shared input-file hash before replay and carry the manifest's input
identities through the harness; do not invent a different floating-point JSON
serialization for input/provenance hashes. Record the candidate Rust source,
binary and config hashes in its own run metadata. `reference_sha256` identifies
the compared freeze, not the Rust implementation or a reviewed deploy revision.

```sh
python3 scripts/shadow_state_parity/compare.py \
  --manifest "$OUTPUT/combo/manifest.json" \
  --expected "$OUTPUT/combo/expected_state_trace.jsonl.gz" \
  --actual "$RUST_ACTUAL_TRACE"
```

Only schema-declared numeric price/scale fields allow absolute tolerance 1e-8.
Counters, booleans, enums/strings, IDs, order and timestamps compare exactly.
Missing/extra fields, duplicate JSON keys, non-finite numbers, missing/extra
inputs and incomplete/reordered phases fail. Coverage comes from the input
manifest, so two equally truncated traces do not pass. Failures identify the
first row/input/field. Keep both traces for diagnosis.

A self-comparison checks artifact integrity only. A numerical candidate PASS
does not establish broker isolation, operational receipt/restart behavior or
permission to deploy. Those are separate acceptance gates.

## Independent USDRUBF Rust Replay

The new `usdrubf_shadow_replay` binary computes the six frozen profiles directly
from raw M10 OHLCV. Its model is isolated under `src/bin/usdrubf_shadow_replay/`:
no runtime-host registration, Redis client, command route or live adapter calls.
This is an offline numerical implementation, not a deployed shadow runner.

```sh
cargo run --offline -p strategy-runtime --bin usdrubf_shadow_replay -- \
  --handoff "$USD_HANDOFF" --reference-state "$USD_OUTPUT" \
  --output "$RUST_OUTPUT"

python3 scripts/shadow_state_parity/verify_usdrubf_rust.py \
  --handoff "$USD_HANDOFF" --reference-state "$USD_OUTPUT" \
  --candidate "$RUST_OUTPUT"
```

`RUST_OUTPUT` must be new, with an existing parent, outside both immutable input
directories. Source/contract/raw-file hashes are verified. Rust selects its own
weekday/session bars and validates all accepted AND excluded input identities
against the manifest before calculating features. Full sessions and consecutive
execution bars are required. Raw labels are naive Europe/Moscow, not UTC.

The reference manifest supplies only checked input identities and provenance
hash material. The Rust model never reads expected state rows, signals, features
or trade CSVs. Yesterday/previous2 context is accumulated from completed sessions;
VWAP uses compensated prefix summation consistent with the frozen reference.
Captures, thresholds, counters, ownership, pending actions, terminal guards and
trade economics come from its own state. No new broker bracket semantics are
introduced, and no numerical tolerance was widened to obtain a PASS.

Each profile produces `actual_state_trace.jsonl`, `trades.csv` and `events.csv`.
`run_metadata.json` binds source/binary/config/input and trace manifests. Its
`GENERATED_NOT_YET_COMPARED` status is deliberately not an acceptance claim.
The separate verifier checks every state row and all frozen outputs, then writes
`verification.json`; an existing verification report is never overwritten.
Missing profiles, changed inputs/manifests/traces and incomplete phase coverage
fail. Keep failed runs separate from accepted artifacts.

On 2026-10-30 all six profiles passed independent Rust/Python numerical state
comparison: 140,682 state rows, 629 evaluated trade rows and 1,258 events. Counts,
identities, branches, timestamps and ownership match exactly; declared floating
fields use the unchanged absolute 1e-8 tolerance. A second Rust run reproduced
the same trace hashes. Private artifacts are stored outside the deploy repository.

The numerical warmup preserves the reference's missing-prev2 return fallback;
it is NOT the prospective two-complete-session readiness policy. Durable receipt
cutoff, late-history quarantine, restart/dedup, prospective decision persistence,
independent consumer groups and zero broker publications remain separate gates.
This PASS neither activates a service nor authorizes replacing live VWAP/brackets.

## Independent IMOEXF Rust Replay

`imoexf_shadow_replay` implements only the frozen weekday Author41-short
reaction07 MR and MR-first/Hybrid BO053 combo. It is a separate offline binary,
not a new runtime strategy registration. Existing High180, Author41 live
brackets, Author42 and old replay executables are unchanged.

```sh
cargo run --offline -p strategy-runtime --bin imoexf_shadow_replay -- \
  --handoff "$HANDOFF" --reference-state "$OUTPUT" \
  --output "$IMO_RUST_OUTPUT"

python3 scripts/shadow_state_parity/verify_imoexf_rust.py \
  --handoff "$HANDOFF" --reference-state "$OUTPUT" \
  --candidate "$IMO_RUST_OUTPUT"
```

The output directory must be new and outside both immutable input directories.
Rust pins source/contract/cost-policy hashes, verifies the prepared input file
against both state manifests and independently validates every ordered M10
OHLCV row. It rejects weekend/service/unaligned labels, negative volume,
non-finite prices and duplicates. This input is already prepared, unlike
USDRUBF's raw-selection experiment; it does not certify a live ingress filter.

Prior context is accumulated only from completed sessions. MR uses previous
range divided by previous LOW, a maximum of two filled entries, inclusive
10:00 signal cutoff, high-touch stop priority, close-based take, 20:00 time exit
and breakeven after strictly more than 20 held bars. These are reaction signals:
even stop/breakeven executes at the next consecutive bar's OPEN, not at a broker
bracket price. BO waits three hours from the first actual model bar, uses CLOSE
stop conditions and gives the 23:30 EOD signal priority. Next-open entry at or
after 23:30 is suppressed. No forced 10:00 MR exit or Author42 gate is added.

Observations advance while MR owns the slot. Fill counters change only when a
pending entry is applied; pending state never contains a future price. An exit
signal cannot also schedule an entry. After the exit is applied at next open,
the completed execution bar may generate a new, later next-open entry, exactly
as in the freeze. Missing action bars or an unresolved day-end position fail
the run, rather than inventing a gap-flatten or discarding position state.
The numerical model preserves the reference's incomplete-context fallback;
it does not assert prospective two-complete-session readiness.

Each sleeve exports its own trace, trades and events, with source/binary/input
hashes in `run_metadata.json`. The separate verifier checks all state phases
and frozen outputs before creating `verification.json`. Pending receipts are
not fabricated: historical receive/process timestamps stay null.

On 2026-10-30 both sleeves passed: 26,171 MR and 26,287 combo state rows,
179 evaluated trade rows and 358 events. There are 8,679 inputs per sleeve;
warmup is traced, while trade/event economics starts on 2026-07-14. Counts
across sleeves are overlapping experiments, not unique portfolio activity.
Discrete fields match exactly and floats use unchanged absolute 1e-8 tolerance.
Repeated Rust runs reproduce the trace hashes. Private evidence lives in the
separate `imoexf_rust_v1` derivative bundle, not the frozen handoff or deploy repo.

## RI Legacy Handoff Preflight

`audit_ri_handoff.py` checks the old `ri_context_handoff` without changing it.
It validates the manifest-pinned RIU6/RIZ6 inputs and action streams, closing
fixtures and their selected gateway-availability evidence. It is a preflight,
not a state comparator or a new parity certification.

```sh
python3 scripts/shadow_state_parity/audit_ri_handoff.py \
  --handoff "$RI_HANDOFF" --output "$NEW_REPORT_JSON"
```

The report must be a new file outside the handoff. Exit **1** means failed
artifact integrity; exit **2** means the legacy artifacts are insufficient for
operational certification, even if their checked hashes and evidence match.
This tool deliberately has no successful operational-certification exit code.

The legacy producer sets ordinary `available_epoch = epoch + 600`; only selected
closing HistoryGap rows use rounded gateway receipt evidence. Neither field
proves local durable-before-cutoff acceptance. No complete source sequence,
durable barriers or independent expected state trace was included. A file named
`expected_state_trace` alone does not satisfy that requirement.

Keep the next two acceptance tracks separate:

1. **Historical numerical replay:** preserve the declared synthetic availability
   clock and build an independent reference state export for both policy arms.
   Include anchors, quotas, BO observation, ownership/pending and suppression
   branches. This does not reconstruct past real receipt/durability.
2. **Operational acceptance:** use new original durable ingress evidence and
   deterministic interruption/cutoff fixtures. Do not synthesize old `fsync`
   timestamps or import this legacy CSV as a production receipt journal.

The RI gates below now cover per-event branches and selected state independently,
including the declared historical numerical scope. Original receipt/durability
certification remains separate acceptance work.
The 236/176 legacy action counts are not a full paired-runner state-parity PASS.

## RI Independent Prospective Fixtures

`ri_reference.py` is a separate Python implementation of the CURRENT prospective
MR-first/no-overlap contract, with hand-computed boundary assertions. It is not
an instrumented copy of Rust and does not read Rust actions, states or prices
from expected trades. It is also NOT the April research combo that removes
overlapping intervals retrospectively. Do not label it frozen research parity.

```sh
python3 scripts/shadow_state_parity/run_ri_fixture_parity.py \
  --output "$NEW_RI_FIXTURE_OUTPUT"
python3 -m unittest discover -s scripts/shadow_state_parity -p test_ri_reference.py -v
```

The new output directory needs an existing parent. The command writes synthetic
inputs and expected states, runs the explicitly selected Rust test-only exporter,
then compares both arms. Rust receives ONLY the inputs and a new actual-output
path, never the expected trace. No receipt import API is added to a deployment.
Original handoff files, Redis, live configs and the broker are untouched.

Sixteen scenarios cover 376 post-event states and 94 diagnostic actions across
`runtime_accepted_only` / `ri_confirmed_closing_context_v1`: closing recovery,
missing previous2, strict MR thresholds, stop priority, shared two-entry quota,
21-bar breakeven, inclusive 10:00 MR entry versus 20:00 exit, sixth-bar BO levels,
MR priority over a pending BO, Friday and extreme filters, next-open stops,
three BO entries per day and terminal pending-entry rejection at 23:00.

The schema compares previous/previous2 OHLC/range/count, MR and BO counters,
captured position price/scale/held bars, phase and lifecycle owner/side/entry time,
pending role/side/reason,
BO observer count/first timestamp/extremes/levels, used-side flags, actual executed
branch sequence and prospective actions. Floats retain absolute tolerance 1e-8;
all other fields and order are exact. Missing/extra fields and rows fail. The
verifier regenerates coverage from all inputs, so equally truncated expected and
actual files cannot pass. Input/source/binary hashes are retained with the report.

Only `prospective_intent_suppressed` records count as actions here, never repeated
retrospective `shadow_path_active` records. Diagnostic PnL follows this adapter's
existing contract (MR cost 2 points, BO config cost 0), NOT broker economics or a
new cost2 claim. Stops retain the existing logical touch price; the fixture does
not predict actual market execution or propose TP/SL brackets.

Every scenario compares instrumented against uninstrumented Rust decisions/state
and performs witness restoration midway before continuing. Branch recording is
off in ordinary live/dry-run/legacy shadow instances; only the frozen facade
enables it and clears it on EVERY receipt, including context-only events. A branch
records the path actually taken, not proof of a valid price candidate behind
every blocked branch. Broker-specific deferred/reject paths are not exercised by
these assumed-fill fixtures and remain covered separately by runtime tests.

The frozen facade identity is now `ri_frozen_prospective_shadow_v2`: adding branch
data changes outbox/witness hashes. Old v1 generations must NOT be silently opened,
relabelled or reset. Preserve them for audit and select a new reviewed generation.
The local fixture status is `PASS_BOUNDED_SYNTHETIC_STATE`; that synthetic report
does not certify historical parity or original receipt/durability. Historical
numerical coverage has its own report below. Cutoff fixtures for weekend variants
and explicit operational rollout review remain required before shadow activation.

## RI Historical Numerical State

```sh
python3 scripts/shadow_state_parity/run_ri_history_parity.py \
  --handoff "$RI_CONTEXT_HANDOFF" --output "$NEW_RI_HISTORY_OUTPUT"
python3 -m unittest discover -s scripts/shadow_state_parity -p test_ri_history.py -v
```

The handoff is the original `live_baseline_revalidation/ri_context_handoff`.
The output must be NEW and outside that original revalidation tree. Source
manifest, closing evidence and hashes are audited before/after execution. The
command uses an explicit ignored **release** test, never a blanket `--ignored`.
It neither connects to Redis nor changes any runtime/service configuration.

`ri_history.py` applies the independent prospective reference continuously by
session, refusing to reset unresolved MR/BO/pending state at a boundary. Rust runs
the actual facade continuously and verifies full-prefix witness restoration.
Both policies retain the raw origins, weekends/service bars and mixed availability
clock from `RIU6_excluded_gap_input.csv` / `RIZ6_excluded_gap_input.csv`. The first
decision dates are explicitly 2026-07-03 / 2026-09-03 after two context weekdays.
Pre-transition sessions use 09:00/80 bars, post-transition 07:00/92 bars. Receipt
routing, observed/non-observed classification and post-event states/actions are
compared; a late closing input cannot rewrite the opening frozen context.

Accepted local result: **22,616 input-processing rows, 18,774 observed states,
820 actions across two contracts x two policies**, with no overnight/tail reset.
Scope is 54 RIU6 and 40 RIZ6 sessions per policy, with overlapping contract dates;
this is not a live portfolio performance series. A nullable state before the first
observation means warmup has routing-only coverage, not full internal state parity.
The old handoff action projection is checked secondarily, NEVER used to produce
the Python expectation: 234/234 and 174/174 corrected actions match after excluding
one explicitly reported pre-activation MR round in each contract.

`PASS_HISTORICAL_NUMERICAL_STATE` certifies this selected numerical schema, not
original receipts, the April research combo, broker execution or production GO.
The historical `available_epoch` is label+600 for ordinary bars; only 14 closing
rows have separate gateway receipt evidence. The test-only exporter labels its
equal received/durable clock as NUMERICAL ONLY. It creates no deployable ingress
journal and supplies no evidence of historical fsync. Preserve these limits in
downstream reports. MR diagnostic cost is 2 points and BO cost 0, not broker fees.

Negative checks cover duplicate/order/non-finite/identity violations, transition
and weekend/service exclusion, closing input either side of the cutoff, delayed
first observation, unresolved carry/tail, changed routing/hidden state, equally
truncated traces and missing recovery evidence. Reports retain source/input/binary
hashes. A numerical mismatch emits `mismatch.json`, never a PASS verification.

## Frozen Reaction Integration

`strategy_runtime::shadow_reaction::NumericalSession` connects a published
weekend-context frozen view to the same numerical IMOEXF/USD engines used by the
offline replay binaries. It accepts only `origin=Replay`, keeps a single regular
session prefix, and reconstructs full decision state from checkpoint inputs.
This is not a live adapter, durable ingress or deployment entrypoint.

Run the focused tests from the Rust workspace:

```sh
env -u REDIS_URL -u RI_SHADOW_REDIS_URL cargo test --offline -p strategy-runtime shadow_reaction --lib
env -u REDIS_URL -u RI_SHADOW_REDIS_URL cargo test --offline -p strategy-runtime checkpoint_preserves_decimal_quote_fingerprints_exactly --lib
```

The facade checks frozen config identity, sequential M10 execution, pending
entry/exit recovery, MR-first ownership, counters/BO observation, and missing EOD
execution. `complete()` is required before a future runner advances the session;
discarding an unresolved facade is not a recovery procedure. Snapshot generation
does not provide a production atomic write protocol. The independent comparison
below now covers synthetic integrated weekend decision traces and the separate
BO-only control. The separate historical procedure below checks the declared
historical window; it does not certify production receipt timing or the runner.

`serde_json/float_roundtrip` is required for exact checkpoint fingerprints and
decimal PnL reconstruction. Never weaken checksum/state comparison to accommodate
a lossy JSON parse. The checkpoint version and config fingerprint must change if
the numerical contract changes; an existing identity is not a tuning surface.

## Independent Weekend Reaction Fixtures

```sh
python3 scripts/shadow_state_parity/run_reaction_parity.py \
  --imoexf-handoff "$IMOEXF_STATE_HANDOFF" \
  --usdrubf-handoff "$USDRUBF_STATE_HANDOFF" \
  --output "$NEW_REACTION_OUTPUT"
python3 -m unittest discover -s scripts/shadow_state_parity -p test_reaction.py -v
```

The handoff variables must identify the immutable IMOEXF and USD packages used
by the earlier frozen baseline checks. The test suite needs these variables in
the environment for its reference tests; without them those tests are skipped,
not accepted. The output must be a new directory outside either handoff.

`reaction_reference.py` uses the hash-pinned Python model functions, with an
explicit daily-context injection calculated by `weekend_reference.py`. It runs
both original and read-only-instrumented model functions and requires their
outputs to agree. No Rust states or expected fills are supplied to either model.
`reaction_compare.py` checks the declared state projection, exact coverage and
ownership, with absolute float tolerance 1e-8. `reaction_fixtures.py` defines
126 paired cases including MR-only, BO-only, combo and Previous-day variants.
The 09 weekend variants are fixture coverage, not an extra prospective approval.

The opt-in Rust `weekend_reaction_export_actual` test consumes only the generated
inputs. Its 630 restart points restore files and verify full internal state.
The Python comparator covers anchors, decisions, observation, quotas, pending
levels/ownership, events and rounds/gross points, not broker fees or Kahan's
internal compensation. The fixture cost policy is synthetic, not a broker fee
schedule. No fixture totals should be read as trading performance.

Reports contain input/source/binary/reference hashes and explicit false flags
for full historical parity, production receipt certification and deployment.
The production collector, paired runner, durable outbox and activation/recovery
boundaries are separate gates. Do not replace actual receipt evidence with
synthetic timestamps or use this Replay-only facade as a live adapter.

## Historical Weekend Reaction Comparison

```sh
python3 scripts/shadow_state_parity/run_reaction_history.py \
  --imoexf-raw "$IMOEXF_RAW_10M_PARQUET" \
  --imoexf-handoff "$IMOEXF_STATE_HANDOFF" \
  --usdrubf-handoff "$USDRUBF_STATE_HANDOFF" \
  --output "$NEW_HISTORICAL_REACTION_OUTPUT"
python3 -m unittest discover -s scripts/shadow_state_parity -p test_reaction_history.py -v
```

Use the original `live_baseline_revalidation/cache/IMOEXF_raw_10m.parquet`
and the same frozen handoffs as above. USD prices come from the handoff's
`raw_model_input.csv`. Inputs are read-only. Output must not exist or be inside
the handoffs/raw-input directory. `--start` / `--end` allow a smaller diagnostic
interval inside **2026-07-20..2026-10-28**; only the full default interval can set
`full_declared_window=true`. No network, broker, Redis or VPS access is used.

Seven numerical configs have two context policies each: IMOEXF MR/BO/combo,
USD07 Previous-day H3/W4, H4/W4, H3/W2 and USD09 H3/W2. The USD09 weekend arm is
offline coverage only. Native VWAP is NOT context-injected or included in this
weekend experiment; its independent baseline parity remains separate.

Historical prices lack original receipt metadata. The report therefore declares
`historical_prices_declared_close_clock_not_production_receipts` and assumes
`received=durable=processed=bar_label+600s`, with freeze at calendar opening.
These are numerical availability assumptions, not reconstructed live receipt
proof. Sequences and source IDs identify local per-case replay records, not
original Redis IDs. No delayed-history, clock, collector or live restart
acceptance follows.

Every selected weekday needs the complete execution grid, including the 23:40
next-open fill bar. Missing/duplicate/invalid inputs fail, not forward-fill or
silently shorten coverage. Inventory includes service exclusions and missing
weekend labels. Only weekdays trade; accepted weekend sessions affect context.
Calendar exclusions are empty in this historical slice, not an exchange-calendar
integration. Regular and weekend completeness rules are those documented below.

Each session is isolated. Both policies receive identical prices from the older
of the two preceding complete weekdays through the prior day. A second inventory
over ALL available raw history must select identical previous/previous2 anchors
and OHLC/counts, so bounded loading cannot silently choose older/different context.
No position is carried between these cases. `complete()` and the reference must
both be flat without pending orders; an unresolved exit fails the run instead of
being dropped at the next case. Continuous multi-session runner recovery is a
separate acceptance gate, not implied by these per-session replays.

At bar 31 and after the 23:30 bar, Rust restores both context and numerical model
from files and checks full internal state. Expected state is independently rebuilt
from raw prices and the pinned Python references; instrumented/uninstrumented
outputs must match. The common comparator keeps its 1e-8 absolute float tolerance
and exact coverage/identity/owner/counter checks. The historical exporter has a
distinct schema/evidence gate; the synthetic exporter does not accept it.

`run_contract.json`, `inventory.json`, chunk inputs/expected/actual traces and test
logs preserve provenance. `verification.json` is written only after every chunk
passes, one candidate binary/source set is confirmed, and inputs remain unchanged.
On failure, `failure.json` records the failed gate; earlier passing chunks are not
full acceptance. Reports also include per-model/policy/component gross-point CSVs.
They are alternative models, not additive portfolio results, and NOT net economics.
The IMOEXF numerical cost config explicitly uses zero fees; neither its historical
tariff nor USD fee accounting is part of this state/gross-points comparison.

## Weekend Context Cutoff Fixtures

```sh
python3 scripts/shadow_state_parity/run_weekend_context_parity.py \
  --output "$NEW_WEEKEND_CONTEXT_OUTPUT"
python3 -m unittest discover -s scripts/shadow_state_parity -p test_weekend_context.py -v
env -u REDIS_URL -u RI_SHADOW_REDIS_URL \
  cargo test --offline -p strategy-runtime --lib shadow_session_context
```

This is a **context-only** acceptance slice, not new active strategy services.
The `shadow_session_context` core is unregistered in the host and has no Intent/
broker API. It accepts supplied provenance only. Its opt-in collector bridge is
documented below; model-position/pending recovery and the continuous paired
runner must still be integrated and tested separately.
No High180 seed or riskgate session ledger is read or written.

`weekend_reference.py` independently computes context from the declared input;
`weekend_fixtures.py` supplies 13 scenarios for IMOEXF07, USD07 and USD09 under
weekday-only and weekend-context policies. Friday -> Saturday -> Sunday -> Monday
-> Tuesday remains separate calendar sessions. Regular context is a complete
89/101-slot M10 session, not RI's 80/92 heuristic. Weekends use >=44/55 labels
09:50..18:50, first <=10:00 and last >=18:50, including the audit's 09:50 auction.
Pre-transition weekdays retain 09:00. Exclusions are explicit configuration,
not a live exchange-calendar lookup or inferred closure.

The cutoff is opening 07:00/09:00, NOT first event time. Both original receipt
and durability must be before it. Processing can be later only with that evidence
and before freeze. Late/conflicting/post-barrier backdated inputs stay quarantined
on later dates too. Immutable snapshots include input/route/freeze history and
identity/checksum; restart reconstructs and checks the saved view. A failed write
does not expose new context. Stop on storage errors; do not silently retry into
an existing checkpoint or resume using a new full-history fetch.

The explicit ignored Rust exporter receives ONLY inputs and writes its own trace.
It restores actual temporary on-disk checkpoints; Python supplies the independent
uninterrupted expectation. Result: 78 cases, 31,686 operation states, 31,140
receipts, 234 freezes and 312 restores. Exact source sequence selections,
OHLC/range/counts, first/last/availability times, skip reasons, previous/previous2,
cutoff/barrier metadata, readiness and return match. Source/identity hashes are
validated by Rust reconstruction rather than the cross-language logical projection.
The comparator re-derives coverage; missing/extra fields/rows and equally shortened
expected/actual data cannot pass. Source/input/executable hashes are retained.

Status `PASS_SYNTHETIC_CONTEXT_CUTOFF_RESTART` is deliberately NOT MR/BO action
parity, original production receipt evidence, full historical weekend parity or
deployment approval. The USD09 weekend variant is test-only coverage; initial
prospective rollout scope still keeps the 09 comparator weekday-only. Existing
shadows/live configs and their numerical/execution definitions remain unchanged.

The core bounds are 12,000 receipts, 128 freezes and 32 MiB/checkpoint, requiring
explicit rotation. The large offline trace artifacts are not deploy assets or
a model of daily VPS storage. Use only the named ignored exporter, never run all
ignored tests blindly. Complete integration/operational gates before activation.

## Durable Weekend Context Collector

`shadow_session_context::collector::Collector` is an opt-in library bridge,
not a deployable model runner. It reuses the RI immutable raw ingress journal
and read-only Redis source unchanged on disk. New crate-local accessors expose
source IDs with their original collector timestamps and saved attestations;
neither RI strategy policy nor the journal serialization/version changes.

The production constructor uses the wall clock. There is no API for injecting
receipt/durability from a downloaded bar, an envelope timestamp or a checkpoint.
`poll_once` reads versioned Bar envelopes via XRANGE, with an exact durable cursor
check. It never creates a group, acknowledges, trims, adds or publishes Redis
records. A source/storage/parse error marks the generation failed; an ordinary
restart must not skip the unknown interval. File/directory fsync and the exclusive
writer lock belong to the shared journal. Receipt means local collector arrival,
not gateway arrival or proof of exchange completeness.

`replay_pair` feeds the exact committed prefix to weekday-only and weekend-context
stores, in that order. Identity, config hash, calendar, receipt stream, sequences
and original-prefix hashes must agree. Both stores advance or neither does.
`seal_cutoff` requires a caught-up source poll at/after calendar opening. It saves
the original journal attestation and derives the context hash INCLUDING source
IDs; a caller-created barrier or rehashed fake checkpoint is insufficient.

`freeze_pair_new` publishes both frozen context checkpoints in ONE immutable file
before exposing either result in memory. Store these files OUTSIDE the ingress
directory, whose allowed contents are closed by the shared journal format.
Never overwrite a file or silently delete a torn temporary/FAILED marker.
`restore_pair` checks both nested checkpoints and every freeze against the original
journal, including config/generation, source IDs and receipt/durability. An older
checkpoint cannot be selected after a later cutoff was sealed: that incomplete
boundary needs explicit recovery. A failed restore cannot fall back to empty
stores. Later receipts may be replayed from the journal, but cannot rewrite an
already frozen prior context. Redis freshness is false after reopen until polling
succeeds again.

Warmup readiness can differ between the two policies. A frozen file may therefore
contain an unready context; it does NOT authorize model activation. Require both
arms ready and the planned common full-session activation boundary in the future
runner. The numerical facade still rejects `origin=Live` execution. This bridge
does not automatically relabel live/history bars into tradable Replay events.

```sh
env -u REDIS_URL -u RI_SHADOW_REDIS_URL \
  cargo test --offline -p strategy-runtime --lib shadow_session_context::collector
env -u REDIS_URL -u RI_SHADOW_REDIS_URL \
  cargo test --offline -p strategy-runtime --lib readonly_redis \
    -- --ignored --nocapture --test-threads=1
```

The second command runs four explicit local Redis tests (RI shared source,
RI paired runner, reaction bridge and reaction runner). Each starts its own loopback process in a temporary directory,
checks Redis PID ownership, uses a restricted read-only ACL and shuts the process
down afterward. It never connects to `REDIS_URL`, live/VPS Redis or a broker.
`redis-server` must be on PATH, or set `REDIS_SERVER_BIN` explicitly. Reaction
calendar tests use an injected fixture clock, real fsync and a real isolated Redis;
they are not original production receipt evidence.

The shared ingress limits remain 10,000 receipts, 128 barriers and 64 MiB per
generation. A paired snapshot is capped at 80 MiB, but snapshots outside ingress
are NOT included in `usage.committed_bytes`. Aggregate snapshot retention, safe
generation rotation and VPS storage measurements remain runner/rollout gates.
No cleanup job or rotation/reset is authorized by these tests.

The collector alone does not supply model recovery or decisions. Its opt-in paired
runner below now adds these locally; resource acceptance and a separately approved
isolated shadow rollout remain pending. There is no new executable, service config,
live route or promotion in this slice.

## Continuous Paired Reaction Runner

`shadow_session_context::collector::runner::ReactionShadowRunner` owns a new
generation directory with only `ingress/` and `transactions/`. Never reuse a
generation ID or mix it with an older shadow's state. `RunnerSpec` fixes symbol,
source/receipt streams, model parameters, excluded dates and first decision session.
Pair order is always weekday control then weekend-context/no-trades candidate.
Supported prospective pairs here are IMOEXF MR/BO053/combo and USD07 Previous-day
H3/W4 or H4/W4. Other offline controls do not automatically become services.

The source must contain canonical external M10 Bar envelopes; this code does not
aggregate M1 or infer an upstream timeframe. Public create/poll/open uses the wall
clock. A generation is created before the first decision cutoff. Fresh history
cannot claim earlier arrival; missing original context requires actual warmup.
Cutoff uses a fresh caught-up poll. Both arms activate only if both anchors are
ready; otherwise the whole day's pair is suppressed. Calendar/session skips are
explicit failures, not silent changes to the scoring window.

Only original canonical same-day Live receipts drive the numerical model, after
receipt/durable/closed-bar validation. History and HistoryGap are context-only;
weekends/service bars never execute. The private conversion to numerical Replay
retains original provenance in every output step. Live duplicate quotes do not
repeat execution; conflicting or history-owned substitutions fail closed. Missing
consecutive labels or unresolved position/pending/EOD prevent next-day rollover.
There is no new timer or synthetic final fill.

`poll_once` stages both policies and publishes only after transaction fsync and
atomic HEAD. The shared typed transaction store preserves RI's existing disk format.
Every committed operation contains both context hashes and a model witness; restore
reexecutes the original sequence and compares the exact transaction. It does not
read a mutable snapshot as a substitute for receipt provenance. Uncommitted cutoff,
extra/torn files or inconsistent state require operator audit; do not remove them
to make startup pass. Failed write/model/source paths block further automatic work.

Read outputs with `committed_output(index)`. `outbox_id` is stable across restart;
delivery is at-least-once. Full `states` are cumulative diagnostic traces, **not
incremental economics**. Use `deltas[arm].trades` and `deltas[arm].events` for new
records and deduplicate their stable IDs. These are simulated reactions, not real
executions or original exchange-receipt evidence. No Redis ACK/write, broker client
or automatic stdout decision publisher is provided.

```sh
env -u REDIS_URL -u RI_SHADOW_REDIS_URL \
  cargo test --offline -p strategy-runtime --lib reaction_pair
env -u REDIS_URL -u RI_SHADOW_REDIS_URL \
  cargo test --offline -p strategy-runtime --lib readonly_redis \
    -- --ignored --nocapture --test-threads=1
```

Local checkpoint: 12 ordinary runner tests plus one isolated Redis test; 500
ordinary library tests and 82 Python tests across the package/reference helpers.
This is not a rerun of the historical independent Python/Rust parity report through
the continuous runner. Long-generation numerical/resource/recovery comparison,
activation/latency/supervisor runbook and a deployable opt-in CLI are next gates.
Limits: 10,000 raw receipts/64 MiB/128 cutoffs; 20,000 transactions/256 MiB with a
16 MiB single-file cap and observation batches of 16. Whole-generation replay and
disk/RSS usage must be measured before sizing a 20-30-session run. No automatic
compaction, rotation, cleanup, live config update or rollout is implemented.

## Continuous Reaction Acceptance Harness

`run_continuous_reaction.py` drives five separately isolated paired generations:
IMOEXF MR, BO053 and combo; Previous-day USD07 H3/W4 and H4/W4. The default window
is 2026-10-19..2026-10-26 (six regular sessions, warmup plus intervening weekends).
It does not reset the runner at each day's start. Each raw bar commits real local
receipt files and an ingest/observe transaction pair. Freeze uses the original
local journal attestation; this harness bypasses Redis polling deliberately.

The Python oracle maintains two continuous independent context stores and calls
the pinned original/instrumented Python models only at their daily frozen-context
boundary. It checks uninstrumented trades/events against instrumented outputs,
full raw anchors, every model state and newly appended event/trade deltas. The
Rust exporter receives inputs only, never expected decisions or PnL. Coverage is
rebuilt from raw prices, not inferred from matching possibly truncated traces.
Three restarts include the first-session intraday and pending-EOD boundaries, plus
whole-generation recovery. Stable IDs and no extra output after restart are checked.

```sh
python3 scripts/shadow_state_parity/run_continuous_reaction.py \
  --imoexf-raw "$IMOEXF_RAW_10M_PARQUET" \
  --imoexf-handoff "$HANDOFF" --usdrubf-handoff "$USD_HANDOFF" \
  --output "$NEW_CONTINUOUS_OUTPUT" --test-opt-level 1
```

All output directories must be new and outside immutable handoffs. The driver
selects only `reaction_continuous_export_actual`, removes Redis environment URLs,
and records source/reference/input/binary hashes. It uses a test-only clock:
receipt=durable=processed=label+600, freeze=07:00. Historical bars marked Live in
this fixture are **not** production Live receipts. No production import API or
timestamp backdating was added. This test makes no broker-fill/fee/latency claim.

Opt-level is a Cargo test-profile override (including dependencies), default 1;
debug assertions and fsync remain enabled. Record it when comparing resources.
The unoptimized `continuous_reaction_6sessions_v1` and package-only-optimized
`continuous_reaction_6sessions_v2` attempts were interrupted and
retained as failed evidence, not merged into a successful run. This is a local
macOS/debug-assertion test, not a Linux release/VPS sizing result. Compilation is
excluded from reported runner durations. RSS samples are not process peak; byte counts are file
lengths, not allocated blocks; restart uses a warm local cache in the same process.

Artifacts: `input.json`, independent `expected.jsonl`, candidate `trace.jsonl`,
`report.json`, test log, final `verification.json` and deduplicated `trades.csv`.
By default successful temporary generation directories are removed by the owning
test; failed ones are retained for audit. Existing generations/artifacts are never
cleaned. The driver's timeout stops the entire owned Cargo/test process group.
No service, live route, Redis cleanup, rotation or automatic promotion is included.

This bounded check does not certify 20-30 sessions of capacity. Full-generation
replay cost, cold startup and peak memory remain separate measurement gates.

The 2026-10-31 local run `continuous_reaction_6sessions_v3` passed: 60 arm-sessions,
6,060 independent state comparisons, 80 model rounds, 160 event deltas and 15
restarts, without cross-day carry or duplicate economic IDs. All five generations
use the same binary; source/reference/input hashes stayed unchanged. Alternative
profiles and both context policies are counted separately, not as one portfolio.

Final generation size is 6,390,620..7,905,008 logical bytes (3,685..3,721 files).
Largest RSS samples per pair are 29,104..60,448 KiB, NOT peaks. Final same-process
warm restoration takes 28.47..29.82 sec versus 4.27..4.65 sec near first-day EOD.
Receipt ingest+observe p95 is 0.100..0.104 sec; the worst receipt is 0.808 sec.
This is an explicit long-generation recovery sizing risk, not a failed parity
check and not evidence about current VPS load. Keep long-generation/cold recovery,
original receipt availability and deployable supervision as separate gates.

See the [observation and promotion contract](../../docs/shadow-observation-and-live-promotion.md)
for scope and acceptance gates. Private artifact locations are maintained outside
the publication repository. Do not redistribute private economics with engineering
code or treat this numerical fixture as prospective soak evidence.

### Long Generation And New-Process Recovery

`--profiles` selects an explicit subset of the same five approved pairs; unknown
or duplicate names fail before execution. `--process-recovery` builds the library
test binary once, then runs it directly to keep Cargo/compiler costs out of
candidate CPU/RSS measurements. It retains each successful owned generation and
launches a second test process with the same binary, input and recorded final
fixture clock. That process replays the journal and compares the exact state,
HEAD, transaction count, status and directory usage against the original witness.
There is no new transaction or decision publication on restore.

```sh
python3 scripts/shadow_state_parity/run_continuous_reaction.py \
  --imoexf-raw "$IMOEXF_RAW_10M_PARQUET" \
  --imoexf-handoff "$HANDOFF" --usdrubf-handoff "$USD_HANDOFF" \
  --output "$NEW_CAPACITY_OUTPUT" --test-opt-level 1 \
  --start 2026-09-29 --end 2026-10-26 \
  --profiles imoexf_combo usdrubf_previous_day07_h4_w4_shadow_v1 \
  --process-recovery
```

The command covers 20 complete regular execution sessions. It is a bounded
capacity/parity fixture for two selected pairs, not certification of all profiles
or 20 prospective soak sessions. Expected coverage is regenerated from raw data.
`process_measurement.json` and `recovery_measurement.json` record Unix `wait4`
wall/CPU/max-RSS and I/O counters for the owned child. RSS is normalized to bytes
on macOS/Linux and is a high-water mark, not concurrent aggregate memory of
all processes. Rust binary hashing is streamed to avoid allocating the whole
debug executable into the measured process. Other diagnostics/trace costs remain
part of the measurement. The unmeasured Python oracle is a separate process.
Without `--process-recovery`, the measured child is Cargo and may include a build;
do not use that mode's `process_measurement.json` as a runtime memory budget.

`candidate/process_recovery.json` contains the restored witness and internal
open duration. New-process recovery is NOT cold-filesystem-cache recovery: no
host cache flush, server restart or privileged operation is performed. Compare
same build settings; these results do not establish a Linux release/VPS SLA.

Build, candidate and recovery processes each have a 1,200-second timeout. Timeout
or interruption terminates only the owned process group and retains failed
evidence. No code deletes retained generations or edits production receipts.
Repeated recovery cannot overwrite its existing report. Use a new output directory
for a new run; never point the fixture exporter at a production generation.

The reaction Collector caches verified immutable cutoff projections in memory
per instance. Cache hits still require original journal membership and healthy
collector/clock; fresh opens verify the journal and rebuild this derived cache.
No cache is restored from disk or used instead of checking the current prefix,
model witness or transaction chain. A generation's event/trade IDs and file
schemas are unaffected. This removes repeated projection work during whole-log
recovery; it does not implement snapshot-based fast recovery or compaction.

The local `continuous_capacity_20sessions_v2` run passed for the two selected
pairs: 80 arm-sessions, 8,080 states, 124 rounds, six in-process restores and two
new-process restores. All 5,388 IMOEXF transaction rows exactly match the baseline
export. Warm IMOEXF restoration decreased from 376.22 to 173.78 sec in matched
local fixtures; the original overall v1 attempt was interrupted, not accepted.
Final fresh-process restores take 174.24 / 166.79 sec (IMOEXF / USD). Generation
sizes are 23.69 / 25.18 MB logical; measured exporter peak RSS is 161.45 / 172.20
MiB. Allocated disk and evidence traces are additional to logical lengths.
This remains a sequential macOS/test-profile fixture, not a concurrent Linux/VPS
capacity guarantee or prospective soak. The engineering handoff records the
current resource/supervision gates; actual verification outputs stay private.

## RI Synthetic Resource Probe

The ignored test `ri_paired_runner_long_generation_benchmark` measures the local
paired runner on two warmup weekdays plus 30 synthetic decision weekdays, with
101 closed M10 inputs per day. The internal clock exists only under `cfg(test)`.
It uses no Redis/network, preserves no temporary generation after the test, and
does not prove numerical parity, a real trading calendar or VPS capacity.

```sh
env -u REDIS_URL -u RI_SHADOW_REDIS_URL \
  RI_BENCH_SESSIONS=30 RI_BENCH_REPORT="$NEW_BENCH_REPORT_JSON" \
  cargo test --offline --release -p strategy-runtime --lib \
  ri_paired_runner_long_generation_benchmark -- --ignored --nocapture --test-threads=1
```

The report's parent must exist; an existing report is never overwritten. The
test writes measurements, EOD-flat checks and complete replay/recovery outcome.
RSS samples are KiB, not process peak; directory bytes are logical file lengths,
not allocated filesystem blocks. Recovery is in-process on a warm local cache,
not a cold-machine recovery SLA. For isolated process CPU/peak measurements,
build with `cargo test --release --lib --no-run`, then time only the printed
test executable with the same test selector; do not include compiler resources.
Avoid blanket `--ignored` invocations: they now also run this long fsync probe.

## Tests

Portable comparator tests require only Python's standard library:

```sh
python3 -m unittest discover -s scripts/shadow_state_parity -p test_compare.py -v
```

Reference tests additionally require the private handoff and pandas/numpy:

```sh
IMOEXF_STATE_HANDOFF="$HANDOFF" USDRUBF_STATE_HANDOFF="$USD_HANDOFF" \
python3 -m unittest discover \
  -s scripts/shadow_state_parity -p 'test_*.py' -v
```

Without the corresponding environment variables, reference tests are explicitly skipped, not
certified. They test unchanged actions, MR quota/ownership, BO observation during
MR, absence of future prices in pending state, causal prior context, source-hash
pinning, read-only source loading and rejection of changed execution/feed contracts.
The USDRUBF suite also checks all six profiles, uncapped MR, stop-priority,
captured scale versus current range, H3/H4 and W2/W4 windows, warmup fallback,
causal features, negative/zero volume, service/weekend/09 filtering, missing bars
and terminal-entry suppression. On 2026-10-30 all 35 tests passed with both
private handoffs: 16 portable comparator, 8 IMOEXF reference and 11 USDRUBF reference.

The Rust candidate verifiers add nine negative provenance/report tests (pandas
required, no private data): four USDRUBF and five IMOEXF. The complete Python
suite has 44 tests before the RI preflight additions. Five portable RI tests
add incomplete-evidence blocking, frozen-file integrity, matching/unique/timely
closing receipts, missing data and strict path/JSON checks (49 at that checkpoint).
Seven independent RI fixture tests add hand-calculated state/action assertions,
context-policy separation, quota/window/overlap checks and negative comparison
coverage (56 Python tests total).

```sh
cargo test --offline -p strategy-runtime --bin usdrubf_shadow_replay
cargo test --offline -p strategy-runtime --bin imoexf_shadow_replay
```

Its 15 Rust tests cover MR/BO lifecycle, next-open semantics, TP/SL priority,
H3/H4/W2/W4, MR uncapped versus BO one-per-side quota, same-event entry suppression,
no-overnight, incomplete/duplicate/nonfinite data, weekend/service filtering,
causal features, volume fallback, raw CSV timestamps, provenance selection and
immutable output-directory protection. They need no Redis or private fixtures.

The IMOEXF binary adds 16 tests: reaction phases/fills, strict low-normalized
MR thresholds and quota, inclusive entry cutoff versus 20:00 exit, stop/take
priority, held-bar breakeven, BO observation/wait/return gates, close stops,
EOD priority, missing-action/no-overnight failure, causal prior context,
input/provenance validation, naive timestamps and immutable output paths.

Remaining: RI context recovery, weekend paired references, receipt/restart and
isolated shadow rollout acceptance. IMOEXF and USDRUBF weekday numerical parity
is complete; prospective integration is not.
The existing Rust
`usdrubf_hybrid_replay` is a different, older prepared-VWAP replay contract
(including MR K=.045 and tick=.0025 constants), not the new raw-M10 shortlist.
Neither a PASS there nor a self-comparison here certifies the new Rust models.
Do not retune that legacy replay or the live VWAP/bracket adapter to manufacture
parity. The isolated new model is an explicit, separate contract.

## Linux Release Compatibility Probe

`linux_release.py` pins an allowlisted source snapshot plus accepted 20-session
IMOEXF combo and USD H4/W4 paired inputs/independent expected traces. It builds
release test/worker binaries in an isolated offline Linux container, runs service
guards and retained-generation export/new-process recovery, then verifies on the
host against the independent expected state trace. Failed attempts and snapshots
are never overwritten. See the [service runbook](../../docs/reaction-shadow-supervisor-runbook-2026-10-31.md#local-linux-release-probe)
for commands, fixed local environment and acceptance limits.

The current probe uses emulated x86_64 GNU/Linux on an ARM Docker Desktop host;
it is not a native VPS resource/concurrency benchmark or production musl build.
It does not send broker commands or connect to Redis. A successful compatibility
probe does not approve deployment, clean-source status, prospective receipt
provenance or cold-machine recovery. Source identity explicitly remains a pinned
dirty-worktree snapshot until a separately reviewed scoped build is made.

Six portable probe tests cover immutable copy/hash bounds, source allowlisting,
manifest completeness, path/symlink escapes, release identity, process separation
and rejection of unsupported certification claims. With the prior suite and both
explicit private handoffs, 96 Python tests passed on 2026-10-31.
