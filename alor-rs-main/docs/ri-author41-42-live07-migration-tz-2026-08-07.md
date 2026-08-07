# RI Author41/42 Live07 Migration TZ

Date: `2026-08-07`

Status:

```text
READY_FOR_ENGINEERING_IMPLEMENTATION
NO_PARAMETER_RETUNE
LIVE07_TARGET_CONTRACT = canonical07_e10
```

## 1. Goal

Move the active RI Author41/42 micro-live contour from the old `09:00` model
session to the new MOEX regular futures session starting at `07:00 MSK`.

The target is not a new strategy and not a parameter refit. It is a session
contract migration:

```text
legacy09     -> canonical07_e10
09:00..23:49 -> 07:00..23:49
MR entries   -> first 3 model hours, 07:00..10:00
```

## 2. Current Production Context

Current active live config:

```text
configs/runtime.ri_author41_42.micro.7502MIW.RIU6.roll-2026-06-12.toml
```

Current active instrument:

```text
symbol       = RIU6
order_symbol = RTS-9.26
portfolio    = 7502MIW
qty          = 1
timeframe    = 10m
execution    = action_scoped_only
```

Current live is still a legacy09 model-clock contour:

```text
session_start = 09:00:00
session_end   = 23:49:59
min_anchor_bars = 80
anchor_first_bar_at_or_before = 09:10:00
anchor_last_bar_at_or_after   = 23:30:00
```

## 3. Target Model Contract

Use the canonical07_e10 contract.

### Session

```text
model_session_start = 07:00:00
model_session_end   = 23:49:59
weekends            = excluded
timeframe           = closed 10m bars
```

The `06:50` bar is an auction/service bar and must not update:

```text
model state
previous-session anchor
Author41 MR signal state
Author42 BO observation state
risk/session diagnostics
entries
exits
```

### Anchor Guard

For sessions from `2026-07-14` onward:

```text
min_anchor_bars = 92
anchor_first_bar_at_or_before = 07:10:00
anchor_last_bar_at_or_after   = 23:30:00
```

For pre-transition history before `2026-07-14`, keep the legacy anchor guard:

```text
pre_transition_min_anchor_bars = 80
pre_transition_anchor_first_bar_at_or_before = 09:10:00
pre_transition_anchor_last_bar_at_or_after   = 23:30:00
anchor_transition_date = 2026-07-14
```

Excluded model dates remain:

```text
2026-06-12
2026-11-04
```

## 4. Author41 MR Contract

Do not retune K, K2, stops or costs.

Short profile:

```text
k = 0.20
k2 = 0.020
stop_k = 0.75
min_range = 0.005
max_range = 0.100
max_entries_per_day = 2
roundtrip_cost_points = 2.0
```

Long profile:

```text
k = 0.11
k2 = 0.005
stop_k = 1.00
min_range = 0.005
max_range = 0.100
max_entries_per_day = 2
roundtrip_cost_points = 2.0
```

Target timing:

```text
author41_entry_start = 07:00:00
author41_entry_end   = 10:00:00
author41_time_exit   = 20:00:00
```

Important execution rule:

```text
Do not force-close MR at 10:00 solely because the MR entry window ended.
```

After entry, MR must keep the frozen exit contract:

```text
take_author_close
stop
breakeven_limit
time_exit at 20:00
last-bar safety close only if required by runtime/backtest boundary
```

Rationale: the post-transition replay showed that forcing MR out at the window
end materially weakens canonical07_e10.

## 5. Author42 BO Contract

Do not retune BO parameters.

```text
k = 0.42
stop_hour_k = 0.50
stop_k = 0.18
min_prev_hl_ratio = 1.01
prev_extreme_move = 0.025
first_hour_extreme_k = 1.50
exclude_friday = true
exclude_june_window = true
allow_reentry_on_day_extreme = true
roundtrip_cost_points = 2.0
exit_time = 23:00:00
```

Under canonical07, Author42 first-hour/opening-level observation must be based
on the 07:00 session clock. BO observation state must update on every eligible
model bar independently of MR pending/position lifecycle.

The following P0 hardening must be present before live migration:

