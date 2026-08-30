// SCAFFOLD: true
//! Common test infrastructure — client-auth-hosted-identity acceptance tests
//! (Slice 01, ADR-036).
//!
//! Driving port: Admin HTTP :9090 (`HostedIdentityAdminContext`, real
//! `build_admin_router` composition root — mirrors
//! `tests/client_auth/common/mod.rs::ClientAuthAdminContext`, since this
//! slice's own scope is a single session-authed admin action, no gRPC/REST
//! data-plane surface).
//! Driven internal: testcontainers-rs Postgres (System DB) — reused
//! unchanged. `hosted_identity_signing_keys` is a new TABLE (migration
//! 0028), not a new PORT.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_core::admin::email::{EmailError, EmailMessage, IEmailSender};
use embyr_core::auth::{argon2, ecies};
use embyr_server::{
    adapters::{
        cap_status_cache::CapStatusCache, credential_cache::CredentialCache,
        email::NoopEmailSender, stripe_gateway::StripeGateway, system_db::SystemDb,
    },
    admin::router::build_admin_router,
};

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{assert_state_delta, set_to};

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for hosted-identity acceptance assertions.
/// Response fields and DB public-column reads via dedicated helpers — never
/// internal struct fields.
pub mod universe {
    pub const ENABLE_STATUS: &str = "response.enable.status_code";
    pub const SIGNING_KEY_ROW_EXISTS: &str = "db.hosted_identity_signing_keys.row_exists";
    pub const SIGN_UP_STATUS: &str = "response.sign_up.status_code";
    pub const ACCOUNT_ROW_EXISTS: &str = "db.hosted_identity_accounts.row_exists";
}

// ─── HostedIdentityAdminContext ────────────────────────────────────────────────

/// Ephemeral test context: Postgres + Axum admin server on an ephemeral
/// port. Mirrors `ClientAuthAdminContext` (admin-only feature, lightweight
/// session seed — no full Argon2id+TOTP sign-in dance).
pub struct HostedIdentityAdminContext {
    _container: ContainerAsync<Postgres>,
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
}

pub const HOSTED_IDENTITY_ADMIN_KEY: &str = "test-admin-key-hosted-identity";

impl HostedIdentityAdminContext {
    pub async fn new() -> Self {
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

        let system_db = Arc::new(SystemDb::new(&db_url).await.expect("SystemDb::new failed"));
        system_db.migrate().await.expect("Migrations failed");
        let pool = system_db.pool().clone();

        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Hosted Identity Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new("stripe-secret-key-not-configured"));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();

        let router = build_admin_router(
            system_db,
            HOSTED_IDENTITY_ADMIN_KEY.to_string(),
            None,
            credential_cache,
            [0u8; 32],
            None,
            Arc::new(NoopEmailSender),
            None,
            None,
            1000.0,
            prometheus_handle,
            stripe_gateway,
            String::new(),
            Arc::new(CapStatusCache::new()),
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
        tokio::task::yield_now().await;

        let client = reqwest::Client::builder()
            .cookie_store(false)
            .build()
            .expect("reqwest client");

        HostedIdentityAdminContext {
            _container: container,
            base_url,
            client,
            pool,
            account_id,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Seed a signed-in session for `self.account_id` with the given role.
    /// Mirrors `ClientAuthAdminContext::seed_session`.
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$Y2xpZW50YXV0aA$placeholderplaceholderplaceholder";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Hosted Identity Test User")
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .expect("insert user");

        sqlx::query(
            "INSERT INTO account_members (user_id, account_id, role, joined_at) \
             VALUES ($1, $2, $3, now())",
        )
        .bind(user_id)
        .bind(self.account_id)
        .bind(role)
        .execute(&self.pool)
        .await
        .expect("insert account_member");

        let cookie_value = uuid::Uuid::new_v4().to_string();
        let token_hash = blake3::hash(cookie_value.as_bytes()).as_bytes().to_vec();

        sqlx::query(
            "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
             VALUES ($1, $2, $3, now() + interval '1 hour')",
        )
        .bind(user_id)
        .bind(self.account_id)
        .bind(&token_hash)
        .execute(&self.pool)
        .await
        .expect("insert session");

        format!("embyr_session={cookie_value}")
    }

