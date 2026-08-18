# RI Author42 BO Risk-Filter Challengers

- Date: `2026-08-18`
- Status: `APPROVED_FOR_SHADOW_OBSERVATION`
- Scope: RI Author41/42 `canonical07` shadow diagnostics only
- Live impact: none

## Context

The `2026-08-17` RI BO session produced several same-side Author42 BO re-entries.
The audit package
`analiz_alpha_si/ri_author42_bo_late_entry_audit_2026_08_18/ri_author42_bo_late_entry_audit_report.md`
shows that late BO entries are valid under the current frozen contract:

- `allow_reentry_on_day_extreme = true`;
- no model-level evening or `13:00` cutoff exists;
- live hardening only suppresses stale entry materialization at or after
  `author42_exit_time = 23:00`.

Therefore this line is not a hotfix. It is a controlled risk-filter challenger
line for 10-20 more sessions.

## Challenger Policies

Two isolated `canonical07` prospective-shadow contours are introduced:

1. `bo_last_entry17`

   - Config: `configs/runtime.ri_author41_42.shadow07.bo_last_entry17.7502MIW.toml`
   - Policy: `author42_last_entry_time = "17:00:00"`
   - Meaning: a BO signal may appear before/around the cutoff, but the actual
     next-bar entry is allowed only if that next-bar entry time is `<= 17:00`.

2. `bo_max2`

   - Config: `configs/runtime.ri_author41_42.shadow07.bo_max2.7502MIW.toml`
   - Policy: `author42_max_entries_per_day = 2`
   - Meaning: Author42 BO may materialize at most two entries per regular
     session.

Both contours must keep:

- `trade_mode = "paper"`;
- `allow_live_orders = false`;
- `allow_paper_orders = false`;
- isolated command, ack, health, runtime state and decision journal paths.

## Acceptance Window

Observe for at least 10 complete sessions, preferably 20, against:

- current RI `live07` actual broker rounds;
- current RI `shadow07` baseline;
- RI `shadow09` legacy diagnostic reference.

Daily comparison should include:

- Author41 MR PnL;
- Author42 BO PnL;
- total combo PnL;
- BO entries suppressed by `entry_after_last_entry_time`;
- BO entries suppressed by `max_entries_per_day_reached`;
- whether large positive BO continuation days were accidentally cut.

## Decision Rule

Do not promote either challenger to live based on one bad BO day.

Promotion can be discussed only if the challenger:

- reduces repeated late-day churn in live-like observation;
- preserves most large BO continuation winners;
- improves or at least does not materially degrade 10-20 session net PnL;
- does not introduce model/live drift around next-bar entry semantics.

Current preferred candidates from the audit:

- `last_entry_17`: strongest long-history/post-transition balance;
- `max_2_entries_per_day`: good operational cap with smaller semantic change.

Rejected for immediate live use:

- hard `13:00` cutoff, because it removes a large part of the intended
  continuation window and materially changes the Author42 model.
