#!/usr/bin/env bash
set -euo pipefail

REDIS_URL=${REDIS_URL:-redis://127.0.0.1:6379}
PORTFOLIO=${PORTFOLIO:-7502T0U}

DEFAULT_STREAM_BARS="md.bars.${PORTFOLIO}.1m"
DEFAULT_STREAM_COMMANDS="cmd.orders.${PORTFOLIO}"
DEFAULT_STREAM_ACKS="cmd.acks.${PORTFOLIO}"
DEFAULT_STREAM_BROKER_ORDERS="broker.orders.${PORTFOLIO}"
DEFAULT_STREAM_DLQ_COMMANDS="dlq.${DEFAULT_STREAM_COMMANDS}"
DEFAULT_STREAM_HEALTH="events.health"
DEFAULT_CONSUMER_GROUP="gateway"
DEFAULT_CONSUMER_NAME="auto"

STREAM_FILTER=""
CONSUMER_GROUP="${DEFAULT_CONSUMER_GROUP}"
CONSUMER_NAME="${DEFAULT_CONSUMER_NAME}"
SHOW_HELP=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stream)
      STREAM_FILTER=${2:-}
      shift 2
      ;;
    --group)
      CONSUMER_GROUP=${2:-}
      shift 2
      ;;
    --consumer)
      CONSUMER_NAME=${2:-}
      shift 2
      ;;
    -h|--help)
      SHOW_HELP=true
      shift
      ;;
    *)
      echo "Unknown argument: $1" >&2
      SHOW_HELP=true
      break
      ;;
  esac
done

usage() {
  cat <<EOF_USAGE
Redis sanity check helper

Usage:
  scripts/redis_sanity.sh --stream <stream> [--group <group>] [--consumer <consumer>]
  scripts/redis_sanity.sh --help

Examples:
  scripts/redis_sanity.sh --stream events.health
  scripts/redis_sanity.sh --stream cmd.orders.${PORTFOLIO} --group gateway --consumer worker-1
  REDIS_URL=redis://127.0.0.1:6379 scripts/redis_sanity.sh --stream ${DEFAULT_STREAM_BARS}

Notes:
  - --stream is required to focus the check and avoid noisy output.
  - If consumer group does not exist, script prints a friendly hint with XINFO GROUPS.
EOF_USAGE
}

if [[ "$SHOW_HELP" == "true" || -z "$STREAM_FILTER" ]]; then
  usage
  exit 0
fi

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

print_xpending_friendly() {
  local stream=$1
  echo "\n$ redis-cli -u ${REDIS_URL} XPENDING ${stream} ${CONSUMER_GROUP}"
  local output
  output=$(redis-cli -u "${REDIS_URL}" XPENDING "${stream}" "${CONSUMER_GROUP}" 2>&1 || true)
  if [[ "$output" == *"NOGROUP"* ]]; then
    echo "consumer group is missing for stream '${stream}' and group '${CONSUMER_GROUP}'"
    echo "hint: inspect groups with: redis-cli -u ${REDIS_URL} XINFO GROUPS ${stream}"
  else
    echo "$output"
  fi
}

print_xpending_consumer_friendly() {
  local stream=$1
  echo "\n$ redis-cli -u ${REDIS_URL} XPENDING ${stream} ${CONSUMER_GROUP} - + 10 ${CONSUMER_NAME}"
  local output
  output=$(redis-cli -u "${REDIS_URL}" XPENDING "${stream}" "${CONSUMER_GROUP}" - + 10 "${CONSUMER_NAME}" 2>&1 || true)
  if [[ "$output" == *"NOGROUP"* ]]; then
    echo "consumer group is missing for stream '${stream}' and group '${CONSUMER_GROUP}'"
    echo "hint: inspect groups with: redis-cli -u ${REDIS_URL} XINFO GROUPS ${stream}"
  else
    echo "$output"
  fi
}

print_xrevrange() {
  local stream=$1
  run redis-cli -u "${REDIS_URL}" XREVRANGE "${stream}" + - COUNT 3 || true
}

cat <<EOF_HINT
Redis sanity check

Selected stream: ${STREAM_FILTER}
Group: ${CONSUMER_GROUP}
Consumer: ${CONSUMER_NAME}

Default streams reference:
- bars: ${DEFAULT_STREAM_BARS}
- cmd.orders: ${DEFAULT_STREAM_COMMANDS}
- cmd.acks: ${DEFAULT_STREAM_ACKS}
- broker.orders: ${DEFAULT_STREAM_BROKER_ORDERS}
- dlq.cmd.orders: ${DEFAULT_STREAM_DLQ_COMMANDS}
- events.health: ${DEFAULT_STREAM_HEALTH}
EOF_HINT

echo "\n== TYPE =="
print_type "${STREAM_FILTER}"

echo "\n== XINFO GROUPS =="
print_xinfo_groups "${STREAM_FILTER}"

echo "\n== XPENDING (group summary) =="
print_xpending_friendly "${STREAM_FILTER}"

echo "\n== XPENDING (consumer detail) =="
print_xpending_consumer_friendly "${STREAM_FILTER}"

echo "\n== XREVRANGE last 3 =="
print_xrevrange "${STREAM_FILTER}"
