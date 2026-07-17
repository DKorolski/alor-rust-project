# MOEX Early Session Shadow Observation Template

Date:

Observer:

Session:

```text
weekday:
complete_session: yes/no
market_notes:
```

## Service Health

```text
ri_legacy09:
ri_canonical07:
usdrubf_legacy09:
usdrubf_canonical07:
imoexf_legacy09:
imoexf_canonical07:
```

## Safety

```text
broker_commands_emitted: 0 expected
broker_orders_created: 0 expected
broker_positions_created_by_shadow: 0 expected
command_stream_check:
```

## Data Quality

```text
auction_0650_excluded:
first_canonical07_bar:
first_legacy09_bar:
missing_10m_bars:
stale_price_flags:
zero_volume_flags:
```

## Strategy Summary

| Strategy | Contour | MR trades | BO trades | Pre-09 decisions | PnL points | Notes |
|---|---|---:|---:|---:|---:|---|
| RI Author41/42 | legacy09 | | | | | |
| RI Author41/42 | canonical07 | | | | | |
| Alor-USDRUBF | legacy09 | | | | | |
| Alor-USDRUBF | canonical07 | | | | | |
| IMOEXF Hybrid | legacy09 | | | | | |
| IMOEXF Hybrid | canonical07 | | | | | |

## Divergence Classification

```text
same_decision:
anchor_drift:
session_open_drift:
clock_shift_only:
side_changed:
legacy_only:
canonical_only:
overlap_arbitration_changed:
risk_gate_changed:
exit_path_changed:
feed_quality_difference:
```

## Volume Diagnostics

```text
volume_0700_0859:
volume_since_0900:
early_volume_share:
signal_bar_volume_notes:
```

## Engineering Notes

```text
restart/replay_determinism:
riskgate_ledger_update:
unexpected_logs:
operator_action_required:
```

## Verdict

```text
KEEP_OBSERVING
INVESTIGATE
REPLAY_MISMATCH
SAFETY_STOP
```
