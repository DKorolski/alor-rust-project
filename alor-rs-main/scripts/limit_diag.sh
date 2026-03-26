#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  limit_diag.sh init-run [base_dir]
  limit_diag.sh capture pre <run_dir>
  limit_diag.sh capture post <run_dir>
  limit_diag.sh preflight <run_dir> <sessiongap|hybrid> [label]
  limit_diag.sh wait-ready <sessiongap|hybrid> [timeout_sec]
  limit_diag.sh restart-gateway <sessiongap|hybrid> [timeout_sec]
  limit_diag.sh place <sessiongap|hybrid> <price> [qty] [side]
  limit_diag.sh cancel <sessiongap|hybrid> <order_id>
  limit_diag.sh loop <sessiongap|hybrid> <price> [iterations] [qty] [side] [sleep_sec]
  limit_diag.sh tz16-baseline <sessiongap|hybrid> <price> [idle_sec] [qty] [side]
  limit_diag.sh tz16-cadence <sessiongap|hybrid> <price> <interval_sec> [total_window_sec] [qty] [side]
  limit_diag.sh tz16-reconnect <sessiongap|hybrid> <price> [idle_sec] [qty] [side]
  limit_diag.sh trace <run_dir> <sessiongap|hybrid> <request_id>
  limit_diag.sh trace-order <run_dir> <sessiongap|hybrid> <order_id>

Examples:
  RUN_DIR=$(scripts/limit_diag.sh init-run)
  scripts/limit_diag.sh capture pre "$RUN_DIR"
  scripts/limit_diag.sh preflight "$RUN_DIR" sessiongap iter1.before
  scripts/limit_diag.sh wait-ready sessiongap 120
  scripts/limit_diag.sh restart-gateway sessiongap 120
  scripts/limit_diag.sh place sessiongap 81.71
  scripts/limit_diag.sh cancel sessiongap 2023555922907497864
  scripts/limit_diag.sh loop sessiongap 80.10 20
  scripts/limit_diag.sh tz16-baseline sessiongap 79.00 1800
  scripts/limit_diag.sh tz16-cadence sessiongap 79.00 600 1800
  scripts/limit_diag.sh tz16-reconnect sessiongap 79.00 1800
  scripts/limit_diag.sh capture post "$RUN_DIR"
  scripts/limit_diag.sh trace "$RUN_DIR" sessiongap 75b8a7f6-0d34-4664-beac-bc2060625f43
  scripts/limit_diag.sh trace-order "$RUN_DIR" sessiongap 2023555922907497864

Notes:
  - Intended to run on the VPS host where /opt/trading-sessiongap and /opt/trading-hybrid exist.
  - Uses grep/sed only; no rg/jq dependency.
EOF
}

require_arg() {
  if [ $# -lt "$1" ]; then
    usage >&2
    exit 1
  fi
}

stack_env() {
  case "${1:-}" in
    sessiongap)
      STACK=sessiongap
      STACK_DIR=${SESSIONGAP_DIR:-/opt/trading-sessiongap}
      STACK_PROJECT=sessiongap
      STACK_PORTFOLIO=7502MIW
      STACK_SYMBOL=USDRUBF
      STACK_SOURCE=manual-l2-p1
      STACK_PLACE_ID=manual.limit.l2.p1
      STACK_CANCEL_ID=manual.limit.l2.p1.cancel
      ;;
    hybrid)
      STACK=hybrid
      STACK_DIR=${HYBRID_DIR:-/opt/trading-hybrid}
      STACK_PROJECT=hybrid
      STACK_PORTFOLIO=7502SN6
      STACK_SYMBOL=IMOEXF
      STACK_SOURCE=manual-l2-hybrid-p1
      STACK_PLACE_ID=manual.limit.l2.hybrid.p1
      STACK_CANCEL_ID=manual.limit.l2.hybrid.p1.cancel
      ;;
    *)
      echo "unknown stack: $1" >&2
      exit 1
      ;;
  esac
  STACK_COMPOSE_FILE="${STACK_DIR}/docker-compose.yml"
  STACK_CMD_STREAM="cmd.orders.${STACK_PORTFOLIO}"
  STACK_ACK_STREAM="cmd.acks.${STACK_PORTFOLIO}"
  STACK_BROKER_ORDERS_STREAM="broker.orders.${STACK_PORTFOLIO}"
  STACK_BROKER_POSITIONS_STREAM="broker.positions.${STACK_PORTFOLIO}"
}

compose_ps() {
  local stack_name=$1
  stack_env "$stack_name"
  docker compose -p "$STACK_PROJECT" -f "$STACK_COMPOSE_FILE" ps
}

capture_readiness() {
  local stack_name=$1
  stack_env "$stack_name"
  docker exec "${STACK_PROJECT}-alor-gateway-1" curl -sS http://127.0.0.1:8081/readiness
}

capture_cws_debug() {
  local stack_name=$1
  stack_env "$stack_name"
  docker exec "${STACK_PROJECT}-alor-gateway-1" curl -sS http://127.0.0.1:8081/debug/cws
}

capture_stream_tail() {
  local stack_name=$1
  local stream_name=$2
  local count=${3:-80}
  stack_env "$stack_name"
  docker compose -p "$STACK_PROJECT" -f "$STACK_COMPOSE_FILE" exec -T redis \
    redis-cli --raw XREVRANGE "$stream_name" + - COUNT "$count"
}

capture_logs() {
  local stack_name=$1
  local service=$2
  local since_minutes=${3:-15}
  stack_env "$stack_name"
  docker compose -p "$STACK_PROJECT" -f "$STACK_COMPOSE_FILE" logs \
    --since="${since_minutes}m" --tail=500 "$service"
}

capture_phase() {
  local phase=$1
  local run_dir=$2
  local log_minutes
  local current_stack

  mkdir -p "$run_dir"
  case "$phase" in
    pre) log_minutes=${PRE_LOG_WINDOW_MIN:-5} ;;
    post) log_minutes=${POST_LOG_WINDOW_MIN:-15} ;;
    *)
      echo "unknown capture phase: $phase" >&2
      exit 1
      ;;
  esac

  for current_stack in sessiongap hybrid; do
    stack_env "$current_stack"
    capture_readiness "$current_stack" > "${run_dir}/${current_stack}.readiness.${phase}.json"
    capture_cws_debug "$current_stack" > "${run_dir}/${current_stack}.cws.debug.${phase}.json"
    compose_ps "$current_stack" > "${run_dir}/${current_stack}.ps.${phase}.txt"

    capture_stream_tail "$current_stack" "$STACK_CMD_STREAM" 80 \
      > "${run_dir}/${current_stack}.cmd.orders.${phase}.txt"
    capture_stream_tail "$current_stack" "$STACK_ACK_STREAM" 80 \
      > "${run_dir}/${current_stack}.cmd.acks.${phase}.txt"
    capture_stream_tail "$current_stack" "$STACK_BROKER_ORDERS_STREAM" 80 \
      > "${run_dir}/${current_stack}.broker.orders.${phase}.txt"
    capture_stream_tail "$current_stack" "$STACK_BROKER_POSITIONS_STREAM" 80 \
      > "${run_dir}/${current_stack}.broker.positions.${phase}.txt"

    capture_logs "$current_stack" alor-gateway "$log_minutes" \
      > "${run_dir}/${current_stack}.gateway.${phase}.log"
    capture_logs "$current_stack" strategy-runtime "$log_minutes" \
      > "${run_dir}/${current_stack}.runtime.${phase}.log"
  done
}

gen_req_id() {
  if [ -r /proc/sys/kernel/random/uuid ]; then
    cat /proc/sys/kernel/random/uuid
  else
    uuidgen
  fi
}

