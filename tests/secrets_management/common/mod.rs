// SCAFFOLD: true
//! Common test infrastructure — secrets-management acceptance tests.
//!
//! Infrastructure policy (`docs/architecture/atdd-infrastructure-policy.md`):
//!   Driving port:    `embyr-server` binary subprocess (env-var driven), mirroring
//!                    `tests/production_readiness/common/mod.rs::ServerProcess`.
//!                    HTTP-shaped assertions (signin, operator routes, /metrics) use
//!                    `reqwest::Client` against the spawned process's admin port.
//!   Driven internal: testcontainers-rs Postgres 15-alpine; fresh per test.
//!   Driven external: LocalStack (`testcontainers::GenericImage`) for AWS Secrets
//!                    Manager `@real-io` scenarios — same pattern as
//!                    `tests/acceptance/us_10_aws_secrets.rs`.
//!                    GCP Secret Manager has NO subprocess-level real-I/O path today:
//!                    `GcpSecretFetcher::new(base_url, ...)` takes an explicit
//!                    `base_url` with no env-var override in `ServerConfig`
//!                    (see OQ-SM-4 / ADR-018 Alternatives A6). GCP scenarios that
//!                    would need this are `#[ignore]`d with `@requires_external`
//!                    until a follow-up feature adds the override knob.
//!
//! This harness is an independent copy of the `ServerProcess` shape already
//! established in `tests/production_readiness/common/mod.rs` — test suites in
//! this workspace do not cross-import harnesses (same precedent as
//! `tests/admin_api_v2/common/mod.rs` maintaining its own `AdminTestContext`
//! rather than reusing another suite's).
//!
//! Assertion-mode note (Mandate 8 / Layered Test Discipline): subprocess exit-code
//! and stderr-content scenarios (US-SM-01/02, config-validation shape) use plain
//! `assert_eq!`/`assert!`, matching the established `pr01_config_from_env.rs`
//! precedent for this exact test shape. HTTP-response scenarios that mutate
//! observable auth state (US-SM-03/04 signin + operator-route outcomes) use
//! `assert_state_delta` with the `universe` module below, matching the
//! `admin_api_v2` precedent for HTTP-response acceptance tests.

#![allow(dead_code, unused_imports)]

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for secrets-management acceptance assertions.
/// All keys are port-exposed (HTTP status codes, response body flags, process
/// exit codes) — never internal struct fields.
pub mod universe {
    /// HTTP status code of a `POST /admin/v1/auth/signin` request (as string).
    pub const SIGNIN_STATUS: &str = "response.signin.status_code";
    /// HTTP status code of a request to an operator-guarded route (as string).
    pub const OPERATOR_ROUTE_STATUS: &str = "response.operator_route.status_code";
    /// HTTP status code of `GET /metrics` (as string).
    pub const METRICS_STATUS: &str = "response.metrics.status_code";
    /// HTTP status code of `GET /admin/v1/projects/:id` (dual-auth route, as string).
    pub const DUAL_AUTH_ROUTE_STATUS: &str = "response.dual_auth_route.status_code";
    /// Integer exit code of the server process (as string: "0", "1", "None").
    pub const PROCESS_EXIT_CODE: &str = "process.exit_code";
    /// Whether stderr output contains the expected error message.
    pub const STDERR_HAS_ERROR: &str = "process.stderr.required_message_present";
}

// ─── Imports ─────────────────────────────────────────────────────────────────
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand_core::{OsRng, RngCore};
use testcontainers::{
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{
        runners::AsyncRunner as ModulesAsyncRunner, ContainerAsync as ModulesContainerAsync,
        ImageExt,
    },
};
use totp_rs::{Algorithm as TotpAlgorithm, TOTP};

// ─── Test key material ─────────────────────────────────────────────────────────

/// 64-hex-char test encryption key (32 bytes) — the "current" rotation key in
/// scenarios that also configure a previous key. Value is arbitrary, not a
/// real secret. Matches `tests/production_readiness/common/mod.rs::TEST_ENCRYPTION_KEY`.
pub const TEST_ENCRYPTION_KEY: &str =
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

/// A second, distinct 64-hex-char test encryption key — the "previous"
/// (retiring) key in rotation-window scenarios. Must never equal
/// `TEST_ENCRYPTION_KEY` (D-SM-5's `DuplicateRotationKey` constraint).
pub const TEST_ENCRYPTION_KEY_PREVIOUS: &str =
    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f";

