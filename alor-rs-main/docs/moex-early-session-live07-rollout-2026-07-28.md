# MOEX Early-Session Live07 Rollout

Date: 2026-07-28

## Decision

The workline is split into two independent changes.

1. IMOEXF Hybrid is prepared for a controlled live07 replacement rollout.
2. RI Author41/42 remains live09. Its canonical07 shadow is upgraded to a
   prospective adapter that mirrors the live event loop without emitting
   broker commands.

Alor-USDRUBF is outside this change.

## IMOEXF Live07 Candidate

Runtime:

```text
configs/runtime.hybrid.live.7502MIW.riskgate-canonical07.toml
```

Gateway:

```text
configs/gateway.hybrid.live.7502MIW.action-scoped.canonical07.toml
```

Only the opening-session clock translation changes:

```text
regular/model start: 09:00 -> 07:00
MR session end:      11:59 -> 09:59
MR forced bar:       11:50 -> 09:50
BO wait:             unchanged at 3h
first BO eligibility approximately 10:00
```

The following production contracts are unchanged:

- profile `imoexf_primary_riskgate_high180_lb120`;
- MR variant `high180`;
- all MR K/TP/SL values;
- BO K, stop and range values;
- quantity `6`;
- same-day no-overnight policy;
- action-scoped Market/CreateStopLimit/DeleteStopLimit paths;
- canonical production risk-gate ledger
  `runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120`;
- `session_rollover_hour_utc = 6`.

The candidate uses a new consumer group and runtime-state key. This gives the
runtime a clean operational bootstrap without deleting or rebuilding the
long-lived risk-gate ledger.

The risk-gate ledger has an explicit audited session-policy transition. Rows
before `2026-07-28` retain `Mon-Fri 09:00..23:49`; rows from `2026-07-28`
onward use `Mon-Fri 07:00..23:49`. Runtime identity validation accepts each
policy only on its proper side of that boundary. It does not relabel historical
rows. If rollout is delayed beyond the planned 2026-07-29 session, the
transition date must be moved to the first prior session that the new runtime
will reconstruct as canonical07.

## RI Prospective Shadow

The canonical07 runtime config now uses:

```text
mode = "prospective_shadow"
allow_order_emission = false
```

The mode runs the same prospective Author41/Author42 event-loop functions as
micro-live. Candidate entry and exit intents are journaled as:

```text
adapter_decision = "prospective_intent_suppressed"
```

Each candidate row includes `candidate_scheduled_ts_local`, so entry/exit
timing can be compared directly with the micro-live journal.

The adapter assumes an immediate shadow fill after each candidate so the next
bar sees the same position lifecycle that a normally filled live order would
produce. It always returns an empty intent list to the host. Broker
ack/position/command-prepared callbacks cannot advance this shadow lifecycle.

The existing finalized batch/no-overlap journal remains active in the same
generation. This provides two views in one append-only journal:

- `shadow_path_active`: frozen replay/finalized path;
- `prospective_intent_suppressed`: operational live-like path.

The new generation is isolated by:

```text
consumer_group = strategy-runtime-ri-author41-42-shadow07-prospective-v1-7502MIW
runtime_state = runtime.state.ri_author41_42.shadow07.prospective_v1.7502MIW
journal = /reports/moex_early_session_ri_decisions_canonical07_prospective_v1.jsonl
```

RI live09 is not changed by this patch.

## Safety Gate

Do not replace the IMOEXF live contour unless all conditions are true:

1. Broker position in `IMOEXF` on `7502MIW` is exactly flat.
2. There are no working regular or stop orders for `IMOEXF`.
3. Runtime has no pending entry, exit or protective-repair lifecycle.
4. The canonical risk-gate ledger exists and its last finalized session is
   recorded before the restart.
5. The configured risk-gate session-policy transition date matches the actual
   rollout/reconstruction boundary.
6. The exchange session is closed or the work is performed before 07:00 MSK.
7. The old live09 and new live07 runtime are never active simultaneously.

