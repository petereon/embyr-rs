//! US-SM-02 — Encryption key sourced from AWS/GCP Secrets Manager at startup.
//!
//! Acceptance criteria verified here:
//!   AC-SM-02-01: `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` sources the encryption key from AWS.
//!   AC-SM-02-02: fetched value is hex-decoded/validated via the identical
//!                `ConfigError::InvalidEncryptionKey` path as the plain-env-var case.
//!   AC-SM-02-03: when neither secret-ref var is set, plain `EMBYR_ENCRYPTION_KEY` is unchanged.
//!   AC-SM-02-04: more than one encryption-key source is a config error; exit 1 before any I/O.
//!   AC-SM-02-05: secret fetch failure causes exit 1 with a named error; no port bound.
//!   AC-SM-02-06: `oidc_providers.rs`/`auth.rs`/`projects.rs` require zero code changes —
//!                proven indirectly by scenario 2 (OIDC creation succeeds unmodified).
//!
//! Driving port: `embyr-server` binary subprocess (config resolution) + admin-port HTTP
//!               (`POST /admin/v1/oidc_providers`, session-auth-guarded route, proven via
//!               the operator/dual-auth bearer as a stand-in until session auth is in scope
//!               of a different feature — see scenario 2 note).
//!
//! Builds directly on US-SM-01's raw-string fetch method (D-SM-3) — no new fetcher
//! capability required; only new `ServerConfig` fields/vars (ADR-018 §3).
//!
//! Scaffold classification target: RED — today `EMBYR_ENCRYPTION_KEY` is the sole
//! resolution path; scenarios fail for "missing/invalid EMBYR_ENCRYPTION_KEY" reasons.

use std::time::Duration;

use crate::common::{
    create_raw_secret, localstack_aws_env, make_sm_client, nonexistent_secret_arn,
    seed_signed_in_session, start_localstack, start_postgres_container, ServerProcess,
    TEST_ENCRYPTION_KEY,
};

// ─── AC-SM-02-01: AWS-sourced encryption key ─────────────────────────────────

/// Server starts with the encryption key sourced from AWS Secrets Manager.
///
/// Journey (chained from US-SM-01's walking skeleton shape: real Postgres +
/// real LocalStack + real subprocess; here the ARN targets the ENCRYPTION key
/// instead of the admin key):
///   Given: DATABASE_URL and EMBYR_ADMIN_KEY are set to valid values
///   And:   EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN points to a secret containing a valid 64-hex-char key
///   And:   plain EMBYR_ENCRYPTION_KEY is not set
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the server starts successfully
///
/// @real-io @US-SM-02 @AC-SM-02-01
#[tokio::test]
async fn server_starts_with_encryption_key_from_aws_secrets_manager() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_pg, db_url) = start_postgres_container().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let arn = create_raw_secret(&sm_client, "sm02-encryption-key", TEST_ENCRYPTION_KEY).await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push(("EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN", arn));
    env.push(("EMBYR_ADMIN_KEY", "testkey".to_string()));

    let mut server = ServerProcess::start_owned(&db_url, &env);

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must start with an AWS-sourced EMBYR_ENCRYPTION_KEY"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-02-02 / AC-SM-02-06: OIDC encryption unmodified by this feature ───

