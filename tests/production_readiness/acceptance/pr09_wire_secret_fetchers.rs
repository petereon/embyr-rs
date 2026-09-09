// @real-io @US-01
//! wire-secret-fetchers (US-01) — `main.rs` (the real production composition
//! root) never constructs real `AwsSecretFetcher`/`GcpSecretFetcher`
//! instances; all three call sites (`FirestoreService`, `build_admin_router`,
//! `transaction_sweeper::spawn`) hardcode `None, None`. Every `aws_secret`/
//! `gcp_secret`-mode project fails closed 100% of the time, in every
//! environment, regardless of whether that cloud's credentials are actually
//! available (production-readiness-audit-2026-09-08.md finding #7).
//!
//! Acceptance criteria verified here:
//!   AC-WSF-01: with AWS credentials genuinely available (via the standard
//!              AWS SDK credential chain), a real gRPC request against an
//!              `aws_secret`-mode project succeeds through the REAL
//!              `embyr-server` binary — not `Status::internal`. Walking
//!              skeleton: this is the single most important test in this
//!              file, proving `main.rs` itself was wired, not just that the
//!              fetcher mechanism works in isolation (already proven by
//!              `tests/acceptance/us_10_aws_secrets.rs` via a test-only
//!              composition path).
//!   AC-WSF-02: the GCP equivalent — see this test's own doc comment for a
//!              real testability gap this DISTILL pass found (no env-var
//!              override exists for `GCP_SECRET_MANAGER_BASE_URL`, unlike
//!              AWS's `AWS_ENDPOINT_URL`), which narrows what this test can
//!              prove to wiring, not a full round-trip.
//!   AC-WSF-03: admin provisioning of an `aws_secret` project succeeds —
//!              covered as a subset of the AC-WSF-01 walking skeleton
//!              (provisioning is the first half of that same journey); not
//!              duplicated as a separate LocalStack-spinning test.
//!   AC-WSF-04: a deployment with neither cloud configured starts up
//!              normally (no global fail-fast) and `direct_pg` is unaffected.
//!   AC-WSF-05: an already-provisioned `aws_secret` project still fails
//!              closed (`Status::internal`, never a panic, never a stale/
//!              wrong-DSN success) when this deployment genuinely lacks
//!              usable AWS credentials — named non-regression, not new
//!              behavior.
//!   AC-WSF-06: the SAME constructed fetcher instances reach the third
//!              composition-root call site too — `transaction_sweeper::spawn`
//!              actually reclaims an orphaned transaction for a real
//!              `aws_secret` project once AWS credentials are available,
//!              rather than silently skipping it (ADR-054 §D7). Side effect:
//!              this necessarily exercises `resolve_aws_secret_dsn`
//!              (`customer_db_connect.rs`), closing
//!              `composite-index-real-creation`'s follow-up-flagged
//!              zero-test-coverage gap for that function.
//!   (AC-WSF-07 is the full regression suite, run by the orchestrator once,
//!   not a scenario in this file — `us_10_aws_secrets.rs`/
//!   `us_11_gcp_secrets.rs` must stay green, unmodified.)
//!
//! Driving ports: the existing gRPC :8080 listener (any RPC dispatching on
//!   `backend_mode`) and the existing Admin :9090 `POST /admin/v1/projects`
//!   provisioning endpoint — both already exist; this feature changes only
//!   what fetcher instances the composition root constructs and threads into
//!   already-existing `Option<Arc<...>>` parameters.
//!
//! Layer: WS/`@wiring_e2e` (real subprocess, real I/O, several seconds each)
//!   — per `nw-test-design-mandates` Layered Test Discipline, this layer uses
//!   traditional assertions (not `assert_state_delta`), example-only (no
//!   PBT), matching pr01-pr08's own established style in this same file
//!   family.
//!
//! Scaffold state: NONE created by this DISTILL pass — no new production
//!   module is imported (`AwsSecretFetcher`, `GcpSecretFetcher`,
//!   `CLOUD_SECRET_FETCHER_TTL_SECS` per DESIGN's own Handoff Package are
//!   either pre-existing or a one-line `pub const` addition DELIVER makes).
//!   All tests are expected to FAIL against today's code for the right
//!   reason — `main.rs`'s own hardcoded `None, None` at all three call
//!   sites — not a test-setup bug. DELIVER implements DESIGN's fully
//!   specified Handoff Package (§ Construction Decisions D-WSF-1..4).
//!
//! Walking skeleton: `aws_secret_project_serves_real_grpc_request_when_aws_credentials_available`
//!   — NOT `#[ignore]`. All other tests: `#[ignore]` — DELIVER unskips them
//!   one at a time, mirroring pr01-pr08's own established convention.
//!
//! ─────────────────────────────────────────────────────────────────────────
//! DISTILL finding — GCP testability gap (see feature-delta.md
//! "Wave: DISTILL / [REF] Upstream Issue" for the full writeup):
//!
//! Unlike AWS (`aws_config::load_defaults` respects the standard
//! `AWS_ENDPOINT_URL`/`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` env vars —
//! the exact mechanism the AWS walking skeleton below uses to point the REAL
//! subprocess at LocalStack with zero production-code changes), `main.rs`'s
//! own locked construction of `GcpSecretFetcher` (DESIGN § D-WSF-1) uses the
//! hardcoded `pub const GCP_SECRET_MANAGER_BASE_URL =
//! "https://secretmanager.googleapis.com"` (`config.rs:54`) with NO env-var
//! override — confirmed by that constant's own pre-existing doc comment ("No
//! env-var override exists yet ... out of scope for this feature"), which
//! this feature's own DESIGN wave leaves unchanged (D-WSF-3 is a visibility
//! bump only, not an override mechanism). `us_11_gcp_secrets.rs`'s own
//! mock-GCP mechanism only works because it constructs `GcpSecretFetcher`
//! directly with a mock `base_url` and hands the already-built instance to
//! `start_test_server_with_gcp_fetcher` — a TEST-ONLY composition path. The
//! real subprocess this feature exists to prove wired has no such door.
//!
//! Consequence: `gcp_secret_project_wiring_reached_when_token_configured`
//! below cannot fabricate a real secret through the real subprocess the way
//! the AWS walking skeleton does — it can only reach the REAL
//! `secretmanager.googleapis.com` over the network with a token that isn't
//! real, so it proves WIRING (the fetcher is genuinely `Some`, not the old
//! `gcp_secret_fetcher_not_configured`), not a full round-trip against a
//! real document. Tagged `@requires_external`; not part of the deterministic
//! CI regression run. If a fully-deterministic AC-WSF-02 proof is required,
//! DESIGN needs to add a test-injectable base-URL override for
//! `GcpSecretFetcher`'s construction in `main.rs` (out of this DISTILL
//! pass's scope — DISTILL does not modify production code).
//! ─────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::time::Duration;

