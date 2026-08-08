# ADR-017: Production Server Startup — ServerConfig and main() Implementation

## Status

Accepted

## Context

embyr-rs cannot start in production. Three blockers identified from codebase analysis (see
`docs/feature/production-readiness/feature-delta.md`):

1. `crates/embyr-server/src/main.rs` is a 3-line stub:
   `fn main() { println!("embyr-server starting"); }`. The real startup sequence
   (env config, tracing, prometheus, migrations, system DB probe, bind 3 ports, graceful
   shutdown) exists only in `lib.rs` as private test-only helper functions
   (`alloc_test_components`, `spawn_all_servers`).

2. No production startup path reads from environment variables or wires the full component
   graph for production use. `alloc_test_components` binds ephemeral ports (`127.0.0.1:0`)
   and is private to the crate.

3. No `ServerConfig` struct exists. Configuration values are either hard-coded in test
   helpers or read with scattered `std::env::var` calls in `lib.rs`
   (`default_rate_limit_capacity()`).

**Working reference pattern:** `crates/embyr-agent/src/main.rs` reads from
`AgentConfig::from_env()`, runs a startup probe, then calls `server::run(cfg)`.
`crates/embyr-agent/src/config.rs` collects all required env vars before returning,
accumulates errors, and provides typed defaults for optional vars. This is the established
pattern in the workspace.

**Prior ADR constraints that apply:**

- ADR-016 (Prometheus): startup ordering invariant —
  `install_recorder → migrate_db → run_probes → bind_listeners → serve`. Prometheus
  recorder must be installed before any TCP listener opens and before any metric can be
  emitted.

- Architecture brief § Substrate Probes: `System DB connectivity`, `System DB schema
  migration`, `Port availability`, `Admin key present` are all hard gates that must run
  before accepting traffic. `SystemDb::new()`, `SystemDb::migrate()`, and
  `SystemDb::probe()` are already implemented and public.

- Architecture brief § Operational Simplicity (rank 6): single binary, three TCP listeners,
  no coordination plane. The production path must not add new infrastructure dependencies.

- admin-api-v2 startup extension: `EMBYR_ENCRYPTION_KEY` must be validated as exactly 32
  bytes (64 hex chars). If accounts table is non-empty and key is absent, refuse to start.

**Relevant locked decisions from DISCUSS wave:**

- D-PR-1: env vars `DATABASE_URL` (required), `EMBYR_ADMIN_KEY` (required),
  `EMBYR_ENCRYPTION_KEY` (32-byte hex, required), `EMBYR_RATE_LIMIT_RPS` (default 1000.0),
  `GRPC_PORT` (default 8080), `REST_PORT` (default 8081), `ADMIN_PORT` (default 9090),
  `RUST_LOG` (default "info").
- D-PR-2: init order: tracing → prometheus → migrations → probe SystemDb → bind ports →
  spawn_all_servers.
- D-PR-5: graceful shutdown on SIGTERM/SIGINT via `tokio::signal`; uses the existing
  oneshot pattern from `TestServer.shutdown_tx`.
- D-PR-6: exit code 1 on any required var missing, probe failure, or port bind failure.
  No partial startup — all three ports or none.
- D-PR-7: config struct in `crates/embyr-server/src/config.rs` (new file).

## Decision

**Introduce `crates/embyr-server/src/config.rs` with `ServerConfig::from_env()`, add a
public `alloc_production_components()` function to `lib.rs`, make `spawn_all_servers`
public, and implement a real production `#[tokio::main]` in `main.rs`.**

Three files change. No new workspace dependencies are introduced.

---

### Component 1: `crates/embyr-server/src/config.rs` (NEW)

`ServerConfig` struct with all production configuration. Mirrors `AgentConfig` in structure:

```
pub struct ServerConfig {
    pub db_url: String,            // DATABASE_URL — required, non-empty
    pub admin_key: String,         // EMBYR_ADMIN_KEY — required, non-empty
    pub encryption_key: [u8; 32], // EMBYR_ENCRYPTION_KEY — 64-char hex, required
    pub rate_limit_rps: f64,       // EMBYR_RATE_LIMIT_RPS — default 1000.0, >0 finite
    pub grpc_port: u16,            // GRPC_PORT — default 8080
    pub rest_port: u16,            // REST_PORT — default 8081
    pub admin_port: u16,           // ADMIN_PORT — default 9090
    pub log_level: String,         // RUST_LOG — default "info"
}
```

