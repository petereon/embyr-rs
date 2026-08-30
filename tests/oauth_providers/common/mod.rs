// SCAFFOLD: true
//! Common test infrastructure — oauth-providers acceptance tests (Slice 01,
//! ADR-037).
//!
//! Driving port: Admin HTTP :9090 (`OAuthProviderAdminContext`, real
//! `build_admin_router` composition root — mirrors
//! `tests/client_auth_hosted_identity/common/mod.rs::HostedIdentityAdminContext`,
//! since this slice's own scope is a single session-authed admin action, no
//! gRPC/REST data-plane surface).
//! Driven internal: testcontainers-rs Postgres (System DB) — reused
//! unchanged. `oauth_provider_credentials`/`oauth_signing_keys` are new
//! TABLEs (migrations 0029/0030), not new PORTs.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_core::auth::argon2;
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

/// Port-exposed observable names for oauth-providers acceptance assertions.
/// Response fields and DB public-column reads via dedicated helpers — never
/// internal struct fields.
pub mod universe {
    pub const REGISTER_STATUS: &str = "response.register.status_code";
    pub const CREDENTIAL_ROW_EXISTS: &str = "db.oauth_provider_credentials.row_exists";
    pub const SIGNING_KEY_ROW_EXISTS: &str = "db.oauth_signing_keys.row_exists";
}

// ─── OAuthProviderAdminContext ────────────────────────────────────────────────

/// Ephemeral test context: Postgres + Axum admin server on an ephemeral
/// port. Mirrors `HostedIdentityAdminContext` (admin-only feature,
/// lightweight session seed — no full Argon2id+TOTP sign-in dance).
pub struct OAuthProviderAdminContext {
    _container: ContainerAsync<Postgres>,
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
}

pub const OAUTH_PROVIDER_ADMIN_KEY: &str = "test-admin-key-oauth-providers";

impl OAuthProviderAdminContext {
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
            "INSERT INTO accounts (name) VALUES ('OAuth Providers Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new("stripe-secret-key-not-configured"));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();

        // AES-256-GCM key for oauth_signing_keys.private_key_enc — a
        // deterministic, non-zero test key (mirrors production wiring where
        // EMBYR_ENCRYPTION_KEY is always a real 32-byte secret; the hosted-
        // identity test harness uses [0u8; 32] since it never exercises
        // EMBYR_ENCRYPTION_KEY-based encryption — this feature's own
        // signing-key encryption DOES, so it needs a real key here).
        let encryption_key: [u8; 32] =
            *blake3::hash(b"oauth-providers-test-encryption-key").as_bytes();

        let router = build_admin_router(
            system_db,
            OAUTH_PROVIDER_ADMIN_KEY.to_string(),
            None,
            credential_cache,
            encryption_key,
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

        OAuthProviderAdminContext {
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
    /// Mirrors `HostedIdentityAdminContext::seed_session`.
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$b2F1dGhwcm92aWRlcnM$placeholderplaceholderplacehold";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("OAuth Providers Test User")
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

    /// Insert an active project under `self.account_id`. `backend_mode` is
    /// irrelevant to this feature (ADR-037 Resolution 2: no backend_mode
    /// gate) — always `direct_pg` for simplicity. A placeholder api_key hash
    /// is sufficient — this slice's own endpoint takes no `api_key` field
    /// (ADR-037 Decision 2's own positive consequence).
    pub async fn insert_project(&self, project_id: &str) {
        let api_key_hash = argon2::hash_api_key(b"placeholder-api-key").expect("hash api key");
        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, 'direct_pg', $3, 'active', $1)",
        )
        .bind(project_id)
        .bind(self.account_id)
        .bind(&api_key_hash)
        .execute(&self.pool)
        .await
        .expect("insert project");
    }

    pub async fn credential_row_exists(&self, project_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM oauth_provider_credentials \
             WHERE project_id = $1 AND provider = 'google'",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
            > 0
    }

    pub async fn stored_client_id(&self, project_id: &str) -> Option<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT client_id FROM oauth_provider_credentials \
             WHERE project_id = $1 AND provider = 'google'",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .expect("query stored client_id")
    }

    pub async fn signing_key_row_exists(&self, project_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM oauth_signing_keys WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
            > 0
    }

    /// Read the stored signing-key material directly (public_key,
    /// private_key_enc) — used by AC-19-02 to prove a Client-ID redefine
    /// never regenerates the signing key.
    pub async fn signing_key_material(&self, project_id: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
            "SELECT public_key, private_key_enc FROM oauth_signing_keys WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .expect("query signing key material")
    }
}
