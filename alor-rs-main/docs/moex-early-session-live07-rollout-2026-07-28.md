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
5. The exchange session is closed or the work is performed before 07:00 MSK.
6. The old live09 and new live07 runtime are never active simultaneously.

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
7. Confirm quantity `6`, session start `07:00`, MR end `09:59` and BO wait `3`.
8. Observe the first full session without changing K, quantity or exit policy.

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