```text
BO observer cannot skip bars while MR has pending entry/exit/position state.
BO entry must be suppressed when the materialization event is at or after
author42_exit_time.
```

Reference patch/doc:

```text
docs/ri-author42-bo-observation-hardening-2026-08-06.md
```

## 6. MR/BO Interaction

Production target remains strict no-overlap:

```text
MR and BO must not hold simultaneous RI exposure.
If MR is active, BO signal may be observed/journaled but must not create live
broker exposure.
```

Do not add BO-priority reversal in this migration. A future challenger may test
MR-to-BO reversal or BO-priority rules, but this live07 migration keeps the
existing no-overlap production contract.

## 7. Expected Backtest Reference

Latest local recheck package:

```text
analiz_alpha_si/ri_author41_42_session07_recheck_2026_08_07/
```

Complete-session window:

```text
2026-07-14 .. 2026-08-06
```

Primary comparison:

```text
legacy09 combo:        +7048.5 points, 27 trades, MaxDD 2408
canonical07_e10 combo: +9603.0 points, 36 trades, MaxDD 1907
```

Canonical07_e10 component attribution after no-overlap:

```text
Author41 MR: +3211.0 points, 22 trades
Author42 BO: +6392.0 points, 14 trades
```

MR window sensitivity:

```text
canonical07_e10 MR 07:00..10:00: +3211.0
canonical07_e11 MR 07:00..11:00:  +995.5
canonical07_e12 MR 07:00..12:00:  +993.5
```

MR hold-vs-window-exit diagnostic:

```text
canonical07_e10 baseline_hold combo:   +9603.0
canonical07_e10 mr_window_exit combo:  +3902.0
```

Interpretation: use `07:00..10:00` as the MR entry window, but keep frozen MR
holding/exit behavior after entry.

## 8. Implementation Tasks

### Task A. Create Live07 Runtime Config

Create a new live config from the current micro-live RIU6 config.

Recommended file:

```text
configs/runtime.ri_author41_42.micro.7502MIW.RIU6.live07.toml
```

Required changes:

```text
session_open_hour = 7
session_open_minute = 0

[trading_periods]
session_start = "07:00:00"
session_end   = "23:49:59"

[strategy.ri_author41_42]
session_start_time = "07:00:00"
session_end_time   = "23:49:59"
author41_entry_end_time = "10:00:00"
author41_time_exit      = "20:00:00"
author42_exit_time      = "23:00:00"
min_anchor_bars = 92
anchor_first_bar_at_or_before = "07:10:00"
anchor_last_bar_at_or_after   = "23:30:00"
anchor_transition_date = "2026-07-14"
pre_transition_min_anchor_bars = 80
pre_transition_anchor_first_bar_at_or_before = "09:10:00"
pre_transition_anchor_last_bar_at_or_after   = "23:30:00"
```

Keep unchanged:

```text
profile_id
qty
symbol
order_symbol
execution_path
rollover policy
excluded_model_dates
all Author41/Author42 K/stop/cost parameters
```

### Task B. Create/Validate Live07 Gateway Config

If the current market-data stream already includes 07:00 bars, only runtime
stream wiring may need to change. If not, create a gateway config that publishes
the 07:00 session bars.

Required stream property:

```text
RIU6 10m bars include 07:00..23:49 regular bars
06:50 auction/service bars may exist in raw feed but must not be model-eligible
```

Gateway must not start a second live RI runtime generation at the same time as
the old live09 runtime.

Engineering prep status on `2026-08-07`:

```text
candidate runtime config:
  configs/runtime.ri_author41_42.micro.7502MIW.RIU6.live07.toml

legacy09 rollback config remains unchanged:
  configs/runtime.ri_author41_42.micro.7502MIW.RIU6.roll-2026-06-12.toml

live stream checked on VPS:
  md.bars.7502MIW.RIU6.10m includes 07:00, 07:10, ... regular bars

gateway config change:
  not required for 7502MIW unless the upstream RIU6 stream wiring changes

required runtime image:
  manual-20260807-ri-bo-observation-b386ba5 or newer
```

