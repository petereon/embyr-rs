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
| `embyr-server` binary (subprocess) | `std::process::Command::new(target/debug/embyr-server)` spawned with env vars; `reqwest::Client` polls `GET :{admin_port}/healthz → 200`; SIGTERM via `kill -TERM <pid>`. Testcontainers Postgres as the driven-internal port for the DB connection. | Added: 2026-08-08 (DISTILL wave, feature production-readiness). Tests live at `tests/production_readiness/`. Binary resolved from workspace `target/debug/` or `target/release/`. |
| `embyr-db-prep` binary (subprocess) | `std::process::Command::new(target/debug/embyr-db-prep)` spawned with env vars (`EMBYR_DB_PREP_DSN`, optionally `EMBYR_DB_PREP_DML_ROLE_DSN`); process is one-shot (exits after doing its work, does not stay up) — no healthz polling. Test waits on `Child::wait_with_output()` (or `.wait()` + captured stdio) and asserts on exit code + stdout/stderr content. Testcontainers Postgres as the driven-internal port for the DB connection (same container reused across a prep-then-verify scenario where applicable). | Added: 2026-08-16 (DISTILL wave, feature customer-db-onboarding). Direct extension of the `embyr-server` subprocess row's mechanism — same spawn/binary-resolution pattern, differs only in completion signal (process exit vs. healthz endpoint, since this binary is one-shot not a long-running server). Tests live at `tests/customer_db_onboarding/`. Binary resolved from workspace `target/debug/` or `target/release/`. |

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
| Stripe (Customer/Subscription/Invoice/UsageRecord API + webhooks) | **REAL** — real Stripe test-mode API (`sk_test_...`) over the network + Stripe CLI (`stripe trigger`) to synthesize webhook events. **No fake/mock.** | D-13 (LOCKED, feature `card-payments-backend`) explicitly overrides this table's default (fake for driven-external/non-deterministic ports) for Stripe specifically: "consistent with this codebase's stated preference for real-infrastructure integration tests over mocks (testcontainers Postgres, real subprocess servers elsewhere)." `StripeGateway` (`crates/embyr-server/src/adapters/stripe_gateway.rs`) has no trait interface — mirrors ADR-015's `RateLimiter` "concrete struct, one implementation" precedent. Key resolved via `tests/card_payments_backend/common/mod.rs::stripe_secret_key()` (process env, falling back to `.env.local` at the workspace root). Added: 2026-08-11 (DISTILL wave, feature card-payments-backend). |
