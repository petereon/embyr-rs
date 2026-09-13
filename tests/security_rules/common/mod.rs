// SCAFFOLD: true
//! Common test infrastructure — security-rules acceptance tests.
//!
//! Infrastructure policy (`docs/architecture/atdd-infrastructure-policy.md`):
//!   Driving ports: admin port (:9090, reqwest against Axum) and gRPC data
//!     port (:8080, in-process tonic client) already have reusable rows
//!     (inherited from `client-auth`'s own bootstrap) — no new policy rows
//!     needed. security-rules introduces no genuinely new port CLASS, only
//!     new routes/call-site behavior on existing ports.
//!   Driven internal: testcontainers-rs Postgres (System DB row) — reused
//!     unchanged. `access_rules` is a new TABLE, not a new PORT (DESIGN §
//!     Driven Ports + Adapters — security-rules: "No new driven port. ...
//!     the existing already-probed SystemDb connection pool is reused").
//!   Driven external: none — this feature introduces zero new outbound
//!     network dependency.
//!
//! Two test contexts, mirroring `tests/client_auth/common/mod.rs`'s own
//! shape exactly:
//!   `SecurityRulesAdminContext` — admin-only server (`build_admin_router`
//!     directly). Used by sr01 (define/redefine) and sr05 (simulate) — both
//!     Activity A / admin-port-only stories (feature-delta.md § Story Map).
//!   `SecurityRulesFullContext` — the FULL production composition root
//!     (`embyr_server::start_test_server`) — real gRPC :8080, REST :8081,
//!     admin :9090. Used by sr02/sr03/sr04, which must exercise the REAL
//!     `handle_get_document` rule-evaluation seam, not a hand-assembled
//!     router.
//!
//! Token minting is REUSED from `client-auth`'s own fixture helpers via a
//! path import (per the DISTILL dispatch instructions) rather than
//! duplicated — Slice 02/03's scenarios need real signed-in Maria/Dana
//! sessions exactly like `client-auth`'s own did, and the token shape
//! (ADR-024, unchanged by this feature) is identical.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

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

// ─── client-auth fixture reuse (token minting — same shape, ADR-024 unchanged) ──
#[path = "../../client_auth/common/mod.rs"]
pub mod client_auth_common;
pub use client_auth_common::{mint_client_identity_token, now_unix, public_key_b64};

/// composite-index-real-creation (ADR-072 Decision C): AES-256-GCM-encrypt
/// `dsn` under `key` (12-byte random nonce prefix + ciphertext), the exact
/// shape `adapters::customer_db_connect::resolve_dsn_without_api_key`
/// decrypts via `decrypt_with_rotation` — mirrors
/// `tests/customer_db_transaction_sweeper`'s own identical `encrypt_dsn`
/// helper. `[0u8; 32]` matches `embyr_server::start_test_server`'s own
/// hardcoded test `encryption_key` (`lib.rs::start_test_server_with_keepalive`).
fn encrypt_dsn_for_test_server(dsn: &str) -> Vec<u8> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use rand_core::{OsRng, RngCore};

    const TEST_SERVER_ENCRYPTION_KEY: [u8; 32] = [0u8; 32];
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(&TEST_SERVER_ENCRYPTION_KEY).expect("valid key");
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, dsn.as_bytes()).expect("encrypt dsn");
    let mut enc = nonce_bytes.to_vec();
    enc.extend_from_slice(&ct);
    enc
}

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for security-rules acceptance assertions.
/// Response fields and DB public-column reads via dedicated helpers — never
/// internal struct fields.
pub mod universe {
    pub const DEFINE_STATUS: &str = "response.define.status_code";
    pub const ACCESS_RULE_CONDITION_SOURCE: &str = "db.access_rules.condition_source";
    pub const GET_DOC_GRPC_CODE: &str = "response.get_document.grpc_code";
    pub const SIMULATE_OUTCOME: &str = "response.simulate.outcome";
}

// ─── SecurityRulesAdminContext (sr01/sr05 — admin-only server) ────────────────

/// Ephemeral test context: Postgres + Axum admin server on an ephemeral
/// port. Mirrors `tests/client_auth/common/mod.rs::ClientAuthAdminContext`
/// (admin-only feature, lightweight session seed — this feature does not
/// test admin authentication itself, only session-authed
/// access-rule-lifecycle routes).
pub struct SecurityRulesAdminContext {
    _container: ContainerAsync<Postgres>,
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
}

