# VPS Live Observations 2026-04-16

Date (session under review): **2026-04-16**  
Evidence: `docker logs --since 2026-04-16T00:00:00Z --until 2026-04-17T00:00:00Z` on **`155.212.170.21`** for each stack’s **strategy-runtime** and **alor-gateway** (SSH key `~/.ssh/id_rsa_lightnode`). Broker ledger reconciliation: **§6**.

## Scope

| Compose project | strategy-runtime | alor-gateway |
| --- | --- | --- |
| `trading-sessiongap` | `trading-sessiongap-strategy-runtime-1` | `trading-sessiongap-alor-gateway-1` |
| `trading-hybrid` | `trading-hybrid-strategy-runtime-1` | `trading-hybrid-alor-gateway-1` |
| `trading-alor-usdrubf` | `trading-alor-usdrubf-strategy-runtime-1` | `trading-alor-usdrubf-alor-gateway-1` |

## Executive summary

| Stack | Intents / trades (UTC day) | Errors / anomalies |
| --- | --- | --- |
| **sessiongap** | **09:00** entry buy **`2023556000216511771`** @ **76.72**; **20:30** exit **`bf5758cf-f47a-5007-934c-ce7b9e749b96`** sell **`2023556000216816386`** @ **76.18** → **Flat** | **No** `orphan_trade` on exit (clean vs **2026-04-15**); overnight hub/CWS churn **~03:30–03:34**; **~23:40** `ws_connected` / **SyncingGap** |
| **hybrid** | **Single** MR short **07:11 UTC**; **no** BO / **no** second MR in window | Bracket **TP** `protocol_reset_without_close_handshake`, **SL** `cws response timeout`; **SL @ 2765.5** filled on broker (**`orphan_trade`** buy **`2033126149424476454`**); gateway **9** WARN lines |
| **alor-usdrubf** | **MR short** **06:07** @ **76.03** → **`mr_take`** flat **06:45** @ **75.96**; **BO long** **08:03** @ **76.5**; **EOD** flat **20:32** @ **76.19** | **Many** hourly **`bo_stop1_long`** **`market`** exits **rejected** (`protocol_reset`); one **`trading_window_closed`** at **15:51 UTC**; **EOD** first attempt **20:31** rejected, **retry 20:32** succeeded; gateway **50** WARN lines |

---

## 1. `trading-sessiongap` (`session_gap_standalone`, USDRUBF)

### Intents / fills / phases

- **03:30–03:33 UTC** — `live_guard_changed` during **SyncingGap / SyncingHistory** (`ws_connected` churn).
- **05:59:09 UTC** — **`live_guard_changed` → `ALLOWED`** (ahead of formal rollover log).
- **06:00:01 UTC** — **`session rollover summary`**: `new_session_date=2026-04-16`, `gap_minutes=550`, `old_session_close=75.76`, `prev_close=75.76`.
- **09:00:04 UTC** — **`Flat → PendingEntry`**; **`intent_emitted`** `place` **`request_id=ddfa3668-32b7-509c-b658-eb33be5c69a7`**.
- **09:00:06 UTC** — **`execution confirmed`** **`order_id=2023556000216511771`**, **buy**, **exec_price=76.72**; **`PendingEntry → InPosition`**.
- **20:30:03 UTC** — **`InPosition → PendingExit`**; **`intent_emitted`** `place` **`request_id=bf5758cf-f47a-5007-934c-ce7b9e749b96`**.
- **20:30:03 UTC** — **`execution confirmed`** **`order_id=2023556000216816386`**, **sell**, **exec_price=76.18**; **`PendingExit → Flat`**.
- **23:40 UTC** — **`live_guard_changed`** **ALLOWED → BLOCKED** (`ws_connected=false`), then gateway/phase churn toward **SyncingGap**.

### Gateway (WARN count **3** on 2026-04-16)

- Pattern consistent with prior days: **ws_hub** / TLS idle disconnects overnight.

### Reading

- **Operational outcome:** one **round-trip**, **flat**; **clean exit** (no **`orphan_trade`** line in runtime slice).
- **Note:** runtime entry timestamp **09:00 UTC** = **12:00 MSK** on broker ledger (**§6**); vs **2026-04-15** journal (**12:00 UTC** entry) the shift is **calendar / session binding**, not a ledger mismatch.

