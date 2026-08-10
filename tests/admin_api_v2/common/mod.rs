// SCAFFOLD: true
//! Common test infrastructure — admin-api-v2 acceptance tests.
// Items here are consumed incrementally as each slice's tests are unskipped.
// Suppress unused warnings for infra that later slices will use.
#![allow(dead_code, unused_imports)]
//!
//! Provides:
//! - `AdminTestContext`: ephemeral Postgres + Axum admin server on a random port.
//! - `totp_code_now`: generates a valid TOTP code for the seeded user.
//! - `state_delta`: re-export of the project-level state-delta port.
//!
//! Infrastructure policy (from docs/architecture/atdd-infrastructure-policy.md):
//!   Driving port:    reqwest::Client against Axum admin server on ephemeral port.
//!   Driven internal: testcontainers-rs Postgres image; sqlx::migrate!; fresh per test module.
//!   Driven external: NoopEmailSender (in-process); FakeEmailSender (capture variant).
//!   Clock:           tokio::time::advance (paused clock) per #[tokio::test(start_paused = true)].

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Domain snapshot types used in universe assertions ────────────────────────

/// Port-exposed observable names used in state-delta universe assertions.
/// These map to public API response fields, not internal struct fields.
pub mod universe {
    pub const SESSION_COOKIE_SET: &str = "response.set_cookie.embyr_session";
    pub const RESPONSE_STATUS: &str = "response.status_code";
    pub const ACCOUNT_LOCKED: &str = "auth.account_locked";
    pub const SESSION_ROW_EXPIRES: &str = "db.sessions.expires_at";
    pub const PROJECT_LIST_COUNT: &str = "response.projects.len";
    pub const SDK_KEY_PLAINTEXT_IN_RESPONSE: &str = "response.sdk_key.key_present";
    pub const SDK_KEY_HASH_IN_DB: &str = "db.sdk_api_keys.key_hash_stored";
    pub const MEMBER_COUNT: &str = "response.members.len";
    pub const INVITATION_ROW_CREATED: &str = "db.invitations.row_exists";
    pub const EMAIL_SENT_COUNT: &str = "fake_email_sender.sent_count";
    pub const OIDC_SECRET_IN_RESPONSE: &str = "response.oidc_provider.client_secret_present";
    pub const SESSION_COOKIE_CLEARED: &str = "response.set_cookie.max_age_zero";
}

// ─── Imports ─────────────────────────────────────────────────────────────────
use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{password_hash::SaltString, Algorithm as Argon2Algorithm, Argon2, Params, PasswordHasher, Version};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand_core::{OsRng, RngCore};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use totp_rs::{Algorithm as TotpAlgorithm, TOTP};

use embyr_server::{
    adapters::{
        credential_cache::CredentialCache,
        email::NoopEmailSender,
        system_db::SystemDb,
    },
    admin::router::build_admin_router,
};

// ─── Test server ─────────────────────────────────────────────────────────────

/// Ephemeral test context: Postgres container + seeded data + admin HTTP server.
///
/// Holds the container alive for the duration of the test. All fields are pub
/// to allow direct DB assertions in tests (e.g., session token hash checks).
pub struct AdminTestContext {
    /// Keeps the Postgres container alive for the test's duration.
    _container: ContainerAsync<Postgres>,
    /// Base URL of the ephemeral admin HTTP server, e.g. "http://127.0.0.1:54321".
    pub base_url: String,
    /// reqwest client with no default headers (tests set Auth/Cookie headers per request).
    pub client: reqwest::Client,
    /// Direct pool reference for DB assertion queries in tests (e.g., session hash verification).
    pub pool: sqlx::PgPool,
    /// TOTP secret stored as base64url(raw_20_bytes) — used by `totp_code_now()`.
    pub totp_secret_b32: String,
    /// UUID of the seeded account (string form).
    pub account_id: String,
    /// UUID of the seeded user/Owner (string form).
    pub user_id: String,
    /// Email of the seeded user.
    pub user_email: String,
    /// Plain-text password of the seeded user.
    pub user_password: String,
    /// Email of the seeded Viewer role user.
    pub viewer_email: String,
    /// Plain-text password of the seeded Viewer role user.
    pub viewer_password: String,
    /// TOTP secret for the Viewer user (base64url-no-pad of raw 20 bytes).
    pub viewer_totp_secret_b32: String,
    /// UUID of the seeded Viewer user (string form).
    pub viewer_id: String,
    /// Email of the seeded Admin role user.
    pub admin_email: String,
    /// Plain-text password of the seeded Admin role user.
    pub admin_password: String,
    /// TOTP secret for the Admin user (base64url-no-pad of raw 20 bytes).
    pub admin_totp_secret_b32: String,
    /// UUID of the seeded Admin user (string form).
    pub admin_id: String,
}

