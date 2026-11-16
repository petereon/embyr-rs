//! Shared fixtures for dsn-single-store acceptance tests (ADR-083).
//!
//! Real Postgres via testcontainers for BOTH the system DB and a customer DB; legacy rows are
//! inserted directly with `ecies::encrypt`, exactly as pre-ADR-083 `provision` wrote them.
//! CI only: do not run these locally (Docker).
#![allow(dead_code)]

use std::sync::Arc;

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, value::ValueType, GetDocumentRequest,
};
use embyr_server::{
    adapters::{connection_secrets::ServerKeys, system_db::SystemDb},
    start_test_server, TestServer,
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

/// The in-process test server's `EMBYR_ENCRYPTION_KEY` (see `lib.rs` test starters).
pub const TEST_KEY: [u8; 32] = [0u8; 32];
pub const ADMIN_KEY: &str = "test-admin-key-secret";

pub fn keys() -> ServerKeys {
    ServerKeys { current: TEST_KEY, previous: None }
}

pub async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("host port");
    (container, format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
}

/// Customer DB with the current customer schema applied; returns its URL and pool.
pub async fn start_ready_customer_db() -> (ContainerAsync<Postgres>, String, sqlx::PgPool) {
    let (container, url) = start_postgres().await;
    let pool = sqlx::PgPool::connect(&url).await.expect("customer pool");
    sqlx::migrate!("../../migrations/customer").run(&pool).await.expect("customer migrate");
    (container, url, pool)
}

pub struct Env {
    pub _sys: ContainerAsync<Postgres>,
    pub system_db: Arc<SystemDb>,
    pub sys_pool: sqlx::PgPool,
}

pub async fn start_system_db() -> Env {
    let (container, url) = start_postgres().await;
    let system_db = Arc::new(SystemDb::new(&url).await.expect("system db"));
    system_db.migrate().await.expect("system migrate");
    let sys_pool = system_db.pool().clone();
    Env { _sys: container, system_db, sys_pool }
}

pub fn ecies_of(api_key: &str, plaintext: &str) -> Vec<u8> {
    ecies::encrypt(&ecies::derive_public_key(api_key.as_bytes()), plaintext.as_bytes())
        .expect("ecies encrypt")
}

/// Pre-ADR-083 `direct_pg` row: DSN only in `ecies_encrypted_dsn`.
pub async fn insert_legacy_direct(pool: &sqlx::PgPool, project_id: &str, api_key: &str, dsn: &str) {
    let hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash");
    sqlx::query(
        "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
         VALUES ($1, 'active', 'direct_pg', $2, $3)",
    )
    .bind(project_id)
    .bind(&hash)
    .bind(ecies_of(api_key, dsn))
    .execute(pool)
    .await
    .expect("insert legacy project");
}

pub async fn seed_greeting(cust_pool: &sqlx::PgPool, project_id: &str, text: &str) {
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields) \
         VALUES ($1, 'messages', 'greeting', $2) \
         ON CONFLICT (project_id, collection_path, document_id) DO UPDATE SET fields = EXCLUDED.fields",
    )
    .bind(project_id)
    .bind(serde_json::json!({ "greeting": { "t": "S", "v": text } }))
    .execute(cust_pool)
    .await
    .expect("seed doc");
}

/// GetDocument(messages/greeting) over real gRPC with `api_key`; returns the greeting text.
pub async fn get_greeting(
    server: &TestServer,
    project_id: &str,
    api_key: &str,
) -> Result<String, tonic::Status> {
    let channel = tonic::transport::Channel::from_shared(format!("http://{}", server.grpc_addr))
        .unwrap()
        .connect()
        .await
        .expect("grpc connect");
    let mut client = FirestoreClient::new(channel);
    let mut req = tonic::Request::new(GetDocumentRequest {
        name: format!("projects/{project_id}/databases/(default)/documents/messages/greeting"),
        ..Default::default()
    });
    req.metadata_mut().insert("authorization", format!("bearer {api_key}").parse().unwrap());
    let doc = client.get_document(req).await?.into_inner();
    match doc.fields.get("greeting").and_then(|v| v.value_type.as_ref()) {
        Some(ValueType::StringValue(s)) => Ok(s.clone()),
        other => panic!("unexpected greeting value: {other:?}"),
    }
}

/// Bounded wait for a background conversion to land (the request path never blocks on it).
pub async fn wait_for<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if check().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

pub async fn column_bytes(pool: &sqlx::PgPool, project_id: &str, column: &str) -> Option<Vec<u8>> {
    // `column` is a test-controlled literal.
    sqlx::query_scalar(&format!("SELECT {column} FROM projects WHERE id = $1"))
        .bind(project_id)
        .fetch_one(pool)
        .await
        .expect("read column")
}

pub async fn server_store_dsn(pool: &sqlx::PgPool, project_id: &str) -> Option<String> {
    column_bytes(pool, project_id, "backend_pg_dsn_enc").await.and_then(|c| keys().decrypt(&c))
}

/// Seeds account + user + membership + a live session; returns the `Cookie` header value.
pub async fn seed_session(pool: &sqlx::PgPool, account_id: uuid::Uuid, role: &str) -> String {
    let user_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (account_id, email, display_name, password_hash) \
         VALUES ($1, $2, 'u', 'x') RETURNING id",
    )
    .bind(account_id)
    .bind(format!("{role}-{}@example.com", uuid::Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO account_members (user_id, account_id, role, joined_at) VALUES ($1, $2, $3, now())",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(role)
    .execute(pool)
    .await
    .expect("insert member");
    let token = format!("tok-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(blake3::hash(token.as_bytes()).as_bytes().to_vec())
    .execute(pool)
    .await
    .expect("insert session");
    format!("embyr_session={token}")
}

pub async fn new_account(pool: &sqlx::PgPool) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO accounts (name) VALUES ($1) RETURNING id")
        .bind(format!("acct-{}", uuid::Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("insert account")
}

/// Insert a project row owned by `account_id` with `mode` and no secrets yet.
pub async fn insert_owned_project(
    pool: &sqlx::PgPool,
    account_id: uuid::Uuid,
    project_id: &str,
    api_key: &str,
    status: &str,
) {
    let hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash");
    sqlx::query(
        "INSERT INTO projects (id, account_id, status, backend_mode, api_key_hash_current) \
         VALUES ($1, $2, $3, 'direct_pg', $4)",
    )
    .bind(project_id)
    .bind(account_id)
    .bind(status)
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert owned project");
}

pub async fn scrape_metrics(server: &TestServer) -> String {
    reqwest::Client::new()
        .get(format!("http://{}/metrics", server.admin_addr))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .send()
        .await
        .expect("scrape")
        .text()
        .await
        .expect("metrics body")
}

pub async fn server_for(env: &Env) -> TestServer {
    start_test_server(Arc::clone(&env.system_db)).await
}