// ─── Port allocation ─────────────────────────────────────────────────────────

/// Bind :0 and return the assigned ephemeral port.
pub fn find_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind :0 failed")
        .local_addr()
        .expect("local_addr")
        .port()
}

// ─── Binary path resolution ───────────────────────────────────────────────────

/// Resolve the path to the pre-built `embyr-server` binary.
///
/// `CARGO_BIN_EXE_embyr-server` is set by Cargo at compile time to the exact
/// path of the `[[bin]]` it just built for this test run — correct
/// regardless of `CARGO_TARGET_DIR`/profile, unlike a hand-assembled
/// `target/{debug,release}/embyr-server` guess relative to the workspace
/// root (which breaks the moment builds are redirected to a shared target
/// directory, e.g. via `~/.cargo/config.toml`'s `[build] target-dir`).
pub fn embyr_server_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_embyr-server"))
}

// ─── Postgres container helper ────────────────────────────────────────────────

/// Start a Postgres 15-alpine testcontainer and return the connection URL.
pub async fn start_postgres_container() -> (ModulesContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("failed to start Postgres testcontainer");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get host port for Postgres");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, db_url)
}

// ─── LocalStack (AWS Secrets Manager emulation) ────────────────────────────────

/// Start a LocalStack container exposing the AWS Secrets Manager emulator.
/// Mirrors `tests/acceptance/us_10_aws_secrets.rs::start_localstack`.
pub async fn start_localstack() -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("localstack/localstack", "3.5")
        .with_exposed_port(ContainerPort::Tcp(4566))
        .with_wait_for(WaitFor::message_on_stdout("Ready."))
        .start()
        .await
        .expect("start localstack");
    let port = container
        .get_host_port_ipv4(4566)
        .await
        .expect("get localstack port");
    let endpoint_url = format!("http://127.0.0.1:{port}");
    (container, endpoint_url)
}

/// Build an AWS Secrets Manager SDK client pointed at LocalStack.
pub async fn make_sm_client(endpoint_url: &str) -> aws_sdk_secretsmanager::Client {
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    aws_sdk_secretsmanager::Client::new(&aws_config)
}

/// Create a secret in LocalStack whose value is the RAW string `raw_value`
/// (no `{"dsn": ...}` JSON wrapper — this is the shape `get_raw_secret` reads,
/// distinct from the existing DSN-JSON `parse_dsn` path per D-SM-3).
///
/// Returns the secret's ARN.
pub async fn create_raw_secret(
    client: &aws_sdk_secretsmanager::Client,
    name: &str,
    raw_value: &str,
) -> String {
    let resp = client
        .create_secret()
        .name(name)
        .secret_string(raw_value)
        .send()
        .await
        .expect("create secret in LocalStack");
    resp.arn().expect("arn present").to_string()
}

/// An ARN that does not exist in LocalStack — used for "fetch fails" scenarios.
pub fn nonexistent_secret_arn() -> String {
    "arn:aws:secretsmanager:us-east-1:000000000000:secret:nonexistent-secret-ABCDEF".to_string()
}

/// Standard AWS SDK env vars pointed at a LocalStack endpoint, for injection
/// into a spawned `embyr-server` subprocess. `ServerConfig::from_env()`'s AWS
/// credential resolution uses the standard SDK default chain (ADR-018 §3,
/// B-SM-08) which honours `AWS_ENDPOINT_URL` — the same mechanism LocalStack
/// integration already relies on elsewhere in this workspace.
pub fn localstack_aws_env(endpoint_url: &str) -> Vec<(&'static str, String)> {
    vec![
        ("AWS_ACCESS_KEY_ID", "test".to_string()),
        ("AWS_SECRET_ACCESS_KEY", "test".to_string()),
        ("AWS_REGION", "us-east-1".to_string()),
        ("AWS_ENDPOINT_URL", endpoint_url.to_string()),
    ]
}

// ─── TOTP seeding helpers (US-SM-03 rotation scenarios) ────────────────────────

