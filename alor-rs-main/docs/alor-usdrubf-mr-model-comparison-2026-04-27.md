# Alor USDRUBF MR Model Comparison (2026-04-27)

## Scope

This memo compares the current live `alor_usdrubf_hybrid` MR block with the
new USDRUBF MR research branch discussed on 2026-04-27.

The goal is to decide whether the current live MR should be replaced, or
whether the new logic should remain a challenger/shadow model.

## Current Live MR: `alor_usdrubf_hybrid`

Runtime config:

- `configs/runtime.alor_usdrubf.live.7502T0U.toml`
- feed: `10m`
- symbol: `USDRUBF`
- strategy kind: `alor_usdrubf_hybrid`
- live size: fixed `1` contract

Current MR parameters:

| parameter | value |
|---|---:|
| `mr_min_rel_range` | `0.006` |
| `mr_max_rel_range` | `0.050` |
| `mr_k_short` | `0.045` |
| `mr_take_k_short` | `0.16` |
| `mr_stop_k_short` | `0.43` |
| `mr_last_entry_time` | `11:40:00` |
| `mr_force_exit_time` | `11:50:00` |

Runtime MR contract:

- short-only MR;
- session VWAP anchor, computed from 10m bars using typical-price weighting;
- current session range is the scale;
- entry condition: price is above session VWAP, but not farther than
  `mr_k_short * session_range`;
- entry is queued from the closed bar signal and filled by the runtime event
  contract;
- MR exits use stop/take from entry price and the signal-time session range;
- MR hard time cutoff is `11:50`.

Current hybrid-level challenger already checked:

| variant | period | return % | Sharpe | MaxDD % | trades | MR trades | BO trades |
|---|---|---:|---:|---:|---:|---:|---:|
| `baseline` | full | 54.19 | 2.42 | 3.20 | 823 | 328 | 495 |
| `mr035_exit1200` | full | 57.18 | 2.54 | 2.46 | 783 | 288 | 495 |
| `baseline` | test_30 | 11.63 | 2.41 | 2.05 | 268 | 116 | 152 |
| `mr035_exit1200` | test_30 | 12.97 | 2.71 | 1.83 | 250 | 98 | 152 |
| `baseline` | recent_forward | 2.37 | 2.99 | 1.70 | 41 | 15 | 26 |
| `mr035_exit1200` | recent_forward | 2.00 | 2.48 | 1.78 | 38 | 12 | 26 |

Read: `mr_k_short = 0.035` is a valid narrow challenger, but not a clear
production replacement because the recent forward slice is slightly weaker
than the current baseline.

## New MR Research Branch

The new branch is not one model; it contains three related ideas:

1. `High180`-style short MR;
2. side-gated long/short morning MR with up to two entries per side;
3. BO-context first probe plus mirrored-K reentry.

These ideas share a different rationale from the current live MR:

- use previous regular-day range / opening context rather than current session
  VWAP as the main anchor;
- classify prior strong directional days and look for next-day weak opening /
  intraday mean reversion behavior;
- use side-specific gates because long and short are not symmetric on USDRUBF;
- optionally allow a second mirrored entry after the first probe.

## Direct MR Comparison

The most direct comparison is current live-style short MR versus
`High180` short-only candidates on the same cached 10m data through
`2026-04-21`.

| variant | period | return % | Sharpe | MaxDD % | trades | win % | read |
|---|---|---:|---:|---:|---:|---:|---|
| `current_vwap_short_mr` | full | 4.21 | 1.34 | 1.47 | 160 | 73.1 | current live MR contour, stable and low DD |
| `current_vwap_short_mr` | test_30 | 0.16 | 0.16 | 1.30 | 66 | 66.7 | weak late slice |
| `current_vwap_short_mr` | recent_forward | 0.41 | 3.45 | 0.17 | 15 | 60.0 | positive but small |
| `high180_ks045_short_only` | full | 6.08 | 1.05 | 3.12 | 193 | 73.1 | higher return, higher DD |
| `high180_ks045_short_only` | test_30 | 3.78 | 2.16 | 1.19 | 64 | 73.4 | materially better late slice |
| `high180_ks045_short_only` | recent_forward | 1.70 | 8.15 | 0.00 | 10 | 100.0 | very strong but small sample |

Filtered short-side candidates improve the High180 contour:

