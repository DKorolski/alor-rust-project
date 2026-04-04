# Sessiongap Runtime Image Decision Memo

Date: 2026-03-27

## 1. Decision Question

Before the next controlled live window, which exact `strategy-runtime` image should be treated as the approved `sessiongap` runtime baseline?

Current mismatch:

- currently running on VPS:
  - `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`
- currently resolved by `docker compose config` from `.env`:
  - `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-774b917-diag-20260326`

This mismatch exists only because the stack currently uses one shared `IMAGE_TAG` variable for both:

- `alor-gateway`
- `strategy-runtime`

## 2. Facts Already Confirmed

### 2.1 What is actually running now

On VPS:

- `sessiongap-strategy-runtime-1` is running as `strategy-runtime:dev-a1ee034`
- `sessiongap-alor-gateway-1` is running as `alor-gateway:dev-774b917-diag-20260326`

### 2.2 What was intentionally rolled out on 2026-03-26

Per `docs/create-limit-hardening-2.0-results-2026-03-26.md`:

- hardening 2.0 rollout scope was gateway-only;
- `strategy-runtime` was explicitly not recreated.

That means the operationally validated contour after hardening rollout was:

- gateway on `dev-774b917-diag-20260326`
- runtime still on `dev-a1ee034`

### 2.3 What runtime image `dev-a1ee034` represents

Git commit:

- `a1ee034 fix(runtime): restore state when live intents are dropped`

Also, `docs/create-limit-delete-limit-post-fix-clean-loop-2026-03-25.md` records:

- runtime image left unchanged:
  - `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`

So `dev-a1ee034` is not random residue. It is the runtime image that stayed in place across the later gateway-only hardening validation wave.

### 2.4 What is not established

There is currently no explicit rollout note saying that:

- `strategy-runtime:dev-774b917-diag-20260326`
- was intentionally selected,
- deployed,
- and validated as the runtime baseline for the next live window.

Also, commit `774b917` is a gateway hardening commit:

- `feat(gateway): recycle stale control path before limit entry`

So the `dev-774b917-diag-20260326` tag is clearly justified for gateway, but not yet clearly justified as the runtime image to freeze for the live window.

## 3. Options

### Option A. Freeze runtime on `dev-a1ee034`

What this means:

- keep `sessiongap` runtime baseline on `ghcr.io/dkorolski/alor-rust-project/strategy-runtime:dev-a1ee034`
- keep gateway on `ghcr.io/dkorolski/alor-rust-project/alor-gateway:dev-774b917-diag-20260326`
- split image selection so compose resolves to the same pair that is already running

Pros:

- preserves the contour that actually existed after the gateway-only hardening rollout;
- avoids introducing a last-minute runtime change before the live window;
- matches the strongest operational evidence we already have;
- makes `running state == resolved state` once tags are split cleanly.

Cons:

- requires deployment cleanup because one shared `IMAGE_TAG` is no longer enough;
- needs an explicit per-service image-tag policy in compose or env.

Risk profile:

- lowest.

### Option B. Promote runtime to `dev-774b917-diag-20260326`

What this means:

- recreate `sessiongap` runtime onto the compose-resolved tag;
- make both gateway and runtime use the same current tag.

Pros:

- removes the mismatch without changing compose structure;
- gives one tag for the whole stack.

Cons:

- changes runtime immediately before the controlled live window;
- there is no equivalent evidence pack showing that this runtime image was the intended validated baseline;
- the tag naming is tied to gateway hardening rollout and looks diagnostic;
- broadens the pre-window risk surface without a strong reason.

Risk profile:

- medium to high.

### Option C. Leave VPS as-is and rely on “do not recreate runtime”

What this means:

- keep current containers untouched;
- keep shared `IMAGE_TAG` pointing at `dev-774b917-diag-20260326`;
- assume operators will avoid any compose action that recreates runtime.

Pros:

- no immediate deployment work.

Cons:

- fragile and operationally unsafe;
- a normal `docker compose up -d`, `pull`, or recreate can silently change runtime;
- the exact stack for the live window remains ambiguous.

Risk profile:

- unacceptable for pre-live sign-off.

## 4. Recommendation

Recommended choice:

- **Option A: freeze `sessiongap` runtime on `dev-a1ee034` and split image tags by service.**

Why:

1. It best matches the actual validated contour after the 2026-03-26 hardening rollout.
2. It avoids introducing a fresh runtime change right before the live window.
3. It lets us keep hardening 2.0 on gateway without pretending that runtime was part of that rollout.
4. It gives one clean answer to the operator question:
   - gateway image for window:
     - `alor-gateway:dev-774b917-diag-20260326`
   - runtime image for window:
     - `strategy-runtime:dev-a1ee034`

## 5. Exact Next Step

Do this before any further live-window preparation:

1. Replace the shared stack-wide `IMAGE_TAG` model with explicit per-service tags, for example:
   - `GATEWAY_IMAGE_TAG=dev-774b917-diag-20260326`
   - `RUNTIME_IMAGE_TAG=dev-a1ee034`
2. Update compose so:
   - `alor-gateway` uses `GATEWAY_IMAGE_TAG`
   - `strategy-runtime` uses `RUNTIME_IMAGE_TAG`
3. Re-run read-only verification:
   - `docker compose config`
   - `docker ps`
   - confirm `sessiongap` resolved state matches running state exactly
4. Only after that continue to:
   - hardening config pinning
   - env/backups hygiene
   - final pre-window readiness check

## 6. Operational Rule For The Window

Until image-tag split is done:

- do not treat the current VPS state as fully frozen for controlled live;
- do not run generic compose recreate flows for `sessiongap` runtime.

After image-tag split is done:

- the exact stack answer becomes stable and reviewable.

## 7. Bottom Line

The safest pre-window move is not to “upgrade runtime to whatever compose currently points to”.

The safest move is:

- keep the already-running `sessiongap` runtime baseline `dev-a1ee034`,
- keep gateway hardening on `dev-774b917-diag-20260326`,
- and make deployment config reflect that pairing explicitly.
