# Fresh Path PASS

Run directory:

- `/opt/diag-captures/20260326-225749`

Stack:

- `sessiongap`

Result:

- fresh passive `create:limit -> delete:limit` passed cleanly
- `place`:
  - `request_id = 20f9cb69-ac40-41eb-8752-3a56ea7eec94`
  - `broker_order_id = 2023555935792437604`
  - `accepted -> working`
- `cancel`:
  - `request_id = 390f4ea6-8489-4a30-a93d-aba37ce5710d`
  - `accepted -> canceled`
  - `filled = 0.0`

Hardening expectation:

- no recycle on fresh control path

Observed:

- no `control_path_stale_detected`
- no `control_path_recycle_start`
- no `control_path_send_after_recycle`

Conclusion:

- fresh entry path remained unchanged;
- the `TZ 2.0` hardening did not add unnecessary recycle on healthy fresh path.
