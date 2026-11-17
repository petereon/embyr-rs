// @driving_port @real-io @adapter-integration
//! api-key-rotation slice 4 (ADR-085): `POST /admin/v1/projects/:id/api_key/{rotate,revoke}`
//! against the real admin router, gRPC handler and REST handlers, with real Postgres.
//!
//!   AC-AKR-01/02/09/19  rotate: new key once, both keys work in the window, GET shows dates
//!   AC-AKR-05/10        revoke: old key dead at once even when cached, previous columns NULL
//!   AC-AKR-06           second rotate drops the first key
//!   AC-AKR-07           concurrent rotates (N=2, N=8): one 200, the rest 409 rotation_conflict
//!   AC-AKR-08           unconverted DSN / agent bundle: 409, no column changes; aws_secret passes
//!   AC-AKR-11           a replica whose cache still holds the old key serves it (documented bound)
//!   AC-AKR-12           403/403/404/404/401, suspended rotatable, audit event, no-store, no leaks
//!   AC-AKR-14 (2nd)     legacy hosted signing key regenerated; failed rotate rolls back
//!   AC-AKR-15           Listen opened with the old key survives revoke; a new one is rejected
//!
//! Expiry is never waited for (AC-AKR-03 lives in akr01). AC-AKR-11's "rejected after the TTL" half is
//! the `credential_cache` unit test (`served(.., CREDENTIAL_TTL) == None`). CI only: Docker.

#[path = "../../dsn_single_store/common/mod.rs"]
mod common;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use common::*;
use embyr_core::auth::{argon2, ecies};
use embyr_core::client_identity::{verify_client_identity_token, ClientIdentityCredential};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    structured_query::CollectionSelector,
    target::{self, query_target},
    target_change::TargetChangeType,
    CreateDocumentRequest, Document, ListenRequest, StructuredQuery, Target,
};
use embyr_server::TestServer;
use tokio_stream::StreamExt;

const K0: &str = "akr4-original-key";

/// Global capture of server-side tracing output (one subscriber per test binary).
mod logs {
    use std::io;
    use std::sync::{Arc, Mutex, Once, OnceLock};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    pub struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Captured {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    static LOGS: OnceLock<Captured> = OnceLock::new();
    static INIT: Once = Once::new();

    pub fn init() -> Captured {
        let logs = LOGS.get_or_init(Captured::default).clone();
        INIT.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_writer(logs.clone())
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .finish();
            tracing::subscriber::set_global_default(subscriber).expect("install tracing subscriber");
        });
        logs
    }

    pub fn text(logs: &Captured) -> String {
        String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned()
    }
}

struct Ctx {
    env: Env,
    _cust: testcontainers_modules::testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    cust_url: String,
    cust_pool: sqlx::PgPool,
    server: TestServer,
    account: uuid::Uuid,
    owner: String,
    admin: String,
    viewer: String,
    client: reqwest::Client,
}

async fn ctx() -> Ctx {
    logs::init();
    let env = start_system_db().await;
    let (cust, cust_url, cust_pool) = start_ready_customer_db().await;
    let server = server_for(&env).await;
    let account = new_account(&env.sys_pool).await;
    let owner = seed_session(&env.sys_pool, account, "Owner").await;
    let admin = seed_session(&env.sys_pool, account, "Admin").await;
    let viewer = seed_session(&env.sys_pool, account, "Viewer").await;
    Ctx { env, _cust: cust, cust_url, cust_pool, server, account, owner, admin, viewer, client: reqwest::Client::new() }
}

struct Reply {
    status: u16,
    cache_control: Option<String>,
    body: serde_json::Value,
}

impl Ctx {
    /// Converted `direct_pg` project owned by `self.account`, current key `K0`, with a greeting.
    async fn seed(&self, id: &str, status: &str) {
        let hash = argon2::hash_api_key(K0.as_bytes()).expect("hash");
        sqlx::query(
            "INSERT INTO projects (id, account_id, status, backend_mode, api_key_hash_current, backend_pg_dsn_enc) \
             VALUES ($1, $2, $3, 'direct_pg', $4, $5)",
        )
        .bind(id)
        .bind(self.account)
        .bind(status)
        .bind(hash)
        .bind(keys().encrypt(&self.cust_url))
        .execute(&self.env.sys_pool)
        .await
        .expect("seed project");
        seed_greeting(&self.cust_pool, id, "hello").await;
    }

