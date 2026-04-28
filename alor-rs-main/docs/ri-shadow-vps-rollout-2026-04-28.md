# RI Author41/42 Shadow VPS Rollout - 2026-04-28

## Scope

This note records the first VPS rollout of the RI Author41/42 shadow-only contour.

The deployment is intentionally isolated from the three live trading stacks:

- no strategy runtime that emits broker commands is started;
- the stack has its own Redis instance;
- the model journal is written to a local report file only;
- the gateway subscribes to market data and uses an isolated command stream inside the RI shadow Redis.

## VPS stack

- Host: `155.212.170.21`
- Compose project: `trading-ri-shadow`
- Working directory: `/opt/trading-ri-shadow`
- Runtime image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:manual-8eeef73-ri-shadow`
- Gateway image: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:manual-5430299-protplace-20260428`

Services:

- `trading-ri-shadow-redis-1`
- `trading-ri-shadow-alor-gateway-1`
- `trading-ri-shadow-ri-shadow-runner-1`

## Market-data contract

The model layer remains the RI continuous shadow profile, but the live Alor subscription requires the current exchange contract ticker.

- Initial attempt: `RI`
- Alor response: `Instrument with symbol RI was not found in exchange MOEX`
- Active subscription ticker: `RIM6`
- Redis bar stream: `md.bars.RI.10m`
- Runner filter symbol: `RIM6`
- Model profile emitted in journal: `ri_author41_42_primary_combo_cost2`

This keeps the analytics-facing stream name stable while making the transport subscription explicit.

## Startup verification

At rollout time the stack reached the expected shadow-only state:

- gateway connected and subscribed to `RIM6` bars;
- gateway backfilled `273` historical bars;
- Redis stream `md.bars.RI.10m` had `274` entries;
- consumer group `moex-author41-42-shadow-ri` had `lag=0` and `pending=0`;
- shadow journal file existed at `/opt/trading-ri-shadow/volumes/reports/moex_author41_42_shadow_ri.jsonl`;
- journal had started producing Author41/42 shadow records.

## Resource check

Post-start resource read:

- VPS RAM: `7.7Gi` total, `4.8Gi` available;
- swap: `53Mi / 3.9Gi`;
- disk `/`: `42G / 79G`, `56%` used;
- RI Redis: about `8MiB / 768MiB`;
- RI gateway and runner: negligible memory footprint at startup.

Existing live Redis memory at the same read:

- `trading-sessiongap-redis-1`: about `579MiB / 1GiB`;
- `trading-alor-usdrubf-redis-1`: about `646MiB / 1GiB`;
- `trading-hybrid-redis-1`: about `605MiB / 1GiB`.

## Operational notes

- This is not a live trading rollout.
- No RI order-emitting runtime is enabled.
- The gateway still has CWS authorization because it is the existing transport gateway shape, but it reads commands only from the isolated RI Redis.
- If this contour is promoted beyond shadow observation, the next engineering hardening item is a feed-only gateway mode or explicit command-consumer disable switch.

## Follow-up checks

- Confirm that `RIM6` remains the intended active RI contract for the observation period.
- Check the journal after the next full session for duplicate/intraday rolling records versus finalized records.
- If journal volume grows unexpectedly, add a finalized-only output mode or retention policy for `/opt/trading-ri-shadow/volumes/reports`.
