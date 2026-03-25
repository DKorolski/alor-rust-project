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
  limit_diag.sh trace <run_dir> <sessiongap|hybrid> <request_id>
  limit_diag.sh trace-order <run_dir> <sessiongap|hybrid> <order_id>

Examples:
  RUN_DIR=$(scripts/limit_diag.sh init-run)
  scripts/limit_diag.sh capture pre "$RUN_DIR"
  scripts/limit_diag.sh place sessiongap 81.71
  scripts/limit_diag.sh cancel sessiongap 2023555922907497864
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

  mkdir -p "$run_dir"
  case "$phase" in
    pre) log_minutes=${PRE_LOG_WINDOW_MIN:-5} ;;
    post) log_minutes=${POST_LOG_WINDOW_MIN:-15} ;;
    *)
      echo "unknown capture phase: $phase" >&2
      exit 1
      ;;
  esac

  for stack_name in sessiongap hybrid; do
    stack_env "$stack_name"
    capture_readiness "$stack_name" > "${run_dir}/${stack_name}.readiness.${phase}.json"
    compose_ps "$stack_name" > "${run_dir}/${stack_name}.ps.${phase}.txt"

    capture_stream_tail "$stack_name" "$STACK_CMD_STREAM" 80 \
      > "${run_dir}/${stack_name}.cmd.orders.${phase}.txt"
    capture_stream_tail "$stack_name" "$STACK_ACK_STREAM" 80 \
      > "${run_dir}/${stack_name}.cmd.acks.${phase}.txt"
    capture_stream_tail "$stack_name" "$STACK_BROKER_ORDERS_STREAM" 80 \
      > "${run_dir}/${stack_name}.broker.orders.${phase}.txt"
    capture_stream_tail "$stack_name" "$STACK_BROKER_POSITIONS_STREAM" 80 \
      > "${run_dir}/${stack_name}.broker.positions.${phase}.txt"

    capture_logs "$stack_name" alor-gateway "$log_minutes" \
      > "${run_dir}/${stack_name}.gateway.${phase}.log"
    capture_logs "$stack_name" strategy-runtime "$log_minutes" \
      > "${run_dir}/${stack_name}.runtime.${phase}.log"
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
