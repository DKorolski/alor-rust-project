# TZ 1.4: Preflight Snapshot And Activity-Aging Matrix

Date: 2026-03-25

Related documents:

- `docs/create-limit-review-submission-2026-03-25.md`
- `docs/create-limit-diagnostic-status-update-2026-03-25.md`
- `docs/create-limit-delete-limit-formal-chronology-2026-03-25.md`

## 1. Purpose

This note defines the next narrow diagnostic step after the latest `sessiongap` restart-aging checks.

The immediate question is no longer whether the path can recover after reconnect.

That has already been shown.

The next question is whether the residual intermittent `create:limit` / `delete:limit` incident is driven primarily by:

- simple elapsed time on a clean CWS session;
- accumulated control-path activity on that session;
- or a rarer sequence/state condition that is not explained by time alone.

## 2. New Observation That Changes The Plan

The latest `sessiongap` restart-based baseline produced:

- immediate passive `create:limit -> delete:limit` `PASS`;
- `10m` later `PASS` on the same clean session;
- `16m` later `PASS` on the same clean session.

These checks were still on the same clean session identity:

- same `gateway_instance_id`;
- same `cws_connection_instance_id`;
- `cws_protocol_reset_total = 0`;
- `readiness = true`;
- `cws_authorized = true`.

Interpretation:

- a simple deterministic time-decay framing is now materially weaker;
- elapsed time by itself does not currently explain the failure;
- the stronger working framing is now:
  - activity accumulation,
  - rare sequence trigger,
  - or conditional control-plane degradation.

## 3. Scope Of TZ 1.4

### 3.1 Add preflight snapshot before each test action

Each controlled probe should now save a compact preflight snapshot immediately before the test action.

The preflight snapshot should capture enough information to compare:

- `PASS` pre-state;
- first `FAIL` pre-state.

### 3.2 Separate idle aging from active aging

The next diagnostic matrix should explicitly split:

1. `idle aging`
2. `active aging`

This is needed because `10m PASS` and `16m PASS` materially weaken a pure time-based explanation.

## 4. Preflight Snapshot Contents

The new preflight snapshot should be captured `1-3` seconds before the test action and saved under the run directory.

Minimum fields:

- `readiness`
- `gateway_phase`
- `gateway_instance_id`
- `auth_principal_fingerprint`
- `access_token_fingerprint`
- `access_token_last_source`
- `access_token_last_consumer`
- `cws_authorized`
- `cws_connection_instance_id`
- `cws_connection_age_sec`
- `cws_connect_seq`
- `cws_reconnect_seq`
- `cws_protocol_reset_total`
- `cws_limit_send_total`
- `cws_limit_error_total`
- `cws_pending_failed_total`
- `cws_pending_count`
- `ws_last_rx_age_sec`
- `last_orders_ts`
- `last_orders_age_sec`
- `last_positions_ts`
- `last_positions_age_sec`
- `active_subscriptions_count`
- `desired_subscriptions_count`
- `backpressure_lagged`
- `event_backpressure_lagged`
- `event_sink_degraded`
- `last_event_publish_ts`
- `last_event_publish_age_sec`
- `commands_received_total`
- `commands_accepted_total`
- `commands_rejected_total`
- `commands_duplicate_total`
- `command_expired_total`
- `command_validation_failed_total`
- `command_processed_total`
- `command_consumer_alive`
- `command_consumer_last_poll_ts_utc`
- `command_consumer_last_poll_age_sec`
- `command_consumer_last_message_id`
- `command_consumer_errors_total`
- `command_consumer_redis_timeouts_total`
- `cws_errors_total`
- `orders_ws_events_total`

Supporting artifacts:

- readiness JSON;
- `docker compose ps` for the target stack;
- short tails of:
  - `cmd.orders.*`
  - `cmd.acks.*`
  - `broker.orders.*`
  - `broker.positions.*`
- short gateway log tail;
- short runtime log tail.

## 5. Experiment Matrix

### 5.1 Branch A: idle aging

Procedure:

1. create a clean baseline, typically after a fresh gateway restart;
2. do not send control commands for the waiting interval;
3. after the wait, run one safe passive `create:limit -> delete:limit` probe.

Suggested intervals:

- `30m`
- `45m`
- `60m`

Question answered:

- does the path degrade simply because the connection gets older while mostly idle?

### 5.2 Branch B: active aging

Procedure:

1. create a clean baseline;
2. do not wait passively;
3. run safe passive `create:limit -> delete:limit` cycles every `2-3` minutes;
4. stop at first `FAIL` or after the planned cycle budget completes.

Suggested cycle budgets:

- `5`
- `10`
- `15`

Question answered:

- does repeated control activity correlate with the first `FAIL` more strongly than elapsed wall-clock time?

## 6. How To Evaluate A Connection History

For a single `cws_connection_instance_id`, the next useful unit of analysis is no longer just the failing command.

It is the history from:

- last clean `PASS`
- through all intermediate control operations
- to first `FAIL`

For that history, the review package should record:

- connection identity;
- command count on that connection;
- cycle count since restart;
- last successful place/cancel;
- last order/position event ages before the failing probe;
- whether any reconnect/reauthorize happened before the first failure;
- whether counters or lag indicators already looked abnormal before the probe.

## 7. Expected Outcome Of TZ 1.4

This phase is successful if it yields a materially stronger answer to one of these questions:

- failures correlate primarily with idle connection age;
- failures correlate primarily with accumulated control-path activity;
- failures still appear without either simple age or simple activity thresholds, implying a rarer sequence/state trigger.

## 8. Current Working Conclusion

After the latest restart-aging checks, the strongest current formulation is:

> Fresh restart creates a clean baseline, and idle aging through at least `16` minutes does not by itself reproduce the incident. The remaining failure class is therefore more likely to depend on accumulated control activity, sequence/state conditions, or another intermittent control-plane trigger rather than a deterministic break after a fixed amount of time.