impl SecurityRulesAdminContext {
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
            "INSERT INTO accounts (name) VALUES ('Security Rules Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new("stripe-secret-key-not-configured"));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();

        let router = build_admin_router(
            system_db,
            "test-admin-key-security-rules".to_string(),
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

        SecurityRulesAdminContext {
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
            "$argon2id$v=19$m=65536,t=3,p=4$c2VjdXJpdHlydWxlcw$placeholderplaceholderplaceh";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Security Rules Test User")
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

    /// Insert an active project under `self.account_id`.
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

    /// Directly seed an `access_rules` row (bypassing the define endpoint —
    /// used by AC-17-02's redefine-precondition setup, mirroring
    /// `ClientAuthAdminContext::seed_credential`'s identical
    /// bypass-the-endpoint allowance).
    pub async fn seed_access_rule(
        &self,
        project_id: &str,
        collection_path: &str,
        condition_source: &str,
    ) {
        sqlx::query(
            "INSERT INTO access_rules (project_id, collection_path, condition_source) \
             VALUES ($1, $2, $3)",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(condition_source)
        .execute(&self.pool)
        .await
        .expect("insert access_rules row");
    }

    /// Read the currently-stored `condition_source` for `(project_id,
    /// collection_path)` — the state-delta observable for AC-17-01/02
    /// (Mandate 8: port-exposed, via a dedicated read helper, never an
    /// internal struct field).
    pub async fn access_rule_condition_source(
        &self,
        project_id: &str,
        collection_path: &str,
    ) -> Option<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT condition_source FROM access_rules WHERE project_id = $1 AND collection_path = $2",
        )
        .bind(project_id)
        .bind(collection_path)
        .fetch_optional(&self.pool)
        .await
        .unwrap_or(None)
    }
}

// ─── SecurityRulesFullContext (sr02/sr03/sr04 — full production stack) ────────

/// Full production composition root: gRPC :8080, REST :8081, admin :9090,
/// via `embyr_server::start_test_server` — the SAME function the existing
/// 113-scenario regression suite (72 `embyr-rs` + 41 `client-auth`) itself
/// uses (Pillar 3: no hand-assembled router for this seam).
///
/// Seeds four documents across three collections on construction, matching
/// the Trailmark domain examples DISCUSS's UAT scenarios use:
///   `journal_entries/{project}-maria-doc`      — `{owner_id: "maria-santos"}`
///   `journal_entries/{project}-no-owner-doc`   — `{}` (AC-17-09 fail-closed)
///   `trail_guides/{project}-guide-doc`         — `{title: "Guide"}`
///   `app_config/{project}-config-doc`          — `{enabled: true}` (AC-17-14)
pub struct SecurityRulesFullContext {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub sys_pool: sqlx::PgPool,
    // security-rules-cel-parity (Slice 02, ADR-062): exposed so CP02's own
    // scenarios can seed `profiles/<uid>`-shaped documents beyond the fixed
    // journal_entries/trail_guides/app_config set `new()` seeds below —
    // mirrors `sys_pool`'s own pub-field precedent, not a new port.
    pub cust_pool: sqlx::PgPool,
    pub api_key: String,
    pub project_id: String,
    pub account_id: uuid::Uuid,
}

impl SecurityRulesFullContext {
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
            "INSERT INTO accounts (name) VALUES ('Security Rules Full-Stack Test Account') RETURNING id",
        )
        .fetch_one(&sys_pool)
        .await
        .expect("insert account");

