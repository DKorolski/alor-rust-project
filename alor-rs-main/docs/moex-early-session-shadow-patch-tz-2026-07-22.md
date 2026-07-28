# TZ: MOEX Early-Session Shadow Patch and Extended A/B

Date: `2026-07-22`

Status:

```text
READY_FOR_ENGINEERING
SHADOW_ONLY
LIVE09_UNCHANGED
NO_ALPHA_RETUNE
```

## 1. Scope

This document is a focused implementation patch to:

```text
docs/moex-early-session-shadow-contours-tz-2026-07-17.md
```

It covers the `07:00` session recut for:

- IMOEXF Hybrid Riskgate High180;
- IMOEXF Author41-short Hybrid diagnostic;
- RI Author41/42;
- Alor-USDRUBF Hybrid `challenger_mr035`.

The goal is to make the already prepared `legacy09` and `canonical07` shadow
contours faithful to their declared session contracts and suitable for an
extended A/B observation. This work does not authorize a live clock change.

## 2. Evidence Available at the Start of Work

Offline A/B replay used complete weekday sessions from `2026-07-14` through
`2026-07-21`. The partial `2026-07-22` session was excluded. Frozen K values,
stops, targets, cost assumptions and component arbitration were not retuned.

### 2.1. IMOEXF

Native points per contract:

| Model | Policy | Trades | Net points | Wins/losses | Worst trade |
|---|---|---:|---:|---:|---:|
| High180 hybrid | `legacy09` | 5 | 90.926 | 5/0 | 5.141 |
| High180 hybrid | `canonical07_phase10` | 5 | 99.425 | 4/1 | -8.858 |
| High180 hybrid | current unpatched `shadow07` behavior | 6 | 103.325 | 5/1 | -8.858 |
| Author41-short hybrid | `legacy09` | 7 | 94.456 | 7/0 | 4.940 |
| Author41-short hybrid | `canonical07_phase10` | 6 | 111.975 | 5/1 | -8.858 |
| Author41-short hybrid | current unpatched `shadow07` behavior | 7 | 117.375 | 6/1 | -8.858 |

Critical finding: `mr_session_end_time=09:59` is present in the current
`shadow07` TOML, but both MR override implementations bypass it. High180 uses
`High180MrConfig::default()` with an `11:59:59` cutoff; Author41-short uses a
hard-coded `12:00` cutoff. The current service is therefore not a true
phase-translated MR shadow.

Source report:

```text
analiz_alpha_si/imoexf_session_recut_2026_07_22/imoexf_session_recut_report.md
```

### 2.2. RI

Author41/42 no-overlap combo, native RI points per contract:

| Policy | Trades | Net points | Wins/losses | Worst trade |
|---|---:|---:|---:|---:|
| `legacy09` | 11 | 6280.5 | 9/2 | -2639.5 |
| `canonical07_phase10` | 11 | 9208.0 | 9/2 | -1342.0 |
| `canonical07_compromise11` | 9 | 2364.5 | 5/4 | -2639.5 |
| `canonical07_liquidity12` | 13 | 5026.5 | 8/5 | -2639.5 |

The primary `canonical07_phase10` result is promising but not confirmatory. It
is materially influenced by large Author42 BO contributions on `2026-07-16`
and `2026-07-20`. Early-session volume was only about `2.6%..6.7%` of daily
volume, but the early range added `310`, `610` and `810` RI points to the full
session range on the first three complete sessions. The session recut therefore
changes model state even when liquidity is relatively low.

### 2.3. USDRUBF

Hybrid combo, native price points per contract:

| Policy | Trades | Net points | Wins/losses | Worst trade |
|---|---:|---:|---:|---:|
| `legacy09` | 8 | 1.0441 | 4/4 | -0.3157 |
| `canonical07_phase09` | 8 | 1.1264 | 4/4 | -0.2662 |
| `model08_compromise10` | 7 | 1.0829 | 4/3 | -0.3157 |

The three results are economically close. Early volume was about `1.1%..4.0%`
of daily volume and usually did not expand the later daily range. The current
evidence supports continued A/B observation, not a live change or a K retune.