send_place() {
  local stack_name=$1
  local price=$2
  local qty=${3:-1.0}
  local side=${4:-buy}
  local req_id ts payload

  stack_env "$stack_name"
  req_id=$(gen_req_id)
  ts=$(date +%s)

  payload=$(cat <<EOF
{"schema_version":1,"ts_utc":$ts,"source":"$STACK_SOURCE","msg_type":"command","payload":{"request_id":"$req_id","created_ts_utc":$ts,"strategy_id":"$STACK_PLACE_ID","portfolio":"$STACK_PORTFOLIO","exchange":"MOEX","symbol":"$STACK_SYMBOL","action":{"place":{"price":$price,"qty":$qty,"side":"$side","comment":"${STACK}_$req_id"}},"intent_class":"entry","ttl_ms":600000}}
EOF
)

  docker compose -p "$STACK_PROJECT" -f "$STACK_COMPOSE_FILE" exec -T redis \
    redis-cli XADD "$STACK_CMD_STREAM" "*" payload "$payload" >/dev/null

  cat <<EOF
STACK=$STACK
REQ_ID=$req_id
TS_UTC=$ts
PRICE=$price
QTY=$qty
SIDE=$side
STREAM=$STACK_CMD_STREAM
EOF
}

send_cancel() {
  local stack_name=$1
  local order_id=$2
  local req_id ts payload

  stack_env "$stack_name"
  req_id=$(gen_req_id)
  ts=$(date +%s)

  payload=$(cat <<EOF
{"schema_version":1,"ts_utc":$ts,"source":"$STACK_SOURCE","msg_type":"command","payload":{"request_id":"$req_id","created_ts_utc":$ts,"strategy_id":"$STACK_CANCEL_ID","portfolio":"$STACK_PORTFOLIO","exchange":"MOEX","symbol":"$STACK_SYMBOL","action":{"cancel":{"order_id":$order_id}},"intent_class":"cancel_cleanup","ttl_ms":600000}}
EOF
)

  docker compose -p "$STACK_PROJECT" -f "$STACK_COMPOSE_FILE" exec -T redis \
    redis-cli XADD "$STACK_CMD_STREAM" "*" payload "$payload" >/dev/null

  cat <<EOF
STACK=$STACK
REQ_ID=$req_id
TS_UTC=$ts
ORDER_ID=$order_id
STREAM=$STACK_CMD_STREAM
EOF
}

trace_req() {
  local run_dir=$1
  local stack_name=$2
  local req_id=$3
  local phase_file

  echo "--- ${stack_name} request trace: ${req_id} ---"
  for file in \
    "${run_dir}/${stack_name}.cmd.orders.post.txt" \
    "${run_dir}/${stack_name}.cmd.acks.post.txt" \
    "${run_dir}/${stack_name}.broker.orders.post.txt" \
    "${run_dir}/${stack_name}.broker.positions.post.txt" \
    "${run_dir}/${stack_name}.gateway.post.log" \
    "${run_dir}/${stack_name}.runtime.post.log"; do
    echo "--- $(basename "$file") ---"
    if [ -f "$file" ]; then
      grep -n "$req_id" "$file" || true
    else
      echo "missing: $file"
    fi
  done

  phase_file="${run_dir}/${stack_name}.gateway.post.log"
  if [ -f "$phase_file" ]; then
    echo "--- $(basename "$phase_file") diagnostics ---"
    grep -nE "cws send|cws recv payload|matched pending|cws_transport_failure|cws_fail_pending|command ack published" \
      "$phase_file" || true
  fi
}

trace_order() {
  local run_dir=$1
  local stack_name=$2
  local order_id=$3

  echo "--- ${stack_name} order trace: ${order_id} ---"
  for file in \
    "${run_dir}/${stack_name}.cmd.orders.post.txt" \
    "${run_dir}/${stack_name}.cmd.acks.post.txt" \
    "${run_dir}/${stack_name}.broker.orders.post.txt" \
    "${run_dir}/${stack_name}.broker.positions.post.txt" \
    "${run_dir}/${stack_name}.gateway.post.log" \
    "${run_dir}/${stack_name}.runtime.post.log"; do
    echo "--- $(basename "$file") ---"
    if [ -f "$file" ]; then
      grep -n "$order_id" "$file" || true
    else
      echo "missing: $file"
    fi
  done
}

json_string_field() {
  local line=$1
  local key=$2
  printf '%s\n' "$line" | sed -n "s/.*\"$key\":\"\\([^\"]*\\)\".*/\\1/p" | head -n 1
}

json_number_field() {
  local line=$1
  local key=$2
  printf '%s\n' "$line" | sed -n "s/.*\"$key\":\\([-0-9.][0-9.]*\\).*/\\1/p" | head -n 1
}

json_bool_field() {
  local line=$1
  local key=$2
  printf '%s\n' "$line" | sed -n "s/.*\"$key\":\\(true\\|false\\).*/\\1/p" | head -n 1
}

json_array_field() {
  local line=$1
  local key=$2
  printf '%s\n' "$line" | sed -n "s/.*\"$key\":\\(\\[[^]]*\\]\\).*/\\1/p" | head -n 1
}

value_or_na() {
  local value=${1:-}
  if [ -n "$value" ]; then
    printf '%s\n' "$value"
  else
    printf 'na\n'
  fi
}

age_sec_from_ts() {
  local ts=${1:-}
  local now_ts=${2:-$(date +%s)}

  case "$ts" in
    ""|null|na) return 0 ;;
  esac

  if ! printf '%s\n' "$ts" | grep -Eq '^-?[0-9]+$'; then
    return 0
  fi

  if [ "$ts" -le 0 ]; then
    return 0
  fi

  printf '%s\n' $((now_ts - ts))
}

summary_value() {
  local summary_file=$1
  local key=$2
  sed -n "s/^${key}=//p" "$summary_file" | head -n 1
}

preflight_compact_line() {
  local summary_file=$1
  local readiness phase conn_id conn_age reconnects limit_sends limit_errors pending
  local last_orders_age last_positions_age commands_processed token_refresh access_token
  local last_rx_age last_tx_age last_control_success_age last_control_failure_age request_map_size oldest_pending_age

  readiness=$(summary_value "$summary_file" READINESS)
  phase=$(summary_value "$summary_file" GATEWAY_PHASE)
  conn_id=$(summary_value "$summary_file" CWS_CONNECTION_INSTANCE_ID)
  conn_age=$(summary_value "$summary_file" CWS_CONNECTION_AGE_SEC)
  reconnects=$(summary_value "$summary_file" CWS_RECONNECT_SEQ)
  limit_sends=$(summary_value "$summary_file" CWS_LIMIT_SEND_TOTAL)
  limit_errors=$(summary_value "$summary_file" CWS_LIMIT_ERROR_TOTAL)
  pending=$(summary_value "$summary_file" CWS_PENDING_COUNT)
  last_orders_age=$(summary_value "$summary_file" LAST_ORDERS_AGE_SEC)
  last_positions_age=$(summary_value "$summary_file" LAST_POSITIONS_AGE_SEC)
  commands_processed=$(summary_value "$summary_file" COMMAND_PROCESSED_TOTAL)
  token_refresh=$(summary_value "$summary_file" TOKEN_REFRESH_COUNT)
  access_token=$(summary_value "$summary_file" ACCESS_TOKEN_FINGERPRINT)
  last_rx_age=$(summary_value "$summary_file" CWS_LAST_RX_AGE_MS)
  last_tx_age=$(summary_value "$summary_file" CWS_LAST_TX_AGE_MS)
  last_control_success_age=$(summary_value "$summary_file" CWS_LAST_CONTROL_SUCCESS_AGE_MS)
  last_control_failure_age=$(summary_value "$summary_file" CWS_LAST_CONTROL_FAILURE_AGE_MS)
  request_map_size=$(summary_value "$summary_file" REQUEST_MAP_SIZE)
  oldest_pending_age=$(summary_value "$summary_file" CWS_OLDEST_PENDING_AGE_MS)

  printf 'readiness=%s phase=%s conn_id=%s conn_age_sec=%s reconnect_seq=%s limit_send_total=%s limit_error_total=%s pending_count=%s oldest_pending_age_ms=%s request_map_size=%s last_rx_age_ms=%s last_tx_age_ms=%s last_control_success_age_ms=%s last_control_failure_age_ms=%s last_orders_age_sec=%s last_positions_age_sec=%s command_processed_total=%s token_refresh_count=%s access_token=%s\n' \
    "$(value_or_na "$readiness")" \
    "$(value_or_na "$phase")" \
    "$(value_or_na "$conn_id")" \
    "$(value_or_na "$conn_age")" \
    "$(value_or_na "$reconnects")" \
    "$(value_or_na "$limit_sends")" \
    "$(value_or_na "$limit_errors")" \
    "$(value_or_na "$pending")" \
    "$(value_or_na "$oldest_pending_age")" \
    "$(value_or_na "$request_map_size")" \
    "$(value_or_na "$last_rx_age")" \
    "$(value_or_na "$last_tx_age")" \
    "$(value_or_na "$last_control_success_age")" \
    "$(value_or_na "$last_control_failure_age")" \
    "$(value_or_na "$last_orders_age")" \
    "$(value_or_na "$last_positions_age")" \
    "$(value_or_na "$commands_processed")" \
    "$(value_or_na "$token_refresh")" \
    "$(value_or_na "$access_token")"
}

