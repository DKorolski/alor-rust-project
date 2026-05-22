# Weekend Session Policy: IMOEXF Hybrid MR + BO

Date: 2026-04-26

## Why This Exists

In 2026, the MOEX feed can contain weekend bars/sessions. The model was not
validated as a weekend-trading strategy, and using weekend bars as ordinary
previous-day anchors materially changes the MR/BO state definitions.

This policy is part of the frozen handoff contract.

## Canonical Data Policy

Use the raw data as delivered, including weekend bars if they are present.

Then build a trading calendar view:

- `regular_weekday_session`: Monday-Friday session with bars in
  `09:00:00..23:49:00`.
- `weekend_session`: Saturday/Sunday bars, if present.
- `tradable_session`: `regular_weekday_session` only.

Do not delete weekend rows from the raw audit dataset. Do exclude them from
signal generation, trade entry, and trade exit simulation for this package.

## Anchor Policy

All previous-session anchors used by this package must be regular weekday
anchors.

For any tradable weekday `D`:

- `previous_regular_close` is the close of the most recent prior
  `regular_weekday_session`.
- `previous_regular_high/low/range` are from that same prior regular weekday
  session.
- Weekend sessions must not become `previous_close`, `previous_high`,
  `previous_low`, or `previous_range`.

Monday behavior:

- Monday uses Friday as the anchor when Friday is the most recent regular
  weekday session.
- If Friday is missing/holiday, walk back to the most recent earlier regular
  weekday session.
- Saturday/Sunday bars are ignored for anchor selection.

## MR Policy

MR uses:

```text
anchor_policy = regular_weekday_anchor
trade_weekends = false
trade_mondays = true
```

MR may trade on Monday, but its previous anchor is the previous regular weekday
session, not Sunday.

MR risk gate uses the same regular weekday trading calendar. Weekend bars do
not generate shadow trades and do not create separate risk-gate decision dates.

## BO Policy

BO uses:

```text
bo_exclude_weekends = true
```

BO entries/exits are evaluated only on tradable weekday sessions. BO previous
close/range inputs must also come from the previous regular weekday session for
consistency with the 2026 weekend-session treatment.

## Replay Acceptance

A replay/runtime implementation is not equivalent if:

- it trades Saturday or Sunday;
- it uses Saturday/Sunday as Monday's previous session;
- it lets weekend bars update MR or BO anchors;
- it computes the MR risk gate on weekend pseudo-sessions;
- it compares against the reference package after dropping weekend data in a
  way that changes weekday session timestamps or bar order.

The expected behavior is: keep weekend data available for audit, but run this
model on a regular-weekday trading calendar with regular-weekday anchors.
