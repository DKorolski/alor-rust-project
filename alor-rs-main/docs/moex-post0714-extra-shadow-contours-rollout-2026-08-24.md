# MOEX Post-0714 Extra Shadow Contours Rollout

- Date: `2026-08-24`
- Status: `DEPLOYED_FOR_SHADOW_OBSERVATION`
- Scope: RI, Alor-USDRUBF, IMOEXF diagnostic shadow contours
- Live impact: none

## Context

The post-`2026-07-14` regime audit recommended keeping live micro contours on
their current production contracts while adding controlled shadow alternatives
for the three MOEX systems.

Reference package:

```text
analiz_alpha_si/moex_post_0714_regime_audit_2026_08_24/
```

This rollout adds only diagnostics. All new services must keep:

- `trade_mode = "paper"`;
- `allow_live_orders = false`;
- `allow_paper_orders = false`;
- isolated runtime state, health, command streams and decision journals.

## Added Contours

### RI Author41/42

Directory on VPS:

```text
/opt/trading-moex-early-shadow-ri
```

New services:

- `runtime-shadow07-bo-last-entry17`
- `runtime-shadow07-bo-max2`

Configs:

```text
configs/runtime.ri_author41_42.shadow07.bo_last_entry17.7502MIW.toml
configs/runtime.ri_author41_42.shadow07.bo_max2.7502MIW.toml
```

Purpose:

- compare BO risk-filter candidates against current RI `live07`;
- observe `last_entry_17` and `max_2_entries_per_day` without changing live.

### Alor-USDRUBF Hybrid

Directory on VPS:

```text
/opt/trading-moex-early-shadow-usdrubf
```

New services:

- `runtime-shadow07-mr1040-bo2`
- `runtime-shadow07-mr1140-bo2`

Configs:

```text
configs/runtime.alor_usdrubf.shadow07.mr1040_bo2.7502MIW.toml
configs/runtime.alor_usdrubf.shadow07.mr1140_bo2.7502MIW.toml
```

Purpose:

- compare earlier/later MR windows under canonical `07:00` session state;
- compare BO wait-2 behavior without touching live USDRUBF micro.

### IMOEXF Hybrid

Directory on VPS:

```text
/opt/trading-moex-early-shadow-imoexf
```

New services:

- `runtime-weekend-state-live07-phase`
- `runtime-weekend-state-legacy09-author41`
- `runtime-weekend-state-compromise-mr1059-bo4`

Configs:

```text
configs/runtime.hybrid_imoexf.shadow_weekend_state.live07_phase.7502MIW.toml
configs/runtime.hybrid_imoexf.shadow_weekend_state.legacy09_author41.7502MIW.toml
configs/runtime.hybrid_imoexf.shadow_weekend_state.compromise_mr1059_bo4.7502MIW.toml
```

Purpose:

- compare Author41-short weekend-state variants;
- keep weekend bars in model state/anchors where the audit requires it;
- still suppress all weekend trade emission.

## Engineering Change

IMOEXF required a runtime-level weekend-state policy because the existing
hybrid runtime could only skip weekend bars entirely.

Added policy:

```text
weekend_state_policy = "state_only"
```

Contract:

- weekday bars behave normally;
- weekend bars update model state via warmup/state path;
- weekend bars do not emit live or paper intents;
- default remains `skip` for existing configs.

## VPS Image

New IMOEXF weekend-state services use:

```text
ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-20260824-shadow-weekend-state
```

RI and USDRUBF extra services reuse already deployed shadow-compatible images.

## Initial Verification

Local checks:

```text
cargo fmt --all -- --check
cargo test -p strategy-runtime -- --test-threads=1
cargo test -p strategy-runtime shadow_policies --test config_tests -- --nocapture
cargo test -p strategy-runtime loads_moex_early_session_shadow_configs_as_diagnostics_only --test config_tests -- --nocapture
cargo test -p strategy-runtime weekend_state_only_updates_model_state_without_emitting_intents -- --nocapture
cargo clippy -p strategy-runtime --all-targets -- -D warnings
```

VPS checks after rollout:

- all seven new containers are `healthy`;
- `errors = 0` for all seven new containers;
- `intent_emitted = 0`, `command prepared = 0`, `command_sent = 0`;
- live guard remains blocked by `allow_live_orders=false` and
  `trade_mode=Paper`;
- Redis usage remains small: RI about `14M`, USDRUBF about `14M`, IMOEXF about
  `16M`;
- VPS resources remain normal: disk about `23%`, available RAM about `6.3GiB`.

## Observation Plan

Observe for at least `10` complete sessions, preferably `20-30`, before any
promotion decision.

Daily comparison should include:

- live actual broker rounds;
- baseline shadow07 and legacy09;
- new challenger shadow contours;
- MR/BO attribution separately;
- suppressed BO entries for RI risk-filter candidates;
- weekend-state impact for IMOEXF.

Current live micro contracts are unchanged by this rollout.

## 2026-09-04 Observation Hardening

Status: `READY_FOR_NEXT_SHADOW_OBSERVATION_WINDOW`

The IMOEXF weekend-state comparison needs component-level attribution before any
promotion decision. The raw closed-trades CSV remains the canonical paper report,
but new runtime builds enrich it with hybrid comment context:

- `strategy_component`: `MR`, `BO` or `UNKNOWN`;
- `entry_role` / `exit_role`: derived from the `HYB|...|r=...` comment tag;
- `entry_comment` / `exit_comment`: retained for audit and later replay joins.

Rows generated before this change remain valid and are read in append mode with
empty component fields. Treat `UNKNOWN` as historical/reporting gap, not as a
strategy signal problem.

Weekly IMOEXF shadow review helper:

```bash
python3 scripts/imoexf_shadow_observation_summary.py \
  --input /reports/moex_early_session_imoexf_trades_weekend_state_live07_phase.csv \
  --input /reports/moex_early_session_imoexf_trades_weekend_state_legacy09_author41.csv \
  --input /reports/moex_early_session_imoexf_trades_weekend_state_compromise_mr1059_bo4.csv \
  --since 2026-09-07 \
  --output-md /reports/imoexf_shadow_observation_summary_2026-09-07.md \
  --output-json /reports/imoexf_shadow_observation_summary_2026-09-07.json
```

The helper also labels cross-day exits:

- `same_day`: normal same-session close;
- `next_session_07xx_gap_exit`: bar-driven next-session `07:00..07:20`
  observation close;
- `next_session_0930_legacy_exit`: legacy next-session `09:20..09:40`
  observation close;
- `cross_day_other`: requires manual review.

Decision hygiene for the next `1-2` weeks:

- keep live IMOEXF MR disabled;
- keep all weekend-state services paper/shadow-only;
- compare `weekend_state_live07_phase` against `legacy09_author41` and
  `compromise_mr1059_bo4` by total PnL, MR PnL, BO PnL and gap labels;
- do not promote on total PnL alone if new `MR` attribution remains weak;
- require no `cross_day_other` gap labels before any live-MR promotion review.
