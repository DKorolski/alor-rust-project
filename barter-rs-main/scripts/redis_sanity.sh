#!/usr/bin/env bash
set -euo pipefail

REDIS_URL=${REDIS_URL:-redis://127.0.0.1:6379}
PORTFOLIO=${PORTFOLIO:-demo}
STREAM_BARS=${STREAM_BARS:-"md.bars.${PORTFOLIO}.1m"}
STREAM_COMMANDS=${STREAM_COMMANDS:-"cmd.orders.${PORTFOLIO}"}
STREAM_ACKS=${STREAM_ACKS:-"cmd.acks.${PORTFOLIO}"}
STREAM_BROKER_ORDERS=${STREAM_BROKER_ORDERS:-"broker.orders.${PORTFOLIO}"}
STREAM_DLQ_COMMANDS=${STREAM_DLQ_COMMANDS:-"dlq.${STREAM_COMMANDS}"}
STREAM_HEALTH=${STREAM_HEALTH:-"events.health"}
CONSUMER_GROUP=${CONSUMER_GROUP:-"gateway"}

run() {
  echo "\n$ $*"
  "$@"
}

print_type() {
  local stream=$1
  run redis-cli -u "${REDIS_URL}" TYPE "${stream}" || true
}

print_xinfo_groups() {
  local stream=$1
  run redis-cli -u "${REDIS_URL}" XINFO GROUPS "${stream}" || true
}

print_xpending() {
  local stream=$1
  run redis-cli -u "${REDIS_URL}" XPENDING "${stream}" "${CONSUMER_GROUP}" || true
}

print_xrevrange() {
  local stream=$1
  run redis-cli -u "${REDIS_URL}" XREVRANGE "${stream}" + - COUNT 3 || true
}

cat <<EOF_HINT
Redis sanity check

Streams:
- bars: ${STREAM_BARS}
- cmd.orders: ${STREAM_COMMANDS}
- cmd.acks: ${STREAM_ACKS}
- broker.orders: ${STREAM_BROKER_ORDERS}
- dlq.cmd.orders: ${STREAM_DLQ_COMMANDS}
- events.health: ${STREAM_HEALTH}
EOF_HINT

echo "\n== TYPE checks =="
print_type "${STREAM_BARS}"
print_type "${STREAM_COMMANDS}"
print_type "${STREAM_ACKS}"
print_type "${STREAM_BROKER_ORDERS}"
print_type "${STREAM_DLQ_COMMANDS}"
print_type "${STREAM_HEALTH}"

echo "\n== XINFO GROUPS =="
print_xinfo_groups "${STREAM_COMMANDS}"
print_xinfo_groups "${STREAM_BARS}"

echo "\n== XPENDING =="
print_xpending "${STREAM_COMMANDS}"
print_xpending "${STREAM_BARS}"

echo "\n== XREVRANGE last 3 =="
print_xrevrange "${STREAM_BARS}"
print_xrevrange "${STREAM_COMMANDS}"
print_xrevrange "${STREAM_ACKS}"
print_xrevrange "${STREAM_BROKER_ORDERS}"
print_xrevrange "${STREAM_DLQ_COMMANDS}"
print_xrevrange "${STREAM_HEALTH}"
