# TZ: MOEX Early-Session Shadow Contours

Date: `2026-07-17`

Status:

```text
READY_FOR_ENGINEERING
SHADOW_ONLY
NO_LIVE_CONFIG_CHANGE
NO_ORDER_EMISSION
```

Implementation addendum after the first offline `2026-07-14..2026-07-21` A/B:

```text
docs/moex-early-session-shadow-patch-tz-2026-07-22.md
```

The addendum is authoritative for the IMOEXF MR override-cutoff patch, current
replay evidence and the extended observation gate. This document remains the
base safety and deployment contract.

## 1. Work Name

MOEX early-session recut: deploy isolated `07:00` shadow contours for:

- RI Author41/42;
- Alor-USDRUBF Hybrid;
- IMOEXF Hybrid Riskgate.

## 2. Goal

Build a safe A/B observation layer for the MOEX futures session introduced on
`2026-07-14` and answer the following question:

> Do the existing MR/BO systems retain useful and operationally coherent model
> behavior when their opening context starts at the beginning of continuous
> trading at `07:00 MSK`, with opening-phase clocks translated by two hours and
> without retuning model parameters?

The work must not modify the currently running `09:00` micro/live contours.

## 3. Exchange Session Contract

The Moscow Exchange schedule effective `2026-07-14` is:

```text
06:50-07:00  opening auction
07:00-10:00  morning trading period
10:00-19:00  main trading period
19:00-23:50  evening trading period
```

Primary source:

- https://www.moex.com/n101220

Model policy for this work:

```text
timezone                    = Europe/Moscow / UTC+3
model_continuous_start      = 07:00:00
model_session_end           = 23:49:59
opening_auction             = excluded
weekends                    = excluded
timeframe                   = canonical closed 10m bars
```

The `06:50` auction candle must not update:

- session open;
- intraday high/low/range;
- MR or BO state;
- previous-session anchors;
- risk-gate state;
- signal or exit state.

For start-stamped 10m bars, the valid continuous-session set begins with the
`07:00` bar and ends with the `23:40` bar. If a data source uses close-stamped
bars, normalize timestamps before applying the model guard.

Weekend sessions remain out of scope, including weekend trading in currency
futures. A weekend bar must not become the previous-session anchor for Monday.

Existing intraday break exclusions must remain unchanged in the first A/B run.
Whether those exclusions are still necessary under the new exchange schedule is
a separate session-policy change and must not be mixed into this experiment.

## 4. Main Design Decision

The primary challenger is not `07:00 data + old absolute opening windows`.
It is a deterministic translation of the opening phase:

```text
old model start = 09:00
new model start = 07:00
opening-phase shift = -2 hours
```

Rules:

1. Session open, current-day state, MR state and BO state start at `07:00`.
2. Relative BO waits retain their numeric value and are counted from `07:00`.
3. Absolute opening-window cutoffs move two hours earlier to preserve the same
   elapsed opening-phase duration and the same MR/BO phase relationship.
4. EOD, session-close and late-day safety exits remain at their existing clock
   time because the exchange close did not move.
5. K values, stops, targets, range filters, costs, side rules, overlap rules,
   seasonal filters and position size are not retuned.

This translation is the primary `canonical07` contract. It is a new shadow
contract and must not be described as parity with the historical `09:00`
freeze.

## 5. Required Contours

For every strategy, run two isolated decision-only contours against the same
closed-bar source:

| Contour | State start | Opening clocks | Purpose |
|---|---:|---|---|
| `legacy09` | `09:00` | current active contract | clean reference |
| `canonical07` | `07:00` | opening phase shifted by `-2h` | primary challenger |

The existing micro/live decision journal may be used as supporting evidence,
but it must not replace the dedicated `legacy09` shadow reference: broker
readiness, pending orders and execution lifecycle can suppress or delay live
decisions.

Optional diagnostic, only if the primary comparison needs attribution:

| Contour | State start | BO clock | Purpose |
|---|---:|---|---|
| `canonical07_legacy_bo_clock` | `07:00` | old absolute BO eligibility | separate anchor drift from BO-clock drift |

