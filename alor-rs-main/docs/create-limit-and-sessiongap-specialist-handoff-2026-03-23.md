# Specialist Handoff: `create:limit` / CWS And `session_gap`

Date: 2026-03-23

Primary artifact:

- `docs/create-limit-and-sessiongap-review-ready-2026-03-23.md`

Purpose:

- provide a short specialist-entry summary for the current narrowed review package;
- focus review on root-cause narrowing, not on whether evidence exists.

## Current Position

- `TZ1` observability work is complete in code and verified live;
- `TZ2` recovery-semantics work is complete in code and locally test-complete;
- the package is ready for specialist review;
- the package is not ready for closure.

## Strongest Current Conclusion

The residual incident class is best treated as:

- intermittent shared CWS limit-order control-plane instability
- with topology/runtime/timing sensitivity
- not explained by a simple deterministic topology switch
- and not limited to `session_gap_standalone`.

## Highest-Value Confirmed Facts

1. `create:market` baseline remained stable on the reviewed line.

2. Fresh current-line `L1 @ T2` reproduced:
   - `cws_error`
   - `protocol_reset_without_close_handshake`
   - `broker_order_id = null`

3. Passive `L2 @ T2` reproduced the same transport class on the narrow manual command path.

4. `L2 @ T1` isolated single-stack topology passed cleanly end-to-end.

5. `L2 @ T3` re-expanded topology reproduced again.

6. Gateway-only coexistence late probes passed, so second-gateway presence alone is not a sufficient trigger.

7. Late discriminator under `sessiongap runtime ON / hybrid runtime OFF` produced:
   - clean `hybrid` `create:limit`
   - first `hybrid` `delete:limit` repro
   - clean retry delete after `hybrid` gateway recovery

8. Late discriminator under `hybrid runtime ON / sessiongap runtime OFF` passed end-to-end.

9. Late discriminator under `both runtimes ON` also passed end-to-end.

## What This Rules Out

- not a blanket live-trading failure;
- not a blanket `create:limit` always-fails condition;
- not a `session_gap`-only explanation;
- not a second-gateway-only explanation;
- not a simple "`both runtimes ON` always repros" rule.

## What Still Needs Specialist Judgment

1. Which transient variable best explains the observed resets:
   - shared session competition
   - reconnect timing
   - runtime-driven timing interaction
   - broker-side instability
   - or another shared CWS control-path condition

2. Why the strongest late repro surfaced on first `delete:limit` under `sessiongap runtime ON / hybrid runtime OFF`, while nearby discriminator probes passed.

3. What the narrowest next validation or fix should be before any further broad live reruns.

## Requested Review Focus

- use the expanded comparative matrix in the main report as the primary evidence artifact;
- review the issue as intermittent and timing-sensitive, not as a deterministic topology toggle;
- recommend the narrowest next hypothesis to validate;
- recommend whether the next action should be:
  - instrumentation refinement,
  - reconnect/session-policy change,
  - broker-facing escalation,
  - or a tightly scoped live validation.

## Recommended Immediate Project Posture

- proceed with specialist review now;
- do not spend another broad live rerun before that review;
- return to live only after the next validation target is explicitly narrowed.