    /// Insert an active project under `self.account_id` with a REAL
    /// Argon2id hash of `api_key` as `api_key_hash_current` — unlike
    /// `ClientAuthAdminContext::insert_project`'s placeholder hash, this
    /// slice's own AC-18-01/05 require Argon2id-verifying a real submitted
    /// key against the stored hash.
    pub async fn insert_project(&self, project_id: &str, backend_mode: &str, api_key: &str) {
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, $3, $4, 'active', $1)",
        )
        .bind(project_id)
        .bind(self.account_id)
        .bind(backend_mode)
        .bind(&api_key_hash)
        .execute(&self.pool)
        .await
        .expect("insert project");
    }

    pub async fn signing_key_row_exists(&self, project_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM hosted_identity_signing_keys WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
            > 0
    }

    /// Read the stored signing-key material directly (public_key,
    /// private_key_enc) — used by AC-18-02 to prove a second enablement
    /// call is byte-identical to the first (no regeneration).
    pub async fn signing_key_material(&self, project_id: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
            "SELECT public_key, private_key_enc FROM hosted_identity_signing_keys \
             WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .expect("query signing key material")
    }
}

// ─── HostedIdentityFullContext (Slice 02 — full production stack) ────────────

/// Full production composition root: gRPC :8080, REST :8081, admin :9090,
/// via `embyr_server::start_test_server` — mirrors
/// `tests/client_auth/common/mod.rs::ClientAuthFullContext` exactly (this
/// slice's own AC-18-05 needs the SAME "real getDoc call after signup"
/// regression proof `client-auth`'s ca02 already established, now carrying
/// a hosted-identity-minted token instead of a customer-minted one).
pub struct HostedIdentityFullContext {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub sys_pool: sqlx::PgPool,
    pub api_key: String,
    pub project_id: String,
    pub account_id: uuid::Uuid,
    /// Slice 04 (US-04): captures every `EmailMessage` sent via
    /// `IEmailSender` during this test's lifetime — used to read the raw
    /// reset-token text mailed by `accounts:sendOobCode` (only its BLAKE3
    /// hash is ever persisted). Wired into EVERY context, not just
    /// reset-password tests — behaviorally identical to `NoopEmailSender`
    /// for hi02/hi03 (`send` always returns `Ok`), so no other slice's tests
    /// are affected by this becoming the default test-harness double.
    pub captured_emails: Arc<FakeEmailSender>,
}

impl HostedIdentityFullContext {
    /// `project_id` is seeded active, `direct_pg` backend, with one document
    /// written at `documents/{project_id}-doc-1` so AC-18-05's subsequent
    /// getDoc call has something real to read. Hosted identity is NOT
    /// enabled by `new()` — call `enable_hosted_identity()` explicitly (so
    /// AC-18-08's "not enabled" scenario can use a context that never does).
    pub async fn new(project_id: &str) -> Self {
        let sys_container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("start system Postgres container");
        let sys_port = sys_container
            .get_host_port_ipv4(5432)
            .await
            .expect("system Postgres host port");
        let sys_url = format!("postgres://postgres:postgres@127.0.0.1:{sys_port}/postgres");

        let cust_container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("start customer Postgres container");
        let cust_port = cust_container
            .get_host_port_ipv4(5432)
            .await
            .expect("customer Postgres host port");
        let cust_url = format!("postgres://postgres:postgres@127.0.0.1:{cust_port}/postgres");

        let system_db = Arc::new(SystemDb::new(&sys_url).await.expect("SystemDb::new failed"));
        system_db.migrate().await.expect("system DB migrations failed");
        let sys_pool = system_db.pool().clone();

        let cust_pool = sqlx::PgPool::connect(&cust_url)
            .await
            .expect("connect customer DB");
        sqlx::migrate!("../../migrations/customer")
            .run(&cust_pool)
            .await
            .expect("customer DB migrations failed");

        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Hosted Identity Full-Stack Test Account') RETURNING id",
        )
        .fetch_one(&sys_pool)
        .await
        .expect("insert account");