This optional contour is not a promotion candidate.

## 6. Safety Contract

Every new contour must be fail-closed:

```text
trade_mode                 = paper or shadow
allow_live_orders          = false
allow_paper_orders         = false
strategy_order_emission    = false
broker_command_writes      = forbidden
decision_journal           = enabled
```

Additional requirements:

- use unique consumer groups;
- use unique runtime-state keys;
- use unique journal and report paths;
- never reuse live pending state;
- never publish to a production command stream;
- do not attach an order-capable gateway command consumer;
- expose a separate health endpoint or unambiguous service health label;
- log the resolved session contract and config hash at startup.

Preferred feed architecture:

```text
read-only 07:00 market-data publisher
  -> isolated shadow bar streams
  -> legacy09 runtime guard
  -> canonical07 runtime guard
```

If the current gateway cannot be made demonstrably market-data-only, add that
mode before deployment. Merely using an unused command stream is not sufficient
unless all command classes are rejected and this is covered by a test.

Do not broaden a production gateway feed as part of the first rollout unless a
test proves that all active live runtimes reject pre-09:00 bars before every
model-state update.

## 7. Active-Config Freeze Before Work

Before creating challenger configs, export the actually deployed config for
each active contour and record:

- host path;
- image tag and Git commit;
- config SHA-256;
- symbol and order symbol;
- profile/model ID;
- all MR and BO parameters;
- session and break policy;
- current risk-gate seed and ledger identity;
- current component arbitration mode.

Deliverable:

```text
moex_early_session_active_config_manifest.csv
```

The `canonical07` config must be generated as a controlled diff from this
snapshot. Allowed semantic differences are limited to:

- model/feed session start;
- translated opening-phase cutoffs listed below;
- anchor completeness guard for the new start;
- contour/profile IDs;
- streams, state keys, journals and health ports;
- forced disabling of order emission.

## 8. Per-Strategy Requirements

### 8.1. RI Author41/42

Baseline profile:

```text
ri_author41_42_primary_combo_cost2
timeframe = 10m
components = author41_mr + author42_bo
arbitration = no-overlap, MR priority
```

Required `canonical07` behavior:

```text
model session start         09:00 -> 07:00
Author41 entry_end          12:00 -> 10:00
Author41 time_exit          unchanged at 20:00
Author42 exit_time          unchanged at 23:00
Author42 first-hour state   07:00..07:50
previous-session anchor     full valid 07:00..23:49 session
```

Author42 expected clock sequence with current implementation semantics:

```text
07:00  first model bar
07:50  sixth 10m bar; first-hour levels become available
08:50  first valid hourly BO check
09:00  earliest scheduled next-bar model entry
```

The exact broker execution delay is out of scope because this contour emits no
orders. The journal must still distinguish:

- source bar timestamp;
- signal timestamp;
- scheduled model-entry timestamp.

Current technical blocker:

`ModelProfile::ri_shadow_10m()` currently constructs a hard-coded
`RegularSessionPolicy::moex_10m()` starting at `09:00`. Changing only TOML
`trading_periods` is therefore insufficient. The session policy must become an
explicit profile/runtime input without changing the `legacy09` default.

RI acceptance tests must prove:

- `06:50` is rejected and `07:00` is accepted in `canonical07`;
- `legacy09` replay is unchanged bit-for-bit;
- the Author42 clock sequence above is reproduced;
- MR/BO no-overlap and same-bar handoff policy remain unchanged;
- excluded dates and rollover rules remain active;
- the anchor completeness guard accepts a normal early session and rejects an
  incomplete prior session.

For the early-session anchor guard, update the first-bar expectation from
`09:10` to `07:10` or earlier. Determine the minimum bar count from the actual
normalized session calendar, not by blindly retaining `80`.

### 8.2. Alor-USDRUBF Hybrid

Use the actually deployed `challenger_mr035` profile as the baseline. Expected
current parameters include:

```text
mr_k_short        = 0.035
mr_take_k_short   = 0.16
mr_stop_k_short   = 0.43
mr_last_entry     = 11:40
mr_force_exit     = 11:50
bo_k              = 0.45
bo_wait_hours     = 2.0
bo_eod_exit       = 23:30
```

The active VPS snapshot is authoritative if it differs from a repository
example.

Required `canonical07` behavior:

```text
model session start         09:00 -> 07:00
MR last entry               11:40 -> 09:40
MR forced exit              11:50 -> 09:50
BO wait                     unchanged at 2.0 elapsed hours
earliest BO eligibility     11:00 -> 09:00
BO EOD exit                 unchanged at 23:30
```

Entry remains closed-bar/next-bar according to the existing model execution
contract. Do not convert the shadow to intrabar-touch logic.

The current MR bracket implementation and its partial-fill reconciliation are
not part of this shadow alpha test. No protective order is emitted. Journal the
model TP/SL levels and theoretical fill/exit event only.

USDRUBF acceptance tests must prove:

- the numeric two-hour wait is counted from the selected model-session start;
- the earliest BO eligibility is `11:00` in `legacy09` and `09:00` in
  `canonical07`;
- the opening-phase MR/BO overlap duration is not accidentally enlarged;
- weekends do not update anchors or signals;
- morning currency-futures price limits do not create non-finite or stale
  levels.

### 8.3. IMOEXF Hybrid Riskgate

Primary operational baseline must be the actually deployed IMOEXF config, not
an inferred research config.

Known discrepancy to record before implementation:

```text
local/live-linked config: bo_wait_hours = 3.0
research handoff:          bo_wait_hours = 4.0
```

The session-change A/B test may change only the session contract. Therefore:

- the primary `legacy09` and `canonical07` pair must clone the active deployed
  wait/stop profile exactly;
- a `wait4` research-parity contour, if desired, must use another experiment ID
  and must not be pooled with the primary A/B result.

For an active `wait3` profile, required `canonical07` behavior is:

```text
model session start         09:00 -> 07:00
MR session end              11:59 -> 09:59
MR forced model exit bar    11:50 -> 09:50
BO wait                     unchanged at 3.0 elapsed hours
earliest BO eligibility     12:00 -> 10:00
BO EOD mode                 unchanged, same-day
```

For a separately labelled research `wait4` profile:

```text
earliest BO eligibility     13:00 -> 11:00
```

The MR window must close before BO eligibility just as in the accepted current
phase layout. If a config produces a larger MR/BO overlap after moving the
session start, startup validation must reject it.

Risk-gate requirements:

- use a separate canonical07 ledger key;
- never append canonical07 shadow PnL to the live/legacy ledger;
- preserve the existing seed only through the last pre-change session;
- mark seed metadata as `legacy09_history_through_2026-07-13`;
- update the new ledger from canonical07 sessions beginning `2026-07-14`;
- journal both raw MR eligibility and risk-gate allow/block decisions.

Because continuous trading before `09:00` did not exist before the exchange
change, the legacy historical seed through `2026-07-13` is an acceptable shared
historical initialization only when auction/service bars remain excluded.

## 9. Volume And Liquidity Diagnostics

No volume filter may be introduced in the initial shadow deployment. Doing so
would combine a session recut with a new alpha gate.

The journal must nevertheless collect:

- volume per 10m bar;
- cumulative volume since `07:00`;
- cumulative volume for `07:00..08:59`;
- cumulative volume since `09:00`;
- early-volume share of full-session volume after the session closes;
- volume at the signal bar;
- entry-time bucket: `07-08`, `08-09`, `09-10`, `10-12`, `12+`;
- spread or top-of-book width if already available without changing the feed;
- missing/zero-volume and stale-price flags.

Expected initial interpretation:

- MR is allowed to produce pre-09:00 shadow decisions;
- RI Author42 and USDRUBF BO should become eligible around `09:00` under their
  current elapsed-time logic;
- IMOEXF BO should become eligible at `10:00` for `wait3` or `11:00` for
  `wait4`;
