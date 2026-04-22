# VPS Live Observations 2026-04-15

Date (session under review): **2026-04-15**  
Evidence: `docker logs --since 2026-04-15T00:00:00Z --until 2026-04-16T00:00:00Z` on **`155.212.170.21`** for each stack’s **strategy-runtime** and **alor-gateway** (SSH key `~/.ssh/id_rsa_lightnode`). No separate Redis freeze for end-of-day; use logs as primary.

## Scope

| Compose project | strategy-runtime | alor-gateway |
| --- | --- | --- |
| `trading-sessiongap` | `trading-sessiongap-strategy-runtime-1` | `trading-sessiongap-alor-gateway-1` |
| `trading-hybrid` | `trading-hybrid-strategy-runtime-1` | `trading-hybrid-alor-gateway-1` |
| `trading-alor-usdrubf` | `trading-alor-usdrubf-strategy-runtime-1` | `trading-alor-usdrubf-alor-gateway-1` |

## Executive summary

| Stack | Intents / trades (UTC day) | Errors / anomalies |
| --- | --- | --- |
| **sessiongap** | **12:00** entry buy **`2023555995921588123`** @ **75.93**; **20:30** scheduled exit **`6464095f-e24f-5f05-a054-7d8637cd6e71`** → **Flat** | **`orphan_trade` WARN** at **20:30:08** on exit path (`trade_id=2023555995921328019`, `order_id=2023555995921740415`); overnight **ws_hub** resets **~03:31 / 03:33 / 03:51**; **~23:40** disconnect churn |
| **hybrid** | **Several** MR + one **BO long** + **EOD BO exit**; see §2 | Bracket legs again hit **`protocol_reset_without_close_handshake`** / **`cws response timeout`**; **MeanRevTimeCutoff** exits **succeeded**; gateway **~15 WARN** lines (CWS + **eof** / **unexpected_eof** overnight) |
| **alor-usdrubf** | **BO short** open **08:02** after **08:01** reject; **bo_stop1** exit with retries **~08:51–10:14**; **BO long** **10:44** after **10:43** reject; **bo_stop1_long** flat **10:51** | **`protocol_reset_without_close_handshake`** on multiple **`create:market`** (runtime + gateway `cws_transport_failure`); **no EOD market** in **19:00–24:00 UTC** slice (already flat after morning cycle) |

---

## 1. `trading-sessiongap` (`session_gap_standalone`, USDRUBF)

### Intents / fills / phases

- **03:31–03:51 UTC** — `live_guard_changed` during **SyncingGap / SyncingHistory** (overnight).
- **06:00:01 UTC** — **`session rollover summary`**: `new_session_date=2026-04-15`, `gap_minutes=550`, prior session refs.
- **06:00:09 UTC** — **`live_guard_changed` → `ALLOWED`**, `phase=LiveReady`.
- **12:00:03 UTC** — **`Flat → PendingEntry → InPosition`**; **`intent_emitted`** `place` **`request_id=1f07034f-a80a-56f3-8075-40a8b6f5807e`**; **`execution confirmed`** **`order_id=2023555995921588123`**, **buy**, **exec_price=75.93**.
- **20:30:07 UTC** — **`InPosition → PendingExit`**; **`intent_emitted`** `place` **`request_id=6464095f-e24f-5f05-a054-7d8637cd6e71`**.
- **20:30:08 UTC** — **`WARN orphan_trade`**: `trade_id=2023555995921328019`, `order_id=2023555995921740415`, **sell** **75.75** (event ordering / attribution noise on exit contour — outcome still **Flat** in the same second).
- **20:30:08 UTC** — **`PendingExit → Flat`** (`mts_utc=1776284940`).
- **23:40 UTC** — `live_guard_changed` **ALLOWED → BLOCKED** (**ws_connected=false**), then **SyncingGap**.

### Gateway (WARN count **4** on 2026-04-15)