impl AdminTestContext {
    /// Build an AdminTestContext by:
    ///   1. Starting a testcontainers Postgres 15-alpine container.
    ///   2. Running sqlx migrations from workspace migrations/.
    ///   3. Seeding one account + user (Argon2id hash) + TOTP secret (AES-256-GCM) + recovery code.
    ///   4. Starting the Axum admin router on an ephemeral port.
    pub async fn new() -> Self {
        // ── 1. Start Postgres container ───────────────────────────────────────
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port for Postgres");
        let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        // ── 2. Run migrations ─────────────────────────────────────────────────
        let system_db = Arc::new(
            SystemDb::new(&db_url)
                .await
                .expect("SystemDb::new failed"),
        );
        system_db.migrate().await.expect("Migrations failed");

        let pool = system_db.pool().clone();

        // ── 3. Seed account ───────────────────────────────────────────────────
        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        // ── 4. Hash password with Argon2id (matching production params) ───────
        let user_password = "test-password-123".to_string();
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::new(
            Argon2Algorithm::Argon2id,
            Version::V0x13,
            Params::new(65536, 3, 4, None).expect("valid Argon2id params"),
        );
        let password_hash = argon2
            .hash_password(user_password.as_bytes(), &salt)
            .expect("Argon2id hash failed")
            .to_string();

        // ── 5. Generate 20-byte raw TOTP secret ───────────────────────────────
        let mut totp_raw = [0u8; 20];
        OsRng.fill_bytes(&mut totp_raw);
        // Store as base64url(no-pad) for in-memory use in totp_code_now().
        let totp_secret_b32 = URL_SAFE_NO_PAD.encode(&totp_raw);

        // ── 6. AES-256-GCM encrypt TOTP secret for DB storage ─────────────────
        // Encryption key = [0u8; 32] (matches build_admin_router test key).
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&[0u8; 32]).expect("AES-256-GCM key");
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, totp_raw.as_ref())
            .expect("AES-256-GCM encrypt failed");
        let mut totp_secret_enc = nonce_bytes.to_vec();
        totp_secret_enc.extend_from_slice(&ciphertext);

        // ── 7. Insert user ────────────────────────────────────────────────────
        let user_email = "test@example.com".to_string();
        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users \
             (account_id, email, display_name, password_hash, totp_secret_enc) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id",
        )
        .bind(account_id)
        .bind(&user_email)
        .bind("Test User")
        .bind(&password_hash)
        .bind(&totp_secret_enc)
        .fetch_one(&pool)
        .await
        .expect("insert user");

        // ── 8. Insert account_member (Owner) ──────────────────────────────────
        sqlx::query(
            "INSERT INTO account_members (user_id, account_id, role, joined_at) \
             VALUES ($1, $2, 'Owner', now())",
        )
        .bind(user_id)
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("insert account_member");

        // ── 9. Insert recovery code BLAKE3("RECOV-0001") ─────────────────────
        // Matches the hardcoded recovery code used in b01 test AC-B01-05.
        let recovery_code_hash = blake3::hash("RECOV-0001".as_bytes())
            .as_bytes()
            .to_vec();
        sqlx::query(
            "INSERT INTO mfa_recovery_codes (user_id, code_hash) VALUES ($1, $2)",
        )
        .bind(user_id)
        .bind(&recovery_code_hash)
        .execute(&pool)
        .await
        .expect("insert recovery code");

        // ── 10. Seed Viewer user ──────────────────────────────────────────────
        let viewer_password = "viewer-password-456".to_string();
        let viewer_salt = SaltString::generate(&mut OsRng);
        let viewer_password_hash = argon2
            .hash_password(viewer_password.as_bytes(), &viewer_salt)
            .expect("Argon2id hash failed for viewer")
            .to_string();

        let mut viewer_totp_raw = [0u8; 20];
        OsRng.fill_bytes(&mut viewer_totp_raw);
        let viewer_totp_secret_b32 = URL_SAFE_NO_PAD.encode(&viewer_totp_raw);

        let mut viewer_nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut viewer_nonce_bytes);
        let viewer_nonce = Nonce::from_slice(&viewer_nonce_bytes);
        let viewer_ct = cipher
            .encrypt(viewer_nonce, viewer_totp_raw.as_ref())
            .expect("AES-256-GCM encrypt failed for viewer TOTP");
        let mut viewer_totp_enc = viewer_nonce_bytes.to_vec();
        viewer_totp_enc.extend_from_slice(&viewer_ct);

        let viewer_email = "viewer@example.com".to_string();
        let viewer_user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash, totp_secret_enc) \
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(account_id)
        .bind(&viewer_email)
        .bind("Viewer User")
        .bind(&viewer_password_hash)
        .bind(&viewer_totp_enc)
        .fetch_one(&pool)
        .await
        .expect("insert viewer user");

        sqlx::query(
            "INSERT INTO account_members (user_id, account_id, role, joined_at) \
             VALUES ($1, $2, 'Viewer', now())",
        )
        .bind(viewer_user_id)
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("insert viewer account_member");

        // ── Seed Admin user ───────────────────────────────────────────────────
        let admin_password = "admin-password-789".to_string();
        let admin_salt = SaltString::generate(&mut OsRng);
        let admin_password_hash = argon2
            .hash_password(admin_password.as_bytes(), &admin_salt)
            .expect("Argon2id hash failed for admin")
            .to_string();

        let mut admin_totp_raw = [0u8; 20];
        OsRng.fill_bytes(&mut admin_totp_raw);
        let admin_totp_secret_b32 = URL_SAFE_NO_PAD.encode(&admin_totp_raw);

        let mut admin_nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut admin_nonce_bytes);
        let admin_nonce = Nonce::from_slice(&admin_nonce_bytes);
        let admin_ct = cipher
            .encrypt(admin_nonce, admin_totp_raw.as_ref())
            .expect("AES-256-GCM encrypt failed for admin TOTP");
        let mut admin_totp_enc = admin_nonce_bytes.to_vec();
        admin_totp_enc.extend_from_slice(&admin_ct);

        let admin_email = "admin@example.com".to_string();
        let admin_user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash, totp_secret_enc) \
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(account_id)
        .bind(&admin_email)
        .bind("Admin User")
        .bind(&admin_password_hash)
        .bind(&admin_totp_enc)
        .fetch_one(&pool)
        .await
        .expect("insert admin user");

        sqlx::query(
            "INSERT INTO account_members (user_id, account_id, role, joined_at) \
             VALUES ($1, $2, 'Admin', now())",
        )
        .bind(admin_user_id)
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("insert admin account_member");

        // ── 11. Build admin router and start Axum server ──────────────────────
        let credential_cache = Arc::new(CredentialCache::new(1024));
        let email_sender = Arc::new(NoopEmailSender);

        // ── Seed projects ─────────────────────────────────────────────────────────

        // Active project in ctx.account_id
        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, 'direct_pg', 'placeholder_hash_active', 'active', 'Test Project')",
        )
        .bind("test-project-seeded-for-account")
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("insert active test project");

        // Deleted project in ctx.account_id (to test exclusion)
        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, 'direct_pg', 'placeholder_hash_deleted', 'deleted', 'Deleted Project')",
        )
        .bind("test-project-deleted")
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("insert deleted test project");

        // Create a second account (no user) and a project in it (for cross-account test)
        let other_account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Other Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert other account");

        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, 'direct_pg', 'placeholder_hash_other', 'active', 'Other Account Project')",
        )
        .bind("project-belonging-to-other-account")
        .bind(other_account_id)
        .execute(&pool)
        .await
        .expect("insert other account project");

        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();
        let router = build_admin_router(
            system_db,
            "test-admin-key-from-env".to_string(),
            None,
            credential_cache,
            [0u8; 32], // test encryption key (matches TOTP encryption above)
            None,
            email_sender,
            None,
            None,
            1000.0,
            prometheus_handle,
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("admin server task panicked");
        });

        // Yield to let the spawned server task start accepting connections.
        tokio::task::yield_now().await;

        let client = reqwest::Client::builder()
            .cookie_store(false)
            .build()
            .expect("reqwest client");

        AdminTestContext {
            _container: container,
            base_url,
            client,
            pool,
            totp_secret_b32,
            account_id: account_id.to_string(),
            user_id: user_id.to_string(),
            user_email,
            user_password,
            viewer_email,
            viewer_password,
            viewer_totp_secret_b32,
            viewer_id: viewer_user_id.to_string(),
            admin_email,
            admin_password,
            admin_totp_secret_b32,
            admin_id: admin_user_id.to_string(),
        }
    }

    /// Generate a valid TOTP code for the seeded user's secret at the current system time.
    pub fn totp_code_now(&self) -> String {
        let totp_raw = URL_SAFE_NO_PAD
            .decode(&self.totp_secret_b32)
            .expect("totp_secret_b32 must be valid base64url-no-pad");
        let totp = TOTP::new(
            TotpAlgorithm::SHA1,
            6,
            1,
            30,
            totp_raw,
            None,
            String::new(),
        )
        .expect("TOTP::new failed");
        totp.generate_current().expect("generate_current failed")
    }

    /// Generate an intentionally wrong TOTP code (always "000000").
    pub fn totp_code_wrong(&self) -> String {
        "000000".to_string()
    }

    /// Generate a valid TOTP code for the seeded Viewer user's secret at the current system time.
    pub fn viewer_totp_code_now(&self) -> String {
        let totp_raw = URL_SAFE_NO_PAD
            .decode(&self.viewer_totp_secret_b32)
            .expect("viewer_totp_secret_b32 must be valid base64url-no-pad");
        let totp = TOTP::new(
            TotpAlgorithm::SHA1,
            6,
            1,
            30,
            totp_raw,
            None,
            String::new(),
        )
        .expect("TOTP::new failed for viewer");
        totp.generate_current().expect("generate_current failed for viewer")
    }

    /// Generate a valid TOTP code for the seeded Admin user's secret at the current system time.
    pub fn admin_totp_code_now(&self) -> String {
        let totp_raw = URL_SAFE_NO_PAD
            .decode(&self.admin_totp_secret_b32)
            .expect("admin_totp_secret_b32 must be valid base64url-no-pad");
        let totp = TOTP::new(
            TotpAlgorithm::SHA1,
            6,
            1,
            30,
            totp_raw,
            None,
            String::new(),
        )
        .expect("TOTP::new failed for admin");
        totp.generate_current().expect("generate_current failed for admin")
    }

    /// Returns the full URL for the given admin API path.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

// ─── FakeEmailSender ─────────────────────────────────────────────────────────

/// Captures emails sent during a test for assertion.
///
/// # RED scaffold
/// Placeholder until IEmailSender port integration in a later slice.
#[derive(Debug, Default)]
pub struct FakeEmailSender {
    pub sent: std::sync::Mutex<Vec<CapturedEmail>>,
}

#[derive(Debug, Clone)]
pub struct CapturedEmail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

impl FakeEmailSender {
    pub fn sent_count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    pub fn last_email(&self) -> Option<CapturedEmail> {
        self.sent.lock().unwrap().last().cloned()
    }
}
