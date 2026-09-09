# ADR-070: Bound the Stripe Webhook Route's Pre-Signature Body Buffer via `axum::body::to_bytes`'s Own Limit Parameter

## Status

Accepted

## Context

`docs/product/production-readiness-audit-2026-09-08.md` finding #3 (Blocker):
`stripe_signature_middleware` (`crates/embyr-server/src/admin/middleware/stripe_signature.rs:44`)
calls `axum::body::to_bytes(body, usize::MAX)` to buffer the raw request body for HMAC verification —
necessary because Stripe's signature is computed over the exact raw bytes, so no streaming/
re-serialization is possible. The `Stripe-Signature` header's mere *presence* is checked first
(line 41), but the body is fully buffered into memory *before* its cryptographic validity is checked
(line 50). `usize::MAX` places no ceiling on this buffer. `POST /admin/v1/webhooks/stripe` carries no
session/operator auth — it is gated only by `stripe_signature_middleware` itself (US-203) — so an
unauthenticated attacker who reaches this route (reachable whenever `stripe-webhook-secret-required`'s
conditional mount is active, i.e. whenever Stripe billing is configured) can send an arbitrarily large
body and force the process to allocate memory proportional to whatever they choose to send, before any
credential is checked. A handful of concurrent requests of this shape exhausts server memory and takes
down every tenant's traffic on that instance.

Two mechanisms were named as open candidates at DISCUSS, deliberately left undecided:
- (a) a manual bounded read inside `stripe_signature_middleware` itself (checking `Content-Length`
  and/or replacing `usize::MAX` with a concrete constant).
- (b) an axum `RequestBodyLimitLayer`/`DefaultBodyLimit` applied at the webhook sub-router's
  construction site in `crates/embyr-server/src/admin/router.rs`.