The RI prospective shadow should also start before 07:00 MSK so its first
journal date is a complete session.

## Rollout Sequence

### RI shadow

1. Preserve the old canonical07 journal.
2. Deploy the patched strategy-runtime image.
3. Replace only the RI canonical07 shadow runtime config.
4. Restart only that shadow runtime.
5. Confirm mode `prospective_shadow` and the new consumer/state generation.
6. Confirm zero rows with `intent_emitted`, `request_id` or `broker_order_id`.

### IMOEXF live

1. Run the full safety gate.
2. Stop the IMOEXF live09 runtime and gateway.
3. Install both canonical07 candidate configs.
4. Start the gateway first and wait for `LiveReady`.
5. Start the runtime and confirm clean history/startup replay suppression.
6. Confirm production risk-gate ledger load and current gate state.
7. Confirm legacy ledger rows remain `09:00` and the first new row is `07:00`.
8. Confirm quantity `6`, session start `07:00`, MR end `09:59` and BO wait `3`.
9. Observe the first full session without changing K, quantity or exit policy.

## Verification

RI journal review:

```bash
python3 scripts/ri_author41_42_journal_review.py \
  /reports/moex_early_session_ri_decisions_canonical07_prospective_v1.jsonl \
  --strict-pre-go \
  --from-date 2026-07-29
```

Required RI result after 5-10 complete sessions:

- no broker-emission evidence;
- no unexpected execution path;
- prospective entries/exits deduplicated by `decision_key`;
- MR and BO economics reported separately;
- candidate timing compared with live09 and finalized canonical07.

Required IMOEXF result after the first 5 clean sessions:

- no orphan or residual position;
- protective TP/SL lifecycle completes or reconciles broker-flat;
- MR is flat before BO eligibility;
- no overnight carry;
- risk-gate ledger finalizes exactly one row per regular session.

## Rollback

The old live09 configs and runtime-state key remain intact. Rollback is:

1. reach broker-flat and cancel all IMOEXF working/stop orders;
2. stop canonical07 runtime and gateway;
3. restore the previous live09 gateway/runtime configs;
4. start gateway, wait for readiness, then start runtime;
5. verify the old runtime-state reconciliation and broker-flat state.

Never roll back by running both live generations together.

## Actual Rollout Record - 2026-07-29

### Deployment outcome

The rollout was performed before the regular session after confirming:

- portfolio `7502MIW` was broker-flat;
- there were no working regular or stop orders;
- the VPS had approximately 6 GiB available RAM and 37 GiB free disk space.

RI canonical07 was restarted only as an isolated prospective shadow:

- config: `runtime.ri_author41_42.shadow07.7502MIW.toml`;
- image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260728-ri-prospective-riskgate-c73a79e`;
- mode: `prospective_shadow`;
- both live and paper order emission were disabled;
- the isolated command stream remained empty and no broker request identifiers were produced.

RI live09 and legacy09 shadow were not changed.

IMOEXF was moved from live09 to canonical07 using:

- gateway config: `gateway.hybrid.live.7502MIW.action-scoped.canonical07.toml`;
- runtime config: `runtime.hybrid.live.7502MIW.riskgate-canonical07.toml`;
- runtime image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260728-ri-prospective-riskgate-c73a79e`.

The old runtime and gateway were stopped before the new generation started. The
gateway was started first and reached CWS-authorized readiness. The runtime then
reached `LiveReady` at `07:10:08 MSK`; the startup replay guard was released by
the first live 10-minute bar. The deployed contract was confirmed as quantity
`6`, session start `07:00`, MR end `09:59`, BO wait `3` hours and action-scoped
gateway execution.

### Risk-gate continuity

Before rollout, the production ledger contained 237 finalized sessions through
`2026-07-27`, with rolling `lb120` shadow PnL of approximately `117` and the MR
gate enabled.

