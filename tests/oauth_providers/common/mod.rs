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

// ─── MockJwksServer (Slice 02) ─────────────────────────────────────────────────

/// A local, test-only fake JWKS server (per this codebase's own
/// "driven external / non-deterministic" fake-with-output-capture
/// convention, `nw-distill` skill) — never a live call to Google, never a
/// hand-mocked verification result. Generates a REAL RSA keypair, serves
/// its public half as a real JWKS document over real HTTP, and mints real
/// RS256-signed Google-ID-token-shaped JWTs with the private half — so
/// `GoogleJwksCache`'s own HTTP-fetching logic and
/// `oauth_identity::verify_google_id_token`'s own RS256 verification are
/// both genuinely exercised end-to-end.
pub struct MockJwksServer {
    pub base_url: String,
    kid: String,
    private_key_pem: String,
}

impl MockJwksServer {
    pub async fn start() -> Self {
        use rand_core::OsRng;
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::traits::PublicKeyParts;
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let mut rng = OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
        let public_key = private_key.to_public_key();
        let n = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
        let kid = "mock-google-kid-1".to_string();

        let jwks_json = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": kid,
                "n": n,
                "e": e,
            }]
        });

        let private_key_pem = private_key
            .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
            .expect("encode PKCS1 PEM")
            .to_string();

        async fn serve_jwks(
            axum::extract::State(jwks): axum::extract::State<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            axum::Json(jwks)
        }

        let router = axum::Router::new()
            .route("/certs", axum::routing::get(serve_jwks))
            .with_state(jwks_json);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock JWKS ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("mock JWKS server task panicked");
        });
        tokio::task::yield_now().await;

        MockJwksServer {
            base_url,
            kid,
            private_key_pem,
        }
    }

    pub fn jwks_url(&self) -> String {
        format!("{}/certs", self.base_url)
    }

    /// Mint a real RS256-signed, Google-ID-token-shaped JWT — plays the
    /// role of Google's own token-issuing service (an external actor),
    /// mirroring this codebase's own established precedent of minting real
    /// signed tokens to simulate a third party (`client-auth`'s own
    /// Trailmark-backend-simulation, `client-auth-hosted-identity`'s own
    /// `FakeEmailSender`).
    pub fn mint_id_token(&self, sub: &str, aud: &str, exp_unix: i64) -> String {
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(self.private_key_pem.as_bytes())
            .expect("build jsonwebtoken RSA key");
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        let claims = serde_json::json!({
            "sub": sub,
            "aud": aud,
            "exp": exp_unix,
            "iss": "https://accounts.google.com",
        });
        jsonwebtoken::encode(&header, &claims, &encoding_key).expect("mint RS256 token")
    }
}

// ─── OAuthProviderFullContext (Slice 02 — full production stack) ─────────────

/// Full production composition root: gRPC :8080, REST :8081, admin :9090,
/// via `embyr_server::start_test_server_with_oauth` — mirrors
/// `tests/client_auth_hosted_identity/common/mod.rs::HostedIdentityFullContext`
/// exactly (this slice's own AC-19-05/10 need the SAME "real getDoc call
/// after sign-in" regression proof).
pub struct OAuthProviderFullContext {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub sys_pool: sqlx::PgPool,
    pub api_key: String,
    pub project_id: String,
    pub account_id: uuid::Uuid,
    pub encryption_key: [u8; 32],
}

impl OAuthProviderFullContext {
    /// `project_id` is seeded active, `direct_pg` backend, with one
    /// document written at `documents/{project_id}-doc-1` so AC-19-05's
    /// subsequent getDoc call has something real to read. Google sign-in is
    /// NOT registered by `new()` — call `register_google_oauth_provider()`
    /// explicitly (so AC-19-08's "not enabled" scenario can use a context
    /// that never does). `google_jwks_url` is threaded straight into
    /// `GoogleJwksCache` — pass `MockJwksServer::start().await.jwks_url()`
    /// for the happy/error-taxonomy scenarios, or an address nothing
    /// listens on (e.g. `"http://127.0.0.1:9/certs"`) for AC-19-11.
    pub async fn new(project_id: &str, google_jwks_url: String) -> Self {
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
            "INSERT INTO accounts (name) VALUES ('OAuth Providers Full-Stack Test Account') RETURNING id",
        )
        .fetch_one(&sys_pool)
        .await
        .expect("insert account");

        let api_key = format!("test-sk-oauth-providers-{project_id}");
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
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

        let encryption_key: [u8; 32] =
            *blake3::hash(b"oauth-providers-full-context-test-encryption-key").as_bytes();

        let server = embyr_server::start_test_server_with_oauth(
            system_db,
            encryption_key,
            google_jwks_url,
        )
        .await;

        OAuthProviderFullContext {
            _sys_container: sys_container,
            _cust_container: cust_container,
            server,
            sys_pool,
            api_key,
            project_id: project_id.to_string(),
            account_id,
            encryption_key,
        }
    }

    /// Hand-seed `oauth_provider_credentials` + `oauth_signing_keys`
    /// directly (bypasses the Slice-01 admin registration endpoint — same
    /// "hand-seeded row" allowance `HostedIdentityFullContext::enable_hosted_identity`
    /// already uses). Generates a fresh Ed25519 keypair, AES-256-GCM
    /// -encrypts the seed under `self.encryption_key` (mirrors
    /// `register_google_oauth_provider`'s own exact crypto call shape,
    /// ADR-037 Decision 2 — NOT ECIES, unlike hosted-identity's own
    /// equivalent).
    pub async fn register_google_oauth_provider(&self, client_id: &str) {
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
            "INSERT INTO oauth_provider_credentials (project_id, provider, client_id) \
             VALUES ($1, 'google', $2)",
        )
        .bind(&self.project_id)
        .bind(client_id)
        .execute(&self.sys_pool)
        .await
        .expect("insert oauth_provider_credentials row");

        sqlx::query(
            "INSERT INTO oauth_signing_keys (project_id, public_key, private_key_enc, algorithm) \
             VALUES ($1, $2, $3, 'EdDSA')",
        )
        .bind(&self.project_id)
        .bind(&public_key[..])
        .bind(&private_key_enc)
        .execute(&self.sys_pool)
        .await
        .expect("insert oauth_signing_keys row");
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
}
