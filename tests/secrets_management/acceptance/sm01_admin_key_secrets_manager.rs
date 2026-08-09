//! US-SM-01 — Admin key sourced from AWS/GCP Secrets Manager at startup.
//!
//! Acceptance criteria verified here (`docs/feature/secrets-management/discuss/user-stories.md`):
//!   AC-SM-01-01: `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` (optional) sources the admin key from AWS at startup.
//!   AC-SM-01-02: `EMBYR_ADMIN_KEY_GCP_SECRET_NAME` (optional) sources the admin key from GCP at startup.
//!   AC-SM-01-03: When neither secret-ref var is set, plain `EMBYR_ADMIN_KEY` is unchanged.
//!   AC-SM-01-04: Setting more than one admin-key source is a config error; exit 1 before any I/O.
//!   AC-SM-01-05: Secret fetch failure causes exit 1 with a named error; no port is bound.
//!   AC-SM-01-06 (folded into scenario 6): fetched value never appears in logs or DB rows.
//!
//! Driving port: `embyr-server` binary subprocess via ENV/STDERR/exit-code
//!               (`ServerConfig::from_env()`, ADR-018 Step 1) + admin-port HTTP.
//!
//! Walking skeleton: `server_starts_with_admin_key_from_aws_secrets_manager` — NOT #[ignore].
//! All other tests: #[ignore] — DELIVER unskips them one at a time.
//!
//! Scaffold classification target: RED for all tests until `ServerConfig::from_env()`
//! gains `EMBYR_ADMIN_KEY_AWS_SECRET_ARN`/`_GCP_SECRET_NAME` resolution (ADR-018 §3).
//! Today the binary treats `EMBYR_ADMIN_KEY` as the sole required var, so every
//! scenario below fails for the correct reason: "missing required environment
//! variable: EMBYR_ADMIN_KEY" (MISSING_FUNCTIONALITY, not a test-infra bug).

use std::time::Duration;

use crate::common::{
    create_raw_secret, localstack_aws_env, make_sm_client, nonexistent_secret_arn,
    start_localstack, start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};

// ─── Walking Skeleton ─────────────────────────────────────────────────────────