capture_preflight() {
  local run_dir=$1
  local stack_name=$2
  local label=${3:-manual}
  local log_minutes=${PREFLIGHT_LOG_WINDOW_MIN:-3}
  local stream_count=${PREFLIGHT_STREAM_COUNT:-40}
  local readiness_json now_ts summary_file readiness_file gateway_log_file runtime_log_file
  local cws_debug_file
  local cmd_orders_file cmd_acks_file broker_orders_file broker_positions_file ps_file
  local readiness gateway_phase gateway_instance_id auth_principal_fingerprint access_token_fingerprint
  local access_token_source access_token_consumer access_token_obtained_ts access_token_last_used_ts
  local access_token_age_ms access_token_ttl_remaining_ms cws_authorized cws_connection_instance_id
  local cws_connected_ts cws_connection_age_sec cws_connect_seq cws_reconnect_seq cws_protocol_reset_total
  local cws_limit_send_total cws_limit_error_total cws_pending_failed_total cws_pending_count
  local cws_last_transport_failure_ts cws_last_limit_send_ts cws_last_limit_error_ts cws_last_successful_send_ts
  local cws_last_successful_ack_ts reconnect_count token_refresh_count ws_last_rx_age_sec
  local last_orders_ts last_orders_age_sec last_positions_ts last_positions_age_sec active_subscriptions_count
  local desired_subscriptions_count backpressure_lagged event_backpressure_lagged event_sink_degraded
  local last_event_publish_ts last_event_publish_age_sec commands_received_total commands_accepted_total
  local commands_rejected_total commands_duplicate_total command_duplicate_total command_expired_total
  local command_validation_failed_total command_processed_total command_consumer_alive
  local command_consumer_last_poll_ts command_consumer_last_poll_age_sec command_consumer_last_message_id
  local command_consumer_errors_total command_consumer_redis_timeouts_total cws_errors_total orders_ws_events_total
  local cws_last_rx_ts cws_last_rx_age_ms cws_last_tx_ts cws_last_tx_age_ms
  local cws_last_control_success_ts cws_last_control_success_age_ms
  local cws_last_control_failure_ts cws_last_control_failure_age_ms
  local cws_last_ping_ts cws_last_pong_ts cws_last_ping_pong_age_ms
  local cws_pending_guids cws_oldest_pending_age_ms request_map_size
  local cws_create_limit_send_total cws_create_limit_success_total cws_create_limit_failure_total
  local cws_delete_limit_send_total cws_delete_limit_success_total cws_delete_limit_failure_total
  local cws_replace_limit_send_total cws_replace_limit_success_total cws_replace_limit_failure_total

  mkdir -p "$run_dir"
  stack_env "$stack_name"
  now_ts=$(date +%s)

  readiness_file="${run_dir}/${STACK}.preflight.${label}.readiness.json"
  cws_debug_file="${run_dir}/${STACK}.preflight.${label}.cws.debug.json"
  summary_file="${run_dir}/${STACK}.preflight.${label}.summary.txt"
  ps_file="${run_dir}/${STACK}.preflight.${label}.ps.txt"
  cmd_orders_file="${run_dir}/${STACK}.preflight.${label}.cmd.orders.txt"
  cmd_acks_file="${run_dir}/${STACK}.preflight.${label}.cmd.acks.txt"
  broker_orders_file="${run_dir}/${STACK}.preflight.${label}.broker.orders.txt"
  broker_positions_file="${run_dir}/${STACK}.preflight.${label}.broker.positions.txt"
  gateway_log_file="${run_dir}/${STACK}.preflight.${label}.gateway.log"
  runtime_log_file="${run_dir}/${STACK}.preflight.${label}.runtime.log"

  capture_readiness "$stack_name" > "$readiness_file"
  capture_cws_debug "$stack_name" > "$cws_debug_file"
  compose_ps "$stack_name" > "$ps_file"
  capture_stream_tail "$stack_name" "$STACK_CMD_STREAM" "$stream_count" > "$cmd_orders_file"
  capture_stream_tail "$stack_name" "$STACK_ACK_STREAM" "$stream_count" > "$cmd_acks_file"
  capture_stream_tail "$stack_name" "$STACK_BROKER_ORDERS_STREAM" "$stream_count" > "$broker_orders_file"
  capture_stream_tail "$stack_name" "$STACK_BROKER_POSITIONS_STREAM" "$stream_count" > "$broker_positions_file"
  capture_logs "$stack_name" alor-gateway "$log_minutes" > "$gateway_log_file"
  capture_logs "$stack_name" strategy-runtime "$log_minutes" > "$runtime_log_file"

  readiness_json=$(cat "$readiness_file")

  readiness=$(json_bool_field "$readiness_json" "readiness")
  gateway_phase=$(json_string_field "$readiness_json" "gateway_phase")
  gateway_instance_id=$(json_string_field "$readiness_json" "gateway_instance_id")
  auth_principal_fingerprint=$(json_string_field "$readiness_json" "auth_principal_fingerprint")
  access_token_fingerprint=$(json_string_field "$readiness_json" "access_token_fingerprint")
  access_token_source=$(json_string_field "$readiness_json" "access_token_last_source")
  access_token_consumer=$(json_string_field "$readiness_json" "access_token_last_consumer")
  access_token_obtained_ts=$(json_number_field "$readiness_json" "access_token_obtained_ts_utc")
  access_token_last_used_ts=$(json_number_field "$readiness_json" "access_token_last_used_ts_utc")
  access_token_age_ms=$(json_number_field "$readiness_json" "access_token_age_ms")
  access_token_ttl_remaining_ms=$(json_number_field "$readiness_json" "access_token_ttl_remaining_ms")
  cws_authorized=$(json_bool_field "$readiness_json" "cws_authorized")
  cws_connection_instance_id=$(json_string_field "$readiness_json" "cws_connection_instance_id")
  cws_connected_ts=$(json_number_field "$readiness_json" "cws_connected_ts_utc")
  cws_connection_age_sec=$(age_sec_from_ts "$cws_connected_ts" "$now_ts")
  cws_connect_seq=$(json_number_field "$readiness_json" "cws_connect_seq")
  cws_reconnect_seq=$(json_number_field "$readiness_json" "cws_reconnect_seq")
  cws_protocol_reset_total=$(json_number_field "$readiness_json" "cws_protocol_reset_total")
  cws_limit_send_total=$(json_number_field "$readiness_json" "cws_limit_send_total")
  cws_limit_error_total=$(json_number_field "$readiness_json" "cws_limit_error_total")
  cws_pending_failed_total=$(json_number_field "$readiness_json" "cws_pending_failed_total")
  cws_pending_count=$(json_number_field "$readiness_json" "cws_pending_count")
  cws_last_transport_failure_ts=$(json_number_field "$readiness_json" "cws_last_transport_failure_ts_utc")
  cws_last_rx_ts=$(json_number_field "$readiness_json" "cws_last_rx_ts_utc")
  cws_last_rx_age_ms=$(json_number_field "$readiness_json" "cws_last_rx_age_ms")
  cws_last_tx_ts=$(json_number_field "$readiness_json" "cws_last_tx_ts_utc")
  cws_last_tx_age_ms=$(json_number_field "$readiness_json" "cws_last_tx_age_ms")
  cws_last_limit_send_ts=$(json_number_field "$readiness_json" "cws_last_limit_send_ts_utc")
  cws_last_limit_error_ts=$(json_number_field "$readiness_json" "cws_last_limit_error_ts_utc")
  cws_last_successful_send_ts=$(json_number_field "$readiness_json" "cws_last_successful_send_ts_utc")
  cws_last_successful_ack_ts=$(json_number_field "$readiness_json" "cws_last_successful_ack_ts_utc")
  cws_last_control_success_ts=$(json_number_field "$readiness_json" "cws_last_control_success_ts_utc")
  cws_last_control_success_age_ms=$(json_number_field "$readiness_json" "cws_last_control_success_age_ms")
  cws_last_control_failure_ts=$(json_number_field "$readiness_json" "cws_last_control_failure_ts_utc")
  cws_last_control_failure_age_ms=$(json_number_field "$readiness_json" "cws_last_control_failure_age_ms")
  cws_last_ping_ts=$(json_number_field "$readiness_json" "cws_last_ping_ts_utc")
  cws_last_pong_ts=$(json_number_field "$readiness_json" "cws_last_pong_ts_utc")
  cws_last_ping_pong_age_ms=$(json_number_field "$readiness_json" "cws_last_ping_pong_age_ms")
  reconnect_count=$(json_number_field "$readiness_json" "reconnect_count")
  token_refresh_count=$(json_number_field "$readiness_json" "token_refresh_count")
  ws_last_rx_age_sec=$(json_number_field "$readiness_json" "ws_last_rx_age_sec")
  last_orders_ts=$(json_number_field "$readiness_json" "last_orders_ts")
  last_orders_age_sec=$(age_sec_from_ts "$last_orders_ts" "$now_ts")
  last_positions_ts=$(json_number_field "$readiness_json" "last_positions_ts")
  last_positions_age_sec=$(age_sec_from_ts "$last_positions_ts" "$now_ts")
  active_subscriptions_count=$(json_number_field "$readiness_json" "active_subscriptions_count")
  desired_subscriptions_count=$(json_number_field "$readiness_json" "desired_subscriptions_count")
  cws_pending_guids=$(json_array_field "$readiness_json" "cws_pending_guids")
  cws_oldest_pending_age_ms=$(json_number_field "$readiness_json" "cws_oldest_pending_age_ms")
  request_map_size=$(json_number_field "$readiness_json" "request_map_size")
  backpressure_lagged=$(json_bool_field "$readiness_json" "backpressure_lagged")
  event_backpressure_lagged=$(json_bool_field "$readiness_json" "event_backpressure_lagged")
  event_sink_degraded=$(json_bool_field "$readiness_json" "event_sink_degraded")
  last_event_publish_ts=$(json_number_field "$readiness_json" "last_event_publish_ts")
  last_event_publish_age_sec=$(age_sec_from_ts "$last_event_publish_ts" "$now_ts")
  commands_received_total=$(json_number_field "$readiness_json" "commands_received_total")
  commands_accepted_total=$(json_number_field "$readiness_json" "commands_accepted_total")
  commands_rejected_total=$(json_number_field "$readiness_json" "commands_rejected_total")
  commands_duplicate_total=$(json_number_field "$readiness_json" "commands_duplicate_total")
  command_duplicate_total=$(json_number_field "$readiness_json" "command_duplicate_total")
  command_expired_total=$(json_number_field "$readiness_json" "command_expired_total")
  command_validation_failed_total=$(json_number_field "$readiness_json" "command_validation_failed_total")
  command_processed_total=$(json_number_field "$readiness_json" "command_processed_total")
  command_consumer_alive=$(json_bool_field "$readiness_json" "command_consumer_alive")
  command_consumer_last_poll_ts=$(json_number_field "$readiness_json" "command_consumer_last_poll_ts_utc")
  command_consumer_last_poll_age_sec=$(age_sec_from_ts "$command_consumer_last_poll_ts" "$now_ts")
  command_consumer_last_message_id=$(json_string_field "$readiness_json" "command_consumer_last_message_id")
  command_consumer_errors_total=$(json_number_field "$readiness_json" "command_consumer_errors_total")
  command_consumer_redis_timeouts_total=$(json_number_field "$readiness_json" "command_consumer_redis_timeouts_total")
  cws_errors_total=$(json_number_field "$readiness_json" "cws_errors_total")
  orders_ws_events_total=$(json_number_field "$readiness_json" "orders_ws_events_total")
  cws_create_limit_send_total=$(json_number_field "$readiness_json" "cws_create_limit_send_total")
  cws_create_limit_success_total=$(json_number_field "$readiness_json" "cws_create_limit_success_total")
  cws_create_limit_failure_total=$(json_number_field "$readiness_json" "cws_create_limit_failure_total")
  cws_delete_limit_send_total=$(json_number_field "$readiness_json" "cws_delete_limit_send_total")
  cws_delete_limit_success_total=$(json_number_field "$readiness_json" "cws_delete_limit_success_total")
  cws_delete_limit_failure_total=$(json_number_field "$readiness_json" "cws_delete_limit_failure_total")
  cws_replace_limit_send_total=$(json_number_field "$readiness_json" "cws_replace_limit_send_total")
  cws_replace_limit_success_total=$(json_number_field "$readiness_json" "cws_replace_limit_success_total")
  cws_replace_limit_failure_total=$(json_number_field "$readiness_json" "cws_replace_limit_failure_total")

  cat > "$summary_file" <<EOF
CAPTURED_TS_UTC=$now_ts
STACK=$STACK
LABEL=$label
READINESS=$(value_or_na "$readiness")
GATEWAY_PHASE=$(value_or_na "$gateway_phase")
GATEWAY_INSTANCE_ID=$(value_or_na "$gateway_instance_id")
AUTH_PRINCIPAL_FINGERPRINT=$(value_or_na "$auth_principal_fingerprint")
ACCESS_TOKEN_FINGERPRINT=$(value_or_na "$access_token_fingerprint")
ACCESS_TOKEN_LAST_SOURCE=$(value_or_na "$access_token_source")
ACCESS_TOKEN_LAST_CONSUMER=$(value_or_na "$access_token_consumer")
ACCESS_TOKEN_OBTAINED_TS_UTC=$(value_or_na "$access_token_obtained_ts")
ACCESS_TOKEN_LAST_USED_TS_UTC=$(value_or_na "$access_token_last_used_ts")
ACCESS_TOKEN_AGE_MS=$(value_or_na "$access_token_age_ms")
ACCESS_TOKEN_TTL_REMAINING_MS=$(value_or_na "$access_token_ttl_remaining_ms")
CWS_AUTHORIZED=$(value_or_na "$cws_authorized")
CWS_CONNECTION_INSTANCE_ID=$(value_or_na "$cws_connection_instance_id")
CWS_CONNECTED_TS_UTC=$(value_or_na "$cws_connected_ts")
CWS_CONNECTION_AGE_SEC=$(value_or_na "$cws_connection_age_sec")
CWS_CONNECT_SEQ=$(value_or_na "$cws_connect_seq")
CWS_RECONNECT_SEQ=$(value_or_na "$cws_reconnect_seq")
CWS_PROTOCOL_RESET_TOTAL=$(value_or_na "$cws_protocol_reset_total")
CWS_LAST_RX_TS_UTC=$(value_or_na "$cws_last_rx_ts")
CWS_LAST_RX_AGE_MS=$(value_or_na "$cws_last_rx_age_ms")
CWS_LAST_TX_TS_UTC=$(value_or_na "$cws_last_tx_ts")
CWS_LAST_TX_AGE_MS=$(value_or_na "$cws_last_tx_age_ms")
CWS_LIMIT_SEND_TOTAL=$(value_or_na "$cws_limit_send_total")
CWS_LIMIT_ERROR_TOTAL=$(value_or_na "$cws_limit_error_total")
CWS_PENDING_FAILED_TOTAL=$(value_or_na "$cws_pending_failed_total")
CWS_PENDING_COUNT=$(value_or_na "$cws_pending_count")
CWS_PENDING_GUIDS=$(value_or_na "$cws_pending_guids")
CWS_OLDEST_PENDING_AGE_MS=$(value_or_na "$cws_oldest_pending_age_ms")
REQUEST_MAP_SIZE=$(value_or_na "$request_map_size")
CWS_LAST_TRANSPORT_FAILURE_TS_UTC=$(value_or_na "$cws_last_transport_failure_ts")
CWS_LAST_LIMIT_SEND_TS_UTC=$(value_or_na "$cws_last_limit_send_ts")
CWS_LAST_LIMIT_ERROR_TS_UTC=$(value_or_na "$cws_last_limit_error_ts")
CWS_LAST_SUCCESSFUL_SEND_TS_UTC=$(value_or_na "$cws_last_successful_send_ts")
CWS_LAST_SUCCESSFUL_ACK_TS_UTC=$(value_or_na "$cws_last_successful_ack_ts")
CWS_LAST_CONTROL_SUCCESS_TS_UTC=$(value_or_na "$cws_last_control_success_ts")
CWS_LAST_CONTROL_SUCCESS_AGE_MS=$(value_or_na "$cws_last_control_success_age_ms")
CWS_LAST_CONTROL_FAILURE_TS_UTC=$(value_or_na "$cws_last_control_failure_ts")
CWS_LAST_CONTROL_FAILURE_AGE_MS=$(value_or_na "$cws_last_control_failure_age_ms")
CWS_LAST_PING_TS_UTC=$(value_or_na "$cws_last_ping_ts")
CWS_LAST_PONG_TS_UTC=$(value_or_na "$cws_last_pong_ts")
CWS_LAST_PING_PONG_AGE_MS=$(value_or_na "$cws_last_ping_pong_age_ms")
CWS_CREATE_LIMIT_SEND_TOTAL=$(value_or_na "$cws_create_limit_send_total")
CWS_CREATE_LIMIT_SUCCESS_TOTAL=$(value_or_na "$cws_create_limit_success_total")
CWS_CREATE_LIMIT_FAILURE_TOTAL=$(value_or_na "$cws_create_limit_failure_total")
CWS_DELETE_LIMIT_SEND_TOTAL=$(value_or_na "$cws_delete_limit_send_total")
CWS_DELETE_LIMIT_SUCCESS_TOTAL=$(value_or_na "$cws_delete_limit_success_total")
CWS_DELETE_LIMIT_FAILURE_TOTAL=$(value_or_na "$cws_delete_limit_failure_total")
CWS_REPLACE_LIMIT_SEND_TOTAL=$(value_or_na "$cws_replace_limit_send_total")
CWS_REPLACE_LIMIT_SUCCESS_TOTAL=$(value_or_na "$cws_replace_limit_success_total")
CWS_REPLACE_LIMIT_FAILURE_TOTAL=$(value_or_na "$cws_replace_limit_failure_total")
RECONNECT_COUNT=$(value_or_na "$reconnect_count")
TOKEN_REFRESH_COUNT=$(value_or_na "$token_refresh_count")
WS_LAST_RX_AGE_SEC=$(value_or_na "$ws_last_rx_age_sec")
LAST_ORDERS_TS=$(value_or_na "$last_orders_ts")
LAST_ORDERS_AGE_SEC=$(value_or_na "$last_orders_age_sec")
LAST_POSITIONS_TS=$(value_or_na "$last_positions_ts")
LAST_POSITIONS_AGE_SEC=$(value_or_na "$last_positions_age_sec")
ACTIVE_SUBSCRIPTIONS_COUNT=$(value_or_na "$active_subscriptions_count")
DESIRED_SUBSCRIPTIONS_COUNT=$(value_or_na "$desired_subscriptions_count")
BACKPRESSURE_LAGGED=$(value_or_na "$backpressure_lagged")
EVENT_BACKPRESSURE_LAGGED=$(value_or_na "$event_backpressure_lagged")
EVENT_SINK_DEGRADED=$(value_or_na "$event_sink_degraded")
LAST_EVENT_PUBLISH_TS=$(value_or_na "$last_event_publish_ts")
LAST_EVENT_PUBLISH_AGE_SEC=$(value_or_na "$last_event_publish_age_sec")
COMMANDS_RECEIVED_TOTAL=$(value_or_na "$commands_received_total")
COMMANDS_ACCEPTED_TOTAL=$(value_or_na "$commands_accepted_total")
COMMANDS_REJECTED_TOTAL=$(value_or_na "$commands_rejected_total")
COMMANDS_DUPLICATE_TOTAL=$(value_or_na "$commands_duplicate_total")
COMMAND_DUPLICATE_TOTAL=$(value_or_na "$command_duplicate_total")
COMMAND_EXPIRED_TOTAL=$(value_or_na "$command_expired_total")
COMMAND_VALIDATION_FAILED_TOTAL=$(value_or_na "$command_validation_failed_total")
COMMAND_PROCESSED_TOTAL=$(value_or_na "$command_processed_total")
COMMAND_CONSUMER_ALIVE=$(value_or_na "$command_consumer_alive")
COMMAND_CONSUMER_LAST_POLL_TS_UTC=$(value_or_na "$command_consumer_last_poll_ts")
COMMAND_CONSUMER_LAST_POLL_AGE_SEC=$(value_or_na "$command_consumer_last_poll_age_sec")
COMMAND_CONSUMER_LAST_MESSAGE_ID=$(value_or_na "$command_consumer_last_message_id")
COMMAND_CONSUMER_ERRORS_TOTAL=$(value_or_na "$command_consumer_errors_total")
COMMAND_CONSUMER_REDIS_TIMEOUTS_TOTAL=$(value_or_na "$command_consumer_redis_timeouts_total")
CWS_ERRORS_TOTAL=$(value_or_na "$cws_errors_total")
ORDERS_WS_EVENTS_TOTAL=$(value_or_na "$orders_ws_events_total")
EOF

  cat <<EOF
RUN_DIR=$run_dir
STACK=$STACK
LABEL=$label
READINESS_FILE=$readiness_file
CWS_DEBUG_FILE=$cws_debug_file
SUMMARY_FILE=$summary_file
EOF
}

