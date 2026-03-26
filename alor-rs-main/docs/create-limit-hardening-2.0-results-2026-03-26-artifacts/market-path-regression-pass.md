# Market Path Regression PASS

Run directory:

- `/opt/diag-captures/20260326-231806-hybrid-market`

Stack:

- `hybrid` paper

Observed:

- market buy:
  - `request_id = 7eed03c0-115c-4536-82a6-28934287e414`
  - `broker_order_id = 2033126085000190190`
  - `filled`
- market sell:
  - `request_id = 996824f7-4515-49f9-81b3-fd83da5328b1`
  - `broker_order_id = 2033126085000190238`
  - `filled`

Hardening expectation:

- market path remains out of scope for recycle-before-send

Observed:

- no `control_path_stale_detected`
- no `control_path_recycle_start`
- no `control_path_recycle_success`
- no `control_path_send_after_recycle`

Conclusion:

- no fresh market-path regression was introduced by `TZ 2.0`.
