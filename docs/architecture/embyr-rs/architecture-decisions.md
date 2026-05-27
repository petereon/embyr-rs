# DESIGN Decisions — embyr-rs

> Wave: DESIGN / Application Architecture
> Updated: 2026-05-23
> Architect: Morgan (nw-solution-architect)
> Builds on: System Architecture (brief.md §§ System Architecture) + Domain Model (brief.md §§ Domain Model)

---

## Key Decisions

| ID | Decision | Verdict | ADR / Brief Ref |
|----|----------|---------|----------------|
| AD-01 | Cargo workspace with five crates (`embyr-proto`, `embyr-core`, `embyr-server`, `embyr-admin`, `embyr-agent`) | Accepted | `brief.md` §§ Application Architecture / Component Decomposition |
| AD-02 | `BackendAdapter` trait defined in `embyr-core` (not `embyr-server`) | Accepted | `brief.md` §§ Driven Ports |
| AD-03 | Auth middleware runs per-request; credential cache caches the resolved adapter, not the auth decision | Accepted | `brief.md` §§ Driving Ports |
| AD-04 | `axum` for both REST port and admin port (Tower-compatible, shares middleware with Tonic) | Accepted | `brief.md` §§ Technology Choices |
| AD-05 | `sqlx` with compile-time query checking over `diesel` or raw `tokio-postgres` | Accepted | `brief.md` §§ Technology Choices |
| AD-06 | `cargo-deny` + custom proc-macro + CI behavioral tests for adapter boundary enforcement (three orthogonal layers) | Accepted | `brief.md` §§ Driven Ports (Probe enforcement) |
| AD-07 | `tracing` with Tokio console support over `log` | Accepted | `brief.md` §§ Technology Choices |
| AD-08 | Dedicated Postgres connection per active project for NOTIFY (not from connection pool) | Accepted | `brief.md` §§ Driven Ports (`NotifyListener`) |
| AD-09 | BrowserChannel handler in `embyr-server::rest` (not a separate crate) | Accepted | `brief.md` §§ Application-Level Decisions Table |
| AD-10 | `google-cloud-secretmanager` community crate with `reqwest` fallback design | Accepted | `brief.md` §§ Application-Level Decisions Table |
| ADR-003 | Tokio 1.x multi-thread scheduler with `spawn_blocking` for Argon2id | Accepted | `adr-003-async-runtime.md` |
| ADR-004 | `tonic` 0.12.x + `tonic-web` for gRPC + gRPC-Web; Axum for REST transcoding | Accepted | `adr-004-grpc-framework.md` |

---

## Architecture Summary

embyr-rs is implemented as a Cargo workspace of five crates, following a hexagonal (ports-and-adapters) pattern enforced by crate-level dependency boundaries.

**Dependency hierarchy (enforced by Cargo, not convention):**

```
embyr-proto     (generated stubs — no domain logic)
      ↑
embyr-core      (pure domain logic + port trait definitions — no IO imports)
      ↑
embyr-server    (composition root: wires adapters, opens 3 TCP listeners)
embyr-admin     (admin HTTP server on :9090 — imports embyr-core, not embyr-server internals)
embyr-agent     (separate binary — imports embyr-core storage traits only)
```

`embyr-core` imports: `std`, `thiserror`, `serde`, `embyr-proto` (for proto value types only), `argon2`, `blake3`, RustCrypto primitives. No `tokio`, `sqlx`, `tonic`, or `axum`.

**Development paradigm:** Functional-where-practical Rust. Pure transformations for protocol encoding/decoding. Explicit `Result<T, E>` error types throughout. Shared mutable state confined to three structures (`CredentialCache`, `BrowserChannelSession` map, `ListenRegistry`), each behind `Arc<RwLock<T>>` or `Arc<Mutex<T>>`.

**Data-plane request lifecycle (hot path):**

```
Firebase SDK
  → TCP :8080 (gRPC) or :8081 (REST/gRPC-Web/BrowserChannel)
  → Auth Interceptor (Tokio Tower layer)
      [1] Load Project from System DB
      [2] Check ProjectStatus (Deleted → NOT_FOUND, Suspended → PERMISSION_DENIED)
      [3] Argon2id verify (spawn_blocking) or cache hit
      [4] Rate limit check (in-process token bucket)
      [5] CredentialCache lookup → BackendAdapter
  → FirestoreHandler (dispatches by RPC method)
  → BC-2 Storage domain functions (pure) → BackendAdapter.execute()
  → Response
```