RI and USDRUBF source report:

```text
analiz_alpha_si/moex_ri_usdrubf_session_recut_2026_07_22/ri_usdrubf_session_recut_report.md
```

## 3. Governed Decision

The engineering target is:

```text
KEEP CURRENT LIVE/MICRO ON LEGACY09
FIX IMOEXF SHADOW WINDOW ENFORCEMENT
CONTINUE ISOLATED LEGACY09 VS CANONICAL07 SHADOW
DO NOT SELECT A NEW WINDOW FROM SIX SESSIONS
```

Allowed semantic changes are limited to session-policy configuration,
configurable window enforcement, shadow isolation, decision journaling and
tests. K values, stops, targets, costs, side logic, seasonal rules, overlap
rules and live position size must remain unchanged.

## 4. Common Session and Feed Contract

All canonical shadows must apply this guard before any strategy, anchor,
risk-gate or component state update:

```text
timezone                 = Europe/Moscow
weekday model start      = 07:00:00
weekday model end        = 23:49:59
06:50 auction bar        = excluded
weekend bars             = excluded
10m input                = closed bars only
timestamp convention     = normalized and declared at startup
Monday previous session  = Friday, never a weekend session
```

Existing breaks remain unchanged for this A/B. A separate experiment is
required to remove or alter a break.

Every runtime must log the resolved values at startup:

```text
session_start
session_end
mr_entry_start
mr_entry_end
mr_force_or_time_exit
bo_wait_hours
bo_first_eligibility
bo_eod_exit
weekends_off
auction_bar_policy
component_arbitration
config_hash
```

Startup must fail if an expected phase relation is violated. In particular,
the IMOEXF phase-recut MR entry window must be closed before its BO eligibility.

## 5. P0: IMOEXF MR Override Window Patch

### 5.1. Problem

The generic MR configuration reads `mr_session_end_time`, but the active
High180 and Author41-short override paths do not consistently use it:

```text
strategy-runtime/src/strategies/hybrid_intraday/high180.rs
strategy-runtime/src/strategies/hybrid_intraday_runtime.rs
```

Changing only the TOML currently creates a misleading `shadow07` identity.

### 5.2. Required implementation

Use the resolved `mr_session_end_time` as the entry cutoff for every selected
MR variant:

- construct both `high180_mr` and `risk_gate_shadow_mr` with a
  `High180MrConfig` whose `entry_end_time` equals the resolved runtime cutoff;
- replace the Author41-short hard-coded `12:00` entry cutoff with the same
  resolved runtime cutoff;
- keep Author41's `20:00` time exit unchanged;
- keep High180 max-hold and bracket levels unchanged;
- do not alter the generic baseline MR behavior;
- journal the resolved cutoff with each MR eligibility decision.

The preferred implementation is one authoritative runtime MR entry cutoff,
not separate hidden cutoffs for each variant.

### 5.3. Required IMOEXF contours

Primary High180 pair:

| Contour | State start | MR entry end | BO wait | First BO eligibility |
|---|---:|---:|---:|---:|
| `high180_legacy09` | 09:00 | 11:59 | 3h | 12:00 |
| `high180_canonical07_phase10` | 07:00 | 09:59 | 3h | 10:00 |

Author41-short diagnostic pair, cloned from the active Author41-short profile:

| Contour | State start | MR entry end | BO wait | First BO eligibility |
|---|---:|---:|---:|---:|
| `author41_legacy09` | 09:00 | 12:00 | 3h | 12:00 |
| `author41_canonical07_phase10` | 07:00 | 10:00 | 3h | 10:00 |

The Author41 pair is a separate diagnostic experiment. It must have separate
strategy IDs, consumer groups, state keys and reports. It must not share the
High180 risk-gate ledger.

### 5.4. Risk-gate isolation

High180 `canonical07` must use a dedicated ledger. The pre-change seed may be
used only as explicitly labelled historical initialization through
`2026-07-13`. From `2026-07-14`, shadow PnL updates the canonical ledger only.

