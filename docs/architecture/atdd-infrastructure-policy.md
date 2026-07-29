# ATDD Infrastructure Policy — embyr-rs

Per `nw-distill` § Project Infrastructure Policy. One file per project. Apply-if-exists; write-if-absent;
rewrite with `--policy=fresh`. Git history is the audit trail.

> Bootstrapped: 2026-05-24 (DISTILL wave, feature embyr-rs)

---

## Driving

| Port | Mechanism | Note |
|------|-----------|------|
| gRPC data port (:8080) | In-process tonic test client via `Channel::from_shared` pointing at the test server started with `tokio::net::TcpListener::bind("127.0.0.1:0")` | Ephemeral port per test; no port conflicts in CI |
| REST / gRPC-Web port (:8081) | `reqwest::Client` (async) against an in-process Axum test server | Covers gRPC-Web, BrowserChannel, REST/JSON paths |
| Admin port (:9090) | `reqwest::Client` against the Axum admin server bound on an ephemeral port | Admin bearer token injected per test |
| Browser WASM SPA (`/admin/` path) | `reqwest::Client::get("/admin/")` against embyr-admin on ephemeral port; asserts 200 + HTML body contains WASM boot `<script>`; separate `#[test]` asserts `.wasm` file in `admin-ui/dist/` is < 5,000,000 bytes. Pure `update()` proptest runs via `cargo test` directly (no browser, no WASM needed). | Walking skeleton = HTTP probe + bundle size gate; TEA state machine = proptest on the pure function. Added: 2026-06-14 (DISTILL wave, feature user-admin-ui) |
| Agent gRPC (:9191) | In-process tonic mTLS test client; test CA + leaf certs generated via `rcgen` in `tests/common/tls.rs` | Client cert required; no-cert test asserts TLS failure |

---

## Driven internal (real)

| Port | Mechanism | Note |
|------|-----------|------|
| System Postgres (project metadata, auth, metrics) | `testcontainers-rs` `Postgres` image, fresh container per test module; `sqlx::PgPool` for migrations | `SYSTEM_DB_URL` injected from container; schema migrated via `sqlx::migrate!` |
| Customer Postgres (documents, transactions, tombstones, indexes) | Same `testcontainers-rs` pattern; separate container from system DB | Fresh per test; ensures tenant isolation; no shared state between test runs |

---

## Driven external / non-deterministic (fake)

| Port | Fake | Note |
|------|------|------|
| AWS Secrets Manager | LocalStack container (`testcontainers-rs` `LocalStack` image) for `@real_io` scenarios; `FakeSecretFetcher` (in-process `Arc<RwLock<HashMap>>`) for in-memory unit tests | LocalStack only in `@real_io` tagged tests; fake for fast unit/acceptance tests |
| GCP Secret Manager | GCP emulator (`localstack` or `fake-gcp-secretmanager` in-process server) for `@real_io` scenarios; `FakeSecretFetcher` for in-memory tests | Same dual-track pattern as AWS |
| embyr-agent (mTLS gRPC) | `MockAgentServer` — in-process tonic server with test TLS certs via `rcgen` | Implements `embyr.agent.v1.StorageAgent`; controllable via shared state for error injection |
| System clock / timestamps | `tokio::time::advance` (paused clock) for deterministic timestamp tests | Enabled per-test via `#[tokio::test(start_paused = true)]` |
