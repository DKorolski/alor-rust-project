# Live Incident Note: `trading-hybrid` — MR entry, TP/SL via CWS error path, unprotected live position

Date: 2026-04-14
Stack / container: `trading-hybrid-strategy-runtime-1` (compose project `trading-hybrid`, **IMOEXF**)

Deployment line (last known from VPS snapshots; confirm after investigation):

- runtime image: `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:vps-fbf744f`
- gateway image: `ghcr.io/dkorolski/alor-rust-project/alor-gateway:vps-fbf744f`

## 1. Executive summary (field observation)

An incident is **in progress or under live observation** on **`trading-hybrid-strategy-runtime-1`**:

1. **Morning (session):** the **Mean Reversion (MR)** subsystem **armed / fired** as expected from an operational perspective (entry side of the hybrid book).
2. **Protective contour (TP / SL):** logs show **`intent_emitted`** for **`place`** and **`create_stop_limit`** (bracket legs), each followed by **`command rejected`** with **`error_code=cws_error`** and **`error_msg=cws response timeout`** (not `protocol_reset_without_close_handshake` in these lines).
3. **Current risk state:** a **live broker position remains open** while **protective exits are not established** in the strategy/gateway model (no reliable TP/SL resting on the broker side from the intended path).
4. **Working hypothesis:** the position may still **flatten when the MR intraday window expires** (MR timeout / window-end exit semantics), i.e. a **time-boxed unwind** rather than a classical protective bracket. This is **not verified** in this note; it is the operator’s expectation pending log and broker-state confirmation.

Runtime logs from the VPS (see **§7**) confirm the MR entry and **`cws_error` / `cws response timeout`** on **both** bracket legs (`place` + `create_stop_limit`), with a **second failed retry ~2 minutes** later; no successful protective ack appears in the pulled window through **`2026-04-14T06:36:32Z` UTC** after reconnect.

## 2. High-level reading

- The failure mode resembles prior **hybrid** soak history: **noisy protective-order path** and **CWS fragility** on **`create:limit`**-class traffic, while **market** entries sometimes recover via retry (see `vps-live-observations-2026-04-10.md`, `hybrid-cws-stop-cleanup-observation-2026-04-07.md`).
- If TP/SL never attach, exposure is **naked intraday risk** until **manual flatten**, **window timeout exit**, or a **later successful protective/repair** sequence.
- **Do not assume** MR timeout will fire without confirming: config, session calendar, and runtime phase for **IMOEXF** on this deployment line.

## 3. Immediate verification checklist (operator)

1. **Runtime logs** (`trading-hybrid-strategy-runtime-1`): grep around local morning for
   `intent_emitted`, `command rejected`, `cws_error`, `protocol_reset`, `orphan_trade`, `delete_stop_limit`, `repair`, `BLOCKED`, `ALLOWED`, MR owner tags.
2. **Gateway logs** (`trading-hybrid-alor-gateway-1`): matching timestamps for **`create:limit`**, **`control_path`**, recycle, ack/reject.
3. **Broker / Alor UI:** confirm **open position** (symbol, qty, side, avg), and whether **any** working protective orders exist under the portfolio.
4. **Redis runtime state** key for hybrid live portfolio (e.g. `runtime.state.*hybrid*7502SN6*` — exact key from deployed config): `last_position_qty`, `tp_order_id`, `sl_stop_order_id`, `safe_mode_*`, `current_owner`, phase.

## 4. Risk statement

Until protective orders are confirmed working or the position is flat:

- treat as **elevated operational risk** (unhedged intraday exposure relative to design intent);
- if MR window exit **does not** materialize before a defined deadline, prepare **manual flatten** per runbook and **freeze** further hybrid intents if required.

## 5. Operational conclusion (preliminary)

- **Class:** hybrid **protective / CWS** instability on **`trading-hybrid`**, triggered after **MR** activity; distinct from a clean “transport reset on market only” story if **`create:limit`** is implicated.
- **Status:** **open** — fill in chronology, request ids, and resolution (flat / manual / timeout) when available.

## 6. Recommended follow-up

1. Append a **“Resolution”** subsection here once the position is **flat** and logs are archived.
2. Cross-link to the next **`vps-live-observations-YYYY-MM-DD.md`** entry for **2026-04-14** with a short summary line.
3. If `orphan_trade` or **stop cleanup** anomalies appear, align narrative with **`hybrid-cws-stop-cleanup-observation-2026-04-07.md`** baseline.
4. ~~Pull **gateway** logs~~ — done; see **§8**.

## 7. Runtime log evidence (VPS, `trading-hybrid-strategy-runtime-1`)

Source: `docker logs` on **`2026-04-14`** (times **UTC**).

### 7.1 MR entry (filled)

- **`2026-04-14T06:20:01.428Z`** — `hybrid actions generated`: `owner=MeanReversion`, `style=Bracket`, `reason=MorningMeanReversionShort`, `stop=Some(2737.5)`, `take=Some(2715.5)` (bar local **`2026-04-14 09:19:00`** in log).
- **`intent_emitted`** `action=place` — `request_id=b2b3b8ba-e5ec-5fd5-b99e-1556030d6418`
- **`command accepted`** — `broker_order_id=2033126140834487322`
- **`execution confirmed`** — `IMOEXF` **sell** `qty=1.0`, `exec_price=2721.5`

