# Redis Test Isolation And Coverage

## Two Different Test Contracts

The CI workflow provisions an isolated Redis service on loopback port6379.
Some legacy runtime unit-test helpers still construct
`redis://127.0.0.1/` directly. Setting `REDIS_URL` does not redirect every unit
helper. Never run that suite assuming an existing developer Redis is disposable;
use a dedicated CI/container/network environment. Do not flush, restart or
reconfigure an unrelated local or production Redis to make tests pass.

Several legacy runtime E2E files are opt-in via `REDIS_URL`: without it they
return early. An `ok` result in that mode is not evidence of an exercised
Redis integration path. A Redis service in CI and an enabled legacy E2E suite
are separate facts. Inspect each fixture's connection policy before enabling it.

The newer explicitly owned shadow probes use their documented PID/loopback,
identity and source/worker opt-in contracts. Ordinary unit/policy success does
not replace native WS/Redis smoke, recovery or terminal/EOD acceptance.

## Existing Legacy E2E Gap

The publication qualification on November13 reproduced all four failures in
`strategy-runtime/tests/e2e_buy_and_close.rs` against an owned ephemeral Redis,
both on the unmodified published baseline `2a1d80d` and on the shadow engineering
tree. Errors include `empty xread reply` and `missing runtime state entry`.
That runtime/strategy/guard fixture path was not changed by the shadow import.

Keep this as an explicit follow-up, not a new shadow regression and not an
all-green workspace claim. Debug fixture startup/readiness, error propagation,
state and event clocks before changing production behavior. Do not weaken live
guards, increase arbitrary timeouts or omit assertions just to silence the test.
The qualified shadow publication does not include the separate mixed local
paper/runtime work or a repair of this legacy E2E family.

## Reporting Rules

- Record actually executed tests separately from ignored or environment-skipped
  Redis/native tests; retain the exact command and source revision.
- Report baseline failures and newly introduced failures separately.
- Native/prospective receipt evidence, historical numerical parity and actual
  broker execution are different acceptance layers.
- Keep raw qualification logs, credentials, temporary databases, binaries and
  generated reports outside the engineering repository.
