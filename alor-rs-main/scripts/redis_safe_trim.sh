#!/usr/bin/env bash
set -euo pipefail

MODE="dry-run"
if [[ "${1:-}" == "--apply" ]]; then
  MODE="apply"
elif [[ "${1:-}" == "--dry-run" || "${1:-}" == "" ]]; then
  MODE="dry-run"
else
  echo "usage: $0 [--dry-run|--apply]" >&2
  exit 2
fi

# Safe stream retention for live soak. This script never touches runtime.state
# or runtime.riskgate.* keys. It only trims explicit stream prefixes.
declare -A LIMITS=(
  [events.health.ri_author41_42.*]=2000
  [events.health]=10000
  [broker.snapshots.*]=10000
  [broker.positions.*]=5000
  [broker.orders.*]=5000
  [broker.trades.*]=5000
  [cmd.orders.*]=5000
  [cmd.acks.*]=5000
  [md.bars.*]=3000
)

CONTAINERS=(
  trading-sessiongap-redis-1
  trading-hybrid-redis-1
  trading-alor-usdrubf-redis-1
  trading-ri-author41-42-7502miw-redis-1
  trading-ri-shadow-redis-1
)

now() { date -Is; }

trim_key() {
  local container="$1"
  local key="$2"
  local limit="$3"
  local type len after

  type=$(docker exec "$container" redis-cli TYPE "$key" | tr -d "\r")
  if [[ "$type" != "stream" ]]; then
    return 0
  fi

  len=$(docker exec "$container" redis-cli XLEN "$key" | tr -d "\r")
  if (( len <= limit )); then
    printf "%s container=%s key=%s len=%s limit=%s action=skip\n" "$(now)" "$container" "$key" "$len" "$limit"
    return 0
  fi

  if [[ "$MODE" == "apply" ]]; then
    docker exec "$container" redis-cli XTRIM "$key" MAXLEN "=" "$limit" >/dev/null
    after=$(docker exec "$container" redis-cli XLEN "$key" | tr -d "\r")
    printf "%s container=%s key=%s len_before=%s len_after=%s limit=%s action=trimmed\n" "$(now)" "$container" "$key" "$len" "$after" "$limit"
  else
    printf "%s container=%s key=%s len=%s limit=%s action=would_trim\n" "$(now)" "$container" "$key" "$len" "$limit"
  fi
}

echo "$(now) redis_safe_trim mode=$MODE"
for container in "${CONTAINERS[@]}"; do
  if ! docker ps --format "{{.Names}}" | grep -qx "$container"; then
    echo "$(now) container=$container action=missing_skip"
    continue
  fi

  docker stats --no-stream --format "$(now) container={{.Name}} mem={{.MemUsage}}" "$container"
  for pattern in "${!LIMITS[@]}"; do
    while IFS= read -r key; do
      [[ -z "$key" ]] && continue
      if [[ "$key" == runtime.riskgate.* || "$key" == runtime.state.* ]]; then
        printf "%s container=%s key=%s action=protected_skip\n" "$(now)" "$container" "$key"
        continue
      fi
      trim_key "$container" "$key" "${LIMITS[$pattern]}"
    done < <(docker exec "$container" redis-cli --scan --pattern "$pattern" | sort)
  done
  docker stats --no-stream --format "$(now) container={{.Name}} mem={{.MemUsage}}" "$container"
done