latest_stream_match() {
  local stack_name=$1
  local stream_name=$2
  local pattern=$3
  local count=${4:-160}

  capture_stream_tail "$stack_name" "$stream_name" "$count" | grep "$pattern" | head -n 1 || true
}

wait_for_stream_match() {
  local stack_name=$1
  local stream_name=$2
  local pattern=$3
  local timeout_sec=${4:-20}
  local count=${5:-160}
  local i line

  for i in $(seq 1 "$timeout_sec"); do
    line=$(latest_stream_match "$stack_name" "$stream_name" "$pattern" "$count")
    if [ -n "$line" ]; then
      printf '%s\n' "$line"
      return 0
    fi
    sleep 1
  done

  return 1
}

latest_symbol_position_line() {
  local stack_name=$1
  stack_env "$stack_name"
  latest_stream_match "$stack_name" "$STACK_BROKER_POSITIONS_STREAM" "\"symbol\":\"$STACK_SYMBOL\"" 160
}

wait_gateway_ready() {
  local stack_name=$1
  local timeout_sec=${2:-120}
  local i readiness_json ready authorized phase

  for i in $(seq 1 "$timeout_sec"); do
    readiness_json=$(capture_readiness "$stack_name" 2>/dev/null || true)
    if [ -n "$readiness_json" ]; then
      ready=$(json_bool_field "$readiness_json" "readiness")
      authorized=$(json_bool_field "$readiness_json" "cws_authorized")
      phase=$(json_string_field "$readiness_json" "gateway_phase")
      if [ "$ready" = "true" ] && [ "$authorized" = "true" ] && [ "$phase" = "LiveReady" ]; then
        printf '%s\n' "$readiness_json"
        return 0
      fi
    fi
    sleep 1
  done

  return 1
}