The live07 candidate intentionally uses a separate `consumer_group`, `health`,
`runtime_state`, and decision journal namespace, while keeping the same
`commands`/`acks` streams used by the existing RI gateway. Rollout should
therefore stop the legacy09 runtime first, then start only the live07 generation
from broker-flat state. Do not run legacy09 and live07 simultaneously.

### Task C. Runtime Logic Verification

Verify all of the following in code/tests:

```text
06:50 bars are ignored by RI model state and signals.
07:00 is accepted as the first model bar.
Previous-session anchor uses 07:00..23:49 after 2026-07-14.
Pre-transition history keeps 09:00 anchor guard.
Author41 entry window ends at 10:00.
Open Author41 positions are not forced out at 10:00.
Author42 BO observation uses the 07:00 session clock.
BO observer advances even if MR is pending or active.
No BO entry can be materialized at or after 23:00.
No MR/BO simultaneous live exposure is possible.
Break bars 14:00..14:04:59 and 18:50..19:04:59 do not create entries.
```

### Task D. Replay/Parity Gate

Run replay on the same data package used by analytics or an equivalent fresh
raw 10m RIU6 feed.

Reference analytics package:

```text
analiz_alpha_si/ri_author41_42_session07_recheck_2026_08_07/
```

Required parity outputs:

```text
legacy09 replay summary
canonical07_e10 replay summary
canonical07_e10 trade list
MR/BO component attribution
MR hold-vs-window-exit diagnostic
```

Acceptance expectation for canonical07_e10 on `2026-07-14..2026-08-06`:

```text
combo close to +9603.0 points
MR close to +3211.0 points
BO accepted in combo close to +6392.0 points
```

If exact values differ, classify drift before rollout:

```text
data drift
session/bar filter drift
execution timing drift
no-overlap arbitration drift
known live-adapter lifecycle drift
```

Do not promote if unexplained drift changes trade side, entry date, or
component-level sign.

### Task E. Live Rollout

Rollout only between sessions or before `07:00 MSK`.

Safety gate:

```text
broker RI position is flat
no working regular orders
no working stop orders
runtime has no pending entry/exit/protective lifecycle
gateway is healthy
new config points to only one live runtime generation
old live09 runtime is stopped before live07 starts
```

Startup verification:

```text
startup log prints session_start=07:00
startup log prints author41_entry_end=10:00
startup log prints author41_time_exit=20:00
startup log prints author42_exit_time=23:00
startup log prints min_anchor_bars=92
startup log prints anchor_transition_date=2026-07-14
first eligible model bar is 07:00, not 06:50
no startup replay intent is emitted for historical bars
```

### Task F. First-Session Observation

For the first live07 session, record:

```text
all model decisions
runtime intents
broker order ids
fills
exit reasons
final broker position
working orders after each terminal cycle
MR/BO no-overlap suppressions
any stale-entry suppression
```

The first session is observation-only:

```text
no K change
no stop change
no quantity increase
no BO-priority/reversal change
```

## 9. Acceptance Criteria

Migration is accepted if:

```text
live07 config exists and is reviewed
07:00 session bars are available and 06:50 is excluded from model state
canonical07_e10 replay matches analytics within explainable tolerance
Author41 does not force-exit at 10:00
Author42 observer hardening is present
no BO entry can be emitted at/after 23:00
strict no-overlap remains active
rollout is done from broker-flat state
first live07 session ends broker-flat or with controlled/open documented state
no orphan/residual orders remain after terminal exits
```

## 10. Explicit Non-Goals

Do not do in this migration:

```text
change Author41 K/K2/stop values
change Author42 K/stop values
increase qty
introduce MR-to-BO reversal
introduce BO-priority overlap
force-close all MR at 10:00
change rollover policy
change order_symbol mapping
mix live09 and live07 runtime generations
```

## 11. Practical Verdict

Recommended target for engineering:

```text
Promote RI Author41/42 to canonical07_e10 only after replay/parity gate.
Use 07:00..10:00 as the MR entry window.
Keep MR holding/exit logic unchanged after entry.
Keep strict no-overlap with BO.
Keep K/stops unchanged.
```
