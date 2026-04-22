# VPS Live Observations 2026-04-14

Date (session under review): **2026-04-14**  
Evidence: `docker logs --since 2026-04-14T00:00:00Z` on **`155.212.170.21`** for each stack’s **strategy-runtime** and **alor-gateway**; latest **`runtime.state*`** stream tail per Redis (read at collection time **2026-04-15 ~06:36 UTC** — reflects post-session snapshot, not an end-of-04-14 freeze).

## Scope

| Compose project | strategy-runtime | alor-gateway |
| --- | --- | --- |
| `trading-sessiongap` | `trading-sessiongap-strategy-runtime-1` (`vps-3cacb25`) | `dev-71c09ac-actionscope2-20260402-131941` |
| `trading-hybrid` | `trading-hybrid-strategy-runtime-1` (`vps-fbf744f`) | `vps-fbf744f` |
| `trading-alor-usdrubf` | `trading-alor-usdrubf-strategy-runtime-1` (`sha-4a0a266`) | `sha-4a0a266` |

Redis stream keys (persisted state):

- `runtime.state.session_gap_standalone.live.7502MIW` → `trading-sessiongap-redis-1`
- `runtime.state.hybrid_intraday.live.action_scoped.imoexf.7502SN6` → `trading-hybrid-redis-1`
- `runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U` → `trading-alor-usdrubf-redis-1`

## Executive summary

| Stack | Trades (from logs) | Main anomalies |
| --- | --- | --- |
| **sessiongap** | USDRUBF: entry **09:00 UTC**, exit **~13:57 UTC**; phases **Flat → InPosition → Flat** | Overnight **ws_hub** resets; mid-day **~10:38 UTC** reconnect churn; **no** runtime `WARN` on `command rejected` in filtered slice |
| **hybrid** | IMOEXF: MR short **06:20 UTC** fill `2033126140834487322`; bracket TP/SL **`cws response timeout`**; **MeanRevTimeCutoff** exit **08:55 UTC** fill `2033126140834559348` | **Major:** protective legs failed on long-lived CWS; TCP **timeout ~06:35 UTC** (see [`live-incident-note-2026-04-14-trading-hybrid-mr-tp-sl-cws.md`](./live-incident-note-2026-04-14-trading-hybrid-mr-tp-sl-cws.md)); later hub/CWS churn **~10:38** / **~23:40 UTC** |
| **alor-usdrubf** | USDRUBF: **~15 min** (**08:01–08:16 UTC**) BO retries failed with **`cws response timeout`** (**no** `intent_dropped_by_guard`); **08:17 UTC** first **guard drop** + **`strategy_state_transition_reverted`**; **08:17:27** **`ALLOWED`**; **08:18 UTC** first fill `2023555991626389852`; **EOD** **20:32 UTC** after **`protocol_reset`** on first try | Timeouts vs **guard** are different paths; success follows **revert + live guard clear** |

**Cross-stack:** all three show **overnight / rollover** `live_guard_changed` (**SyncingGap → History → ALLOWED ~06:00 UTC**). **Market-data / hub:** `ws_hub` **connection reset without close handshake** appears on **sessiongap** and **hybrid** gateways (and **alor-usdrubf** at times); **alor-usdrubf** also logs **TLS unexpected_eof** and **CWS protocol_reset** / **eof** on **2026-04-14**.

---

## 1. `trading-sessiongap` (`session_gap_standalone`, USDRUBF)

### Trades (runtime)

- **2026-04-14T09:00:00Z** — `Flat → PendingEntry → InPosition`; `intent_emitted` `place` `request_id=f74df522-2cf6-5441-bf4b-0269d6d4a043`; **`execution confirmed`** sell `order_id=2023555991626408307`, **exec_price=75.73**.
- **2026-04-14T13:57:01Z** — `InPosition → PendingExit → Flat`; exit `place` `request_id=d610b423-f750-57f9-b02d-c109680938d0`; **`execution confirmed`** path (full line in raw logs).

### Anomalies / infra

- **03:30–03:34 UTC:** bootstrap `live_guard_changed` (**SyncingGap / SyncingHistory**).
- **Gateway `ws_hub`:** `Connection reset without closing handshake` (**03:30**, **03:33**).
- **~10:38 UTC:** `ALLOWED → BLOCKED` (**ws_connected=false**), then **SyncingGap**, back **ALLOWED ~10:40 UTC** (shared pattern with other stacks).
- **~23:50 UTC:** overnight-style disconnect churn.

