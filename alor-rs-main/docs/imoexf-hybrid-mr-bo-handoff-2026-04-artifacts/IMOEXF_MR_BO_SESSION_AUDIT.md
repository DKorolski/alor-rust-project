# IMOEXF MR/BO Session Contract Audit

Date: 2026-04-26

## Scope

Audited artifact:

```text
replay_trades.csv
```

Primary model/scenario:

```text
model_id = hybrid_mr_riskgate_high180_lb120__bo_new_k053
scenario = base_realistic
```

Stress scenarios were checked for the same structural issues.

## Checks

### MR Entry Window

Result: `PASS`

For the primary model under `base_realistic`:

```text
MR trades = 474
MR entry-window violations = 0
MR weekend entry/exit violations = 0
MR entry hours = 09, 10, 11 only
```

MR entries are restricted to the morning window. MR exits may occur after noon
because the selected MR contour allows `max_hold_minutes = 180`.

Observed MR exit hours:

```text
09: 73
10: 156
11: 93
12: 54
13: 62
14: 36
```

Interpretation: the MR signal window closes correctly, but MR positions can
remain open into early afternoon by design.

### Hybrid MR/BO Overlap

Result: `PASS`

For the primary model under `base_realistic`:

```text
hybrid trades = 844
MR trades = 474
BO trades = 370
accepted-position overlaps = 0
```

The no-overlap merge is active. When MR and BO compete, MR has priority and BO
is skipped.

Standalone `bo_new_k053` had `375` BO candidates; the hybrid accepted `370`.
The `5` skipped BO candidates were blocked by an open MR position.

### BO Weekend Data Use

Result: `ANCHOR_USE_PASS / PENDING_BO_GAP_FLATTEN_PARITY_CHECK`

The Python BO engine is configured with:

```text
bo_exclude_weekends = true
```

and `DayBreakoutWaitFixEngine.on_bar()` returns immediately on Saturday/Sunday.
Therefore weekend bars do not update BO `yesterday_close`,
`yesterday_high/low/range`, or BO entry/exit decisions.

However, the current Backtrader-style artifact still contains BO positions that
carry across non-trading gaps because `close()` submitted on the EOD bar fills
on the next available bar. This is a replay fill-semantics nuance that must be
checked explicitly before parity acceptance.

For the primary model:

```text
BO weekend entries = 0
BO weekend exits = 0
BO trades crossing a weekend = 1
BO non-same-day exits = 9
```

The weekend-crossing trade is:

```text
entry_ts = 2023-11-17 17:00:00
exit_ts = 2023-11-20 09:00:00
side = long
```

The same structural weekend-crossing issue appears in:

```text
base_realistic: 1
stress_1tick: 1
conservative_2tick: 1
```

## Required Parity Check Before Replay Acceptance

The model package should not be promoted unless the Rust replay/runtime contract
proves BO cannot carry across a non-tradable gap under the selected execution
semantics.

Required runtime/replay assert:

- BO must be flat before any non-tradable gap.
- If `force_exit_time = 23:30`, the BO position must be closed no later than the
  last regular weekday bar or next regular runtime bar/event.
- If the configured `23:30` EOD exit signal is generated and there is no later
  same-day fill bar, Rust may flatten through the bar/event no-overnight guard
  rather than reproducing Backtrader's next-bar fill.
- Saturday/Sunday bars must remain audit-visible but must not become trading,
  exit, or anchor bars.
- Runtime timer/event-loop flattening without a later bar/event is a follow-up
  `nice to have`, not a blocker for this parity checkpoint.

## Practical Verdict

MR window behavior and MR/BO no-overlap behavior are acceptable.

BO weekend anchor use is acceptable in the current engine. The remaining item is
BO gap-flatten parity: Backtrader can carry because of next-bar fill semantics,
while Rust/barter runtime may correctly flatten through its bar/event
no-overnight guard. A stricter timer hook can be added later.
The replay contract should be treated as `PENDING_BO_GAP_FLATTEN_PARITY_CHECK`
before final parity review.
