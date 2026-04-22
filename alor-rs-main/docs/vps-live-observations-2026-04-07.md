# VPS live observations (2026-04-07)

Operational notes from `sessiongap` and related stacks on VPS (`sessiongap-strategy-runtime-1`, `sessiongap-alor-gateway-1`). Timestamps below are **UTC** unless noted.

## End-of-day soak verdict (operator)

- **Soak quality:** good — live events flowed through **all three** stacks (`session_gap_standalone`, `hybrid_intraday`, `AlorUsdrubfHybrid`).
- **Closures (2026-04-07, per runtime logs):** all three legs reached **flat** by end of UTC day (see **Position closes** below). Earlier same-day note about “two still open” is **superseded** by log evidence.

## Position closes (runtime logs, UTC)

### `session_gap_standalone` (`7502MIW`, `USDRUBF`)

- **20:30:10Z** — `InPosition` → `PendingExit` (`ts_utc=1775593740`).
- **20:30:10Z** — `intent_emitted` `action=place`, `request_id=ad08019e-13fe-5093-b337-1cb75b3de7d9` (cover short).
- **20:30:11Z** — `command accepted`, `broker_order_id=2023555970151887790`.
- **20:30:11Z** — `execution confirmed`: `buy` `qty=1.0`, `exec_price=78.47`, `commission=3.64`.
- **20:30:11Z** — `PendingExit` → `Flat`.

### `hybrid_intraday` (`7502SN6`, `IMOEXF`)

- Morning MR round-trip: entry `place` ~**07:57:40Z** (limit buy filled **08:02:41Z** @ 2785.5); bracket `place` + `create_stop_limit` accepted **08:02:41Z**.
- **08:04:46Z** — `intent_emitted` `delete_stop_limit`; limit **sell** (take) on order `2033126119359680618` **filled** @ **2789.5**; `delete_stop_limit` **accepted** (cleanup). Position closed for this cycle per execution line.

### `AlorUsdrubfHybrid` (`7502T0U`, `USDRUBF`)

- **20:31:05Z** — `intent_emitted` `market` exit, `request_id=7f31e68a-8a47-5375-b147-14fa3fcc05c3` → **`command rejected`** `cws_error` / `protocol_reset_without_close_handshake` (same class of transport flake as other stacks).
- **20:32:10Z** — retry: `intent_emitted` `market`, `request_id=eb72ba9b-77e3-518a-9b69-82c104f20e5d` → **`command accepted`**, `broker_order_id=2023555970151887921`.
- **20:32:10Z** — `execution confirmed`: `buy` `qty=1.0`, `exec_price=78.48`, `commission=3.64`.
- **20:32:10Z** — `broker position confirms flat state` (`qty=0`).

## Session gap standalone (`7502MIW`, `USDRUBF`)

### Entry (confirmed from logs)

- **06:01:12Z** — `live_guard_changed` to `ALLOWED`, `phase=LiveReady` (after overnight `BLOCKED` / history sync as usual).
- **14:00:01Z** — strategy phase `Flat` → `PendingEntry` (`ts_utc=1775570340`).
- **14:00:01Z** — `intent_emitted` `action=place`, `request_id=1331cdfe-3e62-5922-a4db-c717e03ba16d`.
- **14:00:01Z** — `command accepted`, `broker_order_id=2023555970151835364`.
- **14:00:01Z** — `execution confirmed`: `USDRUBF`, `side=sell`, `qty=1.0`, `order_price=78.44`, `exec_price=78.44`, `commission=3.64`.
- **14:00:01Z** — phase `PendingEntry` → `InPosition` (`ts_utc=1775570340`).

### Gateway (same request)

- `command received` for `strategy_id=session_gap_standalone`, `symbol=USDRUBF`, `action=place`.
- **Action-scoped CWS** path: `action_scope_session_open_start`, `create:limit`, `action_scope_authorize_ok` (`force_token_refresh_before_authorize=true`), `cws_limit_ack` `status=accepted`, `broker_order_id=2023555970151835364`.
- Supervisor: `trade` + `order` events (`working` → `filled`) tied to `request_id` via request map.

### Interpretation

- Entry matches expected **session_gap** flow: signal → limit `place` → immediate fill at limit price (no reject on this leg).
- Control path used **action_scoped** + fresh token on authorize, consistent with other live stacks.

### Overnight transport noise (context only)

- Earlier same day, gateway logs show `protocol_reset_without_close_handshake` and occasional `ws hub error` / subscribe retry; by session open the stack was healthy enough for `LiveReady` and the entry above.

## Related

- `AlorUsdrubfHybrid` / repeated position logs: `docs/alor-usdrubf-development-observations-2026-04-06.md` (monitoring addendum 2026-04-07).