use embyr_core::auth::argon2;
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, value::ValueType, CreateDocumentRequest, Document,
    GetDocumentRequest, Value,
};
use testcontainers::{
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
};

use crate::common::{start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY};

// ─── Shared test helpers ─────────────────────────────────────────────────────

fn string_value(s: &str) -> Value {
    Value {
        value_type: Some(ValueType::StringValue(s.to_string())),
    }
}

/// Start a real LocalStack container for AWS Secrets Manager emulation.
/// Duplicated (not cross-imported) from `tests/acceptance/us_10_aws_secrets.rs`
/// — Rust integration test binaries don't share code across `[[test]]`
/// targets without a dedicated support crate; this project's own established
/// convention (see `tests/production_readiness/common/mod.rs::sign_stripe_payload`'s
/// own doc comment) is to duplicate small test helpers per file rather than
/// fight that restriction.
async fn start_localstack() -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("localstack/localstack", "3.5")
        .with_exposed_port(ContainerPort::Tcp(4566))
        .with_wait_for(WaitFor::message_on_stdout("Ready."))
        .start()
        .await
        .expect("start localstack");
    let port = container
        .get_host_port_ipv4(4566)
        .await
        .expect("get localstack port");
    let endpoint_url = format!("http://127.0.0.1:{port}");
    (container, endpoint_url)
}

/// Build an AWS Secrets Manager client pointed at LocalStack, for the TEST
/// side only (creating a secret to provision against) — separate from the
/// real subprocess's own credential resolution, which goes through
/// `AWS_ENDPOINT_URL`/`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` env vars.
async fn make_sm_client(endpoint_url: &str) -> aws_sdk_secretsmanager::Client {
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    aws_sdk_secretsmanager::Client::new(&aws_config)
}

