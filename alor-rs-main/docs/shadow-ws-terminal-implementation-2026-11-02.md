# WS Terminal Components: Implementation Progress

Status: composed MD source implemented; native source admission still gated.
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

The transport API still takes the existing configuration type. The new dedicated
`alor_shadow_md_ws` entrypoint enforces a market-data-only sentinel, one symbol,
M10, pinned history start/config hash and approved endpoints. Launching the
ordinary trading supervisor is NOT an equivalent broker-free deployment.

The `md_source` adapter adds bounded canonical publication, exact-ID readback for
uncertain XADD results, fsynced hash-chained journal and full Redis-prefix checks.
Late revisions persist separately with canonical binding; price changes mark
review dates without rewriting the stream. Reconnect discards uncommitted staging,
not published evidence. It never restores an old current candle as Live.

The reaction supervisor binds this source's manifest, writer lock and atomic
WS-health, requires ready/history-complete at admission, and rejects stale health
or generation rollback. See the [source runbook](shadow-md-ws-source-runbook-2026-11-02.md)
for the distinct observation/model activation dates, ACLs and recovery bounds.

## Verification Boundary

Local gateway library tests after the composed-source/ACK-order change: 96 passed.
The MD-only tests include an owned loopback WebSocket server with a cached dummy
token, ACKs/raw bars and unsubscribe/close; no real OAuth, broker or Redis calls.
They verify that default Trading subscriptions are preserved and the MD path
never requests account subscriptions. Other finalizer tests use injected clocks
and buffers, including restart/tampered witness, timer races and publish failures.

Gateway all-target clippy with `-D warnings`, formatting and diff checks are part
of the scoped verification. These checks are not whole-workspace certification,
independent frozen-model parity, target resource certification or prospective soak.
Native Linux evidence below pins the exact clean code commit, toolchain,
owned network/filesystem boundaries and test results separately.

### Historical Verified Component Smoke

Code commit `2037c1b2a5f497d3fc3013f9683a16538a1aad8e` passed all 91 gateway
library tests on native Linux x86_64 with Rust/Cargo 1.96.1. A manifest-pinned
source snapshot was built/tested under a transient cgroup: private network
namespace (owned loopback WS only), private tmp, read-only system/home, owned
Cargo/target paths, 2 GiB memory cap and 50% CPU quota. No service Redis, broker
credentials or real market-data socket was used by the tests. Existing Docker
container identities, restart counts and health state were unchanged.

Dependency preparation was a separate locked public-registry fetch in an owned
Cargo cache; Cargo.lock/source hashes remained pinned. Test execution was offline.
Build plus tests took approximately 392 seconds; child peak RSS was approximately
663 MiB. These are component-probe measurements, not a gateway runtime sizing or
steady-state capacity guarantee. Operational logs/manifests stay private.

Local legacy WS integration also passed 4 tests, with 1 existing timing-dependent
stress case ignored. Reaction-engine tests passed 11, with 2 explicit artifact
exporters ignored. Gateway all-target clippy `-D warnings`, fmt and diff checks
passed. No full-workspace/independent-model-parity claim follows from these runs.

## Remaining Before Source Admission

1. Re-run the final composed source/worker harness and native Linux checks from
   the new clean scoped commit. Local component checks include 20 supervisor
   tests and owned WS/Redis smoke with bootstrap using the actual worker,
   stop/resume, changed-prefix failure and uncertain-ACK reconciliation.
2. Run a source-only native canary and observe a real terminal cycle. Synthetic
   terminal proofs are not evidence of a successful exchange-session boundary.
   Source-as-observed drift stays separate from model drift.
3. Only then admit new model generations before cutoff. The active RI collector,
   14 retained shadows, live gateway/runtime and failed REST evidence stay intact.

An initial source-only canary rejected its admission without publishing bars:
native ALOR delivered history before its successful subscription response.
It was stopped, evidence preserved, and no model worker was admitted. The scoped
MD fix now preserves that history while waiting for the real ACK 200; it does
not activate on data alone. Default Trading handling remains unchanged. This
requires a newly verified artifact and fresh canary generation, not a silent
restart or readiness bypass. Gateway/model-pair clean-session counters remain
zero. Record later admission separately with exact artifact/health/terminal
evidence. The obsolete REST admission timer was already cancelled;
its cancellation is not a successful replacement rollout.
