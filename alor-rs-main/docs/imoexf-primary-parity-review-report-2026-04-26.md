# IMOEXF Primary Parity Review Report

## Status

- Current status: `SIGNAL_NEAR_PARITY / EXECUTION_CONTRACT_DRIFT_EXPLICIT`
- BO gap-flatten decision: `PENDING_TEAM_DECISION`
- Promotion status: `NOT_READY_FOR_PROMOTION_UNTIL_BO_GAP_FLATTEN_DECISION`

## Model Feed Contract

- Prepared/model feed must contain only regular tradable bars: Monday-Friday `09:00..23:49`.
- Raw/audit feed may contain service bars such as `08:50`, but those bars must not update MR, BO, riskgate, entry/exit, or parity state.
- Rows: `52572`
- Date range: `2023-11-14 10:00:00` -> `2026-04-21 23:40:00`
- Weekend rows: `0`
- Pre-session rows: `0`
- Post-session rows: `0`
- Non-monotonic rows: `0`
- Regular model contract: `True`

## Layer Summary

### Saved Source Reference vs Rust Replay

- MR: source `474`, Rust `471`, exact `464`, missing/extra `10 / 7`.
- BO: source `370`, Rust `375`, exact `0`, missing/extra `370 / 375`.

## MR Read

- Saved source MR drift is mostly stale-reference drift, not a Rust MR signal failure.
- Saved-source MR vs Rust: `474` vs `471`, exact `464`, missing/extra `10 / 7`.
- Filtered canonical MR vs Rust: `471` vs `471`, exact `470`, missing/extra `1 / 1`.
- Saved-source missing causes: `bo_gap_flatten_interaction=1, calendar_zero_riskgate=5, stale_service_bar_midpoint=4`.
- Saved-source actual-extra causes: `source_hybrid_merge_bo_overlap=1`.
- Filtered canonical residual causes: `bo_gap_flatten_interaction=1`.

## BO Read

- BO fill-level exact parity is expected to be poor while comparing Backtrader next-bar fills with Rust close-bar/event-loop actions.
- Fill-level: source `370`, Rust `375`, exact `0`, missing/extra `370 / 375`.
- After source timestamp normalization (`-10m`), entry-signal common `361 / 370`.
- After source timestamp normalization (`-10m`), entry+exit-signal common `350 / 370`.
- Date+side count diffs after normalization: `7`.
- Source cross-day reference carry: `9`.
- Rust cross-day gap-flatten: `7`.
- Residual after signal shift: missing/extra `9 / 14`.

## Required Decision

- Preferred live contract is Rust close-bar/no-overnight behavior.
- `bo_gap_flatten` should be accepted explicitly if the team agrees that BO must not carry through non-tradable gaps.
- Do not tune Rust toward Backtrader cross-day carry unless the team intentionally chooses replay-fill parity over live safety semantics.

## Promotion Gate

- Rebuild the official filtered bundle through `2026-04-21`.
- Run `hybrid_replay --profile imoexf_primary_riskgate_k053 --assert-gap-flatten` on the official bundle.
- Publish this report from official artifacts, not temporary `/tmp` outputs.
- Record the `bo_gap_flatten` decision.
- If accepted and clean, start extended micro soak at live size `1` with explicit MR/BO attribution monitoring.
