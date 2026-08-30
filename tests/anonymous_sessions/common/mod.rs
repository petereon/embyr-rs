// SCAFFOLD: true
//! Common test infrastructure — anonymous-sessions acceptance tests
//! (Slices 01-02, ADR-043).
//!
//! Driving ports: Admin HTTP :9090 (`AnonymousIdentityAdminContext`, real
//! `build_admin_router` composition root — mirrors
//! `tests/oauth_providers/common/mod.rs::OAuthProviderAdminContext` exactly,
//! since Slice 01's own scope is a single session-authed admin action) and
//! REST :8081 + gRPC :8080 (`AnonymousIdentityFullContext`, real
//! `embyr_server::start_test_server_with_oauth` composition root — reused
//! unchanged since anonymous-sessions shares the SAME
//! encryption_key/encryption_key_previous custody mechanism oauth-providers
//! already threads; the `google_jwks_base_url` parameter is inert for this
//! feature, never called).
//! Driven internal: testcontainers-rs Postgres (System DB + Customer DB) —
//! reused unchanged. `anonymous_signing_keys` is a new TABLE (migration
//! 0031), not a new PORT.

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

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for anonymous-sessions acceptance
/// assertions. Response fields and DB public-column reads via dedicated
/// helpers — never internal struct fields.
pub mod universe {
    pub const ENABLE_STATUS: &str = "response.enable.status_code";
    pub const SIGNING_KEY_ROW_EXISTS: &str = "db.anonymous_signing_keys.row_exists";
}

// ─── AnonymousIdentityAdminContext (Slice 01) ─────────────────────────────────

/// Ephemeral test context: Postgres + Axum admin server on an ephemeral
/// port. Mirrors `OAuthProviderAdminContext` (admin-only feature,
/// lightweight session seed — no full Argon2id+TOTP sign-in dance).
pub struct AnonymousIdentityAdminContext {
    _container: ContainerAsync<Postgres>,
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
}

pub const ANONYMOUS_ADMIN_KEY: &str = "test-admin-key-anonymous-sessions";

impl AnonymousIdentityAdminContext {
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
            "INSERT INTO accounts (name) VALUES ('Anonymous Sessions Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new("stripe-secret-key-not-configured"));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();

        // AES-256-GCM key for anonymous_signing_keys.private_key_enc — a
        // deterministic, non-zero test key (mirrors
        // `OAuthProviderAdminContext`'s own identical rationale: this
        // feature's own signing-key encryption DOES exercise
        // EMBYR_ENCRYPTION_KEY-based encryption, so it needs a real key here).
        let encryption_key: [u8; 32] =
            *blake3::hash(b"anonymous-sessions-test-encryption-key").as_bytes();