restart_gateway() {
  local stack_name=$1
  local timeout_sec=${2:-120}

  stack_env "$stack_name"
  docker compose -p "$STACK_PROJECT" -f "$STACK_COMPOSE_FILE" restart alor-gateway >/dev/null
  wait_gateway_ready "$stack_name" "$timeout_sec"
}

tz16_assert_flat() {
  local stack_name=$1
  local summary_file=$2
  local pos_line pos_qty

  stack_env "$stack_name"
  pos_line=$(latest_symbol_position_line "$stack_name")
  pos_qty=$(json_number_field "${pos_line:-}" "qty")
  if [ -n "${pos_qty:-}" ] && [ "$pos_qty" != "0" ] && [ "$pos_qty" != "0.0" ]; then
    echo "ABORT: ${STACK_SYMBOL} position is not flat: qty=${pos_qty}" | tee -a "$summary_file" >&2
    return 1
  fi

  return 0
}

run_limit_cycle() {
  local run_dir=$1
  local stack_name=$2
  local price=$3
  local qty=${4:-1.0}
  local side=${5:-buy}
  local label=${6:-probe}
  local summary_file=${7:-}

  local place_out place_req_id place_ack_line place_status order_id order_line order_status order_filled
  local cancel_out cancel_req_id cancel_ack_line cancel_status prefix

  if [ -z "$summary_file" ]; then
    summary_file="${run_dir}/${stack_name}.${label}.summary.txt"
  fi

  capture_preflight "$run_dir" "$stack_name" "${label}_before" >/dev/null
  prefix="LABEL=${label}"
  echo "${prefix} phase=preflight $(preflight_compact_line "${run_dir}/${stack_name}.preflight.${label}_before.summary.txt")" | tee -a "$summary_file"
  echo "${prefix} phase=place_start price=$price qty=$qty side=$side" | tee -a "$summary_file"

  place_out=$(send_place "$stack_name" "$price" "$qty" "$side")
  place_req_id=$(printf '%s\n' "$place_out" | sed -n 's/^REQ_ID=//p')
  echo "$place_out" | tee -a "$summary_file"

  stack_env "$stack_name"
  place_ack_line=$(wait_for_stream_match "$stack_name" "$STACK_ACK_STREAM" "$place_req_id" 20 160 || true)
  if [ -z "$place_ack_line" ]; then
    echo "${prefix} result=fail reason=place_ack_timeout request_id=$place_req_id" | tee -a "$summary_file"
    return 1
  fi

  place_status=$(json_string_field "$place_ack_line" "status")
  order_id=$(json_number_field "$place_ack_line" "broker_order_id")
  echo "${prefix} place_status=$place_status order_id=${order_id:-}" | tee -a "$summary_file"
  echo "$place_ack_line" >> "$summary_file"

  if [ "$place_status" != "accepted" ] || [ -z "${order_id:-}" ]; then
    echo "${prefix} result=fail reason=place_not_accepted request_id=$place_req_id" | tee -a "$summary_file"
    return 1
  fi

  order_line=$(wait_for_stream_match "$stack_name" "$STACK_BROKER_ORDERS_STREAM" "$order_id" 20 200 || true)
  if [ -z "$order_line" ]; then
    echo "${prefix} result=fail reason=order_event_timeout order_id=$order_id request_id=$place_req_id" | tee -a "$summary_file"
    return 1
  fi

  order_status=$(json_string_field "$order_line" "status")
  order_filled=$(json_number_field "$order_line" "filled")
  echo "${prefix} place_order_status=$order_status filled=${order_filled:-}" | tee -a "$summary_file"
  echo "$order_line" >> "$summary_file"

  if [ "${order_filled:-0}" != "0" ] && [ "${order_filled:-0}" != "0.0" ]; then
    echo "${prefix} result=fail reason=unexpected_fill order_id=$order_id filled=$order_filled" | tee -a "$summary_file"
    return 1
  fi

  if [ "$order_status" != "working" ]; then
    echo "${prefix} result=fail reason=place_not_working order_id=$order_id status=$order_status" | tee -a "$summary_file"
    return 1
  fi

  cancel_out=$(send_cancel "$stack_name" "$order_id")
  cancel_req_id=$(printf '%s\n' "$cancel_out" | sed -n 's/^REQ_ID=//p')
  echo "$cancel_out" | tee -a "$summary_file"

  cancel_ack_line=$(wait_for_stream_match "$stack_name" "$STACK_ACK_STREAM" "$cancel_req_id" 20 200 || true)
  if [ -z "$cancel_ack_line" ]; then
    echo "${prefix} result=fail reason=cancel_ack_timeout request_id=$cancel_req_id order_id=$order_id" | tee -a "$summary_file"
    return 1
  fi

  cancel_status=$(json_string_field "$cancel_ack_line" "status")
  echo "${prefix} cancel_status=$cancel_status order_id=$order_id" | tee -a "$summary_file"
  echo "$cancel_ack_line" >> "$summary_file"

  order_line=
  order_status=
  order_filled=
  for _ in $(seq 1 20); do
    order_line=$(latest_stream_match "$stack_name" "$STACK_BROKER_ORDERS_STREAM" "$order_id" 200)
    if [ -n "$order_line" ]; then
      order_status=$(json_string_field "$order_line" "status")
      order_filled=$(json_number_field "$order_line" "filled")
      if [ "$order_status" = "canceled" ] || [ "$order_status" = "filled" ]; then
        break
      fi
    fi
    sleep 1
  done

  if [ -z "$order_line" ]; then
    echo "${prefix} result=fail reason=cancel_order_event_timeout order_id=$order_id request_id=$cancel_req_id" | tee -a "$summary_file"
    return 1
  fi

  echo "${prefix} cancel_order_status=$order_status filled=${order_filled:-}" | tee -a "$summary_file"
  echo "$order_line" >> "$summary_file"

  if [ "$cancel_status" != "accepted" ]; then
    echo "${prefix} result=fail reason=cancel_not_accepted request_id=$cancel_req_id order_id=$order_id" | tee -a "$summary_file"
    return 1
  fi

  if [ "${order_filled:-0}" != "0" ] && [ "${order_filled:-0}" != "0.0" ]; then
    echo "${prefix} result=fail reason=fill_during_cancel order_id=$order_id filled=$order_filled" | tee -a "$summary_file"
    return 1
  fi

  if [ "$order_status" != "canceled" ]; then
    echo "${prefix} result=fail reason=cancel_not_final order_id=$order_id status=$order_status request_id=$cancel_req_id" | tee -a "$summary_file"
    return 1
  fi

  echo "${prefix} result=pass request_id_place=$place_req_id request_id_cancel=$cancel_req_id order_id=$order_id" | tee -a "$summary_file"
  return 0
}

