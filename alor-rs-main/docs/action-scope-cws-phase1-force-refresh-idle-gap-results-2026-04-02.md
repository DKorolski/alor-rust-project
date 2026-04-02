# Action-Scope CWS Phase 1 Force-Refresh Idle-Gap Results

Date: 2026-04-02

## Goal

Repeat the canonical Phase 1 `~30m idle gap -> fresh create` acceptance case on the live `sessiongap` action-scoped candidate after enabling:

- `action_scope_force_token_refresh_before_authorize = true`

The purpose of this check was to determine whether forcing a fresh access token before every action-scoped `authorize` changes the previously failing post-gap outcome.

## Candidate Contour

- gateway image: `dev-3642910-actionscope-refresh-20260402-103346`
- gateway config: `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`
- runtime image unchanged: `dev-cf913bd-exit21a-r1-20260401-165815`

## Prior Successful Window

The immediately preceding bounded window on the same gateway process had already passed:

- place `request_id=91daffe8-43d3-4046-a5c1-1734e7188b0f`
- cancel `request_id=dca38bbb-41c9-46d4-b598-249f120a2f12`
- `order_id=2023555957266707709`
- final cancel completed at about `2026-04-02 10:47:27 MSK`

## Idle-Gap Window

The next fresh bounded window started at about `2026-04-02 11:21:13 MSK`, which is about `33m 46s` after the prior successful close.

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1.0`
- price: `79.00`

Place:

- `request_id=5465ecd9-9666-4cb8-965f-d4d3fdc7deb4`
- `order_id=2023555957266735235`
- `status=accepted`
- `cws_http_code=200`

Observed intermediate order state:

- `status=working`
- `filled=0.0`

Cancel:

- `request_id=d2b0173e-3c97-4e2c-b245-11e2ceabb79d`
- same `order_id=2023555957266735235`
- `status=accepted`
- `cws_http_code=200`

Observed final order state:

- `status=canceled`
- `filled=0.0`

## Force-Refresh Evidence

Gateway logs for the post-gap `create:limit` window showed:

1. action-scoped session open started
2. action-scoped session open succeeded
3. cached access token invalidated with:
   - `reason="action_scope_force_token_refresh_before_authorize"`
4. access token refreshed with:
   - `consumer="action_scope_cws_authorize"`
   - `token_source="refreshed"`
   - `token_refresh_count=4`
   - `access_token_fingerprint="sha256:c0e9c0cd06b1a044"`
5. `action_scope_authorize_ok` logged:
   - `access_token_source="refreshed"`
   - `force_token_refresh_before_authorize=true`
6. `create:limit` returned `http_code=200`
7. session closed cleanly

Gateway logs for the post-gap `delete:limit` window showed the same pattern:

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

This canonical post-gap check passed once the action-scoped path forced a fresh access token before every `authorize`.

The combined same-day picture is now:

- immediate bounded action-scoped `create -> delete`: `PASS`
- canonical `~30m idle gap -> fresh create` on the earlier action-scoped candidate with cached in-process token state: `FAIL`
- gateway restart with fresh process-fresh token: `PASS`
- canonical `~30m idle gap` on the new force-refresh action-scoped candidate: `PASS`

This materially strengthens the hypothesis that token freshness or process-lived auth state is a primary discriminator in the failure class.