- volume is diagnostic only until enough early-session observations exist.

## 10. Decision Journal Contract

Every model decision, including suppressed decisions, must include:

```text
experiment_id
contour_id
config_hash
image_tag
strategy_id
profile_id
instrument
component
session_policy_id
session_date
source_bar_start_ts
source_bar_close_ts
model_signal_ts
scheduled_entry_ts
scheduled_exit_ts
side
session_open
prev_anchor_date
prev_open
prev_high
prev_low
prev_close
prev_range
trigger_long
trigger_short
take_level
stop_level
exit_reason
overlap_decision
risk_gate_decision
shadow_gross_pnl
shadow_net_pnl
bar_volume
volume_since_0700
volume_0700_0859
volume_since_0900
feed_quality_flags
skip_reason
```

For fields not applicable to a component, write `null`; do not silently omit
the field.

## 11. Comparison Outputs

Produce daily outputs for every strategy:

```text
moex_early_session_<strategy>_decisions_legacy09.jsonl
moex_early_session_<strategy>_decisions_canonical07.jsonl
moex_early_session_<strategy>_trades.csv
moex_early_session_<strategy>_daily.csv
moex_early_session_<strategy>_diff.csv
```

The diff must classify every divergence as one of:

```text
same_decision
anchor_drift
session_open_drift
clock_shift_only
side_changed
legacy_only
canonical_only
overlap_arbitration_changed
risk_gate_changed
exit_path_changed
feed_quality_difference
```

Daily summary fields:

- MR trades and PnL by side;
- BO trades and PnL by side;
- combo PnL;
- pre-09:00 decisions and PnL;
- decisions from `09:00` onward;
- signal-time delta;
- anchor/range delta;
- maximum adverse and favorable excursion where reproducible;
- top-day concentration over the observed sample;
- number of suppressed MR/BO overlaps.

## 12. Required Tests

### Unit tests

1. `06:50` auction bar rejected.
2. `07:00` continuous bar accepted only by `canonical07`.
3. `08:50` bar accepted by `canonical07`, rejected by `legacy09`.
4. Weekend bars rejected by both policies.
5. Session open is the first valid `07:00` bar, never the auction bar.
6. Previous-day anchor uses only valid bars from the selected policy.
7. Opening-phase cutoff translation is exactly two hours.
8. EOD/late-day exits do not move.
9. No contour can emit an order intent to a command stream.

### Strategy clock tests

1. RI Author42: levels at `07:50`, first valid check at `08:50`, earliest
   scheduled entry at `09:00`.
2. USDRUBF: two-hour BO eligibility at `09:00` under `canonical07`.
3. IMOEXF wait3: BO eligibility at `10:00` and MR phase closed beforehand.
4. IMOEXF wait4 diagnostic: BO eligibility at `11:00`.
5. Existing `legacy09` golden tests remain unchanged.

### Replay and restart tests

1. Replay all available complete sessions from `2026-07-14` onward in both
   policies.
2. Compare Rust results with the existing Python session-policy A/B runner.
3. Restart each shadow mid-session and prove deterministic reconstructed state.
4. Verify deduplication: one closed bar updates each contour exactly once.
5. Verify a forming/incomplete 10m bar cannot create a decision.
6. Verify separate risk-gate ledgers remain isolated.

### Safety tests

1. Empty/invalid command-stream write assertion.
2. `allow_live_orders=false` startup assertion.
3. Strategy-specific emission disabled assertion.
4. No working broker order and no broker position can be created by any shadow
   service during a supervised test.

## 13. Deployment Sequence

### Phase 0: Snapshot and controlled diff

1. Export active VPS configs and image/commit metadata.
2. Build the active-config manifest.
3. Resolve the IMOEXF `wait3` versus `wait4` identity.
4. Generate `legacy09` and `canonical07` configs.
5. Review a machine-readable semantic diff.

Exit criterion:

```text
Only approved session/clock/isolation fields differ.
```

### Phase 1: Offline replay