    async fn post_as(&self, cookie: Option<&str>, project: &str, verb: &str) -> Reply {
        post_to(&self.client, &self.server, cookie, project, verb).await
    }

    async fn get_project(&self, cookie: &str, project: &str) -> (u16, String) {
        let r = self
            .client
            .get(format!("http://{}/admin/v1/projects/{project}", self.server.admin_addr))
            .header("Cookie", cookie)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    /// `(current hash, previous hash, previous expiry, rotated_at)` straight from the row.
    async fn key_columns(&self, id: &str) -> (String, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
        sqlx::query_as(
            "SELECT api_key_hash_current, api_key_hash_previous, api_key_previous_expires_at, api_key_rotated_at \
             FROM projects WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.env.sys_pool)
        .await
        .unwrap()
    }
}

async fn post_to(
    client: &reqwest::Client,
    server: &TestServer,
    cookie: Option<&str>,
    project: &str,
    verb: &str,
) -> Reply {
    let mut req = client.post(format!("http://{}/admin/v1/projects/{project}/api_key/{verb}", server.admin_addr));
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let r = req.send().await.unwrap();
    let status = r.status().as_u16();
    let cache_control = r.headers().get("cache-control").map(|v| v.to_str().unwrap().to_string());
    let body = r.json().await.unwrap_or_default();
    Reply { status, cache_control, body }
}

fn new_key(reply: &Reply) -> String {
    reply.body["api_key"].as_str().expect("api_key in response").to_string()
}

/// Does `key` authenticate over gRPC on `server`?
async fn grpc_accepts(server: &TestServer, id: &str, key: &str) -> bool {
    match get_greeting(server, id, key).await {
        Ok(text) => {
            assert_eq!(text, "hello");
            true
        }
        Err(s) => {
            assert_eq!(s.code(), tonic::Code::Unauthenticated, "unexpected failure: {s:?}");
            false
        }
    }
}

/// Does `key` authenticate over REST? Accepted = past the key check (400, feature not enabled).
async fn rest_accepts(server: &TestServer, id: &str, key: &str) -> bool {
    let resp = reqwest::Client::new()
        .post(format!("http://{}/v1/projects/{id}/accounts:signInAnonymously?key={key}", server.rest_addr))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap();
    let accepted = body["reason"] == "ANONYMOUS_AUTH_NOT_ENABLED";
    assert!(accepted || (status == 401 && body["reason"] == "INVALID_API_KEY"), "unexpected {status} {body}");
    accepted
}

async fn accepted_everywhere(server: &TestServer, id: &str, key: &str) -> Option<bool> {
    let (grpc, rest) = (grpc_accepts(server, id, key).await, rest_accepts(server, id, key).await);
    (grpc == rest).then_some(grpc)
}

fn instant(v: &serde_json::Value) -> DateTime<Utc> {
    v.as_str().expect("timestamp string").parse().expect("RFC 3339")
}

/// Audit lines for one project.
fn audit_lines(project: &str) -> Vec<String> {
    let all = logs::text(&logs::init());
    all.lines()
        .filter(|l| l.contains("embyr::audit") && l.contains(&format!("project_id={project} ")))
        .map(str::to_string)
        .collect()
}

/// Journey: Fernbank rotates its key; both keys work during the window; the console shows the dates.
/// @AC-AKR-01 @AC-AKR-02 @AC-AKR-09 @AC-AKR-19
#[tokio::test]
async fn rotate_returns_the_new_key_once_and_both_keys_work_during_the_window() {
    let c = ctx().await;
    c.seed("fernbank", "active").await;
    assert!(grpc_accepts(&c.server, "fernbank", K0).await, "precondition: K0 works (and is cached)");

    let r = c.post_as(Some(&c.admin), "fernbank", "rotate").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.cache_control.as_deref(), Some("no-store"));
    let k1 = new_key(&r);
    assert_eq!(k1.len(), 43);
    assert_ne!(k1, K0);
    let expires = instant(&r.body["previous_key_expires_at"]);
    let rotated = instant(&r.body["rotated_at"]);
    assert_eq!((expires - rotated).num_seconds(), 86_400, "default 24 h window");
    assert!((Utc::now() - rotated).num_seconds().abs() < 120);

    assert_eq!(accepted_everywhere(&c.server, "fernbank", &k1).await, Some(true), "new key");
    assert_eq!(accepted_everywhere(&c.server, "fernbank", K0).await, Some(true), "old key, window open");

    let (status, body) = c.get_project(&c.viewer, "fernbank").await;
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(instant(&v["api_key_rotated_at"]), rotated);
    assert_eq!(instant(&v["api_key_previous_expires_at"]), expires);
    assert!(!body.contains("argon2") && !body.contains("hash"), "GET leaked key material: {body}");
}

/// Journey: a leaked key is revoked. The old key was cached on this replica and still dies at once.
/// @AC-AKR-05 @AC-AKR-10
#[tokio::test]
async fn revoke_kills_a_cached_old_key_at_once_and_clears_the_previous_columns() {
    let c = ctx().await;
    c.seed("leakcheck", "active").await;
    assert!(grpc_accepts(&c.server, "leakcheck", K0).await, "precondition: K0 cached");

    let r = c.post_as(Some(&c.owner), "leakcheck", "revoke").await;
    assert_eq!(r.status, 200);
    assert!(r.body["previous_key_expires_at"].is_null());
    let k1 = new_key(&r);

    assert_eq!(accepted_everywhere(&c.server, "leakcheck", K0).await, Some(false), "revoked key");
    assert_eq!(accepted_everywhere(&c.server, "leakcheck", &k1).await, Some(true), "replacement key");
    let (_, previous, expires, rotated) = c.key_columns("leakcheck").await;
    assert_eq!((previous, expires, rotated.is_some()), (None, None, true));

    let audit = audit_lines("leakcheck");
    assert_eq!(audit.len(), 1, "{audit:?}");
    for field in ["event=\"api_key_revoked\"", "account_id=", "session_id=", "window_secs=0", "previous_expires_at=None"] {
        assert!(audit[0].contains(field), "missing {field} in {}", audit[0]);
    }
}

/// Journey: an emergency second rotate while the first window is still open.
/// @AC-AKR-06
#[tokio::test]
async fn a_second_rotate_kills_the_first_key_and_demotes_the_second() {
    let c = ctx().await;
    c.seed("twice", "active").await;
    let k1 = new_key(&c.post_as(Some(&c.admin), "twice", "rotate").await);
    let second = c.post_as(Some(&c.admin), "twice", "rotate").await;
    assert_eq!(second.status, 200);
    let k2 = new_key(&second);

    assert_eq!(accepted_everywhere(&c.server, "twice", K0).await, Some(false), "two rotations ago");
    assert_eq!(accepted_everywhere(&c.server, "twice", &k1).await, Some(true), "previous, fresh window");
    assert_eq!(accepted_everywhere(&c.server, "twice", &k2).await, Some(true), "current");
}

/// Journey: two operators (or eight scripts) press Rotate at the same instant.
/// @AC-AKR-07
#[tokio::test]
async fn concurrent_rotates_have_exactly_one_winner() {
    let c = ctx().await;
    for n in [2usize, 8] {
        let id = format!("race-{n}");
        c.seed(&id, "active").await;
        let barrier = Arc::new(tokio::sync::Barrier::new(n));
        let mut tasks = vec![];
        for _ in 0..n {
            let (client, addr, cookie, id, barrier) =
                (c.client.clone(), c.server.admin_addr, c.admin.clone(), id.clone(), barrier.clone());
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                let r = client
                    .post(format!("http://{addr}/admin/v1/projects/{id}/api_key/rotate"))
                    .header("Cookie", cookie)
                    .send()
                    .await
                    .unwrap();
                (r.status().as_u16(), r.json::<serde_json::Value>().await.unwrap())
            }));
        }
        let mut winners = vec![];
        for t in tasks {
            let (status, body) = t.await.unwrap();
            match status {
                200 => winners.push(body["api_key"].as_str().unwrap().to_string()),
                409 => assert_eq!(body["error"], "rotation_conflict", "N={n}"),
                other => panic!("N={n}: unexpected status {other} {body}"),
            }
        }
        assert_eq!(winners.len(), 1, "N={n}: exactly one rotate wins");
        let (current, previous, _, _) = c.key_columns(&id).await;
        assert!(argon2::verify_api_key(winners[0].as_bytes(), &current).unwrap(), "N={n}: stored hash is the winner's");
        assert!(argon2::verify_api_key(K0.as_bytes(), &previous.expect("previous")).unwrap(), "N={n}");
    }
}

