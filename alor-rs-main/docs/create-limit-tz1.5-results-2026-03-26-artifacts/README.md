# TZ 1.5 Review Artifacts

This directory contains a compact review bundle for the `TZ 1.5` live runs documented in:

- `docs/create-limit-tz1.5-results-2026-03-26.md`

It is intentionally smaller than the full raw VPS capture directories.

Included here are the files most useful for review:

- preflight summary
- preflight readiness JSON
- preflight `/debug/cws` JSON
- post readiness JSON
- post `/debug/cws` JSON
- gateway post log

The full raw capture directories remain on VPS under:

- `/opt/diag-captures/20260326-100944`
- `/opt/diag-captures/20260326-103237`
- `/opt/diag-captures/20260326-114756`

## Cases

### `idle20-pass`

Source run:

- `/opt/diag-captures/20260326-100944`

Meaning:

- fresh baseline
- ~20m idle
- passive `create:limit -> delete:limit`
- clean `PASS`

Key files:

- `sessiongap.preflight.idle20_before.summary.txt`
- `sessiongap.preflight.idle20_before.readiness.json`
- `sessiongap.preflight.idle20_before.cws.debug.json`
- `sessiongap.readiness.post.json`
- `sessiongap.cws.debug.post.json`
- `sessiongap.gateway.post.log`

### `idle30-repro`

Source run:

- `/opt/diag-captures/20260326-103237`

Meaning:

- fresh baseline
- ~30m idle
- first passive `create:limit`
- clean `REPRO` with `protocol_reset_without_close_handshake`

Key files:

- `sessiongap.preflight.idle30_before.summary.txt`
- `sessiongap.preflight.idle30_before.readiness.json`
- `sessiongap.preflight.idle30_before.cws.debug.json`
- `sessiongap.readiness.post.json`
- `sessiongap.cws.debug.post.json`
- `sessiongap.gateway.post.log`

### `idle30-keepalive-repro`

Source run:

- `/opt/diag-captures/20260326-114756`

Meaning:

- fresh baseline
- ~15m idle
- one safe passive keepalive `place -> cancel`
- another ~15m idle
- main passive `create:limit`
- keepalive passed, main probe still `REPRO`

Key files:

- `sessiongap.preflight.keepalive15_before.summary.txt`
- `sessiongap.preflight.keepalive15_before.readiness.json`
- `sessiongap.preflight.keepalive15_before.cws.debug.json`
- `sessiongap.preflight.idle30_after_keepalive_before.summary.txt`
- `sessiongap.preflight.idle30_after_keepalive_before.readiness.json`
- `sessiongap.preflight.idle30_after_keepalive_before.cws.debug.json`
- `sessiongap.readiness.post.json`
- `sessiongap.cws.debug.post.json`
- `sessiongap.gateway.post.log`