/// OIDC client-secret encryption succeeds with a secrets-manager-sourced key
/// — proves `oidc_providers.rs:130` requires zero code changes (D-SM's ACL
/// boundary: only `ServerConfig::from_env()`/`main.rs` know about
/// secrets-manager sourcing).
///
/// Journey (chained from scenario 1's Given + When — server already running
/// with the AWS-sourced encryption key):
///   Given: the server started with EMBYR_ENCRYPTION_KEY sourced from AWS Secrets Manager
///   When:  an Owner creates an OIDC provider with client_secret "okta-client-secret-9f2c"
///   Then:  the response is HTTP 201
///   And:   the stored client_secret_enc value is not equal to the plaintext
///
/// Implementation note for DELIVER: this scenario needs a signed-in session
/// (session-auth-guarded route) — seed an account/user/session row directly
/// against the system DB, mirroring `tests/admin_api_v2/common/mod.rs`'s
/// `AdminTestContext` seeding, then attach the session cookie to the request.
///
/// @real-io @US-SM-02 @AC-SM-02-02 @AC-SM-02-06
#[tokio::test]
async fn oidc_client_secret_encryption_succeeds_with_secrets_manager_sourced_key() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_pg, db_url) = start_postgres_container().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let arn = create_raw_secret(&sm_client, "sm02-oidc-encryption-key", TEST_ENCRYPTION_KEY).await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push(("EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN", arn));
    env.push(("EMBYR_ADMIN_KEY", "testkey".to_string()));

    let mut server = ServerProcess::start_owned(&db_url, &env);

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must start with an AWS-sourced EMBYR_ENCRYPTION_KEY before the OIDC write path can be exercised"
    );

    // Server subprocess already ran migrations at startup; connect our own
    // pool against the same DB to seed a signed-in session and query results.
    let system_db = embyr_server::adapters::system_db::SystemDb::new(&db_url)
        .await
        .expect("connect to system DB for session seeding");
    let pool = system_db.pool();

    let (_account_id, _user_id, cookie_value) =
        seed_signed_in_session(pool, "owner@sm02-oidc-test.example").await;

    let client_secret_plaintext = "okta-client-secret-9f2c";
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/oidc_providers",
            server.admin_port
        ))
        .header("Cookie", format!("embyr_session={cookie_value}"))
        .json(&serde_json::json!({
            "issuer": "https://sm02-oidc-test.okta.com",
            "client_id": "sm02-oidc-client-id",
            "client_secret": client_secret_plaintext,
        }))
        .send()
        .await
        .expect("POST /admin/v1/oidc_providers");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "OIDC provider creation must succeed unmodified with a secrets-manager-sourced encryption key"
    );

    let body: serde_json::Value = resp.json().await.expect("response body JSON");
    let provider_id = body["id"].as_str().expect("id field present").to_string();
    let provider_uuid = uuid::Uuid::parse_str(&provider_id).expect("valid provider id UUID");

    let client_secret_enc: Vec<u8> =
        sqlx::query_scalar("SELECT client_secret_enc FROM oidc_providers WHERE id = $1")
            .bind(provider_uuid)
            .fetch_one(pool)
            .await
            .expect("fetch stored client_secret_enc");

    assert_ne!(
        client_secret_enc,
        client_secret_plaintext.as_bytes(),
        "stored client_secret_enc must differ from the submitted plaintext"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-02-03: plain env var unchanged ────────────────────────────────────

/// Local development keeps working via the plain `EMBYR_ENCRYPTION_KEY` env
/// var, unchanged.
///
/// Journey:
///   Given: DATABASE_URL and EMBYR_ADMIN_KEY are set
///   And:   EMBYR_ENCRYPTION_KEY is set directly to a valid 64-hex-char value
///   And:   no AWS or GCP secret-ref variables are set
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the server starts successfully using the plain value as the encryption key
///
/// @US-SM-02 @AC-SM-02-03 @backward-compat
#[tokio::test]
async fn plain_env_var_still_works_for_encryption_key() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must start with plain EMBYR_ENCRYPTION_KEY");

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-02-04: invalid length rejected ────────────────────────────────────

/// Fetched encryption key of invalid length is rejected at startup, via the
/// identical `ConfigError::InvalidEncryptionKey` path as the plain-env-var case.
///
/// Journey (error path, real I/O — the fetched value is genuinely wrong-length):
///   Given: EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN points to a secret containing "tooshort"
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the process exits with code 1
///   And:   stderr contains "invalid EMBYR_ENCRYPTION_KEY"
///
/// @error @real-io @US-SM-02 @AC-SM-02-04
#[tokio::test]
async fn fetched_encryption_key_of_invalid_length_is_rejected_at_startup() {
    let (_localstack, endpoint_url) = start_localstack().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let arn = create_raw_secret(&sm_client, "sm02-bad-length-key", "tooshort").await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push(("EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN", arn));
    env.push(("EMBYR_ADMIN_KEY", "testkey".to_string()));
    env.push((
        "DATABASE_URL",
        "postgres://postgres:postgres@127.0.0.1:5432/embyr".to_string(),
    ));

    let borrowed: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut server = ServerProcess::start_env_only(&borrowed);

    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 for a wrong-length fetched encryption key; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("invalid EMBYR_ENCRYPTION_KEY"),
        "stderr must reuse the identical InvalidEncryptionKey message shape; got: {stderr}"
    );
}

// ─── AC-SM-02-05: ambiguous sourcing rejected ────────────────────────────────

/// Startup refuses ambiguous encryption-key sourcing (plain + GCP name both set).
///
/// Journey (error path):
///   Given: EMBYR_ENCRYPTION_KEY is set to a literal value
///   And:   EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME is also set
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the process exits with code 1
///   And:   stderr names both conflicting variables
///
/// @error @US-SM-02 @AC-SM-02-05
#[tokio::test]
async fn startup_refuses_ambiguous_encryption_key_sourcing() {
    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:5432/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        (
            "EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME",
            "projects/finops-prod/secrets/embyr-encryption-key",
        ),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 for ambiguous encryption-key sourcing; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("EMBYR_ENCRYPTION_KEY") && stderr.contains("EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME"),
        "stderr must name both conflicting variables; got: {stderr}"
    );
}

// ─── AC-SM-02-05 (fetch failure variant): secret cannot be fetched ───────────

/// Startup fails cleanly (exit 1, no port bound) when the encryption-key
/// secret cannot be fetched (nonexistent ARN).
///
/// Journey (error path, real I/O):
///   Given: EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN points to a secret that does not exist
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the process exits with code 1
///   And:   stderr contains "could not fetch EMBYR_ENCRYPTION_KEY from AWS Secrets Manager"
///   And:   no TCP port is bound
///
/// @error @real-io @US-SM-02 @AC-SM-02-05
#[tokio::test]
async fn exits_1_when_encryption_key_secret_fetch_fails() {
    let (_localstack, endpoint_url) = start_localstack().await;

    // Warm up LocalStack's lazily-initialized Secrets Manager backend before
    // the timed wait starts (see DESIGN § Root Cause Investigation, Finding 3:
    // the first-ever Secrets Manager call in a LocalStack container's lifetime
    // pays a one-time init cost that can exceed a 10s wait_for_exit bound).
    let sm_client = make_sm_client(&endpoint_url).await;
    let _ = create_raw_secret(&sm_client, "sm02-warmup-secret", "warmup-value").await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push((
        "EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN",
        nonexistent_secret_arn(),
    ));
    env.push(("EMBYR_ADMIN_KEY", "testkey".to_string()));
    env.push((
        "DATABASE_URL",
        "postgres://postgres:postgres@127.0.0.1:5432/embyr".to_string(),
    ));

    let borrowed: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut server = ServerProcess::start_env_only(&borrowed);

    let exit_code = server.wait_for_exit(Duration::from_secs(15)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 when the encryption-key secret cannot be fetched; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("could not fetch EMBYR_ENCRYPTION_KEY") || stderr.contains("SecretFetchFailed"),
        "stderr must name the fetch failure; got: {stderr}"
    );
    assert!(
        !ServerProcess::port_is_bound(server.admin_port),
        "admin port must not be bound after a startup secret-fetch failure"
    );
}