/// Journey: a project whose secrets are still encrypted to the old key must not be rotated.
/// @AC-AKR-08 @AC-AKR-09 (aws_secret half: the guard lets it through)
#[tokio::test]
async fn unconverted_connection_secrets_block_rotate_and_revoke_without_any_write() {
    let c = ctx().await;
    // Legacy direct_pg: DSN only under ECIES-to-K0.
    insert_owned_project(&c.env.sys_pool, c.account, "legacy-dsn", K0, "active").await;
    sqlx::query("UPDATE projects SET ecies_encrypted_dsn = $1 WHERE id = 'legacy-dsn'")
        .bind(ecies_of(K0, &c.cust_url))
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    seed_greeting(&c.cust_pool, "legacy-dsn", "hello").await;
    // Agent project without a server-held TLS bundle.
    insert_owned_project(&c.env.sys_pool, c.account, "legacy-agent", K0, "active").await;
    sqlx::query("UPDATE projects SET backend_mode = 'agent', backend_agent_endpoint = 'agent.example:443' WHERE id = 'legacy-agent'")
        .execute(&c.env.sys_pool)
        .await
        .unwrap();

    for id in ["legacy-dsn", "legacy-agent"] {
        let before = c.key_columns(id).await;
        for verb in ["rotate", "revoke"] {
            let r = c.post_as(Some(&c.owner), id, verb).await;
            assert_eq!((r.status, r.body["error"].as_str()), (409, Some("connection_secrets_unconverted")), "{id} {verb}");
            assert_eq!(c.key_columns(id).await, before, "{id} {verb}: no column may change");
        }
    }
    assert!(grpc_accepts(&c.server, "legacy-dsn", K0).await, "K0 still serves the legacy project");

    // aws_secret rows hold no ECIES secrets: the guard passes.
    insert_owned_project(&c.env.sys_pool, c.account, "aws-proj", K0, "active").await;
    sqlx::query("UPDATE projects SET backend_mode = 'aws_secret', backend_secret_arn = 'arn:aws:secretsmanager:eu-west-1:1:secret:x' WHERE id = 'aws-proj'")
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    assert_eq!(c.post_as(Some(&c.owner), "aws-proj", "rotate").await.status, 200);
}

