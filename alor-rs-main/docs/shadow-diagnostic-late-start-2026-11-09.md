# Diagnostic Late Start

Opt-in for the isolated IMOEXF/USDRUBF numerical reaction pairs only. No live
strategy, RI model, broker transport, signal formula, cost or position size is
changed. This is not an intraday resume or a bypass of the strict morning gate.

## Two Contracts

The existing preflight/granted launcher still requires a future morning cutoff
and rejects a diagnostic spec. `reaction_shadow_late_launch.py` has a separately
versioned create-only plan, approval and supervisor manifest. Do not overwrite
an existing model owner, retarget an old date, remove failed markers or inject
clocks into production to start it.

Late creation is allowed on the declared regular weekday after07:00, before
23:30, with a full startup margin and ready-by no later than23:40. Operational
policy should leave considerably more time than this hard upper bound.

## Immutable Inputs

Use the ordinary frozen model config and costs. Keep `runner.bootstrap` for
2..4 complete regular context sessions, within14 calendar days, plus observed
weekend/service context routed by the existing policy. Add:

```toml
[runner.late_start]
version = "diagnostic_late_start_v1"
source_first_live_session = "APPROVED_SOURCE_MANIFEST_DATE"
first_clean_session = "NEXT_REGULAR_SESSION_DATE"

[runner.late_start.recovery]
last_source_id = "EXACT_INCLUSIVE_SOURCE_ID"
rows = 350 # Example only: actual bootstrap-plus-recovery row count.
source_sha256 = "EXACT_INSPECTION_SHA256"
last_label_utc = 1 # Replace with actual latest closed model label.
```

`first_decision_session` is the diagnostic day, not the first clean day.
`source_first_live_session` pins the existing source identity independently;
it cannot be later than `first_clean_session`. The latter must be the next
regular session in the same exclusion calendar, not an arbitrary date.

Read-only `reaction_shadow_worker inspect-late-start SPEC FIRST_ID LAST_ID`
selects context and recovery hashes from actual Redis data without creating a
generation. For initial selection, SPEC can be the ordinary base config with
the diagnostic first-decision date and without bootstrap/late_start sections.
Pin the returned bootstrap/recovery in the approved spec, then inspect again.
Require a contiguous current-day M10 grid from07:00 to the selected closed
label; no forward-fill, invented bars or silently skipped gaps. Recovery ends
before23:30 and its next expected close must not already be overdue under the
ordinary missing-bar budget. A valid last closed prefix need not arrive again
between M10 closes; this does not waive ages for newly queued bars.

Both historical context import and diagnostic freeze retain actual receipt and
durability clocks. The first frozen context carries
`diagnostic_as_observed_at_utc`: it does not claim those inputs were received
before07:00. An actual current collector barrier, not a backdated barrier,
binds its durable source prefix. Prior published source bytes are untouched.

## Recovery And Forward Processing

Only the pinned prefix receives recovery treatment. The model processes every
eligible current-day bar in order, retaining counters, position and pending
owner; there is no flat fallback. Deltas from this prefix have disposition
`diagnostic_replayed`. Historical simulated fills are not prospective trades
or receipt-time executable PnL.

Subsequent Live-origin bars retain the existing queue, ordinary/terminal age,
missing-bar, source-health and EOD guards. The recovery boundary never moves.

An existing v4 source can still label today's WS publications `History` if its
model-origin activation is tomorrow. On the diagnostic day only, a new
post-recovery History row may drive numerical forward observation when its
original raw WAL payload contains a valid `as_observed_ws` release proof.
Validate release generation, sequence, closed/observation/receipt clocks and
next-timestamp or actual terminal semantics. Such steps are explicitly
`diagnostic_forward_observed`; original origins and timestamps remain intact.
History backfill without that proof is rejected, not promoted to Live. Both
queue and model-age budgets still apply. This exception ends before the first
clean session, when normal Live-origin processing is required.

## Quality And Counting

Every committed output/status of this generation carries
`diagnostic_late_start` with diagnostic day, first clean day, recovery boundary
and current eligibility. The entire first day is excluded from clean-session
and comparable prospective economics, including forward closes of a position
reconstructed from morning history. Never sum cumulative state.trades.

`started_sessions`, `clean_sessions_completed` and the session budget exclude
the diagnostic day. Completing its EOD proves only that numerical exposure and
pending state resolved; it does not earn clean credit. The next session uses
ordinary pre-opening context freeze and freshness rules. It may become clean
only if the previous numerical session really completed flat with its terminal
bar. Missing EOD fails closed; do not invent a fill or carry overnight.

First-day context and transactions remain immutable after rollover. New
revisions/late context remain audit-only. Do not mix this marked output with
legacy parity/economics readers that do not understand the quality contract.

## Launcher And Isolation

The dedicated plan version is `reaction_shadow_late_preflight_v1`. Files pin
worker, supervisor, complete approved spec and v4 source manifest. Paths pin
source, distinct writable model owner, new generation and approval. Redis
binding pins private IPv4/port and a dedicated `shadow_ws_reader_...` username.

Approval version `reaction_shadow_late_admission_v1` pins plan/spec hashes,
review time (at most300s old), ready-by, exact revision/late-context snapshots,
and explicit gates: `complete_context`, `canonical_recovery_prefix`,
`as_observed_audit`, `diagnostic_day_excluded`, `no_order_emission`,
`source_binding`. Two separately hashed, read-only evidence files document
`source_terminal` and `overnight_recovery`; existing applicable source evidence
may be reviewed rather than repeating a night solely for model creation.
That does not certify a newly changed terminal algorithm automatically.

Run `--preflight-only` first: it uses the actual Rust inspector and read-only
Redis but creates no model state. Create then fsyncs a
`reaction_shadow_supervisor_v4_late_start` manifest containing the exact grant
before worker spawn. Valid audit appends after the final successful snapshot
check are as-observed observations, not a writer fence. Changed config/approval,
failed/stale source or insufficient margin blocks spawn; partial attempts are
retained. The full startup margin is checked again after durable reservation.

Use `configs/examples/reaction-shadow-worker-late.service` only after review
and full token substitution. Different non-root source/model UIDs, source and
config read-only mounts, exact-stream read-only Redis ACL, denied command/group
writes, broker-free child environment, resource limits and no automatic restart
remain mandatory. No [Install] section or timer grants admission implicitly.

## Qualification Gates

Require strict-path regression, actual pending/state reconstruction equivalence,
unchanged receipt clocks, changed/missing recovery rejection, forward History
proof rejection/acceptance, post-boundary freshness, first-day EOD exclusion,
next-day clean counting and deterministic restart audit. Test actual native
worker/launcher under separated UIDs and read-only ACL before any rollout.
Intraday resume remains prohibited, including for this generation. An explicit
reviewed recovery contract after a stopped/failed granted attempt is separate
work; do not use the old supervisor CLI to drop its grant.

Local checkpoint: runtime library507 passed/14 opt-in ignored; all5 late-start
unit cases passed, including reconstructed pending state in both models and
History-origin WS terminal consumption before next-day clean counting. Python
policy/grant100 tests passed;3 owned Redis/actual-worker late protocol smokes
passed with actual wall time. Runtime all-target clippy with warnings denied,
fmt and diff checks passed. These local results do not replace native Linux
role/ACL qualification or actual source admission.