- **`ws_hub`**: **`Connection reset without closing handshake`** (**03:31**, **03:33**, **03:51**).
- **23:40** — **`unexpected_eof` / TLS close_notify** on hub.

### Reading

- **Operational outcome:** one full **round-trip**, **flat** at end of exit window.
- **Anomaly:** **`orphan_trade`** on exit — worth correlating with gateway ack / trade stream ordering (same theme as earlier soak notes for **sessiongap**).

---

## 2. `trading-hybrid` (`hybrid_intraday`, IMOEXF)

### MR cycle A (~06:52 UTC local morning)

- **06:52:02 UTC** — `MorningMeanReversionShort` bracket; **`intent_emitted`** `place` **`fe0e9fff-280f-5bda-b7f0-3ca4e65e5329`**; **`execution confirmed`** sell **`2033126145129463235`**, **exec_price=2717.5**.
- **06:52:05 UTC** — TP **`place`** **`81e0f5fa-…`**, SL **`create_stop_limit`** **`80a7ad68-…`**.
- **06:52:05–06:52:10 UTC** — **`command rejected`**: TP **`protocol_reset_without_close_handshake`**; SL **`cws response timeout`**; **`live_guard_changed` ALLOWED ↔ BLOCKED** (cws/gateway).
- **06:55:00 UTC** — repair **`intent_emitted`** `place` **`31a56770-…`** (take leg retry).
- **07:27:30 UTC** — **`execution confirmed`** **buy** **`2033126145129464180`** @ **2713.0** (TP / flatten path); **`delete_stop_limit`** **`716bfbb5-…`**.

### MR cycle B (~08:38 UTC)

- **08:38:01 UTC** — second **MR short** entry fill **`2033126145129513700`** @ **2717.0**; bracket **`6f809562-…`** / **`ed430c43-…`**.
- **08:38:02–06 UTC** — TP **`protocol_reset_without_close_handshake`**; SL **`cws response timeout`**; brief **BLOCKED**.
- **08:40:05 UTC** — another **`intent_emitted`** `place` **`e06e5dad-…`** (protective retry in log window).

### MR time cuts + back-to-back MR (~08:55–08:57 UTC, local **11:54–11:56**)

- **08:55:09 UTC** — **`submit_exit`** **`MeanRevTimeCutoff`**; **`intent_emitted`** **`5e0dec4c-…`**; **`execution confirmed`** **buy** **`2033126145129520967`** @ **2717.5** (flat from first position of this sub-sequence).
- **08:55:12 UTC** — **`cancel`** + **`delete_stop_limit`** cleanup.
- **08:56:08 UTC** — **another** `MorningMeanReversionShort` **immediately** (next bar still inside MR window): entry **`2033126145129521130`** @ **2717.0**.
- **08:57:17 UTC** — second **`MeanRevTimeCutoff`** exit **`2033126145129521423`** @ **2716.0**; **`cancel`** / **`delete_stop_limit`**.

### Intraday breakout + EOD

- **11:11:02 UTC** — **`BreakoutLong`** **`market`-style** `place` **`fc4fe26d-…`**; **`execution confirmed`** **buy** **`2033126145129588636`**, **exec_price=2736.5**.
- **20:31:17 UTC** — **`BreakoutEodExit`**; **`intent_emitted`** **`fcfe5a27-…`**; **`execution confirmed`** **sell** **`2033126145129759424`**, **exec_price=2745.0**.

### Gateway (WARN count **15** on 2026-04-15)

- **03:26** — **`protocol_reset_without_close_handshake`** on **hybrid** CWS (idle).
- **06:52 / 08:38** — transport failure **`create:limit`** in flight (TP leg), pending fail + reconnect.
- **23:40 / 23:50** — **`eof` / unexpected_eof** on CWS + **ws_hub**.

### Reading

- **Bracket fragility** repeats (**TP reset**, **SL timeout**), but **MR time-cut** and **repair TP** still produced **flat** between MR attempts.
- **08:56** MR re-entry right after **08:55** exit is **expected** if bar clock still satisfies MR window rules — note for operators reviewing “double MR” in logs.