        let router = build_admin_router(
            system_db,
            ANONYMOUS_ADMIN_KEY.to_string(),
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

        AnonymousIdentityAdminContext {
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
    /// Mirrors `OAuthProviderAdminContext::seed_session`.
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$YW5vbnltb3Vz$placeholderplaceholderplacehold";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Anonymous Sessions Test User")
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
    /// irrelevant to this feature (ADR-044: no backend_mode gate) — always
    /// `direct_pg` for simplicity. A placeholder api_key hash is sufficient
    /// — this slice's own endpoint takes no `api_key` field.
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

    pub async fn signing_key_row_exists(&self, project_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM anonymous_signing_keys WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
            > 0
    }

    /// Read the stored signing-key material directly (public_key,
    /// private_key_enc) — used by AC-20-02 to prove idempotent re-enablement
    /// never regenerates the signing key.
    pub async fn signing_key_material(&self, project_id: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
            "SELECT public_key, private_key_enc FROM anonymous_signing_keys WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .expect("query signing key material")
    }

    pub async fn signing_key_row_count(&self, project_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM anonymous_signing_keys WHERE project_id = $1")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await
            .expect("count rows")
    }
}

// ─── AnonymousIdentityFullContext (Slice 02 — full production stack) ─────────

/// Full production composition root: gRPC :8080, REST :8081, admin :9090,
/// via `embyr_server::start_test_server_with_oauth` — mirrors
/// `tests/oauth_providers/common/mod.rs::OAuthProviderFullContext` exactly
/// (this slice's own AC-20-05/09 need the SAME "real getDoc call after
/// sign-in" / "real ownership-rule denial" regression proof). The
/// `google_jwks_base_url` parameter is inert for this feature — anonymous
/// sign-in never touches `GoogleJwksCache` — an address nothing listens on
/// is passed deliberately.
pub struct AnonymousIdentityFullContext {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub sys_pool: sqlx::PgPool,
    pub cust_pool: sqlx::PgPool,
    pub api_key: String,
    pub project_id: String,
    pub account_id: uuid::Uuid,
    pub encryption_key: [u8; 32],
}

impl AnonymousIdentityFullContext {
    /// `project_id` is seeded active, `direct_pg` backend. Anonymous auth is
    /// NOT enabled by `new()` — call `enable_anonymous_identity()`
    /// explicitly (so AC-20-06's "not enabled" scenario can use a context
    /// that never does).
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
        system_db
            .migrate()
            .await
            .expect("system DB migrations failed");
        let sys_pool = system_db.pool().clone();

        let cust_pool = sqlx::PgPool::connect(&cust_url)
            .await
            .expect("connect customer DB");
        sqlx::migrate!("../../migrations/customer")
            .run(&cust_pool)
            .await
            .expect("customer DB migrations failed");

        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Anonymous Sessions Full-Stack Test Account') RETURNING id",
        )
        .fetch_one(&sys_pool)
        .await
        .expect("insert account");

        let api_key = format!("test-sk-anonymous-sessions-{project_id}");
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        // ecies_encrypted_dsn is required for the gRPC GetDocument path's own
        // Customer DB resolution (a pre-existing requirement of every
        // data-plane read, unrelated to anonymous-sessions' own sign-in
        // handler — which itself never resolves a Customer DB adapter,
        // Resolution 3/ADR-044) — mirrors `OAuthProviderFullContext::new`'s
        // identical seeding.
        let pub_key = embyr_core::auth::ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = embyr_core::auth::ecies::encrypt(&pub_key, cust_url.as_bytes())
            .expect("ecies encrypt dsn");
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

        let encryption_key: [u8; 32] =
            *blake3::hash(b"anonymous-sessions-full-context-test-encryption-key").as_bytes();

        // google_jwks_base_url is inert for this feature — passed a URL
        // nothing listens on; never fetched by any anonymous-sessions call
        // site.
        let server = embyr_server::start_test_server_with_oauth(
            system_db,
            encryption_key,
            "http://127.0.0.1:9/certs".to_string(),
        )
        .await;

