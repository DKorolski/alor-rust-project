# Action-Scope CWS Phase 1 Force-Refresh Long-Gap Retest Results

Date: 2026-04-02

## Goal

Run one more confidence retest on the same live `sessiongap` candidate after a longer quiet window of about `45-50m`, still using:

- `control_cws_mode = "action_scoped"`
- `action_scope_force_token_refresh_before_authorize = true`

The purpose of this retest was to check whether the force-refresh post-gap pass remains stable beyond the earlier `~30m` windows.

## Candidate Contour

- gateway image: `dev-3642910-actionscope-refresh-20260402-103346`
- gateway config: `configs/gateway.sessiongap.live.7502MIW.action-scoped.toml`
- runtime image unchanged: `dev-cf913bd-exit21a-r1-20260401-165815`

## Retest Window

The next bounded window started at about `2026-04-02 12:43:09 MSK`.

Observed gap from the previous successful close:

- about `49m 46s`

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1.0`
- price: `79.00`

Place:

- `request_id=56b7df14-90c8-40e6-862f-31ab20c0550e`
- `order_id=2023555957266794458`
- `status=accepted`
- `cws_http_code=200`

Observed intermediate order state:

- `status=working`
- `filled=0.0`

Cancel:

- `request_id=673736b1-372e-45b0-b007-2d3dea62e26f`
- same `order_id=2023555957266794458`
- `status=accepted`
- `cws_http_code=200`

Observed final order state:

- `status=canceled`
- `filled=0.0`

## Force-Refresh Evidence

Gateway logs for the `create:limit` window showed:

1. action-scoped session open succeeded
2. cached token invalidated with:
   - `reason="action_scope_force_token_refresh_before_authorize"`
3. token refreshed for `action_scope_cws_authorize` with:
   - `token_source="refreshed"`
   - `token_refresh_count=8`
   - `access_token_fingerprint="sha256:815c6e94fd7b715c"`
4. `action_scope_authorize_ok` logged:
   - `access_token_source="refreshed"`
   - `force_token_refresh_before_authorize=true`
5. `create:limit` returned `http_code=200`
6. session closed cleanly

Gateway logs for the `delete:limit` window showed the same pattern:

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
- gateway stayed `LiveReady`
- runtime stayed `LiveReady / ALLOWED`

## Main Conclusion

This longer-gap force-refresh retest also passed.

The same-day Phase 1 picture is now:

- immediate bounded action-scoped `create -> delete`: `PASS`
- canonical post-gap on cached in-process token state: `FAIL`
- fresh-token restart bounded window: `PASS`
- first force-refresh post-gap retest: `PASS`
- second force-refresh post-gap retest: `PASS`
- longer `~50m` force-refresh retest: `PASS`

This further strengthens the decision to treat `action_scoped + force_token_refresh_before_authorize` as the primary Phase 1 development baseline while moving into Phase 2 validation.