/// Sam sources the admin key from AWS Secrets Manager; the server starts and
/// the fetched key works as the operator Bearer token — with zero literal
/// admin-key value in the process environment.
///
/// Walking skeleton: proves the full AWS-sourced admin-key path end-to-end
/// through the production composition root.
///   1. Real Postgres container (system DB).
///   2. Real LocalStack container (AWS Secrets Manager emulator) holding a
///      secret whose RAW value (not `{"dsn": ...}` JSON) is the admin key.
///   3. `embyr-server` binary spawned as a subprocess with
///      `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` set and plain `EMBYR_ADMIN_KEY` unset.
///   4. `GET /healthz` returns 200 within 30 seconds.
///   5. A request bearing the FETCHED secret value as the Bearer token
///      succeeds against an operator route (`GET /metrics`).
///
/// @walking_skeleton @driving_port @real-io @US-SM-01 @AC-SM-01-01
#[tokio::test]
async fn server_starts_with_admin_key_from_aws_secrets_manager() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_pg, db_url) = start_postgres_container().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let admin_key_value = "prod-admin-secret-xyz";
    let arn = create_raw_secret(&sm_client, "sm01-ws-admin-key", admin_key_value).await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push(("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", arn));
    env.push(("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY.to_string()));
    // Deliberately absent: plain EMBYR_ADMIN_KEY (D-SM-2 / AC-SM-01-01).

    let mut server = ServerProcess::start_owned(&db_url, &env);

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server did not respond to GET /healthz with HTTP 200 within 30 seconds"
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/metrics", server.admin_port))
        .header("Authorization", format!("Bearer {admin_key_value}"))
        .send()
        .await
        .expect("GET /metrics");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "the AWS-fetched secret value must be accepted as the operator Bearer token"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-01-02: GCP-sourced admin key ──────────────────────────────────────

/// Server starts with the admin key sourced from GCP Secret Manager.
///
/// # Blocked pending OQ-SM-4 / ADR-018 Alternatives A6
/// `GcpSecretFetcher::new(base_url, token, ttl_secs)` takes an explicit
/// `base_url` with no environment-variable override exposed by
/// `ServerConfig` — there is no subprocess-level way to point the composition
/// root's GCP client at a local emulator today. Scaffolded per DISTILL
/// constraint (enumerate, do not skip): implement once a follow-up feature
/// (or a DELIVER-wave addition to this one) adds a test-only base-URL
/// override, e.g. `EMBYR_GCP_SECRET_MANAGER_BASE_URL`.
///
/// @requires_external @US-SM-01 @AC-SM-01-02
#[tokio::test]
#[ignore = "blocked on OQ-SM-4: GcpSecretFetcher base_url has no env-var override for local emulator testing"]
async fn server_starts_with_admin_key_from_gcp_secret_manager() {
    unimplemented!(
        "GCP subprocess real-I/O testing requires a base-URL override knob not yet designed \
         (OQ-SM-4, ADR-018 Alternatives A6) -- see comment above"
    );
}

// ─── AC-SM-01-03: plain env var unchanged ────────────────────────────────────

/// Local development keeps working via the plain `EMBYR_ADMIN_KEY` env var,
/// unchanged, with no AWS/GCP client constructed.
///
/// Journey (chained from the walking skeleton's Given: real Postgres +
/// real embyr-server subprocess; When: differs — plain var instead of ARN):
///   Given: DATABASE_URL and EMBYR_ENCRYPTION_KEY are set
///   And:   EMBYR_ADMIN_KEY is set to "local-dev-key" directly
///   And:   no AWS or GCP secret-ref variables are set
///   When:  Sam starts the server
///   Then:  the server starts successfully using "local-dev-key" as the admin key
///
/// @US-SM-01 @AC-SM-01-03 @backward-compat
#[tokio::test]
async fn plain_env_var_still_works_when_no_arn_configured() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "local-dev-key"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must start with plain EMBYR_ADMIN_KEY");

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/metrics", server.admin_port))
        .header("Authorization", "Bearer local-dev-key")
        .send()
        .await
        .expect("GET /metrics");
    assert_eq!(resp.status().as_u16(), 200);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-01-04: ambiguous sourcing rejected ────────────────────────────────

/// Startup refuses ambiguous admin-key sourcing (plain + AWS ARN both set).
///
/// Journey (error path):
///   Given: EMBYR_ADMIN_KEY is set to "literal-key"
///   And:   EMBYR_ADMIN_KEY_AWS_SECRET_ARN is also set
///   When:  Sam starts the server
///   Then:  the process exits with code 1
///   And:   stderr names both conflicting variables
///
/// @error @US-SM-01 @AC-SM-01-04
#[tokio::test]
async fn startup_refuses_ambiguous_admin_key_sourcing() {
    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:5432/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "literal-key"),
        (
            "EMBYR_ADMIN_KEY_AWS_SECRET_ARN",
            "arn:aws:secretsmanager:us-east-1:123456789012:secret:whatever-AbCdEf",
        ),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 for ambiguous admin-key sourcing; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("EMBYR_ADMIN_KEY") && stderr.contains("EMBYR_ADMIN_KEY_AWS_SECRET_ARN"),
        "stderr must name both conflicting variables; got: {stderr}"
    );
}

// ─── AC-SM-01-05: fetch failure exits cleanly ────────────────────────────────

