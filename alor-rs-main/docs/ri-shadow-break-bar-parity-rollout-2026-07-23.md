# RI Operational Shadow Break-Bar Parity Rollout - 2026-07-23

## Status

`DEPLOYED / SHADOW_ONLY / LIVE RI UNCHANGED / TRANSITION P0 VERIFIED / FEED REVISION DRIFT EXPLICIT`

## Released Change

Commit `b6a8a2f` adds the MOEX break-bar guard to the RI Author41/42
operational shadow path only. The shadow feed now excludes bars in the regular
MOEX breaks `14:00:00..14:04:59` and `18:50:00..19:04:59` before model state
updates. The live RI contract is unchanged.

The guard aligns the runtime feed filter with the frozen analytic policy. It
does not by itself close two early transition-session discrepancies where
canonical07 has no BO records for:

- `2026-07-14 15:00 -> 23:00`, `+718` points;
- `2026-07-15 21:00 -> 23:00`, `+208` points.

The root cause is now localized to transition-period anchor eligibility rather
than raw bars, timestamps or the Author42 K/stop formula.

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

The canonical07 startup replay completed successfully. The raw VPS 10m bars
match the analytic input for both missing BO decisions, so this is not a
timestamp or missing-feed issue.

The operational canonical07 config applies one completeness rule to every
historical anchor session:

```text
min_anchor_bars = 92
anchor_first_bar_at_or_before = 07:10
anchor_last_bar_at_or_after = 23:30
```

Sessions before the `2026-07-14` schedule transition began around `09:00` and
therefore fail the canonical `07:10` first-bar rule even when they are valid
complete legacy sessions. Author42 requires two previous eligible daily
sessions:

- on `2026-07-14`, neither required legacy anchor passes the canonical guard;
- on `2026-07-15`, only the new `2026-07-14` session passes;
- from `2026-07-16`, two post-transition canonical sessions are available and
  BO decisions resume.

An offline reproduction with the same uniform 92-bar/07:10 guard removes
exactly the expected `2026-07-14` and `2026-07-15` BO decisions. The parity gap
is therefore at the daily-context/anchor transition layer, not in standalone
Author42 signal math.

The shadow service remains useful and safe for observation, but its historical
canonical07 attribution must not be used for a promotion decision until a
fixture-level transition patch restores the expected context.

## Updated Economic Read

Completed sessions `2026-07-14..2026-07-22`:

| Contour | MR | BO | Total |
|---|---:|---:|---:|
| Live RI / legacy09 | 9 trades, `+4104.5` | 6 trades, `+4218.0` | 15 trades, `+8322.5` |
| Operational shadow07 after break guard | 8 trades, `+1664.0` | 4 trades, `+6912.0` | 12 trades, `+8576.0` |

`2026-07-23` remains incomplete and is excluded from this comparison.
Shadow07 already has two closed MR trades totalling `+846`, but its BO session
must not be included before the session is complete.

For the directly comparable `2026-07-14..2026-07-21` interval:

```text
official analytics canonical07 = +9208
operational shadow07           = +8502
reported net gap               =  -706
missing BO rows                =  -926
```

The two missing BO rows do not by themselves reconcile the aggregate because
`-718 - 208 = -926`, while the reported total gap is `-706`. A remaining
`+220` MR/path-attribution offset must also be reconciled at decision level
before declaring exact parity.

A clean offline uniform-guard replay produces `+8284`, while the reported
operational aggregation is `+8502`, an exact residual of `+218`. This matches
the `2026-07-15 07:10 -> 07:30` MR winner. The leading operational hypothesis
is a repeated `shadow_path_active` row across restart/history warm-up. Reduce
the append-only journal to one latest path status per `decision_key` before
using it for economics.

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

1. Add a schedule-transition date and separate anchor-completeness rules:
   - pre-`2026-07-14`: valid legacy09 session, minimum 80 bars, first bar no
     later than `09:10`, last bar no earlier than `23:30`;
   - from `2026-07-14`: valid canonical07 session, minimum 92 bars, first bar no
     later than `07:10`, last bar no earlier than `23:30`.
2. Add fixture-level tests covering full inputs for `2026-07-13..2026-07-15`
   and the two expected Author42 decisions.
3. Prove auction, break, weekend and excluded-date guards still apply before
   daily context construction.
4. Journal which two dates were selected as `prev` and `prev2`, including each
   session's bar count, first/last bar and eligibility rule.
5. Recompute economics from unique `decision_key` values after applying each
   key's latest active/superseded status; verify whether this removes the
   observed `+218` duplicate-path residual.
6. Rebuild shadow history only after tests reproduce the expected paths and
   leave no unclassified decision drift.

## Transition P0 Verification and VPS Rollout

The transition-aware anchor implementation is deployed only to the VPS
`runtime-shadow07` service. It keeps the frozen signal logic unchanged and
selects the daily completeness rule by the anchor session date:

```text
before 2026-07-14:  >=80 bars, first <=09:10, last >=23:30
from   2026-07-14:  >=92 bars, first <=07:10, last >=23:30
```

Verification completed locally:

- an immutable raw 10m fixture for `2026-07-10`, `2026-07-13` through
  `2026-07-15` proves that valid legacy anchors are accepted only before the
  transition;
- the runtime fixture restores the accepted Author42 BO paths
  `2026-07-14 15:00 -> 23:00, +718` and
  `2026-07-15 21:00 -> 23:00, +208`;
- the reproducible analytic replay returns the transition-aware frozen result
  `MR +1446`, accepted `BO +7762`, combo `+9208`;
- the journal review now treats JSONL as append-only audit data and aggregates
  path economics from the latest status per `decision_key`. Replayed
  `shadow_path_active` rows after restart are reported but do not inflate
  final-active PnL.

VPS validation completed with local image
`manual-20260723-ri-shadow-transition-p0-v2`:

- only `trading-moex-early-shadow-ri-runtime-shadow07-1` was recreated and it
  is healthy;
- `2026-07-13` is now a valid legacy anchor with `86` bars from `09:00`, while
  `2026-07-14` has the canonical `98` bars from `07:00`;
- the operational journal contains both restored Author42 paths:
  `2026-07-14 15:00 -> 23:00, +718` and
  `2026-07-15 21:00 -> 23:00, +208`;
- command and gateway-blackhole streams remained empty, proving that the
  deployed contour stayed shadow-only;
- live RI, gateway, Redis and shadow09 were not restarted.

The scoped final-active journal economics for `2026-07-14..2026-07-21` are
`+9218`, rather than frozen `+9208`. The single `+10` difference is a raw-feed
revision: the VPS live bar at `2026-07-21 08:00` closed at `82550`, while the
frozen parquet used `82540`; the `10:50` exit close is `81830` in both inputs.
This is explicit feed-version drift, not a transition-guard or signal-logic
failure. Any parity claim must continue to name its exact raw-feed artifact.