### 7.2 Bracket protective legs — first attempt (`cws response timeout`)

Immediately after fill, runtime emitted:

- **`intent_emitted`** `action=place` — `request_id=d7e0eeb0-c1bb-5c40-845e-d28b16524cd1` (take leg)
- **`intent_emitted`** `action=create_stop_limit` — `request_id=1ebbe354-e8ff-5121-b282-fa7a18118de6` (stop leg)

Both rejected:

- **`2026-04-14T06:20:07.289Z`** — `command rejected` … `error_code=cws_error`, `error_msg=cws response timeout` (`request_id=d7e0eeb0-c1bb-5c40-845e-d28b16524cd1`)
- **`2026-04-14T06:20:11.911Z`** — same for `request_id=1ebbe354-e8ff-5121-b282-fa7a18118de6`

### 7.3 Bracket protective legs — retry (~2 minutes later)

- **`2026-04-14T06:22:25.736Z`** — `intent_emitted` `place` — `request_id=cdc17a34-1837-5b04-a88a-4c477472ad0f`
- **`2026-04-14T06:22:25.737Z`** — `intent_emitted` `create_stop_limit` — `request_id=c4cdf828-333d-582a-b0c6-7983f4473747`
- **`2026-04-14T06:22:31.188Z`** / **`06:22:36.031Z`** — both **`cws_error` / `cws response timeout`** again (same request ids).

### 7.4 Gateway reconnect churn (no new protective ack in sample)

- **`2026-04-14T06:35:56.991Z`** — `live_guard_changed` `ALLOWED → BLOCKED`, `cws_authorized=false`
- **`2026-04-14T06:36:32.845Z`** — `BLOCKED → ALLOWED` (reasons cleared)

No further `intent_emitted` / `command accepted` for protective orders appears in logs **after `06:36:32Z`** through the collection window (verify later for repair / MR timeout exit).

## 8. Gateway log evidence (VPS, `trading-hybrid-alor-gateway-1`)

Source: `docker logs` filtered from **`2026-04-14T06:00:00Z`** (times **UTC**).

### 8.1 Entry order: action-scoped CWS (healthy)

For **`request_id=b2b3b8ba-e5ec-5fd5-b99e-1556030d6418`** the gateway used **`control_cws_mode=action_scoped`**: open session → refresh token → authorize → **`create:limit`** → HTTP 200 → “order created” → close session. Latency is sub-second; **`cws_request_guid`** present on ack.

This path is **not** the same socket as the long-lived “hybrid” stream used below.

### 8.2 Bracket legs: long-lived CWS connection — `cws response timeout`

Protective commands used the **persistent** CWS client (`cws_connection_instance_id=b119cc78-58b1-4417-9429-6294e0676259`, **`connection_age_ms` ~9.68e7** in log ≈ **~27 h** on that socket at send time).

| Request | Opcode | Send (UTC) | Outcome |
| --- | --- | --- | --- |
| `d7e0eeb0-c1bb-5c40-845e-d28b16524cd1` | `create:limit` (TP buy @ 2715.5) | `06:20:01.898` | **`cws_limit_ack` status=error** at `06:20:06.899` — **`cws response timeout`** |
| `1ebbe354-e8ff-5121-b282-fa7a18118de6` | `create:stopLimit` | `06:20:06.908` | Ack published **Error** at `06:20:11.911` — timeout path |
| `cdc17a34-1837-5b04-a88a-4c477472ad0f` | `create:limit` | `06:22:25.737` | Timeout at `06:22:30.738` |
| `c4cdf828-333d-582a-b0c6-7983f4473747` | `create:stopLimit` | `06:22:30.745` | Error ack at `06:22:35.748` |

Gateway logs explicitly show **`last_successful_ack_ts_utc=None`** on that connection around these sends — **no broker ack** arrived within the client timeout window.

### 8.3 Root socket failure (~15 minutes later)

**`2026-04-14T06:35:52.878Z`** — `cws_transport_failure`:

- **`disconnect_kind=socket_error`**, **`raw_error=IO error: Connection timed out (os error 110)`**
- **`opcode_in_flight=Some("create:stopLimit")`**, **`request_id=Some("1ebbe354-e8ff-5121-b282-fa7a18118de6")`** (first stop leg)
- **`pending_count=4`** — all four outstanding opcodes (**two `create:limit` + two `create:stopLimit`**) listed as failed pending on reconnect cleanup

Then **`cws session error; reconnecting`** → **`2026-04-14T06:36:21.848Z`** authorize succeeds again on a fresh session.

### 8.4 Reading

- **Asymmetry:** MR **entry** succeeded via **fresh action-scoped** CWS; **TP/SL** went through the **stale long-lived** hybrid CWS socket, which **stopped delivering acks** (timeouts), then **TCP timed out** with **four** requests still pending.
- This supports engineering follow-up: route bracket / protective traffic through the **same resilience path as entry** (or force recycle before protective send), or shorten detection of half-open CWS for the stream connection.
