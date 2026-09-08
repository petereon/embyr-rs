# Feature Delta: firestore-tls-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-13 (`production-deployment`, persona P2 Sam Chen) — read in
full.
✓ `docs/product/known-gaps.md` #5 — "No TLS/mTLS on any of the 3 listeners (:8080 gRPC, :8081
REST/gRPC-Web, :9090 Admin)... Depends on deploy topology (fine if a TLS-terminating LB sits in
front)... Blocks production unless mitigated at the LB."
✓ `crates/embyr-server/src/main.rs` — full startup sequence read. Confirmed 3 plain-TCP listener
binds (`bind_or_exit`), and confirmed graceful shutdown (SIGTERM/Ctrl-C + drain) is ALREADY fully
implemented here (see `docs/product/known-gaps.md` gap #6 correction, this session) — irrelevant
to this feature except as a reminder that `main.rs`'s startup sequence comment block (numbered
steps 1-14) must be kept in sync with any new step this feature inserts.
✓ `crates/embyr-server/src/lib.rs::spawn_all_servers` — the gRPC listener already uses
`tonic::transport::Server::builder()` directly (not a bind-based `.serve()`, but
`.serve_with_incoming_shutdown()` against an existing `TcpListenerStream`); the admin listener
uses `axum::serve(admin_listener, admin_app)` directly.
✓ `crates/embyr-server/src/rest/grpc_web.rs::spawn_hybrid_server` — the REST/gRPC-Web listener
does NOT use `axum::serve` at all. It already runs a manual accept loop: `listener.accept()` →
`TokioIo::new(stream)` → `hyper_util::server::conn::auto::Builder` → `serve_connection_with_
upgrades`. This is the exact mechanism a TLS-wrapping step needs to slot into — no new listener
architecture required, just one extra step (`tls_acceptor.accept(stream).await?`) between
`accept()` and `TokioIo::new()`.
✓ `crates/embyr-server/Cargo.toml` — confirmed `tonic = { features = ["tls"] }` is ALREADY
enabled (zero new dependency for the gRPC listener's own TLS termination — tonic's native
`ServerTlsConfig`/`Identity` handles this, and applies automatically to `serve_with_incoming*`
just as much as bind-based `serve()`, per tonic's own documented behavior). Also confirmed
`rustls`, `tokio-rustls`, and `rustls-pemfile` are ALREADY workspace dependencies (currently used
only by `stripe_gateway.rs`'s own outbound TLS client config) — the exact building blocks a
manual TLS-wrapping accept loop for the REST/admin listeners needs, with zero new dependency.
✓ `crates/embyr-server/src/config.rs` — read the full `ServerConfig` struct and its `from_env()`
validation pattern (required vars fail fast with a named error; optional vars default; the 3 port
binds are already an established "all or none" pattern via `D-PR-6`). This feature's own
cert/key path config follows the identical shape.

**No live web verification needed this DISCUSS.** TLS termination for gRPC (tonic native) and for
plain HTTP servers (manual rustls-wrapped accept loop) are standard, uncontroversial mechanisms —
not a contested Firestore-semantics question like several prior features this session.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Infrastructure** (Decision 1) — no user-facing SDK-visible behavior change; a
  deployment/operations capability.
- JTBD: **reuse JOB-13** (`production-deployment`, P2 Sam Chen) — TLS termination is a natural
  extension of "run embyr in production correctly," the same job the graceful-shutdown/Dockerfile/
  CI work (`production-readiness`) already serves.
- Walking Skeleton: **Yes** — plain server-TLS (no mTLS) on all 3 listeners, opt-in via config,
  proven end-to-end with a real TLS handshake against each listener.
- UX Research Depth: **Lightweight** — an operator-facing capability with an established persona
  (Sam Chen) and job (JOB-13); no new emotional arc needed beyond what JOB-13 already documents.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P2 Sam Chen (Service Operator / Platform Engineer), unchanged.

**Job**: JOB-13 `production-deployment`, unchanged job_story. This feature's own realization: Sam
deploying embyr into a network topology WITHOUT a TLS-terminating load balancer in front (a
common self-hosted or bare-metal deployment shape) today has no option to encrypt traffic to any
of the 3 listeners at all — client SDK traffic (API keys!), admin API traffic (admin key!), and
REST/gRPC-Web browser traffic all travel in plaintext. After this feature, Sam can point 2
config vars at a PEM cert/key pair and get TLS on all 3 listeners; omitting them preserves
today's plaintext behavior byte-for-byte (correct default for LB-fronted deployments, per the
gap's own severity note).

## Wave: DISCUSS / [REF] Scope Assessment: PASS

3 listeners, 1 shared config surface (2 new env vars), reuses existing dependencies entirely (no
new crates). Effort well under 1 day. Not oversized.

## Wave: DISCUSS / [REF] User Stories

### US-01: Server-side TLS, opt-in via config, on all 3 listeners

**job_id**: JOB-13

**Elevator Pitch**
- **Before**: `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH` don't exist; embyr always serves plain
  TCP on all 3 ports; a self-hosted deployment without a fronting LB has no in-process option to
  encrypt any traffic (API keys, admin key, and browser traffic all travel in plaintext).
- **After**: setting both `EMBYR_TLS_CERT_PATH` and `EMBYR_TLS_KEY_PATH` to a PEM cert chain and
  private key makes all 3 listeners (`:8080` gRPC, `:8081` REST/gRPC-Web, `:9090` Admin) require
  and correctly terminate TLS — a client connecting with `grpc.Channel(..., credentials=grpc.
  ssl_channel_credentials())` (or an admin `curl --cacert`) gets a valid, working encrypted
  connection. Leaving both unset preserves today's plaintext behavior exactly.
- **Decision enabled**: Sam can choose, per deployment, whether embyr terminates TLS itself
  (self-hosted, no LB) or relies on a fronting LB (today's default, unchanged) — without either
  choice requiring a code change.

**Acceptance Criteria**
- **AC-TLS-01**: With `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH` unset, all 3 listeners serve
  plain TCP exactly as before (regression guard — this feature must not change default behavior).
- **AC-TLS-02**: With both vars set to a valid PEM cert/key pair, a gRPC client using TLS
  transport credentials against `:8080` completes a real RPC successfully.
- **AC-TLS-03**: With both vars set, an HTTPS client against `:9090/healthz` (Admin) completes
  successfully.
- **AC-TLS-04**: With both vars set, an HTTPS client against `:8081/healthz` (REST/gRPC-Web)
  completes successfully.
- **AC-TLS-05**: With exactly ONE of the two vars set (not both, not neither), the server fails
  fast at startup with a named config error — mirrors the existing "all or none" pattern already
  used for the 3 port binds (`D-PR-6`), applied here to the 2 TLS vars.
- **AC-TLS-06**: With both vars set but pointing at a nonexistent file or unparseable PEM content,
  the server fails fast at startup with a named config error, not a panic or a silent fallback to
  plaintext.

## Wave: DISCUSS / [REF] Definition of Done

1. All 6 ACs proven with real TLS handshakes (not mocked) against each listener.
2. Default (unset) behavior unchanged — proven by a regression test, not just reasoning.
3. Zero new dependencies (tonic's own `tls` feature already enabled; `rustls`/`tokio-rustls`/
   `rustls-pemfile` already workspace deps).
4. Full regression suite clean (pre-existing flakes excepted, triaged not assumed).
5. Mutation testing: 0 missed on all viable mutants.
6. Evolution doc written, `known-gaps.md` #5 updated to CLOSED.
7. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **mTLS (client certificate verification)** — the gap's own severity reasoning is entirely about
  plain server-side TLS ("blocks production unless mitigated at the LB"); mTLS is named in the
  gap's TITLE but not its actual severity argument. No evidenced customer need for client-cert
  verification specifically, and it adds real operational complexity (cert issuance/distribution/
  rotation for every client) this feature's own scope does not need to solve. Deferred as a
  separate, future feature if evidence emerges.
- **Cert rotation / hot-reload** — TLS cert/key are read once at startup (`ServerConfig::from_
  env()`), matching every other secret this config already loads once at boot
  (`EMBYR_ADMIN_KEY`, `EMBYR_ENCRYPTION_KEY`). Rotating a TLS cert requires a restart, same as
  rotating any other credential today — not a new limitation this feature introduces.
- **Per-listener TLS configuration** — one cert/key pair serves all 3 listeners (the simplest,
  laziest-correct design: a single TLS identity for the whole server process). Per-listener
  distinct certs would be pure speculative flexibility with no named need.
- **ACME / Let's Encrypt automation** — out of scope; Sam provides pre-issued PEM files, matching
  how every other secret in this config is already sourced (env var pointing at a resolved
  value, not automated issuance).

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (direct implementation) — reuses `spawn_hybrid_server`'s own already-established
manual accept-loop pattern and tonic's own native `tls_config()`; no new architecture.

## Wave: DISCUSS / [REF] Driving Ports

The same 3 existing listeners (`:8080`, `:8081`, `:9090`) — this feature changes HOW they
terminate connections, not what they expose.

## Wave: DISCUSS / [REF] Pre-requisites

None beyond what already exists in the dependency graph.

---

## Wave: DESIGN

### D1 — `ServerConfig` gains 2 new optional fields

```rust
// crates/embyr-server/src/config.rs
pub struct ServerConfig {
    // ...existing...
    /// `EMBYR_TLS_CERT_PATH` — path to a PEM cert chain; both this and
    /// `tls_key_path` must be set together, or both absent (fail-fast,
    /// mirrors D-PR-6's "all or none" pattern for the 3 port binds).
    /// `None` (both absent) means plain TCP on all 3 listeners — today's
    /// unchanged default, correct for LB-terminated deployments.
    pub tls_cert_path: Option<String>,
    /// `EMBYR_TLS_KEY_PATH` — path to a PEM private key. See `tls_cert_path`.
    pub tls_key_path: Option<String>,
}
```

`from_env()` gains: read both vars as `Option<String>`; if exactly one is `Some`, return a new
`ConfigError::TlsConfigIncomplete` naming which var is missing (AC-TLS-05). If both `Some`,
eagerly read and parse the PEM files here (not deferred to listener-bind time) so a bad path or
malformed PEM fails fast at the SAME startup stage as every other config error (AC-TLS-06),
mirroring `validate_encryption_key_hex`'s own eager-validation pattern.

### D2 — A single shared `TlsMaterial` type, resolved once, used by all 3 listeners

```rust
// crates/embyr-server/src/config.rs (or a new tls.rs — small enough to keep in config.rs)
pub struct TlsMaterial {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
}
```

Read once in `from_env()` as raw PEM bytes (not yet parsed into a `rustls::ServerConfig` or
tonic `Identity` — each of the 2 consuming mechanisms, tonic-native and manual-rustls, wants a
different type built FROM these same bytes; parsing once into a mechanism-specific type here
would force one to convert back). `main.rs` builds each mechanism-specific type from the same
`TlsMaterial` at the point of use (Step 8, alongside the existing listener binds).

### D3 — gRPC listener: tonic's own native `tls_config`

```rust
// crates/embyr-server/src/lib.rs, inside spawn_all_servers's gRPC task, before .add_service()
let mut builder = tonic::transport::Server::builder();
if let Some(tls) = &tls_material {
    let identity = tonic::transport::Identity::from_pem(&tls.cert_pem, &tls.key_pem);
    builder = builder
        .tls_config(tonic::transport::ServerTlsConfig::new().identity(identity))
        .expect("TLS identity already validated at startup config time");
}
let grpc_fut = builder
    .add_service(FirestoreServer::new(service))
    .serve_with_incoming_shutdown(grpc_incoming, ...);
```

### D4 — REST + Admin listeners: extend the existing manual accept loop with an optional
`tokio_rustls::TlsAcceptor`

`spawn_hybrid_server` (REST/gRPC-Web) already manually accepts connections. Add one optional
`Option<Arc<rustls::ServerConfig>>` parameter; when `Some`, wrap each accepted stream:

```rust
// crates/embyr-server/src/rest/grpc_web.rs — inside the accept loop, before TokioIo::new()
let stream: Box<dyn TokioAsyncReadWrite> = match &tls_acceptor {
    Some(acceptor) => match acceptor.accept(stream).await {
        Ok(tls_stream) => Box::new(tls_stream),
        Err(_) => continue, // failed handshake — drop this connection, keep accepting
    },
    None => Box::new(stream),
};
let io = TokioIo::new(stream);
```

(Exact boxing/trait-object shape decided during DELIVER — the key structural decision, locked
here, is: ONE optional wrapping step in the existing loop, no new accept-loop architecture.)

The Admin listener (`axum::serve(admin_listener, admin_app)` in `lib.rs`) does NOT have a manual
accept loop today — `axum::serve` owns it internally and has no TLS hook. **Decision**: migrate
the Admin listener to the SAME manual accept-loop shape as `spawn_hybrid_server` (a new,
smaller `spawn_axum_server(listener, app, tls_acceptor)` in `rest/grpc_web.rs` or a shared
location), rather than adding a new dependency (`axum-server`) for just this one listener. This
keeps the TLS-wrapping logic in exactly one place for both axum-based listeners, avoiding a
second, differently-shaped mechanism that could drift out of sync — a real security-sensitive
concern DRY genuinely matters for here.

### D5 — `main.rs` gains one step: build TLS material once, thread it to both listeners

Step 8's own doc comment (`TcpListener::bind × 3 — all three ports or none`) gains a note that
TLS identity construction happens here too, immediately after config resolution and before
`spawn_all_servers` is called — `tls_material: Option<TlsMaterial>` becomes a new parameter (or 2:
one native-tonic `Option<Identity>`, one rustls `Option<Arc<ServerConfig>>` — exact split decided
during DELIVER based on what's cheapest to construct once vs. per-consumer).

## Wave: DESIGN / Handoff Package

5 files touched: `config.rs` (2 new fields + validation), `lib.rs` (`spawn_all_servers` threads
TLS material to gRPC + admin), `rest/grpc_web.rs` (`spawn_hybrid_server`'s accept loop gains
optional TLS wrapping; new `spawn_axum_server` for the Admin listener reusing the same
mechanism), `main.rs` (build TLS material once, wire through). Zero new dependencies. Zero
security-critical compliance-check surface touched (this feature is transport-layer only, does
not interact with `filter_binds_field_to_uid` or any access-control logic).