---

## 2. `trading-hybrid` (`hybrid_intraday`, IMOEXF)

### MR only (no BO, no EOD trade)

- **06:00:00 UTC** — **`hybrid day rollover`**: `next_day=2026-04-16`, `prev_day_close=2745.5`, `prev_day_range=42.5`.
- **07:11:01 UTC** (`dt_local=2026-04-16 10:10:00`) — **`submit_entry`** **MorningMeanReversionShort** bracket **stop=2765.5**, **take=2740.0**; **`intent_emitted`** `place` **`af5d4a73-8113-5daf-98c4-bfed081d3ed4`**.
- **07:11:02 UTC** — **`execution confirmed`** **sell** **`2033126149424444243`**, **exec_price=2746.5**; TP **`place`** **`ec459704-…`**, SL **`create_stop_limit`** **`552605aa-…`**.
- **07:11:02–07 UTC** — **`command rejected`**: TP **`protocol_reset_without_close_handshake`**; SL **`cws response timeout`**; brief **`live_guard_changed` ALLOWED → BLOCKED** (CWS).
- **07:13:09 UTC** — repair **`intent_emitted`** `place` **`ce97556a-0f13-545a-acea-8c02c453a8da`** (take leg path).
- **07:39:30 UTC** — **`cancel`** **`c384dc95-…`**; **`WARN orphan_trade`**: **buy** **`2033126149424476454`** @ **2765.5** (broker **SL** fill while runtime bracket state was inconsistent); duplicate **`cancel`** line in same millisecond window.

### Gateway (WARN count **9** on 2026-04-16)

- **`protocol_reset` / `eof`** on CWS and **ws_hub** (overnight + post-bracket).

### Reading

- **Day outcome:** **one** MR leg, **stopped out** at **2765.5**; **flat** afterward — **no** intraday BO or hybrid EOD fill in logs.
- **Same class** of failure as **2026-04-15**: bracket legs vs transport; here **stop price** printed via **`orphan_trade`** after **`cancel`**.

---

## 3. `trading-alor-usdrubf` (`alor_usdrubf_hybrid_v1`, USDRUBF)

### Mean reversion (morning)

- **06:05 UTC** — **`mr_short_signal`** accepted (`signal_price≈75.91`).
- **06:06:00 UTC** — entry **`market`** **`4dccc3df-…`** → **`command rejected`** **`protocol_reset_without_close_handshake`**; defer next bar.
- **06:07:05 UTC** — retry **`325e16c1-…`** → **`execution confirmed`** **`2023556000216260702`**, **sell** **76.03**; **`initial_broker_sync_open`** short.
- **06:43:02 UTC** — exit **`mr_take`** **`3569bed6-…`** → **`protocol_reset`** rejected; defer.
- **06:45:01 UTC** — retry **`44bb81a4-…`** → **`execution confirmed`** **`2023556000216278205`**, **buy** **75.96**; **`open_to_flat`**.

### Breakout long (day hold + exit saga)