Because the A/B services run in `trade_mode = paper`, runtime ledger appends
must be enabled only through an explicit shadow flag. Ordinary paper/backtest
contours must continue to avoid Redis risk-gate writes. The intended config
shape is:

```text
risk_gate_persist_in_shadow = true
```

only on the isolated High180 risk-gate shadow contours that own their dedicated
ledger streams.

Journal both:

- raw High180 MR decision before the gate;
- rolling 120-session value;
- allow/block result;
- accepted or suppressed model decision.

## 6. P1: RI Shadow Hardening

The required TOMLs already expose the relevant clocks:

```text
configs/runtime.ri_author41_42.shadow09.7502MIW.toml
configs/runtime.ri_author41_42.shadow07.7502MIW.toml
```

Required primary pair:

| Contour | State/signal start | Author41 entry end | Author42 first hour | Author42 first check |
|---|---:|---:|---:|---:|
| `legacy09` | 09:00 | 12:00 | 09:00..09:50 | 10:50 |
| `canonical07_phase10` | 07:00 | 10:00 | 07:00..07:50 | 08:50 |

No RI alpha code change is required if replay proves the TOML values are
actually consumed. Engineering must nevertheless add or verify the following:

- startup log of resolved Author41 and Author42 clocks;
- journal of the six bars used for the Author42 first-hour levels;
- first-hour high/low, trigger levels and each hourly check timestamp;
- source-bar timestamp, signal timestamp and scheduled next-bar entry timestamp;
- strict no-overlap with MR priority, unchanged from the frozen combo;
- explicit suppression journal when a BO candidate overlaps an MR position;
- anchor guard of at least `92` expected bars with first bar no later than
  `07:10` and last bar after `23:30` for a normal canonical session;
- existing RI rollover and excluded-session guards unchanged.

Anchor validation must be schedule-transition aware. Do not apply the
canonical 92-bar/07:10 rule retrospectively to valid pre-`2026-07-14` legacy
sessions. The required rules are:

```text
session date < 2026-07-14:
  minimum bars = 80
  first bar <= 09:10
  last bar >= 23:30

session date >= 2026-07-14:
  minimum bars = 92
  first bar <= 07:10
  last bar >= 23:30
```

Author42 context journaling must identify the exact `prev` and `prev2`
sessions and the rule under which each was accepted. This transition contract
is required to reproduce the expected `2026-07-14 15:00` and
`2026-07-15 21:00` BO decisions.

The `08:00/11:00` and `09:00/12:00` variants remain offline sensitivity rows.
Do not deploy them as additional VPS services during the first extended A/B.

### 6.1. Mandatory MR/BO arbitration diagnostic

The six-session combo advantage must not be described as standalone BO
improvement. Standalone Author42 BO produced:

```text
canonical07_phase10 = +6756.0 points
legacy09            = +7230.0 points
```

The accepted combo BO attribution became `+7762.0` versus `+4142.0` because of
MR-priority/no-overlap:

- on `2026-07-20`, legacy09 retained an MR short losing `-2639.5` points and
  dropped the overlapping `15:00` BO long that made `+3088.0` standalone;
- on `2026-07-15`, canonical07 retained an MR long losing `-1342.0` points and
  dropped three overlapping BO shorts totalling `-1006.0` standalone.

Before any RI session or arbitration promotion, replay two predeclared modes:

```text
strict_no_overlap_mr_priority
opposite_bo_priority_handoff_diagnostic
```

The challenger is decision-only. When a confirmed next-bar Author42 entry is
opposite an active MR position, record a hypothetical MR close followed by a
BO entry using the existing next-bar contract. Do not synthesize an intrabar or
same-bar fill. Same-side BO candidates remain separately classified and must
not be silently treated as a reversal.

Run the comparison over the full frozen RI history and the post-`2026-07-14`
session-recut sample. Report MR PnL surrendered, BO PnL recovered, turnover,
drawdown, tail losses, handoff count and cost stress. This diagnostic does not
change the frozen live no-overlap contract.

#### Resolution path for missing transition BO decisions

The frozen canonical07 replay contains accepted Author42 decisions on:

```text
2026-07-14 15:00 -> 23:00  +718
2026-07-15 21:00 -> 23:00  +208
```