        AnonymousIdentityFullContext {
            _sys_container: sys_container,
            _cust_container: cust_container,
            server,
            sys_pool,
            cust_pool,
            api_key,
            project_id: project_id.to_string(),
            account_id,
            encryption_key,
        }
    }

    /// Hand-seed `anonymous_signing_keys` directly (bypasses the Slice-01
    /// admin enablement endpoint — same "hand-seeded row" allowance
    /// `OAuthProviderFullContext::register_google_oauth_provider` already
    /// uses). Generates a fresh Ed25519 keypair, AES-256-GCM-encrypts the
    /// seed under `self.encryption_key` (mirrors
    /// `enable_anonymous_identity`'s own exact crypto call shape).
    pub async fn enable_anonymous_identity(&self) {
        use aes_gcm::{
            aead::{Aead, KeyInit},
            Aes256Gcm, Nonce,
        };
        use ed25519_dalek::SigningKey;
        use rand_core::RngCore;

        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let public_key = signing_key.verifying_key().to_bytes();
        let private_key_seed = signing_key.to_bytes();

        let mut nonce_bytes = [0u8; 12];
        rand_core::OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&self.encryption_key).expect("32-byte key");
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, private_key_seed.as_slice())
            .expect("aes-gcm encrypt");
        let mut private_key_enc = nonce_bytes.to_vec();
        private_key_enc.extend_from_slice(&ct);

        sqlx::query(
            "INSERT INTO anonymous_signing_keys (project_id, public_key, private_key_enc, algorithm) \
             VALUES ($1, $2, $3, 'EdDSA')",
        )
        .bind(&self.project_id)
        .bind(&public_key[..])
        .bind(&private_key_enc)
        .execute(&self.sys_pool)
        .await
        .expect("insert anonymous_signing_keys row");
    }

    pub fn rest_url(&self, path: &str) -> String {
        format!("http://{}{}", self.server.rest_addr, path)
    }

    pub async fn sign_in_anonymously(&self, api_key: &str) -> (u16, serde_json::Value) {
        let url = self.rest_url(&format!(
            "/v1/projects/{}/accounts:signInAnonymously?key={}",
            self.project_id, api_key
        ));
        let resp = reqwest::Client::new()
            .post(url)
            .send()
            .await
            .expect("sign_in_anonymously request failed");
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp
            .json()
            .await
            .expect("sign_in_anonymously body must be JSON");
        (status, body)
    }

    /// Directly seed an access rule against the System DB — mirrors
    /// `tests/security_rules/common/mod.rs::SecurityRulesFullContext::seed_access_rule`.
    pub async fn seed_access_rule(&self, collection_path: &str, condition_source: &str) {
        sqlx::query(
            "INSERT INTO access_rules (project_id, collection_path, condition_source) \
             VALUES ($1, $2, $3)",
        )
        .bind(&self.project_id)
        .bind(collection_path)
        .bind(condition_source)
        .execute(&self.sys_pool)
        .await
        .expect("insert access_rules row");
    }

    /// Insert a document into the Customer DB directly, with a caller-chosen
    /// `owner_id` field — used by AC-20-09 (Maria's anonymous `uid` is only
    /// known AFTER she signs in, unlike the other identity features' own
    /// static-fixture Maria/Dana names).
    pub async fn insert_document_with_owner(
        &self,
        collection_path: &str,
        document_id: &str,
        owner_id: &str,
    ) {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, create_time, update_time, version) \
             VALUES ($1, $2, $3, $4, now(), now(), 1)",
        )
        .bind(&self.project_id)
        .bind(collection_path)
        .bind(document_id)
        .bind(serde_json::json!({"owner_id": {"t": "S", "v": owner_id}}))
        .execute(&self.cust_pool)
        .await
        .expect("insert document with owner_id");
    }

    pub fn document_resource_name(&self, collection: &str, document_id: &str) -> String {
        format!(
            "projects/{}/databases/(default)/documents/{}/{}",
            self.project_id, collection, document_id
        )
    }

    /// Real gRPC `GetDocument` call — driving port entry: real
    /// `FirestoreClient` against the real composition root, real
    /// `authorization` + optional `x-embyr-client-identity` metadata.
    pub async fn get_document(
        &self,
        resource_name: &str,
        client_identity_token: Option<&str>,
    ) -> Result<tonic::Response<embyr_proto::firestore::Document>, tonic::Status> {
        use embyr_proto::firestore::firestore_client::FirestoreClient;

        let channel = tonic::transport::Endpoint::new(format!("http://{}", self.server.grpc_addr))
            .expect("valid endpoint")
            .connect()
            .await
            .expect("connect to gRPC server");
        let mut client = FirestoreClient::new(channel);
        let mut request = tonic::Request::new(embyr_proto::firestore::GetDocumentRequest {
            name: resource_name.to_string(),
            ..Default::default()
        });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.api_key).parse().unwrap(),
        );
        if let Some(token) = client_identity_token {
            request.metadata_mut().insert(
                "x-embyr-client-identity",
                format!("Bearer {token}").parse().unwrap(),
            );
        }

        client.get_document(request).await
    }
}
