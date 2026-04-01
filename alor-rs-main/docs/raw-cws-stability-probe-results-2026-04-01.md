# Raw CWS Stability Probe Results

Date: 2026-04-01

## Goal

Separate the broker/CWS control-path question from:

- strategy runtime state,
- Redis business flow,
- gateway hardening policy,
- `sessiongap` ownership / recovery semantics.

The probe used a minimal raw CWS client path:

1. connect to CWS,
2. authorize,
3. idle,
4. send one real `create:limit`,
5. optional cancel if order create succeeds.

## Executed Cases

### Case A: `idle30`

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1`
- price: `79.50`
- idle: `1800 sec`
- reconnect before send: `false`
- cancel final order: `true`

Artifact directory on VPS:

- `/opt/raw-cws-probe-results/idle30/raw-cws-probe-1775065131`

Observed sequence:

1. initial CWS connect succeeded
2. `authorize` ack succeeded
3. probe remained idle for 30 minutes
4. first real `create:limit` send was attempted
5. send failed immediately with:
   - `WebSocket protocol error: Connection reset without closing handshake`

Important event tail:

- `idle_complete`
- `create_limit_prepare`
- `send opcode=create:limit`
- `final_send_failed error="WebSocket protocol error: Connection reset without closing handshake"`

### Case B: `idle30 + reconnect-before-final-send`

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1`
- price: `79.50`
- idle: `1800 sec`
- reconnect before send: `true`
- cancel final order: `true`

Artifact directory on VPS:

- `/opt/raw-cws-probe-results/idle30-reconnect/raw-cws-probe-1775067084`

Observed sequence:

1. initial CWS connect succeeded
2. `authorize` ack succeeded
3. probe remained idle for 30 minutes
4. probe explicitly closed the old session
5. probe opened a fresh CWS session
6. second `authorize` ack succeeded
7. first real `create:limit` on the fresh session was attempted
8. send failed immediately with:
   - `WebSocket protocol error: Connection reset without closing handshake`

Important event tail:

- `idle_complete`
- `close_start phase="reconnect_before_final_send"`
- `connect_start phase="reconnect_before_final_send"`
- `authorize_ok phase="reconnect_before_final_send"`
- `create_limit_prepare`
- `send opcode=create:limit`
- `final_send_failed error="WebSocket protocol error: Connection reset without closing handshake"`

### Case C: `standalone idle60` smoke

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1`
- price: `79.50`
- idle: `60 sec`
- reconnect before send: `false`
- cancel final order: `true`

Artifact directory on VPS:

- `/opt/standalone-cws-probe-results/idle60-smoke/standalone-cws-probe-1775069997`

Observed sequence:

1. standalone probe connected successfully
2. `authorize` ack succeeded
3. probe remained idle for 60 seconds
4. real `create:limit` was sent
5. order create returned `httpCode=200`
6. optional `delete:limit` returned `httpCode=200`
7. no open position remained after the run

Important summary:

- `status=send_ok`
- `final_order_id=2023555952971993586`
- `final_http_code=200`
- `cancel_final_http_code=200`
- `final_error=none`

### Case D: `standalone idle30`

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1`
- price: `79.50`
- idle: `1800 sec`
- reconnect before send: `false`
- cancel final order: `true`

Artifact directory on VPS:

- `/opt/standalone-cws-probe-results/idle30/standalone-cws-probe-1775070448`

Observed sequence:

1. standalone probe connected successfully
2. `authorize` ack succeeded
3. probe remained idle for 30 minutes
4. first real `create:limit` send was attempted
5. send failed immediately with:
   - `WebSocket protocol error: Connection reset without closing handshake`
6. no `order_id` was returned
7. no follow-up cancel was possible

Important event tail:

- `idle_complete`
- `create_limit_prepare`
- `send opcode=create:limit`
- `final_send_failed error="WebSocket protocol error: Connection reset without closing handshake"`

Important summary:

- `status=send_failed`
- `final_order_id=none`
- `final_http_code=none`
- `cancel_final_http_code=none`
- `final_error=WebSocket protocol error: Connection reset without closing handshake`

### Case E: `window-scoped short session`, first send

Purpose:

- approximate an `entry`-like control action with a short-lived fresh CWS session,
- then fully close CWS after the order/cancel lifecycle completes.

Parameters:

- symbol: `USDRUBF`
- side: `buy`
- qty: `1`
- price: `79.50`
- idle: `60 sec`
- reconnect before send: `false`
- cancel final order: `true`

Artifact directory on VPS:

- `/opt/standalone-cws-probe-results/window-scoped-entry60/standalone-cws-probe-1775072985`

Observed sequence:

1. standalone probe connected successfully
2. `authorize` ack succeeded
3. probe remained idle for 60 seconds
4. `create:limit` returned `httpCode=200`
5. `delete:limit` returned `httpCode=200`
6. probe closed CWS cleanly

Important summary:

- `status=send_ok`
- `final_order_id=2023555952971996062`
- `final_http_code=200`
- `cancel_final_http_code=200`
- `final_error=none`
- `close_error=none`

### Case F: `window-scoped short session`, second send after 30m with no open CWS

Purpose:

- approximate an `exit`-like control action after a 30-minute gap,
- with no CWS session left open between the two order windows.

Parameters:

- symbol: `USDRUBF`
- side: `sell`
- qty: `1`
- price: `82.50`
- idle: `60 sec`
- reconnect before send: `false`
- cancel final order: `true`

Artifact directory on VPS:

- `/opt/standalone-cws-probe-results/window-scoped-exit60/standalone-cws-probe-1775074890`

Observed sequence:

1. first short-lived standalone run had already completed and closed CWS
2. approximately 30 minutes passed with no open CWS session
3. a new standalone probe opened a fresh CWS session
4. `authorize` ack succeeded
5. after 60 seconds idle, `create:limit` returned `httpCode=200`
6. `delete:limit` returned `httpCode=200`
7. probe closed CWS cleanly

Important summary:

- `status=send_ok`
- `final_order_id=2023555952972000226`
- `final_http_code=200`
- `cancel_final_http_code=200`
- `final_error=none`
- `close_error=none`

## Main Reading

The strongest result from 2026-04-01 is:

- the same failure class reproduces without `sessiongap`,
- without runtime state machine,
- without Redis command/ack orchestration,
- and even after a fresh reconnect + authorize immediately before the first control send.

This materially strengthens the hypothesis that a significant part of the failure lives in:

- broker-side / CWS session behavior,
- or the thin raw CWS control path itself,

not only in gateway policy logic.

The standalone results make the picture sharper:

- short-lived standalone control sessions can succeed,
- but the same thin standalone client fails after a 30-minute idle window,
- so the degradation is now reproduced even outside `alor-gateway` code reuse.

The additional window-scoped result strengthens a practical mitigation hypothesis:

- if CWS is left open and idle for 30 minutes, the first subsequent control send can reset;
- if CWS is opened only for a short order window, used, and then closed,
- and a second fresh short-lived session is opened 30 minutes later,
- both short-lived sends can succeed.

## What This Does Not Yet Prove

This result still does not fully separate:

- broker-side behavior,
- thin websocket client behavior,
- TLS/WebSocket library behavior.

But it does prove the issue is reproducible outside the full production contour.

It also does not yet prove that:

- "open a fresh CWS session immediately before every order and close it right after" is already a confirmed fix.

Current evidence on that idea is now directionally positive, but still incomplete:

- `standalone idle60` says a short fresh session can send successfully,
- the new window-scoped two-send experiment says two separate short fresh sessions, separated by 30 minutes with no open CWS between them, can both send successfully,
- but the earlier raw probe case `idle30 + reconnect-before-final-send` still failed,
- and the standalone `idle30 + reconnect-before-final-send` discriminator has not yet been run.

So the prudent reading today is:

- window-scoped / per-order CWS lifecycle is now a strong candidate mitigation,
- and it has stronger evidence than a simple "reconnect right before send on a previously long-idle path",
- but it is still not a fully proven production baseline.

## Operational Safety Notes

- `sessiongap` was stopped while the probe ran.
- After the probe set, no open broker position remained.
- No active working orders remained.
- `sessiongap` was brought back up and returned to `LiveReady / ALLOWED`.

## Follow-Up

The next most useful discriminators are now:

1. `standalone idle30 + reconnect-before-final-send`
2. optionally `standalone idle30 + ping keepalive`
3. if needed later, an explicit gateway mode that opens CWS only for a bounded control window and closes it after order result / cancel lifecycle

Why this is next:

- if standalone reconnect-before-send passes, then session freshness right before send may still be a valid mitigation path;
- if it fails the same way as plain standalone `idle30`, then the deeper issue is not solved simply by reconnecting before the order.

Operationally, the strongest current workaround hypothesis is:

- do not leave a control CWS session open across the whole strategy holding period,
- instead open CWS only around a specific control action,
- wait for the create / cancel / replace outcome,
- then close CWS again.
