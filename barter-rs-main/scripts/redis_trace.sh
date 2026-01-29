#!/usr/bin/env bash
set -euo pipefail

REDIS_URL=${REDIS_URL:-redis://127.0.0.1:6379}
PORTFOLIO=${PORTFOLIO:-7502T0U}
STREAM_BARS=${STREAM_BARS:-"md.bars.${PORTFOLIO}.1m"}
STREAM_COMMANDS=${STREAM_COMMANDS:-"cmd.orders.${PORTFOLIO}"}
STREAM_ACKS=${STREAM_ACKS:-"cmd.acks.${PORTFOLIO}"}
STREAM_BROKER_ORDERS=${STREAM_BROKER_ORDERS:-"broker.orders.${PORTFOLIO}"}
STREAM_DLQ_COMMANDS=${STREAM_DLQ_COMMANDS:-"dlq.${STREAM_COMMANDS}"}
STREAM_HEALTH=${STREAM_HEALTH:-"events.health"}

watch_stream() {
  local label=$1
  local stream=$2
  echo "--- watching ${label}: ${stream}"
  redis-cli -u "${REDIS_URL}" XREAD BLOCK 0 STREAMS "${stream}" "$" |
    sed -u "s/^/[${label}] /"
}

cleanup() {
  echo "\nStopping redis trace watchers..."
  kill 0
}
trap cleanup INT TERM

cat <<EOF_HINT
Redis trace watchers (Ctrl+C to stop)

Expected flow:
- bars -> strategy publishes cmd.orders
- cmd.orders -> gateway picks up, publishes cmd.acks
- broker.orders -> broker events for request_id / broker_order_id
- dlq.cmd.orders -> only on decode/validation failures
- events.health -> command_consumer_* fields advance
EOF_HINT

watch_stream "bars" "${STREAM_BARS}" &
watch_stream "cmd.orders" "${STREAM_COMMANDS}" &
watch_stream "cmd.acks" "${STREAM_ACKS}" &
watch_stream "broker.orders" "${STREAM_BROKER_ORDERS}" &
watch_stream "dlq.cmd.orders" "${STREAM_DLQ_COMMANDS}" &
watch_stream "events.health" "${STREAM_HEALTH}" &

wait