**Real-time delivery path (data plane subsystem — see C4 Component Diagram):**

```
BackendAdapter.execute() on write
  → Postgres commit + NOTIFY doc_changes_<project_id>
  → PostgresNotifyListener (dedicated connection, per-project Tokio task)
  → DocChange → ListenRegistry.fan_out()
  → Per-subscriber channel (capacity=64)
  → ListenHandler Tokio task (per active Listen stream)
      → in-memory query filter (embyr-core::realtime)
      → BackendAdapter.get_document() (re-fetch, handles 8 KB NOTIFY cap)
      → Tonic stream sender → Firebase SDK onSnapshot callback
```

---

## Architecture Constraints Established

The following constraints are established by the Application Architecture and binding on all subsequent DELIVER wave work:

| Constraint | Binding rule |
|-----------|-------------|
| `embyr-core` must have zero IO crate imports | Enforced by `cargo-deny` (`deny.toml`) and CI `cargo-depcheck` script |
| Every driven adapter must implement `probe()` | Enforced by Rust trait bound (compile-time), proc-macro AST check (pre-commit), CI behavioral test (runtime) |
| Argon2id verification must use `tokio::task::spawn_blocking` | Enforced by code review; documented as a mandatory pattern in implementer handoff |
| `LISTEN` connections must use a dedicated `PgConnection`, not `PgPool` | Enforced by code review; the `NotifyListener` adapter is structurally separate from `PostgresBackendAdapter` |
| Admin port (9090) must use a separate Axum `TcpListener` bind | Enforced structurally at composition root (`embyr-server/src/main.rs`) |
| Startup sequence: probe all adapters before opening any TCP listener | Enforced by composition root: `wire() → probe_all() → listen()` |
| BrowserChannel session state and ListenRegistry are in-process only | Locked decisions D09/D10; no Redis, no external store |

---

## Reuse Analysis

Greenfield codebase. All five crates are new. Third-party OSS reuse decisions:

| Concern | Selected library | License | Rejected alternatives |
|---------|----------------|---------|----------------------|
| gRPC server + codegen | `tonic` 0.12.x + `tonic-build` | MIT | `grpcio` (C binding, no Tokio native), `h2` raw (too low level), `tower-grpc` (unmaintained) |
| Async runtime | `tokio` 1.x | MIT | `async-std` (Tokio library incompatibility), `smol` (same), single-thread (CPU starvation) |
| HTTP framework | `axum` 0.7.x | MIT | `actix-web` (incompatible middleware model with Tonic Tower), `warp` (less active) |
| Postgres driver | `sqlx` 0.7.x | MIT/Apache 2.0 | `diesel` (ORM abstraction incompatible with JSONB ops), `tokio-postgres` raw (manual migration) |
| Argon2id | `argon2` (RustCrypto) | MIT/Apache 2.0 | Roll-own (security risk), C binding (OS dependency) |
| ECIES primitives | `x25519-dalek` + `hkdf` + `aes-gcm` | MIT/Apache 2.0 | OpenSSL crate (C dependency, linking complexity) |
| BLAKE3 | `blake3` | CC0/Apache 2.0 | SHA-256 (10× slower for cache key with no security benefit) |
| TLS | `rustls` + `tokio-rustls` | MIT/Apache 2.0 | OpenSSL (C dependency) |
| LRU cache | `lru` | MIT | `moka` (distributed; overkill for in-process LRU), roll-own |
| AWS secrets | `aws-sdk-secretsmanager` | Apache 2.0 | Direct HTTP (more complex SIGV4 signing) |
| GCP secrets | `google-cloud-secretmanager` | MIT | Direct HTTP (fallback option, OQ-01) |
| Config | `config` | MIT/Apache 2.0 | `figment` (smaller ecosystem), roll-own |
| Error types | `thiserror` | MIT/Apache 2.0 | Roll-own derive (equivalent complexity, no benefit) |
| Tracing | `tracing` + `tracing-subscriber` | MIT | `log` (no spans, no async context propagation) |
| Metrics | `metrics` + `metrics-exporter-prometheus` | MIT | Direct Prometheus client (higher coupling) |
| Dep enforcement | `cargo-deny` | MIT/Apache 2.0 | `import-linter` (Python only), `ArchUnit` (JVM only) |

All selected libraries are open source. No proprietary dependencies.

---

## Technology Stack

**Five-crate Cargo workspace:**

