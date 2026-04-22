# Sessiongap Parity Review (2026-04-22)

## Scope

This note summarizes the current parity status for `session_gap_standalone` / `session_gap_replay` versus the legacy Python `dbo_v3` USDRUBF session-gap strategy.

The goal of this review is not to claim full parity yet, but to clearly separate:

- what is already understood and stable,
- what has been successfully translated,
- what still differs materially.

## Main Conclusions

1. We are far enough along to treat this strategy as understood and reviewable.
2. The old conclusion that forward performance was outright negative is no longer valid.
3. The strongest parity-aligned configuration is:
   - `10m` feed
   - `signal_minute = 50`
   - `wait_hours = 3`
   - `max_entry_hour = 16`
   - `exit_offset_min = 29`
4. Under that configuration, Rust replay is directionally and structurally close to `dbo_v3`, but still not exact parity.
5. The remaining gap is now concentrated in exit semantics, not in entry semantics.

## What We Confirmed

### 1. `sessiongap` is not an MR strategy

`sessiongap` is a session breakout / continuation strategy with session-gap context filters.

It does **not** contain a separate morning mean-reversion component.

Both Python and Rust implementations use the same directional logic:

- long only when price is above previous-close anchor plus threshold and above early-session structure;
- short only when price is below previous-close anchor minus threshold and below early-session structure.

### 2. `wait_hours = 3` is better than `wait_hours = 2`

Forward comparison on `2026-02-12 .. 2026-04-22` with `10m` feed:

- `dbo_v3`, `wait=2`: `-0.73%`, Sharpe `-0.75`, MaxDD `2.62%`, `27` trades
- `dbo_v3`, `wait=3`: `+1.49%`, Sharpe `1.92`, MaxDD `1.51%`, `26` trades
- Rust parity replay, `wait=2`: `+0.07%`, Sharpe `0.09`, MaxDD `1.95%`, `27` trades
- Rust parity replay, `wait=3`: `+2.10%`, Sharpe `3.27`, MaxDD `1.52%`, `26` trades

So the current evidence supports keeping `wait_hours = 3`.

### 3. Plain runtime-like `1m` sessiongap is weaker than `dbo_v3`

Current forward read on `2026-02-12 .. 2026-04-22`:

- `dbo_v3_current_10m`: `+1.24%`, Sharpe `1.59`, MaxDD `1.51%`, `26` trades
- `session_gap_replay_1m`: `+0.14%`, Sharpe `0.17`, MaxDD `2.12%`, `33` trades

This means:

- the raw `1m` runtime-like implementation is not a valid drop-in proxy for the legacy `10m` backtest;
- parity work should be evaluated primarily on the `10m` parity replay path, not on the raw `1m` path.

### 4. `10m` parity replay is now viable

We extended `session_gap_replay` to support env-driven overrides for:

- `signal_minute`
- `wait_hours`
- `max_entry_hour`
- `exit_offset_min`
- `close_hour`
- `close_minute`

Using:

- `signal_minute = 50`
- `wait_hours = 3`
- `max_entry_hour = 16`
- `exit_offset_min = 29`

the Rust replay now produces:

- `26` trades on the same forward window as `dbo_v3`
- positive return
- structurally aligned entries

## Current Parity Status

The parity-aligned `10m` replay still fails strict full parity, but the failure is now narrow and informative:

- trade count matches: `26 vs 26`
- side matches
- quantity often matches
- entry timestamps match
- entry prices match

The first divergence is now:

- `entry_match = true`
- `side_match = true`
- `qty_match = true`
- `entry_price_match = true`
- but `exit_match = false`
- and `exit_price_match = false`

This is strong evidence that the remaining mismatch is primarily in exit handling.

## Important Engineering Notes

### 1. Do not use plain `10m` replay without parity overrides

The original `session_gap_replay` expected `minute == 59`, so a direct `10m` run produced zero signals.

That mode should not be used for parity conclusions.

### 2. Treat the new `10m` parity replay as the canonical review harness

For legacy comparison, use:

- `10m` bars
- `signal_minute = 50`
- `wait_hours = 3`
- `max_entry_hour = 16`
- `exit_offset_min = 29`

### 3. Remaining work is exit-contract parity

Most likely remaining mismatch sources:

- TP/SL recognition timing inside the bar
- when close orders become pending vs executed
- forced session-end close semantics
- interaction between bracket-style legacy behavior and replay exit scheduling
- commission / net-vs-gross treatment if final exact parity is required

## Recommended Next Steps

1. Keep `wait_hours = 3` as the parity baseline.
2. Keep the `10m` parity replay path in the repo and use it as the comparison harness.
3. Do not use raw `1m` replay numbers as the primary parity verdict against `dbo_v3`.
4. Focus the next parity pass on exit semantics only:
   - TP timing
   - SL timing
   - forced exit timing
   - bar-open vs bar-close execution assumptions
5. Only after exit parity is tightened should we decide whether runtime strategy behavior is equivalent enough for production sign-off.

## Practical Verdict

At this point, the strategy is sufficiently understood to brief the developer confidently:

- the strategy logic is no longer a mystery;
- the correct wait parameter is `3h`, not `2h`;
- a proper `10m` parity harness now exists;
- the remaining gap is specific and actionable.

So yes: this branch is ready for developer-facing review, with the explicit caveat that **full parity is not yet complete and the remaining work is concentrated in exit semantics**.