A clean runtime generation does not reconstruct the previous unfinished
risk-gate session from warmup bars. Therefore, the canonical07 shadow result for
`2026-07-28` was imported into the production ledger through the same
idempotent `SETNX`/`XADD`/`HSET` write contract:

- `shadow_pnl_points=3.9`;
- `shadow_trade_count=1`;
- rolling `lb120` changed from approximately `117` to `112`;
- `mr_enabled_current_session=true`;
- `mr_enabled_next_session=true`;
- session policy: regular weekdays, `07:00..23:49`.

Pre-import backups were saved on the VPS:

- `/opt/rollout-candidates/c73a79e/riskgate-production-ledger.pre-20260728-backfill.json`;
- `/opt/rollout-candidates/c73a79e/riskgate-production-state.pre-20260728-backfill.json`.

The resulting materialized state had 238 rows, last finalized session
`2026-07-28`, rolling `lb120=112.00000000000003`, current shadow session
`2026-07-29` and the MR gate enabled.

The already-running process retained its startup diagnostic rolling value until
the next finalization or controlled restart. No intraday restart was performed
because both persisted and in-memory states kept the gate enabled, so trading
behavior was unaffected.

### Startup catch-up observation

The new consumer generation initially read historical broker trade/order events.
This produced catch-up-only orphan/stale-stop warnings and stale cancel attempts,
mostly `Order to cancel not found`, plus one transient OAuth refresh `502`.
Broker truth remained flat, no regular or stop orders were created, and consumer
lag converged to zero.

Follow-up hardening: a future clean-generation rollout should initialize
non-bar consumer groups at the stream tail while retaining historical bar warmup.

### Post-check

After the first live bar:

- all target containers were healthy;
- all portfolio positions, regular orders and stop orders were empty;
- RI prospective shadow had emitted zero commands;
- no target `WARN` or `ERROR` events appeared after `07:10`;
- VPS memory and disk headroom remained safe.

Status: rollout completed successfully; continue controlled live/shadow
observation without changing quantity, K values or exit policy.

## Post-Rollout Hybrid Lifecycle Incident - 2026-07-29

### Live false overnight rescue

The first canonical07 IMOEXF BO entry inherited `active_cycle_id=6a16ab8800`
from a historical tagged stop observed during startup catch-up. The stop later
became terminal, but the historical cycle remained available and
`SubmitEntry` reused it for a new flat-to-open BO lifecycle.

The cycle encoded `2026-05-27`, while the new entry occurred on `2026-07-29`.
The no-overnight guard therefore classified the valid intraday position as a
cross-day carry and emitted an early market exit. Broker truth converged to
flat, but the exit was not a model exit.

Fix:

- every accepted flat-to-open hybrid entry allocates a fresh cycle from the
  final runtime event timestamp;
- historical tagged orders may still restore lifecycle context for
  reconciliation, but that context cannot be reused by a later new entry;
- a regression test covers a historical working-to-canceled stop followed by a
  fresh BO entry and requires no premature no-overnight action.

### Paper synthetic-fill residual

The IMOEXF shadow runtimes create a paper entry on one model bar and apply its
synthetic fill on the following 10-minute bar. The shared pending cleanup used
the live `pending_timeout_sec=60`, so the next bar first cleared the pending
entry at age 600 seconds and then treated the synthetic position fill as an
unexpected broker residual.

Fix:

- wall-clock pending timeout cleanup is now live-only;
- paper and backtest pending state remains valid until the bar-driven synthetic
  fill or the paper lifecycle resolves it;
- a regression test advances a paper entry by 600 seconds and requires the
  following synthetic fill to be accepted without residual safe mode.

### Validation and rollout scope

Validation completed locally:

- `cargo fmt --all -- --check`;
- full `cargo test -p strategy-runtime`;
- `cargo clippy -p strategy-runtime --all-targets -- -D warnings`;
- dedicated stale-cycle and paper next-bar-fill regression tests.

The runtime-only rollout is limited to:

- `/opt/trading-hybrid`, service `strategy-runtime`;
- `/opt/trading-moex-early-shadow-imoexf`, services `runtime-shadow07` and
  `runtime-shadow09`.