run_tz16_baseline() {
  local stack_name=$1
  local price=$2
  local idle_sec=${3:-1800}
  local qty=${4:-1.0}
  local side=${5:-buy}
  local run_dir summary_file

  run_dir="/opt/diag-captures/$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$run_dir"
  summary_file="${run_dir}/${stack_name}.tz16.idle30-baseline.summary.txt"
  : > "$summary_file"

  echo "SCENARIO=idle30-baseline stack=$stack_name idle_sec=$idle_sec price=$price qty=$qty side=$side" | tee -a "$summary_file"
  restart_gateway "$stack_name" 120 >/dev/null
  capture_preflight "$run_dir" "$stack_name" "baseline" >/dev/null
  echo "SCENARIO=idle30-baseline phase=baseline $(preflight_compact_line "${run_dir}/${stack_name}.preflight.baseline.summary.txt")" | tee -a "$summary_file"

  tz16_assert_flat "$stack_name" "$summary_file" || {
    capture_phase post "$run_dir"
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 2
  }

  echo "SCENARIO=idle30-baseline phase=idle sleep_sec=$idle_sec" | tee -a "$summary_file"
  sleep "$idle_sec"

  if ! run_limit_cycle "$run_dir" "$stack_name" "$price" "$qty" "$side" "main_probe" "$summary_file"; then
    capture_phase post "$run_dir"
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 1
  fi

  capture_phase post "$run_dir"
  echo "SCENARIO=idle30-baseline result=pass" | tee -a "$summary_file"
  echo "RUN_DIR=$run_dir"
  echo "SUMMARY_FILE=$summary_file"
}

