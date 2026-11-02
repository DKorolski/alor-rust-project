# WS Terminal Components: Implementation Progress

Status: first implementation slice, not a deployable WS source or live approval.
See the [approved work plan](shadow-ws-terminal-gateway-plan-2026-11-02.md).

## Implemented Components

`alor-gateway/src/terminal_bar.rs` provides a clock/network/publisher-independent
as-observed state machine. An explicit policy pins first activation, M10 session
calendar per symbol, exceptional/no-session dates, grace and bounded freshness.
Terminal fallback defaults off. Ordinary live candles release on the next label.
The legacy `Router::start` and live supervisor have NOT been rewired to this core.

The owner supplies actual monotonic/UTC clock samples, acknowledged connection
generation/heartbeat/gap evidence and an attested drained WS prefix. Terminal
release requires a real matching buffered candle after interval end plus grace.
Missing data, stale heartbeat, unresolved subscription/gap, clock steps and an
undrained queue cannot manufacture a bar. Deadline failures remain visible even
when a queue or publication is stuck.

Release is a proposal with immutable OHLCV and receipt/generation/policy proof.
No committed watermark advances until the owner calls `commit` after verifying
the exact durable append. A failed/unknown append leaves the proposal intact;
feed reset cannot discard it. Timer/next-label races and later duplicate updates
do not create another committed bar. Reconnect clears only unpublished buffers.

Valid late price/volume updates are returned as audit outcomes, without changing
canonical input. The core caches the last two committed rows per symbol; older
updates are explicitly unclassified until the outer adapter consults canonical
evidence. It does NOT silently conclude that an untracked correction was volume-only.

`restore_committed` accepts only a small verified, ordered, policy-matching durable
prefix. It restores no old live buffer, feed readiness or monotonic clock. This
API validates witnesses but is not itself a Redis reconciliation implementation.
The outer adapter remains responsible for matching the actual immutable source
rows and atomically managing write/readback/uncertain-ACK evidence.

`WsHub::start_market_data_only` reuses the maintained connection, auth, bars
subscription, heartbeat, reconnect and resubscribe mechanics. Initial/warm
subscriptions stop after bars; no portfolio/order/stop/trade subscriptions are
built. Rejected/missing-status ACKs cannot activate MD subscriptions. MD-only
shutdown attempts unsubscribe and a bounded close handshake. `WsHub::start`
still selects the existing Trading scope; its ACK/shutdown behavior is unchanged.

This API still takes the existing transport configuration type. It is a reusable
transport building block, NOT a market-data-only CLI/config runner or proof that
launching the ordinary trading supervisor would be broker-free.

## Verification Boundary

Local gateway library tests: 91 passed (68 existing, 19 finalizer, 4 MD-only).
The MD-only tests include an owned loopback WebSocket server with a cached dummy
token, ACKs/raw bars and unsubscribe/close; no real OAuth, broker or Redis calls.
They verify that default Trading subscriptions are preserved and the MD path
never requests account subscriptions. Other finalizer tests use injected clocks
and buffers, including restart/tampered witness, timer races and publish failures.

Gateway all-target clippy with `-D warnings`, formatting and diff checks are part
of the scoped verification. These checks are not whole-workspace certification,
independent frozen-model parity, target resource certification or prospective soak.
Native Linux evidence, if subsequently collected, must pin the exact clean commit,
toolchain, owned network/filesystem boundaries and test results separately.

## Remaining Before Source Admission

1. Implement the MD-only source runner/config and connect maintained WS events,
   per-symbol ACK/gap/queue evidence, History handling and this release core.
   Do not wire the live supervisor or change default `Router::start` during that
   shadow stage. The same finalizer API can later serve reviewed live wiring.
2. Implement bounded canonical Redis publication and exact-payload reconciliation,
   durable source identity/watermark/readback, audit persistence and capacity
   gates. A core `commit` test does not establish end-to-end exactly-once delivery.
3. Integrate WS source health/identity into worker supervision; add deny-by-default
   source/reader ACL tests and bootstrap inspection with actual worker binaries.
4. Prove the composed path with an owned fake WS/Redis harness, disconnect/
   publication-timeout/restart tests, then a native source-only canary and a real
   terminal cycle. Source-as-observed drift stays separate from model drift.
5. Only then admit new model generations before cutoff. The active RI collector,
   14 retained shadows, live gateway/runtime and failed REST evidence stay intact.

No new WS gateway service or model generation has been started by this component
slice. Prospective gateway/model-pair clean-session counters remain zero for this
new source. The separate obsolete REST admission timer was already cancelled;
its cancellation is not a successful replacement rollout.