---

## 3. `trading-alor-usdrubf` (`alor_usdrubf_hybrid_v1`, USDRUBF)

### BO short leg (morning)

- **08:01:03 UTC** — **`market`** entry **`c3c12529-…`** → **`command rejected`** **`protocol_reset_without_close_handshake`** (deferred next bar); **`live_guard_changed`** **BLOCKED** ~**08:01:08–08:01:34**, **ALLOWED** **08:01:39**.
- **08:02:02 UTC** — retry **`market`** **`03a83d67-…`** → **`execution confirmed`** **`order_id=2023555995921379427`**, **sell** **exec_price=75.03**; **`initial_broker_sync_open`** **short**.
- **08:51:00 UTC** — **`bo_stop1_short`** exit **`77f1e0ca-…`** → **`command rejected`** **`protocol_reset_without_close_handshake`**.
- **09:51:18 UTC** — another **`command rejected`** **`protocol_reset`** on **`72d433f2-…`** (continuation of exit/repair path in logs).
- **10:14:02 UTC** — **`execution confirmed`** **buy** **`2023555995921467238`**, **exec_price=75.4**; **`open_to_flat`**.

### BO long leg (late morning)

- **10:42–10:43 UTC** — **`bo_long_signal`** → **`market`** **`43371e83-…`** → **`command rejected`** **`protocol_reset_without_close_handshake`**.
- **10:44:02 UTC** — retry **`9642fde4-…`** → **`execution confirmed`** **`2023555995921496950`**, **buy** **75.49**; **`flat_to_open`**.
- **10:51:01 UTC** — **`bo_stop1_long`** exit **`9833d515-…`** → **`execution confirmed`** **`2023555995921501252`**, **sell** **75.56**; **`open_to_flat`**.
  - **Semantics (code):** `bo_stop1_long` is **not** `bo_eod_exit` and **not** a native exchange stop. It fires when the **1m bar’s session-local timestamp** has **`minute() == 50`** and **`close < stop1`** (stop1 set at entry from session open + `bo_stop1_range * range`). **`bo_eod_exit`** is a separate branch at **`bo_eod_exit_time`** (config **`23:30:00`** local for `7502T0U`). Fill at **:01** can lag the **:50** bar’s `close_time_utc` by a second.

### Gateway (WARN count **19** on 2026-04-15)

- Multiple **`cws_transport_failure`** with **`create:market`** in flight, same **`request_id`**s as runtime rejects (**08:01**, **08:51**, **09:51**, **10:43**).
- **Overnight / 23:40** — **`protocol_reset`** and **`eof`** on stream.

### Evening

- Grep over **19:00–24:00 UTC** on **2026-04-15** showed **no** additional **`bo_eod_exit`** / **`intent_emitted`** lines for this strategy (consistent with **flat** after **10:51**).

### Reading

- **Day outcome:** **two** BO round-trips (short then long) with **multiple `protocol_reset`** failures on **entry and exit** that still **converged** via retries / later bars.
- **Same failure class** as **2026-04-14** soak: **`create:market`** + **transport reset**.

---

## 4. Cross-stack notes