/// AES-256-GCM encrypt `raw` (20-byte TOTP secret) under `key`, prefixing the
/// 12-byte random nonce — the exact `totp_secret_enc` on-disk shape read by
/// `admin/handlers/auth.rs:255`.
pub fn encrypt_totp_secret(key: &[u8; 32], raw: &[u8; 20]) -> Vec<u8> {
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key).expect("valid AES-256-GCM key");
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, raw.as_ref())
        .expect("AES-256-GCM encrypt failed");
    let mut enc = nonce_bytes.to_vec();
    enc.extend_from_slice(&ciphertext);
    enc
}

/// Truncate a valid `totp_secret_enc` blob to below the 12-byte nonce minimum
/// — the `RotationDecryptError::Malformed` trigger shape.
pub fn corrupt_below_nonce_minimum(enc: &[u8]) -> Vec<u8> {
    enc[..8.min(enc.len())].to_vec()
}

/// Seed one account + one user with a real Argon2id-hashed password (same
/// production params as `admin/handlers/auth.rs::signin` — matches the
/// pattern in `tests/admin_api_v2/common/mod.rs`) and a `totp_secret_enc` row
/// encrypted under `encrypted_under_key`. Returns `(account_id, user_id,
/// totp_raw_secret, password)` — the raw 20-byte TOTP secret is needed to
/// compute a live TOTP code via [`totp_code_now`]; `password` is the
/// plaintext the caller must send in the `POST /admin/v1/auth/signin` JSON
/// body (signin's Argon2id check verifies it, so it must be sent for the
/// request to pass JSON validation and reach the TOTP branch under test).
pub async fn seed_totp_user(
    pool: &sqlx::PgPool,
    email: &str,
    encrypted_under_key: &[u8; 32],
) -> (uuid::Uuid, uuid::Uuid, [u8; 20], String) {
    use argon2::{
        password_hash::SaltString, Algorithm as Argon2Algorithm, Argon2, Params, PasswordHasher,
        Version,
    };

    let account_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (name) VALUES ('Secrets Rotation Test Account') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert account");

    let mut totp_raw = [0u8; 20];
    OsRng.fill_bytes(&mut totp_raw);
    let totp_secret_enc = encrypt_totp_secret(encrypted_under_key, &totp_raw);

    // Real Argon2id hash (production params) so signin's password-verification
    // step succeeds — the rotation scenarios exercise the TOTP-decrypt branch,
    // which is only reachable after a genuine password check passes.
    let password = "sm03-rotation-test-password".to_string();
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::new(
        Argon2Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, None).expect("valid Argon2id params"),
    );
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .expect("Argon2id hash failed")
        .to_string();

    let user_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (account_id, email, display_name, password_hash, totp_secret_enc) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(account_id)
    .bind(email)
    .bind("Secrets Rotation Test User")
    .bind(&password_hash)
    .bind(&totp_secret_enc)
    .fetch_one(pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO account_members (user_id, account_id, role, joined_at) \
         VALUES ($1, $2, 'Owner', now())",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("insert account_member");

    (account_id, user_id, totp_raw, password)
}

/// Overwrite an existing user's `totp_secret_enc` column with a corrupted
/// (below-nonce-minimum) value, for the "malformed ciphertext" sad path.
pub async fn corrupt_totp_secret(pool: &sqlx::PgPool, user_id: uuid::Uuid) {
    sqlx::query("UPDATE users SET totp_secret_enc = $1 WHERE id = $2")
        .bind(vec![0u8; 4]) // 4 bytes: below the 12-byte nonce minimum
        .bind(user_id)
        .execute(pool)
        .await
        .expect("corrupt totp_secret_enc");
}

/// Generate a valid TOTP code for `totp_raw` at the current system time.
pub fn totp_code_now(totp_raw: &[u8; 20]) -> String {
    let totp = TOTP::new(
        TotpAlgorithm::SHA1,
        6,
        1,
        30,
        totp_raw.to_vec(),
        None,
        String::new(),
    )
    .expect("TOTP::new failed");
    totp.generate_current().expect("generate_current failed")
}

// ─── Session-auth seeding helper (US-SM-02 scenario 2) ─────────────────────────

