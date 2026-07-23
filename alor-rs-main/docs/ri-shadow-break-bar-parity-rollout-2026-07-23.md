# RI Operational Shadow Break-Bar Parity Rollout - 2026-07-23

## Status

`DEPLOYED / SHADOW_ONLY / LIVE RI UNCHANGED / PARITY INVESTIGATION OPEN`

## Released Change

Commit `b6a8a2f` adds the MOEX break-bar guard to the RI Author41/42
operational shadow path only. The shadow feed now excludes bars in the regular
MOEX breaks `14:00:00..14:04:59` and `18:50:00..19:04:59` before model state
updates. The live RI contract is unchanged.

The guard aligns the runtime feed filter with the frozen analytic policy. It
does not by itself close the operational discrepancy where canonical07 has no
BO record for `2026-07-15 21:00`; that remaining Rust/Python Author42 parity
gap is tracked below.

## VPS Rollout

The deployment was performed at a confirmed flat boundary:

- `RTS-9.26`, `IMOEXF`, and `USDRUBF` broker positions were zero;
- no working or stop orders remained;
- the RI live runtime, gateways, and Redis were not restarted.

The following runtimes were recreated with local immutable image tag
`manual-20260723-ri-shadow-usdrubf-b6a8a2f`:

- `trading-moex-early-shadow-ri-runtime-shadow07-1`;
- `trading-moex-early-shadow-ri-runtime-shadow09-1`;
- `trading-alor-usdrubf-strategy-runtime-1`.

The image is present locally on the VPS as
`sha256:e56b839d4c810c185296ba6e703f5a4a5629e744b431d3707a39358e339eef8d`.
Publishing this tag to GHCR was unavailable because the VPS has no registry
credentials. This does not affect the applied local rollout, but the image
should be published from an authenticated CI or operator workstation before a
future host rebuild.

## Validation

All recreated containers became healthy. Both RI shadow command streams and
the shadow gateway blackhole stream were empty after startup, confirming that
the shadows remain non-trading.

The canonical07 startup replay completed successfully, but a follow-up
economic recomputation found that the expected `2026-07-15 21:00` Author42 BO
record is still absent. The raw VPS 10m bars match the analytic input around
that time, so this is not a timestamp or missing-feed issue. It is a remaining
Rust/Python Author42 replay/journal parity issue.

The shadow service remains useful and safe for observation, but its historical
canonical07 BO attribution must not be used for a promotion decision until a
fixture-level parity patch restores or formally explains this row.

Alor-USDRUBF restarted flat with no orders or stops and waits for a fresh live
bar. Its image also contains the prior BO partial-entry hardening patch.

## Verification Performed

Before rollout:

- `cargo fmt --all --check` passed;
- `cargo test -p strategy-runtime --lib` passed: `341` tests.

`cargo clippy -p strategy-runtime --lib -- -D warnings` still reports ten
pre-existing findings outside this change. They are tracked separately and did
not block this targeted safe rollout.

## Follow-Up Required

1. Add a fixture-level Rust/Python parity test for the verified `2026-07-15`
   canonical07 input and expected Author42 path.
2. Journal Author42 level construction, hourly checks and candidate suppression
   reasons so the next mismatch is observable without offline reconstruction.
3. Rebuild the shadow history only after the test proves the expected late BO
   is either reproduced or explicitly classified as an intentional contract
   difference.