Raw VPS 10m bars match the analytical input. The break-bar guard is required
and remains in force, but it does not explain the two remaining missing rows.

The operational config applies the post-transition `92 bars / first <= 07:10`
anchor rule to pre-transition sessions that validly began around `09:00`.
Author42 requires both `prev` and `prev2`; it therefore has insufficient
eligible daily context on 14 and 15 July. A transition-aware guard restores
both decisions and exact frozen analytics:

```text
uniform canonical guard combo = +8284
transition-aware guard combo  = +9208
```

The P0 patch must use legacy completeness rules before `2026-07-14` and
canonical completeness rules from that date onward. It must not alter
Author42 K, stops or hourly trigger logic.

Journal aggregation must also deduplicate repeated path events by
`decision_key`. Runtime restart/history warm-up may append
`shadow_path_active` again for an existing key because the in-memory path
index is rebuilt. Economics use the latest active/superseded status and count
each final-active key once.

## 7. P1: Alor-USDRUBF Shadow Hardening

The required TOMLs already exist:

```text
configs/runtime.alor_usdrubf.shadow09.7502MIW.toml
configs/runtime.alor_usdrubf.shadow07.7502MIW.toml
```

Required pair:

| Contour | State start | MR last entry | MR forced exit | BO wait | First BO eligibility |
|---|---:|---:|---:|---:|---:|
| `legacy09` | 09:00 | 11:40 | 11:50 | 2h | 11:00 |
| `canonical07_phase09` | 07:00 | 09:40 | 09:50 | 2h | 09:00 |

The current Rust configuration path already consumes the MR cutoff and BO wait
fields. Required work is therefore replay/journal hardening, not alpha redesign:

- log resolved window values at startup;
- journal current-day session open and prior-session range;
- journal raw MR and BO candidates even when arbitration suppresses them;
- report whether a decision originated before or after `09:00`;
- preserve closed-bar/next-bar decision semantics;
- keep weekend bars out of model state;
- keep MR bracket execution disabled in shadow and record theoretical TP/SL
  only;
- keep `model08_compromise10` offline-only.

Do not combine this session experiment with the open live MR bracket
partial-fill/reconciliation patch. They are separate work items.

## 8. Decision Journal and A/B Diff

The journal schema from the base TZ remains mandatory. Add these fields if not
already present:

```text
resolved_mr_entry_end
resolved_bo_first_eligibility
opening_range_start
opening_range_end
opening_range_bar_count
opening_range_high
opening_range_low
raw_component_candidate
suppression_reason
risk_gate_ledger_key
risk_gate_rolling_value
pre_09_flag
```

The daily A/B diff must classify divergences as:

```text
same_decision
clock_shift_only
session_open_drift
previous_range_drift
trigger_level_drift
side_changed
legacy_only
canonical_only
overlap_suppressed
risk_gate_changed
exit_path_changed
feed_quality_difference
```

For every divergence, retain both source rows rather than only a summary count.

## 9. Required Tests

### 9.1. Common tests

1. Reject `06:50` before every model-state update.
2. Accept the closed `07:00` bar in canonical contours only.
3. Reject weekends from anchors, signals and risk-gate sessions.
4. Keep EOD and late-day exits at their frozen absolute times.
5. Restore identical state and decisions after a mid-session restart.
6. Prove all shadow services are unable to emit broker commands.
7. Prove `legacy09` golden decisions remain unchanged.

### 9.2. IMOEXF regression tests

1. High180 emits no new MR entry after `09:59` under canonical phase10.
2. Author41-short emits no new MR entry after `10:00` under canonical phase10.
3. The risk-gate shadow High180 engine uses the same cutoff as the tradable
   High180 engine.
4. Legacy cutoffs remain `11:59:59` and `12:00` respectively.
5. BO wait3 begins at the selected model-session start.
6. Startup rejects a canonical phase10 config whose MR window overlaps BO.

### 9.3. RI tests

1. Canonical Author42 levels use exactly the `07:00..07:50` closed bars.
2. First hourly check occurs at `08:50`; earliest scheduled next-bar entry is
   `09:00`.