1. Load all available bars beginning `2026-07-14`.
2. Run both policies for all three strategies.
3. Produce decision and daily diffs.
4. Pass strategy clock, no-order and restart tests.

Exit criterion:

```text
REPLAY_PASS / LEGACY09_NO_REGRESSION / NO_ORDER_PATH
```

### Phase 2: VPS shadow bring-up

1. Start the read-only `07:00` market-data contour.
2. Start the six mandatory runtimes: three `legacy09`, three `canonical07`.
3. Verify unique streams, groups, states, journals and health identities.
4. Verify startup logs show the expected session and component clocks.
5. Supervise the first full session from before `07:00` through final cleanup.

Exit criterion:

```text
ALL_SHADOWS_HEALTHY / JOURNALS_ADVANCING / ZERO_BROKER_COMMANDS
```

### Phase 3: Observation

Observation minimum:

```text
preliminary review = 5 complete weekday sessions
decision review    = 10 complete weekday sessions minimum
```

If fewer than five BO decisions are observed for a strategy, continue the
shadow until either five BO decisions or twenty complete sessions are reached.

No performance parameter may be changed during this observation window.

## 14. Acceptance Criteria

Engineering work is complete when:

1. All mandatory shadow services run from a read-only `07:00` feed.
2. The auction/service bar is excluded before every model-state update.
3. Current live/micro configs and services are unchanged.
4. `legacy09` reproduces the accepted model decision stream.
5. `canonical07` uses full valid early-session anchors.
6. Opening-phase MR cutoffs move by exactly two hours.
7. Relative BO waits are counted from `07:00` and match the expected clocks.
8. EOD and late-day exits remain unchanged.
9. MR/BO arbitration and no-overlap behavior remain explicit and journaled.
10. IMOEXF risk-gate state is isolated from the live ledger.
11. Volume diagnostics are present but do not gate decisions.
12. Restart/replay results are deterministic.
13. No shadow service emits or can emit a broker command.
14. Daily A/B reports are reproducible from committed scripts.

## 15. Promotion Rule

Completion of this TZ does not authorize micro/live promotion.

Promotion requires a separate reviewed decision based on:

- offline replay over all available post-change sessions;
- at least the observation minimum above;
- component-level MR/BO comparison;
- concentration and drawdown review;
- liquidity and estimated slippage review for pre-09:00 decisions;
- parity review between the research runner and Rust state machine;
- an explicit new frozen session contract.

Possible verdicts:

```text
KEEP_LEGACY09
PROMOTE_CANONICAL07_TO_MICRO_CANDIDATE
PARTIAL_PROMOTION_MR_OR_BO_ONLY
EXTEND_SHADOW
REJECT_EARLY_SESSION_RECUT
```

## 16. Deliverables

Required repository artifacts:

```text
docs/moex-early-session-shadow-contours-tz-2026-07-17.md
docs/moex-early-session-shadow-runbook-2026-07.md
docs/moex-early-session-shadow-observation-template-2026-07.md
configs/<strategy>.shadow09.<portfolio>.toml
configs/<strategy>.shadow07.<portfolio>.toml
configs/moex-early-session-md-only.<portfolio>.toml
moex_early_session_active_config_manifest.csv
moex_early_session_config_diff.csv
replay and A/B report artifacts
```

Code changes must include tests for the configurable RI session policy and all
strategy clocks listed in this document.

## 17. Rollback

Rollback consists only of stopping and removing the new shadow services and
their isolated consumer groups/state keys.

The rollback must not require:

- restarting current micro/live strategies;
- changing production gateway command paths;
- modifying broker positions or orders;
- restoring any live risk-gate ledger.

## 18. Final Engineering Read

The target implementation is a clean model-session recut, not an implicit feed
extension:

```text
auction excluded
continuous model day begins at 07:00
opening MR phase shifts with the opening
relative BO clock begins at 07:00
BO naturally becomes eligible near/after the 09:00 liquidity increase
late-day and EOD contracts stay fixed
live remains on the frozen 09:00 contract during observation
```