// ─── AC-WSF-01 / AC-WSF-03: AWS walking skeleton ─────────────────────────────

/// Morgan provisions an `aws_secret`-mode project against the REAL
/// `embyr-server` binary, with AWS credentials genuinely available via the
/// standard AWS SDK credential chain (pointed at LocalStack), and Alex's SDK
/// call succeeds — proving `main.rs` itself constructs and threads a real
/// `aws_secret_fetcher`, not just that the mechanism works in isolation.
///
/// Journey (walking skeleton — covers AC-WSF-01 and, as a subset, AC-WSF-03):
///   Given: LocalStack AWS Secrets Manager has a secret containing a real
///          customer Postgres DSN
///   And:   the embyr-server deployment has AWS credentials available via
///          the standard AWS SDK credential chain (AWS_ENDPOINT_URL/
///          AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY/AWS_REGION, pointed at
///          LocalStack)
///   When:  Morgan provisions project "wsf-ws-aws-project" with
///          backend_mode=aws_secret and the secret's ARN
///   Then:  provisioning succeeds (201, not 500 aws_secret_fetcher_not_configured)
///   And:   Alex's SDK client creates a document and reads it back —
///          succeeds, not Status::internal("aws_secret_fetcher not configured")
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-WSF-01 @AC-WSF-03
#[tokio::test]
async fn aws_secret_project_serves_real_grpc_request_when_aws_credentials_available() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_cust, cust_url) = start_postgres_container().await;
    let (_sys, sys_url) = start_postgres_container().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let secret_string = format!(r#"{{"dsn": "{cust_url}"}}"#);
    let create_resp = sm_client
        .create_secret()
        .name("wsf-ws-dsn-secret")
        .secret_string(&secret_string)
        .send()
        .await
        .expect("create secret in LocalStack");
    let arn = create_resp.arn().expect("arn present").to_string();

    let mut server = ServerProcess::start(
        &sys_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("AWS_ENDPOINT_URL", &endpoint_url),
            ("AWS_ACCESS_KEY_ID", "test"),
            ("AWS_SECRET_ACCESS_KEY", "test"),
            ("AWS_REGION", "us-east-1"),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must start and become healthy with AWS credentials pointing at LocalStack \
         (AWS fetcher construction is unconditional and infallible per DESIGN D-WSF-1)"
    );

    let http = reqwest::Client::new();
    let provision_resp = http
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/projects",
            server.admin_port
        ))
        .header("Authorization", "Bearer testkey")
        .json(&serde_json::json!({
            "project_id": "wsf-ws-aws-project",
            "backend_mode": "aws_secret",
            "secret_arn": arn,
        }))
        .send()
        .await
        .expect("provisioning request must complete");

    let provision_status = provision_resp.status();
    let provision_body: serde_json::Value = provision_resp
        .json()
        .await
        .expect("provisioning response body must be JSON");
    assert_eq!(
        provision_status,
        201,
        "provisioning an aws_secret project must succeed (AC-WSF-03) once main.rs constructs \
         a real aws_secret_fetcher and AWS credentials genuinely resolve via LocalStack — a \
         500 aws_secret_fetcher_not_configured here means main.rs still hardcodes None; got \
         {provision_status}: {provision_body}"
    );
    let api_key = provision_body["api_key"]
        .as_str()
        .expect("api_key must be present in provisioning response")
        .to_string();

    let channel =
        tonic::transport::Channel::from_shared(format!("http://127.0.0.1:{}", server.grpc_port))
            .expect("valid gRPC channel URI")
            .connect_lazy();
    let mut grpc = FirestoreClient::new(channel);

    let parent = "projects/wsf-ws-aws-project/databases/(default)/documents".to_string();
    let mut fields = HashMap::new();
    fields.insert("greeting".to_string(), string_value("hello-from-aws-secret"));
    let mut create_req = tonic::Request::new(CreateDocumentRequest {
        parent,
        collection_id: "wsf_probe".to_string(),
        document_id: "doc-1".to_string(),
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    });
    create_req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().expect("valid metadata value"),
    );

    let created = grpc
        .create_document(create_req)
        .await
        .expect(
            "CreateDocument must succeed through the real aws_secret_fetcher wiring (AC-WSF-01) \
             — a Status::internal(\"aws_secret_fetcher not configured on this server\") here \
             means main.rs still hardcodes None, None",
        )
        .into_inner();
    assert!(
        created.name.ends_with("wsf_probe/doc-1"),
        "expected created document name to end with wsf_probe/doc-1, got {}",
        created.name
    );

    let mut get_req = tonic::Request::new(GetDocumentRequest {
        name: created.name.clone(),
        ..Default::default()
    });
    get_req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().expect("valid metadata value"),
    );
    let fetched = grpc
        .get_document(get_req)
        .await
        .expect("GetDocument must succeed through the real aws_secret_fetcher wiring")
        .into_inner();

    assert_eq!(
        fetched.name, created.name,
        "the document Alex's SDK reads back must be the one Morgan's provisioned aws_secret \
         project just wrote"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WSF-02: GCP wiring (documented testability gap — see file header) ───

/// See this file's own header doc comment ("DISTILL finding — GCP
/// testability gap") for the full explanation. This test proves `main.rs`
/// constructed `Some(gcp_secret_fetcher)` when `EMBYR_GCP_ACCESS_TOKEN` is
/// set — it does NOT prove a full round-trip against a real document,
/// because `GCP_SECRET_MANAGER_BASE_URL` has no test-injectable override and
/// this suite has no real GCP project to point at.
///
/// Journey:
///   Given: the embyr-server deployment has EMBYR_GCP_ACCESS_TOKEN configured
///          (a syntactically-plausible but non-real bearer token)
///   When:  Morgan calls the admin provisioning endpoint with
///          backend_mode=gcp_secret and a resource name
///   Then:  the response is NOT 500 gcp_secret_fetcher_not_configured — the
///          real Google endpoint rejects the fake token/resource with a
///          DIFFERENT error, proving the fetcher is genuinely Some
///
/// Requires network egress to secretmanager.googleapis.com.
///
/// @error @boundary @US-01 @AC-WSF-02 @requires_external
#[tokio::test]
#[ignore]
async fn gcp_secret_project_wiring_reached_when_token_configured() {
    let (_sys, sys_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &sys_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            (
                "EMBYR_GCP_ACCESS_TOKEN",
                "wsf-test-invalid-token-not-a-real-credential",
            ),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must start normally with EMBYR_GCP_ACCESS_TOKEN set"
    );

    let http = reqwest::Client::new();
    let resp = http
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/projects",
            server.admin_port
        ))
        .header("Authorization", "Bearer testkey")
        .json(&serde_json::json!({
            "project_id": "wsf-gcp-wiring-project",
            "backend_mode": "gcp_secret",
            "gcp_resource_name": "projects/wsf-nonexistent/secrets/wsf-nonexistent-secret",
        }))
        .send()
        .await
        .expect("provisioning request must complete (requires network egress to Google)");

    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .expect("provisioning response body must be JSON");

    assert_ne!(
        status, 500,
        "a 500 here means gcp_secret_fetcher is still None -- main.rs never reached the \
         Some(...) construction arm; got {status}: {body}"
    );
    assert_ne!(
        body["error"].as_str(),
        Some("gcp_secret_fetcher_not_configured"),
        "the OLD error this feature exists to eliminate must not appear once \
         EMBYR_GCP_ACCESS_TOKEN is set; got {body}"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WSF-04: neither cloud configured — no global fail-fast ─────────────

/// A deployment with neither AWS credentials nor EMBYR_GCP_ACCESS_TOKEN
/// configured starts up normally, and an existing direct_pg-mode project's
/// behavior is provably unchanged (not merely "still works").
///
/// Journey (regression guard):
///   Given: the embyr-server deployment has no AWS credentials and no
///          EMBYR_GCP_ACCESS_TOKEN configured
///   When:  the server starts
///   Then:  the server starts successfully and binds all three listeners
///   And:   an existing direct_pg-mode project continues to serve
///          CreateDocument/GetDocument requests exactly as before
///
/// @error @boundary @US-01 @AC-WSF-04
#[tokio::test]
#[ignore]
async fn server_with_neither_cloud_configured_starts_and_direct_pg_unaffected() {
    let (_pg, db_url) = start_postgres_container().await;

    // Sibling customer DB — system (`migrations/`) and customer
    // (`migrations/customer/`) migration sets both start at sqlx migration
    // version 1 and collide if applied to the same database (mirrors pr04's
    // own established pattern).
    let sys_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("connect to system postgres to create customer database");
    sqlx::query("CREATE DATABASE wsf04_customer")
        .execute(&sys_pool)
        .await
        .expect("create sibling customer database");
    drop(sys_pool);
    let last_slash = db_url
        .rfind('/')
        .expect("db_url must contain a path separator");
    let customer_db_url = format!("{}/wsf04_customer", &db_url[..last_slash]);

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must start with neither AWS credentials nor EMBYR_GCP_ACCESS_TOKEN configured \
         -- no global fail-fast is permitted for a missing cloud credential (System Constraints)"
    );

    let http = reqwest::Client::new();
    let provision_resp = http
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/projects",
            server.admin_port
        ))
        .header("Authorization", "Bearer testkey")
        .json(&serde_json::json!({
            "project_id": "wsf04-direct-pg-project",
            "dsn": customer_db_url,
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("provisioning request must complete");
    assert!(
        provision_resp.status().is_success(),
        "a direct_pg project must provision successfully exactly as before this feature; got {}",
        provision_resp.status()
    );
    let body: serde_json::Value = provision_resp
        .json()
        .await
        .expect("provisioning response body must be JSON");
    let api_key = body["api_key"]
        .as_str()
        .expect("api_key must be present")
        .to_string();

    let channel =
        tonic::transport::Channel::from_shared(format!("http://127.0.0.1:{}", server.grpc_port))
            .expect("valid gRPC channel URI")
            .connect_lazy();
    let mut grpc = FirestoreClient::new(channel);
    let parent = "projects/wsf04-direct-pg-project/databases/(default)/documents".to_string();
    let mut fields = HashMap::new();
    fields.insert("k".to_string(), string_value("v"));
    let mut req = tonic::Request::new(CreateDocumentRequest {
        parent,
        collection_id: "wsf04_probe".to_string(),
        document_id: "doc-1".to_string(),
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    });
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().expect("valid metadata value"),
    );
    let doc = grpc
        .create_document(req)
        .await
        .expect(
            "direct_pg CreateDocument must succeed unchanged by this feature's own \
             unconditional aws_secret_fetcher construction",
        )
        .into_inner();
    assert!(
        doc.name.ends_with("wsf04_probe/doc-1"),
        "expected created document name to end with wsf04_probe/doc-1, got {}",
        doc.name
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WSF-05: AWS fail-closed non-regression ──────────────────────────────

/// An already-provisioned aws_secret project still fails closed
/// (Status::internal, never a panic, never a stale/wrong-DSN success) when
/// this deployment genuinely lacks usable AWS credentials — the SAME overall
/// failure class as today, explicitly locked as a non-regression.
///
/// Journey (DISCUSS Example 3 — "legacy-staging-test"):
///   Given: the embyr-server deployment has no usable AWS credentials in its
///          credential chain (and the seeded project's ARN does not exist in
///          any real AWS account either way, so this holds regardless of
///          ambient credentials on the machine running this test)
///   And:   a project "legacy-staging-test" already exists with
///          backend_mode=aws_secret (seeded directly — provisioning it for
///          real would itself require a reachable secret, which is not this
///          scenario's precondition; mirrors pr05's own direct-SQL-seed
///          pattern for the identical reason)
///   When:  Alex's SDK client calls GetDocument against "legacy-staging-test"
///   Then:  the request fails with a Status::internal error, not a panic and
///          not a stale/wrong-DSN success
///
/// @error @US-01 @AC-WSF-05
#[tokio::test]
#[ignore]
async fn aws_secret_project_fails_closed_when_aws_credentials_absent() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must start (AWS fetcher construction is unconditional and infallible even \
         with no usable credentials — DESIGN D-WSF-1)"
    );

    let sys_pool = sqlx::PgPool::connect(&db_url)
        .await
        .expect("connect to system postgres");
    let api_key = "wsf05-legacy-staging-test-key";
    let api_key_hash =
        argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
    sqlx::query(
        "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, backend_secret_arn) \
         VALUES ($1, 'active', 'aws_secret', $2, $3)",
    )
    .bind("legacy-staging-test")
    .bind(&api_key_hash)
    .bind("arn:aws:secretsmanager:us-east-1:000000000000:secret:wsf05-nonexistent-xyz")
    .execute(&sys_pool)
    .await
    .expect("seed legacy aws_secret project row");
    sqlx::query(
        "INSERT INTO rate_buckets (project_id, tokens, last_refill) VALUES ($1, 1000, now())",
    )
    .bind("legacy-staging-test")
    .execute(&sys_pool)
    .await
    .expect("seed rate bucket row");

    let channel =
        tonic::transport::Channel::from_shared(format!("http://127.0.0.1:{}", server.grpc_port))
            .expect("valid gRPC channel URI")
            .connect_lazy();
    let mut grpc = FirestoreClient::new(channel);
    let mut req = tonic::Request::new(GetDocumentRequest {
        name: "projects/legacy-staging-test/databases/(default)/documents/wsf05_probe/doc-1"
            .to_string(),
        ..Default::default()
    });
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().expect("valid metadata value"),
    );

    let result = tokio::time::timeout(Duration::from_secs(20), grpc.get_document(req))
        .await
        .expect("GetDocument must complete within 20s, not hang");
    let status = result.expect_err(
        "a request against an aws_secret project with genuinely unusable credentials must \
         fail, not silently succeed with a stale/wrong DSN",
    );

    assert_eq!(
        status.code(),
        tonic::Code::Internal,
        "must fail closed with Status::internal -- the same overall failure class as before \
         this feature, never a panic and never success; got {status:?}"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── DISCUSS scenario 6: GCP provisioning rejected when token unset ────────

/// Zero regression: gcp_secret provisioning is REJECTED, byte-identically to
/// today, when EMBYR_GCP_ACCESS_TOKEN is unset. Unlike AC-WSF-02's wiring
/// test, this scenario needs no network egress at all — gcp_secret_fetcher
/// stays genuinely None and the existing pre-fetch-call gate in
/// provision.rs fires before any HTTP call to Google is attempted.
///
/// Journey:
///   Given: the embyr-server deployment has no EMBYR_GCP_ACCESS_TOKEN configured
///   When:  Morgan calls the admin provisioning endpoint with
///          backend_mode=gcp_secret and a valid-looking gcp_resource_name
///   Then:  provisioning is rejected with the SAME named configuration error
///          as before this feature
///   And:   no project row is created and the server does not crash
///
/// @error @US-01 @AC-WSF-05
#[tokio::test]
#[ignore]
async fn gcp_secret_provisioning_rejected_when_token_unset() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must start with no GCP token configured");

    let http = reqwest::Client::new();
    let resp = http
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/projects",
            server.admin_port
        ))
        .header("Authorization", "Bearer testkey")
        .json(&serde_json::json!({
            "project_id": "wsf06-gcp-rejected-project",
            "backend_mode": "gcp_secret",
            "gcp_resource_name": "projects/wsf06/secrets/wsf06-secret",
        }))
        .send()
        .await
        .expect("provisioning request must complete");

    assert_eq!(
        resp.status(),
        500,
        "gcp_secret provisioning without EMBYR_GCP_ACCESS_TOKEN must be rejected exactly as \
         before this feature; got {}",
        resp.status()
    );
    let body: serde_json::Value = resp
        .json()
        .await
        .expect("provisioning response body must be JSON");
    assert_eq!(body["error"], "gcp_secret_fetcher_not_configured");

    let sys_pool = sqlx::PgPool::connect(&db_url)
        .await
        .expect("connect to system postgres");
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM projects WHERE id = $1")
        .bind("wsf06-gcp-rejected-project")
        .fetch_optional(&sys_pool)
        .await
        .expect("query projects table");
    assert!(
        existing.is_none(),
        "no project row may be created when provisioning is rejected"
    );
    assert!(
        server.child.try_wait().ok().flatten().is_none(),
        "server must not have crashed/panicked while rejecting the request"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}

