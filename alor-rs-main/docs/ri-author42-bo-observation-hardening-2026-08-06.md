# RI Author42 BO Observation Hardening - 2026-08-06

## Status

Ready for controlled rollout. Do not deploy during an active MOEX session.

## Incident

On 2026-08-05 the RI live contour emitted an Author42 BO entry at the 23:00
exit boundary and immediately emitted the matching EOD exit. The broker accepted
both action-scoped market commands. This was not a CWS or gateway failure.

The live BO level was calculated from the 10:00 bar (`90140`) instead of the
canonical sixth model bar at 09:50 (`90120`). At 09:20 the Author41 MR branch had
entered `live_pending_entry`; the shared runtime phase caused the Author42 live
adapter to return before updating its independent observation state. The skipped
bar shifted the BO first-hour index by one.

## Patch Scope

1. Advance Author42 session observations on every eligible canonical 10-minute
   bar, including bars received while Author41 MR is pending entry or exit.
2. Keep BO signal detection and broker emission behind the existing shared
   lifecycle and no-overlap guards.
3. Drop a pending BO entry when its execution bar is at or after
   `author42_exit_time` and emit `ri_bo_entry_suppressed` diagnostics.
4. Preserve the frozen Author41/Author42 formulas, `K=0.42`, EOD `23:00`,
   quantity, rollover rules, and action-scoped market execution path.

## Acceptance Checks

- An MR pending phase on the 09:20 bar does not change the sixth BO observation;
  the reproduced 2026-08-05 long level remains `90120`.
- The 21:50 close at `90130` creates the expected BO long pending action and the
  22:00 bar emits the entry.
- A pending entry presented to the 23:00 bar emits no broker intent, creates no
  live BO position, and is cleared with an operator-visible suppression event.
- Existing Author42 and transition-anchor fixtures remain green.
- Full workspace tests, `cargo fmt --all -- --check`, and
  `cargo clippy -- -D warnings` pass before rollout.

Validation completed on 2026-08-06: all `361` strategy-runtime unit tests and
the remaining workspace integration/doc tests passed; fmt and clippy were clean.

The regression set includes both a focused BO state test and a shared-path test
where a real Author41 MR signal at 09:20 enters `live_pending_entry` through
`live_intents_for_bar`; the Author42 sixth-bar level still resolves to `90120`.

## Rollout Gate

Roll out only in a controlled flat window after the trading session or before a
future session:

- RI, USDRUBF, and IMOEXF broker positions are flat;
- no working orders or stop orders remain;
- build and deploy only the patched RI runtime image/service;
- keep the RI gateway, Redis history, strategy parameters, and quantity intact;
- verify startup replay suppression, `LiveReady`, broker-flat reconciliation,
  and absence of immediate intents from replayed history.

The 2026-08-06 pre-open window was already too short once the root cause and
tests were complete. The safe decision is to validate now and deploy at the next
flat maintenance window, not to restart the contour at session open.

## Post-Rollout Observation

For at least five clean sessions, compare live Author42 observation levels and
entry timestamps with the model/shadow decision journal. Alert on:

- `ri_bo_entry_suppressed`;
- entry and exit intents generated from the same model bar;
- BO level drift from the canonical sixth session bar;
- broker position or pending lifecycle remaining non-flat after EOD.

## Follow-up: Pending Entry TTL

The current BO pending-entry value stores only the side. The EOD guard is enough
for the 2026-08-05 P0 incident, but it is not a general stale-entry TTL. In a
separate follow-up patch, store the expected next canonical bar timestamp with
the pending side and emit only when the observed bar matches that timestamp.
Older pending entries must be cleared with an explicit expiry diagnostic. This
change is intentionally excluded from the current minimal rollout so that its
execution-contract effect can be reviewed and replay-tested independently.