- All three runtimes show **~06:00 UTC** post-rollover **`ALLOWED`** after **SyncingHistory**.
- **All three gateways** log **TLS `unexpected_eof`** or **`ws_hub` reset`** around **23:40 UTC** — shared infra / peer behaviour.
- **Gateway WARN counts (approximate, 2026-04-15 only):** sessiongap **4**, hybrid **15**, alor-usdrubf **19**.

## 5. Operational reading

- **sessiongap:** **Successful** day with one **orphan_trade** cosmetic/event-ordering flag on exit — monitor recurrence.
- **hybrid:** **Successful** multi-leg day; **MR bracket** still **high-noise**; **MeanRevTimeCutoff** and **BO EOD** paths **did** flatten.
- **alor-usdrubf:** **Successful** convergence despite **repeated `protocol_reset`** on **USDRUBF market** intents; no additional **EOD** activity needed in the observed evening window.

## 6. Broker ledger reconciliation (user-supplied)

Assumption: broker UI timestamps are **Europe/Moscow (MSK, UTC+3)** on **2026-04-15**. Sub-account codes **`7502SN6`** (IMOEXF), **`7502T0U`** (USDRUBF BO stack), **`7502MIW`** (USDRUBF sessiongap) match the three stacks below.

### 6.1 Hybrid — IMOEXF (`7502SN6`)

| Broker (MSK) | Side | Qty | Price (log) | UTC (approx.) | Log / narrative |
| --- | --- | --- | --- | --- | --- |
| 09:52 | Продажа | 1 | **2717.5** | 06:52 | MR short entry (`2033126145129463235`) |
| 10:27 | Покупка | 1 | **2713.0** | 07:27 | TP / flatten buy (`2033126145129464180`) |
| 11:38 | Продажа | 1 | **2717.0** | 08:38 | Second MR short (`2033126145129513700`) |
| 11:55 | Покупка | 1 | **2717.5** | 08:55 | MeanRevTimeCutoff exit (`2033126145129520967`) |
| 11:56 | Продажа | 1 | **2717.0** | 08:56 | Immediate next MR short (`2033126145129521130`) |
| 11:57 | Покупка | 1 | **2716.0** | 08:57 | Second time-cut exit (`2033126145129521423`) |
| 14:11 | Покупка | 1 | **2736.5** | 11:11 | Breakout long (`2033126145129588636`) |
| 23:31 | Продажа | 1 | **2745.0** | 20:31 | BreakoutEodExit (`2033126145129759424`) |

**Verdict:** **8/8** legs match runtime **execution** prices and chronological roles (MR A → MR B → time cuts → BO → EOD). No phantom fills; no missing legs vs journal §2.

### 6.2 Alor-usdrubf — USDRUBF (`7502T0U`)

| Broker (MSK) | Side | Qty | Price | UTC (approx.) | Log / narrative |
| --- | --- | --- | --- | --- | --- |
| 11:02 | Продажа | 1 | **75.03** | 08:02 | Short open after reject (`2023555995921379427`) |
| 13:14 | Покупка | 1 | **75.40** | 10:14 | Flatten short (`2023555995921467238`) |
| 13:44 | Покупка | 1 | **75.49** | 10:44 | Long entry after reject (`2023555995921496950`) |
| 13:51 | Продажа | 1 | **75.56** | 10:51 | `bo_stop1_long` exit (`2023555995921501252`) |

**Verdict:** **4/4** match §3 (short round-trip, then long round-trip). Rejected **`protocol_reset`** attempts do not appear as separate fills — expected.

### 6.3 Sessiongap — USDRUBF (`7502MIW`)

| Broker (MSK) | Side | Qty | Price | UTC (approx.) | Log / narrative |
| --- | --- | --- | --- | --- | --- |
| 15:00 | Покупка | 1 | **75.93** | 12:00 | Session entry (`2023555995921588123`) |
| 23:30 | Продажа | 1 | **75.75** | 20:30 | Scheduled exit (same contour as **`orphan_trade` WARN** + `PendingExit → Flat`) |

**Verdict:** **2/2** match §1. Broker sell **75.75** aligns with log **`orphan_trade`** print (**75.75**); end state **flat**.

---

## 7. Follow-up

1. **`sessiongap` `orphan_trade`** at **20:30:08 UTC** — optional deep-dive: match **`order_id=2023555995921740415`** to gateway **`command_consumer`** timeline.
2. **Hybrid** — if **double MR** at **11:54 / 11:55** local is undesirable product-wise, treat as config/orchestrator review (not necessarily a bug).
3. **Alor-usdrubf** — continue tracking **`protocol_reset_without_close_handshake`** density vs **2026-04-14** report.