```
embyr-rs/
  Cargo.toml              (workspace root)
  crates/
    embyr-proto/          (generated proto stubs)
    embyr-core/           (domain logic + port traits — no IO)
    embyr-server/         (composition root + adapters — IO)
    embyr-admin/          (admin HTTP server)
    embyr-agent/          (customer-VPC agent binary)
  proto/                  (vendored .proto files from googleapis)
  migrations/             (system DB sqlx migrations)
```

**Runtime dependencies per binary:**

`embyr-server` binary: `tokio` + `tonic` + `tonic-web` + `axum` + `sqlx` + `argon2` + `blake3` + `x25519-dalek` + `hkdf` + `aes-gcm` + `rustls` + `tokio-rustls` + `lru` + `aws-sdk-secretsmanager` + `google-cloud-secretmanager` + `config` + `serde` + `serde_json` + `thiserror` + `tracing` + `tracing-subscriber` + `metrics` + `metrics-exporter-prometheus` + `cargo-deny` (build/CI).

`embyr-agent` binary: `tokio` + `tonic` + `sqlx` + `rustls` + `tokio-rustls` + `config` + `serde` + `thiserror` + `tracing` + `tracing-subscriber`.

`embyr-agent` is statically linked (`RUSTFLAGS="-C target-feature=+crt-static"` on Linux musl target) for deployment in customer VPCs without runtime dependencies.

---

## Upstream Changes

No changes to prior wave outputs are required. The Application Architecture builds on and is consistent with:

- **System Architecture decisions** (SD-01 through SD-10): all respected. The five-crate workspace implements the single-binary topology (SD-01), in-process Listen registry (SD-02), credential cache (SD-03), per-instance rate limiting (SD-04), NOTIFY fan-out (SD-05), OCC via `version` column (SD-06), separate admin port (SD-07), soft-delete (SD-08), separate agent binary (SD-09), and resume tokens (SD-10).
- **Domain Model decisions** (DD-01 through DD-09): all respected. The three bounded contexts map to `embyr-core::tenant`, `embyr-core::storage`, `embyr-core::realtime`. No Event Sourcing (DD-04), no CQRS (DD-05). `BackendConfig` is a value object in `embyr-core::tenant` (DD-06). `CredentialCache` is infrastructure in `embyr-server::adapters::cache` (DD-08).

**One clarification added (not a conflict):** The Domain Model established that `BackendAdapter` is "the coupling point between BC-1 and BC-2/BC-3" but did not specify which crate owns the trait definition. The Application Architecture resolves this: the trait is defined in `embyr-core::storage` (not in an infrastructure crate), ensuring that `embyr-core` domain functions can be written against it without any upward dependency on adapters.

---

## Open Questions (deferred to DELIVER)

| ID | Question | Blocks |
|----|----------|--------|
| OQ-01 | `google-cloud-secretmanager` crate stability vs. direct HTTPS fallback | S14 (cloud secrets) |
| OQ-02 | `tonic-web` gRPC-Web CORS edge cases for Firebase JS SDK | S12 (browser transport) |
| OQ-03 | Firebase JS SDK BrowserChannel undocumented protocol extensions | S12 (browser transport) |
| OQ-04 | `LISTEN` channel name length: `doc_changes_` + up to 63-char project_id = 75 chars (Postgres max 63). Recommendation: `dc_<BLAKE3_16hex>`. Confirm before S07. | S07 (NOTIFY fan-out) |
| OQ-05 | Operator SLA for cold-start Argon2id latency (first request after restart, ~200–500 ms) | Operational guidance |
| OQ-06 | Firestore conformance test suite availability (public or must be derived from Firebase SDK integration tests) | Acceptance test design |

---

## External Integrations Requiring Contract Tests

Two external API integrations are present. These should be annotated for contract testing in the CI pipeline:

- **AWS Secrets Manager** (`aws-sdk-secretsmanager`, `GetSecretValue` call): consumer-driven contract tests recommended to detect breaking changes in the AWS SDK or API response format. Recommended tool: integration tests against LocalStack in CI (LocalStack provides an AWS Secrets Manager emulator). Not Pact-style contracts — AWS is not a versioned consumer-driven API; functional integration tests against a local emulator are the appropriate mechanism.

- **GCP Secret Manager** (`google-cloud-secretmanager`, `AccessSecretVersion` call): same recommendation. Integration tests against the GCP Secret Manager emulator (`gcp-sdk-go` testkit or Testcontainers GCP module) in CI. Monitor the community crate for breaking API changes.
