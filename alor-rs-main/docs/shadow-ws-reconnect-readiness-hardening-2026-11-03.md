# WS Source Readiness Recovery

Scope: the isolated `alor_shadow_md_ws` adapter, not live gateway/runtime or model
logic. The source-only terminal canary confirmed one real session-boundary release
with an intact canonical prefix; a later connection reset exposed an independent
readiness-deadline defect. Terminal success is not overall reconnect acceptance.

## Cause

On reconnect the adapter correctly invalidated subscription/history readiness.
However, its next tick checked `process_started.elapsed() < 600 seconds` whenever
history was unready. After a mature process had been running for hours, even a
brief recoverable reset could therefore fail the startup timeout immediately.
A quick reconnect sometimes escaped the next tick; that was not reliable recovery.

## Contract

Use a monotonic unready-period budget instead of total process age:

- Initial startup has the existing 600-second budget from process start.
- Once ACK, connection and complete preceding context establish readiness, the
  unready-period marker is cleared.
- A subsequent invalidation starts one new bounded recovery period. Repeated
  reconnect/retry/ACK attempts within the same unready period do not extend it.
- Only confirmed readiness clears the marker. Deadline expiry remains fatal;
  invalid clocks, missing active history, uncertain publication and corrupt
  canonical state retain their existing failure behavior.
- Health exposes `readiness_wait_seconds`: null when ready, elapsed unready time
  otherwise. Worker source freshness/admission checks are not relaxed.

This is not an automatic restart of a failed generation. Preserve FAILED markers,
source journals, Redis rows and earlier receipts. Admit a fresh source generation
with a clean pinned artifact/config/stream after isolated verification. No
in-place marker deletion, retrospective Live backfill or routine REST fallback.

The existing numerical model activation/calendar stays pinned. Do not advance
its first decision date just to fit a pre-open maintenance window. Gateway and
model clean-session counters remain separate; a partial/failed run is not clean.

## Checks

Gateway unit tests simulate a mature ten-hour process, recovery before/at the
deadline, and repeated flapping that cannot postpone startup or recovery expiry.
The owned WS/Redis CLI harness additionally injects a protocol reset after ready,
requires a new generation with ACK/history-complete readiness, and verifies that
the exact source prefix and XADD count are unchanged. It then bootstraps the
actual approved worker and retains existing stop/resume, tamper and uncertain-
ACK checks. Fault injection is limited to the owned fake WS, never live sockets.

Native verification must rebuild the current crate from a clean scoped commit,
pin hashes, and repeat these checks inside the owned network/Redis boundary.
Fresh real canary observation is required before model admission. The previously
observed terminal release is retained as terminal-path evidence, not erased or
relabelled as a clean post-fix source session.
