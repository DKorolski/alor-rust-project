# Alor USDRUBF Orphan Trade Follow-Up - 2026-04-23

Date: 2026-04-23

Scope:

- `trading-alor-usdrubf`
- post-cutover `10m` live contour
- accepted `action-scoped create:market` flows

## Observation

During the 2026-04-23 session, `trading-alor-usdrubf` produced two runtime `orphan_trade` warnings:

1. `2026-04-23T08:10:01Z`
   - `trade_id = 2023556021691095692`
   - `order_id = 2023556021691182542`
   - `side = sell`
   - `qty = 1.0`
   - `price = 74.89`
2. `2026-04-23T15:00:06Z`
   - `trade_id = 2023556021691126265`
   - `order_id = 2023556021691578096`
   - `side = sell`
   - `qty = 1.0`
   - `price = 75.73`

These warnings did not coincide with the old-style market-send transport failure.

Gateway logs around the same paths showed:

- action-scope session open
- forced token refresh before authorize
- `authorize ok`
- `create:market`
- `http_code=Some(200)`
- clean session close

## Reading

The most likely interpretation is:

- market send itself succeeded on the intended action-scoped path
- broker accepted the command
- runtime still failed to correlate the resulting trade into the expected lifecycle and reported `orphan_trade`

So this looks more like:

- trade / order / request lineage matching drift

and less like:

- `create:market` transport-send path regression

## Why this matters

This is a smaller issue than the pre-patch `create:market` burst / defer / timeout storm, but it is still important because it affects operator confidence and lifecycle clarity.

If this persists, operators can no longer assume that:

- accepted action-scoped market flow
- runtime trade confirmation path

are fully aligned in every case.

## Current verdict

Status:

- not a blocker-level transport incident
- not evidence that the new action-scoped market path failed
- valid follow-up engineering signal

Priority:

- medium

## Recommended next step

Review the two 2026-04-23 accepted market cases specifically for:

- request lineage
- broker order id propagation
- trade matching path in runtime
- whether the trade arrived before the relevant pending state had been fully installed or retained

If repeated on later sessions, this should become a dedicated fix line.
