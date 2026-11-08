# Late Context Recovery And Admission V3

Scope: opt-in isolated IMOEXF / USDRUBF shadow MD sources only. No production
gateway/runtime deployment, trading-route, signal, cost, session-calendar or
position-size change. Existing source v3 and admission v1/v2 remain strict.

## Failure Boundary

After a WS reconnect, broker history can include a previously unseen eligible
context candle behind the last committed source ID. A representative case is
a weekend 09:50 label, absent during forward collection but present in the
next historical response. The immutable ordered Redis stream cannot back-insert
that candle. Previously this stopped the source with
`late unknown history would change immutable source prefix`.

Do not remove 09:50 from the calendar to hide the failure: the frozen research
context extension accepts >=44 of 55 labels 09:50..18:50, first <=10:00 and last
>=18:50. This is context, never a tradable weekend bar. Changing that model rule
requires separate versioning/parity analysis.

## Explicit Source V4

New identities may set:

```json
"late_context_history_policy": "audit_only_v1"
```

This selects `shadow_gateway_ws_closed_m10_v4` in the manifest, health and source
envelopes. Omission keeps v3 and its exact serialized configuration shape.
Only the documented enum value is accepted. Never add this field to an existing
generation or substitute a new executable into its running unit.

For an unknown label behind the immutable tail, v4 quarantines only eligible
weekend or explicitly excluded-day context. The bar must have valid finite
OHLCV, the correct symbol/grid/calendar, a real post-close receipt and a positive
connection generation. It is fsynced into `late-context.jsonl`, not published
to Redis, staged for import, passed to the finalizer or used by either model.
This remains audit-only after the model-open cutoff: no frozen context or past
decision is recalculated. Existing-label price/volume revisions still use the
separate original-bound `revisions.jsonl` path.

Unknown active-weekday history behind the tail still fails. Invalid OHLCV,
clock/generation faults, missing source, uncertain publication and normal
source/model/terminal freshness limits remain fail-closed. A missing candle
does not become synthetic data or a clean session by being quarantined.

Each late-context event records:

- `kind=late_unknown_context_bar`, `disposition=audit_only_no_model_replay`.
- Full `observed` History bar, actual `received_at_utc`, `durable_at_utc`, generation.
- Canonical tail label/ID and its actual committed journal-head SHA256.

The audit deduplicates identical bar observations, retaining distinct later
revisions separately. Bounds are 1024 rows / 1 MiB; capacity or fsync failure
stops visibly. Resume checks checksum, canonical-tail binding, context policy,
calendar, clocks, duplicates and complete newline tails. Failed/incomplete source
attempts still cannot resume. Health exposes `late_context_rows` and
`late_context_review_dates` separately from ordinary price-revision counters.

## Read-Only Admission V3

V4 source requires `reaction_shadow_preflight_v3` and
`reaction_shadow_admission_v3`; old plans reject the new source. All v2 gates,
symbol isolation, actual Rust bootstrap inspection, startup margin, terminal
proof and reviewed evidence remain required. Approval additionally pins:

```json
"late_context_review": {
  "policy": "audit_only_v1",
  "journal_sha256": "REVIEWED_EXACT_RAW_FILE_SHA256",
  "rows": 0,
  "review_dates": []
}
```

These are non-launchable placeholders. Even an empty journal needs an explicit
snapshot. Python validates bounded structure, valid OHLCV/context, absence from
canonical bars, actual receipt/durability ordering, and binding to a declared
canonical journal head. It does not independently reimplement Rust proof hashes.
The source owner verifies its canonical and audit hash chains; the actual Rust
inspector verifies the model's source prefix and context.

An unknown/unreviewed/new audit entry, changed journal during inspection, bad
binding, torn tail or health/count/date mismatch blocks admission. Audit review
is not authorization to import the excluded candle, clear flags, reset a failed
source or claim parity with later-restated broker history.

## Controlled Successor Rollout

1. Preserve failed source/worker generations, raw audits and journals unchanged.
2. Verify a clean scoped commit on Linux: gateway unit tests, owned-Redis runner
   regression, owned WS/Redis reconnect cases for both symbols, source-guard and
   admission tests. Never target default/shared Redis in fixtures.
3. Stage a new source stream, state/config, executable hash and dedicated users/
   ACLs. Start source-only, with no model approval files or auto restart/launch.
4. Observe real recovery plus a natural complete regular session and terminal
   on that successor. Historical bootstrap, old-generation proofs and successful
   synthetic smoke cannot substitute for its required prospective evidence.
5. Review both audit journals and the actual final context tail on the selected
   future model morning. Run the real non-root preflight/inspector and explicitly
   start models only after all gates pass with the full startup margin.

A source replacement during a weekend can require moving model admission to a
later weekday to obtain the preceding regular session's real forward terminal.
Do not retain an obsolete activation date by backdating reviews or weakening
terminal/recovery checks. Twenty clean model sessions begin only after actual
model admission, not during source-only or failed-generation collection.

Local verification at implementation checkpoint: 108 gateway unit tests passed,
three owned-Redis runner cases passed, six owned WS/Redis CLI cases passed;
66 Python policy tests passed (two real-worker integrations run separately on
Linux). Gateway all-target clippy, formatting and diff checks are required.
This is focused source/admission coverage, not new frozen-model parity or live GO.

Linux verification of source commit `ce7fcac`, with the separately pinned
fixture-only correction `a400560`, passed 108 gateway unit tests, all three
owned-Redis runner cases, eight service unit tests and the explicit owned-Redis
service integration. Seven RI receipt-audit tests also passed. Non-root,
network-disabled Docker fixtures then passed all 68 policy/actual-worker tests
and six composed WS/Redis/actual-worker cases, with no skips. The resulting
worker binary is unchanged; only the MD-only source executable needs updating.

Two harness problems were retained and corrected without relaxing production
guards: pure UID-policy tests must not run as root; a growing raw WS fixture
must not expand the model bootstrap beyond its 2..4 complete regular sessions.
The fixture now selects its latest two regular sessions plus the actual context
tail, while retaining longer raw history to exercise late-context reconnect.
Original failed reports remain evidence, not successful verification records.
