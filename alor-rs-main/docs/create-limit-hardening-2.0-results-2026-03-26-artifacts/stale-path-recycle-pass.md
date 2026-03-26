# Stale Path Recycle PASS

Run directory:

- `/opt/diag-captures/20260326-225923`

Stack:

- `sessiongap`

Pre-send stale snapshot:

- `CWS_CONNECTION_INSTANCE_ID = 0fd446a4-0eae-4707-9a39-b480fb20230d`
- `CWS_CONNECTION_AGE_SEC = 1052`
- `CWS_LAST_TX_AGE_MS = 1052133`
- `CWS_LAST_CONTROL_SUCCESS_AGE_MS = na`
- `CWS_PENDING_COUNT = 0`
- `REQUEST_MAP_SIZE = 0`

Probe:

- `place request_id = f0e60be3-0778-4459-8023-99846893c015`
- `broker_order_id = 2023555935792442183`
- `cancel request_id = 3fe9596f-2933-4224-ab84-34b1c9ef3d1e`

Observed:

- `control_path_stale_detected`
- `control_path_recycle_start`
- `control_path_recycle_success`
- `control_path_send_after_recycle`

Connection switch:

- previous:
  - `0fd446a4-0eae-4707-9a39-b480fb20230d`
- fresh:
  - `53c26739-3775-4f67-9998-84145da78e9f`

Order lifecycle:

- `place accepted -> working`
- `cancel accepted -> canceled`
- `filled = 0.0`

Counters after run:

- `control_path_stale_detected_total = 1`
- `control_path_recycle_total = 1`
- `control_path_recycle_success_total = 1`
- `control_path_recycle_failed_total = 0`
- `control_path_stale_blocked_send_total = 0`

Conclusion:

- `TZ 2.0` achieved the intended stale-path behavior live:
  - detect stale
  - recycle first
  - only then send