/// Journey: only an Owner/Admin of the owning account may rotate; successes are audited, never logged with key material.
/// @AC-AKR-12
#[tokio::test]
async fn only_owner_or_admin_of_the_owning_account_rotates_and_every_success_is_audited() {
    let c = ctx().await;
    c.seed("guarded", "active").await;
    c.seed("sleepy", "suspended").await;
    c.seed("gone", "deleted").await;
    let other_account = new_account(&c.env.sys_pool).await;
    let foreign = seed_session(&c.env.sys_pool, other_account, "Owner").await;
    let before = c.key_columns("guarded").await;

    for (cookie, project, want) in [
        (Some(c.viewer.as_str()), "guarded", 403),
        (Some(foreign.as_str()), "guarded", 404),
        (Some(c.owner.as_str()), "no-such-project", 404),
        (Some(c.owner.as_str()), "gone", 404),
        (None, "guarded", 401),
    ] {
        for verb in ["rotate", "revoke"] {
            let r = c.post_as(cookie, project, verb).await;
            assert_eq!(r.status, want, "{project} {verb}");
        }
    }
    assert_eq!(c.key_columns("guarded").await, before, "denied requests write nothing");
    assert!(audit_lines("guarded").is_empty());
    assert_eq!(c.key_columns("gone").await.3, None, "deleted project untouched");

    let suspended = c.post_as(Some(&c.admin), "sleepy", "rotate").await;
    assert_eq!(suspended.status, 200, "a suspended project is rotatable");
    let ok = c.post_as(Some(&c.owner), "guarded", "rotate").await;
    assert_eq!((ok.status, ok.cache_control.as_deref()), (200, Some("no-store")));

    let audit = audit_lines("guarded");
    assert_eq!(audit.len(), 1, "{audit:?}");
    for field in ["event=\"api_key_rotated\"", "account_id=", "session_id=", "window_secs=86400", "previous_expires_at=Some("] {
        assert!(audit[0].contains(field), "missing {field} in {}", audit[0]);
    }
    assert_eq!(audit_lines("sleepy").len(), 1);

    // The log buffer is shared by every test in this binary, so only this test's own random new
    // keys are searched globally; the audit line is also checked for the old key and any hash.
    let log = logs::text(&logs::init());
    for secret in [new_key(&ok), new_key(&suspended)] {
        assert!(!log.contains(&secret), "logs leaked a key");
    }
    assert!(!audit[0].contains(K0) && !audit[0].contains("argon2"), "audit event leaked key material");
}