run_tz16_cadence() {
  local stack_name=$1
  local price=$2
  local interval_sec=$3
  local total_window_sec=${4:-1800}
  local qty=${5:-1.0}
  local side=${6:-buy}
  local run_dir summary_file last_mark mark

  if [ "$interval_sec" -le 0 ] || [ "$interval_sec" -ge "$total_window_sec" ]; then
    echo "invalid cadence interval: interval_sec=$interval_sec total_window_sec=$total_window_sec" >&2
    return 2
  fi

  run_dir="/opt/diag-captures/$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$run_dir"
  summary_file="${run_dir}/${stack_name}.tz16.cadence-${interval_sec}s.summary.txt"
  : > "$summary_file"

  echo "SCENARIO=idle30-cadence stack=$stack_name interval_sec=$interval_sec total_window_sec=$total_window_sec price=$price qty=$qty side=$side" | tee -a "$summary_file"
  restart_gateway "$stack_name" 120 >/dev/null
  capture_preflight "$run_dir" "$stack_name" "baseline" >/dev/null
  echo "SCENARIO=idle30-cadence phase=baseline $(preflight_compact_line "${run_dir}/${stack_name}.preflight.baseline.summary.txt")" | tee -a "$summary_file"

  tz16_assert_flat "$stack_name" "$summary_file" || {
    capture_phase post "$run_dir"
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 2
  }

  last_mark=0
  for mark in $(seq "$interval_sec" "$interval_sec" $((total_window_sec - interval_sec))); do
    echo "SCENARIO=idle30-cadence phase=idle_until keepalive_at_sec=$mark sleep_sec=$((mark - last_mark))" | tee -a "$summary_file"
    sleep $((mark - last_mark))
    if ! run_limit_cycle "$run_dir" "$stack_name" "$price" "$qty" "$side" "keepalive${mark}s" "$summary_file"; then
      capture_phase post "$run_dir"
      echo "RUN_DIR=$run_dir"
      echo "SUMMARY_FILE=$summary_file"
      return 1
    fi
    last_mark=$mark
  done

  echo "SCENARIO=idle30-cadence phase=idle_until main_probe_at_sec=$total_window_sec sleep_sec=$((total_window_sec - last_mark))" | tee -a "$summary_file"
  sleep $((total_window_sec - last_mark))

  if ! run_limit_cycle "$run_dir" "$stack_name" "$price" "$qty" "$side" "main_probe" "$summary_file"; then
    capture_phase post "$run_dir"
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 1
  fi

  capture_phase post "$run_dir"
  echo "SCENARIO=idle30-cadence result=pass" | tee -a "$summary_file"
  echo "RUN_DIR=$run_dir"
  echo "SUMMARY_FILE=$summary_file"
}

run_tz16_reconnect() {
  local stack_name=$1
  local price=$2
  local idle_sec=${3:-1800}
  local qty=${4:-1.0}
  local side=${5:-buy}
  local run_dir summary_file

  run_dir="/opt/diag-captures/$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$run_dir"
  summary_file="${run_dir}/${stack_name}.tz16.reconnect-before-order.summary.txt"
  : > "$summary_file"

  echo "SCENARIO=idle30-reconnect-before-order stack=$stack_name idle_sec=$idle_sec price=$price qty=$qty side=$side" | tee -a "$summary_file"
  restart_gateway "$stack_name" 120 >/dev/null
  capture_preflight "$run_dir" "$stack_name" "baseline" >/dev/null
  echo "SCENARIO=idle30-reconnect-before-order phase=baseline $(preflight_compact_line "${run_dir}/${stack_name}.preflight.baseline.summary.txt")" | tee -a "$summary_file"

  tz16_assert_flat "$stack_name" "$summary_file" || {
    capture_phase post "$run_dir"
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 2
  }

  echo "SCENARIO=idle30-reconnect-before-order phase=idle sleep_sec=$idle_sec" | tee -a "$summary_file"
  sleep "$idle_sec"
  capture_preflight "$run_dir" "$stack_name" "before_reconnect" >/dev/null
  echo "SCENARIO=idle30-reconnect-before-order phase=before_reconnect $(preflight_compact_line "${run_dir}/${stack_name}.preflight.before_reconnect.summary.txt")" | tee -a "$summary_file"

  restart_gateway "$stack_name" 120 >/dev/null
  capture_preflight "$run_dir" "$stack_name" "after_reconnect" >/dev/null
  echo "SCENARIO=idle30-reconnect-before-order phase=after_reconnect $(preflight_compact_line "${run_dir}/${stack_name}.preflight.after_reconnect.summary.txt")" | tee -a "$summary_file"

  if ! run_limit_cycle "$run_dir" "$stack_name" "$price" "$qty" "$side" "main_probe" "$summary_file"; then
    capture_phase post "$run_dir"
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 1
  fi

  capture_phase post "$run_dir"
  echo "SCENARIO=idle30-reconnect-before-order result=pass" | tee -a "$summary_file"
  echo "RUN_DIR=$run_dir"
  echo "SUMMARY_FILE=$summary_file"
}

