# RI Author41/42 Rollover And Session Policy

## Status

```text
session-quality guard: implemented, pending controlled rollout
automatic contract switch: not implemented
production roll policy: manual actual-expiry - 1, fallback - 2
exposure: keep current micro size until replay and live validation complete
```

This contract follows the June 2026 rollover audit. It separates session
eligibility from contract switching so that a special session cannot silently
become an Author41/42 previous-day anchor.

## Session Identity

Every exchange session must eventually carry three separate concepts:

```text
calendar_date
trade_date
session_type = regular | weekend_extra | special | shortened | closed
```

The current P0 runtime uses an explicit `excluded_model_dates` calendar plus an
observed anchor-quality guard. Raw gateway bars remain retained for audit.

Excluded dates:

```text
2026-06-12  # DSWD; caused the June anchor discrepancy
2026-11-04  # additional weekend/holiday session for trade date 2026-11-05
```

Excluded sessions:

- do not enter RI model state;
- do not become MR or BO previous-session anchors;
- do not generate RI live intents;
- remain available in raw gateway history.

## Anchor Quality Contract

The RIU6 candidate configs require:

```text
minimum usable 10m bars = 80
first model bar <= 09:10 MSK
last model bar >= 23:30 MSK
```

Only completed sessions passing these checks may become previous-session
anchors. If no eligible anchor exists, the strategy skips trading instead of
mixing contracts or using a low-quality session.

This is a safety/session-definition policy, not an Author41/42 parameter tune.

## Rollover Contract

Use actual exchange expiry dates. Do not derive expiry from an approximate
nominal date.

```text
target:   switch before actual-expiry - 1 trading session
fallback: switch before actual-expiry - 2 trading sessions
```

For RIU6:

```text
actual expiry: 2026-09-17
target switch: between sessions after 2026-09-15, before 2026-09-16
fallback switch: between sessions after 2026-09-14, before 2026-09-15
next contract: RIZ6 / RTS-12.26
```

The switch remains a controlled manual procedure. Runtime validates and logs
the configured expiry/offset policy but does not mutate symbols or compose
streams automatically.

## Roll GO Gate

All conditions are mandatory:

1. Old and new RI contracts are broker-flat.
2. No working regular or stop orders exist for either contract.
3. The next contract resolves through the broker and action-scoped Market path.
4. New-contract history is available for 10m warmup.
5. Spread, volume, and open interest pass the operator liquidity check.
6. Runtime and gateway configs, compose stream overrides, model symbol, and
   broker order symbol all reference the new contract.
7. Runtime starts from zero between sessions.
8. The first thin/incomplete new-contract session cannot become an anchor.

Any failed condition is a NO-GO.

## Required Logs

On startup:

```text
action=ri_rollover_policy_loaded
active_contract
order_symbol
actual_expiry_date
roll_target_sessions_before
roll_fallback_sessions_before
excluded_model_dates
min_anchor_bars
anchor_first_bar_at_or_before
anchor_last_bar_at_or_after
```

When a configured special session is observed:

```text
action=ri_model_session_excluded
calendar_date
reason=configured_non_regular_session
active_contract
```

## Follow-Up

Before making late rollover the fully automated production default:

- add a versioned MOEX calendar source with explicit `trade_date/session_type`;
- implement configurable Rust replay for `expiry - 7/-2/-1`;
- reproduce all 18 historical transitions and the June 2026 special-session
  case;
- add explicit anchor date/contract/quality diagnostics;
- review whether automatic symbol/stream switching is operationally desirable.