`from_env() -> Result<Self, ConfigError>` accumulates ALL errors before returning (same
pattern as `AgentConfig::from_env()`). The operator sees every missing or invalid variable
in one error message.

Typed error enum:

```
pub enum ConfigError {
    MissingVars(Vec<String>),
    InvalidEncryptionKey { reason: String },
    InvalidPort { var: String, value: String },
    InvalidRateLimitRps { value: String },
}
```

`impl fmt::Display for ConfigError` produces human-readable messages written to stderr.

`EMBYR_ENCRYPTION_KEY` parsing: the value must be exactly 64 hexadecimal characters. Decoded
as `[u8; 32]` using `hex::decode` at parse time. Length ≠ 64 or non-hex → `ConfigError::
InvalidEncryptionKey`.

`EMBYR_RATE_LIMIT_RPS` parsing: must be finite and > 0.0. Invalid or non-positive →
`ConfigError::InvalidRateLimitRps`. Fallback to 1000.0 only when absent (not set).

Port validation: `parse::<u16>()` failure for `GRPC_PORT`, `REST_PORT`, or `ADMIN_PORT` →
`ConfigError::InvalidPort`.

**No new crate dependency.** Uses `std::env::var` and `hex::decode`. The `hex` crate is
already a transitive dependency in the workspace (via `blake3`, `x25519-dalek`). Confirm
it appears in `cargo metadata` before adding it explicitly.

**Not using the `config` crate** (despite `config.workspace = true` in `Cargo.toml`). The
`config` crate provides TOML + env layering that is unnecessary when all configuration is
environment-variable-only. The agent pattern with explicit `collect_required` /
`parse_optional_*` helpers is cleaner and produces better per-variable error messages.

---

### Component 2: `crates/embyr-server/src/lib.rs` (MODIFY — small)

Two minimal changes:

**Change 2a — add `pub fn alloc_production_components`.**

New public function alongside the existing private `alloc_test_components`. Takes
`system_db: Arc<SystemDb>` and `config: &ServerConfig`. Returns `ProductionComponents`:

```
pub struct ProductionComponents {
    pub cache: Arc<CredentialCache>,
    pub idx_mgr: Arc<IndexManager>,
    pub metrics: Arc<MetricsAdapter>,
    pub listen_registry: Arc<ListenRegistry>,
    pub active_listeners: Arc<tokio::sync::Mutex<HashMap<String, PostgresNotifyListener>>>,
    pub rate_limiter: Arc<RateLimiter>,
    pub shutdown_tx: tokio::sync::oneshot::Sender<()>,
    pub shutdown_rx: tokio::sync::oneshot::Receiver<()>,
}
```

Implementation:

1. Spawn pool gauge background task (identical to `alloc_test_components`).
2. Create `CredentialCache::new(256)`.
3. Create `IndexManager::new(system_db.pool().clone())`.
4. Create `MetricsAdapter::new(system_db.pool().clone())`.
5. Create `ListenRegistry::new()`.
6. Create `RateLimiter::with_pg(config.rate_limit_rps, config.rate_limit_rps, system_db.pool().clone())`.
7. Create `tokio::sync::oneshot::channel()` for shutdown.

**RateLimiter parameter semantics:** `RateLimiter::with_pg(capacity, refill_rate, pg_pool)`
receives `config.rate_limit_rps` for **both** `capacity` and `refill_rate`. This is
intentional and matches the pattern in all existing test server constructors (e.g.,
`start_test_server_with_keepalive` calls `RateLimiter::new(rate_limit_rps, rate_limit_rps)`).
The architecture brief § Rate Limiting documents this decision:
> "The same value is used for both `capacity` (burst size) and `refill_rate` (tokens/second),
> giving a 1-second window token bucket."
The single `EMBYR_RATE_LIMIT_RPS` env var controls both parameters simultaneously. No
separate burst-size override exists by design (D4 locked decision — no per-project
granularity; operator-wide uniform rate limiting).

**Change 2b — make `spawn_all_servers` public.**

Single keyword change: `fn spawn_all_servers(...)` → `pub fn spawn_all_servers(...)`.

Zero impact on existing tests (tests call `start_test_server*` constructors, not
`spawn_all_servers` directly).

No other changes to `lib.rs`. All `start_test_server_*` constructors remain unchanged.
`alloc_test_components` remains private.

---

### Component 3: `crates/embyr-server/src/main.rs` (REPLACE)

Real production startup. Annotated sequence:

```
Step 1:  ServerConfig::from_env()
         → on Err: eprintln!("embyr-server: {e}") + std::process::exit(1)
         [Before tracing: config provides log_level; agent pattern does config first]

Step 2:  tracing_subscriber::fmt()
             .with_env_filter(
                 EnvFilter::try_from_default_env()
                     .unwrap_or_else(|_| EnvFilter::new(&config.log_level))
             )
             .with_writer(std::io::stderr)
             .init()

Step 3:  observability::get_or_install_prometheus_handle()
         [Must be before any TCP listener — ADR-016 ordering constraint]

Step 4:  SystemDb::new(&config.db_url).await
         → on Err: tracing::error!(error=%e, "startup failed: system DB connection"); exit(1)

Step 5:  system_db.migrate().await
         → on Err: tracing::error!(error=%e, "startup failed: migration"); exit(1)

Step 6:  system_db.probe().await
         → on Err: tracing::error!(error=%e, "startup probe failed"); exit(1)

Step 7:  alloc_production_components(Arc::clone(&system_db), &config)
         → returns ProductionComponents { cache, idx_mgr, metrics, listen_registry,
                                          active_listeners, rate_limiter, shutdown_tx, shutdown_rx }

Step 8:  TcpListener::bind(format!("0.0.0.0:{}", config.grpc_port)).await
         TcpListener::bind(format!("0.0.0.0:{}", config.rest_port)).await
         TcpListener::bind(format!("0.0.0.0:{}", config.admin_port)).await
         → on any Err: tracing::error!(port=..., "startup failed: port bind"); exit(1)
         [All three ports or none — D-PR-6]

Step 9:  Build FirestoreService { system_db, credential_cache: components.cache,
             index_manager: components.idx_mgr, metrics_adapter: components.metrics,
             keepalive_interval: Duration::from_secs(30),
             listen_registry: components.listen_registry,
             active_listeners: components.active_listeners,
             aws_secret_fetcher: None, gcp_secret_fetcher: None,
             rate_limiter: components.rate_limiter }

Step 10: Build admin_app via admin::router::build_admin_router(
             system_db, config.admin_key.clone(), cache_for_admin,
             config.encryption_key, Arc::new(NoopEmailSender),
             None, None, config.rate_limit_rps,
             observability::get_or_install_prometheus_handle() )

Step 11: spawn_all_servers(grpc_listener, rest_listener, admin_listener,
                           service, admin_app, components.shutdown_rx)

Step 12: tracing::info!(
             grpc = %format!("0.0.0.0:{}", config.grpc_port),
             rest = %format!("0.0.0.0:{}", config.rest_port),
             admin = %format!("0.0.0.0:{}", config.admin_port),
             "embyr-server ready"
         )

Step 13: Signal handling — tokio::select! on:
             tokio::signal::ctrl_c() — keyboard interrupt
             tokio::signal::unix::signal(SignalKind::terminate()) — SIGTERM (Kubernetes)
         → on either: let _ = components.shutdown_tx.send(());
                       tracing::info!("shutdown signal received, draining...")

Step 14: tracing::info!("embyr-server stopped")
         [Process exits normally after spawn_all_servers completes drain]
```

**Signal handling note:** `tokio::signal::unix` is Linux/macOS only and is appropriate
because `debian:bookworm-slim` (the Docker runtime stage) is Linux. The `ctrl_c()` handler
covers Ctrl+C in development.

**Drain behavior:** `spawn_all_servers` uses `tokio::select!` on the shutdown oneshot. When
`shutdown_tx` fires, the select arm `_ = async { let _ = shutdown_rx.await; } => {}` cancels
the other arms (gRPC, REST, admin). In-flight requests that are already executing continue
until their handlers return; new accepts are stopped. This matches the architecture brief
"graceful if drain is configured" note. Kubernetes sends SIGTERM then waits
`terminationGracePeriodSeconds` (default 30s) before SIGKILL — compatible with this design.

---

### Config module location and lib.rs registration

`config.rs` is added to `crates/embyr-server/src/` and registered in `lib.rs` with
`pub mod config;` so both `main.rs` and any test code can reference `ServerConfig` and
`ConfigError`.

`main.rs` imports `use embyr_server::{config::ServerConfig, admin, observability, ...}`.

---

## Alternatives Considered

### Alternative 1: Make `alloc_test_components` public, call it from main (rejected)

