# RI Operational Shadow Break-Bar Parity Rollout - 2026-07-23

## Status

`DEPLOYED / SHADOW_ONLY / LIVE RI UNCHANGED`

## Released Change

Commit `b6a8a2f` adds the MOEX break-bar guard to the RI Author41/42
operational shadow path only. The shadow feed now excludes bars in the regular
MOEX breaks `14:00:00..14:04:59` and `18:50:00..19:04:59` before model state
updates. The live RI contract is unchanged.

The guard aligns the runtime with the frozen analytic policy and closes the
operational discrepancy where the canonical07 shadow had no BO record for
`2026-07-15 21:00`.

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

The canonical07 startup journal now contains the previously missing expected
BO decision:

```text
2026-07-15 21:00
component=author42_bo
side=short
scheduled_exit=23:00
reason=time_exit_same_bar_close
shadow_pnl_points=208.0
```

Alor-USDRUBF restarted flat with no orders or stops and waits for a fresh live
bar. Its image also contains the prior BO partial-entry hardening patch.

## Verification Performed

Before rollout:

- `cargo fmt --all --check` passed;
- `cargo test -p strategy-runtime --lib` passed: `341` tests.

`cargo clippy -p strategy-runtime --lib -- -D warnings` still reports ten
pre-existing findings outside this change. They are tracked separately and did
not block this targeted safe rollout.