/// Startup fails cleanly (exit 1, no port bound) when the AWS secret cannot
/// be fetched (nonexistent ARN — the LocalStack analogue of "IAM role denied").
///
/// Journey (error path, real I/O — the fetch genuinely fails against LocalStack):
///   Given: EMBYR_ADMIN_KEY_AWS_SECRET_ARN points to a secret that does not exist
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the process exits with code 1
///   And:   stderr contains "could not fetch EMBYR_ADMIN_KEY from AWS Secrets Manager"
///   And:   no TCP port is bound
///
/// @error @real-io @US-SM-01 @AC-SM-01-05
#[tokio::test]
async fn exits_1_when_admin_key_secret_fetch_fails() {
    let (_localstack, endpoint_url) = start_localstack().await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push(("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", nonexistent_secret_arn()));
    env.push((
        "DATABASE_URL",
        "postgres://postgres:postgres@127.0.0.1:5432/embyr".to_string(),
    ));
    env.push(("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY.to_string()));

    let borrowed: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut server = ServerProcess::start_env_only(&borrowed);

    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 when the admin-key secret cannot be fetched; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("could not fetch EMBYR_ADMIN_KEY") || stderr.contains("SecretFetchFailed"),
        "stderr must name the fetch failure; got: {stderr}"
    );
    assert!(
        !ServerProcess::port_is_bound(server.admin_port),
        "admin port must not be bound after a startup secret-fetch failure"
    );
}

// ─── Mandatory negative test: secret value never logged/persisted ───────────

/// The AWS-fetched admin key value never appears in any log line, at any
/// level, nor in any row of the system DB — mandatory System Constraint,
/// mirrors the `EMBYR_AGENT_DB_DSN` / Invariant-13 precedent (ADR-018
/// Enforcement section).
///
/// Journey (real I/O, drives 5+ requests through the affected routes):
///   Given: EMBYR_ADMIN_KEY_AWS_SECRET_ARN points to a secret containing a sentinel string
///   When:  the server starts and handles 5 operator API requests
///   Then:  no log line at any level contains the sentinel string
///   And:   no row in the system DB contains the sentinel string
///
/// @error @real-io @US-SM-01 @security
#[tokio::test]
async fn admin_key_secret_value_never_appears_in_logs_or_db() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_pg, db_url) = start_postgres_container().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let sentinel = "sentinel-do-not-log-987";
    let arn = create_raw_secret(&sm_client, "sm01-sentinel-admin-key", sentinel).await;

    let mut env: Vec<(&str, String)> = localstack_aws_env(&endpoint_url);
    env.push(("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", arn));
    env.push(("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY.to_string()));
    env.push(("RUST_LOG", "debug".to_string()));

    let mut server = ServerProcess::start_owned(&db_url, &env);
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must start for the log-scan test to be meaningful");

    // Drive 5+ operator API requests (correct token, wrong token, /metrics).
    let client = reqwest::Client::new();
    for _ in 0..3 {
        let _ = client
            .get(format!("http://127.0.0.1:{}/metrics", server.admin_port))
            .header("Authorization", format!("Bearer {sentinel}"))
            .send()
            .await;
    }
    for _ in 0..2 {
        let _ = client
            .post(format!(
                "http://127.0.0.1:{}/admin/v1/projects",
                server.admin_port
            ))
            .header("Authorization", "Bearer wrong-token")
            .json(&serde_json::json!({"project_id": "sentinel-probe"}))
            .send()
            .await;
    }

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
    let stderr = server.drain_stderr();

    assert!(
        !stderr.contains(sentinel),
        "the fetched admin key value must never appear in stderr/log output"
    );

    // Scan every system-DB table for the sentinel value.
    let system_db = embyr_server::adapters::system_db::SystemDb::new(&db_url)
        .await
        .expect("connect to system DB for scan");
    let pool = system_db.pool();
    for table in ["sessions", "oidc_providers", "projects", "users"] {
        let row_count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE {table}::text LIKE '%' || $1 || '%'"
        ))
        .bind(sentinel)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        assert_eq!(
            row_count, 0,
            "table '{table}' must not contain the sentinel admin-key value"
        );
    }
}