### Redis (latest tail at collection)

- Key: `runtime.state.session_gap_standalone.live.7502MIW`
- Snapshot shows **`phase":"Flat"`**, `traded_session` / session fields for **2026-04-15** calendar in payload (new session day); **`last_trade_ts`** references prior session activity — use **runtime logs** for **2026-04-14** ground truth.

---

## 2. `trading-hybrid` (`hybrid_intraday`, IMOEXF)

### Trades (runtime)

- **06:20:01 UTC** — `MorningMeanReversionShort` bracket; **entry fill** `2033126140834487322` @ **2721.5** (sell 1).
- **06:20–06:22 UTC** — TP (`place` **create:limit**) + SL (`create_stop_limit`): **four** `command rejected` **`cws_error` / `cws response timeout`** (`d7e0eeb0…`, `1ebbe354…`, retry `cdc17a34…`, `c4cdf828…`).
- **08:55:05 UTC** — `hybrid actions generated`: **`submit_exit owner=MeanReversion reason=MeanRevTimeCutoff`** (local bar **`11:54:00`**); **`intent_emitted`** `request_id=cf8fe50e-7dc2-5c2e-961d-5372643f9d0d`; **`execution confirmed`** buy **`order_id=2033126140834559348`**, **exec_price=2707.0** — **MR window-end exit succeeded** (flat after broker fill; confirm phase lines in full log if needed).

### Anomalies / infra

- **06:35:52 UTC (gateway):** **TCP `Connection timed out (os error 110)`** on long-lived CWS; **`pending_count=4`** bracket ops — see incident note **§8**.
- **~10:38 / ~23:40 UTC:** `live_guard_changed` + gateway **ws_hub** / CWS churn (aligned with other stacks).

### Redis (latest tail at collection)