**Confirmed before deciding:** `rg 'RequestBodyLimitLayer|DefaultBodyLimit|body_limit|MAX_BODY' crates/embyr-server`
and a full read of `embyr-server/Cargo.toml` and the workspace root `Cargo.toml` show `tower-http` is
**not a dependency anywhere in this workspace**, direct or transitive-and-exposed. Adding it purely for
`RequestBodyLimitLayer` would be a new external dependency, which DISCUSS's own § System Constraints
explicitly rules out ("No new external dependency"). `axum::extract::DefaultBodyLimit` is natively part
of the `axum` crate itself (already pinned at `0.7.9`, confirmed via `Cargo.lock`) and would not add a
new dependency, but it operates by wrapping the request body in a size-limited body type consumed by
axum's own body extractors — it does not change what `stripe_signature_middleware`'s own direct
`axum::body::to_bytes(body, usize::MAX)` call does, because that call already takes an explicit `limit`
parameter that is the intended, first-class mechanism for exactly this purpose. Layering
`DefaultBodyLimit` around a handler that itself calls `to_bytes(body, usize::MAX)` would either have no
effect (if `to_bytes` is invoked directly on the request body rather than through an extractor governed
by `DefaultBodyLimit`'s extension) or would require restructuring the middleware to route through a
`Bytes`/`String` extractor instead of its own explicit `into_parts()`/`to_bytes()` call — a larger,
unnecessary change to working code, for a mechanism that adds a second body-size-limiting code path
alongside `to_bytes`'s pre-existing one.

Confirmed root cause: `axum::body::to_bytes(body, limit)`'s `limit` parameter is not a rejection
threshold checked once against `Content-Length` up front — it is a *streaming* limit enforced while the
body is being aggregated (axum 0.7.9, `axum-core::body::to_bytes`): each polled frame is appended to an
internal buffer and the accumulated length is checked against `limit` after each frame; once exceeded,
the function returns `Err(axum::Error::new(http_body_util::LengthLimitError::default()))` without
continuing to buffer further frames. `http-body-util` (`0.1.3`) is already a direct workspace dependency
of `embyr-server` (`Cargo.toml:25`, pulled in for other body-handling code), so no new dependency is
needed to name or downcast this error type either. This means the vulnerable line's `usize::MAX` is not
a missing guard bolted on top of a correct call — it is the ONE call in this codebase that misuses a
guard the function itself already provides.

## Decision

**Replace `usize::MAX` with a concrete byte ceiling in the SAME `axum::body::to_bytes` call already
present at `stripe_signature.rs:44` (mechanism (a), in its simplest form — no `Content-Length`
pre-check, no router-level layer).** No other file changes.

```rust
// crates/embyr-server/src/admin/middleware/stripe_signature.rs

/// Stripe's own real-world webhook event payloads are typically a few KB,
/// rarely approaching the low hundreds of KB even for large `invoice.*`
/// events with many line items (Stripe publishes no single hard maximum).
/// 5 MiB is a deliberately generous ceiling — 10-50x any realistic real
/// payload — chosen to make AC-WBL-02 regression risk effectively zero
/// while still bounding an attacker's forced per-request allocation to a
/// small, fixed number instead of `usize::MAX` (ADR-070).
const MAX_STRIPE_WEBHOOK_BODY_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

let (parts, body) = req.into_parts();
let bytes = axum::body::to_bytes(body, MAX_STRIPE_WEBHOOK_BODY_BYTES)
    .await
    .map_err(|err| {
        if err
            .into_inner()
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some()
        {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::UNAUTHORIZED
        }
    })?;
```

(Illustrative — DELIVER owns exact naming/decomposition and confirms the `axum::Error::into_inner()` /
`http_body_util::LengthLimitError` downcast compiles and behaves as designed against the real pinned
`axum 0.7.9` / `http-body-util 0.1.3`, via a real HTTP request in an acceptance test — not by design-time
assertion alone.)

**Byte ceiling: 5 MiB (5,242,880 bytes).** Stripe does not publish a single documented hard maximum
webhook payload size; typical events are a few KB, and even large `invoice.finalized`-style events with
many line items stay well under 1 MB per this DISCUSS's and DESIGN's own research. 5 MiB is chosen as a
round, generous number (matching the audit's own "low single-digit megabytes" suggestion) that:
- Leaves 10-50x headroom over any realistic Stripe payload (AC-WBL-02's non-negotiable regression
  guard is the primary driver of "generous," not tightness).
- Still bounds a single request's forced allocation to a small, fixed number instead of unbounded —
  an attacker sending a 10 GB body now costs the process at most ~5 MiB + one frame of overrun, not 10 GB.
- Requires no runtime configuration, environment variable, or new admin-surface knob — this is a
  security ceiling, not an operational tuning parameter; no operator has a legitimate reason to change
  it, and Stripe's payload sizes are stable, well-known behavior of a third party this codebase does not
  control the size of.

**Status code:** `413 Payload Too Large` (`StatusCode::PAYLOAD_TOO_LARGE`) for the size-limit-exceeded
case specifically, discriminated from all other `to_bytes` error causes (network errors, disconnects,
malformed transfer-encoding), which continue to map to `401 Unauthorized` exactly as today —
unchanged, zero regression on the pre-existing error-handling behavior for those cases. This
discrimination is necessary because `axum::body::to_bytes`'s error type does not distinguish
size-limit-exceeded from any other body-read failure at the type level (both are folded into
`axum::Error`); the middleware must downcast the boxed inner error to `http_body_util::LengthLimitError`
to tell them apart. This is the ADR-level *contract* the crafter must satisfy (413 for size, 401 for
everything else); the exact downcast expression is DELIVER's own implementation, not prescribed further
here.

## Alternatives Considered

1. **axum `DefaultBodyLimit`/`tower_http::limit::RequestBodyLimitLayer` at the webhook sub-router's
   construction site (`router.rs`)** — rejected. `tower-http` is not a dependency anywhere in this
   workspace (confirmed by grep); adding it violates DISCUSS's "no new external dependency" constraint
   for a mechanism that duplicates a limit `stripe_signature_middleware`'s own `to_bytes` call already
   supports natively. Even using axum's native `DefaultBodyLimit` (no new dependency) would add a second,
   parallel body-size-limiting code path operating through axum's extractor-extension mechanism, which
   does not naturally compose with this middleware's own manual `into_parts()`/`to_bytes()` pattern
   without restructuring working, already-correct-shaped code for no additional benefit. Also would touch
   `router.rs`, widening blast radius beyond the single file this fix needs (DISCUSS's own narrow-scope
   preference).
2. **Manual `Content-Length` header pre-check before calling `to_bytes` at all** — considered and
   rejected as the *sole* mechanism (though compatible as a defense-in-depth *addition*, not adopted
   here per ponytail: don't add a second gate that does no additional bounding work). `Content-Length` is
   attacker-controlled and can be omitted or lied about (chunked transfer-encoding has no
   `Content-Length` at all); a pre-check alone would not close the vulnerability for a chunked-encoded
   attacker request with no `Content-Length` header, whereas `to_bytes`'s own streaming limit bounds the
   buffer regardless of what headers the attacker sends or omits. `to_bytes`'s limit parameter is a
   strict superset of what a `Content-Length` pre-check would provide.
3. **A configurable byte ceiling via environment variable/config** — rejected. This is a security
   invariant informed by a third party's (Stripe's) own payload-size behavior, not an
   operator-tunable trade-off; no legitimate deployment configuration would need to change it, and an
   operator-facing knob for a value they cannot meaningfully reason about (and might weaken) adds
   surface area with no offsetting benefit (ponytail: no config for a value that never changes).

## Consequences

### Positive
- Exactly one file changes: `crates/embyr-server/src/admin/middleware/stripe_signature.rs`. Zero
  changes to `router.rs` (does not re-touch `stripe-webhook-secret-required`'s already-shipped
  conditional-mount logic), zero new dependencies, zero new admin routes/config surface.
- The fix is the correct, idiomatic use of an API this codebase already calls — not new infrastructure.
- Bounds worst-case per-request forced memory allocation from unbounded to ~5 MiB regardless of
  attacker-supplied body size or transfer-encoding.
- Zero behavior change for any legitimate, normally-sized Stripe request (AC-WBL-02) — 5 MiB is far
  above any realistic payload, so the code path taken for real traffic is unaffected.
- Zero behavior change to the 401 rejection path for any other existing body-read failure mode.

### Negative / accepted residuals
- 5 MiB is a DESIGN-chosen generous estimate, not a number Stripe formally publishes as a maximum. If a
  future, legitimate Stripe event ever exceeds 5 MiB (no evidence found that this occurs in practice),
  it would be rejected with 413 rather than processed — judged acceptable given the multiple-order-of-
  magnitude headroom over every documented real-world payload size found during this DESIGN's research.
- Finding #39 (same route, `event_type` unbounded Prometheus label cardinality) remains unfixed — out of
  scope for this feature (DISCUSS § Out of Scope), unrelated mechanism (label cardinality, not body
  buffering).

## Enforcement

No new automated static-enforcement mechanism is warranted — this is a single-file, single-constant,
single-error-mapping-branch change with no new port/adapter boundary and no new external dependency.
The existing/new acceptance test suite (see feature-delta.md DESIGN § Handoff Package) is the
enforcement mechanism: a real oversized request must be rejected with 413 before full buffering, and a
real normally-sized signed request must still succeed — both proven against a real running server
instance, per this repo's own established proof standard for audit-derived fixes.
