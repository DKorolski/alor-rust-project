# Isolated Closed M10 Source Contract

`scripts/shadow_closed_bar_source.py` is an optional standalone market-data
source for the new IMOEXF/USD reaction shadows. It does not modify the shared
gateway router, legacy streams, live models or order paths. See the
[service runbook](reaction-shadow-supervisor-runbook-2026-10-31.md) and
[live-promotion gates](shadow-observation-and-live-promotion.md#last-candle-and-eod-responsibilities).
Current source identity is `shadow_authenticated_rest_closed_m10_v3`; the
[revision-hardening/readmission plan](shadow-source-revision-hardening-2026-11-02.md)
supersedes v2's blanket revision failure. Existing v2 deployments are not changed
by editing this document or upgrading a local checkout.

## Problem and Boundary

The current gateway router buffers a Live candle until a greater timestamp
arrives. Without a next-session tick, a terminal candle can remain buffered and
arrive later as History/HistoryGap. A complete-session shadow must not invent its
EOD fill or infer flat from missing input. This source confirms closed candles
independently and publishes to a NEW stream with a NEW generation identity.

REST and WS OHLCV need not match exactly. Compare them before admission, keep
both arms on the same selected source and separate source drift from model and
broker-execution drift. Do not splice new-source economics into old generations.
This source is not itself a live EOD watchdog or permission to switch live input.

## Confirmation and Time

- Only authenticated `GET https://api.alor.ru/md/v2/history` and OAuth refresh;
  no order endpoint, CWS client or portfolio subscription. Do not use anonymous
  responses as timely live evidence.
- M10 labels are interval BEGIN, despite the legacy `close_time_utc` field name.
  The 23:40 candle closes at 23:50. First confirmation must occur after close+5s.
- Require two identical OHLCV responses at least five seconds apart after close.
  A changed candidate restarts confirmation. This stability assumption is not
  an exchange finality certificate. Later revisions to the two most recently
  published bars are classified and durably audited. Default policy still fails
  on any revision; the narrowly scoped volume exception is described below.
  Neither policy rewrites already-published/frozen evidence.
- REST `to` covers the full last requested interval (`label+599`); filter payload
  labels to the requested closed range. Do not accidentally omit the final
  interval by querying only through its starting label.
- Validate finite OHLCV, geometry, alignment, duplicates, clock/HTTP Date and
  monotonic consistency. Wall-clock skew allowance is at most five seconds.
- After activation, only same-day closed bars confirmed within 90 seconds of
  close are Live. The consumer independently retains its 120-second lateness
  limit. Preserve `source_proof`, confirmation times and candle hash; do not
  treat this as a historical WS event reclassified to Live.
- Prior context, weekend and excluded holiday bars are History. Record genuine
  ingestion times; downloaded history is never original timely receipt evidence.

Weekday source labels are 07:00..23:40; weekend/excluded-date raw context labels
09:50..18:50 follow the frozen research extension. Model/calendar guards still
forbid weekend/holiday trading. Excluded dates must be explicitly reviewed; the
source does not discover an exchange calendar automatically.

Sparse context is handled separately from missing weekday input. An absent
weekend/holiday label needs two responses after close+90s and is recorded in
`context-gaps.jsonl`, not emitted as a synthetic candle. A later appearance of
that absent bar fails for review. This does not relax regular-session completeness
or the worker's expected-live/EOD deadlines.

## Published Revision Policy

`revision_policy` is immutable generation configuration. Omission means
`strict_ohlcv`, not implicit permission to ignore volume.

| Policy / change | Outcome |
| --- | --- |
| `strict_ohlcv`: volume-only revision | Audit + fail closed |
| `imoexf_author41_short_bo053_volume_audit`: volume-only revision | Audit separately; retain original OHLCV and continue |
| Any policy: OHLC revision | Audit `price_revision` + fail closed |
| Any policy: published bar absent in REST response | Audit `missing_published_bar` + fail closed |
| Invalid timestamp, negative/nonfinite volume, invalid geometry | Fail closed |

The exception is opt-in, restricted to symbol IMOEXF and the independent
Author41-short/BO053 price-only reaction model. Supervisor admission additionally
binds the source to the IMOEXF worker engine. It is not permitted for USDRUBF,
VWAP strategies, RI, shared live feeds or an arbitrary downstream consumer.
Tests compare complete numerical MR/BO state at every step, including both
weekday/weekend-context arms. Raw input hashes still differ: this is a declared
source policy, NOT a claim of identical raw-volume/reference provenance.

`revisions.jsonl` records original and observed OHLCV, symbol, policy, class,
receipt time, resolution and checksum. Identical revisions are deduplicated
across clean restart. A later distinct volume revision gets another record.
The canonical Redis prefix, source proofs and frozen model state stay unchanged;
both arms keep consuming the original first-published candle. No new model bar
is manufactured for a revision. Both upward and downward valid volume changes
are audited; invalid volumes are never tolerated.

Audit bounds: 1 MiB / 1,024 distinct revisions. Exhaustion, write/fsync failure,
torn/checksum-invalid audit or source/audit mismatch blocks operation. A fatal
audit also blocks resume if the process crashed before writing `FAILED.json`.
Do not delete audit records to reclaim capacity or permit restart.

## Identity and Persistence

Stream: `shadow.source.closed10m.<SYMBOL>.<GENERATION>`. IDs are deterministic
`bar_label_ms-0`, NOT receipt timestamps. Envelope publication time and the
consumer's own receipt/durability clocks remain separate. One source writer owns
one new key; both comparison arms consume that same source.

`manifest.json` pins config, `writer.lock` permits one local writer, Redis retains
the canonical append-only source prefix. Resume requires matching configuration
and an intact prefix. A missing/trimmed stream or `FAILED.json` blocks restart.
Keep failure evidence; a new generation requires explicit operator admission.

Source Redis ACL: `XRANGE/XLEN/XADD/PING` on its exact key, no trim, groups, live
state or command keys. Use a different OS user/credential from the read-only
worker. Limit source network access to the approved market-data/OAuth service
and Redis. Model workers receive no broker token and no broker network route.

Example shape, not a launch instruction:

```json
{
  "symbol": "IMOEXF",
  "stream": "shadow.source.closed10m.IMOEXF.REVIEWED_GENERATION",
  "bootstrap_start": "2026-10-28",
  "first_live_session": "2026-11-02",
  "excluded_dates": ["2026-11-04"],
  "revision_policy": "strict_ohlcv"
}
```

Provide `SHADOW_ALOR_REFRESH_TOKEN` and `SHADOW_SOURCE_REDIS_URL` through an
approved secret mechanism, never CLI values, Git, journals or worker environment.
Review dates rather than reusing this historical example blindly.
For the specifically reviewed IMOEXF reaction pair, explicitly substitute
`imoexf_author41_short_bo053_volume_audit`. Do not do this for the USD source.

The matching reaction supervisor requires `--source-state` for a
`shadow.source.closed10m.*` stream. Its v2 manifest pins the source directory and
manifest hash; stream, symbol, activation date, excluded dates and engine/policy
must agree. It checks FAILED, manifest integrity and the held writer lock before
spawn, before/after polling and during waits (at most one-second check interval).
Failure stops the owned worker and leaves an incomplete attempt requiring audit.
It never restarts the source, repairs a generation or grants broker access.

Give the supervisor read/traverse-only access to the source state directory,
manifest, FAILED marker and writer lock. Use an approved shared read group/ACL
(for example directory 0750 and files 0640 with appropriate ownership), not
world-writable state or sharing source credentials. Verify these permissions
under the actual worker OS user. A held lock is liveness evidence, not proof
of feed progress: model-bar/queue/freshness deadlines remain mandatory.

Bounds: 4,500 rows; 40 calendar days from bootstrap start; HTTP 2 MiB/8 seconds;
Redis socket timeout five seconds. No automatic trim, reconnect loop or generation
rollover. Use explicit service admission, `Restart=no`, no reboot autostart and
target-tested memory/CPU limits. If ACLs are runtime-only, Redis restart requires
controlled reprovision before admitting the source or reader again.

## Acceptance

1. Run source tests and native-host smoke with dedicated credentials/network.
   Verify forbidden-key/order-route denial and protected environment handling.
2. Compare authenticated REST/WS labels and OHLCV; record differences privately.
   Approve source identity independently of the model/context selection.
3. Start source-only, inspect 2-4 complete regular sessions and timely fresh
   closed bars. Never emit incomplete candles or rewrite an old source key.
4. Inspect/pin worker bootstrap against the new stream. Create a fresh paired
   generation before cutoff, verify loaded/caught-up/armed and zero historical
   decisions; confirm both frozen contexts when the first session starts.
5. Observe terminal 23:40 delivery after 23:50 within budget, complete/flat paired
   EOD, clean explicit restart and next-session transition. Startup readiness
   alone does not satisfy these checks or count as a clean prospective session.

```sh
python3 -W error::ResourceWarning -m unittest discover -s scripts \
  -p test_shadow_closed_bar_source.py -v
```

Tests cover terminal publication without a next tick, incomplete/revised/late
bars, receipt clocks, calendar, sparse context, HTTP bounds, duplicate-free resume
and missing state, volume-only policy/deduplication, fatal audit recovery and
source dependency loss. Broker deadlines/finality still need prospective observation.
Rollback stops only the owned source/worker, preserving generations and failed
markers. A gateway finalizer or live runtime watchdog is separate reviewed work,
not a silent replacement of this evidence contract.
