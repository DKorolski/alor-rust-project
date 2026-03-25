#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  limit_diag.sh init-run [base_dir]
  limit_diag.sh capture pre <run_dir>
  limit_diag.sh capture post <run_dir>
  limit_diag.sh place <sessiongap|hybrid> <price> [qty] [side]
  limit_diag.sh cancel <sessiongap|hybrid> <order_id>
  limit_diag.sh loop <sessiongap|hybrid> <price> [iterations] [qty] [side] [sleep_sec]
  limit_diag.sh trace <run_dir> <sessiongap|hybrid> <request_id>
  limit_diag.sh trace-order <run_dir> <sessiongap|hybrid> <order_id>

Examples:
  RUN_DIR=$(scripts/limit_diag.sh init-run)
  scripts/limit_diag.sh capture pre "$RUN_DIR"
  scripts/limit_diag.sh place sessiongap 81.71
  scripts/limit_diag.sh cancel sessiongap 2023555922907497864
  scripts/limit_diag.sh loop sessiongap 80.10 20
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
