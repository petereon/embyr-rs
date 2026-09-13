// SCAFFOLD: true
//! Common test infrastructure — client-auth acceptance tests.
//!
//! Infrastructure policy (`docs/architecture/atdd-infrastructure-policy.md`):
//!   Driving ports: gRPC data port (:8080, in-process tonic client) and
//!     REST/gRPC-Web port (:8081, reqwest against in-process Axum) — both
//!     ALREADY have reusable rows; admin port (:9090, reqwest against Axum)
//!     likewise. No new policy rows needed — client-auth introduces no
//!     genuinely new port CLASS, only new routes on existing ports.
//!   Driven internal: testcontainers-rs Postgres (System DB row) — reused
//!     unchanged. `client_identity_credentials` is a new TABLE, not a new
//!     PORT (DESIGN § Driven Ports + Adapters — client-auth: "No new driven
//!     port. ... the existing SystemDb driven port is reused unchanged").
//!   Driven external: none — this feature introduces zero new outbound
//!     network dependency (DESIGN § External Integrations — client-auth).
//!
//! Two test contexts:
//!   `ClientAuthAdminContext` — admin-only server (`build_admin_router`
//!     directly, mirrors `tests/admin_api_v2` / `tests/card_payments_backend`
//!     precedent), session-cookie auth via a lightweight direct-DB seed
//!     (mirrors `tests/card_payments_backend/common/mod.rs::seed_session` —
//!     this feature does not test admin authentication itself, only
//!     session-authed credential-lifecycle routes). Used by Slice 01/03/04
//!     (register / rotate / debug-verify).
//!   `ClientAuthFullContext` — the FULL production composition root
//!     (`embyr_server::start_test_server`, the exact function the existing
//!     72-scenario `embyr-rs` suite already uses) — real gRPC :8080, REST
//!     :8081, admin :9090, all wired together. Used by Slice 02 (sign-in +
//!     the AC-16-08 regression guardrail) and the algorithm-confusion
//!     regression, since those scenarios must exercise the REAL
//!     `authenticate()`-extension seam, not a hand-assembled router.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_core::auth::{argon2, ecies};
use embyr_server::{
    adapters::{
        cap_status_cache::CapStatusCache, credential_cache::CredentialCache,
        email::NoopEmailSender, stripe_gateway::StripeGateway, system_db::SystemDb,
    },
    admin::router::build_admin_router,
    middleware::signin_rate_limit::SigninRateLimiter,
};

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for client-auth acceptance assertions.
/// Response fields and DB public-column reads via dedicated helpers — never
/// internal struct fields.
pub mod universe {
    pub const REGISTER_STATUS: &str = "response.register.status_code";
    pub const REGISTER_RAW_KEY_ABSENT: &str = "response.register.raw_key_absent";
    pub const ROTATE_STATUS: &str = "response.rotate.status_code";
    pub const SIGN_IN_STATUS: &str = "response.sign_in.status_code";
    pub const SIGN_IN_REASON: &str = "response.sign_in.reason";
    pub const GET_DOC_STATUS: &str = "response.get_document.grpc_status";
    pub const VERIFY_DEBUG_STATUS: &str = "response.verify_debug.status_code";
    pub const CREDENTIAL_ROW_EXISTS: &str = "db.client_identity_credentials.row_exists";
    pub const CREDENTIAL_ROTATED_AT_SET: &str = "db.client_identity_credentials.rotated_at_set";
}

// ─── Token minting (simulates Trailmark's OWN backend — real crypto, not a mock) ──

/// Mint a client-identity token exactly per ADR-024 § Token format:
/// `header.payload.signature`, base64url (no pad), `alg: EdDSA`. This plays
/// the role of Trailmark's own backend — an external actor embyr never
/// calls — so real Ed25519 signing here is correct test design, not a mock
/// of any embyr-owned port.
pub fn mint_client_identity_token(
    signing_key: &SigningKey,
    sub: &str,
    aud: &str,
    exp_unix: i64,
) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD
        .encode(serde_json::json!({"sub": sub, "aud": aud, "exp": exp_unix}).to_string());
    let signing_input = format!("{header}.{payload}");
    let signature = signing_key.sign(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
    format!("{signing_input}.{sig_b64}")
}

/// Algorithm-confusion attack token (ADR-024 Enforcement regression, ca05):
/// `alg: HS256` using the registered PUBLIC key bytes as the HMAC secret.
pub fn mint_hs256_confusion_token(
    public_key_bytes: &[u8; 32],
    sub: &str,
    aud: &str,
    exp_unix: i64,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD
        .encode(serde_json::json!({"sub": sub, "aud": aud, "exp": exp_unix}).to_string());
    let signing_input = format!("{header}.{payload}");
    let mut mac =
        Hmac::<Sha256>::new_from_slice(public_key_bytes).expect("HMAC accepts any key length");
    mac.update(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{sig_b64}")
}

pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn public_key_b64(signing_key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes())
}

// ─── ClientAuthAdminContext (Slice 01/03/04 — admin-only server) ─────────────

/// Ephemeral test context: Postgres + Axum admin server on an ephemeral
/// port. Mirrors `tests/card_payments_backend/common/mod.rs::CpbTestContext`
/// (admin-only feature, lightweight session seed — no full Argon2id+TOTP
/// sign-in dance, since this feature does not test admin authentication
/// itself).
pub struct ClientAuthAdminContext {
    _container: ContainerAsync<Postgres>,
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
}

pub const CLIENT_AUTH_ADMIN_KEY: &str = "test-admin-key-client-auth";

