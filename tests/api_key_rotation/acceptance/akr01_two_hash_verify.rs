// @driving_port @real-io @adapter-integration
//! api-key-rotation slice 1 (ADR-085): two-hash verify with SQL-evaluated expiry.
//!
//! Nothing writes `api_key_hash_previous` until slice 4, so the fixtures seed it directly.
//! Expiry is exercised by writing `api_key_previous_expires_at` into the past (never by
//! sleeping): the live predicate runs in Postgres `now()`.
//!
//!   AC-AKR-02: window open -> K0 and K1 both authenticate at every verify site.
//!   AC-AKR-03: window passed -> K0 rejected like a wrong key, K1 still works.
//!   AC-AKR-04: previous hash with NULL expiry -> ignored (fail closed).
//!   AC-AKR-20: never-rotated project (new columns NULL) -> behaves as before.
//!
//! Verify sites: gRPC :8080 (GetDocument), REST accounts:signInAnonymously (sign_in_anonymously.rs),
//! REST accounts:signUp (adapters/project_auth.rs). CI only: Docker.

#[path = "../../dsn_single_store/common/mod.rs"]
mod common;
use common::{get_greeting, keys, seed_greeting, server_for, start_ready_customer_db, start_system_db, Env};
use embyr_core::auth::argon2;
use embyr_server::TestServer;

const K0: &str = "akr-old-key";
const K1: &str = "akr-new-key";

/// Seed a direct_pg project (server-held DSN, so either key can open the backend).
/// `previous_expiry_secs`: `None` = NULL expiry; `Some(s)` = now() + s seconds.
async fn seed(env: &Env, cust_url: &str, id: &str, previous: bool, previous_expiry_secs: Option<f64>) {
    let hash = |k: &str| argon2::hash_api_key(k.as_bytes()).expect("hash");
    sqlx::query(
        "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, api_key_hash_previous, \
         api_key_previous_expires_at, backend_pg_dsn_enc) \
         VALUES ($1, 'active', 'direct_pg', $2, $3, now() + make_interval(secs => $4), $5)",
    )
    .bind(id)
    .bind(hash(K1))
    .bind(previous.then(|| hash(K0)))
    .bind(previous_expiry_secs)
    .bind(keys().encrypt(cust_url))
    .execute(&env.sys_pool)
    .await
    .expect("seed project");
}

/// Does `key` authenticate at the gRPC site?
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

/// Does `key` authenticate at the two REST sites? Both must agree.
/// Accepted = past the key check (400 for the not-enabled feature); rejected = 401 INVALID_API_KEY.
async fn rest_accepts(server: &TestServer, id: &str, key: &str) -> bool {
    let client = reqwest::Client::new();
    let mut verdicts = vec![];
    for (action, accepted_reason) in [
        ("signInAnonymously", "ANONYMOUS_AUTH_NOT_ENABLED"),
        ("signUp", "HOSTED_IDENTITY_NOT_ENABLED"),
    ] {
        let resp = client
            .post(format!("http://{}/v1/projects/{id}/accounts:{action}?key={key}", server.rest_addr))
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect("REST call");
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.expect("json body");
        let accepted = body["reason"] == accepted_reason;
        assert!(
            accepted || (status == 401 && body["reason"] == "INVALID_API_KEY"),
            "{action}: unexpected response {status} {body}"
        );
        verdicts.push(accepted);
    }
    assert_eq!(verdicts[0], verdicts[1], "REST verify sites disagree for key {key}");
    verdicts[0]
}

/// `Some(verdict)` when gRPC and both REST sites agree, `None` when they diverge.
async fn accepted_everywhere(server: &TestServer, id: &str, key: &str) -> Option<bool> {
    let (grpc, rest) = (grpc_accepts(server, id, key).await, rest_accepts(server, id, key).await);
    (grpc == rest).then_some(grpc)
}

/// Journey: Fernbank rotated its key; the window is still open.
/// @AC-AKR-02
#[tokio::test]
async fn both_keys_authenticate_while_the_window_is_open() {
    let env = start_system_db().await;
    let (_c, cust_url, cust) = start_ready_customer_db().await;
    seed(&env, &cust_url, "fernbank", true, Some(3600.0)).await;
    seed_greeting(&cust, "fernbank", "hello").await;
    let server = server_for(&env).await;

    assert_eq!(accepted_everywhere(&server, "fernbank", K1).await, Some(true), "current key");
    assert_eq!(accepted_everywhere(&server, "fernbank", K0).await, Some(true), "previous key, window open");
}

/// Journey: the window passed (fixture writes the expiry into the past); no job ran.
/// @AC-AKR-03
#[tokio::test]
async fn previous_key_is_rejected_after_the_window_and_current_key_still_works() {
    let env = start_system_db().await;
    let (_c, cust_url, cust) = start_ready_customer_db().await;
    seed(&env, &cust_url, "solstice", true, Some(-60.0)).await;
    seed_greeting(&cust, "solstice", "hello").await;
    let server = server_for(&env).await;

    assert_eq!(accepted_everywhere(&server, "solstice", K0).await, Some(false), "expired previous key");
    assert_eq!(accepted_everywhere(&server, "solstice", K1).await, Some(true), "current key");
}

/// Journey: a never-rotated project and a previous hash with no expiry behave as one key.
/// @AC-AKR-04 @AC-AKR-20
#[tokio::test]
async fn never_rotated_and_expiryless_previous_behave_as_single_key() {
    let env = start_system_db().await;
    let (_c, cust_url, cust) = start_ready_customer_db().await;
    seed(&env, &cust_url, "plain", false, None).await;
    seed(&env, &cust_url, "no-expiry", true, None).await;
    for id in ["plain", "no-expiry"] {
        seed_greeting(&cust, id, "hello").await;
    }
    let server = server_for(&env).await;

    for id in ["plain", "no-expiry"] {
        assert_eq!(accepted_everywhere(&server, id, K1).await, Some(true), "{id}: current key");
        assert_eq!(accepted_everywhere(&server, id, K0).await, Some(false), "{id}: other key");
    }
}
