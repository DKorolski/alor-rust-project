# Bounded Ordinary-Bar Collection, Not Freshness Waiver

Scope: isolated MD source v3 and read-only shadow supervision. No trading gateway,
CWS path, strategy parameters, live adapter or model age limit is changed.
Existing v1/v2 generations, including failed generations, remain immutable.

## Why A Separate Budget

A greater timestamp for an ordinary M10 candle can arrive after the terminal
publication budget. A reproduced 19:50 label / 20:00 close / 20:02:05 next-timestamp
case is 125 seconds late. The ordinary legacy feed can continue collecting it,
while a source using the terminal 90-second limit for every candle stops.
Collection continuity and prospective model freshness are different contracts.
This change does not establish the cause of the upstream delay or promise that
arbitrarily late data is tradable.

## V3 Contract

`shadow_gateway_ws_closed_m10_v3` requires an explicit immutable
`ordinary_observation_max_lateness_seconds`, bounded from the configured terminal
lateness to at most 300 seconds. Templates select 300 for USD and 90 for IMOEXF.
There is no automatic version migration or silent default that relaxes v2.
Use a new stream/state generation and an explicitly reviewed activation date.

The pure finalizer has a separate opt-in `new_with_observation` constructor.
`new` and `new_with_context` preserve their old policy digests and behavior.
Only a non-terminal, non-context, actually observed ordinary candle closed by a
greater real timestamp may use the collection budget. Quotes after that budget,
bad clocks, stale heartbeat, unconfirmed ACK/history/generation, or an unresolved
publication still fail. No missing candle, next-timestamp or receipt is invented.

A release beyond the original terminal budget uses `next_timestamp_delayed`,
not `next_timestamp`. Its proof retains actual first/last receipt, close, cutoff,
next label, generation and the new policy digest. Rehashing it as timely or
restoring it with a v1/v2/context/terminal policy is rejected. Canonical IDs,
append/readback/fsync, audit-only revisions and prefix checks are unchanged.

For active ordinary sessions `Live` origin continues to mean genuine forward
observation, NOT a promise of freshness. Pre-activation/context origin rules
are unchanged; no active delayed candle is disguised as History. Event logs
expose actual lateness and `timely_under_terminal_budget`. Health exposes
`ordinary_delayed_rows`, `last_ordinary_delay_seconds` and
`model_freshness_policy=consumer_owned_unchanged`. Source `ready` is collection
liveness only; it is not permission to advance a model.

Terminal fallback still requires the real terminal candle, drained prefix and
its original grace/deadline. Context interval release keeps its original budget.
Delayed-ordinary proofs cannot be used to relax either contract.

## Model Safety Is Unchanged

The supplied reaction service policy still has queue age 30 seconds and model
lateness 120 seconds. At 125 seconds an active model fails closed BEFORE numerical
advancement; it may already have failed the missing-bar deadline before the
source receives that candle. The source can retain evidence independently.
There is no retrospective reaction, fill or automatic intraday model resume.

This patch alone is therefore NOT GO for an active model exposed to 125-second
latency. Changing a model freshness budget would require separate controlled
review/evidence. Do not describe successful collection as a clean model session.
When evaluating delays, distinguish bootstrap/context dates from active model
dates and use the consumer's own receipt and durability clocks.

The model-side code change only extracts the existing unchanged age checks for
a direct regression test. Neither limits nor failure semantics are retuned.

## Price Revisions

An open-only revision seen after reconnect in both v1 and v2 supports upstream
historical repricing, not a v2-only finalizer regression. It remains a PRICE
revision: preserve the original canonical candle, append audit evidence and
retain the review date. Open prices can affect pending next-open fills in
reaction models; do not globally ignore open revisions because one sample has
no numerical effect.

The reviewed sample was checked with the frozen-cost Rust reaction engine for
MR, BO and combo under both context policies. Engine/per-bar state outputs were
unchanged for that historical day, and a raw daily anchor-metric projection was
unchanged. BO had no executed round on that day; this does not establish general
BO next-open fill invariance. M1 and M10 live reference bars also agreed with
the original open. The reason for the broker's later history repricing cannot
be established without additional upstream evidence. This is narrowly scoped price sensitivity on DECLARED historical
close clocks, not original operational receipt parity, a live High180/riskgate
replay or equality of raw input/checkpoint provenance hashes. Private observations
and reproducible price artifacts remain outside the published engineering tree.
No running source's review flag is automatically removed.

## Verification And Rollout Gates

Regression coverage must include 90-second default rejection, 125-second delayed
evidence with continued next-bar progress, the bounded upper limit, stale feed,
unchanged context/terminal behavior, proof tamper/restoration and immutable prefix.
The owned-Redis Runner test admits before activation, then exercises a genuine
post-activation Live-origin delayed event, durable XADD/journal and restore.
Its clock injection is cfg(test)-only, unavailable in production/CLI fixtures.
The model test separately confirms the unchanged 120-second rejection.

Repeat local/native scoped tests and both-symbol CLI ACL/bootstrap/reconnect/
uncertain-ACK cases from a pinned clean artifact before installing any v3 source.
If installed, use only a fresh source-only generation. Preserve failed v2 and
working v1/v2/RI/live services. Then obtain natural terminal/recovery evidence,
perform source-revision and actual worker bootstrap review, and approve the
first model date explicitly. Do not force an old manifest's calendar date or
grant clean-session credit from synthetic tests or a partial collection day.

Local checkpoint: 107 gateway library tests passed (two owned-Redis pipeline
tests excluded from the default suite and passed explicitly), and 500 runtime
library tests passed with 14 intentionally ignored integrations/exports. The
separate historical sensitivity export passed on 12 cases / six paired traces.
Supervisor 23 tests, audit 7 tests and four composed CLI smoke cases passed.
Both changed crates passed all-target clippy with warnings denied, fmt and diff
checks. Linux artifact verification and operational rollout remain separate.

Native checkpoint: clean `5e59027ede881e257eaef45e1acb793398f827c4` passed 107
gateway library tests, both explicitly invoked owned-Redis pipelines, the runtime
125/120-second model gate test, seven audit tests, 23 supervisor tests and four
CLI smoke cases. Offline rebuilt source, private network, CPU 50% / memory 2 GiB;
about 334 seconds. Running-container identity/health/restart metadata was unchanged.
Source binary SHA256:
`5a97e11fc9227e2c16802f7b595b60d526957952364004500cbe5b6809d8ee67`.

The historical price-sensitivity export was independently repeated on the native
runtime test binary in another private-network cgroup (CPU 50%, memory 1 GiB).
All six paired traces again had zero differing engine/per-bar outputs. This
repeats the narrowly scoped numerical result, not original receipt or live parity.
No v3 service was installed, no failed generation resumed, and no model admitted.
