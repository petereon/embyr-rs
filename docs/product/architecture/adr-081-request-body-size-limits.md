# ADR-081: Explicit Body/Message Size Ceilings on All Three Listeners

## Status
Accepted

## Context
`docs/product/production-readiness-audit-2026-09-08.md` finding #24 (Medium, Reliability):
no explicit message/request-body size limit is configured anywhere in `embyr-server`
(`crates/embyr-server/src/lib.rs:348,447-448`) — zero hits for
`max_decoding_message_size`/`DefaultBodyLimit`. The system relies entirely on tonic's and
axum's own implicit library defaults rather than a deliberate, tested ceiling.

Confirmed by direct reading, not assumed:
- **gRPC (:8080 native + :8081 grpc-web)**: both surfaces wrap the identical
  `FirestoreServer<FirestoreService>` (`lib.rs:481` and `rest/grpc_web.rs:65`). Neither call
  site sets `.max_decoding_message_size(...)`. Tonic 0.12's generated service wrapper applies
  its own compiled-in default (4 MiB) automatically when unset — a real ceiling exists today,
  it is simply not one anyone in this codebase chose or documented, and it is silently
  version-dependent (a tonic upgrade could change it without this codebase noticing).
- **REST/:8081 axum sub-router** (`accounts_bridge_dispatch`, browser channel, healthz):
  `accounts_bridge_dispatch`'s `body_bytes: axum::body::Bytes` parameter resolves via axum's
  own `impl FromRequest for bytes::Bytes`, which already enforces an implicit 2 MiB ceiling
  (axum 0.7's built-in `Bytes`/`Json`/`String`/`Form` extractor default) unless a
  `DefaultBodyLimit` layer overrides or disables it. No such layer exists today — the route is
  not literally unbounded, but its effective 2 MiB ceiling is an accidental default, not a
  chosen one.
- **Admin (:9090)**: same implicit-2-MiB-via-extractor situation for ordinary JSON admin
  routes. The one exception, `stripe_signature_middleware`, bypasses extractors entirely (calls
  `axum::body::to_bytes(body, ...)` directly on the raw body) and already has its OWN explicit,
  ADR-070-governed 5 MiB ceiling — untouched by this ADR.
- `docs/SPEC.md` documents no Firestore document/request size limits at all (confirmed by grep:
  zero hits for `MiB`/`document size`/`1048576`). This ADR's ceilings are therefore informed by
  real Firestore's own publicly documented platform limits (1 MiB max document size, 10 MiB max
  API request size) — general Firestore-protocol knowledge external to this repo, not a value
  this codebase had previously recorded anywhere — since embyr-server implements the Firestore
  wire protocol and must remain compatible with what real Firestore SDKs may legitimately send.
- Grep confirms no existing test sends a gRPC/REST/admin JSON payload anywhere near megabyte
  scale (`tests/**/*.rs`, searched for `.repeat(`/`vec![0u8;`/`vec![b'a';` patterns) — the only
  megabyte-scale test fixtures belong to the already-exempt Stripe webhook route
  (`pr07_stripe_webhook_body_limit.rs`), which uses its own separate, already-shipped mechanism
  (ADR-070) untouched here.

## Decision
Set explicit, named, deliberately-chosen byte ceilings at all three listener surfaces, using
each framework's own existing first-class configuration knob — no new dependency, no new
pattern:

| Surface | Mechanism | Ceiling | Call site |
|---|---|---|---|
| gRPC native (:8080) | `FirestoreServer::new(service).max_decoding_message_size(N)` | 10 MiB (10,485,760 B) | `crates/embyr-server/src/lib.rs`, `spawn_all_servers`, before `.add_service(...)` (~line 481) |
| gRPC-Web (:8081) | same call, same constant | 10 MiB | `crates/embyr-server/src/rest/grpc_web.rs`, `HybridService::new`, before `GrpcWebLayer::new().layer(...)` (~line 65) |
| REST/:8081 axum sub-router | `axum::extract::DefaultBodyLimit::max(N)` layered on `axum_app` | 2 MiB (2,097,152 B) | `crates/embyr-server/src/lib.rs`, `spawn_all_servers`, applied to the merged `axum_app` alongside the existing `cors` layer (~line 452) |
| Admin (:9090) | `axum::extract::DefaultBodyLimit::max(N)` layered on `admin_app` | 1 MiB (1,048,576 B) | `crates/embyr-server/src/admin/router.rs`, outermost layer on the `Router` returned by `build_with_aws`/`build_with_gcp` |

