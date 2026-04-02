# Action-Scope CWS Phase 1 Fresh-Token Restart Results

Date: 2026-04-02

## Goal

Repeat the controlled Phase 1 `create -> delete` check after forcing a fresh gateway process and a fresh access token, to test whether token freshness changes the outcome.

## Method

1. restart only the `sessiongap` gateway candidate
2. wait for gateway `LiveReady`
3. confirm the new process obtained a fresh access token
4. run a passive `create -> delete` bounded control cycle

Candidate contour:

- gateway image: `dev-3642910-actionscope1-20260402-004402`
- gateway config: `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`
- runtime image unchanged: `dev-cf913bd-exit21a-r1-20260401-165815`

## Fresh Token Evidence

After the gateway restart:

- new `gateway_instance_id=1ef409cc-7305-4eb6-b6aa-4a1159a0eb6d`
- `token_refresh_count=1`
- `access_token_fingerprint=sha256:07006ef19a81331e`
- `access_token_obtained_ts_utc=1775114711`

Gateway startup log:

- `refreshing alor access token`
- `refreshed alor access token consumer="ws_hub_connect"`

Important nuance:

- the later action-scoped `authorize` logs still show `access_token_source="cached"`
- but that cached token belongs to the freshly restarted gateway process and had just been refreshed at startup

So this run used a fresh process-fresh access token even though the action-scoped authorize step reused it from in-memory cache.

## Controlled Cycle

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1.0`
- price: `78.00`

Place:

- `request_id=862ccc3f-388a-4cbe-9aa4-5bbce4b3a2b2`
- `order_id=2023555957266686184`
- `status=accepted`
- `cws_http_code=200`

Observed intermediate order state:

- `status=working`
- `filled=0.0`

Cancel:

- `request_id=97c8df07-bae0-4548-a6a2-0371ea67d342`
- same `order_id=2023555957266686184`
- `status=accepted`
- `cws_http_code=200`

Observed final order state:

- `status=canceled`
- `filled=0.0`

## Action-Scope Evidence

Action-scoped logs showed:

1. `create:limit`
   - fresh session open
   - authorize ok
   - `create:limit` send result `http_code=200`
2. `delete:limit`
   - second fresh session open
   - authorize ok
   - `delete:limit` send result `http_code=200`

Post-run counters:

- `action_scope_open_total=2`
- `action_scope_send_total=4`
- `action_scope_close_total=2`
- `commands_received_total=2`
- `command_processed_total=2`
- `last_action_scope_error=none`

## Safety Outcome

- no fill occurred
- final order status was `canceled`
- no `USDRUBF` broker position was opened
- gateway stayed `LiveReady`
- runtime stayed `LiveReady / ALLOWED`

## Main Conclusion

This fresh-token restart check passed.

Combined with the earlier same-day results, the current directional reading is:

- immediate action-scoped bounded windows can pass
- the canonical `~30m idle gap -> fresh create` case failed when the gateway reused its pre-existing in-process token state
- after restarting the gateway and obtaining a fresh process-fresh access token, the same bounded `create -> delete` flow passed again

This materially strengthens the working hypothesis that access-token freshness or process-lived auth state is a meaningful discriminator in the failure class.