| variant | side | full return % | Sharpe | MaxDD % | trades | test return % | recent return % |
|---|---|---:|---:|---:|---:|---:|---:|
| `short_base_ks050` | short | 7.15 | 1.21 | 2.80 | 208 | 3.70 | 1.78 |
| `short_kill5_500` | short | 8.19 | 1.45 | 1.65 | 189 | 3.23 | 1.69 |
| `short_kill5_750` | short | 8.41 | 1.46 | 1.81 | 201 | 3.61 | 1.69 |
| `short_prev_up_0p25` | short | 7.73 | 1.70 | 0.95 | 144 | 2.32 | 1.51 |

Read: the strongest direct replacement candidate is not the raw long/short
copy. It is a short-side High180 contour with a side-specific risk layer,
especially `short_kill5_500` / `short_kill5_750` or the more conservative
`short_prev_up_0p25`.

## Long/Short Morning MR

Long-side is not a simple mirror of short-side on USDRUBF.

The raw long/short version is unacceptable because long-side tail risk is too
large. The viable versions require side-specific gates.

| variant | full return % | Sharpe | MaxDD % | trades | long | short | train % | test % | recent % |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `ls_nogate_max2` | 3.70 | 0.37 | 6.40 | 557 | 404 | 153 | -1.46 | 5.23 | 1.56 |
| `ls_perm3_prev_max2` | 7.88 | 1.28 | 2.13 | 216 | 152 | 64 | 3.70 | 4.03 | 2.05 |
| `ls_practical_mixed_max2` | 11.44 | 1.61 | 2.35 | 282 | 150 | 132 | 6.82 | 4.32 | 2.64 |

Read: `ls_practical_mixed_max2` is the strongest research candidate, but it is
a new MR design, not a drop-in parameter change. It introduces long-side
permission, short-side kill-switch behavior, previous-day extreme filtering,
and capped reentry.

## Mirrored-K Reentry

The mirrored-K reentry experiment asks whether the first MR probe can add a
second entry from a mirrored trigger level after the first target.

Equity comparison:

| strategy | total net RUB | trades | active days | win % | daily MaxDD RUB | first net RUB | reverse net RUB |
|---|---:|---:|---:|---:|---:|---:|---:|
| `rank30_first_only` | 2297.0 | 100 | 100 | 92.0 | -657.1 | 2297.0 | 0.0 |
| `rank30_with_mirrored_reentry` | 4583.4 | 155 | 100 | 90.3 | -741.4 | 2297.0 | 2286.4 |
| `best_side_first_only` | 2353.0 | 95 | 95 | 92.6 | -690.8 | 2353.0 | 0.0 |
| `best_side_with_mirrored_reentry` | 5189.6 | 145 | 95 | 92.4 | -653.5 | 2353.0 | 2836.6 |

Read: mirrored reentry is promising and appears to add a second structural
edge, but it is farther from current production logic than the short-only
High180 challenger. It requires a separate runtime contract and parity pass.

## Decision

Do not replace the current live `alor_usdrubf_hybrid` MR immediately.

Recommended path:

1. Keep current live baseline unchanged.
2. Keep `mr_k_short = 0.035` as the narrow runtime challenger because it
   changes only one core knob and preserves the existing runtime contract.
3. Promote `High180 short-side with risk layer` to a separate shadow/challenger
   research package.
4. Treat `ls_practical_mixed_max2` and mirrored-K reentry as second-stage
   challengers after the direct short-side High180 contour is replay-aligned.

## Promotion Criteria

Before replacing current live MR, the new branch should pass:

- 10m replay parity against the Python research harness;
- 70/30 plus recent-forward validation with identical costs;
- stress-cost validation;
- day-level concentration and worst-day diagnostics;
- side attribution: long and short must each be explainable;
- no hidden dependency on 2026-specific tuning;
- live implementation must be feature-flagged and disabled by default until
  replay parity is confirmed.

## Practical Implementation Recommendation

Near-term implementation should be staged:

1. Add a disabled-by-default `high180_usdrubf_mr` research/replay profile.
2. Start with short-only `short_kill5_500` or `short_kill5_750`.
3. Add long-side and mirrored reentry only after the short-only profile has
   clean replay parity and a stable operational contract.
4. Keep the existing BO block untouched while validating the MR replacement.

Current verdict:

`CURRENT_LIVE_MR_KEEP`

New branch verdict:

`PROMISING_CHALLENGER_NOT_PRODUCTION_DEFAULT`