impl ClientAuthAdminContext {
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
            "INSERT INTO accounts (name) VALUES ('Client Auth Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new("stripe-secret-key-not-configured"));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();

        let router = build_admin_router(
            system_db,
            CLIENT_AUTH_ADMIN_KEY.to_string(),
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
            None,
            Arc::new(CapStatusCache::new()),
            SigninRateLimiter::new(150.0, 10.0 / 60.0),
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

        ClientAuthAdminContext {
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
    /// Mirrors `tests/card_payments_backend/common/mod.rs::seed_session`.
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$Y2xpZW50YXV0aA$placeholderplaceholderplaceholder";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Client Auth Test User")
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

    /// Insert an active project under `self.account_id` (id/backend fields
    /// are placeholders — these tests never authenticate against the
    /// project's `api_key`, only its `client_identity_credentials` row).
    pub async fn insert_project(&self, project_id: &str) {
        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, 'direct_pg', 'placeholder_hash', 'active', $1)",
        )
        .bind(project_id)
        .bind(self.account_id)
        .execute(&self.pool)
        .await
        .expect("insert project");
    }

    /// Directly seed a `client_identity_credentials` row (bypassing the
    /// register endpoint — used by rotate/verify tests per Slice 03/04's own
    /// "hand-seeded credential row if needed" allowance, feature-delta.md §
    /// Elephant Carpaccio Slices).
    pub async fn seed_credential(&self, project_id: &str, public_key: &[u8; 32]) {
        sqlx::query(
            "INSERT INTO client_identity_credentials (project_id, public_key_current, algorithm) \
             VALUES ($1, $2, 'EdDSA')",
        )
        .bind(project_id)
        .bind(&public_key[..])
        .execute(&self.pool)
        .await
        .expect("insert client_identity_credentials row");
    }

    pub async fn credential_row_exists(&self, project_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM client_identity_credentials WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
            > 0
    }
}

// ─── ClientAuthFullContext (Slice 02 + algorithm-confusion — full production stack) ──

/// Full production composition root: gRPC :8080, REST :8081, admin :9090, all
/// via `embyr_server::start_test_server` — the SAME function the existing
/// 72-scenario `embyr-rs` acceptance suite uses (Pillar 3: no hand-assembled
/// router for this seam). A customer DB is also stood up, matching
/// `tests/acceptance/walking_skeleton.rs`'s pattern, since the AC-16-08
/// regression guardrail requires a REAL `getDoc` call to succeed.
pub struct ClientAuthFullContext {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub sys_pool: sqlx::PgPool,
    pub api_key: String,
    pub project_id: String,
    /// Owning account for `project_id` — enables session-authed admin calls
    /// (register/rotate/debug-verify) against the SAME composition root
    /// that also serves gRPC/REST, so rotation scenarios (ca03) can rotate
    /// via the admin port and immediately verify via sign-in without
    /// switching test contexts.
    pub account_id: uuid::Uuid,
}

impl ClientAuthFullContext {
    /// `project_id` is seeded active, `direct_pg` backend, with one document
    /// written at `documents/{project_id}-doc-1` (field `title` = "hello")
    /// so `getDoc` scenarios have something real to read.
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
            "INSERT INTO accounts (name) VALUES ('Client Auth Full-Stack Test Account') RETURNING id",
        )
        .fetch_one(&sys_pool)
        .await
        .expect("insert account");

        let api_key = format!("test-sk-client-auth-{project_id}");
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn =
            ecies::encrypt(&pub_key, cust_url.as_bytes()).expect("ecies encrypt dsn");

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

        let server = embyr_server::start_test_server(system_db).await;

        ClientAuthFullContext {
            _sys_container: sys_container,
            _cust_container: cust_container,
            server,
            sys_pool,
            api_key,
            project_id: project_id.to_string(),
            account_id,
        }
    }

    pub fn admin_url(&self, path: &str) -> String {
        format!("http://{}{}", self.server.admin_addr, path)
    }

    /// Seed a signed-in admin session for `self.account_id` (mirrors
    /// `ClientAuthAdminContext::seed_session` — same lightweight direct-DB
    /// seed, since this feature does not test admin authentication itself).
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$Y2xpZW50YXV0aA$placeholderplaceholderplaceholder";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Client Auth Full-Stack Test User")
        .bind(password_hash)
        .fetch_one(&self.sys_pool)
        .await
        .expect("insert user");

        sqlx::query(
            "INSERT INTO account_members (user_id, account_id, role, joined_at) \
             VALUES ($1, $2, $3, now())",
        )
        .bind(user_id)
        .bind(self.account_id)
        .bind(role)
        .execute(&self.sys_pool)
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
        .execute(&self.sys_pool)
        .await
        .expect("insert session");

        format!("embyr_session={cookie_value}")
    }

    pub async fn seed_credential(&self, public_key: &[u8; 32]) {
        sqlx::query(
            "INSERT INTO client_identity_credentials (project_id, public_key_current, algorithm) \
             VALUES ($1, $2, 'EdDSA')",
        )
        .bind(&self.project_id)
        .bind(&public_key[..])
        .execute(&self.sys_pool)
        .await
        .expect("insert client_identity_credentials row");
    }

    pub fn document_resource_name(&self) -> String {
        format!(
            "projects/{}/databases/(default)/documents/trip-journal/{}-doc-1",
            self.project_id, self.project_id
        )
    }

    pub fn rest_url(&self, path: &str) -> String {
        format!("http://{}{}", self.server.rest_addr, path)
    }
}
