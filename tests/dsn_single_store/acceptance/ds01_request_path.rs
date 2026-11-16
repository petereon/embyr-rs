//! Slice 1 (ADR-083): server-key read path + lazy convert-on-next-authenticated-request.
//! AC-DSS-03, AC-DSS-05, AC-DSS-06. Driving port: real gRPC GetDocument against the in-process server.
//! AC-DSS-04 (legacy agent bundle) lives in tests/acceptance/us_12_agent_backend.rs, which owns the
//! mTLS mock-agent fixtures.

#[path = "../common/mod.rs"]
mod common;
use common::*;

const KEY: &str = "ds01-api-key-0001";

/// AC-DSS-03: legacy-only row → first request succeeds, server store gets the same DSN, and a
/// fresh server (cold cache, legacy column removed) serves purely from the server store.
#[tokio::test]
async fn legacy_row_converts_on_first_request_then_serves_from_server_store() {
    let env = start_system_db().await;
    let (_c, cust_url, cust_pool) = start_ready_customer_db().await;
    insert_legacy_direct(&env.sys_pool, "ds01-a", KEY, &cust_url).await;
    seed_greeting(&cust_pool, "ds01-a", "hello").await;

    let server = server_for(&env).await;
    assert_eq!(get_greeting(&server, "ds01-a", KEY).await.unwrap(), "hello");
    assert!(
        wait_for(|| async { server_store_dsn(&env.sys_pool, "ds01-a").await.as_deref() == Some(cust_url.as_str()) })
            .await,
        "server store must hold the same DSN after the first authenticated request"
    );

    sqlx::query("UPDATE projects SET ecies_encrypted_dsn = NULL WHERE id = 'ds01-a'")
        .execute(&env.sys_pool)
        .await
        .unwrap();
    let cold = server_for(&env).await;
    assert_eq!(get_greeting(&cold, "ds01-a", KEY).await.unwrap(), "hello");
}

/// AC-DSS-05: both columns, DSNs differ → the legacy (currently served) DSN wins, is written over
/// the server copy, and the divergence counter records it. Traffic behaviour does not change.
#[tokio::test]
async fn divergent_pair_keeps_serving_legacy_dsn_and_records_divergence() {
    let env = start_system_db().await;
    let (_c, cust_url, cust_pool) = start_ready_customer_db().await;
    insert_legacy_direct(&env.sys_pool, "ds01-b", KEY, &cust_url).await;
    seed_greeting(&cust_pool, "ds01-b", "from-legacy").await;
    // An earlier PATCH-era server copy pointing nowhere.
    sqlx::query("UPDATE projects SET backend_pg_dsn_enc = $1 WHERE id = 'ds01-b'")
        .bind(keys().encrypt("postgres://nobody:x@127.0.0.1:1/none"))
        .execute(&env.sys_pool)
        .await
        .unwrap();

    let server = server_for(&env).await;
    assert_eq!(get_greeting(&server, "ds01-b", KEY).await.unwrap(), "from-legacy");
    assert!(
        wait_for(|| async { server_store_dsn(&env.sys_pool, "ds01-b").await.as_deref() == Some(cust_url.as_str()) })
            .await,
        "legacy DSN must overwrite the divergent server copy"
    );
    assert!(scrape_metrics(&server).await.contains("embyr_dsn_convert_divergent_total"));
}

/// AC-DSS-06 (failure): a conversion write that errors never fails the request; a later request
/// retries and succeeds.
#[tokio::test]
async fn failed_conversion_does_not_fail_request_and_is_retried_later() {
    let env = start_system_db().await;
    let (_c, cust_url, cust_pool) = start_ready_customer_db().await;
    insert_legacy_direct(&env.sys_pool, "ds01-c", KEY, &cust_url).await;
    seed_greeting(&cust_pool, "ds01-c", "hello").await;
    sqlx::query("ALTER TABLE projects ADD CONSTRAINT block_dsn CHECK (backend_pg_dsn_enc IS NULL)")
        .execute(&env.sys_pool)
        .await
        .unwrap();

    let server = server_for(&env).await;
    assert_eq!(get_greeting(&server, "ds01-c", KEY).await.unwrap(), "hello");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(column_bytes(&env.sys_pool, "ds01-c", "backend_pg_dsn_enc").await, None);

    sqlx::query("ALTER TABLE projects DROP CONSTRAINT block_dsn").execute(&env.sys_pool).await.unwrap();
    let retry = server_for(&env).await;
    assert_eq!(get_greeting(&retry, "ds01-c", KEY).await.unwrap(), "hello");
    assert!(
        wait_for(|| async { server_store_dsn(&env.sys_pool, "ds01-c").await.as_deref() == Some(cust_url.as_str()) })
            .await
    );
}

/// AC-DSS-06 (race): concurrent first requests all succeed and the row converges to the DSN.
#[tokio::test]
async fn concurrent_first_requests_all_succeed_and_converge() {
    let env = start_system_db().await;
    let (_c, cust_url, cust_pool) = start_ready_customer_db().await;
    insert_legacy_direct(&env.sys_pool, "ds01-d", KEY, &cust_url).await;
    seed_greeting(&cust_pool, "ds01-d", "hello").await;

    let server = server_for(&env).await;
    let (a, b, c, d) = tokio::join!(
        get_greeting(&server, "ds01-d", KEY),
        get_greeting(&server, "ds01-d", KEY),
        get_greeting(&server, "ds01-d", KEY),
        get_greeting(&server, "ds01-d", KEY),
    );
    for result in [a, b, c, d] {
        assert_eq!(result.unwrap(), "hello");
    }
    assert!(
        wait_for(|| async { server_store_dsn(&env.sys_pool, "ds01-d").await.as_deref() == Some(cust_url.as_str()) })
            .await
    );
}

/// Fault injection: a server-store value that cannot be decrypted (wrong key) and no legacy copy
/// is a clean Internal error, never a panic or a connection to a garbage DSN.
#[tokio::test]
async fn undecryptable_server_store_without_legacy_is_an_internal_error() {
    let env = start_system_db().await;
    let hash = embyr_core::auth::argon2::hash_api_key(KEY.as_bytes()).unwrap();
    let foreign = embyr_server::adapters::connection_secrets::ServerKeys { current: [9u8; 32], previous: None };
    sqlx::query(
        "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, backend_pg_dsn_enc) \
         VALUES ('ds01-e', 'active', 'direct_pg', $1, $2)",
    )
    .bind(&hash)
    .bind(foreign.encrypt("postgres://x@127.0.0.1:1/x"))
    .execute(&env.sys_pool)
    .await
    .unwrap();

    let server = server_for(&env).await;
    let status = get_greeting(&server, "ds01-e", KEY).await.unwrap_err();
    assert_eq!(status.code(), tonic::Code::Internal);
}