3. Author41 emits no new entry after `10:00` in canonical phase10.
4. MR priority and no-overlap are unchanged.
5. Anchor completeness uses the canonical session, not the old 80-bar rule.

### 9.4. USDRUBF tests

1. BO first eligibility is `09:00` in canonical and `11:00` in legacy.
2. Canonical MR last entry/force exit are `09:40/09:50`.
3. No weekend bar becomes Monday's previous-session anchor.
4. Closed-bar and next-bar timestamps are both present in the journal.

## 10. Replay Acceptance

Run Rust replay for every complete session from `2026-07-14` through the latest
complete available weekday and compare it with the committed Python runners.

Required artifacts:

```text
moex_early_session_replay_manifest.csv
moex_early_session_decision_diff.csv
moex_early_session_daily_by_component.csv
moex_early_session_window_resolution.csv
moex_early_session_feed_quality.csv
moex_early_session_replay_report.md
```

Acceptance conditions:

```text
legacy09_no_regression = true
canonical_clock_tests = pass
python_rust_decision_parity = explained or exact
unclassified_decision_drift = 0
shadow_order_emission = 0
```

Price differences caused only by explicitly different execution simulators may
be reported separately, but signal side, source bar, component ownership and
window eligibility must not remain unexplained.

## 11. Extended Observation

Minimum observation after the IMOEXF patch and clean service restart:

```text
preliminary review = 10 complete weekdays
decision review    = 20 complete weekdays
preferred review   = 30 complete weekdays
```

Continue beyond 20 sessions if any primary contour has fewer than five BO
decisions. Do not reset the observation clock for a reporting-only change; do
reset it for a semantic model/window change.

Report by strategy and component:

- trade and decision count;
- net native points and estimated ruble PnL;
- win/loss count;
- worst trade and maximum drawdown;
- top-one and top-three day concentration;
- pre-09:00 decision count and PnL;
- legacy-only and canonical-only decisions;
- side changes;
- anchor/range changes;
- suppressed overlaps;
- risk-gate allow/block counts;
- estimated slippage by time bucket when observable.

## 12. Promotion Gate

No contour may move to micro/live from this task. A later promotion review must
show:

1. deterministic replay and restart parity;
2. no unexplained signal drift;
3. sufficient decisions across more than a few dominant trend days;
4. acceptable pre-09:00 liquidity and slippage;
5. component-level result, not combo PnL alone;
6. stable no-overlap behavior;
7. isolated and valid risk-gate history;
8. a newly frozen session contract.

Allowed later verdicts:

```text
KEEP_LEGACY09
EXTEND_SHADOW
PROMOTE_CANONICAL07_MR_ONLY_TO_MICRO_CANDIDATE
PROMOTE_CANONICAL07_BO_ONLY_TO_MICRO_CANDIDATE
PROMOTE_CANONICAL07_COMBO_TO_MICRO_CANDIDATE
REJECT_CANONICAL07
```

## 13. Engineering Completion Criteria

The patch is complete when:

1. IMOEXF MR override cutoffs are config-driven and covered by tests.
2. High180 and Author41 canonical shadows enforce the declared phase window.
3. RI and USDRUBF startup logs expose all resolved model clocks.
4. All primary shadow pairs have isolated state, streams and reports.
5. Every contour uses the canonical feed guard before model updates.
6. Rust replay has no unexplained decision-level drift against the Python A/B.
7. Legacy09 golden behavior is unchanged.
8. No shadow path can emit an order.
9. Extended observation reports are reproducible from committed artifacts.

## 14. Practical Priority

Implementation order:

```text
P0  patch IMOEXF MR override cutoff and add regression tests
P0  replay IMOEXF High180 and Author41 shadow pairs
P1  verify RI resolved clocks and Author42 first-hour journal
P1  verify USDRUBF resolved clocks and component journal
P1  deploy isolated primary shadow pairs
P2  collect 20-30 complete weekday sessions
P3  hold a separate session-freeze promotion review
```

The short A/B currently supports proceeding with engineering and shadow
observation. It does not support changing the live model start from `09:00` to
`07:00` yet.