RI, USDRUBF, all gateways, Redis instances, strategy parameters and risk-gate
formulas remain unchanged. The canonical risk-gate streams and materialized
state must be preserved. Shadow runtime state may be restarted cleanly, but its
risk-gate ledgers must not be reset.

Acceptance after deployment:

- all three target runtimes become healthy;
- live broker truth is flat with no working regular or stop orders before
  restart;
- the live risk-gate ledger row count, last finalized session and gate status
  remain unchanged;
- both shadow command streams remain empty;
- no new `breakout_no_overnight_guard_exit` may reference a cycle date older
  than the current live entry;
- no paper synthetic fill may produce
  `broker_residual_emergency_exit` after `hybrid_pending_gc_entry`.

### Actual hardening rollout

The runtime hardening was deployed at `20:14 MSK` on `2026-07-29` after all
portfolio instruments were broker-flat and no working regular or stop orders
remained.

- commit: `244be59`;
- image tag: `manual-20260729-hybrid-lifecycle-244be59`;
- image architecture: `linux/amd64`;
- image tar SHA-256:
  `ebf3bad81ba3243ea7cecaee9e0fb63e4bdf9588b4a15c942d3cf46011b7190a`.

The GHCR push was denied because the local registry session did not have usable
package-write credentials. The exact tagged image was therefore exported and
loaded directly into Docker on the VPS. This does not change the running image
identity, but the tag is VPS-local until it is later published to GHCR.

Only the live IMOEXF runtime and the two IMOEXF shadow runtimes were recreated.
The two shadow operational runtime-state streams were archived and reset.
Gate ledgers and materialized gate state were retained.

Pre-rollout backups are stored under `/opt/rollout-candidates/244be59`.

Post-restart checks:

- all three target containers were healthy;
- live gate state remained at 238 rows, last finalized `2026-07-28`, rolling
  `lb120=112.0`, with current and next MR sessions enabled;
- shadow07 and shadow09 remained at 184 rows each, with their rolling values
  unchanged;
- shadow current-session metadata advanced cleanly to `2026-07-29`;
- both shadow command streams remained absent/empty;
- live released `waiting_for_next_bar_after_restart` on the next 10-minute bar
  and did not re-emit the earlier BO entry;
- the guard-release bar did not create a broker position, regular order or stop
  order.

The naturally occurring shadow BO candidate at model time `20:10 MSK` then
provided the focused production confirmation. More than 60 seconds after the
candidate, both shadow states still retained their pending entry. On the next
10-minute bar they accepted the synthetic fill and converged to:

- `last_position_qty=6`;
- `current_owner=intraday_breakout`;
- `pending_entry=null`;
- `active_cycle_id=6a6a33e800`, representing the current session;
- `safe_mode_close_only=false`.

No `hybrid_pending_gc_entry`, `broker_residual_emergency_exit`, live broker
command or shadow command-stream record accompanied the transition. The
bar-driven paper fill fix is therefore confirmed in the deployed event loop.

On the following live bar, the still-valid BO-long condition generated a new
live entry rather than replaying the earlier request:

- model event time: `20:20 MSK`;
- request id: `fd98f330-8ff5-5a05-bcd9-a3c415636ca9`;
- action-scoped CWS result: accepted, HTTP `200`;
- broker fill: `6 @ 2246.5`;
- broker/runtime cycle: `6a6a364000`, representing the current `2026-07-29`
  session;
- owner: `IntradayBreakout`;
- safe mode: disabled.

This position is controlled by the unchanged BO stop/EOD contract. It also
provides the first deployed evidence that a fresh live entry no longer reuses
the stale `6a16ab8800` cycle.

The following live bar kept broker and runtime position at `+6 @ 2246.5`.
No `breakout_no_overnight_guard_exit`, exit intent, `WARN` or `ERROR` occurred,
confirming that the stale-cycle false overnight rescue did not recur.