/// Journey: a second replica still holds the old key in its cache when it is revoked elsewhere.
/// @AC-AKR-11
#[tokio::test]
async fn a_replica_with_the_old_key_cached_keeps_serving_it_until_its_ttl() {
    let c = ctx().await;
    c.seed("replicas", "active").await;
    let replica_b = server_for(&c.env).await; // own CredentialCache over the same system DB
    assert!(grpc_accepts(&replica_b, "replicas", K0).await, "precondition: replica B cached K0");

    assert_eq!(c.post_as(Some(&c.owner), "replicas", "revoke").await.status, 200);

    assert!(!grpc_accepts(&c.server, "replicas", K0).await, "serving replica: dead at once");
    assert!(
        grpc_accepts(&replica_b, "replicas", K0).await,
        "other replica: served from its cache, bounded by CREDENTIAL_TTL (60 s)"
    );
}

fn listen_request(project: &str, collection: &str) -> ListenRequest {
    ListenRequest {
        database: format!("projects/{project}/databases/(default)"),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: format!("projects/{project}/databases/(default)/documents"),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector { collection_id: collection.into(), all_descendants: false }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn open_listen(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project: &str,
    key: &str,
) -> Result<tonic::Streaming<embyr_proto::firestore::ListenResponse>, tonic::Status> {
    let mut req = tonic::Request::new(tokio_stream::once(listen_request(project, "counters")));
    req.metadata_mut().insert("authorization", format!("bearer {key}").parse().unwrap());
    client.listen(req).await.map(|r| r.into_inner())
}

/// Journey: a dashboard keeps its live Listen open across a revoke; a new Listen with the old key is refused.
/// @AC-AKR-15
#[tokio::test]
async fn an_open_listen_survives_revoke_but_a_new_listen_with_the_old_key_is_rejected() {
    let c = ctx().await;
    c.seed("watcher", "active").await;
    let channel =
        tonic::transport::Channel::from_shared(format!("http://{}", c.server.grpc_addr)).unwrap().connect_lazy();
    let mut listener = FirestoreClient::new(channel.clone());
    let mut stream = open_listen(&mut listener, "watcher", K0).await.expect("listen with K0");
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended")
            .expect("stream error");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            if TargetChangeType::try_from(tc.target_change_type) == Ok(TargetChangeType::Current) {
                break;
            }
        }
    }

    let k1 = new_key(&c.post_as(Some(&c.owner), "watcher", "revoke").await);

    let mut write = tonic::Request::new(CreateDocumentRequest {
        parent: "projects/watcher/databases/(default)/documents".into(),
        collection_id: "counters".into(),
        document_id: "c1".into(),
        document: Some(Document { name: String::new(), fields: HashMap::new(), ..Default::default() }),
        ..Default::default()
    });
    write.metadata_mut().insert("authorization", format!("bearer {k1}").parse().unwrap());
    FirestoreClient::new(channel).create_document(write).await.expect("write with the new key");

    let delivered = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(msg)) = stream.next().await {
            if let Some(listen_response::ResponseType::DocumentChange(_)) = msg.response_type {
                return true;
            }
        }
        false
    })
    .await;
    assert!(matches!(delivered, Ok(true)), "the open stream keeps delivering after the revoke");

    let rejected = match open_listen(&mut listener, "watcher", K0).await {
        Err(status) => status,
        Ok(mut s) => s.next().await.expect("a message").expect_err("old key must be rejected"),
    };
    assert_eq!(rejected.code(), tonic::Code::Unauthenticated);
}

/// Seed a legacy hosted-identity row: Ed25519 seed ECIES-encrypted to `K0`. Returns the public key.
async fn seed_legacy_signing_key(c: &Ctx, id: &str) -> Vec<u8> {
    let sk = ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng);
    let public = sk.verifying_key().to_bytes().to_vec();
    let enc = ecies::encrypt(&ecies::derive_public_key(K0.as_bytes()), &sk.to_bytes()).unwrap();
    sqlx::query("INSERT INTO hosted_identity_signing_keys (project_id, public_key, private_key_enc) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&public)
        .bind(enc)
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    public
}

