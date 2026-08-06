# IMOEXF Live MR Temporary Suspension

Date: 2026-08-04

Status: `DEPLOYED / BO_ONLY LIVE OBSERVATION`

## Decision

Temporarily run the live IMOEXF canonical07 contour as:

```text
live broker execution = BO-only bo_new_k053
High180 MR live entries = disabled
High180 10m shadow accounting = enabled
lb120 risk-gate ledger maintenance = enabled
```

This is an observation control, not a new model freeze. MR parameters remain in
the config so that the canonical 10-minute shadow path and the long-lived
risk-gate ledger continue to collect evidence for a later research recut.

## Evidence

Complete-session model replay for 2026-07-14 through 2026-08-03:

| Contour | MR | BO after MR arbitration | Combo |
|---|---:|---:|---:|
| High180 canonical07 | `-8.10` | `+61.29` | `+53.19` |
| Author41-short canonical07 | `-40.24` | `+81.29` | `+41.05` |
| BO-only canonical07 | disabled | `+60.79` | `+60.79` |

The BO-only result is concentrated in a small number of strong sessions and is
not a scale-up signal. Quantity must remain unchanged during this observation.

Broker-truth MR fills also show weak recent behavior. The completed IMOEXF MR
cycles after 2026-07-14 produced a negative quantity-weighted result. The
2026-08-04 MR cycle was still in flight when the decision sample was frozen, so
it is excluded from that sample; it later closed before the rollout.

## BO Identity

High180 and Author41 replacement profiles use the same IMOEXF breakout sleeve:

```text
bo_new_k053
bo_k = 0.53
bo_stop1_range = 0.51
bo_stop2_range = 0.35
bo_big_move_threshold = 0.025
bo_wait_hours = 3.0
```

The different BO totals in the two hybrid replays do not represent different
BO formulas. MR has priority under the one-position/no-overlap contract, so a
different MR sleeve suppresses a different subset of otherwise valid BO
entries. RI Author42 is not being imported into the IMOEXF runtime.

## Runtime Contract

The new config field is:

```toml
live_mr_entries_enabled = false
```

Default is `true`, preserving every existing HybridIntraday profile that omits
the field.

When the field is false:

- a valid live MR candidate is logged as `mr_entry_suppressed` with
  `reason=live_mr_entries_disabled`;
- no MR broker intent, pending entry, position ownership or bracket is created;
- the BO engine sees the contour as flat and is not suppressed by shadow MR;
- High180 shadow entries/exits and daily session finalization continue;
- the risk-gate stream and materialized state continue to advance;
- an already-restored real MR position can still execute its normal exit and
  reconciliation lifecycle.

## Deployment Gate

Do not deploy while the 2026-08-04 MR position or any protective order is live.

Required pre-deployment checks:

1. Broker position `IMOEXF = 0` on portfolio `7502MIW`.
2. No working IMOEXF regular order.
3. No working IMOEXF stop order.
4. Runtime has reconciled broker-flat and has no pending entry/exit tail.

The rollout may reset only the operational strategy snapshot after flat. It
must preserve the canonical risk-gate ledger, state and finalized-session
guards rooted at:

```text
runtime.riskgate.sessions.hybrid_imoexf.imoexf_primary_high180_lb120
```

The gateway configuration and action-scoped execution path are unchanged.

## Deployment Record

The patch was deployed after broker-flat confirmation on 2026-08-04:

- runtime image: `manual-20260804-imoexf-bo-only`;
- rollout time: `13:44 MSK`;
- only `trading-hybrid-strategy-runtime-1` was recreated;
- the gateway and Redis containers were not restarted;
- bootstrap reported zero open strategy positions, regular orders and stop
  orders;
- resolved config reported `live_mr_entries_enabled=false`;
- the risk-gate startup decision was `UseExistingLedger` with 242 existing
  records, last finalized session `2026-08-03` and rolling lb120 sum `90.8`.

The runtime reached `ALLOWED` on the first post-restart 10-minute bar. A later
WS reset at `13:54 MSK` coincided with the configured intermediate clearing
window. The gateway remained fail-closed in `SyncingGap` and returned to
`LiveReady` at `14:00:00`; runtime guard returned to `ALLOWED` at `14:00:08`.
At completion, the primary bar consumer group had `pending=0` and `lag=0`.

## Post-Deployment Acceptance

Verify on startup:

- resolved config prints `live_mr_entries_enabled=false`;
- existing risk-gate records are loaded instead of importing the seed again;
- `last_finalized_session_date` and `ledger_rows_count` do not move backward;
- live guard reaches `ALLOWED`;
- no unexpected working order exists.

Verify during observation:

- valid High180 shadow events continue to reach daily ledger finalization;
- a would-be live MR entry produces only the suppression diagnostic;
- an eligible BO signal can emit and execute normally;
- no MR-tagged broker order appears after the rollout boundary;
- BO remains strict no-overnight and reaches broker-flat.

## Rollback

After a new frozen 07:00 MR analysis and approval, restore:

```toml
live_mr_entries_enabled = true
```

Rollback also requires a broker-flat maintenance window. Do not rebuild or
replace the accumulated risk-gate ledger as part of that change.
