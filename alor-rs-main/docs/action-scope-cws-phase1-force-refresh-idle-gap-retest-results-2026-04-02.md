# Action-Scope CWS Phase 1 Force-Refresh Idle-Gap Retest Results

Date: 2026-04-02

## Goal

Run one more canonical `~30m idle gap -> fresh create` confidence retest on the same live `sessiongap` action-scoped candidate with:

- `action_scope_force_token_refresh_before_authorize = true`

The purpose of this retest was to confirm that the earlier same-day force-refresh post-gap pass was not a one-off.

## Candidate Contour

- gateway image: `dev-3642910-actionscope-refresh-20260402-103346`
- gateway config: `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`
- runtime image unchanged: `dev-cf913bd-exit21a-r1-20260401-165815`

## Prior Successful Window

The previous force-refresh idle-gap window had already passed on the same gateway process:

- result note: `docs/action-scope-cws-phase1-force-refresh-idle-gap-results-2026-04-02.md`
- previous successful final close: about `2026-04-02 11:22:18 MSK`

## Retest Window

The next fresh bounded window started at about `2026-04-02 11:53:00 MSK`, which is about `30m 42s` after the previous successful close.

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1.0`
- price: `79.00`

Place:

- `request_id=d0af59fa-5494-44bb-b4e8-4189d50d25aa`
- `order_id=2023555957266754372`
- `status=accepted`
- `cws_http_code=200`

Observed intermediate order state:

- `status=working`
- `filled=0.0`

Cancel:

- `request_id=d87f107c-f81f-4cef-b4f7-2d8bbf349496`
- same `order_id=2023555957266754372`
- `status=accepted`
- `cws_http_code=200`

Observed final order state:

- `status=canceled`
- `filled=0.0`

## Force-Refresh Evidence

Gateway logs for the retest `create:limit` window showed:

1. action-scoped session open started
2. action-scoped session open succeeded
3. cached token invalidated with:
   - `reason="action_scope_force_token_refresh_before_authorize"`
4. token refreshed for `action_scope_cws_authorize` with:
   - `token_source="refreshed"`
   - `token_refresh_count=6`
   - `access_token_fingerprint="sha256:8a9f29605effec9f"`
5. `action_scope_authorize_ok` logged:
   - `access_token_source="refreshed"`
   - `force_token_refresh_before_authorize=true`
6. `create:limit` returned `http_code=200`
7. session closed cleanly

Gateway logs for the retest `delete:limit` window showed the same pattern:

1. cached token invalidated
2. token refreshed for `action_scope_cws_authorize`
3. `action_scope_authorize_ok` logged:
   - `access_token_source="refreshed"`
   - `force_token_refresh_before_authorize=true`
4. `delete:limit` returned `http_code=200`
5. session closed cleanly

## Safety Outcome

- no fill occurred
- final order status was `canceled`
- no `USDRUBF` broker position was opened
- latest broker `USDRUBF` position remained `qty=0.0`
- gateway stayed `LiveReady`
- runtime stayed `LiveReady / ALLOWED`

## Main Conclusion

This second force-refresh post-gap retest also passed.

The updated same-day picture is now:

- immediate bounded action-scoped `create -> delete`: `PASS`
- canonical `~30m idle gap -> fresh create` on cached in-process token state: `FAIL`
- fresh-token restart bounded window: `PASS`
- first canonical idle-gap on force-refresh action-scoped candidate: `PASS`
- second canonical idle-gap retest on the same force-refresh candidate: `PASS`

This further strengthens the working conclusion that token freshness or process-lived auth state is the primary discriminator in the failure class.