**gRPC (10 MiB, raised from tonic's implicit 4 MiB):** matches real Firestore's own published
per-RPC request-size ceiling. A real Firestore-protocol-compatible SDK may legitimately send a
`BatchWrite`/`Commit` containing several near-1-MiB documents in one call — tonic's undocumented
4 MiB default could silently reject such a request even though genuine Firestore would accept
it. Making the ceiling explicit at 10 MiB both documents intent and closes a latent
protocol-compatibility gap, while still bounding a single request's forced decode allocation to
a small, fixed number (not attacker-controlled `usize::MAX`-style unboundedness — that was never
the actual gap here, since tonic already refuses anything over its compiled-in default; the gap
was purely "nobody chose this on purpose").

**REST (2 MiB): unchanged from today's effective (implicit) behavior**, now made explicit.
Auth/session JSON payloads (`email`/`password`, OAuth `id_token`, custom tokens) are realistically
a few KB even for large JWTs; 2 MiB leaves 100x+ headroom with zero behavior change and zero
regression risk.

**Admin (1 MiB): tightened from the 2 MiB implicit default**, reflecting admin JSON payload
shape (project/index/secret-rotation config, all sub-KB to low-KB in practice) — still 1000x+
headroom over any real admin payload, confirmed by grep against existing admin route tests.

**No environment variable / operator-facing knob for any of the four values.** These are
protocol- and payload-shape-informed ceilings, not operational tuning parameters (mirrors
ADR-070's identical reasoning for the Stripe webhook ceiling: "no config for a value that never
changes" — ponytail). Unlike `EMBYR_RATE_LIMIT_RPS`/`EMBYR_CORS_ALLOWED_ORIGINS`, which vary
legitimately per deployment's real traffic/origin shape, a Firestore document's real-world size
behavior and this system's own internal payload shapes do not vary per deployment.

## Alternatives Considered
1. **Leave tonic/axum's implicit defaults as-is, undocumented.** Rejected — this is exactly the
   audit finding; an implicit, silently-version-dependent default is not a "deliberate, tested
   ceiling," even where its current numeric value happens to be reasonable.
2. **A single global constant reused across all three surfaces.** Rejected — gRPC document/batch
   traffic and REST/admin JSON auth-and-config traffic have genuinely different legitimate payload
   shapes (documents vs. credentials/config); a single number would either be too tight for
   Firestore-parity batch writes or needlessly loose for small JSON routes. Per-surface ceilings,
   each justified by that surface's own real payload shape, is the smaller-blast-radius, more
   defensible choice (mirrors this session's own root-cause/narrow-scope precedent from ADR-070).
3. **`EMBYR_*`-style environment-variable configuration for all four ceilings**, mirroring
   `EMBYR_RATE_LIMIT_RPS`/`EMBYR_CORS_ALLOWED_ORIGINS`. Rejected — these are protocol-shape
   security/reliability ceilings an operator has no legitimate, informed reason to change, not
   traffic-shape operational knobs; adding a config surface for a value that should not vary adds
   attack/misconfiguration surface with no offsetting benefit (ponytail).
4. **Keep tonic's 4 MiB implicit gRPC default rather than raising to 10 MiB.** Rejected — 4 MiB is
   below real Firestore's own documented request-size ceiling, meaning a real Firestore-SDK
   batch write that genuine Firestore would accept could be silently rejected by this proxy;
   10 MiB restores Firestore-protocol parity while still bounding worst-case allocation to a
   small fixed number instead of an unversioned library default.

## Consequences
### Positive
- All four ceilings are named, doc-commented constants instead of implicit library defaults —
  self-documenting, immune to silent change on a tonic/axum version bump.
- gRPC ceiling raised to genuine Firestore-protocol parity (10 MiB), closing a latent
  compatibility gap for legitimate large batch writes.
- REST ceiling unchanged in effective behavior (2 MiB) — zero regression risk.
- Admin ceiling tightened (1 MiB) with large confirmed headroom over real admin payloads.
- Zero new dependencies (tonic and axum both already expose these knobs natively).
- Zero interaction with the Stripe webhook route's own ADR-070 mechanism — confirmed the manual
  `to_bytes` call there bypasses the `DefaultBodyLimit` extension entirely, so the two limits
  compose independently and cannot conflict.

### Negative / accepted residuals
- The 10 MiB gRPC figure and the 1 MiB document figure are drawn from real Firestore's own
  published platform limits (external knowledge), not from anything `docs/SPEC.md` documents
  internally — `docs/SPEC.md` should be updated to record these limits explicitly so future
  features don't have to re-derive them (tracked as a documentation follow-up, not blocking this
  feature).
- If a future legitimate use case needs a REST/admin JSON payload genuinely larger than these
  ceilings (unlikely given current payload shapes), it will be rejected with axum's default
  `413 Payload Too Large` and require a deliberate ADR revision, not a silent config change —
  judged the correct trade-off (explicit ceilings should require an explicit decision to widen).

## Enforcement
No new automated static-enforcement mechanism — these are direct, one-call-site-each
configuration values using each framework's own first-class API, with no new port/adapter
boundary. The acceptance test suite (see feature-delta.md DESIGN § Handoff Package) is the
enforcement mechanism: a real oversized gRPC/REST/admin request must be rejected at each
surface's configured ceiling, and a real request at/under the ceiling must still succeed,
proven against a real running server instance per this repo's established proof standard.
