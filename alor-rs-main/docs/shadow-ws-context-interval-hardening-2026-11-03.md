# Context-Only WS Interval Finalization

Scope: isolated MD source v2, not the trading gateway. The admitted v1 canary,
RI collector, retained shadows, live strategies and broker paths are unchanged.
This engineering patch does not admit model pairs or award clean sessions.

## Contract

Source version is `shadow_gateway_ws_closed_m10_v2`. The immutable source config
must explicitly include `context_finalization = true` (JSON boolean). V1 state
cannot be resumed by this binary. Use a new stream, state directory and reviewed
generation; do not edit an existing manifest or attach a collector in-place.
The supervisor still supports v1 with its original exact schema and health gate.

`TerminalFinalizer::new` preserves the default policy/hash and behavior.
Only `new_with_context` enables the additional context policy. Its policy hash
binds the calendar, excluded dates and versioned context-finalization contract.
Eligible context means weekends or explicit model-excluded dates, from 09:50
through the configured terminal label. A null session override means no bars.
These are frozen source-profile bounds, not a universal exchange calendar.

An already observed non-terminal context candle can release at its own interval
end plus grace without waiting for the next real timestamp. The release requires
the existing healthy generation, ACK, resolved history, fresh heartbeat, checked
clock and a drained event queue. The 90-second maximum lateness still applies.
Reason is `context_interval_ws`, with actual first/last receipt, observation
cutoff, generation, policy digest and no next-label claim. The canonical envelope
remains `History`, even after model activation: this is context, never a trade.

No buffer means no candle. Missing intermediate context slots are preserved.
No volume or OHLCV is synthesized. An expected terminal candle must still exist:
it uses `session_boundary_ws`, not a fabricated interval/complete-session marker.
Ordinary tradable weekdays retain greater-real-timestamp release, the original
freshness/deadline checks and the existing terminal fallback.

Canonical rows and deterministic IDs remain immutable. Pending fsync, uncertain
XADD readback, journal durability, revision audit and prefix verification use
the same storage path. A timer/next-timestamp race cannot commit twice. Reset
cannot discard an uncertain publication, and restart never restores old buffers.

## History Is Not Timely Live

Historical import records bind `received_at_utc`, `imported_at_utc`, generation
and one explicit `import_mode`:

- `bootstrap`: label precedes model activation and actual import precedes its
  07:00 cutoff.
- `context_recovery_before_cutoff`: an actual closed context candle arrives and
  is durably imported strictly before the next eligible regular 07:00 cutoff.

The next cutoff skips weekends/excluded dates with a bounded calendar search.
Receipt must be after interval close and no later than publication. Ordinary
unknown active-weekday history is still rejected. Unknown history behind the
canonical prefix is rejected even before cutoff; this is not sorted insertion
or silent recovery. At/after cutoff no new historical context can rewrite the
frozen context. Already-known frames remain revision-audit input only.

## Verification And Admission

Default tests cover sparse/no-buffer behavior, context terminal, lateness,
undrained queue, stale feed, weekday equivalence, explicit exclusions, immutable
revisions, policy-bound restoration and invalid historical provenance/cutoffs.
The original negative test uses the default non-opt-in finalizer deliberately;
it remains a regression guard against relaxing all Live deadlines.

An explicit owned-Redis pipeline test exercises Runner raw/history/ACK handling,
context timer -> canonical History envelope -> real XADD -> durable journal ->
verified prefix -> clean stop/restore. It checks missing-slot preservation and
rejection of reconnect history behind the prefix. Its clock injection exists
only under `cfg(test)`, not in production or the CLI fixture switch.

```sh
REDIS_SERVER_BIN=/ABS/redis-server cargo test --locked -p alor-gateway --lib \
  sparse_context_pipeline -- --ignored --nocapture
```

This test starts only its own Redis on a temporary Unix socket, verifies PID and
always reaps its child. It does not connect to localhost:6379 or service Redis.
The separate CLI smoke uses owned WS/RESP and PID-verified random-port Redis;
it continues testing both symbols, reader/source ACL denial, history-before-ACK,
reconnect, actual model bootstrap, stop/resume, prefix tamper and uncertain ACK.

Before a v2 source-only deployment: finish scoped local/native checks and pin the
artifact. Before model admission: require real source terminal/recovery evidence,
actual worker bootstrap, unchanged calendar and genuine pre-cutoff warmup.
Synthetic context tests do not establish real-weekend clean sessions or model
parity. Missing/late receipts still require review and a new generation if needed.
Dates remain conditional; neither a calendar deadline nor 20 sessions gives
automatic promotion. Full live gateway integration remains a separate review.

Local checkpoint: 105 gateway library tests passed; the owned-Redis pipeline
test is excluded from the default suite and passed by explicit invocation.
The 22 supervisor tests, 7 read-only audit tests and 4 composed CLI smoke cases
passed. Gateway all-target clippy with warnings denied, fmt and diff checks passed.
Native verification and real-forward evidence are separate gates.