- **08:01 UTC** — **`bo_long_signal`** path; **08:02** first **`market`** **`20456b33-…`** → **`protocol_reset`** rejected.
- **08:03:02 UTC** — retry **`1a15a5e5-…`** → **`execution confirmed`** **`2023556000216431594`**, **buy** **76.5**; **`flat_to_open`** long.
- **09:51 through 19:51 UTC** — on each **hourly `:50` local grid**, runtime emits **`live exit intent`** **`exit_reason=bo_stop1_long`** + **`market`**; **almost all** acks **`protocol_reset_without_close_handshake`** (and **`live_guard`** churn **BLOCKED ↔ ALLOWED`** after each burst).
- **15:51:02 UTC** — one reject with **`error_code=trading_window_closed`** / **`validation failed`** (venue/session rule), still deferred.
- **20:31:04 UTC** — **`bo_eod_exit`** **`49798821-…`** → **`protocol_reset`** rejected.
- **20:32:25 UTC** — **`bo_eod_exit`** retry **`28813fcb-…`** → **`execution confirmed`** **`2023556000216816459`**, **sell** **76.19**; **`open_to_flat`**.

### Gateway (WARN count **50** on 2026-04-16)

- Dense **`cws_transport_failure`** with **`create:market`** in flight on MR entry, MR exit, and through the **BO exit** campaign; **`ws_hub`** resets overnight.

### Reading

- **Risk outcome:** strategy **did** flatten via **EOD** after **~12h** of failed **`bo_stop1_long`** attempts — **no evidence in logs of duplicate fill** beyond single long lot, but **operational stress** and **venue window** interaction deserve a dedicated incident note if this repeats.
- **Contrast 2026-04-15:** same **`bo_stop1_long`** semantics, but **2026-04-16** had **persistent transport failures** on exit until **EOD** bar.

---

## 4. Cross-stack notes

- **Hybrid** and **usdrubf** both show **bracket / `create:market`** fragility; **sessiongap** had a quieter transport footprint (**3** gateway WARNs vs **50** on usdrubf).
- **~23:50 UTC** — **hybrid** and **usdrubf** runtimes log **CWS / ws** disconnect pattern similar to prior days.

## 5. Operational reading

- **sessiongap:** routine **round-trip**; **exit cleaner** than prior day.
- **hybrid:** **small loss / stop** day on **one** MR; no trend leg.
- **alor-usdrubf:** **functionally OK** (flat by **EOD**) but **high-friction** — treat **2026-04-16** as a flag day for **exit reliability** on **USDRUBF** **`market`** under **`protocol_reset`** storms.

## 6. Broker ledger reconciliation (user-supplied)

Assumption: broker UI timestamps are **Europe/Moscow (MSK, UTC+3)** on **2026-04-16**. Sub-account codes **`7502T0U`**, **`7502SN6`**, **`7502MIW`** match the three stacks.

### 6.1 Alor-usdrubf — USDRUBF (`7502T0U`)

| Broker (MSK) | Side | Qty | Price | UTC (approx.) | Log / narrative |
| --- | --- | --- | --- | --- | --- |
| 09:07 | Продажа | 1 | **76.03** | 06:07 | MR short open (`2023556000216260702`) |
| 09:45 | Покупка | 1 | **75.96** | 06:45 | `mr_take` flatten (`2023556000216278205`) |
| 11:03 | Покупка | 1 | **76.50** | 08:03 | BO long (`2023556000216431594`) |
| 23:32 | Продажа | 1 | **76.19** | 20:32 | `bo_eod_exit` (`2023556000216816459`) |

**Verdict:** **4/4** legs match §3. Rejected **`bo_stop1_long`** / first **EOD** attempts do not appear as extra fills. Broker stamp **23:32** vs runtime **20:32:25 UTC** is normal display/latency skew.

### 6.2 Hybrid — IMOEXF (`7502SN6`)

| Broker (MSK) | Side | Qty | Price | UTC (approx.) | Log / narrative |
| --- | --- | --- | --- | --- | --- |
| 10:11 | Продажа | 1 | **2746.5** | 07:11 | MR short entry (`2033126149424444243`) |
| 10:39 | Покупка | 1 | **2765.5** | 07:39 | SL / cover (`orphan_trade` `2033126149424476454`) |

**Verdict:** **2/2** match §2 (single MR, stop-out). No BO / no extra IMOEXF legs.

### 6.3 Sessiongap — USDRUBF (`7502MIW`)

| Broker (MSK) | Side | Qty | Price | UTC (approx.) | Log / narrative |
| --- | --- | --- | --- | --- | --- |
| 12:00 | Покупка | 1 | **76.72** | 09:00 | Session entry (`2023556000216511771`) |
| 23:30 | Продажа | 1 | **76.18** | 20:30 | Scheduled exit (`2023556000216816386`) |

**Verdict:** **2/2** match §1. Confirms **09:00 UTC** entry ≡ **12:00 MSK** on broker.

---

## 7. Follow-up

1. **usdrubf:** post-mortem on **hourly `bo_stop1_long`** reject loop + **`trading_window_closed`** at **15:51 UTC** — consider backoff, idempotency metrics, or alert if **`exit_intent_inflight`** spans **>N** hours with same reason.
2. **hybrid:** correlate **`orphan_trade`** @ **2765.5** with gateway **`create:stop_limit`** never acked (SL timeout path).
3. **sessiongap:** optional doc note explaining **09:00 vs 12:00 UTC** entry time shift vs **2026-04-15** (session calendar / MOEX refs).