        let api_key = format!("test-sk-security-rules-{project_id}");
        let api_key_hash =
            embyr_core::auth::argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        let pub_key = embyr_core::auth::ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = embyr_core::auth::ecies::encrypt(&pub_key, cust_url.as_bytes())
            .expect("ecies encrypt dsn");
        // composite-index-real-creation (ADR-072 Decision C): also populate
        // backend_pg_dsn_enc — the api_key-free DSN resolution path
        // (`resolve_dsn_without_api_key`) reads THIS column, never
        // `ecies_encrypted_dsn` (which requires a live api_key to derive
        // its ECIES private key — not available in CreateIndex's own
        // admin-session auth context).
        let backend_pg_dsn_enc = encrypt_dsn_for_test_server(&cust_url);

        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn, \
              backend_pg_dsn_enc) \
             VALUES ($1, $2, 'active', 'direct_pg', $3, $4, $5)",
        )
        .bind(project_id)
        .bind(account_id)
        .bind(&api_key_hash)
        .bind(&encrypted_dsn)
        .bind(&backend_pg_dsn_enc)
        .execute(&sys_pool)
        .await
        .expect("insert project");

        for (collection, doc_id, fields) in [
            (
                "journal_entries",
                format!("{project_id}-maria-doc"),
                serde_json::json!({"owner_id": {"t": "S", "v": "maria-santos"}}),
            ),
            (
                "journal_entries",
                format!("{project_id}-no-owner-doc"),
                serde_json::json!({}),
            ),
            (
                "trail_guides",
                format!("{project_id}-guide-doc"),
                serde_json::json!({"title": {"t": "S", "v": "Guide"}}),
            ),
            (
                "app_config",
                format!("{project_id}-config-doc"),
                serde_json::json!({"enabled": {"t": "B", "v": true}}),
            ),
        ] {
            sqlx::query(
                "INSERT INTO documents (project_id, collection_path, document_id, fields, create_time, update_time, version) \
                 VALUES ($1, $2, $3, $4, now(), now(), 1)",
            )
            .bind(project_id)
            .bind(collection)
            .bind(doc_id)
            .bind(fields)
            .execute(&cust_pool)
            .await
            .expect("insert seed document");
        }

        let server = embyr_server::start_test_server(system_db).await;

        SecurityRulesFullContext {
            _sys_container: sys_container,
            _cust_container: cust_container,
            server,
            sys_pool,
            cust_pool,
            api_key,
            project_id: project_id.to_string(),
            account_id,
        }
    }

    pub fn admin_url(&self, path: &str) -> String {
        format!("http://{}{}", self.server.admin_addr, path)
    }

    /// Seed a signed-in admin session for `self.account_id` (mirrors
    /// `SecurityRulesAdminContext::seed_session` /
    /// `ClientAuthFullContext::seed_session` — same lightweight direct-DB
    /// seed, since this feature does not test admin authentication
    /// itself). Used by sr05's simulation scenarios, which need BOTH the
    /// admin port (simulate) and the gRPC port (real getDoc) on the SAME
    /// composition root.
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$c2VjdXJpdHlydWxlcw$placeholderplaceholderplaceh";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Security Rules Full-Stack Test User")
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

    /// Directly seed an `access_rules` row — the "hand-seeded rule row"
    /// allowance the same way `ClientAuthFullContext::seed_credential`
    /// hand-seeds a credential row, since sr02/sr03/sr04 test rule
    /// EVALUATION, not rule DEFINITION (that is sr01's own job).
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

    /// Directly seed a `client_identity_credentials` row so Maria/Dana
    /// tokens (minted via the reused `mint_client_identity_token` helper)
    /// verify — mirrors `ClientAuthFullContext::seed_credential`.
    pub async fn seed_client_identity_credential(&self, public_key: &[u8; 32]) {
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

    /// Seed a document directly into the customer DB (bypassing
    /// `CreateDocument` — the same bypass-the-endpoint allowance
    /// `seed_access_rule` above already uses), mirroring `new()`'s own
    /// journal_entries/trail_guides/app_config seeding shape exactly.
    /// security-rules-cel-parity (Slice 02): lets CP02 seed
    /// `profiles/<uid>`-shaped documents this fixture's fixed seed set
    /// doesn't cover.
    pub async fn seed_document(
        &self,
        collection: &str,
        document_id: &str,
        fields: serde_json::Value,
    ) {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, create_time, update_time, version) \
             VALUES ($1, $2, $3, $4, now(), now(), 1)",
        )
        .bind(&self.project_id)
        .bind(collection)
        .bind(document_id)
        .bind(fields)
        .execute(&self.cust_pool)
        .await
        .expect("insert seed document");
    }

    pub fn document_resource_name(&self, collection: &str, doc_suffix: &str) -> String {
        format!(
            "projects/{}/databases/(default)/documents/{}/{}-{}",
            self.project_id, collection, self.project_id, doc_suffix
        )
    }

    /// A resource name for a document ID that was NEVER seeded at all
    /// (AC-17-10's existence-non-leakage comparison partner).
    pub fn nonexistent_document_resource_name(&self, collection: &str) -> String {
        format!(
            "projects/{}/databases/(default)/documents/{}/{}-does-not-exist",
            self.project_id, collection, self.project_id
        )
    }

    /// Real gRPC `GetDocument` call — driving port entry (Pillar 3): real
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
