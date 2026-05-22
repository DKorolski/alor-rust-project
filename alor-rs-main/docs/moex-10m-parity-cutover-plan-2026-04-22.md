# MOEX 10m Parity Cutover Plan (2026-04-22)

Date: 2026-04-22

## Scope

This plan covers the next parity-alignment cutover for the three active MOEX strategy lines:

- `trading-sessiongap`
- `trading-hybrid`
- `trading-alor-usdrubf`

## Why This Cutover Exists

Current review evidence points to one dominant remaining structural drift:

- frozen handoff / parity replay bundles are built on `10m`
- current live runtime contour has been running on `1m`

This is already documented in:

- `docs/sessiongap-parity-review-2026-04-22.md`
- `docs/moex-handoff-timeframe-parity-observation-2026-04-22.md`

The cutover goal is therefore not to redesign the strategies, but to align the live runtime contour with the frozen parity contour as closely as possible.

## Main Decision

The next live contour should run on `10m` feed for all three lines.

### `sessiongap`

Target live parity contour:

- `10m` feed
- `signal_minute = 50`
- `wait_hours = 3`
- `max_entry_hour = 16`
- `session_end = 23:49`
- `exit_offset_min = 29`

Important note:

- the target behavior is an effective forced exit around `23:30 MSK`
- this is achieved via `23:49 - 29 min`
- not by changing the session close itself to `23:30`

### `hybrid`

Target live parity contour:

- `10m` feed
- no strategy retune in this patch line
- only timeframe alignment

### `alor-usdrubf`

Target live parity contour:

- `10m` feed
- no strategy retune in this patch line
- only timeframe alignment

## Required Patch Scope

### 1. Sessiongap runtime code patch

`sessiongap` live runtime previously used a hardcoded signal minute:

- `minute == 59`

That is incompatible with the reviewed parity contour, so this patch line adds:

- configurable `signal_minute` in `SessionGapStandaloneSettings`
- propagation through runtime config loading
- runtime strategy usage of configured `signal_minute`

This is the only required code change in the runtime logic for this cutover line.

### 2. Live config updates

The following active live configs are updated:

- `configs/runtime.sessiongap.live.7502MIW.toml`
- `configs/gateway.sessiongap.live.7502MIW.action-scoped.phase2.toml`
- `configs/runtime.hybrid.live.7502SN6.action-scoped.toml`
- `configs/gateway.hybrid.live.7502SN6.action-scoped.toml`
- `configs/runtime.alor_usdrubf.live.7502T0U.toml`
- `configs/gateway.alor_usdrubf.live.7502T0U.toml`

Config change classes:

- `tf_sec: 60 -> 600`
- bars stream names:
  - `.1m -> .10m`
- `sessiongap` parity parameters:
  - `signal_minute = 50`
  - `max_entry_hour = 16`
  - `exit_offset_min = 29`

## Rollout Policy

This cutover should be done on VPS:

- in a pre-open window
- with all three stacks flat
- with `from zero` Redis restart

### Why `from zero` is required

The contour is changing from `1m` to `10m`.

That means old Redis tails and runtime state no longer describe the same event cadence.

So for all three stacks:

- stop stack
- remove live Redis persistence tail
- recreate empty Redis data dir
- start stack fresh on `10m`

This is required to avoid mixed-contour ambiguity.

## Handling Of Old 1m Contour

The old `1m` live contour is not intended to remain as a long-running parallel baseline.

Working decision:

- use the same VPS stack names
- cut over the active live contour in place
- do not keep a permanent parallel `1m` live contour after successful `10m` start

## Validation Checklist After Rollout

After morning cutover, confirm:

1. all containers are healthy
2. Redis restart counts remain `0`
3. Redis startup logs show fresh AOF creation, not old replay tail
4. readiness path returns through:
   - `SyncingHistory`
   - `LiveReady`
   - `ALLOWED`
5. bars stream names are `md.bars.<portfolio>.10m`
6. Redis memory stays low after startup
7. no stale `1m` tail remains in the live path

## What This Patch Does Not Claim

This cutover does **not** claim that full parity is now solved.

Expected remaining open area after timeframe alignment:

- exit semantics
- especially for `sessiongap`, where review evidence already suggests the remaining mismatch is concentrated in exit handling

So the post-cutover goal is:

- remove the largest structural drift first
- then re-evaluate whether the residual parity gap is small enough to justify deeper exit-contract work

## Practical Expected Outcome

Best-case expected outcome:

- `hybrid` and `alor-usdrubf` move materially closer to frozen parity simply from timeframe alignment
- `sessiongap` moves materially closer after timeframe alignment plus the reviewed parity parameter tune

If residual gaps remain after that, they should be much cleaner to interpret than under the current mixed `1m` vs `10m` contour.