/// Seed a signed-in Owner session directly against the system DB, mirroring
/// `tests/admin_api_v2/common/mod.rs`'s `AdminTestContext` seeding pattern
/// (insert an account row, a user row with Owner role via `account_members`,
/// a `sessions` row with a BLAKE3-hashed token) — but scoped to just what a
/// session-auth-guarded write path needs, without the full `AdminTestContext`
/// HTTP-server bootstrap (this suite drives the server as a subprocess).
///
/// Returns `(account_id, user_id, cookie_value)`; attach `cookie_value` as
/// `Cookie: embyr_session=<cookie_value>` on subsequent requests.
pub async fn seed_signed_in_session(
    pool: &sqlx::PgPool,
    email: &str,
) -> (uuid::Uuid, uuid::Uuid, String) {
    let account_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (name) VALUES ('Secrets Management Session Test Account') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert account");

    // Placeholder Argon2id-shaped hash string — password auth is not
    // exercised by session-cookie-seeded scenarios.
    let password_hash =
        "$argon2id$v=19$m=65536,t=3,p=4$c2VjcmV0c21hbmFnZW1lbnQ$placeholderplaceholderplaceholder";

    let user_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (account_id, email, display_name, password_hash) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(account_id)
    .bind(email)
    .bind("Secrets Management Session Test User")
    .bind(password_hash)
    .fetch_one(pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO account_members (user_id, account_id, role, joined_at) \
         VALUES ($1, $2, 'Owner', now())",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("insert account_member");

    // Raw cookie value handed to the caller; only its BLAKE3 hash is stored,
    // matching `admin/middleware/session_auth.rs`'s cookie-path lookup.
    let cookie_value = uuid::Uuid::new_v4().to_string();
    let token_hash = blake3::hash(cookie_value.as_bytes()).as_bytes().to_vec();

    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&token_hash)
    .execute(pool)
    .await
    .expect("insert session");

    (account_id, user_id, cookie_value)
}

// ─── ServerProcess harness ────────────────────────────────────────────────────

/// Wraps a spawned `embyr-server` subprocess with its port allocation.
/// Independent copy of `tests/production_readiness/common/mod.rs::ServerProcess`.
///
/// `stdout`/`stderr` are drained continuously by background threads into
/// `output_buf` from the moment the process is spawned (real bug, found
/// 2026-08-31: `Stdio::piped()` pipes have a bounded OS buffer — typically
/// 64KB on Linux. `RUST_LOG=debug` produces easily that much output during
/// this codebase's own (now sizeable, still-growing) migration set alone.
/// If nobody reads the pipe while `wait_for_healthy` only polls `/healthz`,
/// the child blocks on its own `write()` syscall the moment the buffer
/// fills — a classic subprocess pipe deadlock, not a slow-startup timing
/// issue. The child can NEVER become healthy in that state no matter how
/// long the caller waits, which is why bumping `wait_for_healthy`'s timeout
/// made the failure rate worse, not better: longer-blocked children just
/// compounded resource contention for everyone else. Draining continuously,
/// from spawn time, is the actual fix — a longer timeout was treating the
/// symptom.
pub struct ServerProcess {
    pub child: Child,
    pub grpc_port: u16,
    pub rest_port: u16,
    pub admin_port: u16,
    output_buf: std::sync::Arc<std::sync::Mutex<String>>,
}

impl ServerProcess {
    /// Spawn `embyr-server` with `DATABASE_URL` injected plus `extra_env`.
    pub fn start(db_url: &str, extra_env: &[(&str, &str)]) -> Self {
        let owned: Vec<(&str, String)> = extra_env
            .iter()
            .map(|(k, v)| (*k, v.to_string()))
            .collect();
        let borrowed: Vec<(&str, &str)> =
            owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        Self::spawn_with(Some(db_url), &borrowed)
    }

    /// Spawn `embyr-server` with `DATABASE_URL` injected plus owned `extra_env`
    /// pairs (use when a value, e.g. a LocalStack endpoint URL, is computed at
    /// runtime rather than a `&'static str`).
    pub fn start_owned(db_url: &str, extra_env: &[(&str, String)]) -> Self {
        let borrowed: Vec<(&str, &str)> =
            extra_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        Self::spawn_with(Some(db_url), &borrowed)
    }

    /// Spawn `embyr-server` with ONLY the provided env vars (no `DATABASE_URL`
    /// injection, no inherited environment) — for config-error scenarios.
    pub fn start_env_only(env_vars: &[(&str, &str)]) -> Self {
        Self::spawn_with(None, env_vars)
    }

