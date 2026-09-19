# Feature Delta: request-body-size-limits

Token-budget mode: DISCUSS + DESIGN combined in one pass.

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` finding #24 (Medium, Reliability)
confirmed: no explicit message/request-body size limit anywhere in `embyr-server`
(`crates/embyr-server/src/lib.rs:348,447-448`), zero hits for
`max_decoding_message_size`/`DefaultBodyLimit`.
✓ `crates/embyr-server/src/lib.rs` read in full (1150 lines). `spawn_all_servers` (line 331):
`tonic::transport::Server::builder()` (line 470) → `.add_service(FirestoreServer::new(service))`
(line 481) — no `.max_decoding_message_size(...)` call anywhere. `axum_app` built at lines
361-452, merged with `accounts_bridge_app`, `.layer(cors)` applied outermost (line 452) — no
`DefaultBodyLimit` layer present.
✓ `crates/embyr-server/src/rest/grpc_web.rs` read in full (154 lines). `HybridService::new`
(line 64) constructs `FirestoreServer::new(grpc_service)` wrapped in `GrpcWebLayer::new()` (line
65) — this is the SAME `FirestoreServer<FirestoreService>` type as the native :8080 path, serving
identical Firestore RPCs (including document writes) over gRPC-Web on :8081. Confirms both gRPC
surfaces need the same treatment, not just the one the audit line-numbers point at.
✓ Confirmed via grep: `tower-http = { version = "0.5", features = ["cors"] }` IS now a direct
`embyr-server` dependency (added by `cors-origin-policy`, AFTER `stripe-webhook-body-limit`'s own
ADR-070 concluded "not a dependency anywhere in this workspace" — that conclusion is now stale for
`tower-http` generally, though irrelevant here since axum's own native `DefaultBodyLimit`
mechanism is used, not `tower_http::limit::RequestBodyLimitLayer` — zero new dependency either
way).
✓ Confirmed axum 0.7.9's actual behavior (not guessed): `axum::body::Bytes` used as a handler
parameter type (`accounts_bridge_dispatch`'s `body_bytes: axum::body::Bytes`) resolves via axum's
`impl FromRequest for bytes::Bytes` — the SAME extractor mechanism `Json`/`String`/`Form` use
internally. This impl already enforces an **implicit 2 MiB default** (axum's own built-in
`Bytes`/`Json`/`String`/`Form` ceiling) unless a `DefaultBodyLimit` layer overrides or disables
it. **The REST surface is therefore not literally unbounded today** — it has an accidental,
undocumented 2 MiB ceiling. This matters: it means "making the ceiling explicit" for REST can be a
zero-behavior-change documentation/enforcement fix, not necessarily a new restriction.
✓ Confirmed tonic 0.12's generated `FirestoreServer` wrapper applies its own compiled-in default
(4 MiB) automatically when `.max_decoding_message_size(...)` is unset — same "real ceiling exists,
nobody chose it" situation as REST, on the gRPC side.
✓ `crates/embyr-server/src/admin/middleware/stripe_signature.rs` re-read. Confirmed its
`axum::body::to_bytes(body, MAX_STRIPE_WEBHOOK_BODY_BYTES)` call (ADR-070, 5 MiB) operates on the
raw body obtained via `req.into_parts()` — **not** through an extractor — so it does **not** read
the `DefaultBodyLimit` extension at all. A new admin-level `DefaultBodyLimit` layer therefore
cannot conflict with or double-govern the already-shipped Stripe webhook ceiling; the two
mechanisms compose independently. Confirmed, not assumed.
✓ `docs/SPEC.md` grepped (case-insensitive) for `MiB`, `document size`, `1048576`, `1 MiB` — **zero
matches**. This repo has never internally documented Firestore's real-world document/request size
limits. This DESIGN uses real Firestore's own publicly documented platform limits (1 MiB max
document size, 10 MiB max API request size) as external, well-established informing knowledge —
flagged explicitly rather than silently assumed, since it is not sourced from this repo.
✓ Grepped `tests/**/*.rs` for `.repeat(`, `vec![0u8;`, `vec![b'a';`/`vec![b'x';`-shaped large
payload construction. Only megabyte-scale fixtures found belong to
`tests/production_readiness/acceptance/pr07_stripe_webhook_body_limit.rs` (the already-shipped,
already-exempt ADR-070 Stripe route). No gRPC document-write test, no REST auth-route test, no
other admin-route test constructs a payload anywhere near megabyte scale. Confirms this feature's
chosen ceilings carry no regression risk against the existing suite.
✓ `docs/product/architecture/adr-070-stripe-webhook-body-size-ceiling.md` and its
`docs/feature/stripe-webhook-body-limit/feature-delta.md` read in full — reused as this feature's
own precedent for: narrow root-cause fix shape, "no env var for a value that never changes"
reasoning, and doc-commented `const` convention colocated at each call site.
✓ `crates/embyr-server/src/config.rs` grepped for the `EMBYR_*` convention (confirmed 15+ existing
env vars: `EMBYR_RATE_LIMIT_RPS`, `EMBYR_CORS_ALLOWED_ORIGINS`, `EMBYR_TENANT_DB_MAX_CONNECTIONS`,
etc.) — all govern genuinely deployment-varying operational behavior (traffic shape, pool sizing,
allowed origins). None govern a protocol/payload-shape security ceiling; this feature does not fit
that pattern (see ADR-081 Alternative 3).

## Wave: DISCUSS / [REF] Decisions (locked, not re-litigated at DELIVER)

- **Scope: all three listener surfaces** (:8080 gRPC native, :8081 gRPC-Web + REST, :9090 admin) —
  the audit finding names both `max_decoding_message_size` (gRPC) and `DefaultBodyLimit`
  (axum/REST) explicitly, and this DESIGN's own reading confirmed :8081 carries BOTH a gRPC-Web
  surface and a REST surface that need independent treatment, plus :9090 admin as a fourth
  distinct surface. Not a single global constant (see ADR-081 Alternative 2).
- **Mechanism**: each framework's own existing first-class knob —
  `FirestoreServer::new(..).max_decoding_message_size(N)` for both gRPC surfaces,
  `axum::extract::DefaultBodyLimit::max(N)` for REST and admin. Zero new dependency.
- **No environment variable** for any of the four ceilings — protocol/payload-shape security and
  reliability ceilings, not operator-tunable traffic knobs (full reasoning: ADR-081).
- Full ADR written: **ADR-081** (see below) — this DESIGN concluded an ADR IS warranted, contrary
  to the task's own tentative "probably none needed" framing. Reasoning: this decision sets four
  distinct numeric ceilings across three production listener surfaces, with genuine alternatives
  (global-vs-per-surface value, raise-vs-keep the gRPC default, env-var-vs-constant) each requiring
  documented rejection rationale — the same bar ADR-070 already set for a narrower, single-route
  case. "Applying an existing library knob" describes the MECHANISM, not the VALUE-SELECTION
  decision, which is the substantive part needing a record.

## Wave: DESIGN / Exact Wiring

### 1. gRPC native (:8080) — `crates/embyr-server/src/lib.rs`, `spawn_all_servers`
Around line 481 (`grpc_builder.add_service(FirestoreServer::new(service))`), change to:
```
.add_service(FirestoreServer::new(service).max_decoding_message_size(MAX_GRPC_MESSAGE_BYTES))
```
with `const MAX_GRPC_MESSAGE_BYTES: usize = 10 * 1024 * 1024;` (10 MiB) doc-commented at
definition site per ADR-081.

### 2. gRPC-Web (:8081) — `crates/embyr-server/src/rest/grpc_web.rs`, `HybridService::new`
Around line 65 (`GrpcWebLayer::new().layer(FirestoreServer::new(grpc_service))`), change to:
```
GrpcWebLayer::new().layer(
    FirestoreServer::new(grpc_service).max_decoding_message_size(MAX_GRPC_MESSAGE_BYTES),
)
```
Same 10 MiB constant — either a shared `pub(crate) const` in `lib.rs` re-exported, or duplicated
with an identical doc comment (DELIVER's own call on smallest-diff sharing mechanism; both files
already cross-reference each other's types, e.g. `FirestoreService` import).

### 3. REST/:8081 axum sub-router — `crates/embyr-server/src/lib.rs`, `spawn_all_servers`
Around line 452 (`let axum_app = axum_app.merge(accounts_bridge_app).layer(cors);`), add the body
limit layer to the same merged router, e.g.:
```
let axum_app = axum_app
    .merge(accounts_bridge_app)
    .layer(axum::extract::DefaultBodyLimit::max(MAX_REST_BODY_BYTES))
    .layer(cors);
```
with `const MAX_REST_BODY_BYTES: usize = 2 * 1024 * 1024;` (2 MiB — matches today's implicit
axum default exactly; zero effective behavior change, now explicit/tested).

### 4. Admin (:9090) — `crates/embyr-server/src/admin/router.rs`
Outermost `.layer(axum::extract::DefaultBodyLimit::max(MAX_ADMIN_BODY_BYTES))` on the `Router`
returned by `build_with_aws`/`build_with_gcp` (exact insertion point is DELIVER's own call —
DESIGN confirms it must sit outside/alongside whatever layers already exist there, without
re-touching the Stripe webhook sub-router's own `Option<Router>` conditional mount or its
`route_layer(stripe_signature_middleware)` wiring, both untouched by this feature per the
Reading Confirmation above). `const MAX_ADMIN_BODY_BYTES: usize = 1 * 1024 * 1024;` (1 MiB).

## Wave: DESIGN / ADR Decision

**ADR-081 written**: `docs/product/architecture/adr-081-request-body-size-limits.md`. Full
context, 4-way alternatives analysis (global-vs-per-surface constant, env-var-vs-constant,
raise-vs-keep gRPC's implicit default, leave-implicit-vs-make-explicit), consequences, and
enforcement rationale recorded there — not duplicated here.

## Wave: DESIGN / Self-Review — Regression Risk

**Verdict: no existing test is expected to break.**
- gRPC ceiling is RAISED (4 MiB implicit → 10 MiB explicit) — strictly more permissive; cannot
  newly reject anything that passed before.
- REST ceiling is UNCHANGED in effective value (2 MiB implicit → 2 MiB explicit) — byte-identical
  behavior for every existing request shape.
- Admin ceiling is TIGHTENED (2 MiB implicit → 1 MiB explicit) — the only surface where a
  regression is theoretically possible. Grep of `tests/**/*.rs` found zero admin-route test
  constructing a payload approaching 1 MiB; the one MiB-scale fixture in this codebase
  (`pr07_stripe_webhook_body_limit.rs`'s 1 MiB `"x".repeat(...)` filler) belongs to the Stripe
  webhook route, which is structurally exempt (manual `to_bytes` call bypasses the
  `DefaultBodyLimit` extension entirely — confirmed above, not assumed).
- No production code changes made this wave (design-only per task constraints) — self-review is
  a static/grep-based check against the current test suite's actual payload shapes, not a live
  run. DISTILL/DELIVER must still run the full existing suite once at the pre-commit gate to
  confirm empirically.

## Wave: DESIGN / Handoff Package

**Files requiring a change (DELIVER):**
1. `crates/embyr-server/src/lib.rs` — `MAX_GRPC_MESSAGE_BYTES`/`MAX_REST_BODY_BYTES` constants;
   `.max_decoding_message_size(...)` on the native gRPC `FirestoreServer`; `DefaultBodyLimit`
   layer on `axum_app`.
2. `crates/embyr-server/src/rest/grpc_web.rs` — `.max_decoding_message_size(...)` on the
   gRPC-Web `FirestoreServer` inside `HybridService::new`.
3. `crates/embyr-server/src/admin/router.rs` — `MAX_ADMIN_BODY_BYTES` constant; `DefaultBodyLimit`
   layer on the returned admin `Router`.
4. `docs/product/architecture/adr-081-request-body-size-limits.md` — new (this wave).
5. `docs/product/production-readiness-audit-2026-09-08.md` — finding #24 row to be updated to
   CLOSED at FINALIZE (not this wave).

**Files confirmed to need NO change:**
- No `Cargo.toml` changes anywhere — zero new dependencies (tonic's `.max_decoding_message_size`
  and axum's `DefaultBodyLimit` are both already part of already-pinned crates).
- `crates/embyr-server/src/admin/middleware/stripe_signature.rs` — untouched; its own ADR-070
  mechanism is independent and unaffected (confirmed above).

**Regression guards DISTILL/DELIVER must prove, against a real running server instance:**
- A gRPC request with a decoded message over 10 MiB is rejected (tonic's standard
  `RESOURCE_EXHAUSTED` status) on BOTH :8080 and :8081 gRPC-Web; a request at/under 10 MiB
  succeeds.
- A REST request (`accounts:*` route) with a body over 2 MiB is rejected with `413 Payload Too
  Large`; a normally-sized auth payload succeeds unchanged.
- An admin JSON route request with a body over 1 MiB is rejected with `413`; a normally-sized
  admin config payload succeeds unchanged; the Stripe webhook route's own 5 MiB ceiling and
  413/401 discrimination (ADR-070) remain unaffected by the new admin-level layer.
- Full workspace `cargo test` run once at the pre-commit gate (per root `CLAUDE.md` token
  discipline) — zero regressions expected per the Self-Review above.
- Mutation testing on the new constant comparisons after DELIVER, per this repo's `per-feature`
  strategy.

**Open items for DISTILL**: none — mechanism, exact ceilings, and call sites are all locked
above; no ambiguity carried forward.