loop_limit_cycle() {
  local stack_name=$1
  local price=$2
  local iterations=${3:-20}
  local qty=${4:-1.0}
  local side=${5:-buy}
  local sleep_sec=${6:-2}

  local run_dir summary_file
  local place_out place_req_id place_ack_line place_status order_id order_line order_status order_filled
  local cancel_out cancel_req_id cancel_ack_line cancel_status
  local pos_line pos_qty iter

  run_dir="/opt/diag-captures/$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$run_dir"
  capture_phase pre "$run_dir"
  summary_file="${run_dir}/${stack_name}.loop.summary.txt"
  : > "$summary_file"

  stack_env "$stack_name"
  pos_line=$(latest_symbol_position_line "$stack_name")
  pos_qty=$(json_number_field "${pos_line:-}" "qty")
  if [ -n "${pos_qty:-}" ] && [ "$pos_qty" != "0" ] && [ "$pos_qty" != "0.0" ]; then
    echo "ABORT: ${STACK_SYMBOL} position is not flat before loop: qty=${pos_qty}" | tee -a "$summary_file" >&2
    echo "RUN_DIR=$run_dir"
    echo "SUMMARY_FILE=$summary_file"
    return 2
  fi

  for iter in $(seq 1 "$iterations"); do
    capture_preflight "$run_dir" "$stack_name" "iter${iter}.before" >/dev/null
    echo "ITERATION=$iter phase=preflight $(preflight_compact_line "${run_dir}/${stack_name}.preflight.iter${iter}.before.summary.txt")" | tee -a "$summary_file"
    echo "ITERATION=$iter phase=place_start price=$price qty=$qty side=$side" | tee -a "$summary_file"

    place_out=$(send_place "$stack_name" "$price" "$qty" "$side")
    place_req_id=$(printf '%s\n' "$place_out" | sed -n 's/^REQ_ID=//p')
    echo "$place_out" | tee -a "$summary_file"

    place_ack_line=$(wait_for_stream_match "$stack_name" "$STACK_ACK_STREAM" "$place_req_id" 20 160 || true)
    if [ -z "$place_ack_line" ]; then
      echo "ITERATION=$iter result=stop reason=place_ack_timeout request_id=$place_req_id" | tee -a "$summary_file"
      break
    fi

    place_status=$(json_string_field "$place_ack_line" "status")
    order_id=$(json_number_field "$place_ack_line" "broker_order_id")
    echo "ITERATION=$iter place_status=$place_status order_id=${order_id:-}" | tee -a "$summary_file"
    echo "$place_ack_line" >> "$summary_file"

    if [ "$place_status" != "accepted" ] || [ -z "${order_id:-}" ]; then
      echo "ITERATION=$iter result=stop reason=place_not_accepted request_id=$place_req_id" | tee -a "$summary_file"
      break
    fi

    order_line=$(wait_for_stream_match "$stack_name" "$STACK_BROKER_ORDERS_STREAM" "$order_id" 20 200 || true)
    if [ -z "$order_line" ]; then
      echo "ITERATION=$iter result=stop reason=order_event_timeout order_id=$order_id" | tee -a "$summary_file"
      break
    fi

    order_status=$(json_string_field "$order_line" "status")
    order_filled=$(json_number_field "$order_line" "filled")
    echo "ITERATION=$iter place_order_status=$order_status filled=${order_filled:-}" | tee -a "$summary_file"
    echo "$order_line" >> "$summary_file"

    if [ "${order_filled:-0}" != "0" ] && [ "${order_filled:-0}" != "0.0" ]; then
      echo "ITERATION=$iter result=stop reason=unexpected_fill order_id=$order_id filled=$order_filled" | tee -a "$summary_file"
      break
    fi

    if [ "$order_status" != "working" ]; then
      echo "ITERATION=$iter result=stop reason=place_not_working order_id=$order_id status=$order_status" | tee -a "$summary_file"
      break
    fi

    cancel_out=$(send_cancel "$stack_name" "$order_id")
    cancel_req_id=$(printf '%s\n' "$cancel_out" | sed -n 's/^REQ_ID=//p')
    echo "$cancel_out" | tee -a "$summary_file"

    cancel_ack_line=$(wait_for_stream_match "$stack_name" "$STACK_ACK_STREAM" "$cancel_req_id" 20 200 || true)
    if [ -z "$cancel_ack_line" ]; then
      echo "ITERATION=$iter result=stop reason=cancel_ack_timeout request_id=$cancel_req_id order_id=$order_id" | tee -a "$summary_file"
      break
    fi

    cancel_status=$(json_string_field "$cancel_ack_line" "status")
    echo "ITERATION=$iter cancel_status=$cancel_status order_id=$order_id" | tee -a "$summary_file"
    echo "$cancel_ack_line" >> "$summary_file"

    order_line=
    order_status=
    order_filled=
    for _ in $(seq 1 20); do
      order_line=$(latest_stream_match "$stack_name" "$STACK_BROKER_ORDERS_STREAM" "$order_id" 200)
      if [ -n "$order_line" ]; then
        order_status=$(json_string_field "$order_line" "status")
        order_filled=$(json_number_field "$order_line" "filled")
        if [ "$order_status" = "canceled" ] || [ "$order_status" = "filled" ]; then
          break
        fi
      fi
      sleep 1
    done

    if [ -z "$order_line" ]; then
      echo "ITERATION=$iter result=stop reason=cancel_order_event_timeout order_id=$order_id request_id=$cancel_req_id" | tee -a "$summary_file"
      break
    fi

    echo "ITERATION=$iter cancel_order_status=$order_status filled=${order_filled:-}" | tee -a "$summary_file"
    echo "$order_line" >> "$summary_file"

    if [ "$cancel_status" != "accepted" ]; then
      echo "ITERATION=$iter result=stop reason=cancel_not_accepted request_id=$cancel_req_id order_id=$order_id" | tee -a "$summary_file"
      break
    fi

    if [ "${order_filled:-0}" != "0" ] && [ "${order_filled:-0}" != "0.0" ]; then
      echo "ITERATION=$iter result=stop reason=fill_during_cancel order_id=$order_id filled=$order_filled" | tee -a "$summary_file"
      break
    fi

    if [ "$order_status" != "canceled" ]; then
      echo "ITERATION=$iter result=stop reason=cancel_not_final order_id=$order_id status=$order_status request_id=$cancel_req_id" | tee -a "$summary_file"
      break
    fi

    echo "ITERATION=$iter result=pass request_id_place=$place_req_id request_id_cancel=$cancel_req_id order_id=$order_id" | tee -a "$summary_file"
    sleep "$sleep_sec"
  done

  capture_phase post "$run_dir"
  echo "RUN_DIR=$run_dir"
  echo "SUMMARY_FILE=$summary_file"
}

main() {
  require_arg 1 "$@"

  case "$1" in
    init-run)
      local base_dir run_dir
      base_dir=${2:-/opt/diag-captures}
      run_dir="${base_dir}/$(date +%Y%m%d-%H%M%S)"
      mkdir -p "$run_dir"
      printf '%s\n' "$run_dir"
      ;;
    capture)
      require_arg 3 "$@"
      capture_phase "$2" "$3"
      ;;
    preflight)
      require_arg 3 "$@"
      capture_preflight "$2" "$3" "${4:-manual}"
      ;;
    wait-ready)
      require_arg 2 "$@"
      wait_gateway_ready "$2" "${3:-120}"
      ;;
    restart-gateway)
      require_arg 2 "$@"
      restart_gateway "$2" "${3:-120}"
      ;;
    place)
      require_arg 3 "$@"
      send_place "$2" "$3" "${4:-1.0}" "${5:-buy}"
      ;;
    cancel)
      require_arg 3 "$@"
      send_cancel "$2" "$3"
      ;;
    loop)
      require_arg 3 "$@"
      loop_limit_cycle "$2" "$3" "${4:-20}" "${5:-1.0}" "${6:-buy}" "${7:-2}"
      ;;
    tz16-baseline)
      require_arg 3 "$@"
      run_tz16_baseline "$2" "$3" "${4:-1800}" "${5:-1.0}" "${6:-buy}"
      ;;
    tz16-cadence)
      require_arg 4 "$@"
      run_tz16_cadence "$2" "$3" "$4" "${5:-1800}" "${6:-1.0}" "${7:-buy}"
      ;;
    tz16-reconnect)
      require_arg 3 "$@"
      run_tz16_reconnect "$2" "$3" "${4:-1800}" "${5:-1.0}" "${6:-buy}"
      ;;
    trace)
      require_arg 4 "$@"
      trace_req "$2" "$3" "$4"
      ;;
    trace-order)
      require_arg 4 "$@"
      trace_order "$2" "$3" "$4"
      ;;
    *)
      usage >&2
      exit 1
      ;;
  esac
}

main "$@"