- Key: `runtime.state.hybrid_intraday.live.action_scoped.imoexf.7502SN6`
- **`last_position_qty":0.0`**, **`current_owner":null`**, **`tp_order_id`/`sl_stop_order_id":null** — consistent with **flat** after **2026-04-14** MR cycle + time cut exit.

### Incident pointer

- Full gateway/runtime correlation: [`live-incident-note-2026-04-14-trading-hybrid-mr-tp-sl-cws.md`](./live-incident-note-2026-04-14-trading-hybrid-mr-tp-sl-cws.md).

---

## 3. `trading-alor-usdrubf` (`alor_usdrubf_hybrid_v1`, USDRUBF)

### Morning BO entry: timeouts vs guard, then first success (**08:01–08:19 UTC**)

Roughly **15 minutes** (**08:01–08:16 UTC**), **`day_breakout_waitfix`** kept emitting **`market`** entry intents on each bar; the observable failure mode was **`command rejected`** / **`cws_error` / `cws response timeout`** on **`command_acknowledged`** (broker path never acked in time). In that stretch there was **no** **`intent_dropped_by_guard`** — i.e. runtime still tried to push intents into the gateway contour while live guard allowed trading, but **CWS/gateway did not complete** the round-trip.

Immediately **before** the first **successful** placement, a **different** path appeared:

1. **`2026-04-14T08:17:01.207237Z`** — **`intent_dropped_by_guard`**: `action="market"`, `class=Entry`, `reasons=["gateway_ready=false","cws_authorized=false"]` — runtime **refused to emit** because **live guard** already saw the control plane as not ready (distinct from “emitted then timeout on ack”).
2. **Same second** — **`strategy_state_transition_reverted`**: from **`lifecycle_stage=live_entry_intent_emitted`** (bar `last_bar_ts` / `last_processed_bar_ts` **1776154560**) **to** **`entry_reject_deferred_retry`** with **`last_processed_bar_ts` rolled back to 1776154500**, **`entry_intent_inflight` true → false**, **`reason="intent_dropped_by_guard"`** (full `AlorUsdrubfHybrid` snapshots in raw logs).
3. **`2026-04-14T08:17:27.654348Z`** — **`live_guard_changed`**: **`BLOCKED` → `ALLOWED`**, `reasons_before=["cws_authorized=false","gateway_ready=false"]` cleared — **gateway/CWS authorization path healthy enough to trade** again.
4. **`2026-04-14T08:18:01.362Z`** — new **`live entry intent emitted`** (bar **`1776154620`**), **`intent_emitted`** **`market`** **`request_id=0458396a-18e5-5428-8897-8b02f61e00fe`**.
5. **`08:18:01.469Z`** — **`command acknowledged`**, **`Accepted`**, **`broker_order_id=2023555991626389852`**.
6. **`08:18:01.476Z`** — **`execution confirmed`**, **exec_price=75.85**; **`initial_broker_sync_open`**, **qty=-1.0**.
7. **`2026-04-14T08:19:00.603375Z`** — **`entry_intent_inflight=false`**, **`lifecycle_stage=live`**.

**Reading:** the **first fill** landed **after** the **guard-driven drop + state revert**, and **after** **`LiveReady` / `ALLOWED`** was restored — not in the middle of the pure **timeout-on-ack** sequence. The morning story is **two layers**: (A) **many** failed **acks** while intents were still being emitted; (B) one **bar** where the runtime **skipped emit** because **`gateway_ready` / `cws_authorized`** were false, **reverted** deferred-entry bookkeeping, then **re-allowed** and **succeeded** on the next attempt.

### Trades (runtime) — rest of day

- **20:31:17 UTC** — EOD **`bo_eod_exit`** first attempt **`protocol_reset_without_close_handshake`** (`request_id=4f3ffe8f-…`).
- **20:32:48 UTC** — retry **`execution confirmed`** buy **`order_id=2023555991626629063`**, **`open_to_flat`**.

### Anomalies / infra

- **08:16:59–08:17:27 UTC:** `live_guard_changed` **BLOCKED** (**cws_authorized/gateway_ready**) — aligns with guard reasons above; **clear to `ALLOWED`** at **08:17:27** precedes successful **08:18** market.
- **10:38 UTC:** reconnect / **protocol_reset** on gateway (**stack `alor-usdrubf`**).
- **20:31 UTC:** **`create:market`** in flight during **protocol_reset** (gateway **§** pending fail).
- **23:40 / 23:50 UTC:** **eof** / **unexpected_eof** on CWS and **ws_hub**.

### Redis (latest tail at collection)

- Key: `runtime.state.alor_usdrubf_hybrid_v1.live.usdrubf.7502T0U`
- **`hybrid_state":"flat"`**, **`open_position_qty":0.0`**, **`entry_intent_inflight":false** — consistent with **EOD flat** after **2026-04-14**.

---

## 4. Gateway “probes” (WARN / transport, 2026-04-14)

Abbreviated pattern (all three gateways):

- **`ws_hub`:** `WebSocket protocol error: Connection reset without closing handshake` (overnight and intraday).
- **`alor-usdrubf`:** **`protocol_reset_without_close_handshake`**, **`eof` / TLS unexpected_eof** on CWS; **`create:market`** pending failed during reset at **20:31 UTC**.
- **`hybrid`:** **`cws response timeout`** on **`create:limit` / `create:stopLimit`**; **socket_error 110** at **06:35 UTC** (see incident note).

---

## 5. Operational reading

- **sessiongap:** **Normal** intraday outcome on **2026-04-14** (one round-trip, clean fills in log excerpt).
- **hybrid:** **MR time-cut exit worked** after bracket CWS failure — confirms **MeanRevTimeCutoff** path can flatten when protective CWS path is unhealthy (incident still documents bracket risk).
- **alor-usdrubf:** **Noisy morning**: **many** **`cws response timeout`** acks **without** **`intent_dropped_by_guard`**; first **fill** only after **`intent_dropped_by_guard`** + **`strategy_state_transition_reverted`** and **`ALLOWED`** at **~08:17:27 UTC**, then **08:18 UTC** fill; **EOD** succeeded after one **protocol_reset** on first exit try.

## 6. Follow-up

1. Keep **hybrid** incident note updated if post-mortem adds gateway tuning (action-scoped vs stream for bracket).
2. For **alor-usdrubf**, consider correlating **08:01–08:16** timeout burst with **gateway** CPU/network or **Alor** incident window; separately analyze **guard drop + revert** vs **timeout** semantics for operator clarity.
3. Optional: export **full** `docker logs` for **2026-04-14** to cold storage; Redis **XREVRANGE** at **2026-04-14T23:59 UTC** is not available retroactively unless snapshots were taken.