        let api_key = format!("test-sk-hosted-identity-{project_id}");
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).expect("ecies encrypt dsn");

        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, $2, 'active', 'direct_pg', $3, $4)",
        )
        .bind(project_id)
        .bind(account_id)
        .bind(&api_key_hash)
        .bind(&encrypted_dsn)
        .execute(&sys_pool)
        .await
        .expect("insert project");

        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, create_time, update_time, version) \
             VALUES ($1, 'trip-journal', $2, $3, now(), now(), 1)",
        )
        .bind(project_id)
        .bind(format!("{project_id}-doc-1"))
        .bind(serde_json::json!({"title": {"t": "S", "v": "hello"}}))
        .execute(&cust_pool)
        .await
        .expect("insert seed document");

        let captured_emails = Arc::new(FakeEmailSender::default());
        let server = embyr_server::start_test_server_with_email_sender(
            system_db,
            captured_emails.clone(),
        )
        .await;

        HostedIdentityFullContext {
            _sys_container: sys_container,
            _cust_container: cust_container,
            server,
            sys_pool,
            api_key,
            project_id: project_id.to_string(),
            account_id,
            captured_emails,
        }
    }

    /// Hand-seed a reset-token row directly (bypasses `accounts:sendOobCode`)
    /// — used by AC-18-16 to seed an ALREADY-EXPIRED row (can't wait an hour
    /// in a test). Mirrors `enable_hosted_identity`'s own "hand-seeded row"
    /// allowance.
    pub async fn seed_reset_token(
        &self,
        email: &str,
        raw_token: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) {
        let token_hash = blake3::hash(raw_token.as_bytes()).as_bytes().to_vec();
        sqlx::query(
            "INSERT INTO hosted_identity_reset_tokens (project_id, email, token_hash, expires_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&self.project_id)
        .bind(email)
        .bind(&token_hash)
        .bind(expires_at)
        .execute(self.customer_pool().await.pool())
        .await
        .expect("insert hosted_identity_reset_tokens row");
    }

    /// Hand-seed `hosted_identity_signing_keys` directly (bypasses the
    /// Slice-01 admin enable endpoint — same "hand-seeded row" allowance
    /// `ClientAuthAdminContext::seed_credential` already uses). Encrypts a
    /// freshly-generated Ed25519 seed under a pubkey derived from
    /// `self.api_key`, mirroring `enable_hosted_identity`'s own exact
    /// crypto call shape (ADR-036 Decision 2).
    pub async fn enable_hosted_identity(&self) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = signing_key.verifying_key().to_bytes();
        let private_key_seed = signing_key.to_bytes();
        let recipient_pubkey = ecies::derive_public_key(self.api_key.as_bytes());
        let private_key_enc = ecies::encrypt(&recipient_pubkey, &private_key_seed)
            .expect("ecies encrypt signing key");

        sqlx::query(
            "INSERT INTO hosted_identity_signing_keys \
             (project_id, public_key, private_key_enc, algorithm) \
             VALUES ($1, $2, $3, 'EdDSA')",
        )
        .bind(&self.project_id)
        .bind(&public_key[..])
        .bind(&private_key_enc)
        .execute(&self.sys_pool)
        .await
        .expect("insert hosted_identity_signing_keys row");
    }

    pub fn rest_url(&self, path: &str) -> String {
        format!("http://{}{}", self.server.rest_addr, path)
    }

    pub fn document_resource_name(&self) -> String {
        format!(
            "projects/{}/databases/(default)/documents/trip-journal/{}-doc-1",
            self.project_id, self.project_id
        )
    }

    pub async fn hosted_identity_account_exists(&self, email: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM hosted_identity_accounts WHERE project_id = $1 AND email = $2",
        )
        .bind(&self.project_id)
        .bind(email)
        .fetch_one(self.customer_pool().await.pool())
        .await
        .unwrap_or(0)
            > 0
    }

    /// Reconnect to the customer DB for direct assertions (test-only — the
    /// production path always goes through `resolve_customer_db_adapter`).
    async fn customer_pool(&self) -> embyr_server::adapters::postgres_backend::PostgresBackendAdapter {
        let encrypted_dsn: Vec<u8> = sqlx::query_scalar(
            "SELECT ecies_encrypted_dsn FROM projects WHERE id = $1",
        )
        .bind(&self.project_id)
        .fetch_one(&self.sys_pool)
        .await
        .expect("read ecies_encrypted_dsn");
        let dsn_bytes =
            ecies::decrypt(self.api_key.as_bytes(), &encrypted_dsn).expect("decrypt dsn");
        let dsn = String::from_utf8(dsn_bytes).expect("dsn utf8");
        embyr_server::adapters::postgres_backend::PostgresBackendAdapter::new(&dsn)
            .await
            .expect("connect customer db")
    }
}

// ─── FakeEmailSender (Slice 04) ────────────────────────────────────────────────

/// Captures every `EmailMessage` sent during a test, for assertion.
/// Mirrors `tests/admin_api_v2/common/mod.rs::FakeEmailSender` (same
/// capture-double shape) — a fresh, small copy rather than a cross-test-binary
/// import, since each `tests/<feature>/` directory is its own compiled
/// integration-test crate.
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

impl IEmailSender for FakeEmailSender {
    fn send(
        &self,
        message: EmailMessage,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), EmailError>> + Send + '_>>
    {
        Box::pin(async move {
            self.sent.lock().unwrap().push(CapturedEmail {
                to: message.to,
                subject: message.subject,
                body: message.body_text,
            });
            Ok(())
        })
    }
}