async fn hosted(server: &TestServer, id: &str, key: &str, action: &str) -> (u16, serde_json::Value) {
    let r = reqwest::Client::new()
        .post(format!("http://{}/v1/projects/{id}/accounts:{action}?key={key}", server.rest_addr))
        .json(&serde_json::json!({"email": "maria@example.com", "password": "correct-horse-battery"}))
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.json().await.unwrap())
}

fn verifies(token: &str, id: &str, public_key: &[u8]) -> bool {
    let credential = ClientIdentityCredential {
        public_key_current: public_key.try_into().expect("32-byte key"),
        public_key_previous: None,
    };
    verify_client_identity_token(Some(token), id, &credential).is_ok()
}

async fn stored_signing_row(c: &Ctx, id: &str) -> (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>) {
    sqlx::query_as("SELECT public_key, private_key_enc, private_key_srv_enc FROM hosted_identity_signing_keys WHERE project_id = $1")
        .bind(id)
        .fetch_one(&c.env.sys_pool)
        .await
        .unwrap()
}

/// Journey: first rotate of a legacy hosted-identity project regenerates the signing key under the server key.
/// @AC-AKR-14
#[tokio::test]
async fn first_rotate_regenerates_a_legacy_hosted_signing_key_and_old_tokens_stop_verifying() {
    let c = ctx().await;
    c.seed("hosted", "active").await;
    let p0 = seed_legacy_signing_key(&c, "hosted").await;
    assert_eq!(hosted(&c.server, "hosted", K0, "signUp").await.0, 201);
    let (status, body) = hosted(&c.server, "hosted", K0, "signInWithPassword").await;
    assert_eq!(status, 200, "sign-in with K0 before the rotation");
    let t0 = body["idToken"].as_str().unwrap().to_string();
    assert!(verifies(&t0, "hosted", &p0));

    let k1 = new_key(&c.post_as(Some(&c.admin), "hosted", "rotate").await);

    let (p1, legacy, srv) = stored_signing_row(&c, "hosted").await;
    assert_ne!(p1, p0, "new public key");
    assert!(legacy.is_none() && srv.is_some(), "seed now only under the server key");
    let (status, body) = hosted(&c.server, "hosted", &k1, "signInWithPassword").await;
    assert_eq!(status, 200, "sign-in with K1 after the rotation");
    let t1 = body["idToken"].as_str().unwrap();
    assert!(verifies(t1, "hosted", &p1));
    assert!(!verifies(&t0, "hosted", &p1), "a token minted before the rotation no longer verifies (1 h, documented)");
}

/// Journey: the signing-key step fails after the key update; nothing may change.
/// @AC-AKR-14
#[tokio::test]
async fn a_failure_after_the_key_update_rolls_everything_back() {
    let c = ctx().await;
    c.seed("rollback", "active").await;
    let p0 = seed_legacy_signing_key(&c, "rollback").await;
    let row_before = stored_signing_row(&c, "rollback").await;
    let columns_before = c.key_columns("rollback").await;
    sqlx::query("CREATE FUNCTION akr_inject_failure() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'injected'; END $$ LANGUAGE plpgsql")
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER akr_inject_failure BEFORE UPDATE ON hosted_identity_signing_keys FOR EACH ROW EXECUTE FUNCTION akr_inject_failure()")
        .execute(&c.env.sys_pool)
        .await
        .unwrap();

    let r = c.post_as(Some(&c.admin), "rollback", "rotate").await;
    assert_eq!((r.status, r.body["error"].as_str()), (500, Some("internal_error")));
    assert!(r.body.get("api_key").is_none());

    assert_eq!(c.key_columns("rollback").await, columns_before, "projects row unchanged");
    assert_eq!(stored_signing_row(&c, "rollback").await, row_before, "signing row unchanged");
    assert_eq!(hosted(&c.server, "rollback", K0, "signUp").await.0, 201, "K0 still works with the old seed");
    let (_, body) = hosted(&c.server, "rollback", K0, "signInWithPassword").await;
    assert!(verifies(body["idToken"].as_str().unwrap(), "rollback", &p0));
}