// ─── AC-WSF-06: transaction sweeper reaches the third call site ────────────

/// The same constructed fetcher instances reach ALL THREE composition-root
/// call sites, not only FirestoreService/build_admin_router: the transaction
/// sweeper (main.rs's third hardcoded-None call site, ADR-054 §D7) must
/// actually reclaim an orphaned transaction for a real aws_secret project
/// once AWS credentials are available — it must no longer silently skip it.
///
/// Side effect (DESIGN's own § Reading Confirmation): this necessarily
/// exercises `resolve_aws_secret_dsn` (customer_db_connect.rs), closing
/// `composite-index-real-creation`'s follow-up-flagged zero-test-coverage
/// gap for that function — no dedicated test is warranted beyond this one.
///
/// Journey:
///   Given: LocalStack AWS Secrets Manager has a secret containing a real
///          customer Postgres DSN, and an aws_secret project with an
///          orphaned ('active', started 20 minutes ago) transaction exists
///          in that customer database
///   And:   the embyr-server deployment has AWS credentials available
///   When:  the real transaction_sweeper::spawn(...) inside main.rs runs a
///          cycle (EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS=1 for fast test feedback)
///   Then:  the orphaned transaction's status becomes 'expired' within 15s
///
/// @error @US-01 @AC-WSF-06
#[tokio::test]
#[ignore]
async fn transaction_sweeper_reclaims_aws_secret_project_when_aws_credentials_available() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_cust, cust_url) = start_postgres_container().await;
    let (_sys, sys_url) = start_postgres_container().await;

    // Seeding the transaction row directly (bypassing HTTP provisioning, per
    // AC-WSF-05's own reasoning above) requires the customer schema to
    // already exist — apply it by hand, mirroring pr05's setup_tls_server.
    let cust_pool = sqlx::PgPool::connect(&cust_url)
        .await
        .expect("connect customer db");
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .expect("run customer migrations");

    let sm_client = make_sm_client(&endpoint_url).await;
    let secret_string = format!(r#"{{"dsn": "{cust_url}"}}"#);
    let create_resp = sm_client
        .create_secret()
        .name("wsf06-sweeper-dsn-secret")
        .secret_string(&secret_string)
        .send()
        .await
        .expect("create secret in LocalStack");
    let arn = create_resp.arn().expect("arn present").to_string();

    let mut server = ServerProcess::start(
        &sys_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("AWS_ENDPOINT_URL", &endpoint_url),
            ("AWS_ACCESS_KEY_ID", "test"),
            ("AWS_SECRET_ACCESS_KEY", "test"),
            ("AWS_REGION", "us-east-1"),
            ("EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS", "1"),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must start with AWS credentials pointing at LocalStack");

    // Seed AFTER healthy: main.rs applies system migrations at its own
    // startup (Step 5), so the projects/rate_buckets/transactions tables are
    // guaranteed to exist by the time /healthz first returns 200.
    let sys_pool = sqlx::PgPool::connect(&sys_url)
        .await
        .expect("connect system db");
    let api_key_hash =
        argon2::hash_api_key(b"wsf06-sweeper-key").expect("hash api key");
    sqlx::query(
        "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, backend_secret_arn) \
         VALUES ($1, 'active', 'aws_secret', $2, $3)",
    )
    .bind("wsf06-sweeper-project")
    .bind(&api_key_hash)
    .bind(&arn)
    .execute(&sys_pool)
    .await
    .expect("seed aws_secret project row");
    sqlx::query(
        "INSERT INTO rate_buckets (project_id, tokens, last_refill) VALUES ($1, 1000, now())",
    )
    .bind("wsf06-sweeper-project")
    .execute(&sys_pool)
    .await
    .expect("seed rate bucket row");

    let transaction_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (transaction_id, project_id, status, started_at) \
         VALUES ($1, $2, 'active', now() - interval '20 minutes')",
    )
    .bind(transaction_id)
    .bind("wsf06-sweeper-project")
    .execute(&cust_pool)
    .await
    .expect("seed orphaned transaction row");

    // Poll the customer DB directly for the sweeper's own reclaim (the real
    // transaction_sweeper::spawn inside the real main.rs, not a
    // test-invoked run_cycle — proving the wiring, not just the mechanism).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let final_status;
    loop {
        let status: String = sqlx::query_scalar(
            "SELECT status FROM transactions WHERE transaction_id = $1",
        )
        .bind(transaction_id)
        .fetch_one(&cust_pool)
        .await
        .expect("read transaction status");
        if status == "expired" || tokio::time::Instant::now() >= deadline {
            final_status = status;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        final_status, "expired",
        "the transaction sweeper must reclaim an orphaned transaction for a real aws_secret \
         project once AWS credentials are available -- a status other than 'expired' means \
         main.rs's own transaction_sweeper::spawn(...) call is STILL hardcoding None, None \
         and silently skipping this project (ADR-054 §D7)"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(10)).await;
}
