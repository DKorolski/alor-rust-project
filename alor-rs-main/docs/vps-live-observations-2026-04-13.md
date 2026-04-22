# VPS Live Observations 2026-04-13

Date: 2026-04-13

## Evidence snapshot

Collected (UTC): `2026-04-13T20:47:22Z`  
Scope: Docker stacks `trading-sessiongap`, `trading-hybrid`, `trading-alor-usdrubf` on the live VPS (strategy-runtime logs: last 24h, tail 550 lines per container).

## Scope

Observed stacks:

- `trading-sessiongap` (compose project `trading-sessiongap`)
- `trading-hybrid` (compose project `trading-hybrid`)
- `trading-alor-usdrubf` (compose project `trading-alor-usdrubf`)

Compose status at snapshot: all three **`running(3)`**, containers **`Up` ~2 days**, healthchecks **`healthy`**.

### Image tags (observed)

| Stack | strategy-runtime | alor-gateway |
| --- | --- | --- |
| `trading-alor-usdrubf` | `ghcr.io/.../strategy-runtime:sha-4a0a266` | `ghcr.io/.../alor-gateway:sha-4a0a266` |
| `trading-hybrid` | `ghcr.io/.../strategy-runtime:vps-fbf744f` | `ghcr.io/.../alor-gateway:vps-fbf744f` |
| `trading-sessiongap` | `ghcr.io/.../strategy-runtime:vps-3cacb25` | `ghcr.io/.../alor-gateway:dev-71c09ac-actionscope2-20260402-131941` |

## Host / Redis (vs 2026-04-11 note)

Host memory at snapshot:

- total RAM **7.7 GiB**, available **~5.0 GiB**, swap **~3.9 GiB** (essentially unused).

Redis containers:

- **Restart counts:** `trading-hybrid-redis-1` **0**, `trading-sessiongap-redis-1` **0**, `trading-alor-usdrubf-redis-1` **0** (contrast with the 2026-04-11 incident, where hybrid redis had very high restarts).
- **cgroup memory (docker stats):** each redis **~648–699 MiB / 1 GiB** (**~63–68%** of limit).
- **`redis-cli INFO memory`:** `used_memory_human` roughly **634M / 651M / 681M**; `maxmemory_human` reported as **`0B`** (limit enforced by container cgroup, not necessarily reflected in this INFO field).

Kernel `dmesg` (last 50 lines sampled): **no OOM / redis kill matches** in that slice.

**Reading:** infra looks **materially healthier** than the 2026-04-11 Saturday Redis/OOM episode: more host RAM, redis restarts at zero, no OOM hits in the sampled `dmesg` tail.

## Summary

The trading day **2026-04-13** was operationally successful on **sessiongap** (clean entry/exit, flat at EOD) and **`trading-alor-usdrubf`** (multiple round-trips; **flat after EOD exit** in the observed log window).

**`trading-alor-usdrubf`** again showed the **known** `cws_error` / `protocol_reset_without_close_handshake` pattern on several **market** intents (entry/exit/EOD); retries on a later bar or after reconnect **did** converge.

**`trading-hybrid`** (IMOEXF path): the collected **tail** is mostly **overnight bootstrap + day rollover + `LiveReady`**. No strategy trade blotter lines appeared in that tail window (either quiet day for the hybrid book, or higher-volume logs earlier outside the tail). No `orphan_trade` / protective-order warnings appeared **in this excerpt**.

---

## Findings

### 1. `trading-sessiongap` — clean session path

Observed highlights (UTC):

- **06:00** session rollover / warmup (`session_gap_standalone`), then **`LiveReady` / `ALLOWED`**.
- **09:00** entry: `Flat → PendingEntry → InPosition`, **`place`** intent, **`command accepted`**, **`execution confirmed`** (sell USDRUBF).
- **20:30** exit: `InPosition → PendingExit → Flat`, **`place`** intent, **`command accepted`**, **`execution confirmed`** (buy USDRUBF).

Operational reading: **normal** for this stack in the observed window.

### 2. `trading-hybrid` — quiet in log tail, healthy bootstrap

Observed highlights:

- Overnight **`SyncingGap` / `SyncingHistory`** transitions and websocket/gateway churn as expected during bootstrap.
- **06:00** `hybrid_intraday_runtime` **day rollover** (IMOEXF features), then **`live_guard_changed` → `ALLOWED`**, `phase=LiveReady`.

Operational reading: **no anomalies in the sampled tail**; for a fuller intraday verdict, grep a wider window or export full-day logs (the 550-line tail may miss midday lines if volume is high).

### 3. `trading-alor-usdrubf` — flat after EOD; transport resets still reproducible

Observed highlights (UTC):

- **06:00** `replay_guard_cleared`, then **`ALLOWED`** (`LiveReady`).
- **Mean-reversion short** path: signal **07:08**, first **market** entry **07:09** **rejected** with `cws_error` / **`protocol_reset_without_close_handshake`** → deferred; **retry 07:10** **accepted**, fill, short open.
- **MR take-profit exit:** first **market** exit **07:51** **rejected** (same `cws_error`) → **`BLOCKED`/`ALLOWED` churn** → **retry 07:52** **accepted**, **`open_to_flat`**.
- **Day-breakout** path: **08:01** entry **accepted**, position open; later **`bo_stop1_short`** attempts (**14:51**, **15:51**): one path hit **`cws_error`** then recovered; another hit **`trading_window_closed`** / `validation failed` (deferred per policy).
- **EOD `bo_eod_exit`:** **20:31** first **market** exit **rejected** (`cws_error`); **20:32** retry **accepted**, **`open_to_flat`**; **20:33** exit inflight cleared.

Operational reading: **final outcome flat** in logs, but **the same transport-reset fragility on `create:market`** remains the dominant non-business risk.

## Runtime State Check (as of log tail ~20:47 UTC)

From the latest lines in the pulled runtime logs:

- **`trading-sessiongap`:** phase path ends **`Flat`** after **20:30 UTC** exit fill.
- **`trading-alor-usdrubf`:** **`open_to_flat`** and **`exit_intent_inflight=false`** after **~20:33 UTC** (no open position in the tail).
- **`trading-hybrid`:** **`ALLOWED` / `LiveReady`** after morning rollover; no open-position markers in the excerpt.

## Operational Reading

Classify **2026-04-13** as:

- **`trading-sessiongap`:** **normal**
- **`trading-hybrid`:** **no issue visible in tail** (confirm with full-day log if needed)
- **`trading-alor-usdrubf`:** **successful end state with repeated known CWS transport resets** on market orders; retries still converged

## Follow-Up

1. Keep watching **`protocol_reset_without_close_handshake`** on **`trading-alor-usdrubf`** market path (entry/exit/EOD).
2. Optionally pull **full-day** `trading-hybrid` logs for **2026-04-13** to confirm no hidden `orphan_trade` / protective noise outside the tail.
3. Keep monitoring **Redis memory** vs **1 GiB** cgroup caps (stable at snapshot, restarts at zero).
