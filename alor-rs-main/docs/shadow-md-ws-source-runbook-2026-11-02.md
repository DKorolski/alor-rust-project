# Isolated WS Market-Data Source

Scope: `alor_shadow_md_ws`, not the ordinary trading gateway/supervisor.
It only constructs `WsHub::start_market_data_only`. No CWS, order consumer,
account subscriptions, strategy execution or broker command publisher is started.
Live and retained shadow deployments remain unchanged.

## Configuration And Origin

Templates: `configs/examples/shadow-md-ws.gateway.toml` and
`configs/examples/shadow-md-ws.imoexf.json`. They are not launch-ready: replace
paths, generation, dates and gateway-file SHA256 after operator review.
Supply `ALOR_REFRESH_TOKEN` and `SHADOW_SOURCE_REDIS_URL` through protected service
environment files, never CLI arguments or checked-in configuration.

Two dates deliberately differ. `terminal.first_live_session` enables real forward
WS observation, including the source-only canary. Top-level `first_live_session`
is the future model activation. Earlier rows remain `History` even if captured
forward with a terminal proof. History received after subscription never becomes
backdated `Live`. Missing active history is an error, not a repair opportunity.

Weekday collection is M10 labels 07:00..23:40; context collection starts 09:50
and ends at the configured weekend/exception terminal. `excluded_dates` identifies
model context days. A calendar `null` additionally disables source collection for
that date. The sample November 4 exclusion is an explicit observation policy,
not a claim about the exchange holiday schedule. Pin source/worker calendars
together and change neither inside an admitted generation.

Bootstrap needs at least two complete preceding regular sessions. Select only
the 2-4 complete sessions actually required by the approved worker bootstrap;
do not request months of minute data. Current bounds: source lifetime 40 days
from bootstrap start, 4,500 canonical rows / 16 MiB, 1,024 revisions / 1 MiB.
Review capacity before extending soak; capacity exhaustion fails visibly, never
trims data in-place. The worker retains its own stricter bootstrap limits.

## Launch Boundary

Build and pin a clean commit/tag first. Validate on owned loopback WS/Redis with
the actual approved worker. The fixture switch only permits explicit loopback
WS/auth/Redis and a dummy cached token; it is not a production clock override.

```sh
cargo build --locked -p alor-gateway --bin alor_shadow_md_ws
alor_shadow_md_ws create /ABS/source.json /ABS/new-source-state
```

Use a separate non-login OS identity, state directory mode 0750, UMask 0027,
read-only executable/config, small CPU/memory/task limits, no automatic restart,
and a source-only unit. Restrict network egress to MD/auth plus its private Redis.
Source Redis ACL: only `PING`, `XLEN`, `XRANGE`, `XADD` on the single new source
stream. Worker reader ACL: `PING`, `XRANGE`, `MULTI`, `EXEC` on that stream only.
Neither identity may publish `cmd.orders.*`, access live keys, or use default
Redis credentials. Prove denied commands before starting the source.

Existing failed REST generations, old shadow services and live gateways must not
be reused, restarted, deleted, trimmed or rewired during this launch.

## Publication And Health

Ordinary candles release on a greater real WS timestamp. Terminal fallback
requires the actual buffered candle, healthy acknowledged generation/heartbeat,
resolved history and a drained event queue after session end plus grace.
Proofs retain real receipt time separately from processing/cutoff time. No timer
fabricates data. This is an as-observed cutoff, not exchange-certified finality.

Publication stages `pending.json` durably, writes one deterministic Redis ID and
then fsyncs the hash-chained `committed.jsonl`. An uncertain XADD response is
resolved by reading that exact ID/payload, never by blindly retrying XADD.
Unresolved publication or partial evidence prevents resume and requires review.
Late revisions append to `revisions.jsonl`; canonical bars are never rewritten.
Price revisions mark `review_dates`; affected sessions cannot count as clean
without explicit source-drift review. Volume-only revisions remain audit events.

`HEALTH.json` updates atomically every two seconds on disk, not in Redis.
It reports generation, receipt/heartbeat, history readiness, watermark, revision
counts/review dates and `order_emission_allowed=false`. The worker supervisor
checks its exact source manifest, live writer lock and health freshness. Admission
requires ready/history-complete/ACK/connection. Runtime reconnect is tolerated
only within the bounded WS-receipt budget; stale source or invalid generation
terminates/latches the worker. Process existence alone is not readiness.

Initial startup and each subsequent unready period have a bounded 600-second
source history-readiness budget. Process age is not reconnect age, and repeated
flapping does not reset an ongoing outage budget. Worker freshness limits remain
stricter and unchanged. See [recovery hardening](shadow-ws-reconnect-readiness-hardening-2026-11-03.md).

Native observation showed requested history arriving before the final subscription
ACK 200 (396 data frames, then one response). The MD-only transport preserves
these frames under the requested guid without activating it. It still requires
the explicit 200 response; data alone, missing status or a rejection never counts
as confirmed readiness. The default Trading ACK path is unchanged.

## Stop And Resume

SIGTERM/SIGINT closes the MD transport and records an explicit clean stop.
`resume` is manual, same immutable config, matching complete filesystem/Redis
prefix and completed prior attempt. Active-model resume is only before 07:00
and with preceding terminal evidence. No intraday active-model backfill is
silently promoted to Live. `FAILED.json`, pending/unknown files, torn journals,
missing/trimmed/changed Redis rows, symlinks or an incomplete stop block resume.
Do not remove the marker or reset a shared Redis to force recovery.

Before model admission, observe a real terminal release on the native source-only
canary, inspect bootstrap with the approved worker, then create fresh worker
generations before their first decision cutoff. Keep source and model clean-
session counters separate; a partial canary day is not a clean model session.
Live EOD must not wait for the final 23:40 bar to close at 23:50. Promotion of
this source/finalizer to live is a separate reviewed flat/order-clean rollout.

## Reproducible Smoke

```sh
SHADOW_MD_WS_BIN=/ABS/alor_shadow_md_ws \
REACTION_WORKER_BIN=/ABS/reaction_shadow_worker \
python3 -W error::ResourceWarning -m unittest discover \
  -s scripts -p test_shadow_md_ws_cli.py -v
```

The harness starts its own Redis on a random loopback port and verifies its PID.
It supplies owned WS/RESP servers, validates broker-opcode/ACL denial, bootstrap
with the real worker, health guard, stop/resume, tampered-prefix rejection and
uncertain-ACK readback without duplicate append. It never uses port 6379 by
default, broker credentials or service streams. Synthetic clock tests cover
terminal proof creation/restoration separately; only the native forward canary
can validate a real terminal cycle.