`alloc_test_components` binds ephemeral ports (`127.0.0.1:0`) and uses zero-valued
`encryption_key` and hard-coded `"test-admin-key-secret"`. Making it public would:
- Permanently conflate test infrastructure with production paths
- Require callers to override test defaults, creating two competing config paths
- Risk test code accidentally calling the production path or vice versa

The separate `alloc_production_components` function preserves the invariant that
`alloc_test_components` is exclusively for test servers.

### Alternative 2: Scatter env var reads in main.rs without a config struct (rejected)

Reading `std::env::var("DATABASE_URL")` directly in `main()` fails on the first missing
variable, forcing operators to fix one variable at a time and re-run. The `AgentConfig`
accumulator pattern (collect all errors, report all at once) is already established in the
workspace and produces a much better operator experience. Additionally, a config struct is
unit-testable in isolation; raw `env::var` calls in `main()` are not.

### Alternative 3: Use the `config` crate for TOML + env layering (rejected)

`config = "0.14"` is a workspace dependency. Its hierarchical layering (TOML file →
environment → defaults) is valuable when multiple deployment environments each have a
configuration file. embyr-server has no configuration file — all configuration is purely
environment-variable-driven (same as embyr-agent). The `config` crate would add indirection
with no benefit. The agent's direct `std::env::var` approach is simpler, produces better
per-variable error messages, and is already proven in the workspace.

### Alternative 4: Use a separate `embyr-config` workspace crate (rejected)

Extracting config into a shared crate would be useful if multiple binaries needed the same
configuration. `embyr-agent` has `AgentConfig`; `embyr-server` will have `ServerConfig`.
These two configs are intentionally separate — the agent does not need database migration
control, and the server does not need TLS cert paths. A shared config crate would create
false coupling. Keep config local to each binary.

## Consequences

### Positive

- `cargo run -p embyr-server` becomes a deployable production binary.
- `docker run embyr-server` (after US-PR-02 Dockerfile) starts the full server.
- `ServerConfig::from_env()` is unit-testable without starting any server or touching env.
- All startup errors reported upfront — operator sees every missing var simultaneously.
- Startup sequence matches ADR-016 ordering constraint (`install_recorder` before listeners).
- Substrate probes run before any port accepts traffic (architecture brief invariant).
- SIGTERM handling is compatible with Kubernetes rolling deployment (30s drain window).
- Existing tests are unaffected — `alloc_test_components` and all `start_test_server_*`
  constructors remain unchanged.

### Negative / Trade-offs

- `lib.rs` grows by one public function (`alloc_production_components`) and one public
  struct (`ProductionComponents`). The `embyr-server` library API expands. This is
  acceptable because `embyr-server` is a binary crate used only within the workspace.
- `main.rs` contains wiring code (Steps 9–10) that partially overlaps with
  `start_test_server*` constructors in `lib.rs`. This duplication is intentional: test
  constructors hard-code test values; `main.rs` reads from `ServerConfig`. Abstracting the
  wiring into yet another helper would add indirection without benefit.
- `NoopEmailSender` is used in V1 production (invitation emails are no-ops). This is a
  known limitation from admin-api-v2; V2 will add `SmtpEmailSender` as an injected port
  without changing `main()`'s structure.
- `EMBYR_ENCRYPTION_KEY` is always required at startup even if the `accounts` table is
  empty. This is slightly stricter than the admin-api-v2 startup extension (which allows
  a warn-only path on empty accounts). The production-readiness feature treats it as
  required for simplicity (D-PR-1); operators must set it before deploying.

## Enforcement

- Unit tests in `crates/embyr-server/src/config.rs` (`#[cfg(test)]` module) assert that:
  - All three required vars missing → `ConfigError::MissingVars` with all three names
  - `EMBYR_ENCRYPTION_KEY` with 63 hex chars → `ConfigError::InvalidEncryptionKey`
  - `GRPC_PORT=999999` → `ConfigError::InvalidPort` (out of u16 range)
  - All required vars set correctly → `Ok(ServerConfig { ... })`
- US-PR-01 integration test (acceptance) starts `embyr-server` with required env vars
  and asserts `GET :{admin_port}/healthz` returns HTTP 200 within 5 seconds.
- US-PR-01 integration test asserts exit code 1 and stderr content when `DATABASE_URL` is
  absent, when `EMBYR_ADMIN_KEY` is absent, and when `EMBYR_ENCRYPTION_KEY` is 32 chars
  (too short).
- CI `test` job (`cargo test --workspace`) exercises both unit tests and integration tests.
- CI `lint` job (`cargo clippy -- -D warnings`) prevents silent mistakes in `main()`.