    fn spawn_with(db_url: Option<&str>, extra_env: &[(&str, &str)]) -> Self {
        let grpc_port = find_free_port();
        let rest_port = find_free_port();
        let admin_port = find_free_port();

        let bin = embyr_server_binary();
        let mut cmd = Command::new(&bin);
        cmd.env_clear()
            .env("GRPC_PORT", grpc_port.to_string())
            .env("REST_PORT", rest_port.to_string())
            .env("ADMIN_PORT", admin_port.to_string())
            .env("RUST_LOG", "info")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());

        if let Some(db_url) = db_url {
            cmd.env("DATABASE_URL", db_url);
        }
        for (key, val) in extra_env {
            cmd.env(key, val);
        }

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn embyr-server at {bin:?}: {e}"));

        // Drain stdout/stderr continuously from a background thread each,
        // from spawn time — see ServerProcess's own doc comment for why
        // this is load-bearing, not cosmetic: an unread `Stdio::piped()`
        // pipe has a bounded OS buffer, and RUST_LOG=debug can fill it
        // during this codebase's own migration set alone, deadlocking the
        // child on its own write() before it ever reaches /healthz.
        let output_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        for pipe in [
            child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let buf = std::sync::Arc::clone(&output_buf);
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(pipe);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut guard) = buf.lock() {
                        guard.push_str(&line);
                        guard.push('\n');
                    }
                }
            });
        }

        ServerProcess {
            child,
            grpc_port,
            rest_port,
            admin_port,
            output_buf,
        }
    }

    /// Poll `GET /healthz` on the admin port until HTTP 200 or timeout.
    ///
    /// Fails fast (rather than waiting out the full timeout) if the child
    /// process has already exited — a startup-time config/connectivity
    /// error (e.g. a LocalStack/AWS SDK call failing) makes the process
    /// exit almost immediately, so waiting the full timeout just to report
    /// a generic "server must start" failure hides the REAL error and
    /// wastes the whole timeout window for no reason. Prints the child's
    /// captured stdout/stderr on either exit path so a real startup failure
    /// is diagnosable from the test output directly, not just "false".
    pub async fn wait_for_healthy(&mut self, timeout: Duration) -> bool {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/healthz", self.admin_port);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                eprintln!(
                    "ServerProcess::wait_for_healthy: child exited early with {status:?} \
                     before becoming healthy — capturing output:"
                );
                self.dump_output();
                return false;
            }
            if tokio::time::Instant::now() >= deadline {
                eprintln!(
                    "ServerProcess::wait_for_healthy: timed out after {timeout:?} waiting for \
                     /healthz — killing child and capturing whatever it printed:"
                );
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.dump_output();
                return false;
            }
            match client.get(&url).send().await {
                Ok(resp) if resp.status().as_u16() == 200 => return true,
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Print whatever the background drain threads have captured so far —
    /// used by `wait_for_healthy` to make a startup failure diagnosable.
    /// Reads the shared buffer, not the pipes directly (those are owned by
    /// the drain threads now, not `self.child` — see `spawn_with`).
    fn dump_output(&self) {
        let buf = self.output_buf.lock().map(|g| g.clone()).unwrap_or_default();
        eprintln!("--- child stdout+stderr (interleaved) ---\n{buf}\n--- end child output ---");
    }

    /// Send SIGTERM to the child process (Unix); falls back to SIGKILL elsewhere.
    pub fn sigterm(&mut self) {
        let pid = self.child.id();
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
        }
    }

    /// Poll `child.try_wait()` until the process exits or timeout elapses.
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) => {
                    if tokio::time::Instant::now() >= deadline {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(_) => return None,
            }
        }
    }

    /// Return the child's captured stdout+stderr (interleaved) so far. Call
    /// after the process has exited (or after enough time has passed for
    /// the relevant lines to be flushed) — reads the shared buffer the
    /// background drain threads write to (see `spawn_with`), not the pipes
    /// directly (those are owned by the drain threads now, not `self.child`).
    pub fn drain_stderr(&mut self) -> String {
        self.output_buf.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Check whether the given TCP port currently accepts connections.
    pub fn port_is_bound(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().expect("valid addr"),
            Duration::from_millis(100),
        )
        .is_ok()
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
